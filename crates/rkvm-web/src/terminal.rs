use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{ArrayBuffer, Function, Uint8Array};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{KeyboardEvent, MessageEvent, RtcDataChannelType, WheelEvent};

use crate::net::peer::Peer;

const SCROLLBACK_LEN: usize = 5_000;
const CHAR_W: f64 = 13.0 * 0.6;
const LINE_H: f64 = 13.0 * 1.4;

type SharedParser = StoredValue<Rc<RefCell<vt100::Parser>>, LocalStorage>;

#[component]
pub fn Terminal(
    open: RwSignal<bool>,
    peer: StoredValue<Option<Rc<Peer>>, LocalStorage>,
) -> impl IntoView {
    let parser: SharedParser =
        StoredValue::new_local(Rc::new(RefCell::new(vt100::Parser::new(24, 80, SCROLLBACK_LEN))));
    let version = RwSignal::new(0u32);
    let screen_ref = NodeRef::<html::Div>::new();
    let wired = StoredValue::new_local(false);

    let dirty = StoredValue::new_local(Rc::new(Cell::new(false)));
    let schedule = move || {
        if dirty.get_value().replace(true) {
            return;
        }
        let frame = Closure::once_into_js(move || {
            dirty.get_value().set(false);
            version.update(|v| *v = v.wrapping_add(1));
        });
        if let Some(win) = web_sys::window() {
            let _ = win.request_animation_frame(frame.unchecked_ref::<Function>());
        }
    };

    Effect::new(move |_| {
        if wired.get_value() {
            return;
        }
        let Some(p) = peer.get_value() else { return };
        wired.set_value(true);

        let channel = p.terminal.clone();
        channel.set_binary_type(RtcDataChannelType::Arraybuffer);
        let on_msg = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
            let bytes: Vec<u8> = if let Ok(buf) = ev.data().dyn_into::<ArrayBuffer>() {
                Uint8Array::new(&buf).to_vec()
            } else if let Some(s) = ev.data().as_string() {
                s.into_bytes()
            } else {
                return;
            };
            parser.get_value().borrow_mut().process(&bytes);
            schedule();
        });
        channel.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        on_msg.forget();
    });

    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let Some(el) = screen_ref.get() else { return };
        let w = (el.client_width() as f64 - 24.0).max(0.0);
        let h = (el.client_height() as f64 - 20.0).max(0.0);
        let cols = (w / CHAR_W).floor().clamp(20.0, 500.0) as u16;
        let rows = (h / LINE_H).floor().clamp(6.0, 200.0) as u16;
        parser.get_value().borrow_mut().screen_mut().set_size(rows, cols);
        schedule();

        if let Some(p) = peer.get_value() {
            let _ = p.terminal.send_with_str(&format!("{{\"cols\":{cols},\"rows\":{rows}}}"));
            let _ = p.terminal.send_with_u8_array(b"\r");
        }
        let _ = el.focus();
    });

    let on_key = move |ev: KeyboardEvent| {
        let Some(p) = peer.get_value() else { return };
        let key = ev.key();
        let bytes: Vec<u8> = match key.as_str() {
            "Enter" => vec![b'\r'],
            "Backspace" => vec![0x7f],
            "Tab" => vec![b'\t'],
            "Escape" => vec![0x1b],
            "Delete" => vec![0x1b, b'[', b'3', b'~'],
            "Home" => vec![0x1b, b'[', b'H'],
            "End" => vec![0x1b, b'[', b'F'],
            "PageUp" => vec![0x1b, b'[', b'5', b'~'],
            "PageDown" => vec![0x1b, b'[', b'6', b'~'],
            "ArrowUp" => vec![0x1b, b'[', b'A'],
            "ArrowDown" => vec![0x1b, b'[', b'B'],
            "ArrowRight" => vec![0x1b, b'[', b'C'],
            "ArrowLeft" => vec![0x1b, b'[', b'D'],
            k if k.chars().count() == 1 => {
                let c = k.chars().next().expect("len 1");
                if ev.ctrl_key() && c.is_ascii_alphabetic() {
                    vec![(c.to_ascii_uppercase() as u8) & 0x1f]
                } else if ev.ctrl_key() || ev.meta_key() {
                    return;
                } else {
                    k.as_bytes().to_vec()
                }
            }
            _ => return,
        };
        ev.prevent_default();
        parser.get_value().borrow_mut().screen_mut().set_scrollback(0);
        schedule();
        let _ = p.terminal.send_with_u8_array(&bytes);
    };

    let on_wheel = move |ev: WheelEvent| {
        ev.prevent_default();
        let step = 3usize;
        let parser = parser.get_value();
        let mut parser = parser.borrow_mut();
        let cur = parser.screen().scrollback();
        let next = if ev.delta_y() < 0.0 { cur + step } else { cur.saturating_sub(step) };
        parser.screen_mut().set_scrollback(next);
        drop(parser);
        schedule();
    };

    let lines = move || {
        version.track();
        let parser = parser.get_value();
        let parser = parser.borrow();
        let screen = parser.screen();
        let (_, cols) = screen.size();
        let (cy, cx) = screen.cursor_position();
        let show_cursor = !screen.hide_cursor() && screen.scrollback() == 0;
        let cx = cx as usize;

        screen
            .rows(0, cols)
            .enumerate()
            .map(|(r, line)| {
                if show_cursor && r as u16 == cy {
                    let chars: Vec<char> = line.chars().collect();
                    let pad = cx.saturating_sub(chars.len());
                    let before: String =
                        chars.iter().take(cx).collect::<String>() + &" ".repeat(pad);
                    let cur = chars.get(cx).copied().unwrap_or(' ').to_string();
                    let after: String = chars.iter().skip(cx + 1).collect();
                    view! {
                        <div class="trow">
                            {before}<span class="term-cursor">{cur}</span>{after}
                        </div>
                    }
                    .into_any()
                } else {
                    view! { <div class="trow">{line}</div> }.into_any()
                }
            })
            .collect::<Vec<_>>()
    };

    view! {
        <Show when=move || open.get()>
            <div class="term">
                <header>
                    <span>"Serial / Terminal"</span>
                    <button title="Close" on:click=move |_| open.set(false)>
                        "✕"
                    </button>
                </header>
                <div
                    class="term-screen"
                    tabindex="0"
                    data-term="1"
                    node_ref=screen_ref
                    on:keydown=on_key
                    on:wheel=on_wheel
                >
                    {lines}
                </div>
            </div>
        </Show>
    }
}
