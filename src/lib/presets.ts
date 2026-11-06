import { QUALITY_PRESETS } from "$lib/addbar";
import type { AddRow } from "$lib/stores/addbar.svelte";
import type { Settings } from "$lib/settings";

const BOOLEAN_OPTIONS = ["encode", "keepOriginal", "autoPlay", "embedMetadata", "embedThumbnail", "embedChapters", "transcribe", "transcriptLanguageOverridden", "translateToEnglish", "transcriptsSubfolder", "summarize"] as const;
const STRING_OPTIONS = ["videoCodec", "audioCodec", "sponsorblockMode", "whisperModel", "transcriptLanguage", "transcriptionEngine", "geminiModel", "summaryLanguage"] as const;
export type PresetOptions = Partial<Pick<AddRow, typeof BOOLEAN_OPTIONS[number] | typeof STRING_OPTIONS[number] | "sponsorblockCategories">>;
export interface SavedPreset { id: string; name: string; quality: string; options: PresetOptions }

/** Explicit allowlist: never save links, headers, credentials, paths or clip times. */
export function presetOptions(raw: unknown): PresetOptions {
  const value = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const options: Record<string, unknown> = {};
  for (const key of BOOLEAN_OPTIONS) if (typeof value[key] === "boolean") options[key] = value[key];
  for (const key of STRING_OPTIONS) {
    const item = value[key];
    if (item === null || (typeof item === "string" && item.length <= 128 && !/[\u0000-\u001f]/.test(item))) options[key] = item;
  }
  if (Array.isArray(value.sponsorblockCategories)) options.sponsorblockCategories = value.sponsorblockCategories.filter((item): item is string => typeof item === "string" && item.length <= 32).slice(0, 16);
  return options as PresetOptions;
}

export function normaliseSavedPresets(raw: unknown): SavedPreset[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  return raw.flatMap((entry) => {
    if (!entry || typeof entry !== "object") return [];
    const value = entry as Record<string, unknown>;
    const id = typeof value.id === "string" ? value.id.trim() : "";
    const name = typeof value.name === "string" ? Array.from(value.name.trim()).slice(0, 80).join("") : "";
    if (!id || id.length > 64 || !name || seen.has(id) || !QUALITY_PRESETS.some((preset) => preset.value === value.quality)) return [];
    seen.add(id);
    return [{ id, name, quality: value.quality as string, options: presetOptions(value.options) }];
  }).slice(0, 20);
}

export function defaultPresetOptions(settings: Settings): PresetOptions {
  return {
    encode: false, videoCodec: settings.defaultVideoCodec, audioCodec: settings.defaultAudioCodec,
    keepOriginal: settings.keepOriginalAfterEncode, autoPlay: settings.autoPlay,
    embedMetadata: settings.embedMetadata, embedThumbnail: settings.embedThumbnail, embedChapters: settings.embedChapters,
    sponsorblockMode: settings.sponsorblockMode, sponsorblockCategories: [...settings.sponsorblockCategories],
    transcribe: false, whisperModel: settings.activeWhisperModel,
    transcriptLanguage: settings.defaultTranscriptLanguage, transcriptLanguageOverridden: false,
    translateToEnglish: settings.whisperTranslateToEnglish, transcriptsSubfolder: settings.transcriptsSubfolder,
    transcriptionEngine: settings.transcriptionEngineOrder, geminiModel: settings.defaultGeminiModel,
    summarize: settings.defaultSummarize, summaryLanguage: settings.defaultSummaryLanguage,
  };
}
