//! Runs OCR on image files with the device NPU, for checking models and
//! timings without the app: `ocr_image <model dir> [--full] <image>...`.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rkvm_vision::{Detail, Engine, OcrOptions, RgbImage};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().context("usage: ocr_image <model dir> [--full] <image>...")?;
    let mut options = OcrOptions::default();
    let mut images = Vec::new();
    for arg in args {
        if arg == "--full" {
            options.detail = Detail::Full;
        } else {
            images.push(arg);
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
        engine.ocr(&rgb, &options)?;
        let result = engine.ocr(&rgb, &options)?;
        eprintln!("{path}: {} lines, {:?}", result.lines.len(), result.timings);
        println!("{}", serde_json::to_string(&result)?);
    }
    Ok(())
}
