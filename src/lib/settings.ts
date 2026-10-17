/**
 * Pure settings types and helpers shared by the store and the tests.
 *
 * Kept free of Svelte runes and of any `@tauri-apps` import so it can be unit
 * tested directly (AGENTS.md §6: unit tests for every pure function).
 */
import type { LangPreference } from "$lib/i18n";
import type { ThemePreference } from "$lib/theme";

/** Release channel for the yt-dlp updater (spec §5.5, §11.8). */
export type YtdlpChannel = "stable" | "nightly";

/** JavaScript runtime strategy (spec §5.3.3). */
export type JsRuntimeMode = "auto" | "quickjs" | "deno";

/** Default behaviour when a URL has both a video and a `list=` parameter (spec §6.4, §11.8). */
export type PlaylistUrlBehavior = "ask" | "video" | "playlist";

/** Mirror of the Rust `state::Settings` document (spec §2.6, §2.8, §11.8). */
export interface Settings {
  schemaVersion: number;
  language: LangPreference;
  theme: ThemePreference;
  defaultOutputDir: string | null;
  /** Spec §5.2 lookup order #1: "Use my own path", keyed by tool name. */
  toolPaths: Record<string, string>;
  ytdlpChannel: YtdlpChannel;
  /** Spec §11.1: the Setup wizard shows once and is re-openable from Tools. */
  setupWizardCompleted: boolean;
  /** Spec §5.3.3: Auto (QuickJS→Deno) | QuickJS only | Deno only. */
  jsRuntimeMode: JsRuntimeMode;
  /** Spec §8.6: `--cookies-from-browser`, one of {@link BROWSER_COOKIE_NAMES}. */
  cookiesFromBrowser: string | null;
  /** Spec §6.3.3: a Netscape cookies file instead of the browser database. */
  cookiesFile: string | null;
  /** Spec §5.3.2: the Deno install offer was declined once. */
  denoOfferDismissed: boolean;
  /**
   * Expert override for YouTube's `player_client` (spec §5.3.7's "expert
   * field"). Empty means yt-dlp's own client list, which is the maintained one
   * (D74) — the app does not force the original app's list any more.
   */
  youtubePlayerClients: string | null;
  /** Spec §7.2, §11.8: parallel downloads, 1–4 (default 2). */
  parallelDownloads: number;
  /** Spec §6.4, §11.8: max concurrent items per host (default 2). */
  perHostCap: number;
  /** Spec §7.7, §11.8: system notification when the queue goes idle. */
  notifyOnIdle: boolean;
  /** Spec §11.3, §11.8: Ctrl/Cmd+V anywhere adds links. */
  pasteLinksAnywhere: boolean;
  /** Spec §8.2, §11.8: include `[%(id)s]` in the output filename (default true). */
  includeIdInFilename: boolean;
  /** Spec §8.2, §11.8: prefer H.264/AAC for compatibility (default false). */
  preferH264: boolean;
  /** Spec §6.4, §11.8: default behaviour for `watch?v=…&list=…` URLs (default "ask"). */
  playlistUrlBehavior: PlaylistUrlBehavior;
  /** Spec §6.4, §8.2, §11.8: skip videos already recorded in `archive.txt` (default false). */
  useDownloadArchive: boolean;
  /** Spec §6.3.4, §8.2, §11.8: HTTP/HTTPS/SOCKS5 proxy URL (`--proxy`). */
  proxyUrl: string | null;
  /** Spec §8.2, §11.8: bandwidth limit (`--limit-rate`, e.g. `5M`). */
  limitRate: string | null;
  /** Spec §6.3.4, §8.2, §11.8: default custom `Referer` header. */
  customReferer: string | null;
  /** Spec §6.3.4, §8.2, §11.8: default custom `User-Agent` header. */
  customUserAgent: string | null;
  /** Spec §6.3.4, §8.2, §11.8: default extra `Header: Value` lines. */
  customHeaders: string[];
  /** Spec §6.3.5, §11.8: retry once with `--impersonate chrome` on anti-bot failure (default true). */
  impersonateRetry: boolean;
  /** Spec §9.2, §11.8: Keep original downloaded file after successful encode (default false). */
  keepOriginalAfterEncode: boolean;
  /** Spec §9.5, §11.8: Automatically play finished media (default false). */
  autoPlay: boolean;
  /** Spec §8.4, §11.8: Default embed metadata (default false). */
  embedMetadata: boolean;
  /** Spec §8.4, §11.8: Default embed thumbnail (default false). */
  embedThumbnail: boolean;
  /** Spec §8.4, §11.8: Default embed chapters (default false). */
  embedChapters: boolean;
  /** Spec §8.4, §11.8: Default SponsorBlock mode (null | "mark" | "remove"). */
  sponsorblockMode: SponsorBlockMode | null;
  /** Spec §8.4, §11.8: Default SponsorBlock categories. */
  sponsorblockCategories: string[];
  /** Spec §9.3, §11.8: Default video codec (e.g. "libx264"). */
  defaultVideoCodec: string | null;
  /** Spec §9.4, §11.8: Default audio codec (e.g. "aac", "mp3", "opus"). */
  defaultAudioCodec: string | null;
}

/** SponsorBlock mode (spec §8.4): mark as chapters vs remove cuts. */
export type SponsorBlockMode = "mark" | "remove";

/** Mirror of the Rust `commands::settings::ArchiveStatusView` payload. */
export interface ArchiveStatusView {
  path: string;
  entries: number;
}

/** Mirror of the Rust `state::Recovery` payload. */
export interface Recovery {
  backupPath: string | null;
  code: string;
  messageKey: string;
  detail: string | null;
}

/** Mirror of the Rust `state::SettingsView` payload. */
export interface SettingsView {
  settings: Settings;
  recovery: Recovery | null;
  settingsPath: string;
}

/** Shape of every error the backend returns to the frontend (spec §3). */
export interface BackendErrorInfo {
  code: string;
  messageKey: string;
  detail: string | null;
}

export const DEFAULT_SETTINGS: Settings = {
  schemaVersion: 6,
  language: "system",
  theme: "system",
  defaultOutputDir: null,
  toolPaths: {},
  ytdlpChannel: "stable",
  setupWizardCompleted: false,
  jsRuntimeMode: "auto",
  cookiesFromBrowser: null,
  cookiesFile: null,
  denoOfferDismissed: false,
  youtubePlayerClients: null,
  parallelDownloads: 2,
  perHostCap: 2,
  notifyOnIdle: false,
  pasteLinksAnywhere: false,
  includeIdInFilename: true,
  preferH264: false,
  playlistUrlBehavior: "ask",
  useDownloadArchive: false,
  proxyUrl: null,
  limitRate: null,
  customReferer: null,
  customUserAgent: null,
  customHeaders: [],
  impersonateRetry: true,
  keepOriginalAfterEncode: false,
  autoPlay: false,
  embedMetadata: false,
  embedThumbnail: false,
  embedChapters: false,
  sponsorblockMode: null,
  sponsorblockCategories: ["sponsor", "selfpromo", "interaction"],
  defaultVideoCodec: null,
  defaultAudioCodec: "aac",
};

/**
 * Clamp a cap into the range the backend accepts.
 *
 * Spec §7.2: zero would deadlock the queue (nothing could ever start) and a
 * huge number would defeat the lane the spec exists for. The Rust side clamps
 * again — a settings file can be edited by hand — but the UI must not show a
 * value the backend will silently change.
 */
export function clampCap(value: unknown, min: number, max: number, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.round(value)));
}

const LANGUAGES: readonly LangPreference[] = ["system", "it", "en"];
const THEMES: readonly ThemePreference[] = ["system", "dark", "light"];
const CHANNELS: readonly YtdlpChannel[] = ["stable", "nightly"];
const RUNTIME_MODES: readonly JsRuntimeMode[] = ["auto", "quickjs", "deno"];
const PLAYLIST_BEHAVIORS: readonly PlaylistUrlBehavior[] = ["ask", "video", "playlist"];
export const SPONSORBLOCK_MODES: readonly SponsorBlockMode[] = ["mark", "remove"];

/**
 * The browsers `--cookies-from-browser` knows (spec §8.6), in a stable order.
 * Safari exists only on macOS and is filtered by {@link browserNamesForTriple}.
 */
export const BROWSER_COOKIE_NAMES = [
  "chrome",
  "edge",
  "firefox",
  "brave",
  "opera",
  "vivaldi",
  "chromium",
  "safari",
] as const;

/**
 * Which browsers the dropdown may offer on this build.
 *
 * The triple comes from the backend (`tools_status`), which is the same source
 * the Rust-side normalisation uses; an empty triple (plain-browser dev run)
 * offers everything except Safari, because guessing "mac" from a user agent is
 * exactly the unreliable-platform-detection the spec moves to the backend.
 */
export function browserNamesForTriple(triple: string): string[] {
  return BROWSER_COOKIE_NAMES.filter(
    (name) => name !== "safari" || triple.includes("apple-darwin"),
  );
}

export function isJsRuntimeMode(value: unknown): value is JsRuntimeMode {
  return RUNTIME_MODES.includes(value as JsRuntimeMode);
}

export function isLangPreference(value: unknown): value is LangPreference {
  return LANGUAGES.includes(value as LangPreference);
}

export function isThemePreference(value: unknown): value is ThemePreference {
  return THEMES.includes(value as ThemePreference);
}

export function isYtdlpChannelValue(value: unknown): value is YtdlpChannel {
  return CHANNELS.includes(value as YtdlpChannel);
}

export function isPlaylistUrlBehavior(value: unknown): value is PlaylistUrlBehavior {
  return PLAYLIST_BEHAVIORS.includes(value as PlaylistUrlBehavior);
}

export function isSponsorBlockMode(value: unknown): value is SponsorBlockMode {
  return SPONSORBLOCK_MODES.includes(value as SponsorBlockMode);
}

function normaliseOptionalTrimmed(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed !== "" ? trimmed : null;
}

/**
 * Coerce anything the backend sends into a known, complete document.
 *
 * `toolPaths` entries that are not non-empty strings are dropped, which mirrors
 * the Rust-side `normalise()`: an override that is not a usable path would be
 * refused later by the process layer (spec §4.8), and showing it in the UI as
 * if it worked is worse than not showing it.
 */
export function normaliseSettings(raw: Partial<Settings> | null | undefined): Settings {
  const paths: Record<string, string> = {};
  if (raw?.toolPaths && typeof raw.toolPaths === "object") {
    for (const [name, value] of Object.entries(raw.toolPaths)) {
      if (name.trim() === "" || typeof value !== "string" || value === "") continue;
      paths[name] = value;
    }
  }
  const customHeaders = Array.isArray(raw?.customHeaders)
    ? raw.customHeaders
        .filter((h): h is string => typeof h === "string")
        .map((h) => h.trim())
        .filter((h) => h !== "")
    : [];
  return {
    schemaVersion:
      typeof raw?.schemaVersion === "number" ? raw.schemaVersion : DEFAULT_SETTINGS.schemaVersion,
    language: isLangPreference(raw?.language) ? raw.language : "system",
    theme: isThemePreference(raw?.theme) ? raw.theme : "system",
    defaultOutputDir: raw?.defaultOutputDir || null,
    toolPaths: paths,
    ytdlpChannel: isYtdlpChannelValue(raw?.ytdlpChannel) ? raw.ytdlpChannel : "stable",
    setupWizardCompleted: raw?.setupWizardCompleted === true,
    jsRuntimeMode: isJsRuntimeMode(raw?.jsRuntimeMode) ? raw.jsRuntimeMode : "auto",
    cookiesFromBrowser:
      typeof raw?.cookiesFromBrowser === "string" && raw.cookiesFromBrowser !== ""
        ? raw.cookiesFromBrowser
        : null,
    cookiesFile:
      typeof raw?.cookiesFile === "string" && raw.cookiesFile !== "" ? raw.cookiesFile : null,
    denoOfferDismissed: raw?.denoOfferDismissed === true,
    youtubePlayerClients:
      typeof raw?.youtubePlayerClients === "string" && raw.youtubePlayerClients !== ""
        ? raw.youtubePlayerClients
        : null,
    // The caps are clamped here exactly as the Rust `normalise()` does: an
    // empty number field must not become `NaN` and reach the settings file.
    parallelDownloads: clampCap(raw?.parallelDownloads, 1, 4, 2),
    perHostCap: clampCap(raw?.perHostCap, 1, 8, 2),
    notifyOnIdle: raw?.notifyOnIdle === true,
    pasteLinksAnywhere: raw?.pasteLinksAnywhere === true,
    includeIdInFilename:
      typeof raw?.includeIdInFilename === "boolean"
        ? raw.includeIdInFilename
        : DEFAULT_SETTINGS.includeIdInFilename,
    preferH264: raw?.preferH264 === true,
    playlistUrlBehavior: isPlaylistUrlBehavior(raw?.playlistUrlBehavior)
      ? raw.playlistUrlBehavior
      : "ask",
    useDownloadArchive: raw?.useDownloadArchive === true,
    proxyUrl: normaliseOptionalTrimmed(raw?.proxyUrl),
    limitRate: normaliseOptionalTrimmed(raw?.limitRate),
    customReferer: normaliseOptionalTrimmed(raw?.customReferer),
    customUserAgent: normaliseOptionalTrimmed(raw?.customUserAgent),
    customHeaders,
    impersonateRetry:
      typeof raw?.impersonateRetry === "boolean"
        ? raw.impersonateRetry
        : DEFAULT_SETTINGS.impersonateRetry,
    keepOriginalAfterEncode: raw?.keepOriginalAfterEncode === true,
    autoPlay: raw?.autoPlay === true,
    embedMetadata: raw?.embedMetadata === true,
    embedThumbnail: raw?.embedThumbnail === true,
    embedChapters: raw?.embedChapters === true,
    sponsorblockMode: isSponsorBlockMode(raw?.sponsorblockMode) ? raw.sponsorblockMode : null,
    sponsorblockCategories: Array.isArray(raw?.sponsorblockCategories)
      ? raw.sponsorblockCategories
          .filter((c): c is string => typeof c === "string" && c.trim() !== "")
          .map((c) => c.trim())
      : [...DEFAULT_SETTINGS.sponsorblockCategories],
    defaultVideoCodec: normaliseOptionalTrimmed(raw?.defaultVideoCodec),
    defaultAudioCodec:
      normaliseOptionalTrimmed(raw?.defaultAudioCodec) ?? DEFAULT_SETTINGS.defaultAudioCodec,
  };
}

/**
 * The one validation the expert field needs on this side: yt-dlp's own grammar
 * (client names, variants, commas, `-` exclusions). The Rust side repeats it,
 * because a settings file can be edited by hand.
 */
export function sanitizePlayerClients(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  return /^[A-Za-z0-9_,.-]+$/.test(trimmed) ? trimmed.toLowerCase() : null;
}

/** `navigator.languages` when available, so the `system` language has an input. */
export function currentLocaleTags(): string[] {
  if (typeof navigator === "undefined") return [];
  const tags = navigator.languages;
  return tags && tags.length > 0 ? [...tags] : [navigator.language];
}