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

/** Mirror of the Rust `state::Settings` document (spec §2.6, §2.8). */
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
  schemaVersion: 3,
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
};

const LANGUAGES: readonly LangPreference[] = ["system", "it", "en"];
const THEMES: readonly ThemePreference[] = ["system", "dark", "light"];
const CHANNELS: readonly YtdlpChannel[] = ["stable", "nightly"];
const RUNTIME_MODES: readonly JsRuntimeMode[] = ["auto", "quickjs", "deno"];

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
  };
}

/** `navigator.languages` when available, so the `system` language has an input. */
export function currentLocaleTags(): string[] {
  if (typeof navigator === "undefined") return [];
  const tags = navigator.languages;
  return tags && tags.length > 0 ? [...tags] : [navigator.language];
}