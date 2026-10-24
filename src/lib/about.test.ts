/**
 * Tests for the About page helpers (spec §11.8, §13.4).
 */
import { describe, expect, it } from "vitest";
import {
  LICENSES,
  canSelfUpdate,
  normaliseAppInfo,
  normaliseUpdateCheck,
  updatePercent,
  type UpdateProgress,
} from "$lib/about";

describe("the About page data", () => {
  it("normalises a missing backend payload into empty strings, not undefined", () => {
    const info = normaliseAppInfo(null);
    expect(info.version).toBe("");
    expect(info.tempDir).toBeNull();
    expect(info.settingsPath).toBe("");
  });

  it("normalises an update check and keeps unknown fields out", () => {
    const check = normaliseUpdateCheck({
      available: true,
      currentVersion: "0.1.0",
      version: "0.2.0",
      notes: "Fixes",
    });
    expect(check).toEqual({
      available: true,
      currentVersion: "0.1.0",
      version: "0.2.0",
      notes: "Fixes",
      date: null,
    });
  });

  it("computes a clamped percentage from either field", () => {
    const withPercent: UpdateProgress = { downloaded: 1, total: 2, percent: 150 };
    expect(updatePercent(withPercent)).toBe(100);
    const fromBytes: UpdateProgress = { downloaded: 5, total: 10, percent: null };
    expect(updatePercent(fromBytes)).toBe(50);
    const unknown: UpdateProgress = { downloaded: 5, total: null, percent: null };
    expect(updatePercent(unknown)).toBeNull();
  });

  it("only lets an AppImage replace itself on Linux", () => {
    expect(canSelfUpdate("appimage")).toBe(true);
    expect(canSelfUpdate("nsis")).toBe(true);
    expect(canSelfUpdate("app")).toBe(true);
    expect(canSelfUpdate("deb")).toBe(false);
    expect(canSelfUpdate("rpm")).toBe(false);
    expect(canSelfUpdate(null)).toBe(true);
  });

  it("lists the app and the components a user actually receives", () => {
    const names = LICENSES.map((entry) => entry.name);
    expect(names).toContain("Scaricatore Di Porto");
    expect(names).toContain("yt-dlp");
    expect(names).toContain("ffmpeg / ffprobe");
    expect(LICENSES.every((entry) => entry.url.startsWith("https://"))).toBe(true);
  });
});
