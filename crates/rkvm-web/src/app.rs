use std::cell::Cell;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};
use web_sys::HtmlVideoElement;

use crate::hid::{MouseMode, Session};
use crate::net::{peer, rest};
use crate::rpc::RpcClient;
use crate::shell::{Sidebar, Titlebar};
use crate::ui::{Button, Callout, Field, Icon, IconButton, IconName, Variant};

#[derive(Clone, Copy, Default, PartialEq)]
struct VideoInfo {
    width: u32,
    height: u32,
    fps: f64,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Leds {
    num: bool,
    caps: bool,
    scroll: bool,
}

#[component]
pub fn App() -> impl IntoView {
    let i = crate::i18n::provide();
    let theme = crate::theme::provide();
    let video_ref = NodeRef::<html::Video>::new();

    let checking = RwSignal::new(true);
    // Set once the page knows no sign-in is needed; an effect below connects.
    let connect_on_load = RwSignal::new(false);
    let is_setup = RwSignal::new(true);
    let needs_login = RwSignal::new(false);
    let no_password = RwSignal::new(false);
    let nopw_dismissed = RwSignal::new(false);
    let conn_state = RwSignal::new("idle".to_string());
    let error = RwSignal::new(Option::<String>::None);
    crate::util::set_error_sink(error);
    let password = RwSignal::new(String::new());

    let setup_mode = RwSignal::new("noPassword".to_string());
    let setup_pw = RwSignal::new(String::new());

    let video_info = RwSignal::new(VideoInfo::default());
    let leds = RwSignal::new(Leds::default());
    let audio_on = RwSignal::new(false);
    let quality = RwSignal::new(1.0_f64);
    let mouse_relative = RwSignal::new(false);

    let show_settings = RwSignal::new(false);
    let settings_nav = crate::settings_nav::SettingsNav::new();
    // Opens Settings on the page that holds the password section.
    let open_password = move |_| {
        settings_nav.show(crate::settings_nav::Page::Security);
        show_settings.set(true);
    };
    let show_terminal = RwSignal::new(false);
    let sidebar_open = RwSignal::new(false);
    // Focus mode hides the sidebar and titlebar; `peek` slides the titlebar
    // back in as an overlay while the pointer is on it.
    let focus_mode = RwSignal::new(false);
    let peek = RwSignal::new(false);
    let ocr_view = RwSignal::new(Option::<crate::ocr::OcrView>::None);
    // Whether a read has finished in this page, i.e. the models are loaded.
    let ocr_loaded = RwSignal::new(false);

    let rpc_store = StoredValue::new_local(None::<Rc<RpcClient>>);
    let peer_store = StoredValue::new_local(None::<Rc<peer::Peer>>);
    let video_store = StoredValue::new_local(None::<HtmlVideoElement>);
    let mouse_mode = StoredValue::new_local(Rc::new(Cell::new(false)) as MouseMode);
    let hid_session = StoredValue::new_local(Session::default());
    let hid_attached = StoredValue::new_local(false);
    let conn_gen = StoredValue::new_local(Rc::new(Cell::new(0_u64)));
    // Set when the device reports that a newer browser took the session.
    let taken_over = RwSignal::new(false);
    // Automatic reconnect attempt in progress (1-based), 0 when idle.
    let reconnecting = RwSignal::new(0_u32);
    let retry_timer = StoredValue::new_local(None::<Timeout>);

    Effect::new(move |_| {
        spawn_local(async move {
            let setup_done = rest::is_setup().await;
            is_setup.set(setup_done);
            if setup_done {
                needs_login.set(rest::auth_required().await);
                no_password.set(rest::no_password().await);
            }
            checking.set(false);
            // Nothing to choose on the connect card: go straight to the screen.
            if setup_done && !needs_login.get_untracked() {
                connect_on_load.set(true);
            }
        });
    });

    let do_setup = move |_| {
        error.set(None);
        let mode = setup_mode.get();
        let pw = (mode == "password").then(|| setup_pw.get());
        spawn_local(async move {
            match rest::setup(&mode, pw.as_deref()).await {
                Ok(()) => {
                    is_setup.set(true);
                    needs_login.set(rest::auth_required().await);
                    no_password.set(rest::no_password().await);
                    if !needs_login.get_untracked() {
                        connect_on_load.set(true);
                    }
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };

    let do_login = move |_| {
        error.set(None);
        let pw = password.get();
        spawn_local(async move {
            match rest::login_local(&pw).await {
                Ok(()) => {
                    needs_login.set(false);
                    connect_on_load.set(true);
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };

    let connect = move || {
        // A second click before the button re-renders as disabled would open a
        // second session and get the first one kicked by the device.
        if conn_state.get_untracked() == "connecting" {
            return;
        }
        error.set(None);
        taken_over.set(false);
        conn_state.set("connecting".to_string());
        spawn_local(async move {
            let Some(video) = video_ref.get_untracked() else {
                error.set(Some(
                    i.t_now("页面未就绪，请刷新", "Page not ready: reload it").to_string(),
                ));
                conn_state.set("error".to_string());
                return;
            };
            match peer::connect(&video).await {
                Ok(peer) => {
                    let peer = Rc::new(peer);
                    let client = RpcClient::new(peer.rpc.clone());

                    client.on_event(move |method, params| {
                        let Some(params) = params else { return };
                        match method.as_str() {
                            "videoState" => video_info.set(parse_video(&params)),
                            "keyboardLedState" => leds.set(parse_leds(&params)),
                            _ => {}
                        }
                    });
                    // The device tells the old session before it hands the
                    // connection to a newer browser; that one must not be
                    // fought for by reconnecting.
                    client.on_event(move |method, _| {
                        if method == "otherSessionConnected" {
                            taken_over.set(true);
                        }
                    });

                    rpc_store.set_value(Some(client.clone()));
                    peer_store.set_value(Some(peer.clone()));
                    video_store.set_value(Some(video.clone()));
                    crate::hid::init_channels(&peer);
                    *hid_session.get_value().borrow_mut() = Some((peer.clone(), client.clone()));
                    if !hid_attached.get_value() {
                        crate::hid::attach(
                            video.clone(),
                            hid_session.get_value(),
                            mouse_mode.get_value(),
                        );
                        hid_attached.set_value(true);
                    }
                    conn_state.set("connected".to_string());

                    let generation = conn_gen.get_value();
                    let this_gen = generation.get() + 1;
                    generation.set(this_gen);
                    {
                        let peer = peer.clone();
                        spawn_local(async move {
                            let reason = peer::watch(&peer, || generation.get() != this_gen).await;
                            let Some(reason) = reason else { return };
                            hid_session.get_value().borrow_mut().take();
                            rpc_store.set_value(None);
                            peer_store.set_value(None);
                            peer.close();
                            video_info.set(VideoInfo::default());
                            show_settings.set(false);
                            show_terminal.set(false);
                            if taken_over.get_untracked() {
                                error.set(Some(
                                    i.t_now(
                                        "这台设备已在另一个浏览器窗口中打开，这里已断开。",
                                        "The device was opened in another browser window, so this one disconnected.",
                                    )
                                    .to_string(),
                                ));
                            } else {
                                error.set(Some(reason));
                                // Usually the device restarted or the network
                                // blinked: try again on our own.
                                reconnecting.set(1);
                            }
                            conn_state.set("lost".to_string());
                        });
                    }

                    spawn_local(async move {
                        if let Ok(v) = client.call("getVideoState", Value::Null).await {
                            video_info.set(parse_video(&v));
                        }
                        // LED changes arrive as events; fetch the current state so
                        // the indicators are right before the next host change.
                        if let Ok(v) = client.call("getKeyboardLedState", Value::Null).await {
                            leds.set(parse_leds(&v));
                        }
                        if let Ok(v) = client.call("getAudioConfig", Value::Null).await {
                            let enabled =
                                v.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                            audio_on.set(enabled);
                            video.set_muted(!enabled);
                        }
                        if let Ok(v) = client.call("getStreamQualityFactor", Value::Null).await
                            && let Some(f) = v.as_f64()
                        {
                            quality.set(f);
                        }
                    });
                }
                Err(e) => {
                    error.set(Some(e));
                    conn_state.set("error".to_string());
                }
            }
        });
    };

    let do_connect = move |_| {
        retry_timer.set_value(None);
        reconnecting.set(0);
        connect();
    };

    Effect::new(move |_| {
        if connect_on_load.get() {
            connect_on_load.set(false);
            connect();
        }
    });

    // Automatic reconnect after a lost connection: a few quick tries, then
    // every few seconds for about two minutes (a device restart takes ~10 s,
    // a reboot ~30 s). Stops on success, on a manual click and on takeover.
    Effect::new(move |_| {
        let state = conn_state.get();
        let attempt = reconnecting.get_untracked();
        if state == "connected" {
            reconnecting.set(0);
            return;
        }
        if attempt == 0 || !(state == "lost" || state == "error") {
            return;
        }
        if attempt > RECONNECT_TRIES {
            reconnecting.set(0);
            return;
        }
        let delay = (attempt.min(5) * 1000).max(1000);
        retry_timer.set_value(Some(Timeout::new(delay, move || {
            if reconnecting.get_untracked() == attempt {
                reconnecting.set(attempt + 1);
                connect();
            }
        })));
    });

    let toggle_audio = move |_| {
        let Some(client) = rpc_store.get_value() else { return };
        let next = !audio_on.get();
        let video = video_store.get_value();
        spawn_local(async move {
            match client.call("setAudioConfig", json!({ "params": { "enabled": next } })).await {
                Ok(res) => {
                    let enabled = res.get("enabled").and_then(Value::as_bool).unwrap_or(next);
                    audio_on.set(enabled);
                    if let Some(v) = video {
                        v.set_muted(!enabled);
                    }
                }
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    let adjust_quality = move |delta: f64| {
        let Some(client) = rpc_store.get_value() else { return };
        let current = quality.get_untracked();
        let target = ((current + delta).clamp(0.1, 1.0) * 10.0).round() / 10.0;
        if (target - current).abs() < 0.01 {
            return;
        }
        spawn_local(async move {
            match client.call("setStreamQualityFactor", json!({ "factor": target })).await {
                Ok(_) => quality.set(target),
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };
    let quality_up = move |_| adjust_quality(0.1);
    let quality_down = move |_| adjust_quality(-0.1);

    let toggle_mouse = move |_| {
        let next = !mouse_relative.get();
        if !next {
            mouse_relative.set(false);
            mouse_mode.get_value().set(false);
            return;
        }
        // Relative reports go to a separate USB function; without it the host
        // receives nothing, so only switch when the device has it enabled.
        let Some(client) = rpc_store.get_value() else { return };
        spawn_local(async move {
            match client.call("getUsbDevices", Value::Null).await {
                Ok(v) if v.get("relative_mouse").and_then(Value::as_bool).unwrap_or(false) => {
                    error.set(None);
                    mouse_relative.set(true);
                    mouse_mode.get_value().set(true);
                }
                Ok(_) => error.set(Some(
                    i.t_now(
                        "相对鼠标未启用：请先在 设置 → 键盘与鼠标 → USB 设备 中打开",
                        "Relative mouse is disabled: enable it in Settings → Keyboard & mouse → USB devices first",
                    )
                    .to_string(),
                )),
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    let send_cad = move |_| {
        if let Some(p) = peer_store.get_value() {
            crate::hid::send_key_combo(&p, 0x05, &[0x4c]);
        }
    };

    let go_fullscreen = move |_| {
        spawn_local(async move {
            if let Err(e) = crate::util::toggle_fullscreen().await {
                error.set(Some(e));
            }
        });
    };

    let ocr_copy_msg = RwSignal::new(String::new());
    let do_ocr = move |_| {
        let Some(client) = rpc_store.get_value() else { return };
        if ocr_view.get_untracked().is_some_and(|v| v.is_busy()) {
            return;
        }
        ocr_copy_msg.set(String::new());
        ocr_view.set(Some(crate::ocr::OcrView::Reading { first: !ocr_loaded.get_untracked() }));
        spawn_local(async move {
            let view = crate::ocr::recognize(&client).await;
            if matches!(view, crate::ocr::OcrView::Done { .. }) {
                ocr_loaded.set(true);
            }
            // The dialog may have been closed while the device was reading.
            if ocr_view.get_untracked().is_some() {
                ocr_view.set(Some(view));
            }
        });
    };

    let ocr_ref = NodeRef::<html::Div>::new();
    // Take focus when the dialog opens so Esc reaches it, not the host.
    Effect::new(move |_| {
        if let Some(el) = ocr_ref.get() {
            let _ = el.focus();
        }
    });
    let copy_ocr = move |_| {
        let Some(text) = ocr_view.get_untracked().and_then(|v| v.text().map(str::to_string)) else {
            return;
        };
        spawn_local(async move {
            ocr_copy_msg.set(match crate::util::copy_text(&text).await {
                Ok(()) => i.t_now("已复制", "Copied").to_string(),
                Err(e) => e,
            });
        });
    };

    // Toast errors are transient: clear each one after a few seconds unless a
    // newer error replaced it. Errors shown on the Connect card stay put.
    let error_timer = StoredValue::new_local(None::<Timeout>);
    Effect::new(move |_| {
        let Some(msg) = error.get() else { return };
        if conn_state.get_untracked() != "connected" {
            return;
        }
        error_timer.set_value(Some(Timeout::new(6000, move || {
            if error.get_untracked().as_deref() == Some(msg.as_str())
                && conn_state.get_untracked() == "connected"
            {
                error.set(None);
            }
        })));
    });

    let hide_timer = StoredValue::new_local(None::<Timeout>);
    let cancel_hide = move |_| hide_timer.set_value(None);
    let schedule_hide = move |_| {
        hide_timer.set_value(Some(Timeout::new(600, move || peek.set(false))));
    };

    let connected = Signal::derive(move || conn_state.get() == "connected");
    let status = Signal::derive(move || match conn_state.get().as_str() {
        "connected" => (i.t("已连接", "Connected"), "on"),
        "connecting" => (i.t("连接中…", "Connecting…"), "warn"),
        _ if reconnecting.get() > 0 => (i.t("正在重新连接…", "Reconnecting…"), "warn"),
        "lost" => (i.t("连接已断开", "Disconnected"), "warn"),
        "error" => (i.t("连接失败", "Connection failed"), ""),
        _ => (i.t("未连接", "Not connected"), ""),
    });
    let title =
        Signal::derive(
            move || {
                if connected.get() { i.t("远程桌面", "Remote desktop") } else { "" }
            },
        );
    let chip = Signal::derive(move || {
        connected.get().then(|| {
            let v = video_info.get();
            if v.width > 0 {
                format!("{}×{} · {:.0} fps", v.width, v.height, v.fps)
            } else {
                i.t("无信号", "No signal").to_string()
            }
        })
    });
    let host = StoredValue::new(
        web_sys::window().and_then(|w| w.location().host().ok()).unwrap_or_default(),
    );

    view! {
        <div class="app" class:focus=move || focus_mode.get() class:sb-open=move || sidebar_open.get()>
            <Sidebar
                open=sidebar_open
                connected=connected
                show_settings=show_settings
                show_terminal=show_terminal
                status=status
            />

            <main class="main">
                <Show when=move || focus_mode.get()>
                    <button
                        type="button"
                        class="focus-handle"
                        title=move || i.t("显示工具栏", "Show toolbar")
                        on:click=move |_| peek.update(|p| *p = !*p)
                        on:mouseenter=cancel_hide
                        on:mouseleave=schedule_hide
                    ></button>
                </Show>
                <Titlebar
                    title=title
                    chip=chip
                    peek=peek
                    on:mouseenter=cancel_hide
                    on:mouseleave=schedule_hide
                >
                    <Show when=move || connected.get()>
                        <span class="tb-leds">
                            <span class="badge" class:on=move || leds.get().num title="Num Lock">
                                "NUM"
                            </span>
                            <span class="badge" class:on=move || leds.get().caps title="Caps Lock">
                                "CAPS"
                            </span>
                            <span class="badge" class:on=move || leds.get().scroll title="Scroll Lock">
                                "SCR"
                            </span>
                        </span>
                        <span class="vsep"></span>
                        <Button
                            variant=Variant::Ghost
                            small=true
                            class="tb-mouse"
                            title=Signal::derive(move || {
                                i.t(
                                    "绝对：指针跟随你的鼠标位置；相对：像普通鼠标一样移动，适合游戏和 BIOS",
                                    "Absolute: the pointer follows yours. Relative: moves like a plain mouse, for games and BIOS",
                                )
                            })
                            on:click=toggle_mouse
                        >
                            <Icon name=IconName::Mouse/>
                            <span class="tb-mouse-label">
                                {move || {
                                    if mouse_relative.get() {
                                        i.t("鼠标：相对", "Mouse: relative")
                                    } else {
                                        i.t("鼠标：绝对", "Mouse: absolute")
                                    }
                                }}
                            </span>
                        </Button>
                        <span class="tb-quality">
                            <IconButton
                                icon=IconName::Minus
                                small=true
                                label=Signal::derive(move || i.t("降低画质", "Lower stream quality"))
                                disabled=Signal::derive(move || quality.get() <= 0.15)
                                on:click=quality_down
                            />
                            <span title=move || i.t("画质：降低可减少带宽", "Stream quality: lower it to save bandwidth")>
                                {move || format!("{} {:.0}%", i.t("画质", "Quality"), quality.get() * 100.0)}
                            </span>
                            <IconButton
                                icon=IconName::Plus
                                small=true
                                label=Signal::derive(move || i.t("提高画质", "Raise stream quality"))
                                disabled=Signal::derive(move || quality.get() >= 0.95)
                                on:click=quality_up
                            />
                        </span>
                        <IconButton
                            icon=Signal::derive(move || {
                                if audio_on.get() { IconName::Volume } else { IconName::VolumeOff }
                            })
                            label=Signal::derive(move || {
                                if audio_on.get() {
                                    i.t("关闭声音", "Mute device audio")
                                } else {
                                    i.t("打开声音", "Play device audio")
                                }
                            })
                            active=Signal::derive(move || audio_on.get())
                            on:click=toggle_audio
                        />
                        <span class="vsep"></span>
                        <Button
                            variant=Variant::Ghost
                            small=true
                            title=Signal::derive(move || i.t("向被控电脑发送 Ctrl+Alt+Del", "Send Ctrl+Alt+Del to the host"))
                            on:click=send_cad
                        >
                            "Ctrl+Alt+Del"
                        </Button>
                        <IconButton
                            icon=IconName::ScanText
                            label=Signal::derive(move || {
                                i.t("识别屏幕文字（OCR）", "Copy text from screen (OCR)")
                            })
                            on:click=do_ocr
                        />
                        <span class="vsep"></span>
                        <IconButton
                            icon=IconName::Focus
                            label=Signal::derive(move || {
                                if focus_mode.get() {
                                    i.t("退出专注模式", "Exit focus mode")
                                } else {
                                    i.t("专注模式：隐藏侧边栏和工具栏", "Focus mode: hide sidebar and toolbar")
                                }
                            })
                            active=Signal::derive(move || focus_mode.get())
                            on:click=move |_| {
                                focus_mode.update(|f| *f = !*f);
                                peek.set(false);
                                sidebar_open.set(false);
                            }
                        />
                        <IconButton
                            icon=IconName::Maximize
                            label=Signal::derive(move || i.t("全屏", "Fullscreen"))
                            on:click=go_fullscreen
                        />
                    </Show>
                </Titlebar>

                <div class="stage">
                    <video
                        id="kvm-video"
                        node_ref=video_ref
                        autoplay=true
                        playsinline=true
                        muted=true
                        tabindex="-1"
                        on:mousedown=move |_| {
                            peek.set(false);
                            // The HID handler cancels mousedown, which also
                            // cancels the focus move; take focus here so a
                            // toolbar button clicked earlier stops owning keys.
                            if let Some(v) = video_ref.get_untracked() {
                                let _ = v.focus();
                            }
                        }
                    ></video>

                    <Show when=move || {
                        connected.get() && no_password.get() && !nopw_dismissed.get() && !focus_mode.get()
                    }>
                        <Callout class="stage-callout">
                            <div class="callout-row">
                                <span>
                                    {move || i.t(
                                        "未设置密码：同一网络中的任何人都能控制这台电脑，并打开设备的 root 终端。",
                                        "No password set: anyone on this network can control this machine and open a root shell on the device.",
                                    )}
                                </span>
                                <Button small=true on:click=open_password>
                                    {move || i.t("设置密码", "Set password")}
                                </Button>
                                <IconButton
                                    icon=IconName::Close
                                    small=true
                                    label=Signal::derive(move || i.t("暂时隐藏", "Hide until reload"))
                                    on:click=move |_| nopw_dismissed.set(true)
                                />
                            </div>
                        </Callout>
                    </Show>

                    <Show when=move || connected.get() && error.get().is_some()>
                        <div class="toast" role="status">
                            <Icon name=IconName::Alert/>
                            <span title=move || error.get().unwrap_or_default()>
                                {move || error.get().unwrap_or_default()}
                            </span>
                            <IconButton
                                icon=IconName::Close
                                small=true
                                label=Signal::derive(move || i.t("关闭", "Dismiss"))
                                on:click=move |_| error.set(None)
                            />
                        </div>
                    </Show>

                    <Show when=move || connected.get()>
                        <crate::terminal::Terminal open=show_terminal peer=peer_store/>
                    </Show>

                    <Show when=move || !connected.get()>
                        <div class="gate">
                            {move || {
                                if checking.get() {
                                    view! { <span class="spinner" style="color:var(--text-tertiary)"></span> }
                                        .into_any()
                                } else if !is_setup.get() {
                                    view! {
                                        <div class="card">
                                            <h1 class="card-title">{move || i.t("设置设备", "Device setup")}</h1>
                                            <p class="card-sub">
                                                {move || i.t("选择登录方式，之后可以在设置里修改。", "Choose how to sign in. You can change it later in Settings.")}
                                            </p>
                                            <Field label=Signal::derive(move || i.t("登录方式", "Sign-in"))>
                                                <select
                                                    on:change=move |ev| setup_mode.set(event_target_value(&ev))
                                                >
                                                    <option value="noPassword" prop:selected=move || setup_mode.get() == "noPassword">
                                                        {move || i.t("不设密码（同一网络中任何人都能连接）", "No password (anyone on this network can connect)")}
                                                    </option>
                                                    <option value="password" prop:selected=move || setup_mode.get() == "password">{move || i.t("需要密码", "Require a password")}</option>
                                                </select>
                                            </Field>
                                            <Show when=move || setup_mode.get() == "password">
                                                <Field label=Signal::derive(move || i.t("密码", "Password"))>
                                                    <input
                                                        type="password"
                                                        autocomplete="new-password"
                                                        prop:value=move || setup_pw.get()
                                                        on:input=move |ev| setup_pw.set(event_target_value(&ev))
                                                    />
                                                </Field>
                                            </Show>
                                            <Button block=true on:click=do_setup>
                                                {move || i.t("完成设置", "Complete setup")}
                                            </Button>
                                            <div class="err">{move || error.get().unwrap_or_default()}</div>
                                        </div>
                                    }
                                        .into_any()
                                } else if needs_login.get() {
                                    view! {
                                        <div class="card">
                                            <h1 class="card-title">{move || i.t("登录", "Sign in")}</h1>
                                            <p class="card-sub">{move || i.t("输入设备密码。", "Enter the device password.")}</p>
                                            <Field label=Signal::derive(move || i.t("密码", "Password"))>
                                                <input
                                                    type="password"
                                                    autocomplete="current-password"
                                                    prop:value=move || password.get()
                                                    on:input=move |ev| password.set(event_target_value(&ev))
                                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                        if ev.key() == "Enter" {
                                                            do_login(());
                                                        }
                                                    }
                                                />
                                            </Field>
                                            <Button block=true on:click=move |_| do_login(())>
                                                {move || i.t("登录", "Log in")}
                                            </Button>
                                            <div class="err">{move || error.get().unwrap_or_default()}</div>
                                        </div>
                                    }
                                        .into_any()
                                } else {
                                    view! {
                                        <div class="card">
                                            <h1 class="card-title">"rustkvm"</h1>
                                            <p class="card-sub mono">{host.get_value()}</p>
                                            <Show when=move || no_password.get()>
                                                <Callout>
                                                    {move || i.t(
                                                        "未设置密码。同一网络中的任何人都能连接、控制这台电脑，并打开设备的 root 终端。连接后可在 设置 → 访问与安全 中设置密码。",
                                                        "No password is set. Anyone on this network can connect, control this machine and open a root shell on the device. After connecting, set one in Settings → Access & security.",
                                                    )}
                                                </Callout>
                                            </Show>
                                            <Button
                                                block=true
                                                disabled=Signal::derive(move || conn_state.get() == "connecting")
                                                on:click=do_connect
                                            >
                                                <Show when=move || conn_state.get() == "connecting" || reconnecting.get() != 0>
                                                    <span class="spinner"></span>
                                                </Show>
                                                {move || match conn_state.get().as_str() {
                                                    _ if reconnecting.get() > 0 => i.t("正在重新连接…", "Reconnecting…"),
                                                    "connecting" => i.t("连接中…", "Connecting…"),
                                                    "lost" if taken_over.get() => i.t("在这里继续使用", "Use it here instead"),
                                                    "lost" | "error" => i.t("重新连接", "Reconnect"),
                                                    _ => i.t("连接", "Connect"),
                                                }}
                                            </Button>
                                            <div class="err">{move || error.get().unwrap_or_default()}</div>
                                        </div>
                                    }
                                        .into_any()
                                }
                            }}
                        </div>
                    </Show>
                </div>
            </main>

            <Show when=move || connected.get()>
                <crate::settings::Settings
                    open=show_settings
                    rpc=rpc_store
                    peer=peer_store
                    no_password=no_password
                    theme=theme
                    nav=settings_nav
                />
            </Show>

            <Show when=move || ocr_view.get().is_some()>
                <div class="scrim"></div>
                <div
                    class="dialog"
                    role="dialog"
                    aria-modal="true"
                    aria-labelledby="ocr-title"
                    data-modal="1"
                    tabindex="-1"
                    node_ref=ocr_ref
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Escape" {
                            ocr_copy_msg.set(String::new());
                            ocr_view.set(None);
                        }
                    }
                >
                    <div class="card">
                        <h1 class="card-title" id="ocr-title">
                            {move || {
                                if ocr_view.get().is_some_and(|v| v.is_busy()) {
                                    i.t("正在识别屏幕文字", "Reading the screen")
                                } else {
                                    i.t("识别到的文字", "Recognized text")
                                }
                            }}
                        </h1>
                        <p class="card-sub ocr-status" aria-live="polite">
                            <Show when=move || ocr_view.get().is_some_and(|v| v.is_busy())>
                                <span class="spinner"></span>
                            </Show>
                            {move || {
                                let copied = ocr_copy_msg.get();
                                let status = ocr_view.get().map(|v| v.status(i)).unwrap_or_default();
                                if copied.is_empty() { status } else { format!("{status} · {copied}") }
                            }}
                        </p>
                        <Show when=move || ocr_view.get().is_some_and(|v| v.text().is_some())>
                            <textarea rows="10" readonly=true aria-labelledby="ocr-title" prop:value=move || {
                                ocr_view
                                    .get()
                                    .and_then(|v| v.text().map(str::to_string))
                                    .unwrap_or_default()
                            }></textarea>
                        </Show>
                        <div class="actions">
                            <Button
                                variant=Variant::Secondary
                                on:click=move |_| {
                                    ocr_copy_msg.set(String::new());
                                    ocr_view.set(None);
                                }
                            >
                                {move || i.t("关闭", "Close")}
                            </Button>
                            <Button
                                variant=Variant::Secondary
                                disabled=Signal::derive(move || {
                                    ocr_view.get().is_none_or(|v| v.is_busy())
                                })
                                on:click=do_ocr
                            >
                                {move || i.t("重新识别", "Read again")}
                            </Button>
                            <Button
                                disabled=Signal::derive(move || {
                                    ocr_view.get().is_none_or(|v| v.text().is_none())
                                })
                                on:click=copy_ocr
                            >
                                {move || i.t("复制", "Copy")}
                            </Button>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    }
}

/// Automatic reconnect attempts before the gate waits for a click.
const RECONNECT_TRIES: u32 = 30;

fn parse_video(p: &Value) -> VideoInfo {
    VideoInfo {
        width: p.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
        height: p.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
        fps: p.get("fps").and_then(Value::as_f64).unwrap_or(0.0),
    }
}

fn parse_leds(p: &Value) -> Leds {
    Leds {
        num: p.get("num_lock").and_then(Value::as_bool).unwrap_or(false),
        caps: p.get("caps_lock").and_then(Value::as_bool).unwrap_or(false),
        scroll: p.get("scroll_lock").and_then(Value::as_bool).unwrap_or(false),
    }
}
