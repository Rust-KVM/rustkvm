use std::process::Command;

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{Error, Result};

pub const DEFAULT_CONTROL_URL: &str = "https://controlplane.tailscale.com";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailscaleStatus {
    pub online: bool,
    pub hostname: Option<String>,
    pub ip_addresses: Option<Vec<String>>,
    pub self_info: Option<serde_json::Value>,
}

static CONTROL_URL: OnceCell<Mutex<String>> = OnceCell::new();

pub fn get_status(_control_url: &str) -> Result<TailscaleStatus> {
    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .map_err(|source| Error::CommandExec { cmd: "tailscale status", source })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("tailscale status failed: {}", stderr);
        return Ok(TailscaleStatus {
            online: false,
            hostname: None,
            ip_addresses: None,
            self_info: None,
        });
    }

    let raw: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    let online =
        raw.get("BackendState").and_then(|v| v.as_str()).map(|s| s == "Running").unwrap_or(false);

    let hostname =
        raw.get("Self").and_then(|s| s.get("HostName")).and_then(|v| v.as_str()).map(String::from);

    let ip_addresses = raw
        .get("Self")
        .and_then(|s| s.get("TailscaleIPs"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

    Ok(TailscaleStatus { online, hostname, ip_addresses, self_info: raw.get("Self").cloned() })
}

pub fn set_control_url(url: &str) -> Result<String> {
    let trimmed = url.trim().to_string();

    if trimmed.is_empty() {
        let cell = CONTROL_URL.get_or_init(|| Mutex::new(DEFAULT_CONTROL_URL.to_string()));
        *cell.lock() = DEFAULT_CONTROL_URL.to_string();
        return Ok(DEFAULT_CONTROL_URL.to_string());
    }

    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(Error::InvalidControlUrl);
    }

    let normalized = trimmed.trim_end_matches('/').to_string();

    let cell = CONTROL_URL.get_or_init(|| Mutex::new(normalized.clone()));
    *cell.lock() = normalized.clone();

    debug!("Tailscale control URL set to: {}", normalized);
    Ok(normalized)
}

pub fn get_control_url() -> String {
    let cell = CONTROL_URL.get_or_init(|| Mutex::new(DEFAULT_CONTROL_URL.to_string()));
    cell.lock().clone()
}
