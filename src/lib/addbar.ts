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
