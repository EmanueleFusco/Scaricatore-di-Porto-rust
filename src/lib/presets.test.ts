import { describe, expect, it } from "vitest";
import { defaultPresetOptions, normaliseSavedPresets, presetOptions } from "./presets";
import { DEFAULT_SETTINGS, normaliseSettings } from "./settings";
import { transcriptionRequest, usesGemini } from "./addbar";
describe("saved presets", () => {
  it("limits, deduplicates and round-trips presets while dropping private or source-specific values", () => {
    const raw = { id: "p1", name: " Podcast ", quality: "audio", options: {
      transcribe: true, encode: false, audioCodec: "opus", customHeaders: "fake-secret", url: "https://private.example", trimStart: "10", outputDir: "/private", sponsorblockCategories: ["sponsor", 3],
    } };
    const presets = normaliseSavedPresets([raw, raw, { ...raw, id: "", quality: "unknown" }]);
    expect(presets).toHaveLength(1);
    expect(presets[0].name).toBe("Podcast");
    expect(presets[0].options).toEqual({ transcribe: true, encode: false, audioCodec: "opus", sponsorblockCategories: ["sponsor"] });
    expect(normaliseSavedPresets(Array.from({ length: 25 }, (_, i) => ({ ...raw, id: `p${i}` })))).toHaveLength(20);
    const settings = normaliseSettings({ savedPresets: presets, lastPreset: "p1" });
    expect(normaliseSettings(JSON.parse(JSON.stringify(settings)))).toEqual(settings);
    expect(normaliseSettings({ savedPresets: presets, lastPreset: "missing" }).lastPreset).toBeNull();
  });
  it("rejects malformed option types and derives safe global defaults", () => {
    expect(presetOptions({ transcribe: "yes", audioCodec: "a\nsecret", sponsorblockCategories: [] })).toEqual({ sponsorblockCategories: [] });
    expect(defaultPresetOptions(DEFAULT_SETTINGS)).toMatchObject({ transcribe: false, encode: false, transcriptionEngine: DEFAULT_SETTINGS.transcriptionEngineOrder });
    expect(normaliseSavedPresets(null)).toEqual([]);
  });
});
describe("transcription snapshots and cloud consent", () => {
  it("disables cloud work when transcription is off, even if summary defaults are on", () => {
    const options = transcriptionRequest({ transcribe: false, summarize: true, transcriptionEngine: "gemini_only" });
    expect(options.summarize).toBe(false);
    expect(usesGemini(options, "gemini_only")).toBe(false);
  });
  it("identifies explicit and fallback cloud engines and text summaries", () => {
    expect(usesGemini({ transcribe: true }, "subtitles_whisper_gemini")).toBe(true);
    expect(usesGemini({ transcribe: true, transcriptionEngine: "whisper_only" }, "gemini_only")).toBe(false);
    expect(usesGemini({ transcribe: true, transcriptionEngine: "whisper_only", summarize: true }, "whisper_only")).toBe(true);
  });
});
