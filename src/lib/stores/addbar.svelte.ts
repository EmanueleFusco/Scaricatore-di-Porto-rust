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
import {
  isVideoWithListUrl,
  normaliseInspected,
  inspectDelay,
  MetadataCache,
  PLAYLIST_PAGE_SIZE,
  type Inspected,
  type InspectRequestOptions,
} from "$lib/inspect";
import { QUALITY_PRESETS, newLinks, parseHeaderLines, splitLinks } from "$lib/addbar";
import { mergeEntries, nextPageRange } from "$lib/playlist";
import { isTauriRuntime, settingsStore } from "$lib/stores/settings.svelte";
import type { AddItemRequest, QualityChoice } from "$lib/queue";
import type { BackendErrorInfo } from "$lib/settings";

/** One link being added, and how far along its inspection is. */
export interface AddRow {
  /** Stable within one session; not the backend's item id. */
  key: string;
  url: string;
  /**
   * `choice`: a `watch?v=…&list=…` link waiting for *This video only* /
   * *Whole playlist* (spec §6.4). `playlist`: a playlist whose selection panel
   * is open; it is added through the panel, never by the main buttons.
   */
  state: "pending" | "checking" | "choice" | "ready" | "playlist" | "error" | "skipped";
  inspected: Inspected | null;
  errorKey: string | null;
  errorDetail: string | null;
  /** `null` means "whatever the preset says". */
  quality: QualityChoice | null;
  /** Inspect (and download) the single video of a `…&list=…` link (spec §6.4). */
  noPlaylist: boolean;
  /** Live streams: `true` = record from the start, `false` = from now. */
  liveFromStart: boolean;
  /** "More options" (spec §11.3): per-link network headers, as typed. */
  referer: string;
  userAgent: string;
  customHeaders: string;
  /** Loading the next page of a playlist ("Load more", spec §6.4). */
  loadingMore: boolean;
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
  /** Spec §2.9: a URL inspected before answers without a second round-trip. */
  #cache = new MetadataCache();

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
   * Typing or pasting schedules the inspection (spec §11.3).
   *
   * A **paste** is one atomic event, not typing: nothing else is coming, so the
   * wait is pure latency and it starts at once (D76). Typing keeps the 500 ms —
   * without them every keystroke would launch a yt-dlp process. Nothing is lost
   * if the user presses a button first: the button asks for the same inspection
   * and waits for it.
   */
  setText(value: string, pasted = false): void {
    this.text = value;
    this.#cancelTimer();
    if (this.links.length === 0) return;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.submit();
    }, inspectDelay(pasted));
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
        noPlaylist: false,
        liveFromStart: false,
        referer: "",
        userAgent: "",
        customHeaders: "",
        loadingMore: false,
      };
    });
    if (rows.length === 0) return;

    this.rows = [...this.rows, ...rows];
    this.text = "";

    // Spec §2.9: a URL inspected before answers from the cache — no yt-dlp
    // process, no network, no spinner. Only the others are asked for.
    const misses: AddRow[] = [];
    for (const row of rows) {
      // Spec §6.4: a video-with-list link asks once, unless the setting (or an
      // earlier "remember for this session") already decided.
      if (isVideoWithListUrl(row.url)) {
        const decided = this.#decidedBehaviour();
        if (decided === null) {
          this.#patch(row.key, { state: "choice" });
          continue;
        }
        this.#patch(row.key, { noPlaylist: decided === "video" });
        row.noPlaylist = decided === "video";
      }
      const hit = this.#cache.get(row.url);
      if (hit === undefined) {
        misses.push(row);
        continue;
      }
      this.#patch(row.key, {
        state: "ready",
        inspected: hit,
        errorKey: null,
        errorDetail: null,
      });
    }
    if (misses.length > 0) await this.#check(misses);
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

  /** Tick "remember for this session" on the inline choice (spec §6.4). */
  rememberChoice = $state(false);
  #sessionChoice: "video" | "playlist" | null = null;

  /** What a `…&list=…` link does without asking, or `null` to ask. */
  #decidedBehaviour(): "video" | "playlist" | null {
    const setting = settingsStore.settings.playlistUrlBehavior;
    if (setting === "video" || setting === "playlist") return setting;
    return this.#sessionChoice;
  }

  /** The user's answer to *This video only* / *Whole playlist*. */
  async choosePlaylistMode(key: string, mode: "video" | "playlist"): Promise<void> {
    if (this.rememberChoice) this.#sessionChoice = mode;
    const row = this.rows.find((candidate) => candidate.key === key);
    if (row === undefined || row.state !== "choice") return;
    this.#patch(key, { noPlaylist: mode === "video", state: "pending" });
    // Apply the same answer to the other links still waiting, when remembered.
    const others = this.rememberChoice
      ? this.rows.filter((candidate) => candidate.state === "choice" && candidate.key !== key)
      : [];
    for (const other of others) {
      this.#patch(other.key, { noPlaylist: mode === "video", state: "pending" });
    }
    const keys = new Set([key, ...others.map((other) => other.key)]);
    await this.#check(this.rows.filter((candidate) => keys.has(candidate.key)));
  }

  /** "Load more" of a playlist: the next page, merged by position (spec §6.4). */
  async loadMore(key: string): Promise<void> {
    const row = this.rows.find((candidate) => candidate.key === key);
    if (!isTauriRuntime() || row === undefined || row.inspected?.kind !== "playlist") return;
    if (row.loadingMore) return;
    const known = row.inspected;
    this.#patch(key, { loadingMore: true });
    try {
      const [from, to] = nextPageRange(known.entries.length, PLAYLIST_PAGE_SIZE);
      const raw = await invoke<unknown[]>("inspect_urls", {
        requestId: ++this.#request,
        urls: [row.url],
        options: { ...this.#optionsOf(row), itemRange: [from, to] },
      });
      const answer = (raw[0] ?? {}) as Record<string, unknown>;
      if (!answer.inspected) {
        this.#patch(key, { loadingMore: false });
        return;
      }
      const page = normaliseInspected(answer.inspected);
      if (page.kind !== "playlist") {
        this.#patch(key, { loadingMore: false });
        return;
      }
      this.#patch(key, {
        loadingMore: false,
        inspected: {
          ...known,
          count: page.count ?? known.count,
          entries: mergeEntries(known.entries, page.entries),
          truncated: page.truncated,
        },
      });
    } catch (err) {
      this.#patch(key, { loadingMore: false });
      this.#fail(err);
    }
  }

  /** Per-link record-from-start choice of a live stream (spec §6.4). */
  setLiveFromStart(key: string, fromStart: boolean): void {
    this.#patch(key, { liveFromStart: fromStart });
  }

  /** Per-link "More options": referer, user agent, extra headers (spec §11.3). */
  setNetworkOptions(
    key: string,
    options: Partial<Pick<AddRow, "referer" | "userAgent" | "customHeaders">>,
  ): void {
    this.#patch(key, options);
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
    // Spec §2.9: the cache is cleared by Clear.
    this.#cache.clear();
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
      const headers = parseHeaderLines(row.customHeaders);
      return {
        url: row.url,
        title: video?.title ?? null,
        site: video?.site ?? null,
        durationSec: video?.duration ?? null,
        kind: quality.kind === "audioOnly" ? ("audio" as const) : ("video" as const),
        quality,
        outputDir: this.folder,
        groupId: null,
        // `null` for a normal video: only a live stream has a mode to record in.
        liveFromStart: video?.isLive ? row.liveFromStart : null,
        referer: row.referer.trim() === "" ? null : row.referer.trim(),
        userAgent: row.userAgent.trim() === "" ? null : row.userAgent.trim(),
        customHeaders: headers,
      };
    });
  }

  /** Drop the rows that were added, keep the rest. */
  clearAdded(): void {
    this.rows = this.rows.filter((row) => row.state !== "ready");
  }

  /** The per-inspection flags of one row (spec §6.2, §6.4, §11.3). */
  #optionsOf(row: AddRow): InspectRequestOptions {
    const headers = parseHeaderLines(row.customHeaders);
    return {
      noPlaylist: row.noPlaylist ? true : undefined,
      referer: row.referer.trim() === "" ? null : row.referer.trim(),
      userAgent: row.userAgent.trim() === "" ? null : row.userAgent.trim(),
      customHeaders: headers,
    };
  }

  async #check(rows: AddRow[]): Promise<void> {
    if (!isTauriRuntime() || rows.length === 0) return;
    this.#request += 1;
    const request = this.#request;
    this.#mark(rows.map((row) => row.key), "checking");

    // One batch per distinct set of flags: the backend takes one `options` per
    // call, and most pastes have a single group.
    const batches = new Map<string, { options: InspectRequestOptions; rows: AddRow[] }>();
    for (const row of rows) {
      const options = this.#optionsOf(row);
      const id = JSON.stringify(options);
      const batch = batches.get(id) ?? { options, rows: [] };
      batch.rows.push(row);
      batches.set(id, batch);
    }

    try {
      await Promise.all(
        [...batches.values()].map(async (batch) => {
          try {
            const raw = await invoke<unknown[]>("inspect_urls", {
              requestId: request,
              urls: batch.rows.map((row) => row.url),
              options: batch.options,
            });
            if (request !== this.#request) return; // stale answer (spec §12.12)
            this.#applyAnswers(raw);
          } catch (err) {
            if (request !== this.#request) return;
            const info = err as Partial<BackendErrorInfo>;
            for (const row of batch.rows) {
              this.#patch(row.key, {
                state: "error",
                errorKey: typeof info?.messageKey === "string" ? info.messageKey : "error.tauri",
                errorDetail: typeof info?.detail === "string" ? info.detail : null,
              });
            }
          }
        }),
      );
    } finally {
      if (request === this.#request) {
        this.checking = this.rows.some(
          (row) => row.state === "checking" || row.state === "pending",
        );
      }
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
      // Spec §2.9: a complete answer is worth remembering; the cache refuses
      // playlists, errors and degraded lists on its own.
      if (inspected.kind === "video") {
        this.#cache.set(row.url, inspected);
      }
      // A refusal the inspection itself classified (auth, unsupported page, …)
      // is an inline error with its own message, never a downloadable row.
      if (inspected.kind === "error") {
        this.#patch(row.key, {
          state: "error",
          inspected,
          errorKey: inspected.messageKey,
          errorDetail: inspected.detail,
        });
        continue;
      }
      // A playlist opens the selection panel (spec §6.4, §11.4); it is added
      // through the panel, not by the main buttons.
      if (inspected.kind === "playlist") {
        this.#patch(row.key, {
          state: "playlist",
          inspected,
          errorKey: null,
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