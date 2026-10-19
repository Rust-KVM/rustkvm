use rustkvm::hardware::v4l2::{
    CaptureFormat, CaptureSignal, DvTimings, SourceObservation, SourceWatch, parse_bt_timings,
    parse_pix_format,
};

const NV12: u32 = u32::from_le_bytes(*b"NV12");
const NV24: u32 = u32::from_le_bytes(*b"NV24");

fn put(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn signal(fourcc: u32, width: u32, height: u32) -> CaptureSignal {
    CaptureSignal {
        format: Some(CaptureFormat {
            width,
            height,
            fourcc,
            bytesperline: width,
            sizeimage: width * height * 3 / 2,
        }),
        timings: Some(DvTimings { width, height, interlaced: false, pixelclock: 594_000_000 }),
    }
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

#[test]
fn source_change_needs_two_matching_polls() {
    let mut watch = SourceWatch::default();
    let before = signal(NV12, 3840, 2160);
    let after = signal(NV24, 3840, 2160);
    assert_eq!(watch.observe(before), SourceObservation::Baseline);
    assert_eq!(watch.observe(before), SourceObservation::Steady);
    assert_eq!(watch.observe(after), SourceObservation::Pending);
    assert_eq!(watch.observe(after), SourceObservation::Changed { from: before });
    // The next reading after a change becomes the new reference.
    assert_eq!(watch.observe(after), SourceObservation::Baseline);
}

#[test]
fn a_single_odd_reading_is_not_a_change() {
    let mut watch = SourceWatch::default();
    let normal = signal(NV12, 3840, 2160);
    watch.observe(normal);
    assert_eq!(watch.observe(signal(NV12, 1920, 1080)), SourceObservation::Pending);
    assert_eq!(watch.observe(normal), SourceObservation::Steady);
    assert_eq!(watch.observe(signal(NV12, 1920, 1080)), SourceObservation::Pending);
}

#[test]
fn signal_loss_alone_does_not_trigger_but_a_different_return_does() {
    let mut watch = SourceWatch::default();
    let before = signal(NV12, 3840, 2160);
    let lost = CaptureSignal { format: before.format, timings: None };
    watch.observe(before);
    assert_eq!(watch.observe(lost), SourceObservation::NoSignal);
    assert_eq!(watch.observe(lost), SourceObservation::NoSignal);
    assert_eq!(watch.observe(before), SourceObservation::Steady);

    let after = signal(NV24, 3840, 2160);
    assert_eq!(watch.observe(lost), SourceObservation::NoSignal);
    assert_eq!(watch.observe(after), SourceObservation::Pending);
    assert_eq!(watch.observe(after), SourceObservation::Changed { from: before });
}

#[test]
fn pixel_clock_wobble_is_not_a_change() {
    let mut watch = SourceWatch::default();
    let mut a = signal(NV12, 3840, 2160);
    let mut b = a;
    if let (Some(ta), Some(tb)) = (a.timings.as_mut(), b.timings.as_mut()) {
        ta.pixelclock = 593_950_000;
        tb.pixelclock = 593_960_000;
    }
    assert_eq!(watch.observe(a), SourceObservation::Baseline);
    for _ in 0..4 {
        assert_eq!(watch.observe(b), SourceObservation::Steady);
        assert_eq!(watch.observe(a), SourceObservation::Steady);
    }
}

#[test]
fn a_one_off_bogus_mode_is_not_a_change() {
    let mut watch = SourceWatch::default();
    let normal = signal(NV12, 3840, 2160);
    let mut bogus = normal;
    if let Some(t) = bogus.timings.as_mut() {
        t.height = 547;
    }
    watch.observe(normal);
    assert_eq!(watch.observe(bogus), SourceObservation::Pending);
    assert_eq!(watch.observe(normal), SourceObservation::Steady);
}
