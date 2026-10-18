//! Light / dark theme. The inline script in index.html applies the saved
//! choice (or the OS setting) before first paint; this module flips it at
//! runtime. Without a saved choice the page keeps following the OS.

use leptos::prelude::*;

const STORAGE_KEY: &str = "rkvm.theme";

/// Whether the dark palette is showing. Provided once by [`provide`].
#[derive(Clone, Copy)]
pub struct Theme(RwSignal<bool>);

impl Theme {
    pub fn is_dark(self) -> bool {
        self.0.get()
    }

    pub fn toggle(self) {
        self.set_dark(!self.0.get_untracked());
    }

    pub fn set_dark(self, dark: bool) {
        let name = if dark { "dark" } else { "light" };
        if let Some(root) = root() {
            // Both attributes are read only by CSS and the boot script; a
            // failure leaves the old theme showing, nothing breaks.
            let _ = root.set_attribute("data-theme-choice", name);
            let _ = root.set_attribute("data-theme", name);
        }
        crate::util::store_set(STORAGE_KEY, name);
        self.0.set(dark);
    }
}

pub fn provide() -> Theme {
    let dark = root().and_then(|r| r.get_attribute("data-theme")).is_some_and(|t| t == "dark");
    let theme = Theme(RwSignal::new(dark));
    provide_context(theme);
    theme
}

fn root() -> Option<web_sys::Element> {
    web_sys::window()?.document()?.document_element()
}
