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
import { Channel, invoke } from "@tauri-apps/api/core";
import {
  isVideoWithListUrl,
  normaliseInspected,
  inspectDelay,
  MetadataCache,
  PLAYLIST_PAGE_SIZE,
  type Inspected,
  type InspectRequestOptions,
} from "$lib/inspect";
import { QUALITY_PRESETS, newLinks, parseHeaderLines, resolveRowTranscriptLanguage, splitLinks } from "$lib/addbar";
import { mergeEntries, nextPageRange } from "$lib/playlist";
import { isTauriRuntime, settingsStore } from "$lib/stores/settings.svelte";
import { queueStore } from "$lib/stores/queue.svelte";
import type { AddItemRequest, QualityChoice } from "$lib/queue";
import type { BackendErrorInfo } from "$lib/settings";
import { defaultPresetOptions, presetOptions, type PresetOptions, type SavedPreset } from "$lib/presets";

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
  /** M4 media options (spec §9, §8.4). */
  encode: boolean;
  videoCodec: string | null;
  audioCodec: string | null;
  trimStart: string;
  trimEnd: string;
  keepOriginal: boolean;
  autoPlay: boolean;
  embedMetadata: boolean;
  embedThumbnail: boolean;
  embedChapters: boolean;
  sponsorblockMode: "mark" | "remove" | null;
  sponsorblockCategories: string[];
  /** Whisper transcription (spec §10.2). */
  transcribe: boolean;
  whisperModel: string | null;
  transcriptLanguage: string | null;
  transcriptLanguageOverridden: boolean;
  translateToEnglish: boolean;
  transcriptsSubfolder: boolean;
  transcriptionEngine: string | null;
  geminiModel: string | null;
  summarize: boolean;
  summaryLanguage: string | null;
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
  dragging = $state(false);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);

  #counter = 0;
  get rowGeneration(): number { return this.#counter; }
  #presetsRestored = false;
  #presetOptions: PresetOptions = {};

  restorePreset(): void {
    if (this.#presetsRestored) return;
    this.#presetsRestored = true;
    this.#applyPreset(settingsStore.settings.lastPreset ?? null);
  }

  #applyPreset(id: string | null): void {
    const saved = settingsStore.settings.savedPresets?.find((preset) => preset.id === id);
    this.preset = saved?.quality ?? "best";
    this.#presetOptions = saved ? presetOptions(saved.options) : {};
    const options = { ...defaultPresetOptions(settingsStore.settings), ...this.#presetOptions };
    this.rows = this.rows.map((row) => ({ ...row, ...options, quality: null,
      encode: options.encode === true || row.trimStart.trim() !== "" || row.trimEnd.trim() !== "",
    }));
  }

  async selectPreset(id: string | null): Promise<boolean> {
    if (!await settingsStore.update({ lastPreset: id })) return false;
    this.#applyPreset(id);
    return true;
  }

  async savePreset(name: string): Promise<boolean> {
    const label = Array.from(name.trim()).slice(0, 80).join("");
    if (!label) return false;
    const presets = settingsStore.settings.savedPresets;
    const existing = presets.find((entry) => entry.name.toLocaleLowerCase() === label.toLocaleLowerCase());
    if (!existing && presets.length >= 20) return false;
    const saved: SavedPreset = { id: existing?.id ?? crypto.randomUUID(), name: label,
      quality: this.preset,
      options: presetOptions(this.rows[0] ?? defaultPresetOptions(settingsStore.settings)),
    };
    if (!await settingsStore.update({ savedPresets: [...presets.filter((entry) => entry.id !== saved.id), saved], lastPreset: saved.id })) return false;
    this.#applyPreset(saved.id);
    return true;
  }

  async deletePreset(id: string): Promise<boolean> {
    const lastPreset = settingsStore.settings.lastPreset === id ? null : settingsStore.settings.lastPreset;
    if (!await settingsStore.update({ savedPresets: settingsStore.settings.savedPresets.filter((entry) => entry.id !== id), lastPreset })) return false;
    this.#applyPreset(lastPreset);
    return true;
  }
  /** Unique across a webview reload; each request belongs to its own rows. */
  #request = Date.now() * 1000;
  #rowRequests = new Map<string, number>();
  #requests = new Map<number, Map<string, string>>();
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

  get checking(): boolean {
    return this.rows.some((row) => row.state === "checking" || row.state === "pending");
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
        encode: false,
        videoCodec: settingsStore.settings.defaultVideoCodec,
        audioCodec: settingsStore.settings.defaultAudioCodec ?? "aac",
        trimStart: "",
        trimEnd: "",
        keepOriginal: settingsStore.settings.keepOriginalAfterEncode,
        autoPlay: settingsStore.settings.autoPlay,
        embedMetadata: settingsStore.settings.embedMetadata,
        embedThumbnail: settingsStore.settings.embedThumbnail,
        embedChapters: settingsStore.settings.embedChapters,
        sponsorblockMode: settingsStore.settings.sponsorblockMode,
        sponsorblockCategories: [...settingsStore.settings.sponsorblockCategories],
        transcribe: false,
        whisperModel: settingsStore.settings.activeWhisperModel,
        transcriptLanguage: settingsStore.settings.defaultTranscriptLanguage ?? "auto",
        transcriptLanguageOverridden: false,
        translateToEnglish: settingsStore.settings.whisperTranslateToEnglish,
        transcriptsSubfolder: settingsStore.settings.transcriptsSubfolder,
        transcriptionEngine: settingsStore.settings.transcriptionEngineOrder,
        geminiModel: settingsStore.settings.defaultGeminiModel,
        summarize: settingsStore.settings.defaultSummarize,
        summaryLanguage: settingsStore.settings.defaultSummaryLanguage,
        ...presetOptions(this.#presetOptions),
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
      const transcriptLanguage = resolveRowTranscriptLanguage({
        inspectedLanguage: hit.kind === "video" ? hit.language : null,
        currentLanguage: row.transcriptLanguage,
        defaultTranscriptLanguage: settingsStore.settings.defaultTranscriptLanguage,
        languageOverridden: row.transcriptLanguageOverridden,
      });
      this.#patch(row.key, {
        state: "ready",
        inspected: hit,
        transcriptLanguage,
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

  /** Drag-and-drop of a `.txt` file or media file (spec §2.2, §11.5). */
  async dropFile(path: string): Promise<void> {
    this.errorKey = null;
    this.errorDetail = null;
    if (!isTauriRuntime()) return;
    const lower = path.toLowerCase();
    const mediaExts = [
      ".mp4", ".mkv", ".webm", ".avi", ".mov", ".flv", ".wmv",
      ".mp3", ".m4a", ".aac", ".flac", ".wav", ".ogg", ".opus",
    ];
    if (mediaExts.some((ext) => lower.endsWith(ext))) {
      await queueStore.addLocalFile(path);
      return;
    }
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
    const request = this.#startRequest([row]);
    this.#patch(key, { loadingMore: true });
    try {
      const [from, to] = nextPageRange(known.entries.length, PLAYLIST_PAGE_SIZE);
      const raw = await invoke<unknown[]>("inspect_urls", {
        requestId: request,
        urls: [row.url],
        options: { ...this.#optionsOf(row), itemRange: [from, to] },
      });
      if (this.#rowRequests.get(key) !== request) return;
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
      if (this.#rowRequests.get(key) !== request) return;
      this.#patch(key, { loadingMore: false });
      this.#fail(err);
    } finally {
      this.#finishRequest(request);
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

  /** Per-link media options (M4, spec §9, §8.4): encode, trim, embeds, sponsorblock. */
  setMediaOptions(
    key: string,
    options: Partial<
      Pick<
        AddRow,
        | "encode"
        | "videoCodec"
        | "audioCodec"
        | "trimStart"
        | "trimEnd"
        | "keepOriginal"
        | "autoPlay"
        | "embedMetadata"
        | "embedThumbnail"
        | "embedChapters"
        | "sponsorblockMode"
        | "sponsorblockCategories"
        | "transcribe"
        | "whisperModel"
        | "transcriptLanguage"
        | "transcriptLanguageOverridden"
        | "translateToEnglish"
        | "transcriptsSubfolder"
        | "transcriptionEngine"
        | "geminiModel"
        | "summarize"
        | "summaryLanguage"
      >
    >,
  ): void {
    const row = this.rows.find((r) => r.key === key);
    if (!row) return;
    const trimStart = options.trimStart !== undefined ? options.trimStart : row.trimStart;
    const trimEnd = options.trimEnd !== undefined ? options.trimEnd : row.trimEnd;
    const hasTrim = trimStart.trim() !== "" || trimEnd.trim() !== "";
    // Spec §9.2: "Trim implies Encode (coupling kept from the original)"
    const encode = hasTrim ? true : (options.encode !== undefined ? options.encode : row.encode);
    const transcriptLanguageOverridden =
      options.transcriptLanguageOverridden !== undefined
        ? options.transcriptLanguageOverridden
        : options.transcriptLanguage !== undefined
          ? true
          : row.transcriptLanguageOverridden;
    this.#patch(key, { ...options, encode, transcriptLanguageOverridden });
  }

  removeRow(key: string): void {
    this.#cancelRows(new Set([key]));
    this.rows = this.rows.filter((row) => row.key !== key);
  }

  clear(): void {
    this.#cancelTimer();
    this.#cancelRows(new Set(this.rows.map((row) => row.key)));
    this.rows = [];
    this.text = "";
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
        clientKey: row.key,
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
        encode: row.encode || row.trimStart.trim() !== "" || row.trimEnd.trim() !== "",
        videoCodec: row.videoCodec,
        audioCodec: row.audioCodec,
        trimStart: row.trimStart.trim() !== "" ? row.trimStart.trim() : null,
        trimEnd: row.trimEnd.trim() !== "" ? row.trimEnd.trim() : null,
        keepOriginal: row.keepOriginal,
        autoPlay: row.autoPlay,
        embedMetadata: row.embedMetadata,
        embedThumbnail: row.embedThumbnail,
        embedChapters: row.embedChapters,
        sponsorblockMode: row.sponsorblockMode ?? "off",
        sponsorblockCategories: row.sponsorblockCategories,
        transcribe: row.transcribe,
        whisperModel: row.whisperModel,
        transcriptLanguage: resolveRowTranscriptLanguage({
          inspectedLanguage: video?.language,
          currentLanguage: row.transcriptLanguage,
          defaultTranscriptLanguage: settingsStore.settings.defaultTranscriptLanguage,
          languageOverridden: row.transcriptLanguageOverridden,
        }),
        translateToEnglish: row.translateToEnglish,
        transcriptsSubfolder: row.transcriptsSubfolder,
        transcriptionEngine: row.transcriptionEngine,
        geminiModel: row.geminiModel,
        summarize: row.summarize,
        summaryLanguage: row.summaryLanguage,
      };
    });
  }

  /** Drop the rows that were added, keep the rest. */
  clearAdded(keys: readonly string[]): void {
    const added = new Set(keys);
    this.rows = this.rows.filter((row) => !added.has(row.key));
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

    await Promise.all(
      [...batches.values()].map(async (batch) => {
        const request = this.#startRequest(batch.rows);
        const onResult = new Channel<{ requestId: number; answer: unknown }>((event) => {
          if (event.requestId === request) this.#applyAnswers([event.answer], request, false);
        });
        try {
          const raw = await invoke<unknown[]>("inspect_urls", {
            requestId: request,
            urls: batch.rows.map((row) => row.url),
            options: batch.options,
            onResult,
          });
          this.#applyAnswers(raw, request);
        } catch (err) {
          const info = err as Partial<BackendErrorInfo>;
          for (const row of this.rows) {
            if (this.#rowRequests.get(row.key) !== request) continue;
            if (row.state !== "checking" && row.state !== "pending") continue;
            this.#patch(row.key, {
              state: "error",
              errorKey: typeof info?.messageKey === "string" ? info.messageKey : "error.tauri",
              errorDetail: typeof info?.detail === "string" ? info.detail : null,
            });
          }
        } finally {
          this.#finishRequest(request);
        }
      }),
    );
  }

  /** Fold the batch answers in, by URL: they may arrive out of order. */
  #applyAnswers(raw: unknown[], request: number, complete = true): void {
    const answers = new Map<string, Record<string, unknown>>();
    for (const entry of raw) {
      const answer = (entry ?? {}) as Record<string, unknown>;
      answers.set(typeof answer.url === "string" ? answer.url : "", answer);
    }
    for (const row of this.rows) {
      if (this.#rowRequests.get(row.key) !== request) continue;
      if (row.state !== "checking" && row.state !== "pending") continue;
      const answer = answers.get(row.url);
      if (answer === undefined) {
        if (!complete) continue;
        this.#patch(row.key, { state: "error", errorKey: "error.tauri", errorDetail: null });
        continue;
      }
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
      const transcriptLanguage = resolveRowTranscriptLanguage({
        inspectedLanguage: inspected.kind === "video" ? inspected.language : null,
        currentLanguage: row.transcriptLanguage,
        defaultTranscriptLanguage: settingsStore.settings.defaultTranscriptLanguage,
        languageOverridden: row.transcriptLanguageOverridden,
      });
      this.#patch(row.key, { state: "ready", inspected, transcriptLanguage, errorKey: null, errorDetail: null });
    }
  }

  #mark(keys: string[], state: AddRow["state"]): void {
    const set = new Set(keys);
    this.rows = this.rows.map((row) => (set.has(row.key) ? { ...row, state } : row));
  }

  #startRequest(rows: AddRow[]): number {
    this.#cancelRows(new Set(rows.map((row) => row.key)));
    const request = ++this.#request;
    this.#requests.set(request, new Map(rows.map((row) => [row.key, row.url])));
    for (const row of rows) this.#rowRequests.set(row.key, request);
    return request;
  }

  #finishRequest(request: number): void {
    for (const key of this.#requests.get(request)?.keys() ?? []) {
      if (this.#rowRequests.get(key) === request) this.#rowRequests.delete(key);
    }
    this.#requests.delete(request);
  }

  #cancelRows(keys: ReadonlySet<string>): void {
    for (const [request, rows] of this.#requests) {
      const urls: string[] = [];
      for (const [key, url] of rows) {
        if (!keys.has(key)) continue;
        urls.push(url);
        rows.delete(key);
        if (this.#rowRequests.get(key) === request) this.#rowRequests.delete(key);
      }
      if (urls.length > 0 && isTauriRuntime()) {
        void invoke("inspect_cancel", { requestId: request, urls }).catch(() => {});
      }
      if (rows.size === 0) this.#requests.delete(request);
    }
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
