use std::cell::Cell;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};
use web_sys::HtmlVideoElement;

use crate::hid::MouseMode;
use crate::net::{peer, rest};
use crate::rpc::RpcClient;

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
    let video_ref = NodeRef::<html::Video>::new();

    let checking = RwSignal::new(true);
    let is_setup = RwSignal::new(true);
    let needs_login = RwSignal::new(false);
    let conn_state = RwSignal::new("idle".to_string());
    let error = RwSignal::new(Option::<String>::None);
    let password = RwSignal::new(String::new());

    let setup_mode = RwSignal::new("noPassword".to_string());
    let setup_pw = RwSignal::new(String::new());

    let video_info = RwSignal::new(VideoInfo::default());
    let leds = RwSignal::new(Leds::default());
    let audio_on = RwSignal::new(false);
    let quality = RwSignal::new(1.0_f64);
    let mouse_relative = RwSignal::new(false);

    let show_settings = RwSignal::new(false);
    let show_terminal = RwSignal::new(false);
    let toolbar_open = RwSignal::new(false);
    let ocr_text = RwSignal::new(Option::<String>::None);

    let rpc_store = StoredValue::new_local(None::<Rc<RpcClient>>);
    let peer_store = StoredValue::new_local(None::<Rc<peer::Peer>>);
    let video_store = StoredValue::new_local(None::<HtmlVideoElement>);
    let mouse_mode = StoredValue::new_local(Rc::new(Cell::new(false)) as MouseMode);

    Effect::new(move |_| {
        spawn_local(async move {
            let setup_done = rest::is_setup().await;
            is_setup.set(setup_done);
            if setup_done {
                needs_login.set(rest::auth_required().await);
            }
            checking.set(false);
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
                Ok(()) => needs_login.set(false),
                Err(e) => error.set(Some(e)),
            }
        });
    };

    let do_connect = move |_| {
        error.set(None);
        conn_state.set("connecting".to_string());
        spawn_local(async move {
            let Some(video) = video_ref.get_untracked() else {
                error.set(Some("video element not mounted".to_string()));
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

                    rpc_store.set_value(Some(client.clone()));
                    peer_store.set_value(Some(peer.clone()));
                    video_store.set_value(Some(video.clone()));
                    crate::hid::attach(
                        video.clone(),
                        peer.clone(),
                        client.clone(),
                        mouse_mode.get_value(),
                    );
                    conn_state.set("connected".to_string());

                    spawn_local(async move {
                        if let Ok(v) = client.call("getVideoState", Value::Null).await {
                            video_info.set(parse_video(&v));
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
        let target = (quality.get_untracked() + delta).clamp(0.1, 1.0);
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
        mouse_relative.set(next);
        mouse_mode.get_value().set(next);
    };

    let do_reboot = move |_| {
        let confirmed = web_sys::window()
            .and_then(|w| w.confirm_with_message("Reboot the device?").ok())
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        let Some(client) = rpc_store.get_value() else { return };
        spawn_local(async move {
            if let Err(e) = client.call("reboot", json!({ "force": false })).await {
                error.set(Some(e.to_string()));
            }
        });
    };

    let send_cad = move |_| {
        if let Some(p) = peer_store.get_value() {
            crate::hid::send_key_combo(&p, 0x05, &[0x4c]);
        }
    };

    let go_fullscreen = move |_| {
        if let Some(el) =
            web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element())
        {
            let _ = el.request_fullscreen();
        }
    };

    let do_ocr = move |_| {
        let Some(video) = video_store.get_value() else { return };
        ocr_text.set(Some("scanning…".to_string()));
        spawn_local(async move {
            match crate::ocr::recognize(&video).await {
                Ok(text) => ocr_text.set(Some(text)),
                Err(e) => ocr_text.set(Some(e)),
            }
        });
    };

    let hide_timer = StoredValue::new_local(None::<Timeout>);
    let cancel_hide = move |_| hide_timer.set_value(None);
    let schedule_hide = move |_| {
        hide_timer.set_value(Some(Timeout::new(600, move || toolbar_open.set(false))));
    };

    view! {
        <video
            id="kvm-video"
            node_ref=video_ref
            autoplay=true
            playsinline=true
            muted=true
            on:mousedown=move |_| toolbar_open.set(false)
        ></video>

        <Show when=move || conn_state.get() == "connected">
            <button
                class="tb-handle"
                on:click=move |_| toolbar_open.update(|o| *o = !*o)
                on:mouseenter=cancel_hide
                on:mouseleave=schedule_hide
            >
                {move || if toolbar_open.get() { "▲" } else { "▾" }}
            </button>

            <div
                class="toolbar"
                class:open=move || toolbar_open.get()
                on:mouseenter=cancel_hide
                on:mouseleave=schedule_hide
            >
                <span class="tb-stat">
                    {move || {
                        let v = video_info.get();
                        if v.width > 0 {
                            format!("{}×{} @ {:.0}fps", v.width, v.height, v.fps)
                        } else {
                            "no signal".to_string()
                        }
                    }}
                </span>
                <span class="tb-leds">
                    <span class:on=move || leds.get().num title="Num Lock">"NUM"</span>
                    <span class:on=move || leds.get().caps title="Caps Lock">"CAPS"</span>
                    <span class:on=move || leds.get().scroll title="Scroll Lock">"SCR"</span>
                </span>

                <span class="tb-sep"></span>

                <button on:click=toggle_mouse title="Switch absolute / relative pointer">
                    {move || if mouse_relative.get() { "Mouse: Rel" } else { "Mouse: Abs" }}
                </button>
                <span class="tb-group">
                    <button on:click=quality_down title="Lower stream quality">"−"</button>
                    <span class="tb-stat">{move || format!("{:.0}%", quality.get() * 100.0)}</span>
                    <button on:click=quality_up title="Raise stream quality">"+"</button>
                </span>
                <button class:active=move || audio_on.get() on:click=toggle_audio title="Toggle device audio">
                    {move || if audio_on.get() { "Audio: on" } else { "Audio: off" }}
                </button>

                <span class="tb-sep"></span>

                <button on:click=send_cad title="Send Ctrl+Alt+Del">"C·A·D"</button>
                <button on:click=go_fullscreen title="Fullscreen">"⛶"</button>
                <button on:click=do_ocr title="Copy text from screen (OCR)">"OCR"</button>
                <button on:click=move |_| show_terminal.update(|s| *s = !*s) title="Serial terminal">"›_"</button>
                <button on:click=move |_| show_settings.update(|s| *s = !*s) title="Settings">"⚙"</button>

                <span class="tb-sep"></span>

                <button class="tb-danger" on:click=do_reboot title="Reboot device">"⏻"</button>
                <span class="tb-err">{move || error.get().unwrap_or_default()}</span>
            </div>

            <crate::settings::Settings open=show_settings rpc=rpc_store peer=peer_store />
            <crate::terminal::Terminal open=show_terminal peer=peer_store />
            <Show when=move || ocr_text.get().is_some()>
                <div class="panel">
                    <div class="card">
                        <h1>"Recognized text"</h1>
                        <textarea rows="10" readonly=true prop:value=move || {
                            ocr_text.get().unwrap_or_default()
                        }></textarea>
                        <button on:click=move |_| ocr_text.set(None)>"Close"</button>
                    </div>
                </div>
            </Show>
        </Show>

        <Show when=move || !checking.get() && !is_setup.get()>
            <div class="panel">
                <div class="card">
                    <h1>"Device setup"</h1>
                    <select
                        prop:value=move || setup_mode.get()
                        on:change=move |ev| setup_mode.set(event_target_value(&ev))
                    >
                        <option value="noPassword">"No password"</option>
                        <option value="password">"Password"</option>
                    </select>
                    <Show when=move || setup_mode.get() == "password">
                        <input
                            type="password"
                            placeholder="Set a password"
                            prop:value=move || setup_pw.get()
                            on:input=move |ev| setup_pw.set(event_target_value(&ev))
                        />
                    </Show>
                    <button on:click=do_setup>"Complete setup"</button>
                    <div class="err">{move || error.get().unwrap_or_default()}</div>
                </div>
            </div>
        </Show>

        <Show when=move || !checking.get() && is_setup.get() && needs_login.get()>
            <div class="panel">
                <div class="card">
                    <h1>"Sign in"</h1>
                    <input
                        type="password"
                        placeholder="Device password"
                        prop:value=move || password.get()
                        on:input=move |ev| password.set(event_target_value(&ev))
                    />
                    <button on:click=do_login>"Log in"</button>
                    <div class="err">{move || error.get().unwrap_or_default()}</div>
                </div>
            </div>
        </Show>

        <Show when=move || {
            !checking.get()
                && is_setup.get()
                && !needs_login.get()
                && conn_state.get() != "connected"
        }>
            <div class="panel">
                <div class="card">
                    <h1>"rustkvm"</h1>
                    <button
                        on:click=do_connect
                        disabled=move || conn_state.get() == "connecting"
                    >
                        <Show when=move || conn_state.get() == "connecting">
                            <span class="spinner"></span>
                        </Show>
                        {move || {
                            if conn_state.get() == "connecting" { "Connecting…" } else { "Connect" }
                        }}
                    </button>
                    <div class="err">{move || error.get().unwrap_or_default()}</div>
                </div>
            </div>
        </Show>
    }
}

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
