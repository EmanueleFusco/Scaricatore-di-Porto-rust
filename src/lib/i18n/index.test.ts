import { describe, expect, it } from "vitest";
import {
  FALLBACK_LANG,
  hasKey,
  interpolate,
  isEnglishTag,
  isItalianTag,
  keysOf,
  languageFromTags,
  languageName,
  resolveLanguage,
  t,
  tCount,
} from "$lib/i18n";

describe("language detection", () => {
  it("recognises Italian tags with region or script subtags", () => {
    expect(isItalianTag("it")).toBe(true);
    expect(isItalianTag("it-CH")).toBe(true);
    expect(isItalianTag("it_IT")).toBe(true);
    expect(isItalianTag("IT")).toBe(true);
    expect(isItalianTag("italian")).toBe(false);
  });

  it("recognises English tags with region subtags", () => {
    expect(isEnglishTag("en")).toBe(true);
    expect(isEnglishTag("en-GB")).toBe(true);
    expect(isEnglishTag("en_US")).toBe(true);
    expect(isEnglishTag("enochian")).toBe(false);
  });

  it("picks the first supported locale from an ordered list", () => {
    expect(languageFromTags(["de-DE", "fr", "it-IT", "en-GB"])).toBe("it");
    expect(languageFromTags(["de-DE", "fr", "en-GB"])).toBe("en");
  });

  it("falls back when no locale is supported", () => {
    expect(languageFromTags(["de-DE", "ja-JP"])).toBe(FALLBACK_LANG);
    expect(languageFromTags([])).toBe(FALLBACK_LANG);
  });

  it("resolves the system preference against the OS tags", () => {
    expect(resolveLanguage("system", ["it-IT"])).toBe("it");
    expect(resolveLanguage("system", ["en-US"])).toBe("en");
    expect(resolveLanguage("system", ["fr-FR"])).toBe(FALLBACK_LANG);
  });

  it("an explicit preference ignores the OS tags", () => {
    expect(resolveLanguage("en", ["it-IT"])).toBe("en");
    expect(resolveLanguage("it", ["en-US"])).toBe("it");
  });
});

describe("interpolation", () => {
  it("replaces named placeholders", () => {
    expect(interpolate("{a} and {b}", { a: "x", b: 2 })).toBe("x and 2");
  });

  it("leaves unknown placeholders untouched", () => {
    expect(interpolate("{a} and {missing}", { a: "x" })).toBe("x and {missing}");
  });

  it("returns the template when there are no params", () => {
    expect(interpolate("{a}")).toBe("{a}");
  });
});

describe("t()", () => {
  it("translates in the requested language", () => {
    expect(t("it", "nav.queue")).toBe("Coda");
    expect(t("en", "nav.queue")).toBe("Queue");
  });

  it("returns the key itself when it is missing, instead of crashing", () => {
    expect(t("en", "does.not.exist")).toBe("does.not.exist");
  });

  it("falls back to the default language for a missing key", () => {
    // `footer.placeholder` exists in both; a key present only in one language
    // must still resolve rather than showing the raw key.
    expect(hasKey("it", "nav.queue")).toBe(true);
    expect(hasKey("en", "nav.queue")).toBe(true);
  });

  it("interpolates parameters", () => {
    expect(t("it", "toast.settingsRecovered", { path: "/tmp/settings.json.bak" })).toContain(
      "/tmp/settings.json.bak",
    );
  });
});

describe("tCount()", () => {
  it("uses the singular form for exactly one", () => {
    expect(tCount("en", "plural.running", 1)).toBe("1 running");
    expect(tCount("it", "plural.running", 1)).toBe("1 in esecuzione");
  });

  it("uses the plural form for anything else", () => {
    expect(tCount("en", "plural.running", 0)).toBe("0 running");
    expect(tCount("en", "plural.running", 7)).toBe("7 running");
    expect(tCount("it", "plural.running", 3)).toBe("3 in esecuzione");
  });
});

describe("dictionary integrity", () => {
  it("has exactly the same keys in both languages", () => {
    expect(keysOf("it")).toEqual(keysOf("en"));
  });

  it("has no empty translations", () => {
    for (const lang of ["it", "en"] as const) {
      for (const key of keysOf(lang)) {
        const value = t(lang, key);
        expect(value, `${lang}:${key}`).not.toBe("");
        expect(value, `${lang}:${key}`).not.toBe(key);
      }
    }
  });
});

describe("languageName()", () => {
  it("names each language in that language", () => {
    expect(languageName("it")).toBe("Italiano");
    expect(languageName("en")).toBe("English");
  });
});