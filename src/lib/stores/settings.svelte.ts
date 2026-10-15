/**
 * Settings store: mirrors the Rust-side `state::Settings` document
 * (spec §2.6, §2.8) and exposes the active language to the whole UI.
 */
import { invoke } from "@tauri-apps/api/core";
import { resolveLanguage, t, tCount, type Lang, type Params } from "$lib/i18n";
import {
  currentLocaleTags,
  normaliseSettings,
  type BackendErrorInfo,
  type Recovery,
  type Settings,
  type SettingsView,
} from "$lib/settings";
import { themeStore } from "$lib/stores/theme.svelte";

export type { BackendErrorInfo, Recovery, Settings, SettingsView } from "$lib/settings";

/**
 * True inside the real app. `npm run dev` also serves this UI in a plain
 * browser, where the Tauri IPC bridge is absent.
 */
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

class SettingsStore {
  settings = $state<Settings>(normaliseSettings(null));
  recovery = $state<Recovery | null>(null);
  settingsPath = $state("");
  /** Message key of the last failure, so the UI can localise it. */
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);
  saving = $state(false);
  loaded = $state(false);

  async load(): Promise<void> {
    if (!isTauriRuntime()) {
      // `npm run dev` opens the same UI in a plain browser, where there is no
      // IPC. Show the defaults instead of an error banner; inside the real app
      // the backend stays the single source of truth.
      this.loaded = true;
      return;
    }
    try {
      const view = await invoke<SettingsView>("settings_get");
      this.#apply(view);
    } catch (err) {
      this.#fail(err);
    } finally {
      this.loaded = true;
    }
  }

  /** Persist a partial change and adopt whatever the backend normalised. */
  async update(patch: Partial<Settings>): Promise<boolean> {
    const previous = { ...this.settings };
    this.settings = normaliseSettings({ ...previous, ...patch });
    this.saving = true;
    this.errorKey = null;
    this.errorDetail = null;
    if (!isTauriRuntime()) {
      this.#applySideEffects();
      this.saving = false;
      return true;
    }
    try {
      const view = await invoke<SettingsView>("settings_save", { settings: this.settings });
      this.#apply(view);
      return true;
    } catch (err) {
      this.settings = previous;
      this.#fail(err);
      return false;
    } finally {
      this.saving = false;
    }
  }

  dismissError(): void {
    this.errorKey = null;
    this.errorDetail = null;
  }

  #apply(view: SettingsView): void {
    this.settings = normaliseSettings(view.settings);
    this.recovery = view.recovery ?? null;
    this.settingsPath = view.settingsPath ?? "";
    this.#applySideEffects();
  }

  /** Everything that follows a settings change, backend round-trip or not. */
  #applySideEffects(): void {
    // The native window and `data-theme` follow the persisted preference.
    void themeStore.setPreference(this.settings.theme);
    if (typeof document !== "undefined") {
      document.documentElement.lang = activeLang();
    }
  }

  #fail(err: unknown): void {
    const info = err as Partial<BackendErrorInfo>;
    this.errorKey = typeof info?.messageKey === "string" ? info.messageKey : "error.tauri";
    this.errorDetail = typeof info?.detail === "string" ? info.detail : null;
  }
}

export const settingsStore = new SettingsStore();

/**
 * Language in effect right now: the preference, or the OS locale.
 *
 * A function, not an exported `$derived`: Svelte forbids exporting derived
 * state from a module (see https://svelte.dev/e/derived_invalid_export).
 */
export function activeLang(): Lang {
  return resolveLanguage(settingsStore.settings.language, currentLocaleTags());
}

/** Translate with the active language. Components call this in the markup. */
export function tt(key: string, params?: Params): string {
  return t(activeLang(), key, params);
}

/** Translate with the active language, picking the plural form of `count`. */
export function ttCount(key: string, count: number, params?: Params): string {
  return tCount(activeLang(), key, count, params);
}

/** Translate an i18n key coming from the backend (`error.*`, …). */
export function errorText(messageKey: string, detail: string | null): string {
  const message = t(activeLang(), messageKey);
  return detail ? `${message}: ${detail}` : message;
}