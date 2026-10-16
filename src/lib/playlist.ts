/**
 * Pure helpers for the playlist selection panel (spec §6.4, §11.4).
 *
 * Everything that decides *which entries are selected* and *what the queue
 * receives* lives here, free of Svelte runes and of any `@tauri-apps` import,
 * so each rule is a unit test (AGENTS.md §6) and the panel stays declarative.
 *
 * Selection is a set of **positions** (`EntryLite.index`, 1-based, the number
 * the entry has in the full playlist). Positions, not array offsets: a filter or
 * a reversed order changes what is on screen, never which video a number means,
 * and the numbering prefix of the filenames follows the position.
 */
import type { QualityChoice, AddItemRequest } from "$lib/queue";
import {
  CHANNEL_DEFAULT_CAP,
  CHANNEL_WARN_THRESHOLD,
  MIX_DEFAULT_CAP,
  type EntryLite,
  type PlaylistInfo,
} from "$lib/inspect";

/** Above this many selected entries the backend paces its requests (spec §6.4). */
export const PLAYLIST_PACING_THRESHOLD = 20;

/** Default pacing interval `(min, max)` seconds for big playlists (spec §6.4). */
export const DEFAULT_SLEEP_INTERVAL: [number, number] = [1, 4];

/** Longest folder name the app creates (spec §6.4: "max ~100 chars"). */
export const FOLDER_NAME_MAX = 100;

// ---------------------------------------------------------------------------
// Range input: "1-10, 15, 20-"
// ---------------------------------------------------------------------------

/**
 * Parse a range expression into sorted, de-duplicated positions within
 * `1..=max`.
 *
 * Grammar: comma- or space-separated terms, each `N`, `A-B`, `A-` (to the end)
 * or `-B` (from the start). Anything else makes the whole expression invalid
 * (`null`): a typo must not silently select half of what was meant.
 * Positions outside `1..=max` are clipped, and a reversed pair (`10-3`) is read
 * as `3-10`.
 */
export function parseRange(text: string, max: number): number[] | null {
  const trimmed = text.trim();
  if (trimmed === "" || max < 1) return null;
  const picked = new Set<number>();
  for (const term of trimmed.split(/[\s,;]+/)) {
    if (term === "") continue;
    const match = /^(\d*)(-?)(\d*)$/.exec(term);
    if (match === null) return null;
    const [, left, dash, right] = match;
    if (left === "" && right === "") return null;
    let from: number;
    let to: number;
    if (dash === "") {
      // A bare number; `right` is always empty here because the digits all
      // matched the first group.
      from = to = Number(left);
    } else {
      from = left === "" ? 1 : Number(left);
      to = right === "" ? max : Number(right);
    }
    if (!Number.isFinite(from) || !Number.isFinite(to)) return null;
    if (from > to) [from, to] = [to, from];
    for (let n = Math.max(1, from); n <= Math.min(max, to); n += 1) picked.add(n);
  }
  return [...picked].sort((a, b) => a - b);
}

// ---------------------------------------------------------------------------
// Filtering and selection
// ---------------------------------------------------------------------------

/**
 * Case-insensitive text filter over titles. Accents are folded so that
 * `perche` finds `Perché`; an empty query keeps everything.
 */
export function filterEntries(entries: readonly EntryLite[], query: string): EntryLite[] {
  const needle = fold(query.trim());
  if (needle === "") return [...entries];
  return entries.filter((entry) => fold(entry.title).includes(needle));
}

function fold(text: string): string {
  return text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
}

/** Entries the user can actually pick (private/deleted ones are disabled). */
export function selectable(entries: readonly EntryLite[]): EntryLite[] {
  return entries.filter((entry) => entry.available);
}

/** Select every available entry of `visible` (the filtered view). */
export function selectAll(current: ReadonlySet<number>, visible: readonly EntryLite[]): Set<number> {
  const next = new Set(current);
  for (const entry of selectable(visible)) next.add(entry.index);
  return next;
}

/** Deselect every entry of `visible`. */
export function selectNone(current: ReadonlySet<number>, visible: readonly EntryLite[]): Set<number> {
  const next = new Set(current);
  for (const entry of visible) next.delete(entry.index);
  return next;
}

/** Invert the selection over the available entries of `visible`. */
export function invertSelection(
  current: ReadonlySet<number>,
  visible: readonly EntryLite[],
): Set<number> {
  const next = new Set(current);
  for (const entry of selectable(visible)) {
    if (next.has(entry.index)) next.delete(entry.index);
    else next.add(entry.index);
  }
  return next;
}

/** Replace the selection with `positions`, keeping only available entries. */
export function selectPositions(
  entries: readonly EntryLite[],
  positions: readonly number[],
): Set<number> {
  const wanted = new Set(positions);
  return new Set(selectable(entries).filter((e) => wanted.has(e.index)).map((e) => e.index));
}

/**
 * Shift-click: set every available entry between the anchor and the clicked
 * row (inclusive, in the order shown) to `checked`.
 */
export function selectBetween(
  current: ReadonlySet<number>,
  visible: readonly EntryLite[],
  anchor: number,
  target: number,
  checked: boolean,
): Set<number> {
  const a = visible.findIndex((entry) => entry.index === anchor);
  const b = visible.findIndex((entry) => entry.index === target);
  if (a < 0 || b < 0) return new Set(current);
  const [from, to] = a <= b ? [a, b] : [b, a];
  const next = new Set(current);
  for (const entry of visible.slice(from, to + 1)) {
    if (!entry.available) continue;
    if (checked) next.add(entry.index);
    else next.delete(entry.index);
  }
  return next;
}

/**
 * The default selection of a freshly opened playlist (spec §6.4):
 * every available entry, except that a Mix keeps the first 50 and a huge
 * channel tab the first 100 — a click away from "Select all", never silently
 * thousands of downloads.
 */
export function defaultSelection(playlist: PlaylistInfo): Set<number> {
  const available = selectable(playlist.entries).sort((a, b) => a.index - b.index);
  let cap = Number.POSITIVE_INFINITY;
  if (playlist.playlistKind === "mix") cap = MIX_DEFAULT_CAP;
  else if (
    playlist.playlistKind === "channel" &&
    (playlist.count ?? playlist.entries.length) > CHANNEL_WARN_THRESHOLD
  ) {
    cap = CHANNEL_DEFAULT_CAP;
  }
  return new Set(available.slice(0, cap).map((entry) => entry.index));
}

/** Which warning banner the panel shows, as an i18n key, or `null`. */
export function playlistWarningKey(playlist: PlaylistInfo): string | null {
  if (playlist.playlistKind === "mix") return "playlist.warning.mix";
  if (
    playlist.playlistKind === "channel" &&
    (playlist.count ?? playlist.entries.length) > CHANNEL_WARN_THRESHOLD
  ) {
    return "playlist.warning.channel";
  }
  return null;
}

/** Sum of the known durations of `entries`, in seconds (`null` when none known). */
export function totalDuration(entries: readonly EntryLite[]): number | null {
  let total = 0;
  let known = false;
  for (const entry of entries) {
    if (entry.duration !== null) {
      total += entry.duration;
      known = true;
    }
  }
  return known ? total : null;
}

/** Entries in display order: playlist order, or reversed (spec §6.4). */
export function orderEntries(entries: readonly EntryLite[], reversed: boolean): EntryLite[] {
  const sorted = [...entries].sort((a, b) => a.index - b.index);
  return reversed ? sorted.reverse() : sorted;
}

/**
 * Merge a freshly loaded page into the entries already known ("Load more",
 * spec §6.4), keyed by position so a repeated page cannot duplicate rows.
 */
export function mergeEntries(
  known: readonly EntryLite[],
  page: readonly EntryLite[],
): EntryLite[] {
  const byIndex = new Map<number, EntryLite>();
  for (const entry of known) byIndex.set(entry.index, entry);
  for (const entry of page) byIndex.set(entry.index, entry);
  return [...byIndex.values()].sort((a, b) => a.index - b.index);
}

/** The `(from, to)` item range of the next "Load more" page (1-based, inclusive). */
export function nextPageRange(loaded: number, pageSize: number): [number, number] {
  return [loaded + 1, loaded + pageSize];
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

const RESERVED_NAMES = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;

/**
 * A folder name for a playlist title (spec §6.4): illegal characters replaced,
 * trailing dots/spaces trimmed, reserved Windows names escaped, at most 100
 * characters. The backend sanitises again (`output_subdir`); this copy only
 * lets the panel *show* the folder that will be created.
 */
export function sanitizeFolderName(title: string, fallback = "Playlist"): string {
  let name = title
    // eslint-disable-next-line no-control-regex
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .trim();
  name = Array.from(name).slice(0, FOLDER_NAME_MAX).join("");
  name = name.replace(/[. ]+$/, "");
  if (name === "") name = fallback;
  if (RESERVED_NAMES.test(name)) name = `_${name}`;
  return name;
}

/**
 * The numbering prefix of the filename: `001 - ` (spec §6.4). The width is that
 * of the playlist size, never less than 3. Mirrors `format_playlist_prefix` in
 * Rust; the literal `%` escaping is the backend's job.
 */
export function formatPlaylistPrefix(position: number, total: number): string {
  const width = Math.max(3, String(Math.max(1, total)).length);
  return `${String(position).padStart(width, "0")} - `;
}

// ---------------------------------------------------------------------------
// What the queue receives
// ---------------------------------------------------------------------------

export interface PlaylistAddOptions {
  /** One queue group per playlist, created by the caller. */
  groupId: string;
  quality: QualityChoice;
  /** Parent folder; `null` means the settings' default. */
  outputDir: string | null;
  /** Put the entries in `<out>/<playlist title>/` (default on). */
  folderPerPlaylist: boolean;
  /** Prefix `001 - ` to the file names (default on, so order survives). */
  numbering: boolean;
  /** Skip what the download archive already records (default on). */
  useArchive: boolean;
}

/**
 * One `queue_add` request per selected entry (spec §6.4 "Enqueuing"): explicit
 * per-entry URLs, not a playlist process, so each video has its own progress,
 * retry and cancel, and a private one never aborts the rest.
 *
 * The numbering uses the entry's **position in the playlist**, and the total is
 * the playlist size (or the highest position seen), so `001 - ` stays aligned
 * when only part of the list is added.
 */
export function buildPlaylistRequests(
  playlist: PlaylistInfo,
  selected: ReadonlySet<number>,
  options: PlaylistAddOptions,
): AddItemRequest[] {
  const chosen = selectable(playlist.entries)
    .filter((entry) => selected.has(entry.index))
    .sort((a, b) => a.index - b.index);
  const total = Math.max(
    playlist.count ?? 0,
    playlist.entries.reduce((max, entry) => Math.max(max, entry.index), 0),
  );
  const paced = chosen.length > PLAYLIST_PACING_THRESHOLD;
  const subdir = options.folderPerPlaylist ? sanitizeFolderName(playlist.title) : null;
  return chosen.map((entry) => ({
    url: entry.url,
    title: entry.title === "" ? null : entry.title,
    site: playlist.site === "" ? null : playlist.site,
    durationSec: entry.duration === null ? null : Math.round(entry.duration),
    kind: options.quality.kind === "audioOnly" ? ("audio" as const) : ("video" as const),
    quality: options.quality,
    outputDir: options.outputDir,
    outputSubdir: subdir,
    groupId: options.groupId,
    filenamePrefix: options.numbering ? formatPlaylistPrefix(entry.index, total) : null,
    useArchive: options.useArchive,
    sleepInterval: paced ? DEFAULT_SLEEP_INTERVAL : null,
  }));
}
