use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use gstreamer as gst;
use gstreamer_video as gst_video;
use gstreamer_video::VideoFrameExt;
use once_cell::sync::Lazy;
use rkvm_vision::signature::{fingerprint, grid_changed, luma_grid};
use rkvm_vision::{Detail, Engine, Nv12Frame, OcrOptions, OcrResult, Rect, Timings};
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

/// The last read: reused while the screen stays pixel-identical and the
/// options match.
struct LastRead {
    fingerprint: u64,
    options: OcrOptions,
    result: OcrResult,
}

static LAST_READ: Lazy<parking_lot::Mutex<Option<LastRead>>> =
    Lazy::new(|| parking_lot::Mutex::new(None));

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

/// Reads the screen text, or returns the previous read (`cached: true`)
/// when the screen, or the region read, has not changed by a single pixel
/// since then.
pub async fn read_screen_text(options: OcrOptions) -> Result<OcrResult> {
    let engine = engine().await?;
    if let Some(stable_ms) = options.wait_stable_ms {
        wait_screen_stable(WaitStableParams {
            timeout_ms: stable_ms.saturating_mul(3).max(WaitStableParams::default().timeout_ms),
            stable_ms,
        })
        .await?;
    }
    let options = OcrOptions { wait_stable_ms: None, ..options };
    let sample = crate::video::capture_raw_frame().await?;
    tokio::task::spawn_blocking(move || {
        let started = std::time::Instant::now();
        with_nv12(&sample, options.region, |frame, print, copy_ms| {
            if let Some(last) = LAST_READ.lock().as_ref()
                && last.fingerprint == print
                && last.options == options
            {
                let total_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
                return Ok(OcrResult {
                    cached: true,
                    timings: Timings { total_ms, ..Timings::default() },
                    ..last.result.clone()
                });
            }
            let mut result = engine.ocr(frame, &options)?;
            result.timings.prepare_ms += copy_ms;
            result.timings.total_ms += copy_ms;
            *LAST_READ.lock() =
                Some(LastRead { fingerprint: print, options, result: result.clone() });
            Ok(result)
        })
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
    let needle = needle(&params.text)?;
    let result = read_screen_text(OcrOptions {
        detail: params.detail,
        region: params.region,
        ..OcrOptions::default()
    })
    .await?;
    Ok(matches(result, &needle))
}

fn needle(text: &str) -> Result<String> {
    let needle = normalize(text);
    if needle.is_empty() {
        bail!("text must not be empty");
    }
    Ok(needle)
}

fn matches(result: OcrResult, needle: &str) -> Vec<TextMatch> {
    result
        .lines
        .into_iter()
        .filter_map(|line| {
            let hay = normalize(&line.text);
            let start = hay.find(needle)?;
            let chars = hay.chars().count().max(1) as f32;
            let before = hay[..start].chars().count() as f32;
            let len = needle.chars().count() as f32;
            let mid = (before + len / 2.0) / chars;
            let x = line.rect.x + (line.rect.width as f32 * mid).round() as u32;
            let (_, y) = line.rect.center();
            Some(TextMatch { text: line.text, score: line.score, x, y, rect: line.rect })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitForTextParams {
    pub text: String,
    #[serde(default)]
    pub detail: Detail,
    #[serde(default)]
    pub region: Option<Rect>,
    #[serde(default = "default_wait_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_wait_interval_ms")]
    pub interval_ms: u64,
}

fn default_wait_timeout_ms() -> u64 {
    10_000
}

fn default_wait_interval_ms() -> u64 {
    500
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitForTextResult {
    pub found: bool,
    pub waited_ms: u64,
    /// Reads made, including ones answered from the cache.
    pub reads: u32,
    pub matches: Vec<TextMatch>,
}

/// Reads the screen until `text` shows up or `timeout_ms` passes. Unchanged
/// screens are answered from the last read, so polling costs a capture.
pub async fn wait_for_text(params: WaitForTextParams) -> Result<WaitForTextResult> {
    let needle = needle(&params.text)?;
    let timeout = Duration::from_millis(params.timeout_ms.clamp(100, 300_000));
    let interval = Duration::from_millis(params.interval_ms.clamp(100, 10_000));
    let options =
        OcrOptions { detail: params.detail, region: params.region, ..OcrOptions::default() };
    let started = Instant::now();
    let mut reads = 0;
    loop {
        let found = matches(read_screen_text(options.clone()).await?, &needle);
        reads += 1;
        let waited_ms = started.elapsed().as_millis() as u64;
        if !found.is_empty() || started.elapsed() >= timeout {
            return Ok(WaitForTextResult {
                found: !found.is_empty(),
                waited_ms,
                reads,
                matches: found,
            });
        }
        tokio::time::sleep(interval).await;
    }
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

/// Reused across reads: one 4K frame is 12 MB.
static FRAME_COPY: Lazy<parking_lot::Mutex<Vec<u8>>> =
    Lazy::new(|| parking_lot::Mutex::new(Vec::new()));

/// Runs `f` on the frame of `sample`, the fingerprint of the luma inside
/// `region` (the whole frame without one) and the time the frame took to
/// prepare.
///
/// The capture buffer is uncached DMA memory: bilinear sampling straight
/// from it reads every byte from DRAM, which is several times slower than
/// one streaming row copy into a reused buffer and sampling from that.
fn with_nv12<T>(
    sample: &gst::Sample,
    region: Option<Rect>,
    f: impl FnOnce(&Nv12Frame<'_>, u64, u32) -> Result<T>,
) -> Result<T> {
    let started = std::time::Instant::now();
    let frame = nv12_frame(sample)?;
    let (width, height) = (frame.width(), frame.height());
    let (w, h) = (width as usize, height as usize);
    let (y, y_stride) = plane(&frame, 0)?;
    let (uv, uv_stride) = plane(&frame, 1)?;
    let mut copy = FRAME_COPY.lock();
    copy.resize(w * h * 3 / 2, 0);
    let (y_copy, uv_copy) = copy.split_at_mut(w * h);
    for (dst, src) in y_copy.chunks_exact_mut(w).zip(y.chunks(y_stride)) {
        dst.copy_from_slice(src.get(..w).context("short luma row")?);
    }
    for (dst, src) in uv_copy.chunks_exact_mut(w).zip(uv.chunks(uv_stride)) {
        dst.copy_from_slice(src.get(..w).context("short chroma row")?);
    }
    let full = Rect::new(0, 0, width, height);
    let print =
        fingerprint(y_copy, w, region.and_then(|r| r.clamp_to(width, height)).unwrap_or(full));
    let (y_copy, uv_copy) = copy.split_at(w * h);
    let prepare_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    f(&Nv12Frame::new(width, height, y_copy, w, uv_copy, w)?, print, prepare_ms)
}

fn luma_grid_from_sample(sample: &gst::Sample) -> Result<Vec<u8>> {
    let frame = nv12_frame(sample)?;
    let (y, stride) = plane(&frame, 0)?;
    let (cols, rows) = STABLE_GRID;
    Ok(luma_grid(y, stride, frame.width() as usize, frame.height() as usize, cols, rows))
}
