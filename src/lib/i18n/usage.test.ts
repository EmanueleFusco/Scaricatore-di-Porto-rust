/**
 * The dictionaries against the code (spec §11.9).
 *
 * A missing key is not a crash: `t()` falls back to the key itself, so the UI
 * quietly prints `queue.add.audio` where a label should be. That shipped twice
 * (D98), and only a human looking at the screen could catch it. This test
 * cannot see the screen, but it can see what the code asks for: every literal
 * key in the frontend, the families built from the item-state and action
 * unions, and every quality label at the heights a video can actually report —
 * a video may say `576` or `4320`, and no fixed list of height keys covers it.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { hasKey, type Lang } from "$lib/i18n";
import {
  actionKey,
  progressLabelKey,
  qualityLabel,
  stateKey,
  type ItemAction,
  type ItemState,
  type QualityChoice,
} from "$lib/queue";

const LANGS: readonly Lang[] = ["it", "en"];
/** `src/`, from this file's directory: `src/lib/i18n/` → up three. */
const SRC = fileURLToPath(new URL("../../../src/", import.meta.url));

function sourceFiles(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      files.push(...sourceFiles(path));
    } else if (/\.(ts|svelte)$/.test(entry) && !/\.test\.ts$/.test(entry)) {
      // Tests are not the UI: some of them deliberately ask for keys that do
      // not exist, to pin the fallback.
      files.push(path);
    }
  }
  return files;
}

/** `tt("key"…)`, `ttCount("key"…)`, `errorText("key"…)`, `t("it", "key"…)`. */
const KEY_CALLS = [
  /\btt\(\s*"([^"]+)"/g,
  /\bttCount\(\s*"([^"]+)"/g,
  /\berrorText\(\s*"([^"]+)"/g,
  /\btCount\(\s*"[a-z]{2}"\s*,\s*"([^"]+)"/g,
  /\bt\(\s*"[a-z]{2}"\s*,\s*"([^"]+)"/g,
  // Keys that are assigned, not passed: `{ key: "nav.queue" }`, `errorKey:
  // "queue.add.playlist.unsupported"`. One of those was missing from the
  // dictionaries and showed its raw name (the playlist row, D98).
  /\b(?:key|errorKey|messageKey|titleKey|bodyKey|labelKey)\s*[:=]\s*"([a-z][A-Za-z0-9]*(?:\.[A-Za-z0-9]+)+)"/g,
];

function literalKeys(): string[] {
  const keys = new Set<string>();
  for (const file of sourceFiles(SRC)) {
    const text = readFileSync(file, "utf8");
    for (const pattern of KEY_CALLS) {
      for (const match of text.matchAll(pattern)) {
        keys.add(match[1]);
      }
    }
  }
  return [...keys].sort();
}

/** Every state, so adding one to the union breaks this test until it is here. */
const STATES: Record<ItemState, true> = {
  queued: true,
  inspecting: true,
  downloading: true,
  postProcessing: true,
  encoding: true,
  transcribing: true,
  summarizing: true,
  done: true,
  failed: true,
  cancelled: true,
  paused: true,
  interrupted: true,
};

/** Every action, same contract as [`STATES`]. */
const ACTIONS: Record<ItemAction, true> = {
  pause: true,
  resume: true,
  cancel: true,
  retry: true,
  reveal: true,
  open: true,
  remove: true,
  moveToTop: true,
  copyLink: true,
  openPage: true,
  transcribe: true,
};

/** Heights no fixed dictionary of keys could have anticipated. */
const CHOICES: QualityChoice[] = [
  { kind: "best" },
  { kind: "exact", height: 576 },
  { kind: "maxHeight", height: 4320 },
  { kind: "audioOnly" },
  { kind: "explicit", formatId: "137", hasAudio: false },
];

/** Every token the backend can report as a stream/step label, plus a new one. */
const PROGRESS_TOKENS = [
  "preparing",
  "verifying",
  "video",
  "audio",
  "merging",
  "moving",
  "extractingAudio",
  "embedding",
  "encoding",
  "waitingEncode",
  "transcribing",
  "waitingTranscribe",
  "postProcessing",
  "somethingTheBackendInventedLater",
];

describe("the dictionaries know every key the UI asks for", () => {
  /**
   * A key may be pluralised by `tCount`: `<key>.one` / `<key>.other` is then
   * what the dictionaries have to carry, not the bare key.
   */
  const known = (lang: Lang, key: string): boolean =>
    hasKey(lang, key) || hasKey(lang, `${key}.one`) || hasKey(lang, `${key}.other`);

  it.each(LANGS)("covers every literal key in the source (%s)", (lang) => {
    const missing = literalKeys().filter((key) => !known(lang, key));
    expect(missing).toEqual([]);
  });

  it.each(LANGS)("covers every item state and action (%s)", (lang) => {
    const states = Object.keys(STATES) as ItemState[];
    const actions = Object.keys(ACTIONS) as ItemAction[];
    const missing = [...states.map(stateKey), ...actions.map(actionKey)].filter(
      (key) => !known(lang, key),
    );
    expect(missing).toEqual([]);
  });

  it.each(LANGS)("covers every quality label, at any height (%s)", (lang) => {
    const missing = CHOICES.map(qualityLabel)
      .map((label) => label.key)
      .filter((key) => !known(lang, key));
    expect(missing).toEqual([]);
  });

  it.each(LANGS)("covers every progress label the backend can send (%s)", (lang) => {
    // The tokens arrive as data (`video`, `merging`, …), so no literal in the
    // source names these keys: the mapping function is the only thing that
    // knows them, and an unknown token must still land on a real string.
    const missing = PROGRESS_TOKENS.map(progressLabelKey).filter((key) => !known(lang, key));
    expect(missing).toEqual([]);
  });
});
