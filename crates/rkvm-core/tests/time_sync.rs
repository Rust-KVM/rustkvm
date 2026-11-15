use chrono::TimeDelta;
use rkvm_core::time_sync::{STEP_THRESHOLD, needs_step};

#[test]
fn clock_within_http_date_resolution_is_left_alone() {
    assert!(!needs_step(TimeDelta::zero()));
    assert!(!needs_step(TimeDelta::milliseconds(900)));
    assert!(!needs_step(TimeDelta::milliseconds(-900)));
    assert!(!needs_step(STEP_THRESHOLD));
    assert!(!needs_step(-STEP_THRESHOLD));
}

#[test]
fn clock_that_is_clearly_wrong_is_stepped() {
    assert!(needs_step(STEP_THRESHOLD + TimeDelta::milliseconds(1)));
    assert!(needs_step(TimeDelta::seconds(-10)));
    // No RTC battery: the clock starts in 2021.
    assert!(needs_step(TimeDelta::days(5 * 365)));
}
