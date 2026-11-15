use rustkvm::terminal::parse_resize;

#[test]
fn json_object_is_a_resize() {
    let size = parse_resize(br#"{"cols":120,"rows":40}"#).unwrap();
    assert_eq!((size.cols, size.rows), (120, 40));
    assert!(parse_resize(b"  {\"rows\":24,\"cols\":80}\n").is_some());
}

#[test]
fn typed_input_is_not_a_resize() {
    assert!(parse_resize(b"ls -la\r").is_none());
    assert!(parse_resize(b"{").is_none());
    assert!(parse_resize(b"{}").is_none());
    assert!(parse_resize(br#"{"cols":"wide","rows":40}"#).is_none());
    assert!(parse_resize(&[0xff, 0xfe]).is_none());
}
