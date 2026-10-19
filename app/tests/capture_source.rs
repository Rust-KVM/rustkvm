use std::time::{Duration, Instant};

use rustkvm::hardware::v4l2::{
    DvTimings, RelockVerdict, RelockWatch, parse_bt_timings, parse_pix_format,
};

const NV12: u32 = u32::from_le_bytes(*b"NV12");

fn put(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

#[test]
fn parses_mplane_and_single_plane_formats() {
    let mut fmt = [0u8; 200];
    put(&mut fmt, 0, 3840);
    put(&mut fmt, 4, 2160);
    put(&mut fmt, 8, NV12);
    put(&mut fmt, 20, 12_441_600);
    put(&mut fmt, 24, 3840);
    let mp = parse_pix_format(true, &fmt);
    assert_eq!(
        (mp.width, mp.height, mp.bytesperline, mp.sizeimage),
        (3840, 2160, 3840, 12_441_600)
    );
    assert_eq!(mp.fourcc_str(), "NV12");

    let mut fmt = [0u8; 200];
    put(&mut fmt, 0, 1920);
    put(&mut fmt, 4, 1080);
    put(&mut fmt, 8, NV12);
    put(&mut fmt, 16, 1920);
    put(&mut fmt, 20, 3_110_400);
    let sp = parse_pix_format(false, &fmt);
    assert_eq!((sp.width, sp.bytesperline, sp.sizeimage), (1920, 1920, 3_110_400));
}

#[test]
fn parses_bt_timings() {
    let mut data = [0u8; 128];
    put(&mut data, 0, 3840);
    put(&mut data, 4, 2160);
    data[16..24].copy_from_slice(&594_000_000u64.to_ne_bytes());
    let t = parse_bt_timings(&data);
    assert_eq!(
        t,
        DvTimings { width: 3840, height: 2160, interlaced: false, pixelclock: 594_000_000 }
    );
}

/// Feeds one event per second, like the relock loop seen on the T6, and
/// returns the verdict of each second.
fn storm(watch: &mut RelockWatch, start: Instant, secs: u64) -> Vec<RelockVerdict> {
    (0..secs).map(|s| watch.observe(start + Duration::from_secs(s), 1)).collect()
}

#[test]
fn a_replug_or_mode_change_is_not_a_storm() {
    let mut watch = RelockWatch::default();
    let t = Instant::now();
    for s in 0..3 {
        assert_eq!(watch.observe(t + Duration::from_secs(s), 1), RelockVerdict::Quiet);
    }
    for s in 3..60 {
        assert_eq!(watch.observe(t + Duration::from_secs(s), 0), RelockVerdict::Quiet);
    }
}

#[test]
fn a_relock_loop_triggers_a_replug_then_backs_off() {
    let mut watch = RelockWatch::default();
    let t = Instant::now();
    let verdicts = storm(&mut watch, t, 120);
    let replugs: Vec<_> = verdicts
        .iter()
        .enumerate()
        .filter_map(|(s, v)| match v {
            RelockVerdict::Replug { attempt, .. } => Some((s, *attempt)),
            _ => None,
        })
        .collect();
    // First replug once the window holds THRESHOLD events, the next ones
    // 30 s and then 60 s later.
    assert_eq!(replugs, vec![(5, 1), (35, 2), (95, 3)]);
    assert!(matches!(verdicts[20], RelockVerdict::Storm { .. }));
}

#[test]
fn backoff_resets_after_the_input_settles() {
    let mut watch = RelockWatch::default();
    let t = Instant::now();
    storm(&mut watch, t, 6);
    let later = t + Duration::from_secs(6) + RelockWatch::SETTLE;
    assert_eq!(watch.observe(later, 0), RelockVerdict::Quiet);
    let verdicts = storm(&mut watch, later + Duration::from_secs(1), 6);
    assert_eq!(verdicts[5], RelockVerdict::Replug { events: 6, attempt: 1 });
}
