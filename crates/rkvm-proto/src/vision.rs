//! Screen OCR request and result types (`readScreenText` and friends).

use serde::{Deserialize, Serialize};

/// How much of the screen's native resolution text detection sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// `fast`, then `full` when most of the text it found is too small for
    /// the scaled-down pass to read reliably.
    #[default]
    Auto,
    /// Detect on the screen scaled to about 1080p.
    Fast,
    /// Detect at native resolution in overlapping tiles; finds small text on
    /// 4K screens at a few times the cost.
    Full,
}

/// A pixel rectangle in video frame coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn right(&self) -> u32 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(&self) -> u32 {
        self.y.saturating_add(self.height)
    }

    pub fn center(&self) -> (u32, u32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    /// The part of the rectangle inside a `width`×`height` frame, if any.
    pub fn clamp_to(&self, width: u32, height: u32) -> Option<Rect> {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let right = self.right().min(width);
        let bottom = self.bottom().min(height);
        (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OcrOptions {
    pub detail: Detail,
    /// Only read text inside this part of the frame.
    pub region: Option<Rect>,
    /// Drop lines whose recognition confidence is below this (0..1).
    pub min_score: Option<f32>,
    /// Wait until the screen has not changed for this long before reading
    /// (applied by the device before it takes the frame).
    pub wait_stable_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextLine {
    pub text: String,
    pub score: f32,
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    /// Turning the frame into detector input.
    pub prepare_ms: u32,
    pub detect_ms: u32,
    pub recognize_ms: u32,
    pub total_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub width: u32,
    pub height: u32,
    /// Lines in reading order: top to bottom, then left to right.
    pub lines: Vec<TextLine>,
    pub timings: Timings,
    /// The detection pass that produced the lines: `fast` or `full`.
    #[serde(default = "detail_fast")]
    pub detail: Detail,
    /// The screen had not changed since the last read with the same options,
    /// and these are that read's lines.
    #[serde(default)]
    pub cached: bool,
}

fn detail_fast() -> Detail {
    Detail::Fast
}
