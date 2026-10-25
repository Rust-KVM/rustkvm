//! Runs OCR on image files with the device NPU, for checking models and
//! timings without the app:
//! `ocr_image <model dir> [--fast|--full] [--nv12] <image>...`.
//!
//! The default is `auto`, as in the app. `--nv12` converts each image to
//! NV12 first and reads that in place, as the app reads captured frames.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rkvm_vision::{Detail, Engine, Frame, Nv12Frame, OcrOptions, RgbImage};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let dir =
        args.next().context("usage: ocr_image <model dir> [--fast|--full] [--nv12] <image>...")?;
    let mut options = OcrOptions::default();
    let mut nv12 = false;
    let mut images = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--fast" => options.detail = Detail::Fast,
            "--full" => options.detail = Detail::Full,
            "--nv12" => nv12 = true,
            _ => images.push(arg),
        }
    }
    if images.is_empty() {
        bail!("no images given");
    }

    let engine = Engine::load(Path::new(&dir))?;
    eprintln!("{:?}", engine.info());
    for path in images {
        let decoded = image::open(&path).with_context(|| format!("open {path}"))?.to_rgb8();
        let (w, h) = decoded.dimensions();
        let rgb = RgbImage::new(w, h, decoded.into_raw())?;
        let planes = nv12.then(|| to_nv12(&rgb));
        let in_place = match &planes {
            Some((y, uv)) => Some(Nv12Frame::new(w & !1, h & !1, y, w as usize, uv, w as usize)?),
            None => None,
        };
        let frame: &dyn Frame = match &in_place {
            Some(f) => f,
            None => &rgb,
        };
        engine.ocr(frame, &options)?;
        let result = engine.ocr(frame, &options)?;
        eprintln!(
            "{path}: {} lines, {:?}, {:?}",
            result.lines.len(),
            result.detail,
            result.timings
        );
        println!("{}", serde_json::to_string(&result)?);
    }
    Ok(())
}

/// BT.601 limited-range NV12 planes of an RGB image, rows `width` long.
fn to_nv12(rgb: &RgbImage) -> (Vec<u8>, Vec<u8>) {
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 3;
        let d = rgb.data();
        (f32::from(d[i]), f32::from(d[i + 1]), f32::from(d[i + 2]))
    };
    let mut y_plane = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = px(x, y);
            y_plane[y * w + x] = (16.0 + 0.257 * r + 0.504 * g + 0.098 * b).round() as u8;
        }
    }
    let mut uv = vec![128u8; w * (h / 2)];
    for cy in 0..h / 2 {
        for cx in 0..w / 2 {
            let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (pr, pg, pb) = px(2 * cx + dx, 2 * cy + dy);
                (r, g, b) = (r + pr / 4.0, g + pg / 4.0, b + pb / 4.0);
            }
            uv[cy * w + 2 * cx] = (128.0 - 0.148 * r - 0.291 * g + 0.439 * b).round() as u8;
            uv[cy * w + 2 * cx + 1] = (128.0 + 0.439 * r - 0.368 * g - 0.071 * b).round() as u8;
        }
    }
    (y_plane, uv)
}
