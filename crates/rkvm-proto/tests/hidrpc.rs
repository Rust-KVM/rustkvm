use rkvm_proto::hidrpc::{
    HID_KEY_BUFFER_SIZE, Message, MessageType, VERSION, new_handshake_message,
    new_keyboard_led_message, new_keyboard_macro_state_message, new_keyboard_report_message,
    new_keydown_state_message, new_mouse_report_message, new_pointer_report_message,
};

const ALL_TYPES: [MessageType; 12] = [
    MessageType::Handshake,
    MessageType::KeyboardReport,
    MessageType::PointerReport,
    MessageType::WheelReport,
    MessageType::KeypressReport,
    MessageType::MouseReport,
    MessageType::KeyboardMacroReport,
    MessageType::CancelKeyboardMacroReport,
    MessageType::KeypressKeepAliveReport,
    MessageType::KeyboardLedState,
    MessageType::KeydownState,
    MessageType::KeyboardMacroState,
];

fn roundtrip(msg: &Message) -> Message {
    Message::unmarshal(&msg.marshal()).expect("marshalled message must unmarshal")
}

#[test]
fn every_message_type_byte_roundtrips() {
    for ty in ALL_TYPES {
        assert_eq!(MessageType::from_byte(ty as u8).unwrap(), ty);
    }
}

#[test]
fn wire_bytes_match_upstream_protocol() {
    assert_eq!(VERSION, 0x01);
    let wire: Vec<u8> = ALL_TYPES.iter().map(|t| *t as u8).collect();
    assert_eq!(wire, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x32, 0x33, 0x34]);
}

#[test]
fn unknown_type_byte_is_rejected() {
    for b in [0x00, 0x0a, 0x31, 0x35, 0xff] {
        assert!(MessageType::from_byte(b).is_err(), "0x{b:02x} must be rejected");
        assert!(Message::unmarshal(&[b, 1, 2]).is_err());
    }
}

#[test]
fn empty_buffer_is_rejected() {
    assert!(Message::unmarshal(&[]).is_err());
}

#[test]
fn type_only_frame_has_empty_payload() {
    let msg = Message::unmarshal(&[MessageType::CancelKeyboardMacroReport as u8]).unwrap();
    assert_eq!(msg.r#type, MessageType::CancelKeyboardMacroReport);
    assert!(msg.data.is_empty());
}

#[test]
fn handshake_carries_protocol_version() {
    assert_eq!(new_handshake_message().marshal(), [0x01, VERSION]);
}

#[test]
fn keyboard_report_roundtrips() {
    let msg = roundtrip(&new_keyboard_report_message(0x02, &[0x04, 0x05, 0, 0, 0, 0]));
    let report = msg.keyboard_report().unwrap();
    assert_eq!(report.modifier, 0x02);
    assert_eq!(report.keys, [0x04, 0x05, 0, 0, 0, 0]);
}

#[test]
fn keyboard_report_requires_modifier_byte() {
    let msg = Message::new(MessageType::KeyboardReport, vec![]);
    assert!(msg.keyboard_report().is_err());
}

#[test]
fn pointer_report_roundtrips_signed_big_endian() {
    for (x, y) in [(0, 0), (32767, 32767), (-1, i32::MIN), (i32::MAX, 1234)] {
        let msg = roundtrip(&new_pointer_report_message(x, y, 0b101));
        let report = msg.pointer_report().unwrap();
        assert_eq!((report.x, report.y, report.button), (x, y, 0b101));
    }
    assert_eq!(new_pointer_report_message(1, 2, 3).data, [0, 0, 0, 1, 0, 0, 0, 2, 3]);
}

#[test]
fn pointer_report_requires_exact_length() {
    for len in [0, 8, 10] {
        let msg = Message::new(MessageType::PointerReport, vec![0; len]);
        assert!(msg.pointer_report().is_err(), "length {len} must be rejected");
    }
}

#[test]
fn mouse_report_roundtrips_signed_deltas() {
    for (dx, dy) in [(0, 0), (-128, 127), (5, -5)] {
        let msg = roundtrip(&new_mouse_report_message(dx, dy, 1));
        let report = msg.mouse_report().unwrap();
        assert_eq!((report.dx, report.dy, report.button), (dx, dy, 1));
    }
}

#[test]
fn mouse_report_rejects_short_payload() {
    assert!(Message::new(MessageType::MouseReport, vec![1, 2]).mouse_report().is_err());
}

#[test]
fn keypress_report_decodes_press_flag() {
    let down = Message::new(MessageType::KeypressReport, vec![0x04, 1]);
    let up = Message::new(MessageType::KeypressReport, vec![0x04, 0]);
    assert!(down.keypress_report().unwrap().press);
    assert!(!up.keypress_report().unwrap().press);
    assert!(Message::new(MessageType::KeypressReport, vec![0x04]).keypress_report().is_err());
}

#[test]
fn accessor_rejects_wrong_message_type() {
    let msg = new_mouse_report_message(1, 1, 0);
    assert!(msg.keyboard_report().is_err());
    assert!(msg.pointer_report().is_err());
    assert!(msg.keypress_report().is_err());
    assert!(msg.keyboard_macro_report().is_err());
}

#[test]
fn device_to_client_messages_encode_payloads() {
    assert_eq!(new_keyboard_led_message(0b11).marshal(), [0x32, 0b11]);
    assert_eq!(new_keydown_state_message(0x01, &[0x04]).marshal(), [0x33, 0x01, 0x04]);
    assert_eq!(new_keyboard_macro_state_message(true, false).marshal(), [0x34, 1, 0]);
}

fn macro_payload(is_paste: bool, steps: &[(u8, [u8; HID_KEY_BUFFER_SIZE], u16)]) -> Vec<u8> {
    let mut data = vec![u8::from(is_paste)];
    data.extend_from_slice(&(steps.len() as u32).to_be_bytes());
    for (modifier, keys, delay) in steps {
        data.push(*modifier);
        data.extend_from_slice(keys);
        data.extend_from_slice(&delay.to_be_bytes());
    }
    data
}

#[test]
fn keyboard_macro_report_decodes_steps() {
    let steps = [(0x02, [0x04, 0, 0, 0, 0, 0], 20), (0, [0; HID_KEY_BUFFER_SIZE], 500)];
    let msg = Message::new(MessageType::KeyboardMacroReport, macro_payload(true, &steps));
    let report = msg.keyboard_macro_report().unwrap();
    assert!(report.is_paste);
    assert_eq!(report.step_count, 2);
    assert_eq!(report.steps.len(), 2);
    assert_eq!(report.steps[0].modifier, 0x02);
    assert_eq!(report.steps[0].keys, [0x04, 0, 0, 0, 0, 0]);
    assert_eq!(report.steps[0].delay, 20);
    assert_eq!(report.steps[1].delay, 500);
}

#[test]
fn keyboard_macro_report_rejects_inconsistent_length() {
    let mut data = macro_payload(false, &[(0, [0; HID_KEY_BUFFER_SIZE], 0)]);
    data.pop();
    let msg = Message::new(MessageType::KeyboardMacroReport, data);
    assert!(msg.keyboard_macro_report().is_err());

    let msg = Message::new(MessageType::KeyboardMacroReport, vec![0, 1, 2]);
    assert!(msg.keyboard_macro_report().is_err());
}

#[test]
fn keyboard_macro_report_rejects_huge_step_count_without_allocating() {
    let mut data = vec![0u8];
    data.extend_from_slice(&u32::MAX.to_be_bytes());
    let msg = Message::new(MessageType::KeyboardMacroReport, data);
    assert!(msg.keyboard_macro_report().is_err());
}
