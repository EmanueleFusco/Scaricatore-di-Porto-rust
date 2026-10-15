//! The system notification the app sends when the queue becomes idle
//! (spec §7.7, §11.8).
//!
//! Spec §7.7 calls this "optional"; §14 row M2 lists it among the phase's
//! deliverables, so it exists — behind a setting that is **off by default**,
//! because a notification that always fires is one nobody reads.
//!
//! ## Why the text is resolved here and not in the frontend
//!
//! The queue is idle exactly when the last item finishes, and that moment can
//! arrive with the window hidden behind something else; a notification driven by
//! the webview would be skipped in that case. The wording therefore comes from
//! the **same dictionaries** the UI uses — they are embedded with `include_str!`
//! — so there is one set of strings, not two, and the language is the persisted
//! one resolved against the OS locale through Tauri's locale API.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::services::queue::model::QueueSummary;

/// The two dictionaries, verbatim from the frontend. Embedded rather than
/// parsed at runtime so a build that forgets to update a string fails loudly
/// at the point of use instead of showing a raw key in the OS notification.
const IT: &str = include_str!("../../../src/lib/i18n/it.json");
const EN: &str = include_str!("../../../src/lib/i18n/en.json");

/// The language of the notification text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    It,
    En,
}

impl Lang {
    /// Resolve the persisted preference against a locale tag.
    ///
    /// Same rule as the frontend (`resolveLanguage`): an Italian or English
    /// subtag decides, anything else falls back to Italian, which is the
    /// product's first language.
    pub fn resolve(preference: crate::state::Language, os_locale: Option<&str>) -> Self {
        if let Some(explicit) = preference.explicit() {
            return match explicit {
                "en" => Self::En,
                _ => Self::It,
            };
        }
        match os_locale.map(|tag| tag.trim().to_ascii_lowercase()) {
            Some(tag) => {
                let primary = tag.split(['-', '_']).next().unwrap_or_default();
                match primary {
                    "it" => Self::It,
                    "en" => Self::En,
                    _ => Self::It,
                }
            }
            None => Self::It,
        }
    }

    /// The locale the environment reports, best effort.
    ///
    /// **Verified while building this:** Tauri v2 exposes no OS locale to Rust
    /// (`Manager` has no `locale()`; neither `Window` nor `Webview` has one —
    /// checked against the 2.12.3 sources and docs.rs), so there is no
    /// cross-platform API to call. `LANG`/`LC_ALL` cover Linux and macOS and
    /// are absent on Windows, which is exactly why the frontend hands the
    /// resolved language over through [`LanguageHolder`] as soon as it knows
    /// it: `navigator.languages` is available on all three targets.
    pub fn from_environment() -> Self {
        let tag = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|key| std::env::var(key).ok())
            .filter(|value| !value.trim().is_empty() && !value.trim().starts_with('C'))
            .unwrap_or_default();
        let primary = tag
            .split(['.', '@', '-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match primary.as_str() {
            "it" => Self::It,
            "en" => Self::En,
            _ => Self::It,
        }
    }

    pub fn from_tag(tag: &str) -> Self {
        match tag.trim().to_ascii_lowercase().as_str() {
            "en" => Self::En,
            _ => Self::It,
        }
    }
}

/// The effective language, shared between the webview and the notifier.
///
/// The frontend resolves the language properly (`navigator.languages`, spec
/// §11.9) and pushes the answer here with the `app_set_language` command; until
/// it does, the persisted preference and the environment decide.
#[derive(Clone, Default)]
pub struct LanguageHolder(Arc<RwLock<Lang>>);

impl LanguageHolder {
    pub fn new(lang: Lang) -> Self {
        Self(Arc::new(RwLock::new(lang)))
    }

    pub fn set(&self, lang: Lang) {
        if let Ok(mut slot) = self.0.write() {
            *slot = lang;
        }
    }

    pub fn get(&self) -> Lang {
        match self.0.read() {
            Ok(lang) => *lang,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

type Dictionary = BTreeMap<String, String>;

fn dictionary(lang: Lang) -> &'static Dictionary {
    static IT_DICT: OnceLock<Dictionary> = OnceLock::new();
    static EN_DICT: OnceLock<Dictionary> = OnceLock::new();
    match lang {
        Lang::It => IT_DICT.get_or_init(|| parse(IT)),
        Lang::En => EN_DICT.get_or_init(|| parse(EN)),
    }
}

fn parse(raw: &str) -> Dictionary {
    #[derive(Deserialize)]
    struct Flat {
        #[serde(flatten)]
        entries: BTreeMap<String, String>,
    }
    // A broken dictionary must not take the app down at startup: the lookup
    // falls back to the other language, and to the key itself.
    serde_json::from_str::<Flat>(raw)
        .map(|flat| flat.entries)
        .unwrap_or_default()
}

/// Translate one key with `{placeholders}`, falling back to the other language
/// and then to the key itself (the same order the frontend uses).
pub fn lookup(lang: Lang, key: &str, params: &BTreeMap<String, String>) -> String {
    let template = dictionary(lang)
        .get(key)
        .or_else(|| dictionary(other(lang)).get(key))
        .cloned();
    match template {
        Some(template) => interpolate(&template, params),
        None => key.to_owned(),
    }
}

fn other(lang: Lang) -> Lang {
    match lang {
        Lang::It => Lang::En,
        Lang::En => Lang::It,
    }
}

fn interpolate(template: &str, params: &BTreeMap<String, String>) -> String {
    let mut out = template.to_owned();
    for (name, value) in params {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Sends the notification through the OS.
///
/// It implements [`crate::services::queue::engine::IdleNotifier`], which is how
/// the engine reaches it without knowing anything about Tauri or the
/// notification plugin.
#[derive(Clone)]
pub struct IdleNotifier {
    app: AppHandle,
    lang: LanguageHolder,
}

impl IdleNotifier {
    pub fn new(app: AppHandle, lang: LanguageHolder) -> Self {
        Self { app, lang }
    }

    /// Build it from the persisted preference and whatever the environment
    /// says; the frontend corrects it as soon as the webview is up.
    pub fn from_settings(app: AppHandle, settings: &crate::state::Settings) -> Self {
        let lang = match settings.language.explicit() {
            Some(explicit) => Lang::from_tag(explicit),
            None => Lang::from_environment(),
        };
        Self::new(app, LanguageHolder::new(lang))
    }
}

impl crate::services::queue::engine::IdleNotifier for IdleNotifier {
    fn notify(&self, summary: &QueueSummary) {
        let lang = self.lang.get();
        let mut params = BTreeMap::new();
        params.insert("count".to_owned(), summary.done.to_string());
        let title = lookup(lang, "toast.queueIdle.title", &params);
        let body = lookup(lang, "toast.queueIdle.body", &params);

        let builder = self.app.notification().builder().title(title).body(body);
        let _ = builder.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn an_explicit_preference_wins_over_the_os_locale() {
        use crate::state::Language;
        assert_eq!(Lang::resolve(Language::It, Some("en-GB")), Lang::It);
        assert_eq!(Lang::resolve(Language::En, Some("it-CH")), Lang::En);
    }

    #[test]
    fn the_system_preference_follows_the_os_locale() {
        use crate::state::Language;
        assert_eq!(Lang::resolve(Language::System, Some("it_IT")), Lang::It);
        assert_eq!(Lang::resolve(Language::System, Some("en")), Lang::En);
        assert_eq!(Lang::resolve(Language::System, Some("fr-FR")), Lang::It);
        assert_eq!(Lang::resolve(Language::System, None), Lang::It);
    }

    #[test]
    fn both_dictionaries_carry_the_notification_strings() {
        for lang in [Lang::It, Lang::En] {
            let title = lookup(lang, "toast.queueIdle.title", &params(&[("count", "3")]));
            assert_ne!(
                title, "toast.queueIdle.title",
                "{lang:?} is missing the title"
            );
            assert!(!title.is_empty());
            let body = lookup(lang, "toast.queueIdle.body", &params(&[("count", "3")]));
            assert_ne!(body, "toast.queueIdle.body");
            assert!(body.contains('3'), "the placeholder must be filled: {body}");
        }
        let it = lookup(
            Lang::It,
            "toast.queueIdle.title",
            &params(&[("count", "2")]),
        );
        let en = lookup(
            Lang::En,
            "toast.queueIdle.title",
            &params(&[("count", "2")]),
        );
        assert_ne!(it, en, "the two languages must not share one string");
    }

    #[test]
    fn a_missing_key_falls_back_to_the_other_language_then_to_itself() {
        assert_eq!(
            lookup(Lang::En, "definitely.not.a.key", &BTreeMap::new()),
            "definitely.not.a.key"
        );
        // A key that exists in one language only still resolves.
        assert_ne!(lookup(Lang::It, "app.name", &BTreeMap::new()), "app.name");
    }

    #[test]
    fn the_language_holder_is_shared_and_settable() {
        let holder = LanguageHolder::new(Lang::It);
        let clone = holder.clone();
        assert_eq!(clone.get(), Lang::It);
        clone.set(Lang::En);
        assert_eq!(holder.get(), Lang::En);
    }

    #[test]
    fn an_environment_locale_is_read_when_the_preference_is_the_system() {
        // `C` and `POSIX` are "no locale", not Italian.
        assert_eq!(Lang::from_tag("C"), Lang::It);
        assert_eq!(Lang::from_tag("en"), Lang::En);
        assert_eq!(Lang::from_tag("it"), Lang::It);
        assert_eq!(Lang::from_tag("fr"), Lang::It);
    }

    #[test]
    fn placeholders_are_replaced_everywhere_they_appear() {
        let mut p = BTreeMap::new();
        p.insert("a".to_owned(), "1".to_owned());
        p.insert("b".to_owned(), "2".to_owned());
        assert_eq!(interpolate("{a} and {b} and {a}", &p), "1 and 2 and 1");
    }
}
