import { describe, expect, it } from "vitest";
import {
  DEFAULT_SETTINGS,
  isLangPreference,
  isThemePreference,
  normaliseSettings,
  type Settings,
} from "$lib/settings";

describe("normaliseSettings()", () => {
  it("returns the defaults for a missing document", () => {
    expect(normaliseSettings(null)).toEqual(DEFAULT_SETTINGS);
    expect(normaliseSettings(undefined)).toEqual(DEFAULT_SETTINGS);
    expect(normaliseSettings({})).toEqual(DEFAULT_SETTINGS);
  });

  it("keeps valid values", () => {
    const raw: Settings = {
      schemaVersion: 1,
      language: "en",
      theme: "dark",
      defaultOutputDir: "/out",
    };
    expect(normaliseSettings(raw)).toEqual(raw);
  });

  it("replaces an unknown language or theme with the default", () => {
    const out = normaliseSettings({
      language: "klingon" as never,
      theme: "ultraviolet" as never,
    });
    expect(out.language).toBe("system");
    expect(out.theme).toBe("system");
  });

  it("treats an empty output folder as unset", () => {
    expect(normaliseSettings({ defaultOutputDir: "" }).defaultOutputDir).toBeNull();
  });
});

describe("type guards", () => {
  it("recognise exactly the supported values", () => {
    expect(isLangPreference("system")).toBe(true);
    expect(isLangPreference("it")).toBe(true);
    expect(isLangPreference("en")).toBe(true);
    expect(isLangPreference("fr")).toBe(false);

    expect(isThemePreference("system")).toBe(true);
    expect(isThemePreference("dark")).toBe(true);
    expect(isThemePreference("light")).toBe(true);
    expect(isThemePreference("sepia")).toBe(false);
  });
});