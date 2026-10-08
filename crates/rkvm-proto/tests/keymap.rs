use rkvm_proto::hidrpc::HID_KEY_BUFFER_SIZE;
use rkvm_proto::keymap::{
    Key, MOD_LEFT_ALT, MOD_LEFT_CTRL, MOD_LEFT_META, MOD_LEFT_SHIFT, MOD_RIGHT_ALT,
    char_to_keystroke, code_to_modifier, code_to_usage, combo_to_macro_steps, parse_combo,
    resolve_key, text_to_macro_steps,
};

const NO_KEYS: [u8; HID_KEY_BUFFER_SIZE] = [0; HID_KEY_BUFFER_SIZE];

fn keys(first: &[u8]) -> [u8; HID_KEY_BUFFER_SIZE] {
    let mut out = NO_KEYS;
    out[..first.len()].copy_from_slice(first);
    out
}

#[test]
fn dom_codes_map_to_hid_usages() {
    assert_eq!(code_to_usage("KeyA"), Some(0x04));
    assert_eq!(code_to_usage("KeyZ"), Some(0x1d));
    assert_eq!(code_to_usage("Digit0"), Some(0x27));
    assert_eq!(code_to_usage("Enter"), Some(0x28));
    assert_eq!(code_to_usage("F24"), Some(0x73));
    assert_eq!(code_to_usage("ShiftLeft"), None);
    assert_eq!(code_to_modifier("ShiftLeft"), Some(MOD_LEFT_SHIFT));
    assert_eq!(code_to_modifier("MetaRight"), Some(1 << 7));
}

#[test]
fn every_printable_ascii_character_is_typeable() {
    for c in (0x20u8..0x7f).map(char::from) {
        assert!(char_to_keystroke(c).is_some(), "{c:?} has no keystroke");
    }
}

#[test]
fn shifted_characters_use_shift_on_the_base_key() {
    assert_eq!(char_to_keystroke('a'), Some((0, 0x04)));
    assert_eq!(char_to_keystroke('A'), Some((MOD_LEFT_SHIFT, 0x04)));
    assert_eq!(char_to_keystroke('1'), Some((0, 0x1e)));
    assert_eq!(char_to_keystroke('!'), Some((MOD_LEFT_SHIFT, 0x1e)));
    assert_eq!(char_to_keystroke('?'), Some((MOD_LEFT_SHIFT, 0x38)));
    assert_eq!(char_to_keystroke('\n'), Some((0, 0x28)));
    assert_eq!(char_to_keystroke('é'), None);
}

#[test]
fn key_names_accept_codes_aliases_and_characters() {
    assert_eq!(resolve_key("ControlLeft"), Some(Key::Modifier(MOD_LEFT_CTRL)));
    assert_eq!(resolve_key("CTRL"), Some(Key::Modifier(MOD_LEFT_CTRL)));
    assert_eq!(resolve_key("win"), Some(Key::Modifier(MOD_LEFT_META)));
    assert_eq!(resolve_key("AltGr"), Some(Key::Modifier(MOD_RIGHT_ALT)));
    assert_eq!(resolve_key("Delete"), Some(Key::Usage(0x4c)));
    assert_eq!(resolve_key("del"), Some(Key::Usage(0x4c)));
    assert_eq!(resolve_key("esc"), Some(Key::Usage(0x29)));
    assert_eq!(resolve_key("f5"), Some(Key::Usage(0x3e)));
    assert_eq!(resolve_key("F12"), Some(Key::Usage(0x45)));
    assert_eq!(resolve_key("c"), Some(Key::Usage(0x06)));
    assert_eq!(resolve_key("C"), Some(Key::Usage(0x06)));
    assert_eq!(resolve_key("/"), Some(Key::Usage(0x38)));
    assert_eq!(resolve_key("f25"), None);
    assert_eq!(resolve_key("nope"), None);
}

#[test]
fn combo_collects_modifiers_and_keys() {
    assert_eq!(
        parse_combo("Ctrl+Alt+Delete").unwrap(),
        (MOD_LEFT_CTRL | MOD_LEFT_ALT, keys(&[0x4c]))
    );
    assert_eq!(parse_combo(" win + r ").unwrap(), (MOD_LEFT_META, keys(&[0x15])));
    assert_eq!(parse_combo("shift").unwrap(), (MOD_LEFT_SHIFT, NO_KEYS));
    assert_eq!(parse_combo("a+a").unwrap(), (0, keys(&[0x04])));
}

#[test]
fn combo_rejects_bad_input() {
    assert!(parse_combo("").is_err());
    assert!(parse_combo("ctrl++").is_err());
    assert!(parse_combo("ctrl+banana").is_err());
    assert!(parse_combo("a+b+c+d+e+f+g").is_err());
    assert!(parse_combo("a+b+c+d+e+f").is_ok());
}

#[test]
fn combo_steps_press_then_release() {
    let steps = combo_to_macro_steps("ctrl+c", 80).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(
        (steps[0].modifier, steps[0].keys, steps[0].delay),
        (MOD_LEFT_CTRL, keys(&[0x06]), 80)
    );
    assert_eq!((steps[1].modifier, steps[1].keys), (0, NO_KEYS));
}

#[test]
fn text_steps_release_between_every_character() {
    let steps = text_to_macro_steps("Hi\r\n", 15).unwrap();
    assert_eq!(steps.len(), 6, "\\r is skipped, three characters remain");
    assert_eq!((steps[0].modifier, steps[0].keys), (MOD_LEFT_SHIFT, keys(&[0x0b])));
    assert_eq!((steps[2].modifier, steps[2].keys), (0, keys(&[0x0c])));
    assert_eq!((steps[4].modifier, steps[4].keys), (0, keys(&[0x28])));
    for release in steps.iter().skip(1).step_by(2) {
        assert_eq!((release.modifier, release.keys), (0, NO_KEYS));
    }
    assert!(steps.iter().all(|s| s.delay == 15));
}

#[test]
fn text_steps_name_the_unsupported_character() {
    let err = text_to_macro_steps("ok ✓", 10).unwrap_err().to_string();
    assert!(err.contains('✓') && err.contains("index 3"), "{err}");
    assert!(text_to_macro_steps("", 10).unwrap().is_empty());
}
