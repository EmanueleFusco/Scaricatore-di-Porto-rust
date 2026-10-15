/**
 * The Add bar's state (spec §2.2, §11.3).
 *
 * One input takes **one or many** links: a multi-line paste, a drop of text, a
 * dropped `.txt` file. Inspection starts on its own, debounced 500 ms, exactly
 * as in the original (spec §11.3): every link becomes a row with a spinner and
 * then its title, site, duration and available qualities, which the user can
 * change per row before adding them all at once.
 *
 * The batch inspection (`inspect_urls`) is what makes ten links usable: three
 * in parallel (spec §6.2), each with its own cancellation, answers in order.
 * The pure parts — splitting a paste, the links not seen yet, building the
 * quality options — are in `$lib/addbar`, so they can be tested without a
 * Svelte runtime.
 */
import { invoke } from "@tauri-apps/api/core";
import { normaliseInspected, type Inspected } from "$lib/inspect";
import { QUALITY_PRESETS, newLinks, splitLinks } from "$lib/addbar";
import { isTauriRuntime } from "$lib/stores/settings.svelte";
import type { AddItemRequest, QualityChoice } from "$lib/queue";
import type { BackendErrorInfo } from "$lib/settings";

/** One link being added, and how far along its inspection is. */
export interface AddRow {
  /** Stable within one session; not the backend's item id. */
  key: string;
  url: string;
  state: "pending" | "checking" | "ready" | "error" | "skipped";
  inspected: Inspected | null;
  errorKey: string | null;
  errorDetail: string | null;
  /** `null` means "whatever the preset says". */
  quality: QualityChoice | null;
}

const MAX_ROWS = 500;

export { QUALITY_PRESETS, splitLinks };

class AddBarStore {
  /** What the user typed, not split yet. */
  text = $state("");
  rows = $state<AddRow[]>([]);
  /** The preset applied to rows that have no per-row choice. */
  preset = $state("best");
  folder = $state<string | null>(null);
  checking = $state(false);
  dragging = $state(false);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);

  #counter = 0;
  /** Bumped for every batch; an answer older than this is dropped. */
  #request = 0;
  /** The pending debounced inspection, if one is waiting (spec §11.3). */
  #timer: ReturnType<typeof setTimeout> | null = null;

  /** Rows that can be added right now. */
  get ready(): AddRow[] {
    return this.rows.filter((row) => row.state === "ready" && row.url.startsWith("http"));
  }

  get readyCount(): number {
    return this.ready.length;
  }

  get hasRows(): boolean {
    return this.rows.length > 0;
  }

  /** The links in the box, without touching the rows. */
  get links(): string[] {
    return splitLinks(this.text);
  }

  /** The effective quality of a row: its own, or the preset's. */
  effectiveQuality(row: AddRow): QualityChoice {
    if (row.quality !== null) return row.quality;
    const preset = QUALITY_PRESETS.find((option) => option.value === this.preset);
    return preset?.choice ?? { kind: "best" };
  }

  /**
   * Typing or pasting schedules the inspection, 500 ms after the last change
   * (spec §11.3, "as in the original"). Nothing is lost if the user presses a
   * button first: the button asks for the same inspection and waits for it.
   */
  setText(value: string): void {
    this.text = value;
    this.#cancelTimer();
    if (this.links.length === 0) return;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.submit();
    }, 500);
  }

  /** Check everything in the box: the debounce, the button, or Enter. */
  async submit(): Promise<void> {
    this.#cancelTimer();
    const links = this.links;
    if (links.length === 0) return;
    await this.addLinks(links);
  }

  /** A paste of several links, or a drop. */
  async addLinks(links: string[]): Promise<void> {
    this.#cancelTimer();
    const known = this.rows.map((row) => row.url);
    const fresh = newLinks(links.join("\n"), known);
    if (fresh.length === 0) {
      this.text = "";
      return;
    }

    const room = Math.max(0, MAX_ROWS - this.rows.length);
    const rows: AddRow[] = fresh.slice(0, room).map((url) => {
      this.#counter += 1;
      return {
        key: `r${this.#counter}`,
        url,
        state: "pending" as const,
        inspected: null,
        errorKey: null,
        errorDetail: null,
        quality: null,
      };
    });
    if (rows.length === 0) return;

    this.rows = [...this.rows, ...rows];
    this.text = "";
    await this.#check(rows);
  }

  /** Drag-and-drop of text (spec §2.2 "drop text"). */
  async dropText(text: string): Promise<void> {
    const links = splitLinks(text);
    if (links.length > 0) await this.addLinks(links);
  }

  /** Drag-and-drop of a `.txt` file: the backend owns the filesystem. */
  async dropFile(path: string): Promise<void> {
    this.errorKey = null;
    this.errorDetail = null;
    if (!isTauriRuntime()) return;
    try {
      const raw = await invoke<string>("read_links_file", { path });
      await this.dropText(raw);
    } catch (err) {
      this.#fail(err);
    }
  }

  removeRow(key: string): void {
    this.rows = this.rows.filter((row) => row.key !== key);
  }

  clear(): void {
    this.#cancelTimer();
    this.#request += 1;
    this.rows = [];
    this.text = "";
    this.checking = false;
  }

  #cancelTimer(): void {
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
  }

  setQuality(key: string, choice: QualityChoice | null): void {
    this.rows = this.rows.map((row) => (row.key === key ? { ...row, quality: choice } : row));
  }

  /** Re-check one row (spec §11.3: the per-link Retry). */
  async retryRow(key: string): Promise<void> {
    const row = this.rows.find((candidate) => candidate.key === key);
    if (row === undefined) return;
    this.#patch(key, {
      state: "pending",
      errorKey: null,
      errorDetail: null,
      inspected: null,
    });
    await this.#check([this.rows.find((candidate) => candidate.key === key)!]);
  }

  /** Re-check everything that failed (spec §11.3). */
  async retryFailed(): Promise<void> {
    const broken = this.rows.filter(
      (row) => row.state === "error" || row.state === "skipped",
    );
    if (broken.length === 0) return;
    for (const row of broken) {
      this.#patch(row.key, { state: "pending", errorKey: null, errorDetail: null, inspected: null });
    }
    await this.#check(
      broken.map((row) => ({ ...row, state: "pending" as const })),
    );
  }

  /** The payload `queue_add` expects. */
  toRequests(): AddItemRequest[] {
    return this.ready.map((row) => {
      const video = row.inspected?.kind === "video" ? row.inspected : null;
      const quality = this.effectiveQuality(row);
      return {
        url: row.url,
        title: video?.title ?? null,
        site: video?.site ?? null,
        durationSec: video?.duration ?? null,
        kind: quality.kind === "audioOnly" ? ("audio" as const) : ("video" as const),
        quality,
        outputDir: this.folder,
        groupId: null,
      };
    });
  }

  /** Drop the rows that were added, keep the rest. */
  clearAdded(): void {
    this.rows = this.rows.filter((row) => row.state !== "ready");
  }

  async #check(rows: AddRow[]): Promise<void> {
    if (!isTauriRuntime() || rows.length === 0) return;
    this.#request += 1;
    const request = this.#request;
    const urls = rows.map((row) => row.url);
    this.#mark(rows.map((row) => row.key), "checking");

    try {
      const raw = await invoke<unknown[]>("inspect_urls", { requestId: request, urls });
      if (request !== this.#request) return; // stale answer (spec §12.12)
      this.#applyAnswers(raw);
    } catch (err) {
      if (request !== this.#request) return;
      const info = err as Partial<BackendErrorInfo>;
      this.#mark(rows.map((row) => row.key), "error");
      for (const row of rows) {
        this.#patch(row.key, {
          errorKey: typeof info?.messageKey === "string" ? info.messageKey : "error.tauri",
          errorDetail: typeof info?.detail === "string" ? info.detail : null,
        });
      }
    } finally {
      if (request === this.#request) this.checking = false;
    }
  }

  /** Fold the batch answers in, by URL: they may arrive out of order. */
  #applyAnswers(raw: unknown[]): void {
    const answers = new Map<string, Record<string, unknown>>();
    for (const entry of raw) {
      const answer = (entry ?? {}) as Record<string, unknown>;
      answers.set(typeof answer.url === "string" ? answer.url : "", answer);
    }
    for (const row of this.rows) {
      if (row.state !== "checking" && row.state !== "pending") continue;
      const answer = answers.get(row.url);
      if (answer === undefined) continue;
      const error = answer.error as Partial<BackendErrorInfo> | null | undefined;
      if (error !== null && error !== undefined) {
        this.#patch(row.key, {
          state: "error",
          errorKey: typeof error.messageKey === "string" ? error.messageKey : "error.tauri",
          errorDetail: typeof error.detail === "string" ? error.detail : null,
        });
        continue;
      }
      if (!answer.inspected) {
        this.#patch(row.key, { state: "error", errorKey: "error.tauri", errorDetail: null });
        continue;
      }
      const inspected = normaliseInspected(answer.inspected);
      // A playlist is not a queue item in M2 (spec §6.4 arrives in M3): the row
      // says so instead of quietly downloading the wrong thing.
      if (inspected.kind === "playlist") {
        this.#patch(row.key, {
          state: "skipped",
          inspected,
          errorKey: "queue.add.playlist.unsupported",
          errorDetail: null,
        });
        continue;
      }
      this.#patch(row.key, { state: "ready", inspected, errorKey: null, errorDetail: null });
    }
  }

  #mark(keys: string[], state: AddRow["state"]): void {
    const set = new Set(keys);
    this.rows = this.rows.map((row) => (set.has(row.key) ? { ...row, state } : row));
    this.checking = this.rows.some(
      (row) => row.state === "checking" || row.state === "pending",
    );
  }

  #patch(key: string, patch: Partial<AddRow>): void {
    this.rows = this.rows.map((row) => (row.key === key ? { ...row, ...patch } : row));
  }

  #fail(err: unknown): void {
    const info = err as Partial<BackendErrorInfo>;
    this.errorKey = typeof info?.messageKey === "string" ? info.messageKey : "error.tauri";
    this.errorDetail = typeof info?.detail === "string" ? info.detail : null;
  }
}

export const addBarStore = new AddBarStore();