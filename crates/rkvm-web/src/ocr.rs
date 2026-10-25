use rkvm_proto::vision::{Detail, OcrResult};
use serde_json::json;

use crate::i18n::{I18n, Lang};
use crate::rpc::RpcClient;

/// What the OCR dialog shows. Device replies are kept as data and described
/// at render time, so they follow a language switch.
#[derive(Clone, Debug, PartialEq)]
pub enum OcrView {
    /// Waiting for the device; `first` means the models may still be loading.
    Reading {
        first: bool,
    },
    Done {
        text: String,
        lines: usize,
        seconds: f64,
        full: bool,
        cached: bool,
    },
    /// The device has no OCR models installed.
    NotInstalled,
    /// The device answered with something that is not an OCR result.
    Malformed(String),
    Failed(String),
}

impl OcrView {
    pub fn status(&self, i: I18n) -> String {
        match self {
            OcrView::Reading { first: true } => i
                .t(
                    "正在识别屏幕文字… 首次识别要先加载模型，会多等几秒。",
                    "Reading text from the screen… The first read loads the models first and takes a few seconds longer.",
                )
                .to_string(),
            OcrView::Reading { first: false } => {
                i.t("正在识别屏幕文字…", "Reading text from the screen…").to_string()
            }
            OcrView::Done { lines: 0, .. } => {
                i.t("屏幕上没有识别到文字。", "No text found on the screen.").to_string()
            }
            OcrView::Done { lines, seconds, full, cached, .. } => {
                let (unit, full_note, cached_note) = match i.lang() {
                    Lang::Zh => ("行", " · 全分辨率", " · 画面未变，沿用上次结果"),
                    Lang::En => ("lines", " · full resolution", " · screen unchanged, last result"),
                };
                let seconds = match i.lang() {
                    Lang::Zh => format!("{seconds:.1} 秒"),
                    Lang::En => format!("{seconds:.1} s"),
                };
                format!(
                    "{lines} {unit} · {seconds}{}{}",
                    if *full { full_note } else { "" },
                    if *cached { cached_note } else { "" }
                )
            }
            OcrView::NotInstalled => i
                .t(
                    "设备上没有安装 OCR 模型。请按 docs/vision.md 的“安装模型”一节把模型复制到 /userdata/rustkvm/vision/。",
                    "The device has no OCR models. Copy them to /userdata/rustkvm/vision/ as described under \"Installing the models\" in docs/vision.md.",
                )
                .to_string(),
            OcrView::Malformed(e) => {
                format!("{}{e}", i.t("设备返回的识别结果无法解析：", "The device sent an unreadable OCR result: "))
            }
            OcrView::Failed(e) => format!("{}{e}", i.t("无法识别文字：", "Could not read text: ")),
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            OcrView::Done { text, lines, .. } if *lines > 0 => Some(text),
            _ => None,
        }
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, OcrView::Reading { .. })
    }
}

/// Reads the screen text on the device (NPU OCR, `readScreenText`), one
/// recognized line per output line in reading order.
pub async fn recognize(client: &RpcClient) -> OcrView {
    let started = js_sys::Date::now();
    // The first call loads the models, and a busy 4K screen takes a few
    // seconds on the NPU; a retry would only queue a second full pass.
    let reply = match client.call_with("readScreenText", json!({}), 20_000, 1).await {
        Ok(reply) => reply,
        Err(e) if e.message.contains("OCR engine unavailable") => return OcrView::NotInstalled,
        Err(e) => return OcrView::Failed(e.to_string()),
    };
    let seconds = (js_sys::Date::now() - started) / 1000.0;
    match serde_json::from_value::<OcrResult>(reply) {
        Ok(result) => OcrView::Done {
            lines: result.lines.len(),
            text: result.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            seconds,
            full: result.detail == Detail::Full,
            cached: result.cached,
        },
        Err(e) => OcrView::Malformed(e.to_string()),
    }
}
