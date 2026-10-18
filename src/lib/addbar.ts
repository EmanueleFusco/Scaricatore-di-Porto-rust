/**
 * Pure helpers for the Add bar (spec §2.2, §11.3, §8.2).
 *
 * Splitting a paste into links and turning a list of resolutions into the
 * options of a `<select>` are decisions, not rendering: they live here, free of
 * Svelte runes and of any `@tauri-apps` import, so the tests can cover them
 * directly (AGENTS.md §6).
 *
 * Quality is offered twice, on purpose: a **preset** for everything being added
 * (a cap, `MaxHeight`, the safe choice for a batch and for playlists) and a
 * **per-link picker** with the resolutions that video actually reports (the
 * original's behaviour, exact height with a fallback below, spec §8.2).
 */
import type { FormatLite } from "$lib/inspect";
import type { QualityChoice } from "$lib/queue";

/** The heights the preset offers, tallest first. */
export const PRESET_HEIGHTS: readonly number[] = [2160, 1440, 1080, 720, 480, 360];

/** The preset chips of the Add bar, applied to every row that has no choice. */
export const QUALITY_PRESETS: { value: string; choice: QualityChoice }[] = [
  { value: "best", choice: { kind: "best" } },
  ...PRESET_HEIGHTS.map((height) => ({
    value: `max${height}`,
    choice: { kind: "maxHeight" as const, height },
  })),
  { value: "audio", choice: { kind: "audioOnly" } },
];

/** Split a paste into links: whitespace-separated, deduped, http(s) only. */
export function splitLinks(text: string): string[] {
  const seen = new Set<string>();
  const links: string[] = [];
  for (const token of text.split(/\s+/)) {
    const candidate = token.replace(/^[,;"'<[]+/, "").replace(/[,;"'>\]]+$/, "");
    if (!/^https?:\/\//i.test(candidate)) continue;
    if (seen.has(candidate)) continue;
    seen.add(candidate);
    links.push(candidate);
  }
  return links;
}

/** The links in a paste that are not in `known` yet, in order. */
export function newLinks(text: string, known: readonly string[]): string[] {
  const seen = new Set(known);
  return splitLinks(text).filter((url) => !seen.has(url));
}

/** One choice of a `<select>`, with the value that selects it back. */
export interface QualityOption {
  value: string;
  choice: QualityChoice;
}

/**
 * The quality options of one link's row.
 *
 * The video's own heights come first (descending, as the inspection reports
 * them), each an *exact* choice: the original's "1080p" behaviour (spec §8.2).
 * `chosen` — the row's own pick, or the preset while the row follows it — is
 * inserted after *Best* whenever it is not one of those heights, so the select
 * always shows the quality that is actually in force instead of going blank.
 *
 * A source without heights is not a failure (spec §6.2): the row falls back to
 * Best, the preset and Audio only.
 */
export function rowQualityOptions(
  heights: readonly number[],
  chosen: QualityChoice,
): QualityOption[] {
  const options: QualityOption[] = [{ value: "best", choice: { kind: "best" } }];
  const chosenValue = qualityValue(chosen);
  /** An exact height is already in the list below; a cap or an id is not. */
  const amongHeights = chosen.kind === "exact" && heights.includes(chosen.height);
  if (chosenValue !== "best" && chosenValue !== "audio" && !amongHeights) {
    options.push({ value: chosenValue, choice: chosen });
  }
  for (const height of heights) {
    options.push({ value: `exact${height}`, choice: { kind: "exact", height } });
  }
  options.push({ value: "audio", choice: { kind: "audioOnly" } });
  return options;
}

/**
 * The `<option>` value that stands for a choice.
 *
 * `audioOnly` becomes `"audio"`, not `"audioOnly"`: the value is the same token
 * the preset chips use, so one string covers both the preset and the row's own
 * picker and a value can never mean two different things.
 */
export function qualityValue(choice: QualityChoice): string {
  switch (choice.kind) {
    case "exact":
      return `exact${choice.height}`;
    case "maxHeight":
      return `max${choice.height}`;
    case "audioOnly":
      return "audio";
    case "explicit":
      return `id${choice.formatId}`;
    default:
      return "best";
  }
}

/** The choice a `<select>` value stands for, `null` when it is unknown. */
export function choiceOf(
  options: readonly QualityOption[],
  value: string,
): QualityChoice | null {
  return options.find((option) => option.value === value)?.choice ?? null;
}

/**
 * The `Header: Value` lines of the "More options" box (spec §6.3.4, §11.3).
 *
 * One header per line; a line without a colon, or with a control character, is
 * dropped rather than sent: the backend refuses them again (it never trusts the
 * frontend), and a silent drop here would hide the mistake, so the UI shows the
 * count of accepted lines next to the box.
 */
export function parseHeaderLines(text: string): string[] {
  const lines: string[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const colon = line.indexOf(":");
    if (colon < 1 || colon === line.length - 1) continue;
    // eslint-disable-next-line no-control-regex
    if (/[\u0000-\u001f]/.test(line)) continue;
    lines.push(line);
  }
  return lines;
}

/**
 * The rows of the advanced format table (spec §6.3.7, §11.3): every real
 * stream, tallest first, storyboards (`mhtml`) left out. Audio-only streams
 * come after the video ones, best first by size when known.
 */
export function tableFormats(formats: readonly FormatLite[]): FormatLite[] {
  const real = formats.filter(
    (format) => format.protocol !== "mhtml" && format.ext !== "mhtml" && format.formatId !== "",
  );
  const hasVideo = (format: FormatLite): boolean =>
    format.height !== null || (format.vcodec !== null && format.vcodec !== "none");
  const video = real
    .filter(hasVideo)
    .sort((a, b) => (b.height ?? 0) - (a.height ?? 0) || (b.fps ?? 0) - (a.fps ?? 0));
  const audio = real.filter((format) => !hasVideo(format)).sort((a, b) => (b.filesize ?? 0) - (a.filesize ?? 0));
  return [...video, ...audio];
}

/**
 * The explicit quality for one table row. `hasAudio` tells the backend whether
 * the stream already carries sound or needs the best audio merged in.
 */
export function explicitChoiceFor(format: FormatLite): QualityChoice {
  const audio = format.acodec !== null && format.acodec !== "none";
  return { kind: "explicit", formatId: format.formatId, hasAudio: audio };
}

/** Parse trim time string into fractional seconds (spec §9.2). */
export function parseTrimTime(raw: string): number | null {
  const s = raw.trim();
  if (s === "") return null;
  const parts = s.split(":");
  if (parts.length === 1) {
    const sec = Number(parts[0]);
    if (!Number.isFinite(sec) || sec < 0) return null;
    return sec;
  }
  if (parts.length === 2) {
    const min = Number(parts[0]);
    const sec = Number(parts[1]);
    if (!Number.isFinite(min) || !Number.isFinite(sec) || min < 0 || sec < 0 || sec >= 60) {
      return null;
    }
    return min * 60 + sec;
  }
  if (parts.length === 3) {
    const hr = Number(parts[0]);
    const min = Number(parts[1]);
    const sec = Number(parts[2]);
    if (
      !Number.isFinite(hr) ||
      !Number.isFinite(min) ||
      !Number.isFinite(sec) ||
      hr < 0 ||
      min < 0 ||
      min >= 60 ||
      sec < 0 ||
      sec >= 60
    ) {
      return null;
    }
    return hr * 3600 + min * 60 + sec;
  }
  return null;
}

export interface TrimValidationResult {
  valid: boolean;
  startSec: number | null;
  endSec: number | null;
  errorKey: string | null;
}

/** Validate start and end trim against media duration (spec §9.2). */
export function validateTrimInput(
  startRaw: string,
  endRaw: string,
  mediaDurationSec: number | null,
): TrimValidationResult {
  const hasStart = startRaw.trim() !== "";
  const hasEnd = endRaw.trim() !== "";

  if (!hasStart && !hasEnd) {
    return { valid: true, startSec: null, endSec: null, errorKey: null };
  }

  let startSec: number | null = null;
  let endSec: number | null = null;

  if (hasStart) {
    startSec = parseTrimTime(startRaw);
    if (startSec === null) {
      return { valid: false, startSec: null, endSec: null, errorKey: "queue.add.trim.error.invalidFormat" };
    }
  }

  if (hasEnd) {
    endSec = parseTrimTime(endRaw);
    if (endSec === null) {
      return { valid: false, startSec, endSec: null, errorKey: "queue.add.trim.error.invalidFormat" };
    }
    if (endSec <= 0) {
      return { valid: false, startSec, endSec, errorKey: "queue.add.trim.error.invalidFormat" };
    }
  }

  if (startSec !== null && endSec !== null && startSec >= endSec) {
    return { valid: false, startSec, endSec, errorKey: "queue.add.trim.error.startNotBeforeEnd" };
  }

  if (mediaDurationSec !== null && mediaDurationSec > 0) {
    if (startSec !== null && startSec > mediaDurationSec) {
      return { valid: false, startSec, endSec, errorKey: "queue.add.trim.error.startExceedsDuration" };
    }
    if (endSec !== null && endSec > mediaDurationSec) {
      return { valid: false, startSec, endSec, errorKey: "queue.add.trim.error.endExceedsDuration" };
    }
  }

  return { valid: true, startSec, endSec, errorKey: null };
}

/** Standard audio codecs for media processing (spec §9.4). */
export const AUDIO_CODEC_OPTIONS = [
  { value: "aac", label: "AAC (.m4a)" },
  { value: "mp3", label: "MP3 (.mp3)" },
  { value: "opus", label: "Opus (.opus)" },
] as const;

/** Standard SponsorBlock categories (spec §8.4). */
export const SPONSORBLOCK_CATEGORY_KEYS = [
  "sponsor",
  "selfpromo",
  "interaction",
  "intro",
  "outro",
  "preview",
  "filler",
  "music_offtopic",
] as const;

/**
 * Normalise a language tag from yt-dlp inspection (spec §10.2).
 *
 * Extracts primary subtag and converts to lowercase (e.g. "it-IT" -> "it", "en_US" -> "en").
 * Returns null if the language tag is empty or invalid.
 */
export function normaliseLanguage(lang: string | null | undefined): string | null {
  if (typeof lang !== "string") return null;
  const trimmed = lang.trim();
  if (trimmed === "") return null;
  const primary = trimmed.split(/[-_]/)[0]?.trim().toLowerCase();
  return primary && primary.length > 0 ? primary : null;
}

export interface ResolveTranscriptLanguageParams {
  inspectedLanguage?: string | null;
  currentLanguage?: string | null;
  defaultTranscriptLanguage?: string | null;
  languageOverridden?: boolean;
}

/**
 * Determine the transcription language for a row (spec §10.2, Checklist #11).
 *
 * Language logic:
 * 1. User override takes highest precedence.
 * 2. When defaultTranscriptLanguage is "auto", default to the video's language
 *    when known from inspection.
 * 3. Otherwise fall back to currentLanguage, defaultTranscriptLanguage, or "auto".
 */
export function resolveRowTranscriptLanguage(params: ResolveTranscriptLanguageParams): string {
  const {
    inspectedLanguage,
    currentLanguage,
    defaultTranscriptLanguage = "auto",
    languageOverridden = false,
  } = params;

  if (languageOverridden && currentLanguage && currentLanguage.trim() !== "") {
    return currentLanguage.trim();
  }

  const defaultLang = defaultTranscriptLanguage?.trim().toLowerCase() || "auto";
  if (defaultLang === "auto") {
    const normalised = normaliseLanguage(inspectedLanguage);
    if (normalised) {
      return normalised;
    }
  }

  if (currentLanguage && currentLanguage.trim() !== "") {
    return currentLanguage.trim();
  }

  return defaultLang;
}

export interface RowLanguageOption {
  value: string;
  labelKey: string | null;
  label: string;
}

/**
 * Options for the Whisper language dropdown on an Add bar row (spec §10.2, §11.3).
 *
 * Includes standard languages (auto, it, en, es, fr, de) plus the video's
 * detected language and any currently chosen language if not already listed.
 */
export function rowLanguageOptions(
  detectedLanguage?: string | null,
  currentLanguage?: string | null,
): RowLanguageOption[] {
  const options: RowLanguageOption[] = [
    { value: "auto", labelKey: "transcription.language.auto", label: "auto" },
    { value: "it", labelKey: "transcription.language.it", label: "it" },
    { value: "en", labelKey: "transcription.language.en", label: "en" },
    { value: "es", labelKey: "transcription.language.es", label: "es" },
    { value: "fr", labelKey: "transcription.language.fr", label: "fr" },
    { value: "de", labelKey: "transcription.language.de", label: "de" },
  ];

  const extras = [normaliseLanguage(detectedLanguage), normaliseLanguage(currentLanguage)];
  for (const extra of extras) {
    if (extra && !options.some((opt) => opt.value === extra)) {
      options.push({ value: extra, labelKey: null, label: extra });
    }
  }

  return options;
}
