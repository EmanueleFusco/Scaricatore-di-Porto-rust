import { describe, expect, it } from "vitest";
import {
  newLinks,
  parseHeaderLines,
  parseTrimTime,
  splitLinks,
  validateTrimInput,
  AUDIO_CODEC_OPTIONS,
  SPONSORBLOCK_CATEGORY_KEYS,
  normaliseLanguage,
  resolveRowTranscriptLanguage,
  rowLanguageOptions,
} from "$lib/addbar";

describe("splitLinks and newLinks", () => {
  it("splits multiple links by newline and whitespace", () => {
    const raw = "https://example.com/v1\nhttps://example.com/v2   https://example.com/v3";
    expect(splitLinks(raw)).toEqual([
      "https://example.com/v1",
      "https://example.com/v2",
      "https://example.com/v3",
    ]);
  });

  it("filters out empty lines and non-links", () => {
    expect(splitLinks("   \n\n  ")).toEqual([]);
  });

  it("identifies new links that are not already known", () => {
    const known = ["https://example.com/v1", "https://example.com/v2"];
    const input = "https://example.com/v2\nhttps://example.com/v3";
    expect(newLinks(input, known)).toEqual(["https://example.com/v3"]);
  });
});

describe("parseHeaderLines", () => {
  it("parses valid header lines", () => {
    const text = "Authorization: Bearer token123\nX-Custom: value\n";
    expect(parseHeaderLines(text)).toEqual([
      "Authorization: Bearer token123",
      "X-Custom: value",
    ]);
  });

  it("drops invalid or empty header lines", () => {
    const text = "NoColonHere\n   \nValid: Header";
    expect(parseHeaderLines(text)).toEqual(["Valid: Header"]);
  });
});

describe("parseTrimTime (spec §9.2)", () => {
  it("parses pure seconds SS and fractional seconds SS.ms", () => {
    expect(parseTrimTime("45")).toBe(45);
    expect(parseTrimTime("45.5")).toBe(45.5);
    expect(parseTrimTime("0")).toBe(0);
    expect(parseTrimTime("0.25")).toBe(0.25);
  });

  it("parses minutes and seconds MM:SS and MM:SS.ms", () => {
    expect(parseTrimTime("01:30")).toBe(90);
    expect(parseTrimTime("0:45.5")).toBe(45.5);
    expect(parseTrimTime("12:00")).toBe(720);
  });

  it("parses hours, minutes and seconds HH:MM:SS and HH:MM:SS.ms", () => {
    expect(parseTrimTime("01:00:00")).toBe(3600);
    expect(parseTrimTime("01:30:15")).toBe(5415);
    expect(parseTrimTime("02:15:30.5")).toBe(8130.5);
  });

  it("handles whitespace padding", () => {
    expect(parseTrimTime("  01:10  ")).toBe(70);
  });

  it("rejects invalid formats and out-of-range components", () => {
    expect(parseTrimTime("")).toBeNull();
    expect(parseTrimTime("abc")).toBeNull();
    expect(parseTrimTime("-10")).toBeNull();
    expect(parseTrimTime("01:60")).toBeNull(); // seconds >= 60
    expect(parseTrimTime("01:75")).toBeNull();
    expect(parseTrimTime("01:60:00")).toBeNull(); // minutes >= 60
    expect(parseTrimTime("01:02:03:04")).toBeNull(); // too many parts
    expect(parseTrimTime("1:2:3:4:5")).toBeNull();
  });
});

describe("validateTrimInput (spec §9.2)", () => {
  it("returns valid when both bounds are empty", () => {
    const res = validateTrimInput("", "", 120);
    expect(res.valid).toBe(true);
    expect(res.startSec).toBeNull();
    expect(res.endSec).toBeNull();
    expect(res.errorKey).toBeNull();
  });

  it("validates valid start only", () => {
    const res = validateTrimInput("10", "", 120);
    expect(res.valid).toBe(true);
    expect(res.startSec).toBe(10);
    expect(res.endSec).toBeNull();
    expect(res.errorKey).toBeNull();
  });

  it("validates valid end only", () => {
    const res = validateTrimInput("", "01:30", 120);
    expect(res.valid).toBe(true);
    expect(res.startSec).toBeNull();
    expect(res.endSec).toBe(90);
    expect(res.errorKey).toBeNull();
  });

  it("validates valid start and end bounds", () => {
    const res = validateTrimInput("00:30", "01:30", 120);
    expect(res.valid).toBe(true);
    expect(res.startSec).toBe(30);
    expect(res.endSec).toBe(90);
    expect(res.errorKey).toBeNull();
  });

  it("rejects invalid start time format", () => {
    const res = validateTrimInput("not-a-time", "60", 120);
    expect(res.valid).toBe(false);
    expect(res.errorKey).toBe("queue.add.trim.error.invalidFormat");
  });

  it("rejects invalid end time format or end <= 0", () => {
    const res = validateTrimInput("10", "bad", 120);
    expect(res.valid).toBe(false);
    expect(res.errorKey).toBe("queue.add.trim.error.invalidFormat");

    const zeroEnd = validateTrimInput("10", "0", 120);
    expect(zeroEnd.valid).toBe(false);
    expect(zeroEnd.errorKey).toBe("queue.add.trim.error.invalidFormat");
  });

  it("rejects start >= end", () => {
    const equal = validateTrimInput("60", "60", 120);
    expect(equal.valid).toBe(false);
    expect(equal.errorKey).toBe("queue.add.trim.error.startNotBeforeEnd");

    const greater = validateTrimInput("01:30", "01:00", 120);
    expect(greater.valid).toBe(false);
    expect(greater.errorKey).toBe("queue.add.trim.error.startNotBeforeEnd");
  });

  it("rejects start or end exceeding media duration", () => {
    const startExceeds = validateTrimInput("150", "", 120);
    expect(startExceeds.valid).toBe(false);
    expect(startExceeds.errorKey).toBe("queue.add.trim.error.startExceedsDuration");

    const endExceeds = validateTrimInput("10", "150", 120);
    expect(endExceeds.valid).toBe(false);
    expect(endExceeds.errorKey).toBe("queue.add.trim.error.endExceedsDuration");
  });

  it("allows bounds beyond unknown (null) duration", () => {
    const res = validateTrimInput("01:00:00", "02:00:00", null);
    expect(res.valid).toBe(true);
    expect(res.startSec).toBe(3600);
    expect(res.endSec).toBe(7200);
  });
});

describe("M4 constants", () => {
  it("provides standard audio codec options", () => {
    expect(AUDIO_CODEC_OPTIONS.map((o) => o.value)).toEqual(["aac", "mp3", "opus"]);
  });

  it("provides standard SponsorBlock categories", () => {
    expect(SPONSORBLOCK_CATEGORY_KEYS).toContain("sponsor");
    expect(SPONSORBLOCK_CATEGORY_KEYS).toContain("selfpromo");
    expect(SPONSORBLOCK_CATEGORY_KEYS).toContain("interaction");
  });
});

describe("transcription language resolution (spec §10.2, checklist #11, M5)", () => {
  it("normalises video language tags to primary lowercase subtag", () => {
    expect(normaliseLanguage("it-IT")).toBe("it");
    expect(normaliseLanguage("en_US")).toBe("en");
    expect(normaliseLanguage("  ES-419  ")).toBe("es");
    expect(normaliseLanguage("FR")).toBe("fr");
    expect(normaliseLanguage("")).toBeNull();
    expect(normaliseLanguage("   ")).toBeNull();
    expect(normaliseLanguage(null)).toBeNull();
    expect(normaliseLanguage(undefined)).toBeNull();
  });

  it("defaults transcript language to video language when known and default is auto", () => {
    const lang = resolveRowTranscriptLanguage({
      inspectedLanguage: "it-IT",
      currentLanguage: "auto",
      defaultTranscriptLanguage: "auto",
      languageOverridden: false,
    });
    expect(lang).toBe("it");
  });

  it("preserves manual user override when set", () => {
    const lang = resolveRowTranscriptLanguage({
      inspectedLanguage: "it",
      currentLanguage: "en",
      defaultTranscriptLanguage: "auto",
      languageOverridden: true,
    });
    expect(lang).toBe("en");
  });

  it("preserves manual user override even when user chose auto", () => {
    const lang = resolveRowTranscriptLanguage({
      inspectedLanguage: "it",
      currentLanguage: "auto",
      defaultTranscriptLanguage: "auto",
      languageOverridden: true,
    });
    expect(lang).toBe("auto");
  });

  it("respects non-auto default transcript language setting", () => {
    const lang = resolveRowTranscriptLanguage({
      inspectedLanguage: "it",
      currentLanguage: "de",
      defaultTranscriptLanguage: "de",
      languageOverridden: false,
    });
    expect(lang).toBe("de");
  });

  it("falls back to auto when video language is unknown or empty", () => {
    const lang = resolveRowTranscriptLanguage({
      inspectedLanguage: null,
      currentLanguage: "auto",
      defaultTranscriptLanguage: "auto",
      languageOverridden: false,
    });
    expect(lang).toBe("auto");
  });

  it("includes detected video language in dropdown options when not already listed", () => {
    const options = rowLanguageOptions("ja", "auto");
    expect(options.some((opt) => opt.value === "ja")).toBe(true);
    expect(options.find((opt) => opt.value === "ja")?.label).toBe("ja");
  });

  it("does not duplicate detected video language when already in standard list", () => {
    const options = rowLanguageOptions("it-IT", "it");
    const itOptions = options.filter((opt) => opt.value === "it");
    expect(itOptions).toHaveLength(1);
  });
});
