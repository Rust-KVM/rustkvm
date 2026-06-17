use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlVideoElement};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = rkvmOcr, catch)]
    fn rkvm_ocr(canvas: &HtmlCanvasElement) -> Result<js_sys::Promise, wasm_bindgen::JsValue>;
}

pub async fn recognize(video: &HtmlVideoElement) -> Result<String, String> {
    let (w, h) = (video.video_width(), video.video_height());
    if w == 0 || h == 0 {
        return Err("no video frame to scan".to_string());
    }

    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let canvas = document
        .create_element("canvas")
        .map_err(|e| format!("create canvas: {e:?}"))?
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| "not a canvas".to_string())?;
    canvas.set_width(w);
    canvas.set_height(h);

    let ctx = canvas
        .get_context("2d")
        .map_err(|e| format!("get 2d context: {e:?}"))?
        .ok_or("no 2d context")?
        .dyn_into::<CanvasRenderingContext2d>()
        .map_err(|_| "bad 2d context".to_string())?;
    ctx.draw_image_with_html_video_element_and_dw_and_dh(video, 0.0, 0.0, w as f64, h as f64)
        .map_err(|e| format!("draw frame: {e:?}"))?;

    let promise = rkvm_ocr(&canvas).map_err(|e| format!("OCR unavailable: {e:?}"))?;
    let text = JsFuture::from(promise).await.map_err(|e| format!("OCR failed: {e:?}"))?;
    Ok(text.as_string().unwrap_or_default())
}
