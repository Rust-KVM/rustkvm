use leptos::prelude::*;

use super::{Icon, IconName};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    /// Filled inverse: the one main action of a card.
    #[default]
    Primary,
    /// Hairline outline: secondary actions next to a primary one.
    Secondary,
    /// No chrome until hover: toolbar and in-card actions.
    Ghost,
}

impl Variant {
    fn class(self) -> &'static str {
        match self {
            Self::Primary => "btn-primary",
            Self::Secondary => "btn-secondary",
            Self::Ghost => "btn-ghost",
        }
    }
}

/// Text button, 36px (32px with `small`). Event listeners go on the component
/// (`on:click=...`) and land on the `<button>`.
#[component]
pub fn Button(
    #[prop(optional)] variant: Variant,
    #[prop(optional)] small: bool,
    #[prop(optional)] block: bool,
    #[prop(into, optional)] disabled: MaybeProp<bool>,
    #[prop(into, optional)] title: MaybeProp<&'static str>,
    #[prop(into, optional)] active: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    let class = format!(
        "btn {}{}{}",
        variant.class(),
        if small { " sm" } else { "" },
        if block { " block" } else { "" }
    );
    view! {
        <button
            type="button"
            class=class
            class:on=move || active.get().unwrap_or(false)
            title=move || title.get()
            disabled=move || disabled.get().unwrap_or(false)
        >
            {children()}
        </button>
    }
}

/// Square icon-only button, 32px (28px with `small`). `label` is the tooltip
/// and the accessible name, so it is required.
#[component]
pub fn IconButton(
    #[prop(into)] icon: Signal<IconName>,
    #[prop(into)] label: Signal<&'static str>,
    #[prop(optional)] small: bool,
    #[prop(optional)] danger: bool,
    #[prop(into, optional)] active: MaybeProp<bool>,
    #[prop(into, optional)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let class = format!(
        "icon-btn{}{}",
        if small { " sm" } else { "" },
        if danger { " danger" } else { "" }
    );
    view! {
        <button
            type="button"
            class=class
            class:on=move || active.get().unwrap_or(false)
            title=move || label.get()
            aria-label=move || label.get()
            aria-pressed=move || active.get().map(|a| a.to_string())
            disabled=move || disabled.get().unwrap_or(false)
        >
            {move || view! { <Icon name=icon.get()/> }}
        </button>
    }
}
