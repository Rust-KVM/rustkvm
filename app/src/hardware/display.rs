use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use once_cell::sync::OnceCell;
use parking_lot::Mutex as PlMutex;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::get_config_manager;
use crate::hardware::native::socket::call_ctrl_action;
use crate::video::get_video_state;
use crate::webrtc::get_current_session;

const BACKLIGHT_BRIGHTNESS: &str = "/sys/class/backlight/backlight/brightness";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BacklightState {
    Normal,
    Dimmed,
    Off,
}

struct DisplayInner {
    _state: BacklightState,
}

static DISPLAY: OnceCell<RwLock<DisplayInner>> = OnceCell::new();

pub async fn init_display() -> Result<()> {
    let _ = DISPLAY.set(RwLock::new(DisplayInner { _state: BacklightState::Normal }));

    tokio::spawn(async move {
        super_wait_ctrl_client_connected().await;
        info!("setting initial display contents");
        tokio::time::sleep(Duration::from_millis(500)).await;
        let rotation = get_config_manager().get().await.display_rotation;
        let mut params = serde_json::Map::new();
        params.insert("rotation".into(), serde_json::Value::String(rotation));
        let _ = call_ctrl_action("lv_disp_set_rotation", Some(params)).await;
        update_static_contents().await;
        let _ = wake_display(true).await;
        let _ = request_display_update(true).await;
    });

    Ok(())
}

async fn super_wait_ctrl_client_connected() {}

async fn lv_label_set_text(obj: &str, text: &str) {
    let mut params = serde_json::Map::new();
    params.insert("obj".into(), serde_json::Value::String(obj.to_string()));
    params.insert("text".into(), serde_json::Value::String(text.to_string()));
    let _ = call_ctrl_action("lv_label_set_text", Some(params)).await;
}

async fn lv_obj_set_state(obj: &str, state: &str) {
    let mut params = serde_json::Map::new();
    params.insert("obj".into(), serde_json::Value::String(obj.to_string()));
    params.insert("state".into(), serde_json::Value::String(state.to_string()));
    let _ = call_ctrl_action("lv_obj_set_state", Some(params)).await;
}

async fn lv_img_set_src(obj: &str, src: &str) {
    let mut params = serde_json::Map::new();
    params.insert("obj".into(), serde_json::Value::String(obj.to_string()));
    params.insert("src".into(), serde_json::Value::String(src.to_string()));
    let _ = call_ctrl_action("lv_img_set_src", Some(params)).await;
}

async fn update_static_contents() {
    let device_id = crate::hardware::hw::get_device_id();
    lv_label_set_text("ui_Status_Content_Device_Id_Content_Label", &device_id).await;
}

async fn set_display_brightness(brightness: i32) -> Result<()> {
    if !(0..=100).contains(&brightness) {
        return Err(anyhow!("brightness value out of bounds, must be between 0 and 100"));
    }

    if !Path::new(BACKLIGHT_BRIGHTNESS).exists() {
        return Err(anyhow!(
            "brightness value cannot be set, possibly not running on RustKVM hardware"
        ));
    }

    tokio::fs::write(BACKLIGHT_BRIGHTNESS, brightness.to_string())
        .await
        .context("failed to write brightness")?;
    info!(brightness, "set brightness");
    Ok(())
}

pub async fn wake_display(force: bool) -> Result<()> {
    let cfg = get_config_manager().get().await;
    if cfg.display_max_brightness == 0 && !force {
        return Ok(());
    }
    let _ = set_display_brightness(cfg.display_max_brightness as i32).await;
    start_backlight_tickers().await;
    Ok(())
}

static BACKLIGHT_CANCEL: PlMutex<Option<CancellationToken>> = PlMutex::new(None);

async fn start_backlight_tickers() {
    let _ = BacklightState::Dimmed;
    let _ = BacklightState::Off;
    let cfg = get_config_manager().get().await;
    if cfg.display_max_brightness == 0 {
        let _ = set_display_brightness(0).await;
        return;
    }

    let token = CancellationToken::new();
    if let Some(prev) = BACKLIGHT_CANCEL.lock().replace(token.clone()) {
        prev.cancel();
    }

    if cfg.display_dim_after_sec > 0 {
        let dim_after = cfg.display_dim_after_sec;
        let t = token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = t.cancelled() => {}
                _ = tokio::time::sleep(Duration::from_secs(dim_after as u64)) => {
                    if let Err(e) = set_display_brightness((cfg.display_max_brightness / 2) as i32).await {
                        warn!("failed to dim display: {}", e);
                    }
                }
            }
        });
    }

    if cfg.display_off_after_sec > 0 {
        let off_after = cfg.display_off_after_sec;
        let t = token;
        tokio::spawn(async move {
            tokio::select! {
                _ = t.cancelled() => {}
                _ = tokio::time::sleep(Duration::from_secs(off_after as u64)) => {
                    if let Err(e) = set_display_brightness(0).await {
                        warn!("failed to turn off display: {}", e);
                    }
                }
            }
        });
    }
}

pub async fn request_display_update(should_wake: bool) -> Result<()> {
    if should_wake {
        let _ = wake_display(false).await;
    }
    debug!("display updating");
    let v = get_video_state().await;
    if v.ready {
        lv_label_set_text("ui_Home_Footer_Hdmi_Status_Label", "Connected").await;
        lv_obj_set_state("ui_Home_Footer_Hdmi_Status_Label", "LV_STATE_DEFAULT").await;
    } else {
        lv_label_set_text("ui_Home_Footer_Hdmi_Status_Label", "Disconnected").await;
        lv_obj_set_state("ui_Home_Footer_Hdmi_Status_Label", "LV_STATE_USER_2").await;
    }

    let usb_state = crate::api::handlers::usb::get_usb_state().unwrap_or_else(|_| "unknown".into());
    if usb_state == "configured" {
        lv_label_set_text("ui_Home_Footer_Usb_Status_Label", "Connected").await;
        lv_obj_set_state("ui_Home_Footer_Usb_Status_Label", "LV_STATE_DEFAULT").await;
    } else {
        lv_label_set_text("ui_Home_Footer_Usb_Status_Label", "Disconnected").await;
        lv_obj_set_state("ui_Home_Footer_Usb_Status_Label", "LV_STATE_USER_2").await;
    }

    let has_session = get_current_session().await.is_some();
    let active_count = if has_session { 1 } else { 0 };
    lv_label_set_text("ui_Home_Header_Cloud_Status_Label", &format!("{} active", active_count))
        .await;
    lv_img_set_src("ui_Home_Header_Cloud_Status_Icon", "cloud.png").await;
    Ok(())
}
