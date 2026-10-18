use std::rc::Rc;

use js_sys::{Array, Function, Promise, Reflect};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlTextAreaElement;

use crate::rpc::RpcClient;

pub type Rpc = StoredValue<Option<Rc<RpcClient>>, LocalStorage>;

pub fn fire(rpc: Rpc, method: &'static str, params: Value) {
    if let Some(client) = rpc.get_value() {
        spawn_local(async move {
            let _ = client.call(method, params).await;
        });
    }
}

/// Like `fire`, but writes the outcome into `status` so the user can see
/// whether the device accepted the change.
pub fn fire_status(
    rpc: Rpc,
    method: &'static str,
    params: Value,
    status: RwSignal<String>,
    ok: &'static str,
) {
    let Some(client) = rpc.get_value() else {
        status.set("Not connected to the device".to_string());
        return;
    };
    status.set("Saving…".to_string());
    spawn_local(async move {
        match client.call(method, params).await {
            Ok(_) => status.set(ok.to_string()),
            Err(e) => status.set(format!("Failed: {e}")),
        }
    });
}

pub fn json_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn as_string(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub fn json_string_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn call_method(target: &JsValue, name: &str, args: &Array) -> Result<JsValue, JsValue> {
    let f = Reflect::get(target, &JsValue::from_str(name))?.dyn_into::<Function>()?;
    f.apply(target, args)
}

fn js_message(e: &JsValue) -> String {
    e.dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

/// Toggles page fullscreen. `requestFullscreen` returns a promise that
/// browsers reject (permissions, iframes, missing user gesture); surface that
/// instead of leaving an unhandled rejection.
pub async fn toggle_fullscreen() -> Result<(), String> {
    let doc = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let doc_js: &JsValue = doc.as_ref();
    let active = Reflect::get(doc_js, &JsValue::from_str("fullscreenElement"))
        .map(|v| !v.is_null() && !v.is_undefined())
        .unwrap_or(false);
    let result = if active {
        call_method(doc_js, "exitFullscreen", &Array::new())
    } else {
        let el = doc.document_element().ok_or("no document element")?;
        call_method(el.as_ref(), "requestFullscreen", &Array::new())
    };
    let value = result.map_err(|e| format!("Fullscreen unavailable: {}", js_message(&e)))?;
    if let Ok(promise) = value.dyn_into::<Promise>() {
        JsFuture::from(promise)
            .await
            .map_err(|e| format!("Fullscreen blocked by the browser: {}", js_message(&e)))?;
    }
    Ok(())
}

/// Copies text to the clipboard. The async Clipboard API only exists in secure
/// contexts, and the device is usually opened over plain http, so fall back to
/// a temporary textarea and `execCommand("copy")`.
pub async fn copy_text(text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let clipboard = Reflect::get(window.navigator().as_ref(), &JsValue::from_str("clipboard"))
        .unwrap_or(JsValue::UNDEFINED);
    if clipboard.is_object()
        && let Ok(p) = call_method(&clipboard, "writeText", &Array::of1(&JsValue::from_str(text)))
        && let Ok(promise) = p.dyn_into::<Promise>()
        && JsFuture::from(promise).await.is_ok()
    {
        return Ok(());
    }
    let doc = window.document().ok_or("no document")?;
    let body = doc.body().ok_or("no body")?;
    let area = doc
        .create_element("textarea")
        .map_err(|e| js_message(&e))?
        .dyn_into::<HtmlTextAreaElement>()
        .map_err(|_| "textarea cast failed")?;
    area.set_value(text);
    area.set_attribute("style", "position:fixed;opacity:0").map_err(|e| js_message(&e))?;
    body.append_child(&area).map_err(|e| js_message(&e))?;
    area.select();
    let copied = call_method(doc.as_ref(), "execCommand", &Array::of1(&JsValue::from_str("copy")))
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(false);
    area.remove();
    if copied {
        Ok(())
    } else {
        Err("Copy failed: select the text and copy it manually".to_string())
    }
}
