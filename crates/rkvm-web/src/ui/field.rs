use leptos::prelude::*;

use super::{Icon, IconName};

/// Label above a form control. The control is the child, so any input,
/// select or textarea gets the same spacing and focus ring.
#[component]
pub fn Field(#[prop(into)] label: Signal<&'static str>, children: Children) -> impl IntoView {
    view! {
        <label class="field">
            <span class="field-label">{move || label.get()}</span>
            {children()}
        </label>
    }
}

/// Tinted notice with a leading icon. Only the danger tone exists so far: the
/// no-password warning.
#[component]
pub fn Callout(#[prop(optional, into)] class: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class=format!("callout callout-danger {class}") role="alert">
            <Icon name=IconName::Alert/>
            <div class="callout-body">{children()}</div>
        </div>
    }
}
