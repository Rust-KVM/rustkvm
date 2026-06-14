use std::time::Duration;

use anyhow::Result;
use tracing::debug;

use super::types::{AtxState, DcPowerState, MqttSystemState, MqttVideoState};
use crate::power;

pub(super) async fn set_dc_power_state(on: bool) -> Result<()> {
    tokio::task::spawn_blocking(move || power::dc::set_dc_power_state(on))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

pub(super) async fn set_dc_restore_state(state: i32) -> Result<()> {
    tokio::task::spawn_blocking(move || power::dc::set_dc_restore_state(state))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

pub(super) async fn press_atx_power_button(duration: Duration) -> Result<()> {
    let ms = duration.as_millis() as u64;
    tokio::task::spawn_blocking(move || power::atx::press_atx_power_button(ms))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

pub(super) async fn press_atx_reset_button(duration: Duration) -> Result<()> {
    let ms = duration.as_millis() as u64;
    tokio::task::spawn_blocking(move || power::atx::press_atx_reset_button(ms))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

pub(super) fn set_jiggler_state(enabled: bool) -> Result<()> {
    tokio::spawn(async move {
        if let Err(e) = crate::hardware::jiggler::set_jiggler_state(enabled).await {
            tracing::warn!("Failed to set jiggler state from MQTT: {}", e);
        }
    });
    Ok(())
}

pub(super) fn reboot_device() -> Result<()> {
    tokio::spawn(async {
        crate::api::broadcast_will_reboot("mqtt_command").await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let _ = std::process::Command::new("reboot").spawn();
        tokio::time::sleep(Duration::from_secs(5)).await;
        std::process::exit(0);
    });
    Ok(())
}

pub(super) fn trigger_ota_update() -> Result<()> {
    debug!("triggerOTAUpdate — OTA not yet implemented (no auto-update channel)");
    Ok(())
}

pub(super) fn unmount_image() -> Result<()> {
    tokio::spawn(async {
        if let Err(e) = crate::hardware::usb::storage::unmount_image().await {
            tracing::warn!("MQTT unmount_image failed: {}", e);
        }
    });
    Ok(())
}

pub(super) fn mount_with_storage(name: &str) -> Result<()> {
    let name = name.to_string();
    tokio::spawn(async move {
        if let Err(e) = crate::hardware::usb::storage::mount_with_storage(
            &name,
            crate::hardware::usb::storage::VirtualMediaMode::Disk,
        )
        .await
        {
            tracing::warn!("MQTT mount_with_storage('{}') failed: {}", name, e);
        }
    });
    Ok(())
}

pub(super) fn is_virtual_media_mounted() -> bool {
    crate::hardware::usb::storage::get_virtual_media_state().is_some()
}

pub(super) fn get_dc_state() -> DcPowerState {
    power::dc::get_dc_power_state()
}

pub(super) async fn get_video_state() -> MqttVideoState {
    match crate::video::get_video_stats().await {
        Ok(s) => MqttVideoState {
            ready: s.ready,
            error: s.error,
            width: s.width,
            height: s.height,
            fps: s.fps,
        },
        Err(_) => MqttVideoState { ready: false, error: None, width: 0, height: 0, fps: 0.0 },
    }
}

pub(super) fn get_usb_state() -> String {
    crate::hardware::usb::get_current_usb_state()
}

pub(super) fn is_cloud_connected() -> bool {
    use crate::cloud::types::CloudConnectionState;
    crate::cloud::manager::get_cloud_manager().get_state() == CloudConnectionState::Connected
}

pub(super) async fn is_jiggler_enabled() -> bool {
    crate::hardware::jiggler::get_jiggler_state().await
}

pub(super) async fn get_active_sessions() -> i32 {
    crate::webrtc::get_active_session_count().await
}

pub(super) async fn get_network_state() -> (String, String) {
    let ip = get_local_ip().unwrap_or_default();
    let hostname = get_hostname().await;
    (ip, hostname)
}

fn get_local_ip() -> Option<String> {
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if !iface.is_loopback()
                && let if_addrs::IfAddr::V4(v4) = iface.addr
            {
                return Some(v4.ip.to_string());
            }
        }
    }
    None
}

async fn get_hostname() -> String {
    tokio::fs::read_to_string("/proc/sys/kernel/hostname")
        .await
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(super) async fn get_system_state() -> MqttSystemState {
    let mut state = MqttSystemState {
        cpu_load: 0.0,
        temperature: 0.0,
        memory_used: 0,
        memory_total: 0,
        storage_used: 0,
        storage_free: 0,
    };

    if let Ok(data) = tokio::fs::read_to_string("/proc/loadavg").await
        && let Some(load_str) = data.split_whitespace().next()
        && let Ok(load) = load_str.parse::<f64>()
    {
        state.cpu_load = load;
    }

    if let Ok(data) = tokio::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").await
        && let Ok(temp) = data.trim().parse::<f64>()
    {
        state.temperature = temp / 1000.0;
    }

    if let Ok(data) = tokio::fs::read_to_string("/proc/meminfo").await {
        for line in data.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 2 {
                continue;
            }
            if let Ok(val) = fields[1].parse::<u64>() {
                match fields[0] {
                    "MemTotal:" => state.memory_total = val * 1024,
                    "MemAvailable:" => state.memory_used = state.memory_total - (val * 1024),
                    _ => {}
                }
            }
        }
    }

    let images_dir = "/userdata/rustkvm/images";
    // SAFETY: `path` is a valid NUL-terminated C string that outlives the call, and
    // `stat` is a correctly-sized, zero-initialised `statfs` that the kernel fills
    // in. Its fields are only read after `statfs` returns 0 (success).
    unsafe {
        let mut stat: libc::statfs = std::mem::zeroed();
        let path = std::ffi::CString::new(images_dir).unwrap_or_default();
        if libc::statfs(path.as_ptr(), &mut stat) == 0 {
            let total = stat.f_blocks as u64 * stat.f_bsize as u64;
            let free = stat.f_bfree as u64 * stat.f_bsize as u64;
            state.storage_used = (total - free) as i64;
            state.storage_free = free as i64;
        }
    }

    state
}

pub(super) fn get_virtual_media_mount_info() -> (String, String) {
    use crate::hardware::usb::storage::{
        VirtualMediaMode, VirtualMediaSource, get_virtual_media_state,
    };
    match get_virtual_media_state() {
        None => ("-- no media --".to_string(), "none".to_string()),
        Some(s) => {
            let name = s.filename.unwrap_or_else(|| s.url.unwrap_or_default());
            let mode = match (s.source, s.mode) {
                (_, VirtualMediaMode::CDROM) => "cdrom",
                (VirtualMediaSource::Storage, VirtualMediaMode::Disk) => "disk",
                (VirtualMediaSource::HTTP, _) => "http",
                (VirtualMediaSource::WebRTC, _) => "webrtc",
            };
            (name, mode.to_string())
        }
    }
}

pub(super) fn get_installed_version() -> String {
    crate::version::built_app_version().to_string()
}

pub(super) struct UpdateStatus {
    pub(super) installed_version: String,
    pub(super) latest_version: String,
    pub(super) update_available: bool,
}

pub(super) fn get_update_status() -> Option<UpdateStatus> {
    None
}

pub(super) fn is_ota_updating() -> bool {
    false
}

pub(super) fn get_ota_progress() -> f32 {
    0.0
}

pub(super) fn get_active_extension() -> String {
    String::new()
}

pub(super) fn get_current_atx_state() -> AtxState {
    power::atx::get_atx_state().unwrap_or(AtxState { power: false, hdd: false })
}

pub(super) fn get_network_config_url() -> String {
    let ip = get_local_ip().unwrap_or_default();
    if ip.is_empty() {
        return String::new();
    }
    let scheme = "http";
    format!("{}://{}", scheme, ip)
}

pub(super) fn get_sw_version_string() -> String {
    format!("App: {}", env!("CARGO_PKG_VERSION"))
}

pub(super) async fn get_available_images() -> Vec<String> {
    let mut options = vec!["-- no media --".to_string()];
    let images_dir = "/userdata/rustkvm/images";
    if let Ok(mut entries) = tokio::fs::read_dir(images_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_type().await.map(|t| t.is_file()).unwrap_or(false)
                && let Some(name) = entry.file_name().to_str()
            {
                options.push(name.to_string());
            }
        }
    }
    options
}

pub(super) fn get_device_id() -> String {
    crate::hardware::hw::get_device_id()
}
