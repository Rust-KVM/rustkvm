use gloo_timers::callback::Timeout;
use leptos::prelude::*;

use crate::i18n::{Lang, use_i18n};
use crate::theme::Theme;
use crate::ui::{Icon, IconButton, IconName};

/// Left navigation, as in Claude Desktop: a 52px icon rail. Hovering it
/// peeks the full 256px sidebar over the content and leaving hides it again;
/// clicking the logo pins it open, which pushes the content over (the video
/// rescales once, not per frame). On narrow windows a pinned sidebar floats.
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
    on_reboot: Callback<()>,
) -> impl IntoView {
    let i = use_i18n();
    let host = web_sys::window().and_then(|w| w.location().host().ok()).unwrap_or_default();
    let on_desktop = move || !show_settings.get() && !show_terminal.get();
    let disabled = move || !connected.get();

    // Hover peek with short enter/leave delays so passing the pointer over
    // the rail on the way to the video does not flash the panel.
    let peek = RwSignal::new(false);
    let timer = StoredValue::new_local(None::<Timeout>);
    let nav_ref = NodeRef::<leptos::html::Nav>::new();
    let enter = move |_| {
        if open.get_untracked() {
            return;
        }
        timer.set_value(Some(Timeout::new(PEEK_IN_MS, move || peek.set(true))));
    };
    let leave = move || {
        timer.set_value(Some(Timeout::new(PEEK_OUT_MS, move || {
            if !keyboard_inside(nav_ref) {
                peek.set(false);
            }
        })));
    };
    let pin = move |pinned: bool| {
        timer.set_value(None);
        peek.set(false);
        open.set(pinned);
    };

    view! {
        <Show when=move || open.get()>
            <div class="sb-scrim" on:click=move |_| open.set(false)></div>
        </Show>
        <nav
            class="sidebar"
            class:open=move || open.get()
            class:peek=move || peek.get() && !open.get()
            class:expanded=move || open.get() || peek.get()
            data-chrome="1"
            node_ref=nav_ref
            on:mouseenter=enter
            on:mouseleave=move |_| leave()
            on:focusin=move |ev: web_sys::FocusEvent| {
                let visible = ev
                    .target()
                    .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
                    .is_some_and(|el| el.matches(":focus-visible").unwrap_or(false));
                if visible {
                    peek.set(true);
                }
            }
            on:focusout=move |_| leave()
            aria-label=move || i.t("主导航", "Main")
        >
            <div class="sb-panel">
                <div class="sb-top">
                    <button
                        type="button"
                        class="sb-logo"
                        title=move || {
                            if open.get() {
                                i.t("收起侧边栏", "Collapse sidebar")
                            } else {
                                i.t("固定侧边栏", "Keep sidebar open")
                            }
                        }
                        aria-label=move || {
                            if open.get() {
                                i.t("收起侧边栏", "Collapse sidebar")
                            } else {
                                i.t("固定侧边栏", "Keep sidebar open")
                            }
                        }
                        aria-expanded=move || open.get().to_string()
                        on:click=move |_| pin(!open.get_untracked())
                    >
                        <Logo/>
                    </button>
                    <span class="sb-brand sb-label">"rustkvm"</span>
                    <IconButton
                        icon=IconName::Sidebar
                        label=Signal::derive(move || {
                            if open.get() {
                                i.t("收起侧边栏", "Collapse sidebar")
                            } else {
                                i.t("固定侧边栏", "Keep sidebar open")
                            }
                        })
                        active=Signal::derive(move || open.get())
                        on:click=move |_| pin(!open.get_untracked())
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
                        on:click=move |_| show_terminal.update(|s| *s = !*s)
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
                        on:click=move |_| show_settings.update(|s| *s = !*s)
                    >
                        <Icon name=IconName::Sliders/>
                        <span class="sb-label">{move || i.t("设置", "Settings")}</span>
                    </button>
                </div>

                <div class="sb-foot">
                    // Expanded: segmented controls that show the current
                    // choice. Collapsed: one icon each, tooltip says what
                    // is current and what a click does.
                    <div class="sb-pref sb-label" role="group" aria-label=move || i.t("外观", "Appearance")>
                        <span class="sb-pref-name">{move || i.t("外观", "Theme")}</span>
                        <div class="seg">
                            <button
                                type="button"
                                class:on=move || !theme.is_dark()
                                aria-pressed=move || (!theme.is_dark()).to_string()
                                on:click=move |_| theme.set_dark(false)
                            >
                                {move || i.t("浅色", "Light")}
                            </button>
                            <button
                                type="button"
                                class:on=move || theme.is_dark()
                                aria-pressed=move || theme.is_dark().to_string()
                                on:click=move |_| theme.set_dark(true)
                            >
                                {move || i.t("深色", "Dark")}
                            </button>
                        </div>
                    </div>
                    <div class="sb-pref sb-label" role="group" aria-label="语言 / Language">
                        <span class="sb-pref-name">{move || i.t("语言", "Language")}</span>
                        <div class="seg">
                            <button
                                type="button"
                                lang="zh-CN"
                                class:on=move || i.lang() == Lang::Zh
                                aria-pressed=move || (i.lang() == Lang::Zh).to_string()
                                on:click=move |_| i.set(Lang::Zh)
                            >
                                "中文"
                            </button>
                            <button
                                type="button"
                                lang="en"
                                class:on=move || i.lang() == Lang::En
                                aria-pressed=move || (i.lang() == Lang::En).to_string()
                                on:click=move |_| i.set(Lang::En)
                            >
                                "English"
                            </button>
                        </div>
                    </div>
                    <button
                        type="button"
                        class="sb-item sb-rail-only"
                        title=move || {
                            if theme.is_dark() {
                                i.t("外观：深色（点击切换为浅色）", "Theme: dark (click for light)")
                            } else {
                                i.t("外观：浅色（点击切换为深色）", "Theme: light (click for dark)")
                            }
                        }
                        on:click=move |_| theme.toggle()
                    >
                        {move || {
                            let name = if theme.is_dark() { IconName::Moon } else { IconName::Sun };
                            view! { <Icon name=name/> }
                        }}
                    </button>
                    <button
                        type="button"
                        class="sb-item sb-rail-only"
                        title=move || {
                            i.t("语言：中文（点击切换为 English）", "Language: English (click for 中文)")
                        }
                        on:click=move |_| i.toggle()
                    >
                        <Icon name=IconName::Languages/>
                    </button>
                    <button
                        type="button"
                        class="sb-item danger"
                        disabled=disabled
                        title=move || i.t("重启设备（会先确认）", "Reboot device (asks first)")
                        on:click=move |_| on_reboot.run(())
                    >
                        <Icon name=IconName::Power/>
                        <span class="sb-label">{move || i.t("重启设备", "Reboot device")}</span>
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

const PEEK_IN_MS: u32 = 120;
const PEEK_OUT_MS: u32 = 300;

/// Whether keyboard focus is in the sidebar, so a peek stays open while the
/// user tabs through it.
fn keyboard_inside(nav: NodeRef<leptos::html::Nav>) -> bool {
    let Some(nav) = nav.get_untracked() else { return false };
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .is_some_and(|el| nav.contains(Some(&el)) && el.matches(":focus-visible").unwrap_or(false))
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
