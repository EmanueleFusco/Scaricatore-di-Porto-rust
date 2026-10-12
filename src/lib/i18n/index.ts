/**
 * i18n (spec §11.9).
 *
 * IT and EN dictionaries, default from the OS locale, runtime switch. All
 * user-visible strings live here: no hard-coded text in components and no
 * sentence built by concatenation — placeholders go through `t()`.
 */
import en from "./en.json";
import it from "./it.json";

export type Lang = "it" | "en";
/** What is persisted in the settings file. */
export type LangPreference = "system" | Lang;

export type Params = Record<string, string | number>;

const DICTIONARIES: Record<Lang, Record<string, string>> = {
  it: it as Record<string, string>,
  en: en as Record<string, string>,
};

/** Used when the OS locale is neither Italian nor English. */
export const FALLBACK_LANG: Lang = "it";

export const AVAILABLE_LANGS: readonly Lang[] = ["it", "en"];

/** Does `tag` start with an Italian subtag (`it`, `it-CH`, `it_IT`, …)? */
export function isItalianTag(tag: string): boolean {
  return tag.trim().toLowerCase().split(/[-_]/)[0] === "it";
}

/** Does `tag` start with an English subtag (`en`, `en-GB`, …)? */
export function isEnglishTag(tag: string): boolean {
  return tag.trim().toLowerCase().split(/[-_]/)[0] === "en";
}

/**
 * Pick a language from an ordered list of locale tags (the shape of
 * `navigator.languages`).
 */
export function languageFromTags(tags: readonly string[]): Lang {
  for (const tag of tags) {
    if (isItalianTag(tag)) return "it";
    if (isEnglishTag(tag)) return "en";
  }
  return FALLBACK_LANG;
}

/** Resolve the `system` preference against the browser/OS locale. */
export function resolveLanguage(preference: LangPreference, tags: readonly string[]): Lang {
  return preference === "system" ? languageFromTags(tags) : preference;
}

/** Replace `{name}` placeholders. Unknown placeholders are left untouched. */
export function interpolate(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    return value === undefined ? match : String(value);
  });
}

function lookup(lang: Lang, key: string): string | undefined {
  return DICTIONARIES[lang][key];
}

/**
 * Translate `key`, falling back to the other language and finally to the key
 * itself, so a missing string is visible but never crashes the UI.
 */
export function t(lang: Lang, key: string, params?: Params): string {
  const template = lookup(lang, key) ?? lookup(FALLBACK_LANG, key);
  if (template === undefined) return key;
  return interpolate(template, params);
}

/**
 * Plural-aware translation. Looks for `<key>.one` / `<key>.other` first and
 * falls back to the plain `<key>`. Italian and English both use the
 * one/other split, so one rule covers both languages in v1.
 */
export function tCount(lang: Lang, key: string, count: number, params?: Params): string {
  const form = count === 1 ? "one" : "other";
  const merged: Params = { count, ...params };
  const template =
    lookup(lang, `${key}.${form}`) ?? lookup(FALLBACK_LANG, `${key}.${form}`) ?? lookup(lang, key);
  if (template === undefined) return key;
  return interpolate(template, merged);
}

/** True when the key exists in both dictionaries (used by the key-parity test). */
export function hasKey(lang: Lang, key: string): boolean {
  return lookup(lang, key) !== undefined;
}

/** Every key of a dictionary, for the parity test. */
export function keysOf(lang: Lang): string[] {
  return Object.keys(DICTIONARIES[lang]).sort();
}

/** Human-readable language name, written in the language itself. */
export function languageName(lang: Lang): string {
  return lang === "it" ? "Italiano" : "English";
}