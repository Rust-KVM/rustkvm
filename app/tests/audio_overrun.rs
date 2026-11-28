use std::time::{Duration, Instant};

use rustkvm::pipeline::OverrunWindow;

#[test]
fn a_burst_within_the_window_asks_for_a_restart() {
    let mut window = OverrunWindow::default();
    let t0 = Instant::now();
    for i in 0..OverrunWindow::LIMIT {
        assert!(!window.record(t0 + Duration::from_millis(u64::from(i) * 10)));
    }
    assert!(window.record(t0 + Duration::from_millis(500)));
}

#[test]
fn scattered_warnings_never_add_up() {
    let mut window = OverrunWindow::default();
    let t0 = Instant::now();
    for i in 0..100u32 {
        let at = t0 + (OverrunWindow::WINDOW + Duration::from_millis(1)) * i;
        assert!(!window.record(at));
    }
}
