import { describe, expect, it } from "vitest";
import { resolveTheme, windowThemeFor } from "$lib/theme";

describe("resolveTheme()", () => {
  it("returns the explicit preference without consulting the OS", () => {
    expect(resolveTheme("dark", "light")).toBe("dark");
    expect(resolveTheme("light", "dark")).toBe("light");
  });

  it("follows the OS theme for 'system'", () => {
    expect(resolveTheme("system", "dark")).toBe("dark");
    expect(resolveTheme("system", "light")).toBe("light");
  });

  it("falls back to light when the OS theme is unknown", () => {
    expect(resolveTheme("system", null)).toBe("light");
  });
});

describe("windowThemeFor()", () => {
  it("maps 'system' to null so the window follows the OS", () => {
    expect(windowThemeFor("system")).toBeNull();
  });

  it("maps explicit preferences to themselves", () => {
    expect(windowThemeFor("dark")).toBe("dark");
    expect(windowThemeFor("light")).toBe("light");
  });
});