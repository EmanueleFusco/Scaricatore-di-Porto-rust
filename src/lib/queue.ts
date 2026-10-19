/**
 * Pure helpers for the queue UI (M2, spec §7, §11.2, §11.5).
 *
 * Mirrors the Rust payloads of `services::queue`: `QueueItem`, `QueueSummary`
 * and the `queue:update` event. Everything that decides *what the user sees* —
 * which actions a state offers, which i18n key a stage chip uses, what the
 * footer line says, which rows a virtualised list has to render — lives here
 * and is unit-tested, so the components stay declarative and no sentence is
 * ever built by concatenation (spec §11.9).
 */

/** Mirror of `services::queue::model::ItemState`. */
export type ItemState =
  | "queued"
  | "inspecting"
  | "downloading"
  | "postProcessing"
  | "encoding"
  | "transcribing"
  | "summarizing"
  | "done"
  | "failed"
  | "cancelled"
  | "paused"
  | "interrupted";

/** Mirror of `services::queue::model::QualityChoice`. */
export type QualityChoice =
  | { kind: "best" }
  | { kind: "exact"; height: number }
  | { kind: "maxHeight"; height: number }
  | { kind: "audioOnly" }
  | { kind: "explicit"; formatId: string; hasAudio: boolean };

/** Mirror of `services::queue::model::ItemOptions`. */
export interface ItemOptions {
  kind: "video" | "audio";
  quality: QualityChoice;
  outputDir: string;
  cookies: { kind: "none" } | { kind: "browser"; browser: string } | { kind: "file"; file: string };
  runtimeMode: "auto" | "quickJs" | "deno";
  /** M3 (spec §6.4, §8.2): literal prefix before the title in `-o`, e.g. `"01 - "`. */
  filenamePrefix: string | null;
  includeIdInFilename: boolean;
  preferH264: boolean;
  /** Absolute path of the `--download-archive` file, when the archive is on. */
  archiveFile: string | null;
  sleepInterval: [number, number] | null;
  /** `true` = from start, `false` = from now, `null` = not a live stream. */
  liveFromStart: boolean | null;
  proxy: string | null;
  limitRate: string | null;
  referer: string | null;
  userAgent: string | null;
  customHeaders: string[];
  impersonateRetry: boolean;
  encode: boolean;
  videoCodec: string | null;
  audioCodec: string | null;
  trimStart: string | null;
  trimEnd: string | null;
  keepOriginal: boolean;
  autoPlay: boolean;
  embedMetadata: boolean;
  embedThumbnail: boolean;
  embedChapters: boolean;
  sponsorblockMode: "mark" | "remove" | null;
  sponsorblockCategories: string[];
  transcribe: boolean;
  whisperModel: string | null;
  transcriptLanguage: string | null;
  translateToEnglish: boolean | null;
  transcriptsSubfolder: boolean | null;
}

/** Mirror of the backend's `EncoderInfo` (spec §9.3). */
export interface EncoderInfo {
  id: string;
  label: string;
  codec: string;
  isHardware: boolean;
}

/** Mirror of `services::queue::model::ItemProgress`. */
export interface ItemProgress {
  percent: number | null;
  speedBps: number | null;
  etaSec: number | null;
  /** Stable token: `video` | `audio` | `merging` | … */
  label: string;
  subIndex: number | null;
  subTotal: number | null;
}

/** Mirror of the backend's `ErrorInfo`. */
export interface QueueErrorInfo {
  code: string;
  messageKey: string;
  detail: string | null;
}

/** Mirror of `services::queue::model::QueueItem`. */
export interface QueueItem {
  id: string;
  url: string;
  title: string | null;
  site: string | null;
  durationSec: number | null;
  state: ItemState;
  options: ItemOptions;
  groupId: string | null;
  progress: ItemProgress;
  file: string | null;
  /** Size of the produced file, in bytes (spec §11.5). */
  sizeBytes: number | null;
  runtime: string | null;
  requestedHeight: number | null;
  actualHeight: number | null;
  error: QueueErrorInfo | null;
  logTail: string[];
  autoRetries: number;
  /** Unix seconds; the item is waiting for its automatic retry. */
  retryAt: number | null;
  pausedFrom: ItemState | null;
  addedAt: number;
  updatedAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  transcriptFiles: string[];
  transcriptPreview: string | null;
}

/** Mirror of `services::queue::model::QueueSummary`. */
export interface QueueSummary {
  total: number;
  queued: number;
  running: number;
  paused: number;
  interrupted: number;
  done: number;
  failed: number;
  cancelled: number;
  speedBps: number | null;
  pausedAll: boolean;
  parallelDownloads: number;
  perHostCap: number;
}

/** Mirror of `services::queue::events::QueueUpdate`. */
export interface QueueUpdate {
  seq: number;
  items: QueueItem[];
  removed: string[];
  order: string[] | null;
  /** M3: the full group list when it changed, otherwise `null`. */
  groups: QueueGroup[] | null;
  summary: QueueSummary;
  toast: QueueToast | null;
}

/** Mirror of `services::queue::events::QueueSnapshot` (`queue_list`). */
export interface QueueSnapshot {
  seq: number;
  items: QueueItem[];
  groups: QueueGroup[];
  summary: QueueSummary;
  toast: QueueToast | null;
}

export interface QueueGroup {
  id: string;
  title: string;
  folder: string | null;
  index: number;
}

export interface QueueToast {
  messageKey: string;
  params: Record<string, string>;
}

/** Everything the queue UI shows, as one value. */
export interface QueueState {
  items: Record<string, QueueItem>;
  order: string[];
  groups: QueueGroup[];
  summary: QueueSummary;
  /** The newest `seq` the UI has already shown. */
  seq: number;
  toast: QueueToast | null;
}

/** An empty queue, the state the UI starts from. */
export function emptyQueueState(): QueueState {
  return {
    items: {},
    order: [],
    groups: [],
    summary: {
      total: 0,
      queued: 0,
      running: 0,
      paused: 0,
      interrupted: 0,
      done: 0,
      failed: 0,
      cancelled: 0,
      speedBps: null,
      pausedAll: false,
      parallelDownloads: 2,
      perHostCap: 2,
    },
    seq: 0,
    toast: null,
  };
}

/** What `queue_add` answers (spec §7.3 duplicate detection). */
export interface AddResult {
  added: string[];
  duplicates: { url: string; itemId: string; title: string | null }[];
}

/** What the Add bar sends for one link. */
export interface AddItemRequest {
  url: string;
  title: string | null;
  site: string | null;
  durationSec: number | null;
  kind: "video" | "audio";
  quality: QualityChoice;
  outputDir: string | null;
  groupId?: string | null;
  /** Sub-folder under the output folder (playlist title), sanitised by the backend. */
  outputSubdir?: string | null;
  /** M3 per-item overrides; omitted means "use the settings". */
  filenamePrefix?: string | null;
  includeIdInFilename?: boolean | null;
  preferH264?: boolean | null;
  useArchive?: boolean | null;
  sleepInterval?: [number, number] | null;
  liveFromStart?: boolean | null;
  referer?: string | null;
  userAgent?: string | null;
  customHeaders?: string[] | null;
  encode?: boolean | null;
  videoCodec?: string | null;
  audioCodec?: string | null;
  trimStart?: string | null;
  trimEnd?: string | null;
  keepOriginal?: boolean | null;
  autoPlay?: boolean | null;
  embedMetadata?: boolean | null;
  embedThumbnail?: boolean | null;
  embedChapters?: boolean | null;
  sponsorblockMode?: "mark" | "remove" | "off" | null;
  sponsorblockCategories?: string[] | null;
  transcribe?: boolean | null;
  whisperModel?: string | null;
  transcriptLanguage?: string | null;
  translateToEnglish?: boolean | null;
  transcriptsSubfolder?: boolean | null;
}

// ---------------------------------------------------------------------------
// Normalisers
// ---------------------------------------------------------------------------

const STATES: readonly ItemState[] = [
  "queued",
  "inspecting",
  "downloading",
  "postProcessing",
  "encoding",
  "transcribing",
  "summarizing",
  "done",
  "failed",
  "cancelled",
  "paused",
  "interrupted",
];

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function asNullableString(value: unknown): string | null {
  return typeof value === "string" && value !== "" ? value : null;
}

/** An unknown state is never trusted; `interrupted` is the honest landing. */
export function normaliseState(raw: unknown): ItemState {
  return STATES.includes(raw as ItemState) ? (raw as ItemState) : "interrupted";
}

export function normaliseQuality(raw: unknown): QualityChoice {
  const value = (raw ?? {}) as Record<string, unknown>;
  const height = asNumber(value.height);
  switch (value.kind) {
    case "exact":
      return { kind: "exact", height: height ?? 1080 };
    case "maxHeight":
      return { kind: "maxHeight", height: height ?? 1080 };
    case "audioOnly":
      return { kind: "audioOnly" };
    case "explicit":
      return {
        kind: "explicit",
        formatId: asString(value.formatId),
        hasAudio: value.hasAudio === true,
      };
    default:
      return { kind: "best" };
  }
}

export function normaliseItem(raw: unknown): QueueItem {
  const value = (raw ?? {}) as Record<string, unknown>;
  const progress = (value.progress ?? {}) as Record<string, unknown>;
  const options = (value.options ?? {}) as Record<string, unknown>;
  const cookies = (options.cookies ?? { kind: "none" }) as Record<string, unknown>;
  const error = value.error as Record<string, unknown> | null | undefined;
  return {
    id: asString(value.id),
    url: asString(value.url),
    title: asNullableString(value.title),
    site: asNullableString(value.site),
    durationSec: asNumber(value.durationSec),
    state: normaliseState(value.state),
    options: {
      kind: options.kind === "audio" ? "audio" : "video",
      quality: normaliseQuality(options.quality),
      outputDir: asString(options.outputDir),
      cookies:
        cookies.kind === "browser"
          ? { kind: "browser", browser: asString(cookies.browser) }
          : cookies.kind === "file"
            ? { kind: "file", file: asString(cookies.file) }
            : { kind: "none" },
      runtimeMode:
        options.runtimeMode === "quickJs" || options.runtimeMode === "deno"
          ? options.runtimeMode
          : "auto",
      filenamePrefix: asNullableString(options.filenamePrefix),
      includeIdInFilename: options.includeIdInFilename !== false,
      preferH264: options.preferH264 === true,
      archiveFile: asNullableString(options.archiveFile),
      sleepInterval:
        Array.isArray(options.sleepInterval) &&
        options.sleepInterval.length === 2 &&
        options.sleepInterval.every((n) => typeof n === "number")
          ? [options.sleepInterval[0] as number, options.sleepInterval[1] as number]
          : null,
      liveFromStart: typeof options.liveFromStart === "boolean" ? options.liveFromStart : null,
      proxy: asNullableString(options.proxy),
      limitRate: asNullableString(options.limitRate),
      referer: asNullableString(options.referer),
      userAgent: asNullableString(options.userAgent),
      customHeaders: Array.isArray(options.customHeaders)
        ? options.customHeaders.filter((h): h is string => typeof h === "string")
        : [],
      impersonateRetry: options.impersonateRetry !== false,
      encode: options.encode === true,
      videoCodec: asNullableString(options.videoCodec),
      audioCodec: asNullableString(options.audioCodec),
      trimStart: asNullableString(options.trimStart),
      trimEnd: asNullableString(options.trimEnd),
      keepOriginal: options.keepOriginal === true,
      autoPlay: options.autoPlay === true,
      embedMetadata: options.embedMetadata === true,
      embedThumbnail: options.embedThumbnail === true,
      embedChapters: options.embedChapters === true,
      sponsorblockMode:
        options.sponsorblockMode === "mark" || options.sponsorblockMode === "remove"
          ? options.sponsorblockMode
          : null,
      sponsorblockCategories: Array.isArray(options.sponsorblockCategories)
        ? options.sponsorblockCategories.filter((c): c is string => typeof c === "string")
        : [],
      transcribe: options.transcribe === true,
      whisperModel: asNullableString(options.whisperModel),
      transcriptLanguage: asNullableString(options.transcriptLanguage),
      translateToEnglish: typeof options.translateToEnglish === "boolean" ? options.translateToEnglish : null,
      transcriptsSubfolder: typeof options.transcriptsSubfolder === "boolean" ? options.transcriptsSubfolder : null,
    },
    groupId: asNullableString(value.groupId),
    progress: {
      percent: asNumber(progress.percent),
      speedBps: asNumber(progress.speedBps),
      etaSec: asNumber(progress.etaSec),
      label: asString(progress.label),
      subIndex: asNumber(progress.subIndex),
      subTotal: asNumber(progress.subTotal),
    },
    file: asNullableString(value.file),
    sizeBytes: asNumber(value.sizeBytes),
    runtime: asNullableString(value.runtime),
    requestedHeight: asNumber(value.requestedHeight),
    actualHeight: asNumber(value.actualHeight),
    error:
      error && typeof error === "object"
        ? {
            code: asString(error.code) || "UNKNOWN",
            messageKey: asString(error.messageKey) || "error.ytdlp.unknown",
            detail: typeof error.detail === "string" ? error.detail : null,
          }
        : null,
    logTail: Array.isArray(value.logTail)
      ? value.logTail.filter((line): line is string => typeof line === "string")
      : [],
    autoRetries: asNumber(value.autoRetries) ?? 0,
    retryAt: asNumber(value.retryAt),
    pausedFrom: value.pausedFrom ? normaliseState(value.pausedFrom) : null,
    addedAt: asNumber(value.addedAt) ?? 0,
    updatedAt: asNumber(value.updatedAt) ?? 0,
    startedAt: asNumber(value.startedAt),
    finishedAt: asNumber(value.finishedAt),
    transcriptFiles: Array.isArray(value.transcriptFiles)
      ? value.transcriptFiles.filter((f): f is string => typeof f === "string")
      : [],
    transcriptPreview: asNullableString(value.transcriptPreview),
  };
}

export function normaliseSummary(raw: unknown): QueueSummary {
  const value = (raw ?? {}) as Record<string, unknown>;
  return {
    total: asNumber(value.total) ?? 0,
    queued: asNumber(value.queued) ?? 0,
    running: asNumber(value.running) ?? 0,
    paused: asNumber(value.paused) ?? 0,
    interrupted: asNumber(value.interrupted) ?? 0,
    done: asNumber(value.done) ?? 0,
    failed: asNumber(value.failed) ?? 0,
    cancelled: asNumber(value.cancelled) ?? 0,
    speedBps: asNumber(value.speedBps),
    pausedAll: value.pausedAll === true,
    parallelDownloads: asNumber(value.parallelDownloads) ?? 2,
    perHostCap: asNumber(value.perHostCap) ?? 2,
  };
}

export function normaliseUpdate(raw: unknown): QueueUpdate {
  const value = (raw ?? {}) as Record<string, unknown>;
  return {
    seq: asNumber(value.seq) ?? 0,
    items: Array.isArray(value.items) ? value.items.map(normaliseItem) : [],
    removed: Array.isArray(value.removed) ? value.removed.filter((id): id is string => typeof id === "string") : [],
    order: Array.isArray(value.order) ? value.order.filter((id): id is string => typeof id === "string") : null,
    groups: Array.isArray(value.groups) ? value.groups.map(normaliseGroup) : null,
    summary: normaliseSummary(value.summary),
    toast: normaliseToast(value.toast),
  };
}

export function normaliseSnapshot(raw: unknown): QueueSnapshot {
  const value = (raw ?? {}) as Record<string, unknown>;
  return {
    seq: asNumber(value.seq) ?? 0,
    items: Array.isArray(value.items) ? value.items.map(normaliseItem) : [],
    groups: Array.isArray(value.groups) ? value.groups.map(normaliseGroup) : [],
    summary: normaliseSummary(value.summary),
    toast: normaliseToast(value.toast),
  };
}

export function normaliseGroup(group: unknown): QueueGroup {
  const entry = (group ?? {}) as Record<string, unknown>;
  return {
    id: asString(entry.id),
    title: asString(entry.title),
    folder: asNullableString(entry.folder),
    index: asNumber(entry.index) ?? 0,
  };
}

function normaliseToast(raw: unknown): QueueToast | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  const key = asString(value.messageKey);
  if (key === "") return null;
  const params = (value.params ?? {}) as Record<string, unknown>;
  const entries: Record<string, string> = {};
  for (const [name, entry] of Object.entries(params)) {
    entries[name] = String(entry);
  }
  return { messageKey: key, params: entries };
}

export function normaliseAddResult(raw: unknown): AddResult {
  const value = (raw ?? {}) as Record<string, unknown>;
  return {
    added: Array.isArray(value.added) ? value.added.filter((id): id is string => typeof id === "string") : [],
    duplicates: Array.isArray(value.duplicates)
      ? value.duplicates.map((entry) => {
          const duplicate = (entry ?? {}) as Record<string, unknown>;
          return {
            url: asString(duplicate.url),
            itemId: asString(duplicate.itemId),
            title: asNullableString(duplicate.title),
          };
        })
      : [],
  };
}

// ---------------------------------------------------------------------------
// The reducer
// ---------------------------------------------------------------------------

/**
 * Fold one `queue:update` event into what the UI shows (spec §7.6, §11.7).
 *
 * Pure, and kept here rather than in the store, so it can be tested without a
 * Svelte compiler: the store is a thin shell that holds the value and hands it
 * back out. Two rules it exists to keep:
 *
 * 1. **Only the rows that changed get a new object.** Every other item keeps
 *    its identity, so a tick that touches one item repaints one row and not the
 *    list (spec §11.7).
 * 2. **An event that is not newer is dropped.** `seq` is the backend's
 *    counter; the window between the initial `queue_list` and the first event
 *    can deliver them out of order.
 */
export function applyUpdate(state: QueueState, update: QueueUpdate): QueueState {
  if (update.seq !== 0 && update.seq <= state.seq) return state;

  let items = state.items;
  let order = state.order;
  const next: QueueState = {
    ...state,
    seq: update.seq === 0 ? state.seq : update.seq,
    summary: update.summary,
    toast: update.toast === null ? state.toast : update.toast,
  };

  if (update.items.length > 0) {
    items = { ...items };
    const known = new Set(order);
    const fresh: string[] = [];
    for (const item of update.items) {
      items[item.id] = item;
      // An item the UI has never seen arrives without an `order` event on a
      // restart (the list came from `queue_list`), so only add what is missing.
      if (!known.has(item.id)) fresh.push(item.id);
    }
    if (fresh.length > 0) order = [...order, ...fresh];
  }

  if (update.removed.length > 0) {
    const gone = new Set(update.removed);
    const kept: Record<string, QueueItem> = {};
    for (const [id, item] of Object.entries(items)) {
      if (!gone.has(id)) kept[id] = item;
    }
    items = kept;
    order = order.filter((id) => !gone.has(id));
  }

  if (update.order !== null) {
    order = update.order.filter((id) => items[id] !== undefined);
  }

  next.items = items;
  next.order = order;
  if (update.groups !== null) next.groups = update.groups;
  return next;
}

/**
 * Fold the initial `queue_list` snapshot in.
 *
 * `stale` is what the store knows: a `queue_list` that was in flight while
 * events arrived describes a queue that has already moved on, so applying it
 * wholesale would put a row back to `queued` and leave it there — the next
 * event may be a long way off, or never come if the queue went idle. In that
 * case only the rows the UI is missing are taken from it, and the newer
 * summary and sequence are kept.
 */
export function applySnapshot(state: QueueState, snapshot: QueueSnapshot, stale = false): QueueState {
  const byId: Record<string, QueueItem> = {};
  for (const item of snapshot.items) byId[item.id] = item;

  // Groups also travel in events (M3), but the snapshot is the full list.
  const groups = snapshot.groups;

  if (!stale) {
    return {
      items: byId,
      order: snapshot.items.map((item) => item.id),
      groups,
      summary: snapshot.summary,
      seq: snapshot.seq,
      toast: snapshot.toast === null ? state.toast : snapshot.toast,
    };
  }

  const items = { ...state.items };
  const fresh: string[] = [];
  for (const [id, item] of Object.entries(byId)) {
    if (items[id] === undefined) {
      items[id] = item;
      fresh.push(id);
    }
  }
  return {
    items,
    order: fresh.length > 0 ? [...state.order, ...fresh] : state.order,
    groups,
    summary: state.summary,
    seq: state.seq,
    toast: snapshot.toast === null ? state.toast : snapshot.toast,
  };
}

// ---------------------------------------------------------------------------
// States, actions, i18n keys
// ---------------------------------------------------------------------------

/** Is a tool running for this item right now? Mirrors the backend. */
export function isActive(state: ItemState): boolean {
  return (
    state === "inspecting" ||
    state === "downloading" ||
    state === "postProcessing" ||
    state === "encoding" ||
    state === "transcribing" ||
    state === "summarizing"
  );
}

/** Terminal: the item will not move again on its own. */
export function isTerminal(state: ItemState): boolean {
  return state === "done" || state === "failed" || state === "cancelled";
}

/** Parked: terminal, paused or interrupted — waiting for a decision. */
export function isParked(state: ItemState): boolean {
  return isTerminal(state) || state === "paused" || state === "interrupted";
}

/** The stage chip's i18n key (spec §11.5). */
export function stateKey(state: ItemState): string {
  return `queue.state.${state}`;
}

/** Every action an item offers, in the order the card shows them. */
export type ItemAction =
  | "pause"
  | "resume"
  | "cancel"
  | "retry"
  | "reveal"
  | "open"
  | "remove"
  | "moveToTop"
  | "copyLink"
  | "openPage"
  | "transcribe";

/**
 * The actions available for one state (spec §11.5 "Primary actions by
 * state").
 *
 * Cancel is offered whenever something is running, Resume only for what can
 * actually resume, and Retry for the parked failures a user can re-run — never
 * for a done item, which has nothing to retry.
 */
export function actionsFor(
  state: ItemState,
  hasFile: boolean,
  isRemoteUrl: boolean = true,
): ItemAction[] {
  const actions: ItemAction[] = [];
  if (isActive(state)) {
    actions.push("pause", "cancel");
  } else if (state === "paused" || state === "interrupted") {
    actions.push("resume", "cancel", "remove");
  } else if (state === "queued") {
    actions.push("cancel", "remove", "moveToTop");
  } else if (state === "failed" || state === "cancelled") {
    actions.push("retry", "remove", "moveToTop");
  } else if (state === "done") {
    actions.push("remove", "moveToTop");
  }
  if (hasFile && !isActive(state)) {
    actions.push("open", "reveal", "transcribe");
  } else if (hasFile) {
    actions.push("open", "reveal");
  }
  if (isRemoteUrl) {
    actions.push("copyLink", "openPage");
  }
  return actions;
}

/** i18n key of an action button. */
export function actionKey(action: ItemAction): string {
  return `queue.item.action.${action}`;
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/** bytes/s → Mbit/s, the unit the original and spec §8.3 use. */
export function speedMbits(bytesPerSecond: number | null): number | null {
  if (bytesPerSecond === null || !Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) {
    return null;
  }
  const mbit = (bytesPerSecond * 8) / 1024 / 1024;
  // One decimal below 1 Mbit/s: `0` for a slow download reads as "stalled",
  // and the footer and the cards both show this number.
  return mbit >= 1 ? Math.round(mbit) : Math.round(mbit * 10) / 10;
}

/**
 * A file size as a short string: `512 B`, `2 kB`, `480 MB`, `1.4 GB`.
 *
 * Binary divisors, the ones yt-dlp itself prints. One decimal only for
 * gigabytes, where the difference matters; `null` in, `null` out, because an
 * item that produced nothing has no size to show.
 */
export function formatBytes(bytes: number | null): string | null {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return null;
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.round(kb)} kB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

/** Seconds → `h:mm:ss` / `m:ss`, or `null` when there is nothing to show. */
export function secondsToClock(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) return null;
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  const pad = (value: number): string => value.toString().padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(rest)}` : `${minutes}:${pad(rest)}`;
}

/** The last segment of a path, on either separator (spec §11.2 footers). */
export function fileName(path: string): string {
  const trimmed = path.trim();
  const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return cut >= 0 ? trimmed.slice(cut + 1) : trimmed;
}

/** A short label for an item when there is no title (spec §11.5). */
export function itemLabel(item: QueueItem): string {
  if (item.title !== null && item.title.trim() !== "") return item.title;
  return fileName(item.url) || item.url;
}

/**
 * An i18n key plus the parameters it needs.
 *
 * Heights, stream indices and counts are parameters, never part of the key: a
 * video can report any height (`576`, `4320`, …), and a dictionary of fixed
 * keys would print the raw key for every one that is missing (D99).
 */
export interface Label {
  key: string;
  params?: Record<string, string | number>;
}

/** The label of a quality choice (spec §8.2, §11.9). */
export function qualityLabel(choice: QualityChoice): Label {
  switch (choice.kind) {
    case "exact":
      return { key: "queue.item.quality.exact", params: { height: choice.height } };
    case "maxHeight":
      return { key: "queue.item.quality.max", params: { height: choice.height } };
    case "audioOnly":
      return { key: "queue.item.quality.audio" };
    case "explicit":
      return { key: "queue.item.quality.explicit" };
    default:
      return { key: "queue.item.quality.best" };
  }
}

/** What the details panel shows as the chosen quality (spec §11.5). */
export function qualityKey(item: QueueItem): Label {
  return qualityLabel(item.options.quality);
}

/**
 * The i18n key of a stream/step token the backend reports (spec §8.3, §11.5).
 *
 * Tokens are stable (`video`, `audio`, `merging`, …) and never sentences, so
 * word order and plurals stay here, per language (spec §11.9).
 */
export function progressLabelKey(label: string): string {
  switch (label) {
    case "preparing":
    case "verifying":
      return "job.label.preparing";
    case "video":
      return "job.label.video";
    case "audio":
      return "job.label.audio";
    case "merging":
      return "job.label.merging";
    case "moving":
      return "job.label.moving";
    case "extractingAudio":
      return "job.label.extractingAudio";
    case "embedding":
      return "job.label.embedding";
    case "encoding":
      return "job.label.encoding";
    case "waitingEncode":
      return "job.label.waitingEncode";
    case "transcribing":
      return "job.label.transcribing";
    case "waitingTranscribe":
      return "job.label.waitingTranscribe";
    case "postProcessing":
      return "job.label.postProcessing";
    default:
      return "job.label.postProcessing";
  }
}

/**
 * The finer "which stream" detail of a running item: "Video 1/2", "Merging…"
 * (spec §11.5, the *stage detail* next to the stage chip).
 *
 * `null` when the backend has not said anything yet: the card shows its status
 * line regardless, and an empty stage must not become "Video 1/1".
 */
export function stageDetail(item: QueueItem): Label | null {
  const { label, subIndex, subTotal } = item.progress;
  if (label === "") return null;
  if (label === "video" || label === "audio") {
    return {
      key: progressLabelKey(label),
      params: { index: subIndex ?? 1, total: subTotal ?? 1 },
    };
  }
  return { key: progressLabelKey(label) };
}

/**
 * Assemble the single progress line for a running queue item (spec §11.5, §11.7).
 *
 * Cards keep a constant height across their lifecycle. While transcribing,
 * a live segment preview folds into this single-line slot instead of adding
 * a secondary row that would break the virtualizer's row height.
 */
export function progressLine(
  stageText: string | null,
  speedText: string | null,
  etaText: string | null,
  options?: {
    isTranscribing?: boolean;
    transcriptPreview?: string | null;
    waitingDataText?: string;
  },
): string {
  if (options?.isTranscribing && options?.transcriptPreview) {
    return stageText !== null
      ? `${stageText} · ${options.transcriptPreview}`
      : options.transcriptPreview;
  }
  const parts = [stageText, speedText, etaText].filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(" · ") : (options?.waitingDataText ?? "");
}

/**
 * What a card's status line says (spec §11.5): one line for every state.
 *
 * Pinned as a function because the line is always rendered — the card is not
 * allowed to change height when a value appears or disappears, which is both
 * the flicker the user saw and what keeps the virtualiser's row height honest.
 */
export type StatusKind = "error" | "warning" | "progress" | "quality";

/** The kind of status line an item wants: the problem, the progress, or the quality. */
export function statusKindFor(item: QueueItem): StatusKind {
  if (item.error !== null) return "error";
  if (heightWarning(item) !== null) return "warning";
  return isActive(item.state) ? "progress" : "quality";
}

/**
 * The width of the progress slot, in per cent: what yt-dlp last reported, 100
 * for a finished item, 0 when nothing is known.
 *
 * Never `null`: the slot is always there. `percent === null` only decides
 * whether it animates (spec §8.3).
 */
export function barPercentFor(item: QueueItem): number {
  if (item.progress.percent !== null) return item.progress.percent;
  return item.state === "done" ? 100 : 0;
}

/**
 * Where to scroll so the row at `index` is fully in view, or `null` when it
 * already is.
 *
 * "Nearest" semantics, computed from the virtualiser's own numbers instead of
 * the DOM: the row being scrolled to is usually **not rendered yet**, so there
 * is no element to ask. `-1` (an item that rolled off the history cap) is
 * "nowhere", not "row zero".
 */
export function scrollOffsetFor(
  index: number,
  pitch: number,
  scrollTop: number,
  viewport: number,
): number | null {
  if (index < 0 || pitch <= 0) return null;
  const top = index * pitch;
  const bottom = top + pitch;
  if (top >= scrollTop && bottom <= scrollTop + viewport) return null;
  return Math.max(0, top < scrollTop ? top : bottom - viewport);
}

/** A finished item whose resolution is lower than requested (spec §11.5). */
export function heightWarning(
  item: QueueItem,
): { requested: number; actual: number } | null {
  if (item.state !== "done") return null;
  const { requestedHeight, actualHeight } = item;
  if (requestedHeight === null || actualHeight === null || actualHeight >= requestedHeight) {
    return null;
  }
  return { requested: requestedHeight, actual: actualHeight };
}

/** The "Copy details" text: the localised message plus the raw detail. */
export function detailsText(item: QueueItem, message: string | null): string {
  const lines: string[] = [];
  if (message !== null && message !== "") lines.push(message);
  if (item.error?.detail) lines.push(item.error.detail);
  if (item.logTail.length > 0) lines.push(...item.logTail);
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Virtualisation (spec §11.7)
// ---------------------------------------------------------------------------

export interface VisibleRange {
  first: number;
  last: number;
  /** Pixels of empty space above and below, to keep the scrollbar honest. */
  topPad: number;
  bottomPad: number;
}

/**
 * Which rows a virtualised list has to render.
 *
 * Pure arithmetic so the "5 000 items stay usable" requirement of spec §11.7
 * is a test and not a hope: `overscan` rows are rendered above and below the
 * viewport, and the two paddings stand in for everything that is not rendered.
 */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  count: number,
  overscan = 2,
): VisibleRange {
  if (rowHeight <= 0 || count <= 0) {
    return { first: 0, last: -1, topPad: 0, bottomPad: 0 };
  }
  const firstVisible = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visibleRows = Math.ceil(Math.max(0, viewportHeight) / rowHeight) + 1;
  const first = Math.max(0, firstVisible - overscan);
  const last = Math.min(count - 1, firstVisible + visibleRows + overscan - 1);
  return {
    first,
    last,
    topPad: first * rowHeight,
    bottomPad: Math.max(0, (count - 1 - last) * rowHeight),
  };
}

/** The slice of `items` a virtualised list renders right now. */
export function visibleItems<T>(items: readonly T[], range: VisibleRange): T[] {
  if (range.last < range.first) return [];
  return items.slice(range.first, range.last + 1);
}
// ---------------------------------------------------------------------------
// Playlist groups (M3, spec §6.4, §11.5)
// ---------------------------------------------------------------------------

/** What a group header shows and which of its actions make sense. */
export interface GroupStats {
  total: number;
  /** Finished successfully, archive skips included. */
  done: number;
  failed: number;
  /** Running, queued or waiting to retry: the group still has work to do. */
  active: number;
  paused: number;
  /** A folder to open: the directory of the first finished file, if any. */
  firstFile: string | null;
}

/** Fold the items of one group into the numbers of its header. */
export function groupStats(groupId: string, items: readonly QueueItem[]): GroupStats {
  const stats: GroupStats = { total: 0, done: 0, failed: 0, active: 0, paused: 0, firstFile: null };
  for (const item of items) {
    if (item.groupId !== groupId) continue;
    stats.total += 1;
    if (item.state === "done") {
      stats.done += 1;
      if (stats.firstFile === null && item.file !== null) stats.firstFile = item.file;
    } else if (item.state === "failed" || item.state === "cancelled") {
      stats.failed += 1;
    } else if (item.state === "paused" || item.state === "interrupted") {
      stats.paused += 1;
    } else {
      stats.active += 1;
    }
  }
  return stats;
}

/** The groups that still have items, in the order they were created. */
export function nonEmptyGroups(
  groups: readonly QueueGroup[],
  items: readonly QueueItem[],
): QueueGroup[] {
  const used = new Set(items.map((item) => item.groupId).filter((id): id is string => id !== null));
  return groups.filter((group) => used.has(group.id)).sort((a, b) => a.index - b.index);
}

/** Items to list: those of a collapsed group are hidden, the rest keep their order. */
export function itemsOutsideCollapsed(
  items: readonly QueueItem[],
  collapsed: ReadonlySet<string>,
): QueueItem[] {
  if (collapsed.size === 0) return [...items];
  return items.filter((item) => item.groupId === null || !collapsed.has(item.groupId));
}

/** Group progress, 0–100. An empty group is 0, never `NaN`. */
export function groupPercent(stats: GroupStats): number {
  return stats.total === 0 ? 0 : Math.round((stats.done / stats.total) * 100);
}

/** An item finished because the archive already recorded it (spec §6.4). */
export function wasSkippedByArchive(item: QueueItem): boolean {
  return item.state === "done" && item.progress.label === "skippedArchive";
}
