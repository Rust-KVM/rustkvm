//! The OCR engine: PP-OCRv4 models on the three RK3588 NPU cores.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail};
use tracing::info;

use crate::det::{self, DbParams, TextBox};
use crate::image::{Frame, RectF, blank_columns, sample_bgr};
use crate::rec::{self, Segment};
use crate::rknn::{CORES, Context, InternalMem, Runtime, Tensor};
use crate::types::{Detail, OcrOptions, OcrResult, Rect, TextLine, Timings};

/// Where the runtime is looked for when the model directory has none.
pub const SYSTEM_RUNTIME: &str = "/usr/lib/librknnrt.so";
pub const RUNTIME_FILE: &str = "librknnrt.so";
/// The single detector of the first model bundle, used when the directory
/// has no `det_<width>x<height>.rknn`.
pub const LEGACY_DET_FILE: &str = "det.rknn";
pub const DICT_FILE: &str = "ppocr_keys_v1.txt";

const TILE_OVERLAP: u32 = 96;
/// `fast` scales the screen to fit this, about 1080p.
const FAST_SIZE: (f32, f32) = (1920.0, 1088.0);
/// `auto` repeats detection at native resolution when the median text box
/// of the fast pass is lower than this in scaled pixels. Boxes include the
/// DB unclip margin: on a 4K screen 10-14 px text gives 13-15 px boxes at
/// fast scale (and reads 3-4 times better at full), 16 px and larger text
/// and any 1080p text give 15-16 px or more.
const AUTO_MIN_TEXT_PX: f32 = 15.5;
const DEFAULT_MIN_SCORE: f32 = 0.5;
/// PP-OCR pads recognizer input with zeros after normalising to -1..1; the
/// models are converted with mean and std 127.5, so raw 128 is that zero.
const REC_PAD: u8 = 128;
/// Threads that crop and resample text lines for recognition.
const PREP_THREADS: usize = 4;

/// Versions and model shapes, for logs and health reports.
#[derive(Clone, Debug)]
pub struct EngineInfo {
    pub runtime_path: PathBuf,
    pub api_version: String,
    pub driver_version: String,
    /// Detector input shapes, width × height.
    pub det_inputs: Vec<(u32, u32)>,
    pub rec_widths: Vec<u32>,
    /// NPU contexts that take jobs: a detection + recognition one and a
    /// recognition-only one per core.
    pub workers: usize,
    /// Activation memory allocated on the NPU, in MiB.
    pub internal_mib: u32,
}

/// A model file and what its first context reported; every worker loads
/// its own context from `file`.
struct Model {
    file: Vec<u8>,
    in_w: u32,
    in_h: u32,
    output: Tensor,
    internal: u32,
}

impl Model {
    fn load(rt: &Arc<Runtime>, path: &Path) -> Result<Self> {
        let mut file = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let ctx =
            Context::load(rt, &mut file).with_context(|| format!("load {}", path.display()))?;
        let input = ctx.input()?;
        let (in_h, in_w, channels) = match (input.is_nchw, input.dims.as_slice()) {
            (true, &[_, c, h, w]) => (h, w, c),
            (false, &[_, h, w, c]) => (h, w, c),
            (_, dims) => bail!("{}: unexpected input shape {dims:?}", path.display()),
        };
        if channels != 3 {
            bail!("{}: expected 3 input channels, got {channels}", path.display());
        }
        let output = ctx.output()?;
        let internal = ctx.internal_size()?;
        Ok(Self { file, in_w, in_h, output, internal })
    }
}

struct DetShape {
    width: u32,
    height: u32,
    out_len: usize,
}

struct RecModel {
    width: u32,
    steps: usize,
    classes: usize,
    out_len: usize,
    f16: bool,
}

/// What every worker needs to turn raw outputs into results.
struct Shared {
    dets: Vec<DetShape>,
    recs: Vec<RecModel>,
    det_params: DbParams,
    dict: Vec<String>,
}

/// Recognized text and its confidence.
type Piece = (String, f32);

enum Job {
    Det { shape: usize, input: Vec<u8>, reply: Sender<Result<Vec<TextBox>>> },
    Rec { model: usize, input: Vec<u8>, reply: Sender<Result<Piece>> },
}

/// Jobs waiting for a worker. Recognition-only workers skip detection jobs.
#[derive(Default)]
struct Queue {
    state: Mutex<(VecDeque<Job>, bool)>,
    ready: Condvar,
}

impl Queue {
    fn push(&self, job: Job) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| anyhow!("NPU job queue poisoned"))?;
        if state.1 {
            bail!("NPU workers have stopped");
        }
        state.0.push_back(job);
        drop(state);
        self.ready.notify_all();
        Ok(())
    }

    /// The oldest job this worker can run, or `None` once the queue closes.
    fn take(&self, can_detect: bool) -> Option<Job> {
        let mut state = self.state.lock().ok()?;
        loop {
            if state.1 {
                return None;
            }
            let found = state.0.iter().position(|j| can_detect || matches!(j, Job::Rec { .. }));
            if let Some(job) = found.and_then(|i| state.0.remove(i)) {
                return Some(job);
            }
            state = self.ready.wait(state).ok()?;
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.1 = true;
        }
        self.ready.notify_all();
    }
}

/// One NPU context per model, all on one core and never running at the same
/// time, so they share one activation buffer.
struct Worker {
    /// Declared first so it is freed while the context that allocated it
    /// (`dets[0]` or `recs[0]`) is still alive.
    mem: InternalMem,
    dets: Vec<Context>,
    recs: Vec<Context>,
    det_out: Vec<f32>,
    rec_f16: Vec<u16>,
    rec_f32: Vec<f32>,
}

impl Worker {
    fn new(
        rt: &Arc<Runtime>,
        core: u32,
        dets: &[Model],
        recs: &[Model],
        shared: &Shared,
    ) -> Result<Self> {
        let open = |m: &Model| -> Result<Context> {
            let mut ctx = Context::load(rt, &mut m.file.clone())?;
            ctx.set_core(core)?;
            Ok(ctx)
        };
        let mut det_ctx = dets.iter().map(open).collect::<Result<Vec<_>>>()?;
        let mut recs_ctx = recs.iter().map(open).collect::<Result<Vec<_>>>()?;
        let size =
            dets.iter().chain(recs).map(|m| m.internal).max().context("worker without models")?;
        let first = det_ctx.first().or(recs_ctx.first()).context("worker without models")?;
        let mem = InternalMem::new(first, size)?;
        for ctx in det_ctx.iter_mut().chain(recs_ctx.iter_mut()) {
            ctx.set_internal_mem(&mem)?;
        }
        let det_len = if det_ctx.is_empty() {
            0
        } else {
            shared.dets.iter().map(|d| d.out_len).max().unwrap_or(0)
        };
        let rec_len = |f16: bool| {
            shared.recs.iter().filter(|r| r.f16 == f16).map(|r| r.out_len).max().unwrap_or(0)
        };
        Ok(Self {
            mem,
            dets: det_ctx,
            recs: recs_ctx,
            det_out: vec![0.0; det_len],
            rec_f16: vec![0; rec_len(true)],
            rec_f32: vec![0.0; rec_len(false)],
        })
    }

    fn detect(&mut self, shared: &Shared, shape: usize, input: &[u8]) -> Result<Vec<TextBox>> {
        let (ctx, s) = self
            .dets
            .get_mut(shape)
            .zip(shared.dets.get(shape))
            .ok_or_else(|| anyhow!("no detection model {shape}"))?;
        ctx.run(input)?;
        let prob = &mut self.det_out[..s.out_len];
        ctx.output_f32(prob)?;
        Ok(det::db_boxes(prob, s.width as usize, s.height as usize, &shared.det_params))
    }

    fn recognize(&mut self, shared: &Shared, model: usize, input: &[u8]) -> Result<Piece> {
        let (ctx, m) = self
            .recs
            .get_mut(model)
            .zip(shared.recs.get(model))
            .ok_or_else(|| anyhow!("no recognition model {model}"))?;
        ctx.run(input)?;
        if m.f16 {
            let out = &mut self.rec_f16[..m.out_len];
            ctx.output_f16(out)?;
            Ok(rec::ctc_decode_f16(out, m.steps, m.classes, &shared.dict))
        } else {
            let out = &mut self.rec_f32[..m.out_len];
            ctx.output_f32(out)?;
            Ok(rec::ctc_decode(out, m.steps, m.classes, &shared.dict))
        }
    }

    fn serve(mut self, queue: &Queue, shared: &Shared) {
        let can_detect = !self.dets.is_empty();
        while let Some(job) = queue.take(can_detect) {
            match job {
                Job::Det { shape, input, reply } => {
                    let _ = reply.send(self.detect(shared, shape, &input));
                }
                Job::Rec { model, input, reply } => {
                    let _ = reply.send(self.recognize(shared, model, &input));
                }
            }
        }
    }
}

/// `det_<width>x<height>.rknn` files in `dir`, or the legacy `det.rknn`.
fn detector_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("det_")?.strip_suffix(".rknn"))
                .is_some_and(|shape| parse_shape(shape).is_some())
        })
        .collect();
    paths.sort();
    if paths.is_empty() && dir.join(LEGACY_DET_FILE).is_file() {
        paths.push(dir.join(LEGACY_DET_FILE));
    }
    if paths.is_empty() {
        bail!("no det_<width>x<height>.rknn detection model in {}", dir.display());
    }
    Ok(paths)
}

fn parse_shape(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn named_suffix(path: &Path, prefix: &str) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    Some(stem.strip_prefix(prefix)?.to_string())
}

/// PP-OCR models loaded on two workers per NPU core. `ocr` can be
/// called from several threads at once.
pub struct Engine {
    queue: Arc<Queue>,
    workers: Vec<JoinHandle<()>>,
    shared: Arc<Shared>,
    info: EngineInfo,
    rec_height: u32,
}

impl Engine {
    /// Loads every `det_<w>x<h>.rknn` (or `det.rknn`), every
    /// `rec_<width>.rknn` and the dictionary from `dir`, with `librknnrt.so`
    /// from `dir` or the system library path.
    pub fn load(dir: &Path) -> Result<Self> {
        let runtime_path = [dir.join(RUNTIME_FILE), PathBuf::from(SYSTEM_RUNTIME)]
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| {
                anyhow!("{RUNTIME_FILE} not found in {} or {SYSTEM_RUNTIME}", dir.display())
            })?;
        let rt = Runtime::load(&runtime_path)?;

        let mut dets = Vec::new();
        for path in detector_paths(dir)? {
            let model = Model::load(&rt, &path)?;
            let named = named_suffix(&path, "det_").and_then(|s| parse_shape(&s));
            if named.is_some_and(|n| n != (model.in_w, model.in_h)) {
                bail!(
                    "{} takes {}x{}, which its name does not match; reconvert it with \
                     scripts/vision-models.sh",
                    path.display(),
                    model.in_w,
                    model.in_h
                );
            }
            dets.push(model);
        }
        dets.sort_by_key(|m| (m.in_w, m.in_h));

        let mut rec_paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("read {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rec_") && n.ends_with(".rknn"))
            })
            .collect();
        rec_paths.sort();
        if rec_paths.is_empty() {
            bail!("no rec_<width>.rknn recognition models in {}", dir.display());
        }
        let mut recs = Vec::new();
        for path in &rec_paths {
            let model = Model::load(&rt, path)?;
            if named_suffix(path, "rec_").and_then(|w| w.parse::<u32>().ok()) != Some(model.in_w) {
                bail!(
                    "{} takes input width {}, which its name does not match; reconvert it \
                     with scripts/vision-models.sh",
                    path.display(),
                    model.in_w
                );
            }
            recs.push(model);
        }
        recs.sort_by_key(|m| m.in_w);

        let dict_text = std::fs::read_to_string(dir.join(DICT_FILE))
            .with_context(|| format!("read {DICT_FILE}"))?;
        let dict = rec::load_dict(&dict_text);

        let rec_height = recs[0].in_h;
        let mut rec_models = Vec::with_capacity(recs.len());
        for r in &recs {
            if r.in_h != rec_height {
                bail!("recognition models disagree on input height ({} vs {rec_height})", r.in_h);
            }
            // rknn-toolkit2 keeps the (N, T, C) output as (N, T, 1, C) when the
            // model is converted to run its head entirely on the NPU.
            let (steps, classes) = match r.output.dims.as_slice() {
                &[_, t, c] | &[_, t, 1, c] => (t as usize, c as usize),
                dims => bail!("unexpected recognition output shape {dims:?}"),
            };
            if classes < dict.len() + 1 {
                bail!(
                    "dictionary has {} characters but the model only {classes} classes",
                    dict.len()
                );
            }
            rec_models.push(RecModel {
                width: r.in_w,
                steps,
                classes,
                out_len: r.output.elements,
                f16: r.output.is_f16,
            });
        }
        let shared = Arc::new(Shared {
            dets: dets
                .iter()
                .map(|d| DetShape { width: d.in_w, height: d.in_h, out_len: d.output.elements })
                .collect(),
            recs: rec_models,
            det_params: DbParams::default(),
            dict,
        });

        let queue = Arc::new(Queue::default());
        let mut workers = Vec::new();
        let mut internal = 0u64;
        for (index, &core) in CORES.iter().enumerate() {
            for (kind, det_models) in [("npu", dets.as_slice()), ("npu-rec", &[])] {
                let worker = Worker::new(&rt, core, det_models, &recs, &shared)?;
                internal += u64::from(worker.mem.size());
                let (queue, shared) = (queue.clone(), shared.clone());
                let handle = std::thread::Builder::new()
                    .name(format!("{kind}-core{index}"))
                    .spawn(move || worker.serve(&queue, &shared))
                    .context("spawn NPU worker")?;
                workers.push(handle);
            }
        }

        let (api_version, driver_version) =
            Context::load(&rt, &mut recs[0].file.clone())?.versions()?;
        let info = EngineInfo {
            runtime_path,
            api_version,
            driver_version,
            det_inputs: shared.dets.iter().map(|d| (d.width, d.height)).collect(),
            rec_widths: shared.recs.iter().map(|m| m.width).collect(),
            workers: workers.len(),
            internal_mib: (internal >> 20) as u32,
        };
        info!(
            runtime = %info.runtime_path.display(),
            api = %info.api_version,
            driver = %info.driver_version,
            det_inputs = ?info.det_inputs,
            rec_widths = ?info.rec_widths,
            workers = info.workers,
            internal_mib = info.internal_mib,
            "OCR engine loaded"
        );
        Ok(Self { queue, workers, shared, info, rec_height })
    }

    pub fn info(&self) -> &EngineInfo {
        &self.info
    }

    pub fn ocr<F: Frame + ?Sized>(&self, frame: &F, options: &OcrOptions) -> Result<OcrResult> {
        let started = Instant::now();
        let (width, height) = (frame.width(), frame.height());
        let full = Rect::new(0, 0, width, height);
        let region = options
            .region
            .map_or(Some(full), |r| r.clamp_to(width, height))
            .context("region is outside the frame")?;

        let fast_scale =
            (FAST_SIZE.0 / region.width as f32).min(FAST_SIZE.1 / region.height as f32).min(1.0);
        let (detail, passes) = match options.detail {
            Detail::Fast => (Detail::Fast, vec![self.detect(frame, region, fast_scale)?]),
            Detail::Full => (Detail::Full, vec![self.detect(frame, region, 1.0)?]),
            Detail::Auto => {
                let first = self.detect(frame, region, fast_scale)?;
                let small = fast_scale < 1.0
                    && median_height(&first.boxes)
                        .is_some_and(|h| h * fast_scale < AUTO_MIN_TEXT_PX);
                if small {
                    (Detail::Full, vec![first, self.detect(frame, region, 1.0)?])
                } else {
                    (Detail::Fast, vec![first])
                }
            }
        };
        let prepare_ms = passes.iter().map(|p| p.prepare_ms).sum();
        let detect_ms = passes.iter().map(|p| p.detect_ms).sum();
        let mut passes = passes;
        let boxes = passes.pop().map(|p| p.boxes).unwrap_or_default();

        let recognize_started = Instant::now();
        let min_score = options.min_score.unwrap_or(DEFAULT_MIN_SCORE);
        let lines = self
            .recognize(frame, &boxes)?
            .into_iter()
            .filter(|l| !l.text.is_empty() && l.score >= min_score)
            .collect();
        let recognize_ms = elapsed_ms(recognize_started);

        Ok(OcrResult {
            width,
            height,
            lines,
            timings: Timings { prepare_ms, detect_ms, recognize_ms, total_ms: elapsed_ms(started) },
            detail,
            cached: false,
        })
    }

    fn detect<F: Frame + ?Sized>(&self, frame: &F, region: Rect, scale: f32) -> Result<DetPass> {
        let started = Instant::now();
        let scaled_w = ((region.width as f32 * scale).round() as u32).max(1);
        let scaled_h = ((region.height as f32 * scale).round() as u32).max(1);
        let shapes: Vec<(u32, u32)> =
            self.shared.dets.iter().map(|d| (d.width, d.height)).collect();
        let shape = det::choose_shape(&shapes, scaled_w, scaled_h, TILE_OVERLAP, CORES.len())
            .context("no detection model fits the tile overlap")?;
        let (det_w, det_h) = shapes[shape];
        let tiles = det::plan_tiles(scaled_w, scaled_h, det_w, det_h, TILE_OVERLAP);

        let mut prepare = std::time::Duration::ZERO;
        let mut pending = Vec::with_capacity(tiles.len());
        for tile in &tiles {
            let sampling = Instant::now();
            let mut input = vec![0u8; det_w as usize * det_h as usize * 3];
            let src = RectF {
                x0: region.x as f32 + tile.x as f32 / scale,
                y0: region.y as f32 + tile.y as f32 / scale,
                x1: region.x as f32 + tile.right() as f32 / scale,
                y1: region.y as f32 + tile.bottom() as f32 / scale,
            };
            sample_bgr(
                frame,
                src,
                tile.width as usize,
                tile.height as usize,
                &mut input,
                det_w as usize,
            );
            prepare += sampling.elapsed();
            let (reply, rx) = mpsc::channel();
            self.queue.push(Job::Det { shape, input, reply })?;
            pending.push((*tile, rx));
        }

        let mut boxes = Vec::new();
        for (tile, rx) in pending {
            for b in rx.recv().context("NPU worker stopped")?? {
                boxes.push(TextBox {
                    x0: region.x as f32 + (tile.x as f32 + b.x0) / scale,
                    y0: region.y as f32 + (tile.y as f32 + b.y0) / scale,
                    x1: region.x as f32 + (tile.x as f32 + b.x1) / scale,
                    y1: region.y as f32 + (tile.y as f32 + b.y1) / scale,
                    score: b.score,
                });
            }
        }
        let mut boxes = det::merge_boxes(boxes);
        det::reading_order(&mut boxes);
        let prepare_ms = prepare.as_millis().min(u128::from(u32::MAX)) as u32;
        Ok(DetPass { boxes, prepare_ms, detect_ms: elapsed_ms(started).saturating_sub(prepare_ms) })
    }

    fn recognize<F: Frame + ?Sized>(&self, frame: &F, boxes: &[TextBox]) -> Result<Vec<TextLine>> {
        let rects: Vec<Rect> = boxes
            .iter()
            .filter_map(|b| b.to_rect(frame.width(), frame.height()))
            .filter(|r| r.height >= 4 && r.width >= 4)
            .collect();
        let per_thread = rects.len().div_ceil(PREP_THREADS).max(1);
        let lines: Vec<Result<Vec<PendingLine>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = rects
                .chunks(per_thread)
                .map(|chunk| {
                    scope.spawn(move || chunk.iter().map(|&r| self.submit_line(frame, r)).collect())
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().map_err(|_| anyhow!("line preparation panicked"))?)
                .collect()
        });

        let mut out = Vec::with_capacity(rects.len());
        for line in lines.into_iter().collect::<Result<Vec<_>>>()?.into_iter().flatten() {
            let mut text = String::new();
            let (mut weighted, mut total) = (0.0f32, 0usize);
            for (seg, rx) in line.pieces {
                let (piece, score) = rx.recv().context("NPU worker stopped")??;
                if piece.is_empty() {
                    continue;
                }
                if seg.space_before && !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&piece);
                let n = piece.chars().count();
                weighted += score * n as f32;
                total += n;
            }
            if total > 0 {
                out.push(TextLine { text, score: weighted / total as f32, rect: line.rect });
            }
        }
        Ok(out)
    }

    /// Cuts one text line into recognizer inputs and queues them.
    fn submit_line<F: Frame + ?Sized>(&self, frame: &F, rect: Rect) -> Result<PendingLine> {
        let rec_h = self.rec_height as usize;
        let widths: Vec<u32> = self.shared.recs.iter().map(|m| m.width).collect();
        let max_width = widths.last().copied().unwrap_or(0) as usize;
        let crop_h = rect.height as usize;
        let max_crop_w = (max_width * crop_h / rec_h).max(1);
        let blank = blank_columns(frame, rect);
        let mut pieces = Vec::new();
        for seg in rec::split_line(&blank, max_crop_w, crop_h.div_ceil(4)) {
            let crop_w = seg.end - seg.start;
            let needed = (crop_w * rec_h).div_ceil(crop_h).max(8) as u32;
            let Some(width) = rec::pick_width(&widths, needed) else { continue };
            let Some(model) = widths.iter().position(|&w| w == width) else { continue };
            let mut input = vec![REC_PAD; width as usize * rec_h * 3];
            let src = RectF {
                x0: (rect.x as usize + seg.start) as f32,
                y0: rect.y as f32,
                x1: (rect.x as usize + seg.end) as f32,
                y1: rect.bottom() as f32,
            };
            sample_bgr(frame, src, needed.min(width) as usize, rec_h, &mut input, width as usize);
            let (reply, rx) = mpsc::channel();
            self.queue.push(Job::Rec { model, input, reply })?;
            pieces.push((seg, rx));
        }
        Ok(PendingLine { rect, pieces })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.queue.close();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

struct DetPass {
    boxes: Vec<TextBox>,
    prepare_ms: u32,
    detect_ms: u32,
}

struct PendingLine {
    rect: Rect,
    pieces: Vec<(Segment, Receiver<Result<Piece>>)>,
}

fn median_height(boxes: &[TextBox]) -> Option<f32> {
    let mut heights: Vec<f32> = boxes.iter().map(TextBox::height).collect();
    heights.sort_by(f32::total_cmp);
    heights.get(heights.len() / 2).copied()
}

fn elapsed_ms(since: Instant) -> u32 {
    since.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}
