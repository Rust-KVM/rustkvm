use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use futures::future::join_all;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;

use crate::i18n::{I18n, tr, use_i18n};
use crate::net::peer::Peer;
use crate::net::{rest, upload};
use crate::rpc::RpcClient;
use crate::util::{Rpc, as_string, fire, fire_status, json_str};

/// Values are the codes `getKeyboardLayout` returns and `setKeyboardLayout`
/// accepts on the device.
const KEYBOARD_LAYOUTS: &[(&str, &str, &str)] = &[
    ("us", "英语（美国）", "English (US)"),
    ("uk", "英语（英国）", "English (UK)"),
    ("de", "德语", "German"),
    ("fr", "法语", "French"),
    ("es", "西班牙语", "Spanish"),
    ("it", "意大利语", "Italian"),
    ("jp", "日语", "Japanese"),
    ("kr", "韩语", "Korean"),
];

#[component]
pub fn Settings(
    open: RwSignal<bool>,
    rpc: StoredValue<Option<Rc<RpcClient>>, LocalStorage>,
    peer: StoredValue<Option<Rc<Peer>>, LocalStorage>,
    no_password: RwSignal<bool>,
) -> impl IntoView {
    let i = use_i18n();
    let codec = RwSignal::new(String::new());
    let rotation = RwSignal::new(String::new());
    let layout = RwSignal::new(String::new());
    let brightness = RwSignal::new(0_i64);
    // Sent back unchanged: `setBacklightSettings` requires all three fields.
    let dim_after = RwSignal::new(0_i64);
    let off_after = RwSignal::new(0_i64);
    let device_id = RwSignal::new(String::new());
    let app_version = RwSignal::new(String::new());
    let sys_version = RwSignal::new(String::new());

    let mount_url = RwSignal::new(String::new());
    let mount_mode = RwSignal::new("CDROM".to_string());
    // `getVirtualMediaState` reply; described at render time so it follows
    // the language. `mount_err` overrides it until the next refresh.
    let mount_info = RwSignal::new(Value::Null);
    let mount_err = RwSignal::new(String::new());
    let upload_state = RwSignal::new(String::new());

    let network = RwSignal::new(Value::Null);
    let update_state = RwSignal::new(String::new());
    // Only offered after Check reports an update.
    let update_available = RwSignal::new(false);
    let confirm_pw = RwSignal::new(String::new());
    let usb = RwSignal::new(UsbDevices::default());
    let edid = RwSignal::new(String::new());
    let edid_msg = RwSignal::new(String::new());
    let wol_mac = RwSignal::new(String::new());
    let wol_msg = RwSignal::new(String::new());

    let net_dhcp = RwSignal::new(true);
    let net_ip = RwSignal::new(String::new());
    let net_gw = RwSignal::new(String::new());
    let net_dns = RwSignal::new(String::new());
    let net_msg = RwSignal::new(String::new());

    let auto_update = RwSignal::new(false);
    let jiggler = RwSignal::new(false);
    let usb_emul = RwSignal::new(false);
    let log_level = RwSignal::new("INFO".to_string());
    let ssh_key = RwSignal::new(String::new());
    let ssh_msg = RwSignal::new(String::new());

    let mqtt_enabled = RwSignal::new(false);
    let mqtt_broker = RwSignal::new(String::new());
    let mqtt_port = RwSignal::new(1883_i64);
    let mqtt_user = RwSignal::new(String::new());
    let mqtt_pass = RwSignal::new(String::new());
    let mqtt_topic = RwSignal::new("rustkvm".to_string());
    let mqtt_msg = RwSignal::new(String::new());

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
                        let num = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
                        brightness.set(num("max_brightness"));
                        dim_after.set(num("dim_after"));
                        off_after.set(num("off_after"));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getDeviceID", Value::Null).await {
                        device_id.set(as_string(&v));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getLocalVersion", Value::Null).await {
                        app_version.set(json_str(&v, "app_version"));
                        sys_version.set(json_str(&v, "system_version"));
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getVirtualMediaState", Value::Null).await {
                        {
                            mount_err.set(String::new());
                            mount_info.set(v);
                        }
                    }
                }),
                Box::pin(async move {
                    if let Ok(v) = client.call("getNetworkState", Value::Null).await {
                        network.set(v);
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
        let url = mount_url.get().trim().to_string();
        if url.is_empty() {
            mount_err.set(tr("请先输入镜像 URL", "Enter an image URL first").to_string());
            return;
        }
        let mode = mount_mode.get();
        let Some(client) = rpc.get_value() else { return };
        spawn_local(async move {
            match client.call("mountWithHTTP", json!({ "url": url, "mode": mode })).await {
                Ok(_) => {
                    if let Ok(v) = client.call("getVirtualMediaState", Value::Null).await {
                        {
                            mount_err.set(String::new());
                            mount_info.set(v);
                        }
                    }
                }
                Err(e) => mount_err.set(format!("{}{e}", tr("挂载失败：", "Mount failed: "))),
            }
        });
    };
    let do_unmount = move |_| {
        let Some(client) = rpc.get_value() else { return };
        spawn_local(async move {
            match client.call("unmountImage", Value::Null).await {
                Ok(_) => {
                    mount_err.set(String::new());
                    mount_info.set(Value::Null);
                }
                Err(e) => mount_err.set(format!("{}{e}", tr("卸载失败：", "Unmount failed: "))),
            }
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
        upload_state.set(format!("{}0%", tr("正在上传… ", "Uploading… ")));
        spawn_local(async move {
            let report = move |pct: u32| {
                upload_state.set(format!("{}{pct}%", tr("正在上传… ", "Uploading… ")))
            };
            match upload::upload_and_mount(p, client.clone(), file, mode, report).await {
                Ok(()) => {
                    upload_state.set(tr("已上传并挂载", "Uploaded and mounted").to_string());
                    if let Ok(v) = client.call("getVirtualMediaState", Value::Null).await {
                        {
                            mount_err.set(String::new());
                            mount_info.set(v);
                        }
                    }
                }
                Err(e) => upload_state.set(format!("{}{e}", tr("上传失败：", "Upload failed: "))),
            }
        });
    };

    let renew_dhcp = move |_| {
        let Some(client) = rpc.get_value() else { return };
        spawn_local(async move {
            let _ = client.call("renewDHCPLease", Value::Null).await;
            if let Ok(v) = client.call("getNetworkState", Value::Null).await {
                network.set(v);
            }
        });
    };

    let check_update = move |_| {
        let Some(client) = rpc.get_value() else { return };
        update_state.set(tr("正在检查…", "Checking…").to_string());
        spawn_local(async move {
            match client.call_with("getUpdateStatus", Value::Null, 10_000, 4).await {
                Ok(v) => {
                    let app = v.get("appUpdateAvailable").and_then(Value::as_bool).unwrap_or(false);
                    let sys =
                        v.get("systemUpdateAvailable").and_then(Value::as_bool).unwrap_or(false);
                    update_available.set(app || sys);
                    update_state.set(if app || sys {
                        tr("有可用更新", "An update is available").to_string()
                    } else {
                        tr("已是最新版本", "Up to date").to_string()
                    });
                }
                Err(e) => update_state.set(format!("{}{e}", tr("检查失败：", "Check failed: "))),
            }
        });
    };
    let apply_update = move |_| {
        if !update_available.get_untracked() {
            return;
        }
        let Some(client) = rpc.get_value() else { return };
        update_state.set(
            tr("正在更新…完成后设备会自动重启", "Updating… the device restarts when it is done")
                .to_string(),
        );
        spawn_local(async move {
            if let Err(e) = client.call("tryUpdate", Value::Null).await {
                update_state.set(format!("{}{e}", tr("更新失败：", "Update failed: ")));
            }
        });
    };

    let toggle_usb = move |device: &'static str, enabled: bool| {
        fire(rpc, "setUsbDeviceState", json!({ "device": device, "enabled": enabled }));
    };

    let save_edid = move |_| {
        fire_status(
            rpc,
            "setEDID",
            json!({ "edid": edid.get().trim() }),
            edid_msg,
            tr("EDID 已应用", "EDID applied"),
        );
    };

    let send_wol = move |_| {
        let mac = wol_mac.get().trim().to_string();
        if mac.is_empty() {
            wol_msg.set(
                tr(
                    "请输入要唤醒的电脑的 MAC 地址",
                    "Enter the MAC address of the computer to wake",
                )
                .to_string(),
            );
            return;
        }
        fire_status(
            rpc,
            "sendWOLMagicPacket",
            json!({ "macAddress": mac }),
            wol_msg,
            tr("唤醒包已发送", "Magic packet sent"),
        );
    };

    let save_network = move |_| {
        let dns: Vec<String> = net_dns
            .get()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        fire_status(
            rpc,
            "setNetworkSettings",
            json!({ "settings": {
                "dhcp_enabled": net_dhcp.get(),
                "static_ip": net_ip.get(),
                "gateway": net_gw.get(),
                "dns": dns,
            }}),
            net_msg,
            tr("网络设置已保存", "Network settings saved"),
        );
    };

    let save_mqtt = move |_| {
        fire_status(
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
            mqtt_msg,
            tr("MQTT 设置已保存", "MQTT settings saved"),
        );
    };

    let save_ssh = move |_| {
        fire_status(
            rpc,
            "setSSHKeyState",
            json!({ "sshKey": ssh_key.get().trim() }),
            ssh_msg,
            tr("SSH 密钥已保存", "SSH key saved"),
        )
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
        fire(
            rpc,
            "setBacklightSettings",
            json!({
                "max_brightness": value,
                "dim_after": dim_after.get_untracked(),
                "off_after": off_after.get_untracked(),
            }),
        );
        brightness.set(value);
    };
    let submit_password = move |_| {
        pw_msg.set(String::new());
        let (o, n) = (old_pw.get(), new_pw.get());
        if n.len() < 4 {
            pw_msg.set(tr("新密码太短", "New password too short").to_string());
            return;
        }
        if n != confirm_pw.get() {
            pw_msg.set(
                tr("两次输入的新密码不一致", "The two new passwords do not match").to_string(),
            );
            return;
        }
        let first = no_password.get_untracked();
        spawn_local(async move {
            let result = if first {
                rest::set_password(&n).await
            } else {
                rest::change_password(&o, &n).await
            };
            match result {
                Ok(()) => {
                    no_password.set(false);
                    pw_msg.set(
                        if first {
                            tr("密码已设置", "Password set")
                        } else {
                            tr("密码已更新", "Password updated")
                        }
                        .to_string(),
                    );
                    old_pw.set(String::new());
                    new_pw.set(String::new());
                    confirm_pw.set(String::new());
                }
                Err(e) => pw_msg.set(e),
            }
        });
    };

    let codec_btn = move |value: &'static str, zh: &'static str, en: &'static str| {
        view! {
            <button class:active=move || codec.get() == value on:click=move |_| set_codec(value)>
                {move || i.t(zh, en)}
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
                    <h2>{move || i.t("设置", "Settings")}</h2>
                    <button class="close" title=move || i.t("关闭", "Close") aria-label=move || i.t("关闭", "Close") on:click=move |_| open.set(false)>
                        <crate::ui::Icon name=crate::ui::IconName::Close/>
                    </button>
                </header>

                <section>
                    <h3>{move || i.t("视频编码", "Video codec")}</h3>
                    <div class="row">
                        {codec_btn("auto", "自动", "Auto")}
                        {codec_btn("h264", "H.264", "H.264")}
                        {codec_btn("h265", "H.265", "H.265")}
                    </div>
                </section>

                <section>
                    <h3>{move || i.t("屏幕旋转", "Display rotation")}</h3>
                    <div class="row">
                        {rotation_btn("0")}
                        {rotation_btn("90")}
                        {rotation_btn("180")}
                        {rotation_btn("270")}
                    </div>
                </section>

                <section>
                    <h3>{move || i.t("键盘布局", "Keyboard layout")}</h3>
                    <select on:change=set_layout>
                        {KEYBOARD_LAYOUTS
                            .iter()
                            .map(|(code, zh, en)| {
                                view! {
                                    <option value=*code prop:selected=move || layout.get() == *code>
                                        {move || i.t(zh, en)}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </section>

                <section>
                    <h3>{move || i.t("设备屏幕亮度", "Device screen brightness")}</h3>
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
                    <div class="hint">{move || i.t("仅在设备连接了屏幕时生效。", "Only applies when a screen is attached to the device.")}</div>
                </section>

                <section>
                    <h3>{move || i.t("密码", "Password")}</h3>
                    <Show when=move || !no_password.get()>
                        <input
                            type="password"
                            placeholder=move || i.t("当前密码", "Current password")
                            prop:value=move || old_pw.get()
                            on:input=move |ev| old_pw.set(event_target_value(&ev))
                        />
                    </Show>
                    <input
                        type="password"
                        placeholder=move || i.t("新密码", "New password")
                        prop:value=move || new_pw.get()
                        on:input=move |ev| new_pw.set(event_target_value(&ev))
                    />
                    <input
                        type="password"
                        placeholder=move || i.t("再次输入新密码", "Repeat new password")
                        prop:value=move || confirm_pw.get()
                        on:input=move |ev| confirm_pw.set(event_target_value(&ev))
                    />
                    <button on:click=submit_password>
                        {move || {
                            if no_password.get() {
                                i.t("设置密码", "Set password")
                            } else {
                                i.t("修改密码", "Update password")
                            }
                        }}
                    </button>
                    <div class="muted">{move || pw_msg.get()}</div>
                </section>

                <section>
                    <h3>{move || i.t("虚拟媒体", "Virtual media")}</h3>
                    <div class="muted">
                        {move || {
                            let err = mount_err.get();
                            if err.is_empty() { describe_mount(&mount_info.get(), i) } else { err }
                        }}
                    </div>
                    <label class="field">
                        <span>{move || i.t("镜像 URL", "Image URL")}</span>
                        <input
                            type="url"
                            placeholder="https://example.com/image.iso"
                            prop:value=move || mount_url.get()
                            on:input=move |ev| mount_url.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="field">
                        <span>{move || i.t("在主机上显示为", "Present to the host as")}</span>
                        <select
                            on:change=move |ev| mount_mode.set(event_target_value(&ev))
                        >
                            <option value="CDROM" prop:selected=move || mount_mode.get() == "CDROM">
                                {move || i.t("光驱（CD-ROM）", "CD-ROM")}
                            </option>
                            <option value="DISK" prop:selected=move || mount_mode.get() == "DISK">
                                {move || i.t("U 盘", "USB disk")}
                            </option>
                        </select>
                    </label>
                    <div class="row">
                        <button on:click=do_mount>{move || i.t("挂载 URL", "Mount URL")}</button>
                        <button on:click=do_unmount>{move || i.t("卸载", "Unmount")}</button>
                    </div>
                    <label class="field">
                        <span>{move || i.t("或从这台电脑上传镜像", "Or upload an image from this computer")}</span>
                        <input type="file" on:change=on_file />
                    </label>
                    <div class="muted">{move || upload_state.get()}</div>
                </section>

                <section>
                    <h3>{move || i.t("网络", "Network")}</h3>
                    <dl class="kv">
                        {move || {
                            describe_network(&network.get(), i)
                                .into_iter()
                                .map(|(k, v)| view! { <dt>{k}</dt><dd>{v}</dd> })
                                .collect_view()
                        }}
                    </dl>
                    <button on:click=renew_dhcp>{move || i.t("重新获取 DHCP 地址", "Renew DHCP lease")}</button>
                </section>

                <section>
                    <h3>{move || i.t("固件更新", "Firmware update")}</h3>
                    <div class="muted">{move || update_state.get()}</div>
                    <div class="row">
                        <button on:click=check_update>{move || i.t("检查更新", "Check for updates")}</button>
                        <button
                            on:click=apply_update
                            disabled=move || !update_available.get()
                            title=move || i.t("请先检查更新，有可用更新时才能点击", "Check for updates first; enabled when one is available")
                        >
                            {move || i.t("立即更新", "Update now")}
                        </button>
                    </div>
                </section>

                <section>
                    <h3>{move || i.t("USB 设备", "USB devices")}</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().keyboard
                            on:change=move |ev| {
                                toggle_usb("keyboard", event_target_checked(&ev))
                            }
                        />
                        {move || i.t("键盘", "Keyboard")}
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().absolute_mouse
                            on:change=move |ev| {
                                toggle_usb("absoluteMouse", event_target_checked(&ev))
                            }
                        />
                        {move || i.t("绝对鼠标", "Absolute mouse")}
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().relative_mouse
                            on:change=move |ev| {
                                toggle_usb("relativeMouse", event_target_checked(&ev))
                            }
                        />
                        {move || i.t("相对鼠标", "Relative mouse")}
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb.get().mass_storage
                            on:change=move |ev| {
                                toggle_usb("massStorage", event_target_checked(&ev))
                            }
                        />
                        {move || i.t("大容量存储", "Mass storage")}
                    </label>
                </section>

                <section>
                    <h3>"EDID"</h3>
                    <label class="field">
                        <span>{move || i.t("EDID（十六进制）", "EDID (hex)")}</span>
                        <textarea
                            rows="3"
                            placeholder="00ffffffffffff00…"
                            prop:value=move || edid.get()
                            on:input=move |ev| edid.set(event_target_value(&ev))
                        ></textarea>
                    </label>
                    <div class="hint">{move || i.t("留空则恢复默认的显示器标识。", "Leave empty to restore the default display identity.")}</div>
                    <button on:click=save_edid>{move || i.t("应用 EDID", "Apply EDID")}</button>
                    <div class="muted">{move || edid_msg.get()}</div>
                </section>

                <section>
                    <h3>{move || i.t("网络唤醒", "Wake-on-LAN")}</h3>
                    <label class="field">
                        <span>{move || i.t("要唤醒的电脑的 MAC 地址", "MAC address of the computer to wake")}</span>
                        <input
                            type="text"
                            placeholder="AA:BB:CC:DD:EE:FF"
                            prop:value=move || wol_mac.get()
                            on:input=move |ev| wol_mac.set(event_target_value(&ev))
                        />
                    </label>
                    <button on:click=send_wol>{move || i.t("发送唤醒包", "Send magic packet")}</button>
                    <div class="muted">{move || wol_msg.get()}</div>
                </section>

                <section>
                    <h3>{move || i.t("IP 地址", "IP address")}</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || net_dhcp.get()
                            on:change=move |ev| net_dhcp.set(event_target_checked(&ev))
                        />
                        {move || i.t("自动获取地址（DHCP）", "Get address automatically (DHCP)")}
                    </label>
                    <Show when=move || !net_dhcp.get()>
                        <label class="field">
                            <span>{move || i.t("静态 IP", "Static IP")}</span>
                            <input
                                type="text"
                                placeholder="192.168.1.50"
                                prop:value=move || net_ip.get()
                                on:input=move |ev| net_ip.set(event_target_value(&ev))
                            />
                        </label>
                        <label class="field">
                            <span>{move || i.t("网关", "Gateway")}</span>
                            <input
                                type="text"
                                placeholder="192.168.1.1"
                                prop:value=move || net_gw.get()
                                on:input=move |ev| net_gw.set(event_target_value(&ev))
                            />
                        </label>
                        <label class="field">
                            <span>{move || i.t("DNS 服务器（用逗号分隔）", "DNS servers (comma separated)")}</span>
                            <input
                                type="text"
                                placeholder="1.1.1.1, 8.8.8.8"
                                prop:value=move || net_dns.get()
                                on:input=move |ev| net_dns.set(event_target_value(&ev))
                            />
                        </label>
                    </Show>
                    <button on:click=save_network>{move || i.t("应用", "Apply")}</button>
                    <div class="muted">{move || net_msg.get()}</div>
                </section>

                <section>
                    <h3>{move || i.t("功能", "Features")}</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || jiggler.get()
                            on:change=move |ev| send_toggle(rpc, "setJigglerState", jiggler, &ev)
                        />
                        {move || i.t("鼠标防休眠", "Mouse jiggler")}
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || usb_emul.get()
                            on:change=move |ev| send_toggle(rpc, "setUsbEmulationState", usb_emul, &ev)
                        />
                        {move || i.t("USB 模拟", "USB emulation")}
                    </label>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || auto_update.get()
                            on:change=move |ev| send_toggle(rpc, "setAutoUpdateState", auto_update, &ev)
                        />
                        {move || i.t("自动更新", "Auto-update")}
                    </label>
                </section>

                <section>
                    <h3>{move || i.t("日志级别", "Log level")}</h3>
                    <select on:change=set_log>
                        {["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]
                            .iter()
                            .map(|l| {
                                view! {
                                    <option value=*l prop:selected=move || log_level.get() == *l>
                                        {*l}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </section>

                <section>
                    <h3>{move || i.t("SSH 访问", "SSH access")}</h3>
                    <label class="field">
                        <span>{move || i.t("授权公钥", "Authorized public key")}</span>
                        <textarea
                            rows="3"
                            placeholder="ssh-ed25519 AAAA… user@host"
                            prop:value=move || ssh_key.get()
                            on:input=move |ev| ssh_key.set(event_target_value(&ev))
                        ></textarea>
                    </label>
                    <button on:click=save_ssh>{move || i.t("保存 SSH 密钥", "Save SSH key")}</button>
                    <div class="muted">{move || ssh_msg.get()}</div>
                </section>

                <section>
                    <h3>"MQTT"</h3>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || mqtt_enabled.get()
                            on:change=move |ev| mqtt_enabled.set(event_target_checked(&ev))
                        />
                        {move || i.t("发布到 MQTT 服务器", "Publish to an MQTT broker")}
                    </label>
                    <label class="field">
                        <span>{move || i.t("服务器地址", "Broker host")}</span>
                        <input
                            type="text"
                            placeholder="homeassistant.local"
                            prop:value=move || mqtt_broker.get()
                            on:input=move |ev| mqtt_broker.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="field">
                        <span>{move || i.t("端口", "Port")}</span>
                        <input
                            type="number"
                            placeholder="1883"
                            prop:value=move || mqtt_port.get().to_string()
                            on:input=move |ev| mqtt_port.set(event_target_value(&ev).parse().unwrap_or(1883))
                        />
                    </label>
                    <label class="field">
                        <span>{move || i.t("用户名", "Username")}</span>
                        <input
                            type="text"
                            placeholder=move || i.t("可选", "Optional")
                            prop:value=move || mqtt_user.get()
                            on:input=move |ev| mqtt_user.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="field">
                        <span>{move || i.t("密码", "Password")}</span>
                        <input
                            type="password"
                            placeholder=move || i.t("可选", "Optional")
                            prop:value=move || mqtt_pass.get()
                            on:input=move |ev| mqtt_pass.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="field">
                        <span>{move || i.t("主题前缀", "Base topic")}</span>
                        <input
                            type="text"
                            placeholder="rustkvm"
                            prop:value=move || mqtt_topic.get()
                            on:input=move |ev| mqtt_topic.set(event_target_value(&ev))
                        />
                    </label>
                    <button on:click=save_mqtt>{move || i.t("应用 MQTT", "Apply MQTT")}</button>
                    <div class="muted">{move || mqtt_msg.get()}</div>
                </section>

                <crate::settings_advanced::AdvancedSettings open=open rpc=rpc />

                <section>
                    <h3>{move || i.t("关于", "About")}</h3>
                    <dl class="kv">
                        <dt>{move || i.t("设备 ID", "Device ID")}</dt>
                        <dd>{move || or_dash(device_id.get())}</dd>
                        <dt>{move || i.t("程序版本", "App version")}</dt>
                        <dd>{move || or_dash(app_version.get())}</dd>
                        <Show when=move || !sys_version.get().is_empty()>
                            <dt>{move || i.t("系统版本", "System version")}</dt>
                            <dd>{move || sys_version.get()}</dd>
                        </Show>
                    </dl>
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

fn or_dash(s: String) -> String {
    if s.is_empty() { "—".to_string() } else { s }
}

/// Rows for the Network section: address, link state, gateway and DNS.
/// The device can have several interfaces, so when this page was loaded from a
/// different address than the one reported, say so instead of looking wrong.
fn describe_network(v: &Value, i: I18n) -> Vec<(&'static str, String)> {
    let ip = json_str(v, "ip");
    let online = v.get("online").and_then(Value::as_bool).unwrap_or(false);
    let mut rows = vec![
        (i.t("地址", "Address"), or_dash(ip.clone())),
        (
            i.t("链路", "Link"),
            if online { i.t("在线", "Online") } else { i.t("离线", "Offline") }.to_string(),
        ),
    ];
    let gateway = json_str(v, "gateway");
    if !gateway.is_empty() {
        rows.push((i.t("网关", "Gateway"), gateway));
    }
    let dns: Vec<&str> = v
        .get("dns")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !dns.is_empty() {
        rows.push(("DNS", dns.join(", ")));
    }
    let page_host =
        web_sys::window().and_then(|w| w.location().hostname().ok()).unwrap_or_default();
    if !ip.is_empty() && !page_host.is_empty() && page_host != ip {
        rows.push((
            i.t("本页面", "This page"),
            match i.lang() {
                crate::i18n::Lang::Zh => format!("通过 {page_host} 打开"),
                crate::i18n::Lang::En => format!("opened via {page_host}"),
            },
        ));
    }
    rows
}

fn describe_mount(v: &Value, i: I18n) -> String {
    let name = json_str(v, "filename");
    let url = json_str(v, "url");
    let what = if !name.is_empty() {
        name
    } else if !url.is_empty() {
        url
    } else {
        return i.t("未挂载任何镜像", "Nothing mounted").to_string();
    };
    match json_str(v, "mode").as_str() {
        "CDROM" => format!("{}{what}", i.t("已作为光驱挂载：", "Mounted as CD-ROM: ")),
        "DISK" => format!("{}{what}", i.t("已作为 U 盘挂载：", "Mounted as USB disk: ")),
        _ => format!("{}{what}", i.t("已挂载：", "Mounted: ")),
    }
}
