use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::hardware::edid;
use crate::hardware::usb::storage as storage_mod;

static STREAM_QUALITY_FACTOR: AtomicI32 = AtomicI32::new(100);
static VIDEO_CODEC_PREFERENCE: OnceLock<Mutex<String>> = OnceLock::new();
static VIDEO_SLEEP_ENABLED: AtomicBool = AtomicBool::new(false);
static VIDEO_SLEEP_DURATION: AtomicI32 = AtomicI32::new(60);
static DISPLAY_ROTATION: OnceLock<Mutex<String>> = OnceLock::new();

#[derive(Deserialize)]
pub struct DisplayRotationParams {
    pub rotation: String,
}

#[derive(Serialize)]
pub struct DisplayRotationResponse {
    pub rotation: String,
}

pub async fn set_display_rotation(params: DisplayRotationParams) -> Result<Value> {
    if !["0", "90", "180", "270"].contains(&params.rotation.as_str()) {
        return Err(anyhow!("Invalid rotation value: {}", params.rotation));
    }
    let mgr = crate::config::get_config_manager();
    let rotation = params.rotation.clone();
    mgr.update(|cfg| {
        cfg.display_rotation = rotation.clone();
    })
    .await?;
    let rotation_mutex = DISPLAY_ROTATION.get_or_init(|| Mutex::new("0".to_string()));
    *rotation_mutex.lock() = params.rotation.clone();
    info!("Display rotation set to: {} (persisted)", params.rotation);
    Ok(Value::Null)
}

pub async fn get_display_rotation() -> Result<DisplayRotationResponse> {
    let cfg = crate::config::get_config_manager().get().await;
    Ok(DisplayRotationResponse { rotation: cfg.display_rotation })
}

#[derive(Deserialize)]
pub struct BacklightSettings {
    pub max_brightness: i32,
    pub dim_after: i32,
    pub off_after: i32,
}

#[derive(Serialize)]
pub struct BacklightSettingsResponse {
    pub max_brightness: i32,
    pub dim_after: i32,
    pub off_after: i32,
}

pub async fn set_backlight_settings(settings: BacklightSettings) -> Result<Value> {
    if !(0..=255).contains(&settings.max_brightness) {
        return Err(anyhow!("max_brightness must be between 0 and 255"));
    }
    if settings.dim_after < 0 {
        return Err(anyhow!("dim_after must be a positive integer"));
    }
    if settings.off_after < 0 {
        return Err(anyhow!("off_after must be a positive integer"));
    }

    let mgr = crate::config::get_config_manager();
    mgr.update(|cfg| {
        cfg.display_max_brightness = settings.max_brightness as u8;
        cfg.display_dim_after_sec = settings.dim_after as u32;
        cfg.display_off_after_sec = settings.off_after as u32;
    })
    .await?;

    if let Err(e) = tokio::fs::write(
        "/sys/class/backlight/backlight/brightness",
        settings.max_brightness.to_string(),
    )
    .await
    {
        warn!("Failed to set brightness: {}", e);
    }

    info!(
        "Backlight settings applied (persisted): brightness={}, dim_after={}, off_after={}",
        settings.max_brightness, settings.dim_after, settings.off_after
    );
    Ok(Value::Null)
}

pub async fn get_backlight_settings() -> Result<BacklightSettingsResponse> {
    let cfg = crate::config::get_config_manager().get().await;
    Ok(BacklightSettingsResponse {
        max_brightness: cfg.display_max_brightness as i32,
        dim_after: cfg.display_dim_after_sec as i32,
        off_after: cfg.display_off_after_sec as i32,
    })
}

#[derive(Serialize)]
pub struct DisplayState {
    pub ip: String,
    pub usb_connected: bool,
    pub cloud_state: String,
    pub keyboard_led: crate::hardware::usb::KeyboardState,
}

pub fn get_display_state() -> Result<DisplayState> {
    let ip = super::network::get_network_ip_address().unwrap_or_default();
    let usb_connected = super::usb::get_usb_state().unwrap_or_default() == "configured";
    let cloud_state = crate::cloud::manager::get_cloud_manager().get_state().as_str().to_string();
    let keyboard_led = super::hid::get_keyboard_led_state().unwrap_or_default();
    Ok(DisplayState { ip, usb_connected, cloud_state, keyboard_led })
}

pub fn get_stream_quality_factor() -> Result<f64> {
    let factor = STREAM_QUALITY_FACTOR.load(Ordering::Relaxed) as f64 / 100.0;
    Ok(factor)
}

#[derive(Deserialize)]
pub struct StreamQualityParams {
    pub factor: f64,
}

pub fn set_stream_quality_factor(params: StreamQualityParams) -> Result<Value> {
    if !(0.0..=1.0).contains(&params.factor) {
        return Err(anyhow!("Quality factor must be between 0.0 and 1.0"));
    }

    let percentage = (params.factor * 100.0) as i32;
    STREAM_QUALITY_FACTOR.store(percentage, Ordering::Relaxed);

    let factor = params.factor as f32;
    tokio::spawn(async move {
        if let Err(e) = crate::video::update_video_quality(factor).await {
            warn!("Failed to update video quality: {}", e);
        }
    });

    info!("Stream quality factor set to: {}", params.factor);
    Ok(Value::Null)
}

pub async fn get_video_state() -> Result<crate::video::VideoInputState> {
    Ok(crate::video::get_video_state().await)
}

pub fn get_video_codec_preference() -> Result<String> {
    let cell = VIDEO_CODEC_PREFERENCE.get_or_init(|| Mutex::new("auto".to_string()));
    Ok(cell.lock().clone())
}

#[derive(Deserialize)]
pub struct VideoCodecParams {
    pub codec: String,
}

pub fn set_video_codec_preference(params: VideoCodecParams) -> Result<Value> {
    if params.codec != "auto" && params.codec != "h265" && params.codec != "h264" {
        return Err(anyhow!(
            "Invalid codec preference: {} (must be auto, h265, or h264)",
            params.codec
        ));
    }
    let cell = VIDEO_CODEC_PREFERENCE.get_or_init(|| Mutex::new("auto".to_string()));
    *cell.lock() = params.codec.clone();
    info!("Video codec preference set to: {}", params.codec);
    Ok(Value::Null)
}

#[derive(Serialize)]
pub struct VideoSleepModeResponse {
    pub supported: bool,
    pub enabled: bool,
    pub duration: i32,
}

pub fn get_video_sleep_mode() -> Result<VideoSleepModeResponse> {
    Ok(VideoSleepModeResponse {
        supported: true,
        enabled: VIDEO_SLEEP_ENABLED.load(Ordering::Relaxed),
        duration: VIDEO_SLEEP_DURATION.load(Ordering::Relaxed),
    })
}

#[derive(Deserialize)]
pub struct VideoSleepModeParams {
    pub duration: i32,
}

pub fn set_video_sleep_mode(params: VideoSleepModeParams) -> Result<Value> {
    let duration = if params.duration < 0 { -1 } else { params.duration };
    VIDEO_SLEEP_DURATION.store(duration, Ordering::Relaxed);
    VIDEO_SLEEP_ENABLED.store(duration >= 0, Ordering::Relaxed);
    info!("Video sleep mode set: duration={}", duration);
    Ok(Value::Null)
}

pub async fn get_video_log_status() -> Result<String> {
    match crate::video::get_video_stats().await {
        Ok(s) if s.error.is_some() => Ok(format!("error: {}", s.error.unwrap_or_default())),
        Ok(s) if !s.pipeline_started => Ok("not_started".to_string()),
        Ok(s) if !s.ready => Ok("starting".to_string()),
        Ok(_) => Ok("ok".to_string()),
        Err(e) => Ok(format!("error: {e}")),
    }
}

pub fn get_edid() -> Result<String> {
    let edid_bytes = edid::read_edid()?;
    Ok(hex::encode(&edid_bytes))
}

pub fn set_edid(params: serde_json::Value) -> Result<Value> {
    let edid_str = params.get("edid").and_then(|v| v.as_str()).unwrap_or("");

    let edid_hex = if edid_str.is_empty() {
        info!("Restoring EDID to default");
        "00ffffffffffff0031d8341200000000221a010380301b780fee91a3544c99260f50542fcf00315945598180814090409500a940b300023a801871382d40582c4500e00e1100001e000000fd001855185e11000a202020202020000000fc0068646d690a202020202020202000000010000000000000000000000000000001c402032df04c101f04132221200514021101230907078301000068030c001000002201e200eae3050000e30601001a3680a070381f4030203500e00e1100001a1a1d008051d01c2040803500e00e1100001c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007d"
    } else {
        info!("Setting EDID: {}", edid_str);
        edid_str
    };

    if edid_hex.len() > 512 {
        return Err(anyhow!("invalid edid"));
    }

    let edid_bytes = hex::decode(edid_hex).map_err(|_| anyhow!("Invalid EDID format"))?;
    edid::write_edid(&edid_bytes)?;

    let config_manager = crate::config::get_config_manager();
    let edid_hex_to_save = edid_hex.to_ascii_lowercase();
    tokio::spawn(async move {
        if let Err(e) = config_manager
            .update(|config| {
                config.edid_string = Some(edid_hex_to_save);
            })
            .await
        {
            warn!("Failed to save EDID to config: {}", e);
        }
    });

    info!("EDID configuration updated");
    Ok(Value::Null)
}

#[derive(Serialize)]
pub struct VirtualMediaStateResponse {
    pub source: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub size: i64,
}

pub fn get_virtual_media_state() -> Result<Option<VirtualMediaStateResponse>> {
    let s = storage_mod::get_virtual_media_state();
    let mapped = s.map(|st| VirtualMediaStateResponse {
        source: match st.source {
            storage_mod::VirtualMediaSource::WebRTC => "WebRTC".into(),
            storage_mod::VirtualMediaSource::HTTP => "HTTP".into(),
            storage_mod::VirtualMediaSource::Storage => "Storage".into(),
        },
        mode: match st.mode {
            storage_mod::VirtualMediaMode::CDROM => "CDROM".into(),
            storage_mod::VirtualMediaMode::Disk => "Disk".into(),
        },
        filename: st.filename,
        url: st.url,
        size: st.size,
    });
    Ok(mapped)
}

fn parse_vm_mode(mode: &str) -> Result<storage_mod::VirtualMediaMode> {
    match mode.to_ascii_uppercase().as_str() {
        "CDROM" => Ok(storage_mod::VirtualMediaMode::CDROM),
        "DISK" => Ok(storage_mod::VirtualMediaMode::Disk),
        _ => Err(anyhow!("invalid mode")),
    }
}

#[derive(Deserialize)]
pub struct MountHttpParams {
    pub url: String,
    pub mode: String,
}

pub async fn mount_with_http(params: MountHttpParams) -> Result<Value> {
    let mode = parse_vm_mode(&params.mode)?;
    storage_mod::mount_with_http(&params.url, mode).await?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct MountWebRtcParams {
    pub filename: String,
    pub size: i64,
    pub mode: String,
}

pub async fn mount_with_webrtc(params: MountWebRtcParams) -> Result<Value> {
    let mode = parse_vm_mode(&params.mode)?;
    storage_mod::mount_with_webrtc(&params.filename, params.size, mode).await?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct MountStorageParams {
    pub filename: String,
    pub mode: String,
}

pub async fn mount_with_storage(params: MountStorageParams) -> Result<Value> {
    let mode = parse_vm_mode(&params.mode)?;
    storage_mod::mount_with_storage(&params.filename, mode).await?;
    Ok(Value::Null)
}

pub async fn unmount_image() -> Result<Value> {
    storage_mod::unmount_image().await?;
    Ok(Value::Null)
}

#[derive(Serialize)]
pub struct StorageSpaceResponse {
    #[serde(rename = "bytesUsed")]
    pub bytes_used: i64,
    #[serde(rename = "bytesFree")]
    pub bytes_free: i64,
}

#[derive(Serialize)]
pub struct StorageFileInfo {
    pub filename: String,
    pub size: i64,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "totalBytes")]
    pub total_bytes: i64,
}

#[derive(Serialize)]
pub struct StorageFilesResponse {
    pub files: Vec<StorageFileInfo>,
}

#[derive(Deserialize)]
pub struct StartUploadParams {
    pub filename: String,
    pub size: i64,
}

#[derive(Serialize)]
pub struct StartUploadResponse {
    #[serde(rename = "alreadyUploadedBytes")]
    pub already_uploaded_bytes: i64,
    #[serde(rename = "dataChannel")]
    pub data_channel: String,
}

pub async fn start_storage_file_upload(params: StartUploadParams) -> Result<StartUploadResponse> {
    let up = storage_mod::start_storage_file_upload(&params.filename, params.size).await?;
    Ok(StartUploadResponse {
        already_uploaded_bytes: up.already_uploaded_bytes,
        data_channel: up.data_channel,
    })
}

pub async fn list_storage_files() -> Result<StorageFilesResponse> {
    let list = storage_mod::list_storage_files().await?;
    let files = list
        .files
        .into_iter()
        .map(|e| StorageFileInfo {
            filename: e.filename,
            size: e.size,
            created_at: e.created_at,
            total_bytes: e.total_bytes,
        })
        .collect();
    Ok(StorageFilesResponse { files })
}

#[derive(Deserialize)]
pub struct DeleteStorageFileParams {
    pub filename: String,
}

pub async fn delete_storage_file(params: DeleteStorageFileParams) -> Result<Value> {
    storage_mod::delete_storage_file(&params.filename).await?;
    Ok(Value::Null)
}

pub async fn get_storage_space() -> Result<StorageSpaceResponse> {
    let s = storage_mod::get_storage_space().await?;
    Ok(StorageSpaceResponse { bytes_used: s.bytes_used, bytes_free: s.bytes_free })
}

#[derive(Deserialize)]
pub struct MountBuiltInImageParams {
    pub filename: String,
}

pub async fn mount_built_in_image(params: MountBuiltInImageParams) -> Result<Value> {
    storage_mod::mount_built_in_image(&params.filename).await?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct MassStorageModeParams {
    pub mode: String,
}

pub async fn set_mass_storage_mode(params: MassStorageModeParams) -> Result<String> {
    let mode = params.mode.to_ascii_lowercase();
    let cdrom = match mode.as_str() {
        "cdrom" => true,
        "file" => false,
        _ => return Err(anyhow!("invalid mode: {}", params.mode)),
    };
    storage_mod::set_mass_storage_mode(cdrom).await?;
    get_mass_storage_mode().await
}

pub async fn get_mass_storage_mode() -> Result<String> {
    let cdrom = storage_mod::get_mass_storage_cdrom_enabled().await?;
    Ok(if cdrom { "cdrom".to_string() } else { "file".to_string() })
}

#[derive(Deserialize)]
pub struct CheckMountUrlParams {
    pub url: String,
}

#[derive(Serialize)]
pub struct VirtualMediaUrlInfo {
    #[serde(rename = "Usable")]
    pub usable: bool,
    #[serde(rename = "Reason", skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "Size")]
    pub size: i64,
}

pub fn check_mount_url(params: CheckMountUrlParams) -> Result<VirtualMediaUrlInfo> {
    let r = storage_mod::check_mount_url(&params.url)?;
    Ok(VirtualMediaUrlInfo { usable: r.usable, reason: r.reason, size: r.size })
}

#[derive(Deserialize)]
pub struct RpcMountBuiltInImageParams {
    pub filename: String,
}

pub async fn rpc_mount_built_in_image(params: RpcMountBuiltInImageParams) -> Result<Value> {
    mount_built_in_image(MountBuiltInImageParams { filename: params.filename }).await
}
