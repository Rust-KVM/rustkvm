use serde::Serialize;

use crate::api::handlers::network::NetworkStateResponse;
use crate::api::handlers::system::FailsafeModeResponse;
use crate::observability::metrics::{AUDIO_FRAMES_TOTAL, VIDEO_FRAMES_TOTAL};
use crate::version::VersionInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Ok,
    Degraded,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionReport {
    pub version: &'static str,
    pub revision: &'static str,
    pub branch: &'static str,
    pub build_date: &'static str,
    pub platform: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoReport {
    pub ready: bool,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub frames_total: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub status: HealthStatus,
    pub issues: Vec<String>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_uptime_secs: Option<f64>,
    pub version: VersionReport,
    pub video: VideoReport,
    pub audio_frames_total: u64,
    pub usb_state: String,
    pub network: NetworkStateResponse,
    pub failsafe: FailsafeModeResponse,
    pub virtual_media_mounted: bool,
}

pub async fn health_report() -> HealthReport {
    let video_state = crate::video::get_video_state().await;
    let usb_state = crate::api::handlers::usb::get_usb_state().unwrap_or_default();
    let network = crate::api::handlers::network::get_network_state().unwrap_or_else(|_| {
        NetworkStateResponse { online: false, ip: String::new(), gateway: None, dns: Vec::new() }
    });
    let failsafe = crate::api::handlers::system::get_failsafe_mode().unwrap_or_default();
    let virtual_media_mounted = crate::hardware::usb::storage::get_virtual_media_state().is_some();
    let system_uptime_secs = tokio::fs::read_to_string("/proc/uptime")
        .await
        .ok()
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok()));

    let mut issues = Vec::new();
    if !video_state.ready {
        issues.push(match &video_state.error {
            Some(err) => format!("video: {err}"),
            None => "video: no input signal".to_string(),
        });
    }
    if usb_state != "configured" {
        issues.push(format!("usb: gadget state is {usb_state:?}, host not attached"));
    }
    if !network.online {
        issues.push("network: offline".to_string());
    }
    if failsafe.active {
        issues.push(format!("failsafe: {}", failsafe.reason));
    }

    let info = VersionInfo::current();
    HealthReport {
        status: if issues.is_empty() { HealthStatus::Ok } else { HealthStatus::Degraded },
        issues,
        timestamp: chrono::Utc::now().to_rfc3339(),
        system_uptime_secs,
        version: VersionReport {
            version: info.version,
            revision: info.revision,
            branch: info.branch,
            build_date: info.build_date,
            platform: info.platform,
        },
        video: VideoReport {
            ready: video_state.ready,
            width: video_state.width,
            height: video_state.height,
            fps: video_state.frame_per_second,
            error: video_state.error,
            frames_total: VIDEO_FRAMES_TOTAL.get(),
        },
        audio_frames_total: AUDIO_FRAMES_TOTAL.get(),
        usb_state,
        network,
        failsafe,
        virtual_media_mounted,
    }
}
