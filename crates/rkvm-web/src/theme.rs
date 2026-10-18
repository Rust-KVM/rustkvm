//! Theme: follow the OS, light or dark. The inline script in index.html
//! applies the saved choice (or the OS setting) before first paint and keeps
//! following the OS while no choice is set (no `data-theme-choice`); this
//! module changes the choice at runtime.

use leptos::prelude::*;

const STORAGE_KEY: &str = "rkvm.theme";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

/// The current choice. Provided once by [`provide`].
#[derive(Clone, Copy)]
pub struct Theme(RwSignal<ThemeChoice>);

impl Theme {
    pub fn choice(self) -> ThemeChoice {
        self.0.get()
    }

    pub fn set(self, choice: ThemeChoice) {
        // Attributes are read only by CSS and the boot script; a failure
        // leaves the old theme showing, nothing breaks.
        if let Some(root) = root() {
            let name = match choice {
                ThemeChoice::System => {
                    let _ = root.remove_attribute("data-theme-choice");
                    if os_dark() { "dark" } else { "light" }
                }
                ThemeChoice::Light => "light",
                ThemeChoice::Dark => "dark",
            };
            if choice != ThemeChoice::System {
                let _ = root.set_attribute("data-theme-choice", name);
            }
            let _ = root.set_attribute("data-theme", name);
        }
        let stored = match choice {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        };
        crate::util::store_set(STORAGE_KEY, stored);
        self.0.set(choice);
    }
}

pub fn provide() -> Theme {
    let choice = match root().and_then(|r| r.get_attribute("data-theme-choice")).as_deref() {
        Some("light") => ThemeChoice::Light,
        Some("dark") => ThemeChoice::Dark,
        _ => ThemeChoice::System,
    };
    let theme = Theme(RwSignal::new(choice));
    provide_context(theme);
    theme
}

fn root() -> Option<web_sys::Element> {
    web_sys::window()?.document()?.document_element()
}

fn os_dark() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
        .is_some_and(|m| m.matches())
}
