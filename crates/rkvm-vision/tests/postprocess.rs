use rkvm_vision::det::{DbParams, TextBox, db_boxes, merge_boxes, plan_tiles, reading_order};
use rkvm_vision::image::{RectF, RgbImage};
use rkvm_vision::rec::{Segment, ctc_decode, load_dict, pick_width, split_line};
use rkvm_vision::signature::{grid_changed, luma_grid};
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
    let img = RgbImage::from_nv12(w as u32, h as u32, &y, stride, &uv, stride).expect("convert");
    let px: Vec<&[u8]> = img.data().chunks(3).take(4).collect();
    assert_eq!(px[0], [0, 0, 0]);
    assert_eq!(px[1], [255, 255, 255]);
    assert!(px[2].iter().all(|&c| c.abs_diff(128) <= 1));
    assert!(RgbImage::from_nv12(3, 2, &y, stride, &uv, stride).is_err(), "odd width");
    assert!(RgbImage::from_nv12(4, 2, &y[..4], stride, &uv, stride).is_err(), "short plane");
}

#[test]
fn bilinear_sampling_writes_bgr_inside_the_patch_only() {
    let img = RgbImage::new(2, 1, vec![10, 20, 30, 50, 60, 70]).expect("image");
    let mut out = vec![7u8; 3 * 3];
    img.sample_bgr(RectF { x0: 0.0, y0: 0.0, x1: 2.0, y1: 1.0 }, 1, 1, &mut out, 3);
    assert_eq!(&out[..3], &[50, 40, 30], "midpoint average, channels reversed");
    assert!(out[3..].iter().all(|&v| v == 7));
    assert!(RgbImage::new(2, 2, vec![0; 3]).is_err());
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
        img.blank_columns(Rect::new(0, 0, w, h)),
        vec![true, false, true, true, false, true]
    );
    assert!(img.blank_columns(Rect::new(10, 10, 2, 2)).is_empty());
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
}
