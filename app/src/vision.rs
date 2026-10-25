use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use gstreamer as gst;
use gstreamer_video as gst_video;
use gstreamer_video::VideoFrameExt;
use once_cell::sync::Lazy;
use rkvm_vision::signature::{grid_changed, luma_grid};
use rkvm_vision::{Detail, Engine, OcrOptions, OcrResult, Rect, RgbImage};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Models, dictionary and (optionally) `librknnrt.so`; see docs/vision.md.
pub const MODEL_DIR: &str = "/userdata/rustkvm/vision";

const STABLE_GRID: (usize, usize) = (32, 18);
/// A caret blinking in one cell moves its mean by about 1.
const STABLE_THRESHOLD: u8 = 3;
const STABLE_POLL: Duration = Duration::from_millis(200);

static ENGINE: Lazy<Mutex<Option<Arc<Engine>>>> = Lazy::new(|| Mutex::new(None));

/// Loads the OCR engine on first use; a failed load is retried next time.
async fn engine() -> Result<Arc<Engine>> {
    let mut slot = ENGINE.lock().await;
    if let Some(engine) = slot.as_ref() {
        return Ok(engine.clone());
    }
    let engine = tokio::task::spawn_blocking(|| Engine::load(Path::new(MODEL_DIR)))
        .await
        .map_err(|e| anyhow!("join error: {e}"))?
        .context("OCR engine unavailable (see docs/vision.md)")?;
    let engine = Arc::new(engine);
    *slot = Some(engine.clone());
    Ok(engine)
}

pub async fn read_screen_text(options: OcrOptions) -> Result<OcrResult> {
    let engine = engine().await?;
    let sample = crate::video::capture_raw_frame().await?;
    tokio::task::spawn_blocking(move || {
        let image = rgb_from_sample(&sample)?;
        engine.ocr(&image, &options)
    })
    .await
    .map_err(|e| anyhow!("join error: {e}"))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindTextParams {
    pub text: String,
    #[serde(default)]
    pub detail: Detail,
    #[serde(default)]
    pub region: Option<Rect>,
}

#[derive(Debug, Serialize)]
pub struct TextMatch {
    pub text: String,
    pub score: f32,
    /// Where to click: the estimated center of the matched part of the line.
    pub x: u32,
    pub y: u32,
    pub rect: Rect,
}

pub async fn find_screen_text(params: FindTextParams) -> Result<Vec<TextMatch>> {
    let needle = normalize(&params.text);
    if needle.is_empty() {
        bail!("text must not be empty");
    }
    let result = read_screen_text(OcrOptions {
        detail: params.detail,
        region: params.region,
        min_score: None,
    })
    .await?;
    Ok(result
        .lines
        .into_iter()
        .filter_map(|line| {
            let hay = normalize(&line.text);
            let start = hay.find(needle.as_str())?;
            let chars = hay.chars().count().max(1) as f32;
            let before = hay[..start].chars().count() as f32;
            let len = needle.chars().count() as f32;
            let mid = (before + len / 2.0) / chars;
            let x = line.rect.x + (line.rect.width as f32 * mid).round() as u32;
            let (_, y) = line.rect.center();
            Some(TextMatch { text: line.text, score: line.score, x, y, rect: line.rect })
        })
        .collect())
}

/// Lowercase with whitespace removed, so "Sign in" matches "SignIn".
fn normalize(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaitStableParams {
    pub timeout_ms: u64,
    pub stable_ms: u64,
}

impl Default for WaitStableParams {
    fn default() -> Self {
        Self { timeout_ms: 10_000, stable_ms: 1_000 }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitStableResult {
    pub stable: bool,
    pub waited_ms: u64,
}

/// Waits until the screen has not changed for `stable_ms`, or `timeout_ms`.
pub async fn wait_screen_stable(params: WaitStableParams) -> Result<WaitStableResult> {
    let timeout = Duration::from_millis(params.timeout_ms.clamp(100, 120_000));
    let stable_for = Duration::from_millis(params.stable_ms.clamp(100, 60_000));
    let started = Instant::now();
    let mut previous: Option<Vec<u8>> = None;
    let mut unchanged_since = started;
    loop {
        let sample = crate::video::capture_raw_frame().await?;
        let grid = tokio::task::spawn_blocking(move || luma_grid_from_sample(&sample))
            .await
            .map_err(|e| anyhow!("join error: {e}"))??;
        let now = Instant::now();
        if previous.as_ref().is_none_or(|p| grid_changed(p, &grid, STABLE_THRESHOLD)) {
            unchanged_since = now;
        }
        previous = Some(grid);
        let waited_ms = now.duration_since(started).as_millis() as u64;
        if now.duration_since(unchanged_since) >= stable_for {
            return Ok(WaitStableResult { stable: true, waited_ms });
        }
        if now.duration_since(started) >= timeout {
            return Ok(WaitStableResult { stable: false, waited_ms });
        }
        tokio::time::sleep(STABLE_POLL).await;
    }
}

fn nv12_frame(sample: &gst::Sample) -> Result<gst_video::VideoFrameRef<&gst::BufferRef>> {
    let caps = sample.caps().context("raw frame has no caps")?;
    let info = gst_video::VideoInfo::from_caps(caps).context("raw frame caps")?;
    if info.format() != gst_video::VideoFormat::Nv12 {
        bail!("raw frame is {:?}, expected NV12", info.format());
    }
    let buffer = sample.buffer().context("raw frame has no buffer")?;
    gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
        .map_err(|e| anyhow!("map raw frame: {e}"))
}

fn plane<'a>(
    frame: &'a gst_video::VideoFrameRef<&gst::BufferRef>,
    index: u32,
) -> Result<(&'a [u8], usize)> {
    let data = frame.plane_data(index).map_err(|e| anyhow!("raw frame plane {index}: {e}"))?;
    let stride = frame.plane_stride().get(index as usize).copied().unwrap_or(0);
    Ok((data, usize::try_from(stride).context("negative stride")?))
}

fn rgb_from_sample(sample: &gst::Sample) -> Result<RgbImage> {
    let frame = nv12_frame(sample)?;
    let (y, y_stride) = plane(&frame, 0)?;
    let (uv, uv_stride) = plane(&frame, 1)?;
    RgbImage::from_nv12(frame.width(), frame.height(), y, y_stride, uv, uv_stride)
}

fn luma_grid_from_sample(sample: &gst::Sample) -> Result<Vec<u8>> {
    let frame = nv12_frame(sample)?;
    let (y, stride) = plane(&frame, 0)?;
    let (cols, rows) = STABLE_GRID;
    Ok(luma_grid(y, stride, frame.width() as usize, frame.height() as usize, cols, rows))
}
