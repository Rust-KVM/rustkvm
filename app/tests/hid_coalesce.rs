use rustkvm::hardware::usb::hid::{HidReport, coalesce_reports};

fn abs(buttons: u8, x: u16, y: u16) -> HidReport {
    let [x0, x1] = x.to_le_bytes();
    let [y0, y1] = y.to_le_bytes();
    HidReport::AbsMouse([1, buttons, x0, x1, y0, y1])
}

fn rel(buttons: u8, dx: i8, dy: i8) -> HidReport {
    HidReport::RelMouse([buttons, dx as u8, dy as u8, 0])
}

fn key(code: u8) -> HidReport {
    HidReport::Keyboard([0, 0, code, 0, 0, 0, 0, 0])
}

#[test]
fn a_backlog_of_moves_becomes_one_jump() {
    let mut buttons = Some(0);
    let mut batch: Vec<_> = (0..64).map(|i| abs(0, i * 10, i * 5)).collect();
    coalesce_reports(&mut batch, &mut buttons);
    assert_eq!(batch, vec![abs(0, 630, 315)]);
}

#[test]
fn button_changes_and_keys_keep_their_order() {
    let mut buttons = Some(0);
    let mut batch = vec![
        abs(0, 1, 1),
        abs(0, 2, 2),
        key(4),
        abs(1, 2, 2), // press
        abs(1, 3, 3),
        abs(1, 4, 4), // drag
        abs(0, 4, 4), // release
        key(0),
    ];
    coalesce_reports(&mut batch, &mut buttons);
    // The press stays at (2, 2); the drag moves to (4, 4) before the release.
    assert_eq!(batch, vec![abs(0, 2, 2), key(4), abs(1, 2, 2), abs(1, 4, 4), abs(0, 4, 4), key(0)]);
    assert_eq!(buttons, Some(0));
}

#[test]
fn a_wheel_step_splits_moves() {
    let mut buttons = Some(0);
    let mut batch = vec![abs(0, 1, 1), HidReport::AbsWheel([2, 1]), abs(0, 2, 2), abs(0, 3, 3)];
    coalesce_reports(&mut batch, &mut buttons);
    assert_eq!(batch, vec![abs(0, 1, 1), HidReport::AbsWheel([2, 1]), abs(0, 3, 3)]);
}

#[test]
fn relative_moves_are_summed_within_range() {
    let mut batch = vec![rel(0, 10, -5), rel(0, 20, -5), rel(0, 100, 0), rel(1, 1, 1)];
    coalesce_reports(&mut batch, &mut None);
    assert_eq!(batch, vec![rel(0, 30, -10), rel(0, 100, 0), rel(1, 1, 1)]);
}

#[test]
fn the_first_report_after_a_button_change_is_kept() {
    let mut buttons = Some(1);
    let mut batch = vec![abs(0, 5, 5), abs(0, 6, 6), abs(0, 7, 7)];
    coalesce_reports(&mut batch, &mut buttons);
    assert_eq!(batch, vec![abs(0, 5, 5), abs(0, 7, 7)]);
}
