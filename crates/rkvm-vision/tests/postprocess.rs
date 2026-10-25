use rkvm_vision::det::{
    DbParams, TextBox, choose_shape, db_boxes, merge_boxes, plan_tiles, reading_order,
};
use rkvm_vision::image::{Nv12Frame, RectF, RgbImage, blank_columns, sample_bgr};
use rkvm_vision::rec::{
    Segment, ctc_decode, ctc_decode_f16, f16_to_f32, load_dict, pick_width, split_line,
};
use rkvm_vision::signature::{fingerprint, grid_changed, luma_grid};
use rkvm_vision::{OcrOptions, Rect};

fn tb(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
    TextBox { x0, y0, x1, y1, score: 0.9 }
}

#[test]
fn db_finds_one_box_per_blob_and_unclips_it() {
    let (w, h) = (40, 20);
    let mut prob = vec![0.0f32; w * h];
    for y in 5..9 {
        for x in 2..12 {
            prob[y * w + x] = 0.9;
        }
        for x in 20..36 {
            prob[y * w + x] = 0.8;
        }
    }
    prob[15 * w + 30] = 0.95;
    let boxes = db_boxes(&prob, w, h, &DbParams::default());
    assert_eq!(boxes.len(), 2, "the single noisy pixel is dropped: {boxes:?}");
    let first = boxes.iter().find(|b| b.x0 < 10.0).expect("left box");
    let d = 10.0 * 4.0 * 1.5 / (2.0 * 14.0);
    assert!((first.x0 - (2.0 - d)).abs() < 1e-4 && (first.x1 - (12.0 + d)).abs() < 1e-4);
    assert!((first.y0 - (5.0 - d)).abs() < 1e-4 && (first.y1 - (9.0 + d)).abs() < 1e-4);
    assert!((first.score - 0.9).abs() < 1e-4);
}

#[test]
fn db_drops_low_confidence_regions() {
    let (w, h) = (20, 10);
    let mut prob = vec![0.0f32; w * h];
    for y in 2..6 {
        for x in 2..15 {
            prob[y * w + x] = 0.4;
        }
    }
    assert!(db_boxes(&prob, w, h, &DbParams::default()).is_empty());
    assert!(
        db_boxes(&prob[..10], w, h, &DbParams::default()).is_empty(),
        "short maps are rejected"
    );
}

#[test]
fn tiles_cover_the_area_with_overlap() {
    assert_eq!(plan_tiles(1920, 1080, 1920, 1088, 96), vec![Rect::new(0, 0, 1920, 1080)]);
    let tiles = plan_tiles(3840, 2160, 1920, 1088, 96);
    assert_eq!(tiles.len(), 9);
    for t in &tiles {
        assert!(t.right() <= 3840 && t.bottom() <= 2160);
        assert_eq!((t.width, t.height), (1920, 1088));
    }
    let xs: Vec<u32> = tiles.iter().take(3).map(|t| t.x).collect();
    assert_eq!(xs, vec![0, 960, 1920]);
    for pair in xs.windows(2) {
        assert!(pair[0] + 1920 >= pair[1] + 96, "neighbours overlap by at least 96 px");
    }
    assert!(plan_tiles(0, 100, 64, 64, 8).is_empty());
}

#[test]
fn pieces_of_a_line_cut_by_tiles_are_merged() {
    let left = tb(100.0, 50.0, 1000.0, 80.0);
    let right = tb(960.0, 51.0, 1500.0, 79.0);
    let lower_half = tb(300.0, 66.0, 700.0, 80.0);
    let next_line = tb(100.0, 90.0, 900.0, 120.0);
    let merged = merge_boxes(vec![left, right, lower_half, next_line]);
    assert_eq!(merged.len(), 2, "{merged:?}");
    assert!(merged.contains(&tb(100.0, 50.0, 1500.0, 80.0)));
    assert!(merged.contains(&next_line));
}

#[test]
fn table_columns_with_a_gap_stay_separate() {
    let name = tb(100.0, 50.0, 300.0, 80.0);
    let value = tb(340.0, 50.0, 500.0, 80.0);
    assert_eq!(merge_boxes(vec![name, value]).len(), 2);
}

#[test]
fn reading_order_is_rows_then_columns() {
    let mut boxes = vec![
        tb(500.0, 12.0, 600.0, 40.0),
        tb(10.0, 100.0, 90.0, 130.0),
        tb(10.0, 10.0, 200.0, 38.0),
    ];
    reading_order(&mut boxes);
    let xs: Vec<(f32, f32)> = boxes.iter().map(|b| (b.x0, b.y0)).collect();
    assert_eq!(xs, vec![(10.0, 10.0), (500.0, 12.0), (10.0, 100.0)]);
}

#[test]
fn ctc_collapses_repeats_and_handles_space_class() {
    let dict = load_dict("a\nb\n\u{4e2d}\r\n");
    assert_eq!(dict, vec!["a", "b", "\u{4e2d}"]);
    let classes = dict.len() + 2;
    let one_hot = |c: usize| {
        let mut v = vec![0.01f32; classes];
        v[c] = 0.9;
        v
    };
    let steps = [1, 1, 0, 1, 4, 2, 3, 3, 0];
    let probs: Vec<f32> = steps.iter().flat_map(|&c| one_hot(c)).collect();
    let (text, score) = ctc_decode(&probs, steps.len(), classes, &dict);
    assert_eq!(text, "aa b\u{4e2d}");
    assert!((score - 0.9).abs() < 1e-5);
    assert_eq!(ctc_decode(&one_hot(0), 1, classes, &dict), (String::new(), 0.0));
}

#[test]
fn long_lines_split_at_blank_columns() {
    let mut blank = vec![false; 100];
    blank[30..34].fill(true);
    blank[70] = true;
    let segs = split_line(&blank, 40, 3);
    assert_eq!(
        segs,
        vec![
            Segment { start: 0, end: 30, space_before: false },
            Segment { start: 34, end: 70, space_before: true },
            Segment { start: 71, end: 100, space_before: false },
        ]
    );
}

#[test]
fn lines_without_gaps_are_cut_hard() {
    let segs = split_line(&[false; 25], 10, 2);
    let ranges: Vec<(usize, usize)> = segs.iter().map(|s| (s.start, s.end)).collect();
    assert_eq!(ranges, vec![(0, 10), (10, 20), (20, 25)]);
    assert!(split_line(&[true; 5], 10, 2).is_empty());
    let short = split_line(&[true, false, false, true], 10, 2);
    assert_eq!(short, vec![Segment { start: 1, end: 4, space_before: false }]);
}

#[test]
fn recognizer_width_is_the_narrowest_that_fits() {
    let widths = [320, 640, 1280];
    assert_eq!(pick_width(&widths, 100), Some(320));
    assert_eq!(pick_width(&widths, 320), Some(320));
    assert_eq!(pick_width(&widths, 321), Some(640));
    assert_eq!(pick_width(&widths, 5000), Some(1280));
    assert_eq!(pick_width(&[], 10), None);
}

#[test]
fn nv12_converts_with_row_padding() {
    let (w, h, stride) = (4usize, 2usize, 8usize);
    let mut y = vec![0u8; stride * h];
    for row in 0..h {
        y[row * stride..row * stride + w].copy_from_slice(&[16, 235, 126, 82]);
    }
    let uv = vec![128u8; stride];
    let frame = Nv12Frame::new(w as u32, h as u32, &y, stride, &uv, stride).expect("frame");
    let img = RgbImage::from_nv12(&frame);
    let px: Vec<&[u8]> = img.data().chunks(3).take(4).collect();
    assert_eq!(px[0], [0, 0, 0]);
    assert_eq!(px[1], [255, 255, 255]);
    assert!(px[2].iter().all(|&c| c.abs_diff(128) <= 1));
    assert!(Nv12Frame::new(3, 2, &y, stride, &uv, stride).is_err(), "odd width");
    assert!(Nv12Frame::new(4, 2, &y[..4], stride, &uv, stride).is_err(), "short plane");
}

#[test]
fn nv12_sampled_in_place_matches_converting_first() {
    let (w, h) = (16usize, 8usize);
    let y: Vec<u8> = (0..w * h).map(|i| (16 + (i * 37) % 220) as u8).collect();
    // Flat chroma: converting first takes the nearest chroma sample while
    // sampling in place interpolates it, so only the luma detail is compared.
    let uv: Vec<u8> = (0..w * h / 2).map(|i| if i % 2 == 0 { 100 } else { 150 }).collect();
    let frame = Nv12Frame::new(w as u32, h as u32, &y, w, &uv, w).expect("frame");
    let rgb = RgbImage::from_nv12(&frame);
    let src = RectF { x0: 0.0, y0: 0.0, x1: w as f32, y1: h as f32 };
    let (mut a, mut b) = (vec![0u8; w * h * 3], vec![0u8; w * h * 3]);
    sample_bgr(&frame, src, w, h, &mut a, w);
    sample_bgr(&rgb, src, w, h, &mut b, w);
    for (pa, pb) in a.chunks(3).zip(b.chunks(3)) {
        assert!(pa.iter().zip(pb).all(|(x, y)| x.abs_diff(*y) <= 2), "{pa:?} vs {pb:?}");
    }
}

#[test]
fn bilinear_sampling_writes_bgr_inside_the_patch_only() {
    let img = RgbImage::new(2, 1, vec![10, 20, 30, 50, 60, 70]).expect("image");
    let mut out = vec![7u8; 3 * 3];
    sample_bgr(&img, RectF { x0: 0.0, y0: 0.0, x1: 2.0, y1: 1.0 }, 1, 1, &mut out, 3);
    assert_eq!(&out[..3], &[50, 40, 30], "midpoint average, channels reversed");
    assert!(out[3..].iter().all(|&v| v == 7));
    assert!(RgbImage::new(2, 2, vec![0; 3]).is_err());
}

#[test]
fn large_patches_are_sampled_in_parallel_identically() {
    let (w, h) = (700usize, 400usize);
    let data: Vec<u8> = (0..w * h * 3).map(|i| (i % 251) as u8).collect();
    let img = RgbImage::new(w as u32, h as u32, data).expect("image");
    let src = RectF { x0: 3.5, y0: 1.25, x1: 650.0, y1: 390.0 };
    let mut big = vec![0u8; w * h * 3];
    sample_bgr(&img, src, w, h, &mut big, w);
    let mut row = vec![0u8; w * 3];
    for oy in [0usize, 133, 399] {
        let band = RectF {
            y0: src.y0 + (src.y1 - src.y0) * oy as f32 / h as f32,
            y1: src.y0 + (src.y1 - src.y0) * (oy + 1) as f32 / h as f32,
            ..src
        };
        sample_bgr(&img, band, w, 1, &mut row, w);
        let got = &big[oy * w * 3..(oy + 1) * w * 3];
        assert!(got.iter().zip(&row).all(|(a, b)| a.abs_diff(*b) <= 1), "row {oy}");
    }
}

#[test]
fn blank_columns_follow_ink() {
    let (w, h) = (6u32, 4u32);
    let mut data = vec![255u8; (w * h * 3) as usize];
    for y in 1..3 {
        for x in [1u32, 4] {
            let i = ((y * w + x) * 3) as usize;
            data[i..i + 3].copy_from_slice(&[0, 0, 0]);
        }
    }
    let img = RgbImage::new(w, h, data).expect("image");
    assert_eq!(
        blank_columns(&img, Rect::new(0, 0, w, h)),
        vec![true, false, true, true, false, true]
    );
    assert!(blank_columns(&img, Rect::new(10, 10, 2, 2)).is_empty());
}

#[test]
fn half_precision_decodes_like_f32() {
    for (bits, value) in [
        (0x3c00u16, 1.0f32),
        (0x3800, 0.5),
        (0x0000, 0.0),
        (0xc000, -2.0),
        (0x0001, 5.960_464_5e-8),
        (0x7bff, 65504.0),
    ] {
        assert_eq!(f16_to_f32(bits), value, "{bits:#06x}");
    }
    let dict: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    // blank, a, a, blank, b over 4 classes (blank, a, b, space)
    let probs = [
        [0.9f32, 0.05, 0.03, 0.02],
        [0.1, 0.8, 0.05, 0.05],
        [0.1, 0.7, 0.1, 0.1],
        [0.6, 0.2, 0.1, 0.1],
        [0.1, 0.1, 0.75, 0.05],
    ];
    let flat: Vec<f32> = probs.iter().flatten().copied().collect();
    let half: Vec<u16> = flat.iter().map(|&p| to_f16(p)).collect();
    let (text, score) = ctc_decode(&flat, 5, 4, &dict);
    let (text16, score16) = ctc_decode_f16(&half, 5, 4, &dict);
    assert_eq!(text, "ab");
    assert_eq!(text16, text);
    assert!((score - score16).abs() < 1e-3);
}

/// f32 to half for test inputs (normal values only).
fn to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if v == 0.0 || exp <= 0 {
        return 0;
    }
    ((exp as u16) << 10) | ((bits >> 13) & 0x3ff) as u16
}

#[test]
fn detector_shape_is_the_cheapest_cover() {
    let shapes = [(640, 224), (1920, 448), (1920, 1088)];
    // A 1080p screen: three 1920x448 strips, one round on three cores.
    assert_eq!(choose_shape(&shapes, 1920, 1080, 96, 3), Some(1));
    // A small region fits one small tile.
    assert_eq!(choose_shape(&shapes, 400, 150, 96, 3), Some(0));
    // A 1600x200 strip: three small tiles in one round beat one big one.
    assert_eq!(choose_shape(&shapes, 1600, 200, 96, 3), Some(0));
    assert_eq!(choose_shape(&[(64, 64)], 100, 100, 96, 3), None, "tile not larger than overlap");
}

#[test]
fn fingerprint_changes_with_one_pixel_only() {
    let (w, h, stride) = (37u32, 5u32, 40usize);
    let all = Rect::new(0, 0, w, h);
    let mut y = vec![9u8; stride * h as usize];
    let a = fingerprint(&y, stride, all);
    let corner = fingerprint(&y, stride, Rect::new(0, 0, 8, 2));
    y[stride * 2 + 38] = 1; // row padding is ignored
    assert_eq!(fingerprint(&y, stride, all), a);
    y[stride * 4 + 36] = 10;
    assert_ne!(fingerprint(&y, stride, all), a);
    assert_eq!(fingerprint(&y, stride, Rect::new(0, 0, 8, 2)), corner, "change outside the rect");
    assert_ne!(fingerprint(&y, stride, Rect::new(0, 0, w - 1, h)), fingerprint(&y, stride, all));
}

#[test]
fn luma_grid_detects_local_change() {
    let (w, h) = (64usize, 36usize);
    let mut y = vec![100u8; w * h];
    let before = luma_grid(&y, w, w, h, 8, 4);
    assert_eq!(before.len(), 32);
    assert!(!grid_changed(&before, &luma_grid(&y, w, w, h, 8, 4), 3));
    for row in 0..9 {
        for x in 0..8 {
            y[row * w + x] = 250;
        }
    }
    assert!(grid_changed(&before, &luma_grid(&y, w, w, h, 8, 4), 3));
    assert!(grid_changed(&before, &[], 3));
    assert!(luma_grid(&y, w, w, h, 0, 4).is_empty());
}

#[test]
fn rect_clamping_and_options_json() {
    assert_eq!(Rect::new(10, 10, 100, 100).clamp_to(50, 60), Some(Rect::new(10, 10, 40, 50)));
    assert_eq!(Rect::new(80, 0, 10, 10).clamp_to(50, 60), None);
    let opts: OcrOptions =
        serde_json::from_value(serde_json::json!({ "detail": "full", "region": { "x": 1, "y": 2, "width": 3, "height": 4 } }))
            .expect("options");
    assert_eq!(opts.detail, rkvm_vision::Detail::Full);
    assert_eq!(opts.region, Some(Rect::new(1, 2, 3, 4)));
    let empty: OcrOptions = serde_json::from_value(serde_json::json!({})).expect("defaults");
    assert_eq!(empty, OcrOptions::default());
    assert_eq!(empty.detail, rkvm_vision::Detail::Auto);
    let waits: OcrOptions =
        serde_json::from_value(serde_json::json!({ "waitStableMs": 500 })).expect("wait");
    assert_eq!(waits.wait_stable_ms, Some(500));
}
