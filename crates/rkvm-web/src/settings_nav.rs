//! Navigation of the settings dialog: the pages in the left column, which
//! section belongs to which page, and the search box. Sections stay mounted
//! (their signals keep loading and saving); the nav only decides which are
//! shown.

use leptos::prelude::*;

use crate::i18n::I18n;
use crate::ui::{Icon, IconName};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Video,
    Input,
    Media,
    Network,
    Security,
    Hardware,
    System,
}

const PAGES: &[(Page, &str, &str, IconName)] = &[
    (Page::General, "通用", "General", IconName::Sliders),
    (Page::Video, "视频", "Video", IconName::Monitor),
    (Page::Input, "键盘与鼠标", "Keyboard & mouse", IconName::Keyboard),
    (Page::Media, "虚拟媒体", "Virtual media", IconName::Disc),
    (Page::Network, "网络", "Network", IconName::Globe),
    (Page::Security, "访问与安全", "Access & security", IconName::Shield),
    (Page::Hardware, "硬件", "Hardware", IconName::Cpu),
    (Page::System, "系统", "System", IconName::Info),
];

/// Every section of the dialog: id, page, and the title search matches.
const SECTIONS: &[(&str, Page, &str, &str)] = &[
    ("appearance", Page::General, "外观", "Appearance"),
    ("language", Page::General, "语言", "Language"),
    ("codec", Page::Video, "视频编码", "Video codec"),
    ("rotation", Page::Video, "屏幕旋转", "Display rotation"),
    ("edid", Page::Video, "EDID", "EDID"),
    ("layout", Page::Input, "键盘布局", "Keyboard layout"),
    ("usb", Page::Input, "USB 设备", "USB devices"),
    ("macros", Page::Input, "键盘宏", "Keyboard macros"),
    ("media", Page::Media, "虚拟媒体", "Virtual media"),
    ("network", Page::Network, "网络", "Network"),
    ("ip", Page::Network, "IP 地址", "IP address"),
    ("wol", Page::Network, "网络唤醒", "Wake-on-LAN"),
    ("mqtt", Page::Network, "MQTT", "MQTT"),
    ("cloud", Page::Network, "云服务", "Cloud"),
    ("tailscale", Page::Network, "Tailscale", "Tailscale"),
    ("password", Page::Security, "密码", "Password"),
    ("ssh", Page::Security, "SSH 访问", "SSH access"),
    ("https", Page::Security, "HTTPS 证书", "HTTPS certificate"),
    ("brightness", Page::Hardware, "设备屏幕亮度", "Device screen brightness"),
    ("features", Page::Hardware, "功能", "Features"),
    ("power", Page::Hardware, "电源", "Power"),
    ("extension", Page::Hardware, "扩展", "Extension"),
    ("serial", Page::Hardware, "串口控制台", "Serial console"),
    ("reboot", Page::System, "重启设备", "Reboot device"),
    ("firmware", Page::System, "固件更新", "Firmware update"),
    ("timezone", Page::System, "时区", "Time zone"),
    ("log", Page::System, "日志级别", "Log level"),
    ("developer", Page::System, "开发者", "Developer"),
    ("about", Page::System, "关于", "About"),
    ("danger", Page::System, "危险操作", "Danger zone"),
];

fn page_info(page: Page) -> (&'static str, &'static str, IconName) {
    PAGES.iter().find(|p| p.0 == page).map(|p| (p.1, p.2, p.3)).unwrap_or(("", "", IconName::Info))
}

fn hit(q: &str, zh: &str, en: &str) -> bool {
    zh.contains(q) || en.to_lowercase().contains(&q.to_lowercase())
}

#[derive(Clone, Copy)]
pub struct SettingsNav {
    pub page: RwSignal<Page>,
    pub query: RwSignal<String>,
}

impl SettingsNav {
    pub fn new() -> Self {
        Self { page: RwSignal::new(Page::General), query: RwSignal::new(String::new()) }
    }

    fn searching(self) -> Option<String> {
        let q = self.query.get().trim().to_string();
        (!q.is_empty()).then_some(q)
    }

    fn shows(self, id: &str) -> bool {
        let Some(&(_, page, zh, en)) = SECTIONS.iter().find(|s| s.0 == id) else {
            return false;
        };
        match self.searching() {
            None => self.page.get() == page,
            Some(q) => {
                let (pzh, pen, _) = page_info(page);
                hit(&q, zh, en) || hit(&q, pzh, pen)
            }
        }
    }

    /// For `class:off=` on a section: true while the section is not shown.
    pub fn off(self, id: &'static str) -> impl Fn() -> bool + Copy + 'static {
        move || !self.shows(id)
    }

    fn no_results(self) -> bool {
        self.searching().is_some() && !SECTIONS.iter().any(|s| self.shows(s.0))
    }
}

/// Left column: search box and the page list.
#[component]
pub fn SettingsSidebar(nav: SettingsNav, i: I18n) -> impl IntoView {
    view! {
        <div class="set-nav">
            <label class="set-search">
                <Icon name=IconName::Search/>
                <input
                    type="search"
                    placeholder=move || i.t("搜索设置", "Search settings")
                    aria-label=move || i.t("搜索设置", "Search settings")
                    prop:value=move || nav.query.get()
                    on:input=move |ev| nav.query.set(event_target_value(&ev))
                />
            </label>
            <div class="set-group">{move || i.t("设置", "Settings")}</div>
            {PAGES
                .iter()
                .map(|&(page, zh, en, icon)| {
                    view! {
                        <button
                            type="button"
                            class="set-item"
                            class:on=move || nav.query.get().trim().is_empty() && nav.page.get() == page
                            on:click=move |_| {
                                nav.query.set(String::new());
                                nav.page.set(page);
                            }
                        >
                            <Icon name=icon/>
                            <span>{move || i.t(zh, en)}</span>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Heading of the right column: the page name, or the search state.
#[component]
pub fn SettingsHeading(nav: SettingsNav, i: I18n) -> impl IntoView {
    view! {
        <h2 class="set-title">
            {move || {
                if nav.searching().is_some() {
                    i.t("搜索结果", "Search results")
                } else {
                    let (zh, en, _) = page_info(nav.page.get());
                    i.t(zh, en)
                }
            }}
        </h2>
        <Show when=move || nav.no_results()>
            <p class="set-empty">{move || i.t("没有匹配的设置。", "No settings match.")}</p>
        </Show>
    }
}
