/**
 * Pure helpers for the Add bar (spec §2.2, §11.3, §8.2).
 *
 * Splitting a paste into links and turning a list of resolutions into the
 * options of a `<select>` are decisions, not rendering: they live here, free of
 * Svelte runes and of any `@tauri-apps` import, so the tests can cover them
 * directly (AGENTS.md §6).
 */
import type { QualityChoice } from "$lib/queue";

/** The preset chips of the Add bar, applied to every row that has no choice. */
export const QUALITY_PRESETS: { value: string; choice: QualityChoice }[] = [
  { value: "best", choice: { kind: "best" } },
  { value: "max1080", choice: { kind: "maxHeight", height: 1080 } },
  { value: "max720", choice: { kind: "maxHeight", height: 720 } },
  { value: "max480", choice: { kind: "maxHeight", height: 480 } },
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

/** One choice of a `<select>`, with the value that selects it back. */
export interface QualityOption {
  value: string;
  choice: QualityChoice;
}

/**
 * The quality options of one link.
 *
 * `heights` come from the inspection: a video lists its resolutions, as *max*
 * and *exact* (spec §8.2 needs both), and a source without heights offers only
 * Best and Audio only — never a failure, because heights are missing on plenty
 * of real sites (spec §6.2).
 */
export function qualityOptions(heights: readonly number[], audioOnly: boolean): QualityOption[] {
  const options: QualityOption[] = [{ value: "best", choice: { kind: "best" } }];
  if (!audioOnly) {
    for (const height of heights) {
      options.push({ value: `max${height}`, choice: { kind: "maxHeight", height } });
      options.push({ value: `exact${height}`, choice: { kind: "exact", height } });
    }
  }
  options.push({ value: "audio", choice: { kind: "audioOnly" } });
  return options;
}

/** The i18n label of a quality choice (spec §8.2, §11.9). */
export function qualityLabel(choice: QualityChoice): string {
  switch (choice.kind) {
    case "exact":
      return `queue.item.quality.exact.${choice.height}`;
    case "maxHeight":
      return `queue.item.quality.max.${choice.height}`;
    case "audioOnly":
      return "queue.item.quality.audio";
    case "explicit":
      return "queue.item.quality.explicit";
    default:
      return "queue.item.quality.best";
  }
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