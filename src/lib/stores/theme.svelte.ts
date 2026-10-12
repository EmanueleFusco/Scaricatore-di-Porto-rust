/**
 * Reactive theme store: owns the `data-theme` attribute and keeps the native
 * window in sync with the preference (spec §11.9).
 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { resolveTheme, windowThemeFor, type ResolvedTheme, type ThemePreference } from "$lib/theme";

const mediaQuery = (): MediaQueryList | null =>
  typeof window === "undefined" || typeof window.matchMedia !== "function"
    ? null
    : window.matchMedia("(prefers-color-scheme: dark)");

class ThemeStore {
  preference = $state<ThemePreference>("system");
  resolved = $state<ResolvedTheme>("light");
  /** True when the OS theme could not be read (e.g. `npm run dev` in a browser). */
  systemUnknown = $state(false);

  #unlisten: UnlistenFn | null = null;
  #media: MediaQueryList | null = null;
  /** Raw OS theme, kept separate from `resolved` so "system" stays reversible. */
  #system: ResolvedTheme | null = null;

  /** Subscribe to OS theme changes. Safe to call once. */
  async start(): Promise<void> {
    // The Tauri window API is the source of truth for "System" (spec §11.9).
    try {
      const appWindow = getCurrentWindow();
      const initial = await appWindow.theme();
      this.#onSystemTheme(initial);
      this.#unlisten = await appWindow.onThemeChanged((event) => {
        this.#onSystemTheme(event.payload);
      });
    } catch {
      // Not running inside Tauri (e.g. `npm run dev` in a browser): fall back
      // to the CSS media query, which CSS also honours on its own.
      this.systemUnknown = true;
    }

    // Keeps working when the OS theme changes and the Tauri event never fires.
    if (!this.#unlisten) {
      this.#media = mediaQuery();
      const listener = (event: MediaQueryListEvent) =>
        this.#onSystemTheme(event.matches ? "dark" : "light");
      this.#media?.addEventListener("change", listener);
      this.#onSystemTheme(this.#media?.matches ? "dark" : "light");
    }
  }

  async setPreference(preference: ThemePreference): Promise<void> {
    this.preference = preference;
    this.#apply();
    try {
      await getCurrentWindow().setTheme(windowThemeFor(preference));
    } catch {
      // The window theme is cosmetic (title bar, native controls).
    }
  }

  #onSystemTheme(theme: ResolvedTheme | null): void {
    this.#system = theme;
    this.systemUnknown = theme === null;
    this.#apply();
  }

  #apply(): void {
    this.resolved = resolveTheme(this.preference, this.#system);
    if (typeof document === "undefined") return;
    document.documentElement.dataset.theme = this.resolved;
    document.documentElement.style.colorScheme = this.resolved;
  }
}

export const themeStore = new ThemeStore();