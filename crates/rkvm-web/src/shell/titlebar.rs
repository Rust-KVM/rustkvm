use leptos::prelude::*;

/// Top bar of the content column: page title and a status chip on the left,
/// page actions (the children) on the right. In focus mode CSS turns it into
/// an overlay that only shows while `peek` is set.
#[component]
pub fn Titlebar(
    #[prop(into)] title: Signal<&'static str>,
    #[prop(into)] chip: Signal<Option<String>>,
    #[prop(into)] peek: Signal<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <header class="titlebar" class:peek=move || peek.get()>
            <div class="tb-left">
                <span class="tb-title">{move || title.get()}</span>
                {move || chip.get().map(|c| view! { <span class="chip">{c}</span> })}
            </div>
            <div class="tb-right">{children()}</div>
        </header>
    }
}
