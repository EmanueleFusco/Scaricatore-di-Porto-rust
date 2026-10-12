/**
 * Theme helpers (spec §11.9).
 *
 * `system` is resolved from the Tauri window theme API (`window.theme()` +
 * `onThemeChanged`), with `prefers-color-scheme` as the fallback because
 * WebKitGTK does not always report the media query correctly.
 */

export type ThemePreference = "system" | "dark" | "light";
export type ResolvedTheme = "light" | "dark";

/**
 * Effective theme for a preference.
 *
 * `systemTheme` is the value reported by the OS; when it is unknown we fall
 * back to light, which is also the default of `prefers-color-scheme`.
 */
export function resolveTheme(
  preference: ThemePreference,
  systemTheme: ResolvedTheme | null,
): ResolvedTheme {
  if (preference === "dark") return "dark";
  if (preference === "light") return "light";
  return systemTheme ?? "light";
}

/**
 * Value to hand to `getCurrentWindow().setTheme()`: `null` means "follow the
 * system", which is what "System" must map to at the window level.
 */
export function windowThemeFor(preference: ThemePreference): ResolvedTheme | null {
  return preference === "system" ? null : preference;
}