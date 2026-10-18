use std::rc::Rc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};

use crate::rpc::RpcClient;
use crate::util::{Rpc, as_string, fire, fire_status, json_str, json_string_list};

/// Extensions the device knows how to mount (`setActiveExtension`).
const EXTENSIONS: &[(&str, &str)] =
    &[("", "None"), ("atx-power", "ATX power board"), ("dc-power", "DC power board")];

#[component]
pub fn AdvancedSettings(open: RwSignal<bool>, rpc: Rpc) -> impl IntoView {
    let dc_enabled = RwSignal::new(false);
    let dc_restore = RwSignal::new(String::new());
    let dc_reading = RwSignal::new(String::new());
    let atx_state = RwSignal::new(String::new());
    // "dc-power", "atx-power" or "" (no power extension mounted).
    let power_ext = RwSignal::new(String::new());
    let cloud = RwSignal::new(CloudState::default());
    let cloud_api = RwSignal::new(String::new());
    let cloud_app = RwSignal::new(String::new());
    let cloud_msg = RwSignal::new(String::new());
    let ts_url = RwSignal::new(String::new());
    let ts_msg = RwSignal::new(String::new());
    let active_ext = RwSignal::new(String::new());
    let ext_msg = RwSignal::new(String::new());
    let dev_mode = RwSignal::new(false);
    let dev_channel = RwSignal::new(false);
    let tls_mode = RwSignal::new(String::new());
    let tls_cert = RwSignal::new(String::new());
    let tls_key = RwSignal::new(String::new());
    let tls_msg = RwSignal::new(String::new());
    let serial_history = RwSignal::new(Vec::<String>::new());
    let timezone = RwSignal::new(String::new());
    let macros = RwSignal::new(Vec::<MacroItem>::new());
    let edit_id = RwSignal::new(Option::<String>::None);
    let edit_name = RwSignal::new(String::new());
    let edit_steps = RwSignal::new(String::new());
    let macro_msg = RwSignal::new(String::new());

    // Power controls depend on which extension is mounted. Asking for ATX
    // state while it is not mounted only logs an error on the device.
    let load_power = move |c: Rc<RpcClient>, ext: String| async move {
        match ext.as_str() {
            "dc-power" => {
                if let Ok(v) = c.call("getDCPowerState", Value::Null).await {
                    dc_enabled.set(v.get("isOn").and_then(Value::as_bool).unwrap_or(false));
                    dc_restore.set(
                        v.get("restoreState")
                            .and_then(Value::as_i64)
                            .map(|s| s.to_string())
                            .unwrap_or_default(),
                    );
                    let num = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                    dc_reading.set(format!(
                        "{:.2} V · {:.2} A · {:.2} W",
                        num("voltage"),
                        num("current"),
                        num("power")
                    ));
                }
            }
            "atx-power" => match c.call("getATXState", Value::Null).await {
                Ok(v) => {
                    let on = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
                    atx_state.set(format!(
                        "Power LED {} · HDD LED {}",
                        if on("power") { "on" } else { "off" },
                        if on("hdd") { "on" } else { "off" }
                    ));
                }
                Err(e) => atx_state.set(format!("State unavailable: {e}")),
            },
            _ => {}
        }
        power_ext.set(ext);
    };

    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let Some(c) = rpc.get_value() else { return };
        spawn_local(async move {
            let ext = match c.call("getActiveExtension", Value::Null).await {
                Ok(v) => as_string(&v),
                Err(_) => String::new(),
            };
            active_ext.set(ext.clone());
            load_power(c.clone(), ext).await;
            if let Ok(v) = c.call("getCloudState", Value::Null).await {
                let state = CloudState::from_value(&v);
                cloud_api.set(state.api_url.clone());
                cloud_app.set(state.app_url.clone());
                cloud.set(state);
            }
            if let Ok(v) = c.call("getTailscaleControlURL", Value::Null).await {
                ts_url.set(as_string(&v));
            }
            if let Ok(v) = c.call("getDevModeState", Value::Null).await {
                dev_mode.set(v.get("enabled").and_then(Value::as_bool).unwrap_or(false));
            }
            if let Ok(v) = c.call("getDevChannelState", Value::Null).await {
                dev_channel.set(v.as_bool().unwrap_or(false));
            }
            if let Ok(v) = c.call("getTLSState", Value::Null).await {
                tls_mode.set(v.get("mode").and_then(Value::as_str).unwrap_or_default().to_string());
            }
            if let Ok(v) = c.call("getSerialCommandHistory", Value::Null).await {
                serial_history.set(json_string_list(&v));
            }
            if let Ok(v) = c.call("getTimezone", Value::Null).await {
                timezone.set(as_string(&v));
            }
            if let Ok(v) = c.call("getKeyboardMacros", Value::Null).await {
                macros.set(MacroItem::list_from(&v));
            }
        });
    });

    let toggle_dc = move |ev: web_sys::Event| {
        let on = event_target_checked(&ev);
        dc_enabled.set(on);
        fire(rpc, "setDCPowerState", json!({ "enabled": on }));
    };
    let set_restore = move |ev: web_sys::Event| {
        let s: i32 = event_target_value(&ev).parse().unwrap_or(0);
        fire(rpc, "setDCRestoreState", json!({ "state": s }));
    };
    let atx =
        move |action: &'static str| fire(rpc, "setATXPowerAction", json!({ "action": action }));

    let save_cloud_url = move |_| {
        let Some(c) = rpc.get_value() else { return };
        let (api, app) = (cloud_api.get().trim().to_string(), cloud_app.get().trim().to_string());
        cloud_msg.set("Saving…".to_string());
        spawn_local(async move {
            match c.call("setCloudUrl", json!({ "apiUrl": api, "appUrl": app })).await {
                Ok(_) => {
                    cloud_msg.set("Cloud URLs saved".to_string());
                    if let Ok(v) = c.call("getCloudState", Value::Null).await {
                        cloud.set(CloudState::from_value(&v));
                    }
                }
                Err(e) => cloud_msg.set(format!("Failed: {e}")),
            }
        });
    };
    let deregister = move |_| {
        if !confirm("Remove this device from the cloud account?") {
            return;
        }
        let Some(c) = rpc.get_value() else { return };
        spawn_local(async move {
            match c.call("deregisterDevice", Value::Null).await {
                Ok(_) => {
                    cloud_msg.set("Device removed from the cloud".to_string());
                    if let Ok(v) = c.call("getCloudState", Value::Null).await {
                        cloud.set(CloudState::from_value(&v));
                    }
                }
                Err(e) => cloud_msg.set(format!("Failed: {e}")),
            }
        });
    };
    let save_ts = move |_| {
        fire_status(
            rpc,
            "setTailscaleControlURL",
            json!({ "controlURL": ts_url.get().trim() }),
            ts_msg,
            "Control server saved",
        )
    };

    let save_ext = move |_| {
        let Some(c) = rpc.get_value() else { return };
        let ext = active_ext.get();
        ext_msg.set("Applying…".to_string());
        spawn_local(async move {
            match c.call("setActiveExtension", json!({ "extensionId": ext })).await {
                Ok(_) => {
                    ext_msg.set("Extension applied".to_string());
                    load_power(c, ext).await;
                }
                Err(e) => ext_msg.set(format!("Failed: {e}")),
            }
        });
    };
    let toggle_dev = move |ev: web_sys::Event| {
        let on = event_target_checked(&ev);
        dev_mode.set(on);
        fire(rpc, "setDevModeState", json!({ "enabled": on }));
    };
    let toggle_devchan = move |ev: web_sys::Event| {
        let on = event_target_checked(&ev);
        dev_channel.set(on);
        fire(rpc, "setDevChannelState", json!({ "enabled": on }));
    };
    let save_tls = move |_| {
        let mode = tls_mode.get();
        let mut state = json!({ "mode": mode });
        if mode == "custom" {
            state["certificate"] = json!(tls_cert.get());
            state["privateKey"] = json!(tls_key.get());
        }
        fire_status(
            rpc,
            "setTLSState",
            json!({ "state": state }),
            tls_msg,
            "HTTPS settings applied",
        );
    };

    let clear_serial = move |_| {
        serial_history.set(Vec::new());
        fire(rpc, "deleteSerialCommandHistory", Value::Null);
    };

    let new_macro = move |_| {
        edit_id.set(None);
        edit_name.set(String::new());
        edit_steps.set(String::new());
    };
    let load_macro = move |m: MacroItem| {
        edit_id.set(Some(m.id.clone()));
        edit_name.set(m.name.clone());
        edit_steps.set(m.steps_text.clone());
        macro_msg.set(String::new());
    };
    let delete_macro = move |id: String, name: String| {
        if !confirm(&format!("Delete macro \"{name}\"?")) {
            return;
        }
        let kept: Vec<Value> = macros
            .get_untracked()
            .into_iter()
            .filter(|m| m.id != id)
            .map(|m| m.to_value())
            .collect();
        fire_status(
            rpc,
            "setKeyboardMacros",
            json!({ "macros": kept }),
            macro_msg,
            "Macro deleted",
        );
        macros.update(|list| list.retain(|m| m.id != id));
    };
    let save_macro = move |_| {
        let name = edit_name.get().trim().to_string();
        if name.is_empty() {
            macro_msg.set("Give the macro a name first".to_string());
            return;
        }
        let id = edit_id.get().unwrap_or_else(|| format!("macro-{}", js_now()));
        let item = MacroItem { id: id.clone(), name, steps_text: edit_steps.get() };
        let mut list = macros.get_untracked();
        if let Some(slot) = list.iter_mut().find(|m| m.id == id) {
            *slot = item.clone();
        } else {
            list.push(item.clone());
        }
        let payload: Vec<Value> = list.iter().map(MacroItem::to_value).collect();
        fire_status(
            rpc,
            "setKeyboardMacros",
            json!({ "macros": payload }),
            macro_msg,
            "Macro saved",
        );
        macros.set(list);
        edit_id.set(None);
        edit_name.set(String::new());
        edit_steps.set(String::new());
    };

    view! {
        <section>
            <h3>"Power"</h3>
            <Show when=move || power_ext.get() == "dc-power">
                <label class="toggle">
                    <input type="checkbox" prop:checked=move || dc_enabled.get() on:change=toggle_dc />
                    "DC power output"
                </label>
                <div class="muted">{move || dc_reading.get()}</div>
                <label class="field">
                    <span>"After a power loss"</span>
                    <select on:change=set_restore prop:value=move || dc_restore.get()>
                        <option value="0">"Stay off"</option>
                        <option value="1">"Turn on"</option>
                        <option value="2">"Restore last state"</option>
                    </select>
                </label>
            </Show>
            <Show when=move || power_ext.get() == "atx-power">
                <div class="muted">{move || atx_state.get()}</div>
                <div class="row">
                    <button on:click=move |_| atx("power-short")>"Power button"</button>
                    <button on:click=move |_| atx("power-long")>"Force off"</button>
                    <button on:click=move |_| atx("reset")>"Reset"</button>
                </div>
            </Show>
            <Show when=move || power_ext.get().is_empty()>
                <div class="muted">
                    "No power board is active. Pick the ATX or DC power board under Extension to control host power from here."
                </div>
            </Show>
        </section>

        <section>
            <h3>"Cloud"</h3>
            <dl class="kv">
                <dt>"Status"</dt>
                <dd>
                    <span class="dot" class:on=move || cloud.get().connected></span>
                    {move || if cloud.get().connected { "Connected" } else { "Not connected" }}
                </dd>
                <dt>"API"</dt>
                <dd>{move || or_dash(cloud.get().api_url)}</dd>
                <dt>"App"</dt>
                <dd>{move || or_dash(cloud.get().app_url)}</dd>
            </dl>
            <label class="field">
                <span>"API URL"</span>
                <input
                    type="url"
                    placeholder="https://api.example.com"
                    prop:value=move || cloud_api.get()
                    on:input=move |ev| cloud_api.set(event_target_value(&ev))
                />
            </label>
            <label class="field">
                <span>"App URL"</span>
                <input
                    type="url"
                    placeholder="https://app.example.com"
                    prop:value=move || cloud_app.get()
                    on:input=move |ev| cloud_app.set(event_target_value(&ev))
                />
            </label>
            <div class="row">
                <button on:click=save_cloud_url>"Save URLs"</button>
                <button class="tb-danger" on:click=deregister>"Deregister"</button>
            </div>
            <div class="muted">{move || cloud_msg.get()}</div>
        </section>

        <section>
            <h3>"Tailscale"</h3>
            <label class="field">
                <span>"Control server"</span>
                <input
                    type="url"
                    placeholder="https://controlplane.tailscale.com"
                    prop:value=move || ts_url.get()
                    on:input=move |ev| ts_url.set(event_target_value(&ev))
                />
            </label>
            <div class="hint">"Change only if you run your own server, such as Headscale."</div>
            <button on:click=save_ts>"Save"</button>
            <div class="muted">{move || ts_msg.get()}</div>
        </section>

        <section>
            <h3>"Extension"</h3>
            <label class="field">
                <span>"Hardware extension"</span>
                <select
                    prop:value=move || active_ext.get()
                    on:change=move |ev| active_ext.set(event_target_value(&ev))
                >
                    {EXTENSIONS
                        .iter()
                        .map(|(id, name)| view! { <option value=*id>{*name}</option> })
                        .collect_view()}
                </select>
            </label>
            <div class="hint">"A power board adds its controls to the Power section."</div>
            <button on:click=save_ext>"Apply"</button>
            <div class="muted">{move || ext_msg.get()}</div>
        </section>

        <section>
            <h3>"Developer"</h3>
            <label class="toggle">
                <input type="checkbox" prop:checked=move || dev_mode.get() on:change=toggle_dev />
                "Developer mode"
            </label>
            <label class="toggle">
                <input
                    type="checkbox"
                    prop:checked=move || dev_channel.get()
                    on:change=toggle_devchan
                />
                "Pre-release updates"
            </label>
        </section>

        <section>
            <h3>"HTTPS"</h3>
            <label class="field">
                <span>"Certificate"</span>
                <select
                    prop:value=move || tls_mode.get()
                    on:change=move |ev| tls_mode.set(event_target_value(&ev))
                >
                    <option value="">"Off (plain HTTP)"</option>
                    <option value="self-signed">"Self-signed"</option>
                    <option value="custom">"Custom certificate"</option>
                </select>
            </label>
            <Show when=move || tls_mode.get() == "custom">
                <textarea
                    rows="3"
                    placeholder="-----BEGIN CERTIFICATE-----"
                    on:input=move |ev| tls_cert.set(event_target_value(&ev))
                ></textarea>
                <textarea
                    rows="3"
                    placeholder="-----BEGIN PRIVATE KEY-----"
                    on:input=move |ev| tls_key.set(event_target_value(&ev))
                ></textarea>
            </Show>
            <button on:click=save_tls>"Apply"</button>
            <div class="muted">{move || tls_msg.get()}</div>
        </section>

        <section>
            <h3>"Serial console"</h3>
            // `sendCustomCommand` only logs on the device until a serial
            // adapter is supported, so don't offer a Send button that does
            // nothing. Earlier history stays visible and can be cleared.
            <div class="muted">"Serial console is not supported on this device yet."</div>
            <Show when=move || !serial_history.get().is_empty()>
                <ul class="plain">
                    {move || {
                        serial_history
                            .get()
                            .into_iter()
                            .map(|cmd| view! { <li>{cmd}</li> })
                            .collect_view()
                    }}
                </ul>
                <button on:click=clear_serial>"Clear history"</button>
            </Show>
        </section>

        <section>
            <h3>"Time zone"</h3>
            <div>{move || or_dash(timezone.get())}</div>
            <div class="hint">"Changing the device time zone is not supported yet."</div>
        </section>

        <section>
            <h3>"Keyboard macros"</h3>
            <Show when=move || macros.get().is_empty()>
                <div class="muted">"No macros yet."</div>
            </Show>
            <For
                each=move || macros.get()
                key=|m| m.id.clone()
                children=move |m: MacroItem| {
                    let (m_edit, m_run) = (m.clone(), m.clone());
                    let (del_id, del_name) = (m.id.clone(), m.name.clone());
                    view! {
                        <div class="row">
                            <span>{m.name.clone()}</span>
                            <button on:click=move |_| {
                                let steps = m_run.to_value().get("steps").cloned().unwrap_or(json!([]));
                                fire(rpc, "executeKeyboardMacro", steps);
                            }>"Run"</button>
                            <button on:click=move |_| load_macro(m_edit.clone())>"Edit"</button>
                            <button class="tb-danger" on:click=move |_| {
                                delete_macro(del_id.clone(), del_name.clone())
                            }>"Delete"</button>
                        </div>
                    }
                }
            />
            <label class="field">
                <span>{move || if edit_id.get().is_some() { "Edit macro" } else { "New macro" }}</span>
                <input
                    type="text"
                    placeholder="Name"
                    prop:value=move || edit_name.get()
                    on:input=move |ev| edit_name.set(event_target_value(&ev))
                />
            </label>
            <textarea
                rows="4"
                placeholder="ControlLeft,AltLeft | Delete | 50"
                prop:value=move || edit_steps.get()
                on:input=move |ev| edit_steps.set(event_target_value(&ev))
            ></textarea>
            <div class="hint">"One step per line: modifiers | keys | delay in ms"</div>
            <div class="row">
                <button on:click=save_macro>"Save macro"</button>
                <Show when=move || edit_id.get().is_some()>
                    <button on:click=move |_| new_macro(())>"Cancel edit"</button>
                </Show>
            </div>
            <div class="muted">{move || macro_msg.get()}</div>
        </section>

        <section>
            <h3>"Danger zone"</h3>
            <div class="row">
                <button
                    class="tb-danger"
                    on:click=move |_| {
                        if confirm("Reset all settings to their defaults?") {
                            fire(rpc, "resetConfig", Value::Null);
                        }
                    }
                >
                    "Reset settings"
                </button>
                <button
                    class="tb-danger"
                    on:click=move |_| {
                        if confirm("Factory reset: erase all data on the device and reboot?") {
                            fire(rpc, "factoryReset", Value::Null);
                        }
                    }
                >
                    "Factory reset"
                </button>
            </div>
        </section>
    }
}

/// `getCloudState` reply: `{ connected, url, appUrl }`.
#[derive(Clone, Default, PartialEq)]
struct CloudState {
    connected: bool,
    api_url: String,
    app_url: String,
}

impl CloudState {
    fn from_value(v: &Value) -> Self {
        Self {
            connected: v.get("connected").and_then(Value::as_bool).unwrap_or(false),
            api_url: json_str(v, "url"),
            app_url: json_str(v, "appUrl"),
        }
    }
}

fn or_dash(s: String) -> String {
    if s.is_empty() { "—".to_string() } else { s }
}

#[derive(Clone, PartialEq)]
struct MacroItem {
    id: String,
    name: String,
    steps_text: String,
}

impl MacroItem {
    fn list_from(v: &Value) -> Vec<Self> {
        v.as_array()
            .map(|arr| {
                arr.iter()
                    .map(|m| {
                        let steps = m
                            .get("steps")
                            .and_then(Value::as_array)
                            .map(|steps| {
                                steps
                                    .iter()
                                    .map(|s| {
                                        let mods = join_strs(s.get("modifiers"));
                                        let keys = join_strs(s.get("keys"));
                                        let delay =
                                            s.get("delay").and_then(Value::as_u64).unwrap_or(0);
                                        format!("{mods} | {keys} | {delay}")
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_default();
                        MacroItem {
                            id: json_str(m, "id"),
                            name: json_str(m, "name"),
                            steps_text: steps,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn to_value(&self) -> Value {
        let steps: Vec<Value> = self
            .steps_text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                let mods = split_csv(parts.first().copied().unwrap_or(""));
                let keys = split_csv(parts.get(1).copied().unwrap_or(""));
                let delay: u64 = parts.get(2).and_then(|d| d.trim().parse().ok()).unwrap_or(50);
                json!({ "keys": keys, "modifiers": mods, "delay": delay })
            })
            .collect();
        json!({ "id": self.id, "name": self.name, "steps": steps })
    }
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

fn join_strs(v: Option<&Value>) -> String {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","))
        .unwrap_or_default()
}

fn confirm(msg: &str) -> bool {
    web_sys::window().and_then(|w| w.confirm_with_message(msg).ok()).unwrap_or(false)
}

fn js_now() -> f64 {
    js_sys::Date::now()
}
