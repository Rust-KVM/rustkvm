use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use futures::future::join_all;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;

use crate::net::peer::Peer;
use crate::net::{rest, upload};
use crate::rpc::RpcClient;
use crate::util::{Rpc, as_string, fire, json_str};

const KEYBOARD_LAYOUTS: &[(&str, &str)] = &[
    ("en_US", "English (US)"),
    ("en_UK", "English (UK)"),
    ("de_DE", "German"),
    ("fr_FR", "French"),
    ("es_ES", "Spanish"),
    ("it_IT", "Italian"),
    ("ja_JP", "Japanese"),
];

#[component]
pub fn Settings(
    open: RwSignal<bool>,
    rpc: StoredValue<Option<Rc<RpcClient>>, LocalStorage>,
    peer: StoredValue<Option<Rc<Peer>>, LocalStorage>,
) -> impl IntoView {
    let codec = RwSignal::new(String::new());
    let rotation = RwSignal::new(String::new());
    let layout = RwSignal::new(String::new());
    let brightness = RwSignal::new(0_i64);
    let device_id = RwSignal::new(String::new());
    let version = RwSignal::new(String::new());

    let mount_url = RwSignal::new(String::new());
    let mount_mode = RwSignal::new("CDROM".to_string());
    let mount_state = RwSignal::new(String::new());
    let upload_state = RwSignal::new(String::new());

    let network = RwSignal::new(String::new());
    let update_state = RwSignal::new(String::new());
    let usb = RwSignal::new(UsbDevices::default());
    let edid = RwSignal::new(String::new());
    let wol_mac = RwSignal::new(String::new());

    let net_dhcp = RwSignal::new(true);
    let net_ip = RwSignal::new(String::new());
    let net_gw = RwSignal::new(String::new());
    let net_dns = RwSignal::new(String::new());

    let auto_update = RwSignal::new(false);
    let jiggler = RwSignal::new(false);
    let usb_emul = RwSignal::new(false);
    let host_idle = RwSignal::new(false);
    let video_sleep = RwSignal::new(-1_i64);
    let log_level = RwSignal::new("INFO".to_string());
    let ssh_key = RwSignal::new(String::new());

    let mqtt_enabled = RwSignal::new(false);
    let mqtt_broker = RwSignal::new(String::new());
    let mqtt_port = RwSignal::new(1883_i64);
    let mqtt_user = RwSignal::new(String::new());
    let mqtt_pass = RwSignal::new(String::new());
    let mqtt_topic = RwSignal::new("rustkvm".to_string());

    let old_pw = RwSignal::new(String::new());
    let new_pw = RwSignal::new(String::new());
    let pw_msg = RwSignal::new(String::new());

    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let Some(client) = rpc.get_value() else { return };
        spawn_local(async move {
            let client = &client;
            let loads: Vec<Pin<Box<dyn Future<Output = ()> + '_>>> = vec![
                Box::pin(async move {
                    if let Ok(v) = client.call("getVideoCodecPreference", Value::Null).await {
                        codec.set(as_string(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getDisplayRotation", Value::Null).await {
                        rotation.set(json_str(&v, "rotation"));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getKeyboardLayout", Value::Null).await {
                        layout.set(as_string(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getBacklightSettings", Value::Null).await {
                        brightness
                            .set(v.get("max_brightness").and_then(Value::as_i64).unwrap_or(0));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getDeviceID", Value::Null).await {
                        device_id.set(as_string(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getLocalVersion", Value::Null).await {
                        let app = json_str(&v, "app_version");
                        let sys = json_str(&v, "system_version");
                        version.set(format!("app {app} · system {sys}"));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getVirtualMediaState", Value::Null).await {
                        mount_state.set(describe_mount(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getNetworkState", Value::Null).await {
                        let ip = json_str(&v, "ip");
                        let online = v.get("online").and_then(Value::as_bool).unwrap_or(false);
                        network.set(format!(
                            "{} ({})",
                            if ip.is_empty() { "—" } else { &ip },
                            if online { "online" } else { "offline" }
                        ));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getUsbDevices", Value::Null).await {
                        usb.set(UsbDevices::from_value(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getEDID", Value::Null).await {
                        edid.set(as_string(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getNetworkSettings", Value::Null).await {
                        net_dhcp
                            .set(v.get("dhcp_enabled").and_then(Value::as_bool).unwrap_or(true));
                        net_ip.set(json_str(&v, "static_ip"));
                        net_gw.set(json_str(&v, "gateway"));
                        net_dns.set(
                            v.get("dns")
                                .and_then(Value::as_array)
                                .map(|a| {
                                    a.iter()
                                        .filter_map(Value::as_str)
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default(),
                        );
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getAutoUpdateState", Value::Null).await {
                        auto_update.set(v.as_bool().unwrap_or(false));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getJigglerState", Value::Null).await {
                        jiggler.set(v.as_bool().unwrap_or(false));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getUsbEmulationState", Value::Null).await {
                        usb_emul.set(v.as_bool().unwrap_or(false));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getHostDisplayIdleMode", Value::Null).await {
                        host_idle.set(v.get("enabled").and_then(Value::as_bool).unwrap_or(false));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getVideoSleepMode", Value::Null).await {
                        video_sleep.set(v.get("duration").and_then(Value::as_i64).unwrap_or(-1));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getDefaultLogLevel", Value::Null).await {
                        log_level.set(v.as_str().unwrap_or("INFO").to_string());
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getSSHKeyState", Value::Null).await {
                        ssh_key.set(as_string(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getMqttSettings", Value::Null).await {
                        mqtt_enabled
                            .set(v.get("enabled").and_then(Value::as_bool).unwrap_or(false));
                        mqtt_broker.set(json_str(&v, "broker"));
                        mqtt_port.set(v.get("port").and_then(Value::as_i64).unwrap_or(1883));
                        mqtt_user.set(json_str(&v, "username"));
                        mqtt_pass.set(json_str(&v, "password"));
                        mqtt_topic.set(json_str(&v, "base_topic"));
                    }
                }),
            ];
            join_all(loads).await;
        });
    });

    let do_mount = move |_| {
        let url = mount_url.get();
        if url.is_empty() {
            return;
        }
        let mode = mount_mode.get();
        let Some(client) = rpc.get_value() else { return };
        spawn_local(async move {
            match client.call("mountWithHTTP", json!({ "url": url, "mode": mode })).await {
                Ok(_) => {
                    if let Ok(v) = client.call("getVirtualMediaState", Value::Null).await {
                        mount_state.set(describe_mount(&v));
                    }
                }
                Err(e) => mount_state.set(e.to_string()),
            }
        });
    };
    let do_unmount = move |_| {
        let Some(client) = rpc.get_value() else { return };
        spawn_local(async move {
            let _ = client.call("unmountImage", Value::Null).await;
            mount_state.set("none".to_string());
        });
    };

    let on_file = move |ev: web_sys::Event| {
        let Some(input) = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        let Some(file) = input.files().and_then(|f| f.get(0)) else { return };
        let (Some(client), Some(p)) = (rpc.get_value(), peer.get_value()) else { return };
        let mode = mount_mode.get();
        upload_state.set("uploading… 0%".to_string());
        spawn_local(async move {
            let report = move |pct: u32| upload_state.set(format!("uploading… {pct}%"));
            match upload::upload_and_mount(p, client.clone(), file, mode, report).await {
                Ok(()) => {
                    upload_state.set("mounted".to_string());
                    if let Ok(v) = client.call("getVirtualMediaState", Value::Null).await {
                        mount_state.set(describe_mount(&v));
                    }
                }
                Err(e) => upload_state.set(e),
            }
        });
    };

    let renew_dhcp = move |_| {
        let Some(client) = rpc.get_value() else { return };
        spawn_local(async move {
            let _ = client.call("renewDHCPLease", Value::Null).await;
            if let Ok(v) = client.call("getNetworkState", Value::Null).await {
                network.set(json_str(&v, "ip"));
            }
        });
    };

    let check_update = move |_| {
        let Some(client) = rpc.get_value() else { return };
        update_state.set("checking…".to_string());
        spawn_local(async move {
            match client.call_with("getUpdateStatus", Value::Null, 10_000, 4).await {
                Ok(v) => {
                    let app = v.get("appUpdateAvailable").and_then(Value::as_bool).unwrap_or(false);
                    let sys =
                        v.get("systemUpdateAvailable").and_then(Value::as_bool).unwrap_or(false);
                    update_state.set(if app || sys {
                        "update available".to_string()
                    } else {
                        "up to date".to_string()
                    });
                }
                Err(e) => update_state.set(e.to_string()),
            }
        });
    };
    let apply_update = move |_| {
        let Some(client) = rpc.get_value() else { return };
        update_state.set("updating…".to_string());
        spawn_local(async move {
            let _ = client.call("tryUpdate", Value::Null).await;
        });
    };

    let toggle_usb = move |device: &'static str, enabled: bool| {
        fire(rpc, "setUsbDeviceState", json!({ "device": device, "enabled": enabled }));
    };

    let save_edid = move |_| {
        fire(rpc, "setEDID", json!({ "edid": edid.get() }));
    };

    let send_wol = move |_| {
        let mac = wol_mac.get();
        if mac.is_empty() {
            return;
        }
        fire(rpc, "sendWOLMagicPacket", json!({ "macAddress": mac }));
    };

    let save_network = move |_| {
        let dns: Vec<String> = net_dns
            .get()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        fire(
            rpc,
            "setNetworkSettings",
            json!({ "settings": {
                "dhcp_enabled": net_dhcp.get(),
                "static_ip": net_ip.get(),
                "gateway": net_gw.get(),
                "dns": dns,
            }}),
        );
    };

    let save_mqtt = move |_| {
        fire(
            rpc,
            "setMqttSettings",
            json!({ "settings": {
                "enabled": mqtt_enabled.get(),
                "broker": mqtt_broker.get(),
                "port": mqtt_port.get(),
                "username": mqtt_user.get(),
                "password": mqtt_pass.get(),
                "base_topic": mqtt_topic.get(),
                "use_tls": false,
                "tls_insecure": false,
                "enable_ha_discovery": true,
                "enable_actions": false,
            }}),
        );
    };

    let save_ssh = move |_| fire(rpc, "setSSHKeyState", json!({ "sshKey": ssh_key.get() }));
    let set_sleep = move |ev: web_sys::Event| {
        let d: i64 = event_target_value(&ev).parse().unwrap_or(-1);
        video_sleep.set(d);
        fire(rpc, "setVideoSleepMode", json!({ "duration": d }));
    };
    let set_log = move |ev: web_sys::Event| {
        let lvl = event_target_value(&ev);
        log_level.set(lvl.clone());
        fire(rpc, "setDefaultLogLevel", json!({ "level": lvl }));
    };

    let set_codec = move |value: &'static str| {
        fire(rpc, "setVideoCodecPreference", json!({ "codec": value }));
        codec.set(value.to_string());
    };
    let set_rotation = move |value: &'static str| {
        fire(rpc, "setDisplayRotation", json!({ "rotation": value }));
        rotation.set(value.to_string());
    };
    let set_layout = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        fire(rpc, "setKeyboardLayout", json!({ "layout": value }));
        layout.set(value);
    };
    let set_brightness = move |ev: web_sys::Event| {
        let value: i64 = event_target_value(&ev).parse().unwrap_or(0);
        fire(rpc, "setBacklightSettings", json!({ "max_brightness": value }));
        brightness.set(value);
    };
    let submit_password = move |_| {
        pw_msg.set(String::new());
        let (o, n) = (old_pw.get(), new_pw.get());
        if n.len() < 4 {
            pw_msg.set("New password too short".to_string());
            return;
        }
        spawn_local(async move {
            match rest::change_password(&o, &n).await {
                Ok(()) => {
                    pw_msg.set("Password updated".to_string());
                    old_pw.set(String::new());
                    new_pw.set(String::new());
                }
                Err(e) => pw_msg.set(e),
            }
        });
    };

    let codec_btn = move |value: &'static str, text: &'static str| {
        view! {
            <button class:active=move || codec.get() == value on:click=move |_| set_codec(value)>
                {text}
            </button>
        }
    };
    let rotation_btn = move |value: &'static str| {
        view! {
            <button
                class:active=move || rotation.get() == value
                on:click=move |_| set_rotation(value)
            >
                {value}"°"
            </button>
        }
    };

    view! {
        <Show when=move || open.get()>
            <div class="drawer-scrim" on:click=move |_| open.set(false)></div>
            <aside class="drawer">
                <header>
                    <h2>"Settings"</h2>
                    <button class="close" on:click=move |_| open.set(false)>"✕"</button>
                </header>

                <section>
                    <h3>"Video codec"</h3>
                    <div class="row">
                        {codec_btn("auto", "Auto")}
                        {codec_btn("h264", "H.264")}
                        {codec_btn("h265", "H.265")}
                    </div>
                </section>

                <section>
                    <h3>"Display rotation"</h3>
                    <div class="row">
                        {rotation_btn("0")}
                        {rotation_btn("90")}
                        {rotation_btn("180")}
                        {rotation_btn("270")}
                    </div>
                </section>

                <section>
                    <h3>"Keyboard layout"</h3>
                    <select on:change=set_layout prop:value=move || layout.get()>
                        {KEYBOARD_LAYOUTS
                            .iter()
                            .map(|(code, name)| {
                                view! { <option value=*code>{*name}</option> }
                            })
                            .collect_view()}
                    </select>
                </section>

                <section>
                    <h3>"Backlight brightness"</h3>
                    <div class="row">
                        <input
                            type="range"
                            min="0"
                            max="255"
                            prop:value=move || brightness.get().to_string()
                            on:change=set_brightness
                        />
                        <span class="muted">{move || brightness.get()}</span>
                    </div>
                </section>

                <section>
                    <h3>"Change password"</h3>
                    <input
                        type="password"
                        placeholder="Current password"
                        prop:value=move || old_pw.get()
                        on:input=move |ev| old_pw.set(event_target_value(&ev))
                    />
                    <input
                        type="password"
                        placeholder="New password"
                        prop:value=move || new_pw.get()
                        on:input=move |ev| new_pw.set(event_target_value(&ev))
                    />
                    <button on:click=submit_password>"Update password"</button>
                    <div class="muted">{move || pw_msg.get()}</div>
                </section>

                <section>
                    <h3>"Virtual media"</h3>
                    <div class="muted">"Mounted: "{move || mount_state.get()}</div>
                    <input
                        type="url"
                        placeholder="https://…/image.iso"
                        prop:value=move || mount_url.get()
                        on:input=move |ev| mount_url.set(event_target_value(&ev))
                    />
                    <select
                        prop:value=move || mount_mode.get()
                        on:change=move |ev| mount_mode.set(event_target_value(&ev))
                    >
                        <option value="CDROM">"CD-ROM"</option>
                        <option value="DISK">"Disk"</option>
                    </select>
                    <div class="row">
                        <button on:click=do_mount>"Mount URL"</button>
                        <button on:click=do_unmount>"Unmount"</button>
                    </div>
                    <label class="muted">"or upload a local image:"</label>
                    <input type="file" on:change=on_file />
                    <div class="muted">{move || upload_state.get()}</div>
                </section>

                <section>
                    <h3>"Network"</h3>
                    <div class="muted">{move || network.get()}</div>
                    <button on:click=renew_dhcp>"Renew DHCP lease"</button>
                </section>

                <section>
                    <h3>"Firmware update"</h3>
                    <div class="muted">{move || update_state.get()}</div>
                    <div class="row">
                        <button on:click=check_update>"Check"</button>
                        <button on:click=apply_update>"Update now"</button>
                    </div>
                </section>

                <section>
                    <h3>"USB devices"</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().keyboard
                            on:change=move |ev| {
                                toggle_usb("keyboard", event_target_checked(&ev))
                            }
                        />
                        "Keyboard"
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().absolute_mouse
                            on:change=move |ev| {
                                toggle_usb("absoluteMouse", event_target_checked(&ev))
                            }
                        />
                        "Absolute mouse"
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().relative_mouse
                            on:change=move |ev| {
                                toggle_usb("relativeMouse", event_target_checked(&ev))
                            }
                        />
                        "Relative mouse"
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().mass_storage
                            on:change=move |ev| {
                                toggle_usb("massStorage", event_target_checked(&ev))
                            }
                        />
                        "Mass storage"
                    </label>
                </section>

                <section>
                    <h3>"EDID"</h3>
                    <input
                        type="text"
                        placeholder="hex EDID (empty = default)"
                        prop:value=move || edid.get()
                        on:input=move |ev| edid.set(event_target_value(&ev))
                    />
                    <button on:click=save_edid>"Apply EDID"</button>
                </section>

                <section>
                    <h3>"Wake-on-LAN"</h3>
                    <input
                        type="text"
                        placeholder="AA:BB:CC:DD:EE:FF"
                        prop:value=move || wol_mac.get()
                        on:input=move |ev| wol_mac.set(event_target_value(&ev))
                    />
                    <button on:click=send_wol>"Send magic packet"</button>
                </section>

                <section>
                    <h3>"Static network"</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || net_dhcp.get()
                            on:change=move |ev| net_dhcp.set(event_target_checked(&ev))
                        />
                        "DHCP"
                    </label>
                    <Show when=move || !net_dhcp.get()>
                        <input
                            type="text"
                            placeholder="Static IP (e.g. 192.168.1.50)"
                            prop:value=move || net_ip.get()
                            on:input=move |ev| net_ip.set(event_target_value(&ev))
                        />
                        <input
                            type="text"
                            placeholder="Gateway"
                            prop:value=move || net_gw.get()
                            on:input=move |ev| net_gw.set(event_target_value(&ev))
                        />
                        <input
                            type="text"
                            placeholder="DNS (comma separated)"
                            prop:value=move || net_dns.get()
                            on:input=move |ev| net_dns.set(event_target_value(&ev))
                        />
                    </Show>
                    <button on:click=save_network>"Apply network"</button>
                </section>

                <section>
                    <h3>"Features"</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || jiggler.get()
                            on:change=move |ev| send_toggle(rpc, "setJigglerState", jiggler, &ev)
                        />
                        "Mouse jiggler"
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb_emul.get()
                            on:change=move |ev| send_toggle(rpc, "setUsbEmulationState", usb_emul, &ev)
                        />
                        "USB emulation"
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || host_idle.get()
                            on:change=move |ev| {
                                send_toggle(rpc, "setHostDisplayIdleMode", host_idle, &ev)
                            }
                        />
                        "Host display idle"
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || auto_update.get()
                            on:change=move |ev| send_toggle(rpc, "setAutoUpdateState", auto_update, &ev)
                        />
                        "Auto-update"
                    </label>
                </section>

                <section>
                    <h3>"Video sleep"</h3>
                    <div class="row">
                        <input
                            type="number"
                            min="-1"
                            prop:value=move || video_sleep.get().to_string()
                            on:change=set_sleep
                        />
                        <span class="muted">"seconds (-1 = off)"</span>
                    </div>
                </section>

                <section>
                    <h3>"Log level"</h3>
                    <select prop:value=move || log_level.get() on:change=set_log>
                        {["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]
                            .iter()
                            .map(|l| view! { <option value=*l>{*l}</option> })
                            .collect_view()}
                    </select>
                </section>

                <section>
                    <h3>"SSH authorized key"</h3>
                    <input
                        type="text"
                        placeholder="ssh-ed25519 AAAA…"
                        prop:value=move || ssh_key.get()
                        on:input=move |ev| ssh_key.set(event_target_value(&ev))
                    />
                    <button on:click=save_ssh>"Save SSH key"</button>
                </section>

                <section>
                    <h3>"MQTT"</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || mqtt_enabled.get()
                            on:change=move |ev| mqtt_enabled.set(event_target_checked(&ev))
                        />
                        "Enabled"
                    </label>
                    <input
                        type="text"
                        placeholder="Broker host"
                        prop:value=move || mqtt_broker.get()
                        on:input=move |ev| mqtt_broker.set(event_target_value(&ev))
                    />
                    <input
                        type="number"
                        placeholder="Port"
                        prop:value=move || mqtt_port.get().to_string()
                        on:input=move |ev| mqtt_port.set(event_target_value(&ev).parse().unwrap_or(1883))
                    />
                    <input
                        type="text"
                        placeholder="Username"
                        prop:value=move || mqtt_user.get()
                        on:input=move |ev| mqtt_user.set(event_target_value(&ev))
                    />
                    <input
                        type="password"
                        placeholder="Password"
                        prop:value=move || mqtt_pass.get()
                        on:input=move |ev| mqtt_pass.set(event_target_value(&ev))
                    />
                    <input
                        type="text"
                        placeholder="Base topic"
                        prop:value=move || mqtt_topic.get()
                        on:input=move |ev| mqtt_topic.set(event_target_value(&ev))
                    />
                    <button on:click=save_mqtt>"Apply MQTT"</button>
                </section>

                <crate::settings_advanced::AdvancedSettings open=open rpc=rpc />

                <section>
                    <h3>"About"</h3>
                    <div class="muted">"Device: "{move || device_id.get()}</div>
                    <div class="muted">{move || version.get()}</div>
                </section>
            </aside>
        </Show>
    }
}

fn send_toggle(rpc: Rpc, method: &'static str, sig: RwSignal<bool>, ev: &web_sys::Event) {
    let on = event_target_checked(ev);
    sig.set(on);
    fire(rpc, method, json!({ "enabled": on }));
}

#[derive(Clone, Copy, Default, PartialEq)]
struct UsbDevices {
    absolute_mouse: bool,
    relative_mouse: bool,
    keyboard: bool,
    mass_storage: bool,
}

impl UsbDevices {
    fn from_value(v: &Value) -> Self {
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        Self {
            absolute_mouse: b("absolute_mouse"),
            relative_mouse: b("relative_mouse"),
            keyboard: b("keyboard"),
            mass_storage: b("mass_storage"),
        }
    }
}

fn describe_mount(v: &Value) -> String {
    if v.is_null() {
        return "none".to_string();
    }
    let name = json_str(v, "filename");
    let url = json_str(v, "url");
    let mode = json_str(v, "mode");
    let what = if !name.is_empty() {
        name
    } else if !url.is_empty() {
        url
    } else {
        return "none".to_string();
    };
    if mode.is_empty() { what } else { format!("{what} ({mode})") }
}
