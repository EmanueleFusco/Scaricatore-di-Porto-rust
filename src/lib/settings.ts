/**
 * Pure settings types and helpers shared by the store and the tests.
 *
 * Kept free of Svelte runes and of any `@tauri-apps` import so it can be unit
 * tested directly (AGENTS.md §6: unit tests for every pure function).
 */
import type { LangPreference } from "$lib/i18n";
import type { ThemePreference } from "$lib/theme";

/** Mirror of the Rust `state::Settings` document (spec §2.6, §2.8). */
export interface Settings {
  schemaVersion: number;
  language: LangPreference;
  theme: ThemePreference;
  defaultOutputDir: string | null;
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
  schemaVersion: 1,
  language: "system",
  theme: "system",
  defaultOutputDir: null,
};

const LANGUAGES: readonly LangPreference[] = ["system", "it", "en"];
const THEMES: readonly ThemePreference[] = ["system", "dark", "light"];

export function isLangPreference(value: unknown): value is LangPreference {
  return LANGUAGES.includes(value as LangPreference);
}

export function isThemePreference(value: unknown): value is ThemePreference {
  return THEMES.includes(value as ThemePreference);
}

/** Coerce anything the backend sends into a known, complete document. */
export function normaliseSettings(raw: Partial<Settings> | null | undefined): Settings {
  return {
    schemaVersion: raw?.schemaVersion ?? DEFAULT_SETTINGS.schemaVersion,
    language: isLangPreference(raw?.language) ? raw.language : "system",
    theme: isThemePreference(raw?.theme) ? raw.theme : "system",
    defaultOutputDir: raw?.defaultOutputDir || null,
  };
}

/** `navigator.languages` when available, so the `system` language has an input. */
export function currentLocaleTags(): string[] {
  if (typeof navigator === "undefined") return [];
  const tags = navigator.languages;
  return tags && tags.length > 0 ? [...tags] : [navigator.language];
}