mod keymap;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{ArrayBuffer, Uint8Array};
use rkvm_proto::hidrpc::{
    Message, new_handshake_message, new_keyboard_report_message, new_mouse_report_message,
    new_pointer_report_message,
};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    EventTarget, HtmlVideoElement, KeyboardEvent, MessageEvent, MouseEvent, RtcDataChannelType,
    WheelEvent,
};

use crate::net::peer::Peer;
use crate::rpc::RpcClient;

pub type MouseMode = Rc<Cell<bool>>;

#[derive(Default)]
struct KbState {
    modifiers: u8,
    keys: Vec<u8>,
}

pub fn attach(video: HtmlVideoElement, peer: Rc<Peer>, rpc: Rc<RpcClient>, mouse_mode: MouseMode) {
    peer.hidrpc.set_binary_type(RtcDataChannelType::Arraybuffer);
    let _ = peer.hidrpc.send_with_u8_array(&new_handshake_message().marshal());
    {
        let on_msg = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
            if let Ok(buf) = ev.data().dyn_into::<ArrayBuffer>() {
                let bytes = Uint8Array::new(&buf).to_vec();
                if let Ok(msg) = Message::unmarshal(&bytes) {
                    tracing::debug!("hidrpc inbound: {:?}", msg.r#type);
                }
            }
        });
        peer.hidrpc.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        on_msg.forget();
    }

    let kb = Rc::new(RefCell::new(KbState::default()));
    let window: EventTarget = web_sys::window().expect("window").into();

    {
        let peer = peer.clone();
        let kb = kb.clone();
        let cb = Closure::<dyn FnMut(KeyboardEvent)>::new(move |ev: KeyboardEvent| {
            if editable_focused() {
                return;
            }
            ev.prevent_default();
            let code = ev.code();
            let mut st = kb.borrow_mut();
            if let Some(bit) = keymap::code_to_modifier(&code) {
                st.modifiers |= bit;
            } else if let Some(usage) = keymap::code_to_usage(&code) {
                if !st.keys.contains(&usage) && st.keys.len() < 6 {
                    st.keys.push(usage);
                }
            } else {
                return;
            }
            send_keyboard(&peer, &st);
        });
        let _ = window.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        cb.forget();
    }
    {
        let peer = peer.clone();
        let kb = kb.clone();
        let cb = Closure::<dyn FnMut(KeyboardEvent)>::new(move |ev: KeyboardEvent| {
            if editable_focused() {
                return;
            }
            ev.prevent_default();
            let code = ev.code();
            let mut st = kb.borrow_mut();
            if let Some(bit) = keymap::code_to_modifier(&code) {
                st.modifiers &= !bit;
            } else if let Some(usage) = keymap::code_to_usage(&code) {
                st.keys.retain(|&k| k != usage);
            } else {
                return;
            }
            send_keyboard(&peer, &st);
        });
        let _ = window.add_event_listener_with_callback("keyup", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    {
        let peer = peer.clone();
        let video_for_handler = video.clone();
        let mode = mouse_mode.clone();
        let cb = Closure::<dyn FnMut(MouseEvent)>::new(move |ev: MouseEvent| {
            let buttons = (ev.buttons() & 0x1f) as u8;
            if mode.get() {
                if ev.type_() == "mousedown" && !pointer_locked(&video_for_handler) {
                    video_for_handler.request_pointer_lock();
                }
                let dx = ev.movement_x().clamp(-127, 127) as i8;
                let dy = ev.movement_y().clamp(-127, 127) as i8;
                if dx == 0 && dy == 0 && ev.type_() == "mousemove" {
                    return;
                }
                let bytes = new_mouse_report_message(dx, dy, buttons).marshal();
                let _ = peer.hidrpc_unreliable_nonordered.send_with_u8_array(&bytes);
            } else if let Some((x, y, b)) = abs_position(&video_for_handler, &ev) {
                let bytes = new_pointer_report_message(x, y, b).marshal();
                let _ = peer.hidrpc_unreliable_nonordered.send_with_u8_array(&bytes);
            }
        });
        let target: &EventTarget = video.as_ref();
        for name in ["mousemove", "mousedown", "mouseup"] {
            let _ = target.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref());
        }
        cb.forget();
    }

    {
        let cb = Closure::<dyn FnMut(MouseEvent)>::new(move |ev: MouseEvent| ev.prevent_default());
        let target: &EventTarget = video.as_ref();
        let _ = target.add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    {
        let cb = Closure::<dyn FnMut(WheelEvent)>::new(move |ev: WheelEvent| {
            ev.prevent_default();
            let wheel_y = clamp_wheel(ev.delta_y());
            if wheel_y == 0 {
                return;
            }
            rpc.notify("wheelReport", json!({ "wheelY": wheel_y, "wheelX": 0 }));
        });
        let target: &EventTarget = video.as_ref();
        let _ = target.add_event_listener_with_callback("wheel", cb.as_ref().unchecked_ref());
        cb.forget();
    }
}

fn pointer_locked(_video: &HtmlVideoElement) -> bool {
    web_sys::window().and_then(|w| w.document()).and_then(|d| d.pointer_lock_element()).is_some()
}

fn editable_focused() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .map(|el| {
            matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
                || el.has_attribute("contenteditable")
                || el.has_attribute("data-term")
        })
        .unwrap_or(false)
}

fn send_keyboard(peer: &Peer, st: &KbState) {
    let mut keys = [0u8; 6];
    for (slot, &usage) in keys.iter_mut().zip(st.keys.iter()) {
        *slot = usage;
    }
    let bytes = new_keyboard_report_message(st.modifiers, &keys).marshal();
    let _ = peer.hidrpc_unreliable_nonordered.send_with_u8_array(&bytes);
}

fn abs_position(video: &HtmlVideoElement, ev: &MouseEvent) -> Option<(i32, i32, u8)> {
    let (cw, ch) = (video.client_width() as f64, video.client_height() as f64);
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let rel_x = (ev.offset_x() as f64 / cw).clamp(0.0, 1.0);
    let rel_y = (ev.offset_y() as f64 / ch).clamp(0.0, 1.0);
    let buttons = (ev.buttons() & 0x1f) as u8;
    Some(((rel_x * 32767.0).round() as i32, (rel_y * 32767.0).round() as i32, buttons))
}

pub fn send_key_combo(peer: &Peer, modifiers: u8, keys: &[u8]) {
    let press = new_keyboard_report_message(modifiers, keys).marshal();
    let _ = peer.hidrpc_unreliable_nonordered.send_with_u8_array(&press);
    let release = new_keyboard_report_message(0, &[]).marshal();
    let _ = peer.hidrpc_unreliable_nonordered.send_with_u8_array(&release);
}

fn clamp_wheel(delta_y: f64) -> i32 {
    let stepped = if delta_y.abs() >= 100.0 { delta_y / 100.0 } else { delta_y.signum() };
    (-stepped).clamp(-127.0, 127.0) as i32
}
