//! Screen OCR on the RK3588 NPU: PP-OCRv4 text detection and recognition run
//! through Rockchip's `librknnrt`, which is loaded at run time so the app
//! builds and runs without it.

pub mod det;
pub mod image;
pub mod npu;
pub mod rec;
pub mod signature;
pub mod types;

pub use image::RgbImage;
pub use npu::Engine;
pub use types::{Detail, OcrOptions, OcrResult, Rect, TextLine, Timings};
