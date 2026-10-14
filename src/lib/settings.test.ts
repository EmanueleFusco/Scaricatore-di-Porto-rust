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
      ...DEFAULT_SETTINGS,
      schemaVersion: 2,
      language: "en",
      theme: "dark",
      defaultOutputDir: "/out",
      toolPaths: { "yt-dlp": "/opt/mine/yt-dlp" },
      ytdlpChannel: "nightly",
      setupWizardCompleted: true,
    };
    expect(normaliseSettings(raw)).toEqual(raw);
  });

  it("drops a tool override that is not a usable path", () => {
    // Mirrors the Rust-side normalise(): an override that is not a path would be
    // refused by the process layer (spec §4.8), so showing it as if it worked is
    // worse than not showing it.
    expect(
      normaliseSettings({
        toolPaths: { "yt-dlp": "/opt/mine/yt-dlp", ffmpeg: "", "": "/x" },
      }).toolPaths,
    ).toEqual({ "yt-dlp": "/opt/mine/yt-dlp" });
  });

  it("falls back to the stable channel and to an unconfigured wizard", () => {
    const out = normaliseSettings({
      ytdlpChannel: "weekly" as never,
      setupWizardCompleted: undefined,
    });
    expect(out.ytdlpChannel).toBe("stable");
    expect(out.setupWizardCompleted).toBe(false);
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