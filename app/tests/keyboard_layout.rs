use rustkvm::api::handlers::hid::normalize_keyboard_layout;

#[test]
fn short_codes_pass_through() {
    for code in ["us", "uk", "de", "fr", "es", "it", "jp", "kr"] {
        assert_eq!(normalize_keyboard_layout(code), code);
    }
}

#[test]
fn legacy_locale_values_map_to_short_codes() {
    assert_eq!(normalize_keyboard_layout("en_US"), "us");
    assert_eq!(normalize_keyboard_layout("en_UK"), "uk");
    assert_eq!(normalize_keyboard_layout("en_GB"), "uk");
    assert_eq!(normalize_keyboard_layout("de_DE"), "de");
    assert_eq!(normalize_keyboard_layout("ja_JP"), "jp");
    assert_eq!(normalize_keyboard_layout("ko_KR"), "kr");
}

#[test]
fn unknown_values_fall_back_to_us() {
    assert_eq!(normalize_keyboard_layout(""), "us");
    assert_eq!(normalize_keyboard_layout("dvorak"), "us");
}
