//! The OCR engine: PP-OCRv4 models on the three RK3588 NPU cores.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use rknpu2::RKNN;
use rknpu2::api::RknnInitFlags;
use rknpu2::api::runtime::RuntimeAPI;
use rknpu2::io::buffer::{BufMutView, BufView};
use rknpu2::io::input::Input;
use rknpu2::io::output::{Output, OutputKind};
use rknpu2::query::{InputAttr, OutputAttr, SdkVersion, TensorAttrView};
use rknpu2::rknn::NpuCores;
use rknpu2::tensor::{TensorFormat, TensorFormatKind};
use tracing::info;

use crate::det::{self, DbParams, TextBox};
use crate::image::{RectF, RgbImage};
use crate::rec;
use crate::types::{Detail, OcrOptions, OcrResult, Rect, TextLine, Timings};

/// Where the runtime is looked for when the model directory has none.
pub const SYSTEM_RUNTIME: &str = "/usr/lib/librknnrt.so";
pub const RUNTIME_FILE: &str = "librknnrt.so";
pub const DET_MODEL_FILE: &str = "det.rknn";
pub const DICT_FILE: &str = "ppocr_keys_v1.txt";

const TILE_OVERLAP: u32 = 96;
/// Narrower detectors miss body text on desktop screens; the zoo's 480x480
/// model is what a conversion that kept the ONNX file's shape produces.
const MIN_DET_WIDTH: u32 = 1280;
const DEFAULT_MIN_SCORE: f32 = 0.5;
/// PP-OCR pads recognizer input with zeros after normalising to -1..1; the
/// models are converted with mean and std 127.5, so raw 128 is that zero.
const REC_PAD: u8 = 128;

/// Versions and model shapes, for logs and health reports.
#[derive(Clone, Debug)]
pub struct EngineInfo {
    pub runtime_path: PathBuf,
    pub api_version: String,
    pub driver_version: String,
    pub det_input: (u32, u32),
    pub rec_widths: Vec<u32>,
}

struct Net {
    rknn: RKNN<RuntimeAPI>,
    in_h: u32,
    in_w: u32,
    out_dims: Vec<u32>,
    out_len: usize,
}

impl Net {
    fn load(runtime: &Path, model: &Path, cores: NpuCores) -> Result<Self> {
        let mut data = std::fs::read(model).with_context(|| format!("read {}", model.display()))?;
        let rknn = RKNN::new_with_library(runtime, &mut data, RknnInitFlags::empty())
            .map_err(|e| anyhow!("rknn_init {}: {e:?}", model.display()))?;
        rknn.set_core_mask(cores).map_err(|e| anyhow!("set core mask: {e:?}"))?;
        let input =
            rknn.query_with_input::<InputAttr>(0).map_err(|e| anyhow!("query input: {e:?}"))?;
        let output =
            rknn.query_with_input::<OutputAttr>(0).map_err(|e| anyhow!("query output: {e:?}"))?;
        let (in_h, in_w, channels) = match (input.format(), input.dims()) {
            (TensorFormatKind::NCHW(_), &[_, c, h, w]) => (h, w, c),
            (_, &[_, h, w, c]) => (h, w, c),
            (_, dims) => bail!("{}: unexpected input shape {dims:?}", model.display()),
        };
        if channels != 3 {
            bail!("{}: expected 3 input channels, got {channels}", model.display());
        }
        Ok(Self {
            rknn,
            in_h,
            in_w,
            out_dims: output.dims().to_vec(),
            out_len: output.num_elements() as usize,
        })
    }

    /// Runs one BGR `in_h`×`in_w` image (NHWC, u8) and returns the first
    /// output as f32.
    fn run(&self, input: &[u8]) -> Result<Vec<f32>> {
        let mut out = vec![0f32; self.out_len];
        self.rknn
            .set_inputs(Input::new(
                0,
                BufView::U8(input),
                false,
                TensorFormatKind::NHWC(TensorFormat::NHWC),
            ))
            .map_err(|e| anyhow!("rknn_inputs_set: {e:?}"))?;
        self.rknn.run().map_err(|e| anyhow!("rknn_run: {e:?}"))?;
        let mut outputs = [Output {
            index: 0,
            kind: OutputKind::Preallocated { buf: BufMutView::F32(&mut out), want_float: true },
        }];
        self.rknn.get_outputs(&mut outputs).map_err(|e| anyhow!("rknn_outputs_get: {e:?}"))?;
        Ok(out)
    }
}

enum Job {
    Det { input: Vec<u8>, reply: Sender<Result<Vec<f32>>> },
    Rec { model: usize, input: Vec<u8>, reply: Sender<Result<Vec<f32>>> },
}

struct RecModel {
    width: u32,
    steps: usize,
    classes: usize,
}

/// Loaded PP-OCR models, one copy per NPU core, each driven by its own
/// thread. `ocr` can be called from several threads at once.
pub struct Engine {
    jobs: Sender<Job>,
    info: EngineInfo,
    det_params: DbParams,
    rec_height: u32,
    rec_models: Vec<RecModel>,
    dict: Vec<String>,
}

impl Engine {
    /// Loads `det.rknn`, every `rec_<width>.rknn` and the dictionary from
    /// `dir`, with `librknnrt.so` from `dir` or the system library path.
    pub fn load(dir: &Path) -> Result<Self> {
        let runtime = [dir.join(RUNTIME_FILE), PathBuf::from(SYSTEM_RUNTIME)]
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| {
                anyhow!("{RUNTIME_FILE} not found in {} or {SYSTEM_RUNTIME}", dir.display())
            })?;
        // rknpu2 unwraps its own dlopen, and the app aborts on panic: open
        // the library once here so a bad file comes back as an error.
        // SAFETY: librknnrt runs no initialisers with preconditions on load.
        drop(
            unsafe { libloading::Library::new(&runtime) }
                .with_context(|| format!("load {}", runtime.display()))?,
        );

        let det_path = dir.join(DET_MODEL_FILE);
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
        let dict_text = std::fs::read_to_string(dir.join(DICT_FILE))
            .with_context(|| format!("read {DICT_FILE}"))?;
        let dict = rec::load_dict(&dict_text);

        let cores = [NpuCores::CORE0, NpuCores::CORE1, NpuCores::CORE2];
        let mut workers = Vec::with_capacity(cores.len());
        for core in cores {
            let det = Net::load(&runtime, &det_path, core)?;
            if det.in_w < MIN_DET_WIDTH {
                bail!(
                    "{DET_MODEL_FILE} takes {}x{}, narrower than {MIN_DET_WIDTH} px; \
                     reconvert it with scripts/vision-models.sh",
                    det.in_w,
                    det.in_h
                );
            }
            let mut recs = rec_paths
                .iter()
                .map(|p| {
                    let net = Net::load(&runtime, p, core)?;
                    let named = p
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_prefix("rec_"))
                        .and_then(|w| w.parse::<u32>().ok());
                    if named != Some(net.in_w) {
                        bail!(
                            "{} takes input width {}, which its name does not match; \
                             reconvert it with scripts/vision-models.sh",
                            p.display(),
                            net.in_w
                        );
                    }
                    Ok(net)
                })
                .collect::<Result<Vec<_>>>()?;
            recs.sort_by_key(|n| n.in_w);
            workers.push((det, recs));
        }

        let (det, recs) = &workers[0];
        let version =
            det.rknn.query::<SdkVersion>().map_err(|e| anyhow!("query sdk version: {e:?}"))?;
        let rec_height = recs[0].in_h;
        let mut rec_models = Vec::with_capacity(recs.len());
        for r in recs {
            if r.in_h != rec_height {
                bail!("recognition models disagree on input height ({} vs {rec_height})", r.in_h);
            }
            let (steps, classes) = match r.out_dims.as_slice() {
                &[_, t, c] => (t as usize, c as usize),
                dims => bail!("unexpected recognition output shape {dims:?}"),
            };
            if classes < dict.len() + 1 {
                bail!(
                    "dictionary has {} characters but the model only {classes} classes",
                    dict.len()
                );
            }
            rec_models.push(RecModel { width: r.in_w, steps, classes });
        }
        let info = EngineInfo {
            runtime_path: runtime,
            api_version: version.api_version(),
            driver_version: version.driver_version(),
            det_input: (det.in_w, det.in_h),
            rec_widths: rec_models.iter().map(|m| m.width).collect(),
        };

        let (jobs, rx) = mpsc::channel();
        let rx = Arc::new(Mutex::new(rx));
        for (index, (det, recs)) in workers.into_iter().enumerate() {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("npu-core{index}"))
                .spawn(move || worker(&rx, &det, &recs))
                .context("spawn NPU worker")?;
        }
        info!(
            runtime = %info.runtime_path.display(),
            api = %info.api_version,
            driver = %info.driver_version,
            det_input = ?info.det_input,
            rec_widths = ?info.rec_widths,
            "OCR engine loaded"
        );
        Ok(Self { jobs, info, det_params: DbParams::default(), rec_height, rec_models, dict })
    }

    pub fn info(&self) -> &EngineInfo {
        &self.info
    }

    pub fn ocr(&self, image: &RgbImage, options: &OcrOptions) -> Result<OcrResult> {
        let started = Instant::now();
        let (width, height) = (image.width(), image.height());
        let full = Rect::new(0, 0, width, height);
        let region = options
            .region
            .map_or(Some(full), |r| r.clamp_to(width, height))
            .context("region is outside the frame")?;

        let detect_started = Instant::now();
        let boxes = self.detect(image, region, options.detail)?;
        let detect_ms = elapsed_ms(detect_started);

        let recognize_started = Instant::now();
        let min_score = options.min_score.unwrap_or(DEFAULT_MIN_SCORE);
        let lines = self
            .recognize(image, &boxes)?
            .into_iter()
            .filter(|l| !l.text.is_empty() && l.score >= min_score)
            .collect();
        let recognize_ms = elapsed_ms(recognize_started);

        let total_ms = elapsed_ms(started);
        Ok(OcrResult {
            width,
            height,
            lines,
            timings: Timings {
                prepare_ms: total_ms.saturating_sub(detect_ms + recognize_ms),
                detect_ms,
                recognize_ms,
                total_ms,
            },
        })
    }

    fn detect(&self, image: &RgbImage, region: Rect, detail: Detail) -> Result<Vec<TextBox>> {
        let (det_w, det_h) = self.info.det_input;
        let scale = match detail {
            Detail::Fast => (det_w as f32 / region.width as f32)
                .min(det_h as f32 / region.height as f32)
                .min(1.0),
            Detail::Full => 1.0,
        };
        let scaled_w = ((region.width as f32 * scale).round() as u32).max(1);
        let scaled_h = ((region.height as f32 * scale).round() as u32).max(1);
        let tiles = det::plan_tiles(scaled_w, scaled_h, det_w, det_h, TILE_OVERLAP);

        let mut pending = Vec::with_capacity(tiles.len());
        for tile in &tiles {
            let mut input = vec![0u8; det_w as usize * det_h as usize * 3];
            let src = RectF {
                x0: region.x as f32 + tile.x as f32 / scale,
                y0: region.y as f32 + tile.y as f32 / scale,
                x1: region.x as f32 + tile.right() as f32 / scale,
                y1: region.y as f32 + tile.bottom() as f32 / scale,
            };
            image.sample_bgr(
                src,
                tile.width as usize,
                tile.height as usize,
                &mut input,
                det_w as usize,
            );
            let (reply, rx) = mpsc::channel();
            self.submit(Job::Det { input, reply })?;
            pending.push((*tile, rx));
        }

        let mut boxes = Vec::new();
        for (tile, rx) in pending {
            let prob = rx.recv().context("NPU worker stopped")??;
            for b in det::db_boxes(&prob, det_w as usize, det_h as usize, &self.det_params) {
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
        Ok(boxes)
    }

    fn recognize(&self, image: &RgbImage, boxes: &[TextBox]) -> Result<Vec<TextLine>> {
        let rec_h = self.rec_height as usize;
        let widths: Vec<u32> = self.rec_models.iter().map(|m| m.width).collect();
        let max_width = widths.last().copied().unwrap_or(0) as usize;

        let mut lines = Vec::new();
        for b in boxes {
            let Some(rect) = b.to_rect(image.width(), image.height()) else { continue };
            if rect.height < 4 || rect.width < 4 {
                continue;
            }
            let crop_h = rect.height as usize;
            let max_crop_w = (max_width * crop_h / rec_h).max(1);
            let blank = image.blank_columns(rect);
            let mut pending = Vec::new();
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
                image.sample_bgr(
                    src,
                    needed.min(width) as usize,
                    rec_h,
                    &mut input,
                    width as usize,
                );
                let (reply, rx) = mpsc::channel();
                self.submit(Job::Rec { model, input, reply })?;
                pending.push((seg, model, rx));
            }
            lines.push((rect, pending));
        }

        let mut out = Vec::with_capacity(lines.len());
        for (rect, pending) in lines {
            let mut text = String::new();
            let (mut weighted, mut total) = (0.0f32, 0usize);
            for (seg, model, rx) in pending {
                let probs = rx.recv().context("NPU worker stopped")??;
                let m = &self.rec_models[model];
                let (piece, score) = rec::ctc_decode(&probs, m.steps, m.classes, &self.dict);
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
                out.push(TextLine { text, score: weighted / total as f32, rect });
            }
        }
        Ok(out)
    }

    fn submit(&self, job: Job) -> Result<()> {
        self.jobs.send(job).map_err(|_| anyhow!("NPU workers have stopped"))
    }
}

fn worker(rx: &Mutex<Receiver<Job>>, det: &Net, recs: &[Net]) {
    loop {
        let job = match rx.lock() {
            Ok(guard) => guard.recv(),
            Err(_) => return,
        };
        match job {
            Ok(Job::Det { input, reply }) => {
                let _ = reply.send(det.run(&input));
            }
            Ok(Job::Rec { model, input, reply }) => {
                let result = recs
                    .get(model)
                    .ok_or_else(|| anyhow!("no recognition model {model}"))
                    .and_then(|n| n.run(&input));
                let _ = reply.send(result);
            }
            Err(_) => return,
        }
    }
}

fn elapsed_ms(since: Instant) -> u32 {
    since.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}
