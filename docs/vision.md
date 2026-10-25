# On-device screen OCR (RK3588 NPU)

`rustkvm_app` can read the text on the captured screen with the RK3588 NPU,
so an agent gets text and click coordinates instead of sending a screenshot
to the cloud. The engine lives in `crates/rkvm-vision` and runs inside the
app process.

## What it does

| RPC | MCP tool | Result |
| --- | --- | --- |
| `readScreenText` | `read_screen_text` | Text lines in reading order, each with a confidence and a pixel rectangle. The MCP tool returns one `x,y,width,height: text` line per text line. |
| `findScreenText` | `find_text` | Lines containing the given text (case and whitespace ignored) with `x`/`y` to click, in video pixels like `mouse_click`. |
| `waitScreenStable` | `wait_until_stable` | Waits until the screen stops changing (32×18 luma grid, 200 ms polling). Needs no models. |

Options for the OCR calls: `detail` is `fast` (default: the screen is scaled
to fit one 1920×1088 detector pass) or `full` (native resolution in
overlapping tiles, for small text on 4K screens); `region` limits reading to
part of the screen; `minScore` (default 0.5) drops unsure lines.

The models load on the first OCR call, one copy per NPU core, and stay
loaded. If anything is missing the call fails with an error naming the file;
the rest of the app is unaffected.

## Pipeline

1. A raw NV12 frame is taken from the capture pipeline before the encoder
   valve (the same hook as the JPEG screenshot), mapped read-only and
   converted to RGB.
2. **Detection** (PP-OCRv4 DB, INT8, 1088×1920 input). Tiles overlap by 96
   px; boxes from neighbouring tiles that share a line and overlap are
   merged, so a line cut by a tile edge comes back whole.
3. **Recognition** (PP-OCRv4 CTC, FP16, 48 px high). Three model widths (320,
   640, 1280) are loaded; each line uses the narrowest that fits. Lines
   longer than 1280 at 48 px are split at blank columns, never through a
   character, and rejoined with a space where the gap is word-sized. The
   space class of the model is decoded, so English keeps its spaces.
4. Jobs are spread over three worker threads, each owning the models on one
   NPU core. Measured on the device: detection about 20 ms per 480×480 tile,
   recognition about 14 ms per line per core, about 146 lines/s on three
   cores.

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
`det.rknn`, `rec_320.rknn`, `rec_640.rknn`, `rec_1280.rknn`,
`ppocr_keys_v1.txt`, `librknnrt.so` and `LICENSE.librknnrt` (about 40 MB).

To try models without the app, cross-compile the example and run it on the
device: `cargo +stage2 build -Z build-std=std,panic_abort --target
aarch64-unknown-linux-gnu --release -p rkvm-vision --example ocr_image`, then
`ocr_image /userdata/rustkvm/vision [--full] screen.png`.

## Runtime and kernel

- **NPU driver**: the FriendlyELEC `nanopi6-v6.1.y` kernel builds `rknpu`
  0.9.8 as a module and the firmware loads it at boot (`/dev/dri/renderD129`).
  No kernel change is needed.
- **librknnrt**: loaded at run time with `dlopen` through the
  [`rknpu2`](https://crates.io/crates/rknpu2) crate (`libloading` feature),
  from `/userdata/rustkvm/vision/librknnrt.so` or `/usr/lib/librknnrt.so`.
  Nothing links against it at build time, and it is not stored in this
  repository.

## Licences

| Component | Source | Licence |
| --- | --- | --- |
| `librknnrt.so` 2.5.0 | [airockchip/rknn-toolkit2](https://github.com/airockchip/rknn-toolkit2) `rknpu2/runtime/Linux/librknn_api/aarch64` | Rockchip **RKNN SDK License**: closed-source binary, royalty-free use and redistribution for products built on Rockchip chips, no reverse engineering. Shipped next to the models with its licence text, never inside the app binary. |
| `rknn-toolkit2` 2.5.0 | PyPI | Same RKNN SDK License; used only on the build host to convert models. |
| PP-OCRv4 models and dictionary | PaddleOCR via rknn_model_zoo | Apache-2.0 |
| `rknpu2`, `rknpu2-sys` crates | crates.io | MIT OR Apache-2.0 |
| RKNPU kernel driver | Rockchip / FriendlyELEC kernel | GPL-2.0 |
