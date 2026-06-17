use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};

use crate::util::{Rpc, as_string, fire, json_str, json_string_list};

#[component]
pub fn AdvancedSettings(open: RwSignal<bool>, rpc: Rpc) -> impl IntoView {
    let dc_enabled = RwSignal::new(false);
    let dc_supported = RwSignal::new(false);
    let atx_state = RwSignal::new(String::new());
    let cloud_state = RwSignal::new(String::new());
    let cloud_api = RwSignal::new(String::new());
    let cloud_app = RwSignal::new(String::new());
    let ts_url = RwSignal::new(String::new());
    let active_ext = RwSignal::new(String::new());
    let dev_mode = RwSignal::new(false);
    let dev_channel = RwSignal::new(false);
    let tls_mode = RwSignal::new(String::new());
    let tls_cert = RwSignal::new(String::new());
    let tls_key = RwSignal::new(String::new());
    let serial_cmd = RwSignal::new(String::new());
    let serial_history = RwSignal::new(Vec::<String>::new());
    let timezones = RwSignal::new(Vec::<String>::new());
    let macros = RwSignal::new(Vec::<MacroItem>::new());
    let edit_id = RwSignal::new(Option::<String>::None);
    let edit_name = RwSignal::new(String::new());
    let edit_steps = RwSignal::new(String::new());

    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let Some(c) = rpc.get_value() else { return };
        spawn_local(async move {
            if let Ok(v) = c.call("getDCPowerState", Value::Null).await {
                dc_enabled.set(v.get("enabled").and_then(Value::as_bool).unwrap_or(false));
                dc_supported.set(v.get("supported").and_then(Value::as_bool).unwrap_or(false));
            }
            if let Ok(v) = c.call("getATXState", Value::Null).await {
                atx_state.set(compact_json(&v));
            }
            if let Ok(v) = c.call("getCloudState", Value::Null).await {
                cloud_state.set(compact_json(&v));
            }
            if let Ok(v) = c.call("getTailscaleControlURL", Value::Null).await {
                ts_url.set(as_string(&v));
            }
            if let Ok(v) = c.call("getActiveExtension", Value::Null).await {
                active_ext.set(as_string(&v));
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
            if let Ok(v) = c.call("getTimezones", Value::Null).await {
                timezones.set(json_string_list(&v));
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
        fire(rpc, "setCloudUrl", json!({ "apiUrl": cloud_api.get(), "appUrl": cloud_app.get() }))
    };
    let deregister = move |_| {
        if confirm("Deregister this device from the cloud?") {
            fire(rpc, "deregisterDevice", Value::Null);
        }
    };
    let save_ts =
        move |_| fire(rpc, "setTailscaleControlURL", json!({ "controlURL": ts_url.get() }));

    let save_ext =
        move |_| fire(rpc, "setActiveExtension", json!({ "extensionId": active_ext.get() }));
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
        fire(rpc, "setTLSState", json!({ "state": state }));
    };

    let send_serial = move |_| {
        let cmd = serial_cmd.get();
        if cmd.is_empty() {
            return;
        }
        fire(rpc, "sendCustomCommand", json!({ "command": cmd }));
        serial_history.update(|h| h.push(cmd.clone()));
        let hist = serial_history.get_untracked();
        fire(rpc, "setSerialCommandHistory", json!({ "commandHistory": hist }));
        serial_cmd.set(String::new());
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
    };
    let delete_macro = move |id: String| {
        let kept: Vec<Value> = macros
            .get_untracked()
            .into_iter()
            .filter(|m| m.id != id)
            .map(|m| m.to_value())
            .collect();
        fire(rpc, "setKeyboardMacros", json!({ "macros": kept }));
        macros.update(|list| list.retain(|m| m.id != id));
    };
    let save_macro = move |_| {
        let name = edit_name.get();
        if name.is_empty() {
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
        fire(rpc, "setKeyboardMacros", json!({ "macros": payload }));
        macros.set(list);
        new_macro(());
    };

    view! {
        <section>
            <h3>"Power"</h3>
            <Show
                when=move || dc_supported.get()
                fallback=|| view! { <div class="muted">"DC power not supported"</div> }
            >
                <label class="toggle">
                    <input type="checkbox" prop:checked=move || dc_enabled.get() on:change=toggle_dc />
                    "DC power"
                </label>
                <select on:change=set_restore>
                    <option value="0">"Restore: off"</option>
                    <option value="1">"Restore: on"</option>
                    <option value="2">"Restore: last"</option>
                </select>
            </Show>
            <div class="muted">"ATX: "{move || atx_state.get()}</div>
            <div class="row">
                <button on:click=move |_| atx("power-short")>"Power"</button>
                <button on:click=move |_| atx("power-long")>"Force off"</button>
                <button on:click=move |_| atx("reset")>"Reset"</button>
            </div>
        </section>

        <section>
            <h3>"Cloud"</h3>
            <div class="muted">{move || cloud_state.get()}</div>
            <input
                type="text"
                placeholder="API URL"
                prop:value=move || cloud_api.get()
                on:input=move |ev| cloud_api.set(event_target_value(&ev))
            />
            <input
                type="text"
                placeholder="App URL"
                prop:value=move || cloud_app.get()
                on:input=move |ev| cloud_app.set(event_target_value(&ev))
            />
            <div class="row">
                <button on:click=save_cloud_url>"Set cloud URLs"</button>
                <button class="tb-danger" on:click=deregister>"Deregister"</button>
            </div>
        </section>

        <section>
            <h3>"Tailscale"</h3>
            <input
                type="text"
                placeholder="Control URL"
                prop:value=move || ts_url.get()
                on:input=move |ev| ts_url.set(event_target_value(&ev))
            />
            <button on:click=save_ts>"Set control URL"</button>
        </section>

        <section>
            <h3>"Extension"</h3>
            <input
                type="text"
                placeholder="extension id (empty = none)"
                prop:value=move || active_ext.get()
                on:input=move |ev| active_ext.set(event_target_value(&ev))
            />
            <button on:click=save_ext>"Set extension"</button>
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
                "Pre-release channel"
            </label>
        </section>

        <section>
            <h3>"TLS"</h3>
            <select prop:value=move || tls_mode.get() on:change=move |ev| tls_mode.set(event_target_value(&ev))>
                <option value="">"Disabled"</option>
                <option value="self-signed">"Self-signed"</option>
                <option value="custom">"Custom"</option>
            </select>
            <Show when=move || tls_mode.get() == "custom">
                <textarea
                    rows="3"
                    placeholder="Certificate (PEM)"
                    on:input=move |ev| tls_cert.set(event_target_value(&ev))
                ></textarea>
                <textarea
                    rows="3"
                    placeholder="Private key (PEM)"
                    on:input=move |ev| tls_key.set(event_target_value(&ev))
                ></textarea>
            </Show>
            <button on:click=save_tls>"Apply TLS"</button>
        </section>

        <section>
            <h3>"Serial console"</h3>
            <div class="row">
                <input
                    type="text"
                    placeholder="command"
                    prop:value=move || serial_cmd.get()
                    on:input=move |ev| serial_cmd.set(event_target_value(&ev))
                />
                <button on:click=send_serial>"Send"</button>
            </div>
            <div class="muted">
                {move || {
                    let h = serial_history.get();
                    if h.is_empty() { "no history".to_string() } else { h.join(" · ") }
                }}
            </div>
            <button on:click=clear_serial>"Clear history"</button>
        </section>

        <section>
            <h3>"Timezone"</h3>
            <select>
                {move || {
                    timezones
                        .get()
                        .into_iter()
                        .map(|tz| view! { <option value=tz.clone()>{tz.clone()}</option> })
                        .collect_view()
                }}
            </select>
        </section>

        <section>
            <h3>"Keyboard macros"</h3>
            <For
                each=move || macros.get()
                key=|m| m.id.clone()
                children=move |m: MacroItem| {
                    let (m_edit, m_run, m_del) = (m.clone(), m.clone(), m.id.clone());
                    view! {
                        <div class="row">
                            <span class="muted">{m.name.clone()}</span>
                            <button on:click=move |_| {
                                let steps = m_run.to_value().get("steps").cloned().unwrap_or(json!([]));
                                fire(rpc, "executeKeyboardMacro", steps);
                            }>"Run"</button>
                            <button on:click=move |_| load_macro(m_edit.clone())>"Edit"</button>
                            <button on:click={
                                let id = m_del.clone();
                                move |_| delete_macro(id.clone())
                            }>"Del"</button>
                        </div>
                    }
                }
            />
            <input
                type="text"
                placeholder="Macro name"
                prop:value=move || edit_name.get()
                on:input=move |ev| edit_name.set(event_target_value(&ev))
            />
            <textarea
                rows="4"
                placeholder="One step per line: modifiers | keys | delayMs&#10;e.g. ControlLeft,AltLeft | Delete | 50"
                prop:value=move || edit_steps.get()
                on:input=move |ev| edit_steps.set(event_target_value(&ev))
            ></textarea>
            <div class="row">
                <button on:click=save_macro>"Save macro"</button>
                <button on:click=move |_| new_macro(())>"New"</button>
            </div>
        </section>

        <section>
            <h3>"Danger zone"</h3>
            <div class="row">
                <button
                    class="tb-danger"
                    on:click=move |_| {
                        if confirm("Reset configuration to defaults?") {
                            fire(rpc, "resetConfig", Value::Null);
                        }
                    }
                >
                    "Reset config"
                </button>
                <button
                    class="tb-danger"
                    on:click=move |_| {
                        if confirm("FACTORY RESET — erase all data and reboot?") {
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

fn compact_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn confirm(msg: &str) -> bool {
    web_sys::window().and_then(|w| w.confirm_with_message(msg).ok()).unwrap_or(false)
}

fn js_now() -> f64 {
    js_sys::Date::now()
}
