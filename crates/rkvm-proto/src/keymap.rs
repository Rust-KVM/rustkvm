use anyhow::{Result, anyhow, bail};

use crate::hidrpc::{HID_KEY_BUFFER_SIZE, KeyboardMacroStep};

pub fn code_to_modifier(code: &str) -> Option<u8> {
    Some(match code {
        "ControlLeft" => 1 << 0,
        "ShiftLeft" => 1 << 1,
        "AltLeft" => 1 << 2,
        "MetaLeft" | "OSLeft" => 1 << 3,
        "ControlRight" => 1 << 4,
        "ShiftRight" => 1 << 5,
        "AltRight" => 1 << 6,
        "MetaRight" | "OSRight" => 1 << 7,
        _ => return None,
    })
}

pub fn code_to_usage(code: &str) -> Option<u8> {
    let usage = match code {
        "KeyA" => 0x04,
        "KeyB" => 0x05,
        "KeyC" => 0x06,
        "KeyD" => 0x07,
        "KeyE" => 0x08,
        "KeyF" => 0x09,
        "KeyG" => 0x0a,
        "KeyH" => 0x0b,
        "KeyI" => 0x0c,
        "KeyJ" => 0x0d,
        "KeyK" => 0x0e,
        "KeyL" => 0x0f,
        "KeyM" => 0x10,
        "KeyN" => 0x11,
        "KeyO" => 0x12,
        "KeyP" => 0x13,
        "KeyQ" => 0x14,
        "KeyR" => 0x15,
        "KeyS" => 0x16,
        "KeyT" => 0x17,
        "KeyU" => 0x18,
        "KeyV" => 0x19,
        "KeyW" => 0x1a,
        "KeyX" => 0x1b,
        "KeyY" => 0x1c,
        "KeyZ" => 0x1d,
        "Digit1" => 0x1e,
        "Digit2" => 0x1f,
        "Digit3" => 0x20,
        "Digit4" => 0x21,
        "Digit5" => 0x22,
        "Digit6" => 0x23,
        "Digit7" => 0x24,
        "Digit8" => 0x25,
        "Digit9" => 0x26,
        "Digit0" => 0x27,
        "Enter" => 0x28,
        "Escape" => 0x29,
        "Backspace" => 0x2a,
        "Tab" => 0x2b,
        "Space" => 0x2c,
        "Minus" => 0x2d,
        "Equal" => 0x2e,
        "BracketLeft" => 0x2f,
        "BracketRight" => 0x30,
        "Backslash" => 0x31,
        "Semicolon" => 0x33,
        "Quote" => 0x34,
        "Backquote" => 0x35,
        "Comma" => 0x36,
        "Period" => 0x37,
        "Slash" => 0x38,
        "CapsLock" => 0x39,
        "F1" => 0x3a,
        "F2" => 0x3b,
        "F3" => 0x3c,
        "F4" => 0x3d,
        "F5" => 0x3e,
        "F6" => 0x3f,
        "F7" => 0x40,
        "F8" => 0x41,
        "F9" => 0x42,
        "F10" => 0x43,
        "F11" => 0x44,
        "F12" => 0x45,
        "PrintScreen" => 0x46,
        "ScrollLock" => 0x47,
        "Pause" => 0x48,
        "Insert" => 0x49,
        "Home" => 0x4a,
        "PageUp" => 0x4b,
        "Delete" => 0x4c,
        "End" => 0x4d,
        "PageDown" => 0x4e,
        "ArrowRight" => 0x4f,
        "ArrowLeft" => 0x50,
        "ArrowDown" => 0x51,
        "ArrowUp" => 0x52,
        "NumLock" => 0x53,
        "NumpadDivide" => 0x54,
        "NumpadMultiply" => 0x55,
        "NumpadSubtract" => 0x56,
        "NumpadAdd" => 0x57,
        "NumpadEnter" => 0x58,
        "Numpad1" => 0x59,
        "Numpad2" => 0x5a,
        "Numpad3" => 0x5b,
        "Numpad4" => 0x5c,
        "Numpad5" => 0x5d,
        "Numpad6" => 0x5e,
        "Numpad7" => 0x5f,
        "Numpad8" => 0x60,
        "Numpad9" => 0x61,
        "Numpad0" => 0x62,
        "NumpadDecimal" => 0x63,
        "NumpadEqual" => 0x67,
        "IntlBackslash" => 0x64,
        "ContextMenu" => 0x65,
        "F13" => 0x68,
        "F14" => 0x69,
        "F15" => 0x6a,
        "F16" => 0x6b,
        "F17" => 0x6c,
        "F18" => 0x6d,
        "F19" => 0x6e,
        "F20" => 0x6f,
        "F21" => 0x70,
        "F22" => 0x71,
        "F23" => 0x72,
        "F24" => 0x73,
        _ => return None,
    };
    Some(usage)
}

pub const MOD_LEFT_CTRL: u8 = 1 << 0;
pub const MOD_LEFT_SHIFT: u8 = 1 << 1;
pub const MOD_LEFT_ALT: u8 = 1 << 2;
pub const MOD_LEFT_META: u8 = 1 << 3;
pub const MOD_RIGHT_CTRL: u8 = 1 << 4;
pub const MOD_RIGHT_SHIFT: u8 = 1 << 5;
pub const MOD_RIGHT_ALT: u8 = 1 << 6;
pub const MOD_RIGHT_META: u8 = 1 << 7;

const KEY_ENTER: u8 = 0x28;
const KEY_TAB: u8 = 0x2b;
const KEY_SPACE: u8 = 0x2c;

/// Maps a character to the (modifier, usage) pair that types it on a US
/// keyboard layout.
pub fn char_to_keystroke(c: char) -> Option<(u8, u8)> {
    let unshifted = |usage| Some((0, usage));
    let shifted = |usage| Some((MOD_LEFT_SHIFT, usage));
    match c {
        'a'..='z' => unshifted(0x04 + (c as u8 - b'a')),
        'A'..='Z' => shifted(0x04 + (c as u8 - b'A')),
        '1'..='9' => unshifted(0x1e + (c as u8 - b'1')),
        '0' => unshifted(0x27),
        '!' => shifted(0x1e),
        '@' => shifted(0x1f),
        '#' => shifted(0x20),
        '$' => shifted(0x21),
        '%' => shifted(0x22),
        '^' => shifted(0x23),
        '&' => shifted(0x24),
        '*' => shifted(0x25),
        '(' => shifted(0x26),
        ')' => shifted(0x27),
        '\n' => unshifted(KEY_ENTER),
        '\t' => unshifted(KEY_TAB),
        ' ' => unshifted(KEY_SPACE),
        '-' => unshifted(0x2d),
        '_' => shifted(0x2d),
        '=' => unshifted(0x2e),
        '+' => shifted(0x2e),
        '[' => unshifted(0x2f),
        '{' => shifted(0x2f),
        ']' => unshifted(0x30),
        '}' => shifted(0x30),
        '\\' => unshifted(0x31),
        '|' => shifted(0x31),
        ';' => unshifted(0x33),
        ':' => shifted(0x33),
        '\'' => unshifted(0x34),
        '"' => shifted(0x34),
        '`' => unshifted(0x35),
        '~' => shifted(0x35),
        ',' => unshifted(0x36),
        '<' => shifted(0x36),
        '.' => unshifted(0x37),
        '>' => shifted(0x37),
        '/' => unshifted(0x38),
        '?' => shifted(0x38),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Modifier(u8),
    Usage(u8),
}

/// Resolves a key name: a DOM `KeyboardEvent.code` (`KeyA`, `ControlLeft`),
/// a common alias (`ctrl`, `win`, `esc`, `pgdn`, `f5`), or a single
/// printable character. Aliases are case-insensitive.
pub fn resolve_key(name: &str) -> Option<Key> {
    if let Some(bit) = code_to_modifier(name) {
        return Some(Key::Modifier(bit));
    }
    if let Some(usage) = code_to_usage(name) {
        return Some(Key::Usage(usage));
    }
    let lower = name.to_ascii_lowercase();
    let modifier = match lower.as_str() {
        "ctrl" | "control" | "lctrl" => Some(MOD_LEFT_CTRL),
        "shift" | "lshift" => Some(MOD_LEFT_SHIFT),
        "alt" | "option" | "lalt" => Some(MOD_LEFT_ALT),
        "meta" | "win" | "windows" | "super" | "cmd" | "command" | "gui" | "lmeta" => {
            Some(MOD_LEFT_META)
        }
        "rctrl" => Some(MOD_RIGHT_CTRL),
        "rshift" => Some(MOD_RIGHT_SHIFT),
        "ralt" | "altgr" => Some(MOD_RIGHT_ALT),
        "rmeta" | "rwin" | "rcmd" => Some(MOD_RIGHT_META),
        _ => None,
    };
    if let Some(bit) = modifier {
        return Some(Key::Modifier(bit));
    }
    let code = match lower.as_str() {
        "enter" | "return" => "Enter",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "space" | "spacebar" => "Space",
        "backspace" | "bksp" => "Backspace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" => "PageUp",
        "pagedown" | "pgdn" => "PageDown",
        "up" | "arrowup" => "ArrowUp",
        "down" | "arrowdown" => "ArrowDown",
        "left" | "arrowleft" => "ArrowLeft",
        "right" | "arrowright" => "ArrowRight",
        "capslock" | "caps" => "CapsLock",
        "printscreen" | "prtsc" | "prtscr" => "PrintScreen",
        "scrolllock" => "ScrollLock",
        "pause" | "break" => "Pause",
        "numlock" => "NumLock",
        "menu" | "contextmenu" | "apps" => "ContextMenu",
        _ => "",
    };
    if let Some(usage) = code_to_usage(code) {
        return Some(Key::Usage(usage));
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
        && (1..=24).contains(&n)
    {
        return code_to_usage(&format!("F{n}")).map(Key::Usage);
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            char_to_keystroke(c.to_ascii_lowercase()).map(|(_, usage)| Key::Usage(usage))
        }
        _ => None,
    }
}

pub fn parse_combo(combo: &str) -> Result<(u8, [u8; HID_KEY_BUFFER_SIZE])> {
    let mut modifier = 0u8;
    let mut keys = [0u8; HID_KEY_BUFFER_SIZE];
    let mut count = 0;
    for token in combo.split('+').map(str::trim) {
        if token.is_empty() {
            bail!("empty key in combo {combo:?}");
        }
        match resolve_key(token) {
            Some(Key::Modifier(bit)) => modifier |= bit,
            Some(Key::Usage(usage)) => {
                if keys[..count].contains(&usage) {
                    continue;
                }
                if count == HID_KEY_BUFFER_SIZE {
                    bail!("combo {combo:?} has more than {HID_KEY_BUFFER_SIZE} non-modifier keys");
                }
                keys[count] = usage;
                count += 1;
            }
            None => bail!("unknown key {token:?} in combo {combo:?}"),
        }
    }
    Ok((modifier, keys))
}

fn press_and_release(
    modifier: u8,
    keys: [u8; HID_KEY_BUFFER_SIZE],
    hold_ms: u16,
    gap_ms: u16,
) -> [KeyboardMacroStep; 2] {
    [
        KeyboardMacroStep { modifier, keys, delay: hold_ms },
        KeyboardMacroStep { modifier: 0, keys: [0; HID_KEY_BUFFER_SIZE], delay: gap_ms },
    ]
}

pub fn combo_to_macro_steps(combo: &str, hold_ms: u16) -> Result<Vec<KeyboardMacroStep>> {
    let (modifier, keys) = parse_combo(combo)?;
    Ok(press_and_release(modifier, keys, hold_ms, 0).to_vec())
}

pub fn text_to_macro_steps(text: &str, delay_ms: u16) -> Result<Vec<KeyboardMacroStep>> {
    let mut steps = Vec::with_capacity(text.len() * 2);
    for (index, c) in text.chars().enumerate() {
        if c == '\r' {
            continue;
        }
        let (modifier, usage) = char_to_keystroke(c)
            .ok_or_else(|| anyhow!("unsupported character {c:?} at index {index}"))?;
        let mut keys = [0u8; HID_KEY_BUFFER_SIZE];
        keys[0] = usage;
        steps.extend(press_and_release(modifier, keys, delay_ms, delay_ms));
    }
    Ok(steps)
}
