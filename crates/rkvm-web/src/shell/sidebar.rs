use leptos::prelude::*;

use crate::i18n::{Lang, use_i18n};
use crate::theme::Theme;
use crate::ui::{Icon, IconButton, IconName};

/// Left navigation. Collapsed it is a 52px icon rail; expanded (256px) it
/// floats over the stage so the video underneath keeps its size.
#[component]
pub fn Sidebar(
    open: RwSignal<bool>,
    #[prop(into)] connected: Signal<bool>,
    show_settings: RwSignal<bool>,
    show_terminal: RwSignal<bool>,
    /// Connection label and dot class ("on", "warn" or "").
    #[prop(into)]
    status: Signal<(&'static str, &'static str)>,
    theme: Theme,
) -> impl IntoView {
    let i = use_i18n();
    let host = web_sys::window().and_then(|w| w.location().host().ok()).unwrap_or_default();
    let on_desktop = move || !show_settings.get() && !show_terminal.get();
    let disabled = move || !connected.get();

    view! {
        <Show when=move || open.get()>
            <div class="sb-scrim" on:click=move |_| open.set(false)></div>
        </Show>
        <nav class="sidebar" class:open=move || open.get() aria-label=move || i.t("主导航", "Main")>
            <div class="sb-panel">
                <div class="sb-top">
                    <button
                        type="button"
                        class="sb-logo"
                        title=move || i.t("展开侧边栏", "Expand sidebar")
                        on:click=move |_| open.update(|o| *o = !*o)
                    >
                        <Logo/>
                    </button>
                    <span class="sb-brand sb-label">"rustkvm"</span>
                    <IconButton
                        icon=IconName::Sidebar
                        label=Signal::derive(move || i.t("收起侧边栏", "Collapse sidebar"))
                        on:click=move |_| open.set(false)
                    />
                </div>

                <div class="sb-nav">
                    <button
                        type="button"
                        class="sb-item"
                        class:on=move || connected.get() && on_desktop()
                        disabled=disabled
                        title=move || i.t("远程桌面", "Remote desktop")
                        on:click=move |_| {
                            show_settings.set(false);
                            show_terminal.set(false);
                            open.set(false);
                        }
                    >
                        <Icon name=IconName::Monitor/>
                        <span class="sb-label">{move || i.t("远程桌面", "Remote desktop")}</span>
                    </button>
                    <button
                        type="button"
                        class="sb-item"
                        class:on=move || show_terminal.get()
                        disabled=disabled
                        title=move || i.t("设备终端", "Device terminal")
                        on:click=move |_| {
                            show_terminal.update(|s| *s = !*s);
                            open.set(false);
                        }
                    >
                        <Icon name=IconName::Terminal/>
                        <span class="sb-label">{move || i.t("设备终端", "Device terminal")}</span>
                        <span class="sb-meta sb-label">"root"</span>
                    </button>
                    <button
                        type="button"
                        class="sb-item"
                        class:on=move || show_settings.get()
                        disabled=disabled
                        title=move || i.t("设置", "Settings")
                        on:click=move |_| {
                            show_settings.update(|s| *s = !*s);
                            open.set(false);
                        }
                    >
                        <Icon name=IconName::Sliders/>
                        <span class="sb-label">{move || i.t("设置", "Settings")}</span>
                    </button>
                </div>

                <div class="sb-foot">
                    <button
                        type="button"
                        class="sb-item"
                        title=move || i.t("切换浅色 / 深色", "Switch light / dark")
                        on:click=move |_| theme.toggle()
                    >
                        {move || {
                            let name = if theme.is_dark() { IconName::Sun } else { IconName::Moon };
                            view! { <Icon name=name/> }
                        }}
                        <span class="sb-label">
                            {move || {
                                if theme.is_dark() {
                                    i.t("浅色模式", "Light mode")
                                } else {
                                    i.t("深色模式", "Dark mode")
                                }
                            }}
                        </span>
                    </button>
                    <button
                        type="button"
                        class="sb-item"
                        title="中文 / English"
                        on:click=move |_| i.toggle()
                    >
                        <Icon name=IconName::Languages/>
                        <span class="sb-label">
                            {move || if i.lang() == Lang::Zh { "English" } else { "中文" }}
                        </span>
                    </button>
                    <div class="sb-status" title=move || status.get().0>
                        <span class=move || format!("dot {}", status.get().1)></span>
                        <span class="sb-host sb-label">
                            <span>{move || status.get().0}</span>
                            <small>{host}</small>
                        </span>
                    </div>
                </div>
            </div>
        </nav>
    }
}

/// Product mark: a screen with a cursor, in the brand clay color.
#[component]
fn Logo() -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="2" y="4" width="20" height="13" rx="3" fill="currentColor"></rect>
            <rect x="8" y="19" width="8" height="1.8" rx=".9" fill="currentColor"></rect>
            <path d="M10 8.2v6.2l1.7-1.6 1.2 2.4 1.1-.5-1.2-2.4h2.3z" fill="#fff"></path>
        </svg>
    }
}
