//! Chinese / English UI copy. Strings live next to their use as
//! `i18n.t("中文", "English")` pairs, so a view reads in both languages and
//! nothing can fall out of a lookup table. Chinese is the default unless the
//! browser asks for another language; the choice is kept in localStorage.

use std::cell::Cell;

use leptos::prelude::*;

const STORAGE_KEY: &str = "rkvm.lang";

thread_local! {
    // Mirror of the signal for code without a reactive owner (async results,
    // helpers in util.rs). Views use `I18n::t` so they re-render on switch.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::Zh) };
}

/// The current language, read once (not reactive).
pub fn lang_now() -> Lang {
    CURRENT.with(Cell::get)
}

/// The string for the current language, read once (not reactive).
pub fn tr(zh: &'static str, en: &'static str) -> &'static str {
    match CURRENT.with(Cell::get) {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

/// Handle to the current language, provided once by [`provide`] and read with
/// [`use_i18n`]. Copy, so views capture it freely.
#[derive(Clone, Copy)]
pub struct I18n(RwSignal<Lang>);

impl I18n {
    /// Pick the string for the current language. Reactive: call it inside a
    /// closure in a view and the text follows language changes.
    pub fn t(self, zh: &'static str, en: &'static str) -> &'static str {
        match self.0.get() {
            Lang::Zh => zh,
            Lang::En => en,
        }
    }

    /// [`I18n::t`] without tracking, for event handlers and async tasks
    /// that build a message once.
    pub fn t_now(self, zh: &'static str, en: &'static str) -> &'static str {
        tr(zh, en)
    }

    pub fn lang(self) -> Lang {
        self.0.get()
    }

    pub fn toggle(self) {
        let next = match self.0.get_untracked() {
            Lang::Zh => Lang::En,
            Lang::En => Lang::Zh,
        };
        self.0.set(next);
        apply(next);
        crate::util::store_set(STORAGE_KEY, code(next));
    }
}

pub fn provide() -> I18n {
    let lang = match crate::util::store_get(STORAGE_KEY).as_deref() {
        Some("en") => Lang::En,
        Some("zh") => Lang::Zh,
        _ => browser_lang(),
    };
    apply(lang);
    let i18n = I18n(RwSignal::new(lang));
    provide_context(i18n);
    i18n
}

pub fn use_i18n() -> I18n {
    use_context::<I18n>().unwrap_or_else(|| I18n(RwSignal::new(Lang::Zh)))
}

fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => "zh",
        Lang::En => "en",
    }
}

fn browser_lang() -> Lang {
    let nav = web_sys::window().and_then(|w| w.navigator().language());
    match nav {
        Some(l) if !l.to_ascii_lowercase().starts_with("zh") => Lang::En,
        _ => Lang::Zh,
    }
}

fn apply(lang: Lang) {
    CURRENT.with(|c| c.set(lang));
    if let Some(root) =
        web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element())
    {
        let tag = match lang {
            Lang::Zh => "zh-CN",
            Lang::En => "en",
        };
        // Failing to set the lang attribute only affects hyphenation and
        // screen-reader voice, never the UI itself.
        let _ = root.set_attribute("lang", tag);
    }
}
