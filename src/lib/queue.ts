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
    summary: normaliseSummary(value.summary),
    toast: normaliseToast(value.toast),
  };
}

export function normaliseSnapshot(raw: unknown): QueueSnapshot {
  const value = (raw ?? {}) as Record<string, unknown>;
  return {
    seq: asNumber(value.seq) ?? 0,
    items: Array.isArray(value.items) ? value.items.map(normaliseItem) : [],
    groups: Array.isArray(value.groups)
      ? value.groups.map((group) => {
          const entry = (group ?? {}) as Record<string, unknown>;
          return {
            id: asString(entry.id),
            title: asString(entry.title),
            folder: asNullableString(entry.folder),
            index: asNumber(entry.index) ?? 0,
          };
        })
      : [],
    summary: normaliseSummary(value.summary),
    toast: normaliseToast(value.toast),
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

  // `groups` travel only in the snapshot, never in an event.
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
  | "openPage";

/**
 * The actions available for one state (spec §11.5 "Primary actions by
 * state").
 *
 * Cancel is offered whenever something is running, Resume only for what can
 * actually resume, and Retry for the parked failures a user can re-run — never
 * for a done item, which has nothing to retry.
 */
export function actionsFor(state: ItemState, hasFile: boolean): ItemAction[] {
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
  if (hasFile) {
    actions.push("open", "reveal");
  }
  actions.push("copyLink", "openPage");
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
 * Heights are parameters, never part of the key: a video can report any height
 * (`576`, `4320`, …), and a dictionary of fixed keys would print the raw key
 * for every one that is missing (D98).
 */
export interface QualityLabel {
  key: string;
  params?: Record<string, string | number>;
}

/** The label of a quality choice (spec §8.2, §11.9). */
export function qualityLabel(choice: QualityChoice): QualityLabel {
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
export function qualityKey(item: QueueItem): QualityLabel {
  return qualityLabel(item.options.quality);
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