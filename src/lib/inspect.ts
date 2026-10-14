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
}

export type PlaylistKind = "playlist" | "channel" | "mix" | "other";

export interface PlaylistStub {
  id: string;
  title: string;
  site: string;
  /** Named `playlistKind` because `kind` is the union discriminant. */
  playlistKind: PlaylistKind;
  count: number | null;
  truncated: boolean;
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
    };
  }

  if (kind === "playlist") {
    return {
      kind: "playlist",
      id: asString(value.id),
      title: asString(value.title),
      site: asString(value.site),
      playlistKind: PLAYLIST_KINDS.includes(value.playlistKind as PlaylistKind)
        ? (value.playlistKind as PlaylistKind)
        : "other",
      count: asNullableNumber(value.count),
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

/** Bytes/s → "12.34" (Mbit/s), two decimals and the shape the original used. */
export function speedMbits(bytesPerSecond: number | null | undefined): string | null {
  if (typeof bytesPerSecond !== "number" || !Number.isFinite(bytesPerSecond) || bytesPerSecond < 0) {
    return null;
  }
  return ((bytesPerSecond * 8) / (1024 * 1024)).toFixed(2);
}

/** A determinate bar needs a number in 0–100; everything else is the spinner. */
export function isDeterminate(percent: number | null | undefined): boolean {
  return typeof percent === "number" && Number.isFinite(percent);
}

// ---------------------------------------------------------------------------
// Resolution picker (spec §2.9: vertical, ultrawide, 2:1 heights are kept)
// ---------------------------------------------------------------------------

export interface ResolutionOption {
  /** `"best"` or the height as a string. */
  value: string;
  height: number | null;
}

/**
 * The options of the resolution select: every height the backend found,
 * descending, plus "Best available".
 *
 * No height is an answer, not a failure (spec §6.2): the list then holds only
 * "Best available", which is exactly what an audio-only or height-less source
 * can offer.
 */
export function resolutionOptions(video: VideoInfo): ResolutionOption[] {
  const heights = [...new Set(video.heights)].sort((a, b) => b - a);
  const options: ResolutionOption[] = heights.map((height) => ({
    value: String(height),
    height,
  }));
  options.push({ value: "best", height: null });
  return options;
}

/** The first (highest) height, else "best". */
export function defaultResolution(video: VideoInfo): string {
  const options = resolutionOptions(video);
  return options[0]?.value ?? "best";
}

// ---------------------------------------------------------------------------
// The URL metadata cache (spec §2.9: "URL metadata cache", cleared on Clear)
// ---------------------------------------------------------------------------

export interface UrlSummary {
  title: string;
  duration: number | null;
  heights: number[];
  site: string;
}

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
    // Only complete video answers are worth remembering: an error or a
    // playlist stub is cheap to fetch again and would go stale fastest.
    if (inspected.kind !== "video") return;
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

/** The small summary kept for the cache identity, used by the tests. */
export function summarise(video: VideoInfo): UrlSummary {
  return {
    title: video.title,
    duration: video.duration,
    heights: video.heights,
    site: video.site,
  };
}
