use rustkvm::api::{JsonRpcProcessor, JsonRpcResponse, default_registry};
use serde_json::{Value, json};

async fn call(method: &str, params: Value) -> JsonRpcResponse {
    let request = json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1});
    JsonRpcProcessor::new(default_registry()).dispatch(&serde_json::to_vec(&request).unwrap()).await
}

fn error_detail(resp: JsonRpcResponse) -> String {
    let error = resp.error.expect("call must fail");
    assert_eq!(error.code, -32603);
    error.data.and_then(|d| d.as_str().map(str::to_owned)).unwrap_or_default()
}

#[tokio::test]
async fn type_text_rejects_untypeable_text_before_touching_hid() {
    let detail = error_detail(call("typeText", json!({"text": "naïve"})).await);
    assert!(detail.contains("unsupported character"), "{detail}");
}

#[tokio::test]
async fn type_text_rejects_oversized_text() {
    let text = "a".repeat(10_001);
    let detail = error_detail(call("typeText", json!({"text": text})).await);
    assert!(detail.contains("too long"), "{detail}");
}

#[tokio::test]
async fn type_text_without_usb_reports_hid_unavailable() {
    let detail = error_detail(call("typeText", json!({"text": "hello"})).await);
    assert!(detail.contains("USB HID not initialized"), "{detail}");
}

#[tokio::test]
async fn press_combo_rejects_unknown_keys() {
    let detail = error_detail(call("pressCombo", json!({"combo": "ctrl+banana"})).await);
    assert!(detail.contains("unknown key"), "{detail}");
}

#[tokio::test]
async fn mouse_click_validates_button_and_position() {
    let detail = error_detail(
        call("mouseClick", json!({"x": 0.5, "y": 0.5, "normalized": true, "button": "side"})).await,
    );
    assert!(detail.contains("unknown mouse button"), "{detail}");

    let detail =
        error_detail(call("mouseClick", json!({"x": 1.5, "y": 0.5, "normalized": true})).await);
    assert!(detail.contains("outside the screen"), "{detail}");
}

#[tokio::test]
async fn pixel_coordinates_need_a_video_signal() {
    let detail = error_detail(call("mouseMove", json!({"x": 100, "y": 100})).await);
    assert!(detail.contains("video input not ready"), "{detail}");
}

#[tokio::test]
async fn execute_keyboard_macro_validates_key_names() {
    let detail = error_detail(
        call(
            "executeKeyboardMacro",
            json!([{"keys": ["KeyQ"], "modifiers": ["Hyper"], "delay": 100}]),
        )
        .await,
    );
    assert!(detail.contains("unknown key"), "{detail}");
}
