/**
 * Pure types and helpers for link inspection (spec §6.2).
 *
 * Mirrors the Rust `services::inspect` model. No Svelte runes and no
 * `@tauri-apps` imports, so every rule here is unit tested (AGENTS.md §6).
 */

import type { BackendErrorInfo } from "$lib/settings";

/** One format, with only the fields spec §6.2 asks for. */
export interface FormatLite {
  formatId: string;
  ext: string;
  height: number | null;
  width: number | null;
  fps: number | null;
  vcodec: string | null;
  acodec: string | null;
  filesize: number | null;
  protocol: string | null;
  note: string | null;
}

export interface VideoInfo {
  id: string;
  title: string;
  duration: number | null;
  /** `extractor_key`, shown as the site badge (spec §6.3.2). */
  site: string;
  webpageUrl: string | null;
  isLive: boolean;
  language: string | null;
  thumbnailUrl: string | null;
  formats: FormatLite[];
  /** Distinct heights >= 144, descending (spec §6.2). Empty is normal. */
  heights: number[];
  hasVideo: boolean;
  hasAudio: boolean;
  /**
   * The backend's verdict: even after its retries the extraction still looked
   * incomplete (yt-dlp's JS-challenge warning, or no usable height at all), so
   * the list may be missing resolutions. The UI says so and offers a retry.
   */
  degraded: boolean;
}

export type PlaylistKind = "playlist" | "channel" | "mix" | "other";

/** One playlist entry from `--flat-playlist` (spec §6.2, §6.4). */
export interface EntryLite {
  /** 1-based position in the full playlist (`1..=N`). */
  index: number;
  id: string;
  url: string;
  title: string;
  duration: number | null;
  /** `false` for `[Private video]`, `[Deleted video]`, or missing URLs. */
  available: boolean;
}

export interface PlaylistStub {
  id: string;
  title: string;
  site: string;
  webpageUrl: string | null;
  /** Named `playlistKind` because `kind` is the union discriminant. */
  playlistKind: PlaylistKind;
  count: number | null;
  entries: EntryLite[];
  truncated: boolean;
}

/** Alias used by the M3 playlist panel (`spec §6.2`). */
export type PlaylistInfo = PlaylistStub;

/** Optional per-inspection flags sent to `inspect_url` / `inspect_urls` (spec §6.2, §6.4). */
export interface InspectRequestOptions {
  itemRange?: [number, number] | null;
  noPlaylist?: boolean;
  referer?: string | null;
  userAgent?: string | null;
  customHeaders?: string[];
}

export type Inspected =
  | ({ kind: "video" } & VideoInfo)
  | ({ kind: "playlist" } & PlaylistStub)
  | ({ kind: "error" } & BackendErrorInfo);

/** Mirror of the Rust `commands::inspect::InspectView`. */
export interface InspectView {
  requestId: number;
  inspected: Inspected;
}

/** Spec §6.2: debounce 500 ms on the URL input, as the original did. */
export const INSPECT_DEBOUNCE_MS = 500;

/** Default page size for flat playlist inspection (`-I :1000`, spec §6.2). */
export const PLAYLIST_PAGE_SIZE = 1000;

/** Default cap for YouTube Mix/Radio playlists (`list=RD…`, spec §6.4). */
export const MIX_DEFAULT_CAP = 50;

/** Channel tab item count above which the UI warns and caps by default (spec §6.4). */
export const CHANNEL_WARN_THRESHOLD = 200;

/** Default selection cap when a channel tab exceeds `CHANNEL_WARN_THRESHOLD` (spec §6.4). */
export const CHANNEL_DEFAULT_CAP = 100;

/**
 * How long to wait before inspecting (D76).
 *
 * Typing keeps the 500 ms: without it every keystroke would launch a yt-dlp
 * process (0.46 s for the binary alone) and a network request. A **paste** is
 * one atomic event, not typing — nothing else is coming — so waiting there is
 * pure latency and the inspection starts at once.
 */
export function inspectDelay(pasted: boolean): number {
  return pasted ? 0 : INSPECT_DEBOUNCE_MS;
}

const PLAYLIST_KINDS: readonly PlaylistKind[] = ["playlist", "channel", "mix", "other"];

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function asNullableString(value: unknown): string | null {
  return typeof value === "string" && value !== "" ? value : null;
}

function asNullableNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function normaliseFormat(raw: unknown): FormatLite {
  const format = (raw ?? {}) as Partial<FormatLite>;
  return {
    formatId: asString(format.formatId),
    ext: asString(format.ext),
    height: asNullableNumber(format.height),
    width: asNullableNumber(format.width),
    fps: asNullableNumber(format.fps),
    vcodec: asNullableString(format.vcodec),
    acodec: asNullableString(format.acodec),
    filesize: asNullableNumber(format.filesize),
    protocol: asNullableString(format.protocol),
    note: asNullableString(format.note),
  };
}

function normaliseEntry(raw: unknown, fallbackIndex: number): EntryLite {
  const entry = (raw ?? {}) as Partial<EntryLite>;
  const index =
    typeof entry.index === "number" && Number.isFinite(entry.index) && entry.index >= 1
      ? Math.floor(entry.index)
      : fallbackIndex;
  return {
    index,
    id: asString(entry.id),
    url: asString(entry.url),
    title: asString(entry.title),
    duration: asNullableNumber(entry.duration),
    available: entry.available !== false,
  };
}

/** Coerce anything the backend sends into a complete, known-shaped result. */
export function normaliseInspected(raw: unknown): Inspected {
  const value = (raw ?? {}) as Record<string, unknown>;
  const kind = value.kind;

  if (kind === "video") {
    return {
      kind: "video",
      id: asString(value.id),
      title: asString(value.title),
      duration: asNullableNumber(value.duration),
      site: asString(value.site),
      webpageUrl: asNullableString(value.webpageUrl),
      isLive: value.isLive === true,
      language: asNullableString(value.language),
      thumbnailUrl: asNullableString(value.thumbnailUrl),
      formats: Array.isArray(value.formats) ? value.formats.map(normaliseFormat) : [],
      heights: Array.isArray(value.heights)
        ? value.heights.filter((h): h is number => typeof h === "number" && Number.isFinite(h))
        : [],
      hasVideo: value.hasVideo === true,
      hasAudio: value.hasAudio === true,
      degraded: value.degraded === true,
    };
  }

  if (kind === "playlist") {
    const entries = Array.isArray(value.entries)
      ? value.entries.map((item, idx) => normaliseEntry(item, idx + 1))
      : [];
    return {
      kind: "playlist",
      id: asString(value.id),
      title: asString(value.title),
      site: asString(value.site),
      webpageUrl: asNullableString(value.webpageUrl),
      playlistKind: PLAYLIST_KINDS.includes(value.playlistKind as PlaylistKind)
        ? (value.playlistKind as PlaylistKind)
        : "other",
      count: asNullableNumber(value.count),
      entries,
      truncated: value.truncated === true,
    };
  }

  return {
    kind: "error",
    code: asString((value as Partial<BackendErrorInfo>).code) || "UNKNOWN",
    messageKey: asString((value as Partial<BackendErrorInfo>).messageKey) || "error.ytdlp.unknown",
    detail: asNullableString((value as Partial<BackendErrorInfo>).detail),
  };
}

// ---------------------------------------------------------------------------
// URL-case classification helpers (spec §6.4), mirroring `services::inspect`
// ---------------------------------------------------------------------------

/**
 * `true` when `url` has both a single-video target (`watch?v=…` or `youtu.be/<id>`)
 * and a `list=…` query parameter (spec §6.4: inline choice *This video only* /
 * *Whole playlist*).
 */
export function isVideoWithListUrl(url: string): boolean {
  const lower = url.trim().toLowerCase();
  const hasList = lower.includes("?list=") || lower.includes("&list=");
  if (!hasList) return false;
  const hasWatchV =
    lower.includes("watch?v=") ||
    lower.includes("&v=") ||
    lower.includes("youtu.be/") ||
    lower.includes("/shorts/");
  return hasWatchV;
}

/** `true` when `url` is a YouTube Mix / Radio (`list=RD…`, spec §6.4). */
export function isMixUrl(url: string): boolean {
  const lower = url.trim().toLowerCase();
  return lower.includes("list=rd");
}

/**
 * `true` when `url` is a YouTube search/feed/home page (spec §6.4:
 * show clear unsupported message instead of trying to scrape).
 */
export function isSearchOrFeedUrl(url: string): boolean {
  const trimmed = url.trim();
  const afterScheme = trimmed.replace(/^https?:\/\//i, "");
  const slashIdx = afterScheme.indexOf("/");
  const host = (slashIdx === -1 ? afterScheme : afterScheme.slice(0, slashIdx)).toLowerCase();
  const rest = slashIdx === -1 ? "/" : afterScheme.slice(slashIdx);
  const isYoutubeHost =
    host === "youtube.com" ||
    host === "www.youtube.com" ||
    host === "m.youtube.com" ||
    host === "music.youtube.com";
  if (!isYoutubeHost) return false;
  const path = (rest.split(/[?#]/)[0] ?? "/").replace(/\/+$/, "").toLowerCase();
  return (
    path === "" ||
    path === "/results" ||
    path === "/feed" ||
    path.startsWith("/feed/")
  );
}

// ---------------------------------------------------------------------------
// Formatting helpers (the original showed "1.23 Mbps" and a clock, spec §8.3)
// ---------------------------------------------------------------------------

/** `213` → `3:33`, `3723` → `1:02:03`, null/nonsense → null. */
export function secondsToClock(seconds: number | null | undefined): string | null {
  if (typeof seconds !== "number" || !Number.isFinite(seconds) || seconds < 0) return null;
  const total = Math.floor(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  const two = (value: number) => String(value).padStart(2, "0");
  return hours > 0 ? `${hours}:${two(minutes)}:${two(rest)}` : `${minutes}:${two(rest)}`;
}

// ---------------------------------------------------------------------------
// The URL metadata cache (spec §2.9)
// ---------------------------------------------------------------------------

/** How many URLs are remembered. Small on purpose: this is a convenience. */
export const CACHE_LIMIT = 50;

/**
 * The original's `url_cache`: once a URL has been inspected, coming back to it
 * shows the result without a second network round-trip.
 *
 * A plain class with an insertion cap; "cleared on Clear" is the store's job.
 */
export class MetadataCache {
  #entries = new Map<string, Inspected>();

  get(url: string): Inspected | undefined {
    return this.#entries.get(url);
  }

  set(url: string, inspected: Inspected): void {
    // Only complete video answers are worth remembering: an error, a playlist
    // stub, or a **degraded** video (the list may be missing resolutions — the
    // backend already retried) is cheap to fetch again and would go stale
    // fastest. Caching a degraded one would also mean that pasting the same
    // link again shows the same short list with no way to get the real one.
    if (inspected.kind !== "video" || inspected.degraded) return;
    this.#entries.delete(url);
    this.#entries.set(url, inspected);
    while (this.#entries.size > CACHE_LIMIT) {
      const oldest = this.#entries.keys().next();
      if (oldest.done) break;
      this.#entries.delete(oldest.value);
    }
  }

  clear(): void {
    this.#entries.clear();
  }

  get size(): number {
    return this.#entries.size;
  }
}
