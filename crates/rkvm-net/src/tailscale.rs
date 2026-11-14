use std::process::Command;

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailscaleStatus {
    pub online: bool,
    pub hostname: Option<String>,
    pub ip_addresses: Option<Vec<String>>,
    pub self_info: Option<serde_json::Value>,
}

pub fn get_status() -> Result<TailscaleStatus> {
    let output = match Command::new("tailscale").args(["status", "--json"]).output() {
        Ok(output) => output,
        // Tailscale is optional and not part of the default image.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(offline()),
        Err(source) => return Err(Error::CommandExec { cmd: "tailscale status", source }),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("tailscale status failed: {}", stderr);
        return Ok(offline());
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

fn offline() -> TailscaleStatus {
    TailscaleStatus { online: false, hostname: None, ip_addresses: None, self_info: None }
}
