# On-device screen OCR (RK3588 NPU)

`rustkvm_app` can read the text on the captured screen with the RK3588 NPU,
so an agent gets text and click coordinates instead of sending a screenshot
to the cloud. The engine lives in `crates/rkvm-vision` and runs inside the
app process.

## What it does

| RPC | MCP tool | Result |
| --- | --- | --- |
| `readScreenText` | `read_screen_text` | Text lines in reading order, each with a confidence and a pixel rectangle, plus `detail` (the pass used: `fast` or `full`), `cached` and `timings`. The MCP tool returns one `x,y,width,height: text` line per text line. |
| `findScreenText` | `find_text` | Lines containing the given text (case and whitespace ignored) with `x`/`y` to click, in video pixels like `mouse_click`. |
| `waitForText` | `wait_for_text` | Reads the screen every `intervalMs` (default 500) until the text shows up or `timeoutMs` (default 10 s) passes; returns `found`, `waitedMs`, `reads` and the matches with click positions. |
| `waitScreenStable` | `wait_until_stable` | Waits until the screen stops changing (32×18 luma grid, 200 ms polling). Needs no models. |

Options for the OCR calls:

- `detail`: `auto` (default) reads the screen scaled to about 1080p and, when
  most of the text it found is small (median box under 15.5 px at that
  scale, i.e. 4K text of about 14 px or less), reads again at native
  resolution. `fast` never re-reads; `full` always reads at native
  resolution in overlapping tiles.
- `region` limits reading to part of the screen; `minScore` (default 0.5)
  drops unsure lines; `waitStableMs` first waits until the screen has not
  changed for that long.

A read whose region (or the whole screen) is pixel-identical to the last
read's, with the same options, returns that read again with `cached: true`
in about 50 ms, so polling an unchanged screen (`waitForText`) costs a frame
copy, not an NPU pass.

The models load on the first OCR call (about 0.5 s) and stay loaded. If
anything is missing the call fails with "OCR engine unavailable (see
docs/vision.md)" and the web dialog says how to install them; the rest of
the app is unaffected.

## Pipeline

1. A raw NV12 frame is taken from the capture pipeline before the encoder
   valve (the same hook as the JPEG screenshot). The capture buffer is
   uncached DMA memory, so it is copied once, row by row, into a reused
   buffer (about 40 ms for 4K) and read there as NV12: only the pixels the
   models need are converted, and resampling large patches runs on four
   threads. Sampling straight from the mapped buffer took over 200 ms, and
   the old full-frame RGB conversion about 1 core-second per call.
2. **Detection** (PP-OCRv4 DB, INT8). The engine loads every
   `det_<width>x<height>.rknn` and picks, per call, the shape that covers the
   area soonest on three cores (tiles run in rounds of three): a 1080p
   screen is three overlapping 1920×448 strips, one per core; a 4K screen at
   full resolution 18 of them; small regions use 640×224 tiles. Tiles
   overlap by 96 px; boxes from neighbouring tiles that share a line are
   merged. A lone legacy `det.rknn` still works.
3. **Recognition** (PP-OCRv4 CTC, FP16, 48 px high). Three model widths (320,
   640, 1280) are loaded; each line uses the narrowest that fits. Lines
   longer than 1280 at 48 px are split at blank columns, never through a
   character, and rejoined with a space where the gap is word-sized. The
   models are converted with a (1, T, 1, C) output, so all 132 layers run on
   the NPU (the 3D output made rknn-toolkit2 add a CPU Reshape, 5% of the
   time), and the FP16 output is decoded from its raw bits.
4. Jobs go through one queue to six workers, two per NPU core: one runs
   detection and recognition, the other recognition only, so one context
   copies its input or output while the other runs (+17-33% throughput per
   core). The models of a worker share one activation buffer
   (`rknn_set_internal_mem`); CTC decoding runs on the workers.

Measured on a NanoPC-T6, 3840×2160 desktop, 2026-10-25 (before → after
this pipeline; the live screen had 55-70 lines):

| | before | after |
| --- | --- | --- |
| `fast`, whole screen (device / end to end) | 1.65 / 1.84 s | 0.96 / 0.96 s |
| `full`, whole screen | 2.35 / 2.52 s | 1.57 / 1.55 s |
| 1600×200 region | 0.24 / 0.50 s | 0.15 / 0.16 s |
| unchanged region, cached | — | 0.05 s |
| NPU load per core, continuous `full` | 67-69% | 87-88% |
| NPU memory (RssShmem) / app RSS | 1287 MB / 1.80 GB | 678 MB / 0.80 GB |

Offline, on four saved 4K desktop frames (59-232 lines), `fast` takes
0.61-1.57 s and `full` 1.10-2.12 s (before: 0.87-1.99 s and 1.56-2.75 s).
Recognition is now about 80% of a read, at about 87% NPU load. Video stays
at 60 fps during continuous OCR.

Accuracy on synthetic 1080p/4K screens with known text (308 lines, Chinese,
English, mixed and code, 10-28 px, light, dark and low contrast): character
error rate 0.9% for `fast`, 0.4% for `full` and `auto`, every line found.
Typical errors: `|` → `I` and `O` → `0` in code, lost word spaces in small
English text at `fast`. PP-OCRv4 reads 启 as 后 in the Source Han / Noto CJK
typeface (the ONNX model does the same on the CPU, with 后 at 0.96); in
Microsoft YaHei, SimSun, SimHei and DengXian it reads 启 correctly.

## Installing the models

```bash
./scripts/vision-models.sh                 # x86_64 Linux, Python 3.10-3.12
ssh root@<device> mkdir -p /userdata/rustkvm/vision
scp -O out/vision/* root@<device>:/userdata/rustkvm/vision/
```

The script downloads the PP-OCRv4 ONNX models and dictionary from Rockchip's
[rknn_model_zoo](https://github.com/airockchip/rknn_model_zoo) (checksums
pinned), converts them with `rknn-toolkit2` 2.5.0 from PyPI, and fetches
`librknnrt.so` 2.5.0. The device's `/userdata/rustkvm/vision/` then holds
`det_1920x448.rknn` (3.2 MB), `det_640x224.rknn` (2.5 MB), `rec_320.rknn`,
`rec_640.rknn`, `rec_1280.rknn` (7.0-7.2 MB each), `ppocr_keys_v1.txt`,
`librknnrt.so` (8.2 MB) and `LICENSE.librknnrt`, about 34 MB. Remove a
`det.rknn` left from an older bundle: it is only used when no
`det_<w>x<h>.rknn` is present.

To try models without the app, cross-compile the example and run it on the
device: `cargo +stage2 build -Z build-std=std,panic_abort --target
aarch64-unknown-linux-gnu --release -p rkvm-vision --example ocr_image`, then
`ocr_image /userdata/rustkvm/vision [--fast|--full] [--nv12] screen.png`
(`--nv12` reads the image through the same NV12 path as captured frames).

## Runtime and kernel

- **NPU driver**: the FriendlyELEC `nanopi6-v6.1.y` kernel builds `rknpu`
  0.9.8 as a module and the firmware loads it at boot (`/dev/dri/renderD129`).
  No kernel change is needed.
- **librknnrt**: loaded at run time with `dlopen` by
  `crates/rkvm-vision/src/rknn.rs` (C types from
  [`rknpu2-sys`](https://crates.io/crates/rknpu2-sys)), from
  `/userdata/rustkvm/vision/librknnrt.so` or `/usr/lib/librknnrt.so`.
  Nothing links against it at build time, and it is not stored in this
  repository. `rknn_dup_context` is not used: in 2.5.0 duplicates of one
  model that run at the same time overwrite each other's outputs, so every
  worker loads its own contexts (weights are 1.3-5.4 MB per model).

## Licences

| Component | Source | Licence |
| --- | --- | --- |
| `librknnrt.so` 2.5.0 | [airockchip/rknn-toolkit2](https://github.com/airockchip/rknn-toolkit2) `rknpu2/runtime/Linux/librknn_api/aarch64` | Rockchip **RKNN SDK License**: closed-source binary, royalty-free use and redistribution for products built on Rockchip chips, no reverse engineering. Shipped next to the models with its licence text, never inside the app binary. |
| `rknn-toolkit2` 2.5.0 | PyPI | Same RKNN SDK License; used only on the build host to convert models. |
| PP-OCRv4 models and dictionary | PaddleOCR via rknn_model_zoo | Apache-2.0 |
| `rknpu2-sys` crate (C types) | crates.io | MIT OR Apache-2.0 |
| RKNPU kernel driver | Rockchip / FriendlyELEC kernel | GPL-2.0 |
