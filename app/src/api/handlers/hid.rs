use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use parking_lot::{Mutex, RwLock};
use rkvm_proto::keymap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use crate::hardware::usb as usb_mod;
use crate::hardware::usb::KeyboardState as HidKeyboardState;
use crate::hidrpc::{HID_KEY_BUFFER_SIZE, KeyboardMacroStep};

static KEYBOARD_LED: once_cell::sync::OnceCell<RwLock<HidKeyboardState>> =
    once_cell::sync::OnceCell::new();
static KEYBOARD_LAYOUT: OnceLock<Mutex<String>> = OnceLock::new();

pub fn get_keyboard_led_state() -> Result<HidKeyboardState> {
    let cell = KEYBOARD_LED.get_or_init(|| parking_lot::RwLock::new(HidKeyboardState::default()));
    Ok(*cell.read())
}

pub fn set_keyboard_led_state(state: HidKeyboardState) -> Result<Value> {
    let cell = KEYBOARD_LED.get_or_init(|| parking_lot::RwLock::new(HidKeyboardState::default()));
    *cell.write() = state;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct KeyboardReportParams {
    pub modifier: u8,
    pub keys: Vec<u8>,
}

pub fn keyboard_report(params: KeyboardReportParams) -> Result<Value> {
    let mgr = usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
    mgr.read()
        .hid()
        .keyboard_report(params.modifier, &params.keys)
        .map_err(|e| anyhow!("keyboard report failed: {}", e))?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct AbsMouseReportParams {
    pub x: i32,
    pub y: i32,
    pub buttons: u8,
}

pub fn abs_mouse_report(params: AbsMouseReportParams) -> Result<Value> {
    let mgr = usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
    mgr.read()
        .hid()
        .abs_mouse_report(params.x, params.y, params.buttons)
        .map_err(|e| anyhow!("abs mouse report failed: {}", e))?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct RelMouseReportParams {
    pub dx: i8,
    pub dy: i8,
    pub buttons: u8,
}

pub fn rel_mouse_report(params: RelMouseReportParams) -> Result<Value> {
    let mgr = usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
    mgr.read()
        .hid()
        .rel_mouse_report(params.dx, params.dy, params.buttons)
        .map_err(|e| anyhow!("rel mouse report failed: {}", e))?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct WheelReportParams {
    #[serde(rename = "wheelY")]
    pub wheel_y: i8,
}

pub fn wheel_report(params: WheelReportParams) -> Result<Value> {
    let mgr = usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
    mgr.read()
        .hid()
        .abs_mouse_wheel_report(params.wheel_y)
        .map_err(|e| anyhow!("wheel report failed: {}", e))?;
    Ok(Value::Null)
}

pub async fn wake_host() -> Result<Value> {
    let Some(mgr) = usb_mod::get_usb_manager() else {
        return Ok(Value::Null);
    };
    for _ in 0..3 {
        {
            let guard = mgr.read();
            guard
                .hid()
                .rel_mouse_report(1, 0, 0)
                .map_err(|e| anyhow!("wake host (nudge) failed: {}", e))?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        {
            let guard = mgr.read();
            guard
                .hid()
                .rel_mouse_report(-1, 0, 0)
                .map_err(|e| anyhow!("wake host (restore) failed: {}", e))?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    info!("Sent HID wake nudge to host");
    Ok(Value::Null)
}

#[derive(Serialize)]
pub struct KeyDownStateResponse {
    pub modifier: u8,
    pub keys: Vec<u8>,
}

pub fn get_key_down_state() -> Result<KeyDownStateResponse> {
    if let Some(mgr) = usb_mod::get_usb_manager() {
        let state = mgr.read().hid().get_keys_down_state();
        return Ok(KeyDownStateResponse { modifier: state.modifier, keys: state.keys.to_vec() });
    }
    Ok(KeyDownStateResponse { modifier: 0, keys: vec![0, 0, 0, 0, 0, 0] })
}

#[derive(Deserialize)]
pub struct KeypressReportParams {
    pub key: u8,
    pub press: bool,
}

pub fn keypress_report(params: KeypressReportParams) -> Result<Value> {
    let mgr = usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
    mgr.read()
        .hid()
        .keypress_report(params.key, params.press)
        .map_err(|e| anyhow!("keypress report failed: {}", e))?;
    Ok(Value::Null)
}

pub fn get_keyboard_layout() -> Result<String> {
    let layout_mutex = KEYBOARD_LAYOUT.get_or_init(|| Mutex::new("us".to_string()));
    let layout = layout_mutex.lock().clone();
    Ok(layout)
}

#[derive(Deserialize)]
pub struct KeyboardLayoutParams {
    pub layout: String,
}

pub fn set_keyboard_layout(params: KeyboardLayoutParams) -> Result<Value> {
    const VALID_LAYOUTS: &[&str] = &["us", "uk", "de", "fr", "es", "it", "jp", "kr"];
    if !VALID_LAYOUTS.contains(&params.layout.as_str()) {
        return Err(anyhow!("Unsupported keyboard layout: {}", params.layout));
    }

    let layout_mutex = KEYBOARD_LAYOUT.get_or_init(|| Mutex::new("us".to_string()));
    *layout_mutex.lock() = params.layout.clone();

    info!("Keyboard layout set to: {}", params.layout);
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct KeyboardMacrosParams {
    pub macros: Vec<serde_json::Value>,
}

pub async fn get_keyboard_macros() -> Result<Vec<crate::config::KeyboardMacro>> {
    let mgr = crate::config::get_config_manager();
    let cfg = mgr.get().await;
    Ok(cfg.keyboard_macros)
}

pub async fn set_keyboard_macros(params: KeyboardMacrosParams) -> Result<serde_json::Value> {
    if params.macros.len() > crate::config::types::MAX_MACROS_PER_DEVICE {
        anyhow::bail!("too many macros (max {})", crate::config::types::MAX_MACROS_PER_DEVICE);
    }

    let mut new_macros = Vec::with_capacity(params.macros.len());
    for (i, macro_value) in params.macros.into_iter().enumerate() {
        let macro_obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(macro_value)
                .map_err(|e| anyhow::anyhow!("invalid macro at index {}: {}", i, e))?;

        let id =
            macro_obj.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(
                || {
                    format!(
                        "macro-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or(0)
                    )
                },
            );

        let name = macro_obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();

        let sort_order = macro_obj
            .get("sortOrder")
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or((i + 1) as u32);

        let mut steps = Vec::new();
        if let Some(steps_array) = macro_obj.get("steps").and_then(|v| v.as_array()) {
            if steps_array.is_empty() {
                anyhow::bail!("macro at index {} must have at least one step", i);
            }
            for step_value in steps_array.iter() {
                let step_obj = match step_value.as_object() {
                    Some(obj) => obj,
                    None => continue,
                };

                let mut step = crate::config::KeyboardMacroStep {
                    keys: Vec::new(),
                    modifiers: Vec::new(),
                    delay: 0,
                };

                if let Some(keys_array) = step_obj.get("keys").and_then(|v| v.as_array()) {
                    for key_value in keys_array {
                        if let Some(key_str) = key_value.as_str() {
                            step.keys.push(key_str.to_string());
                        }
                    }
                }

                if let Some(mods_array) = step_obj.get("modifiers").and_then(|v| v.as_array()) {
                    for mod_value in mods_array {
                        if let Some(mod_str) = mod_value.as_str() {
                            step.modifiers.push(mod_str.to_string());
                        }
                    }
                }

                if let Some(delay_value) = step_obj.get("delay").and_then(|v| v.as_u64()) {
                    step.delay = delay_value as u32;
                }

                steps.push(step);
            }
        }

        let mut macro_item =
            crate::config::KeyboardMacro { id, name, steps, sort_order: Some(sort_order) };

        if let Err(e) = macro_item.validate() {
            anyhow::bail!("invalid macro at index {}: {}", i, e);
        }

        new_macros.push(macro_item);
    }

    let mgr = crate::config::get_config_manager();
    mgr.update(|cfg| {
        cfg.keyboard_macros = new_macros;
    })
    .await?;

    Ok(serde_json::Value::Null)
}

pub async fn execute_keyboard_macro(macro_steps: Vec<serde_json::Value>) -> Result<Value> {
    let mut steps = Vec::with_capacity(macro_steps.len() * 2);
    for (index, value) in macro_steps.into_iter().enumerate() {
        let mut step: crate::config::KeyboardMacroStep = serde_json::from_value(value)
            .map_err(|e| anyhow!("invalid macro step {index}: {e}"))?;
        step.validate().map_err(|e| anyhow!("invalid macro step {index}: {e}"))?;
        let combo = step.modifiers.iter().chain(step.keys.iter()).cloned().collect::<Vec<_>>();
        let (modifier, keys) = if combo.is_empty() {
            (0, [0u8; HID_KEY_BUFFER_SIZE])
        } else {
            keymap::parse_combo(&combo.join("+"))
                .map_err(|e| anyhow!("invalid macro step {index}: {e}"))?
        };
        let hold = u16::try_from(step.delay).unwrap_or(u16::MAX);
        steps.push(KeyboardMacroStep { modifier, keys, delay: hold });
        steps.push(KeyboardMacroStep {
            modifier: 0,
            keys: [0u8; HID_KEY_BUFFER_SIZE],
            delay: RELEASE_GAP_MS,
        });
    }
    info!(steps = steps.len(), "executing keyboard macro");
    crate::hidrpc::play_macro(false, steps).await?;
    Ok(Value::Null)
}

pub async fn cancel_keyboard_macro() -> Result<Value> {
    crate::hidrpc::cancel_running_macro().await;
    Ok(Value::Null)
}

const RELEASE_GAP_MS: u16 = 10;
const MAX_TYPE_TEXT_CHARS: usize = 10_000;
const ABS_POINTER_MAX: f64 = 32767.0;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeTextParams {
    pub text: String,
    pub delay_ms: Option<u16>,
}

pub async fn type_text(params: TypeTextParams) -> Result<Value> {
    let chars = params.text.chars().count();
    if chars > MAX_TYPE_TEXT_CHARS {
        anyhow::bail!("text too long: {chars} characters (max {MAX_TYPE_TEXT_CHARS})");
    }
    let delay = params.delay_ms.unwrap_or(20).clamp(5, 1000);
    let steps = keymap::text_to_macro_steps(&params.text, delay)?;
    crate::hidrpc::play_macro(true, steps).await?;
    Ok(serde_json::json!({ "typed": chars }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PressComboParams {
    pub combo: String,
    pub hold_ms: Option<u16>,
}

pub async fn press_combo(params: PressComboParams) -> Result<Value> {
    let hold = params.hold_ms.unwrap_or(50).clamp(10, 5000);
    let steps = keymap::combo_to_macro_steps(&params.combo, hold)?;
    crate::hidrpc::play_macro(false, steps).await?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointerParams {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub normalized: bool,
    pub button: Option<String>,
    #[serde(default)]
    pub double: bool,
}

async fn to_abs_pointer(params: &PointerParams) -> Result<(i32, i32)> {
    let (nx, ny) = if params.normalized {
        (params.x, params.y)
    } else {
        let video = crate::video::get_video_state().await;
        if !video.ready || video.width == 0 || video.height == 0 {
            anyhow::bail!("video input not ready; pass normalized coordinates instead");
        }
        (params.x / f64::from(video.width), params.y / f64::from(video.height))
    };
    if !(0.0..=1.0).contains(&nx) || !(0.0..=1.0).contains(&ny) {
        anyhow::bail!("pointer position ({}, {}) is outside the screen", params.x, params.y);
    }
    Ok(((nx * ABS_POINTER_MAX).round() as i32, (ny * ABS_POINTER_MAX).round() as i32))
}

fn button_mask(name: Option<&str>) -> Result<u8> {
    match name.unwrap_or("left").to_ascii_lowercase().as_str() {
        "left" => Ok(1),
        "right" => Ok(2),
        "middle" => Ok(4),
        other => anyhow::bail!("unknown mouse button {other:?} (left, right, middle)"),
    }
}

fn usb_hid() -> Result<std::sync::Arc<crate::hardware::usb::hid::Hid>> {
    let mgr = usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
    Ok(mgr.read().hid())
}

pub async fn mouse_move(params: PointerParams) -> Result<Value> {
    let (x, y) = to_abs_pointer(&params).await?;
    usb_hid()?.abs_mouse_report(x, y, 0)?;
    Ok(Value::Null)
}

pub async fn mouse_click(params: PointerParams) -> Result<Value> {
    let buttons = button_mask(params.button.as_deref())?;
    let (x, y) = to_abs_pointer(&params).await?;
    let hid = usb_hid()?;
    let pause = |ms| tokio::time::sleep(std::time::Duration::from_millis(ms));
    hid.abs_mouse_report(x, y, 0)?;
    pause(20).await;
    for click in 0..if params.double { 2 } else { 1 } {
        if click > 0 {
            pause(80).await;
        }
        hid.abs_mouse_report(x, y, buttons)?;
        pause(50).await;
        hid.abs_mouse_report(x, y, 0)?;
    }
    Ok(Value::Null)
}
