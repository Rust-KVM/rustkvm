use anyhow::{Context, Result, bail};
use once_cell::sync::OnceCell;
use regex::Regex;
use rustix::fs::{Mode, OFlags, open};
use rustix::io::{read, write};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

static DEVICE_ID: OnceCell<String> = OnceCell::new();

pub fn extract_serial_number() -> Result<String> {
    let file = open("/proc/cpuinfo", OFlags::RDONLY, Mode::empty())
        .context("Failed to open /proc/cpuinfo")?;

    let mut content = Vec::new();
    let mut buffer = [0u8; 4096];

    loop {
        match read(&file, &mut buffer) {
            Ok(0) => break,
            Ok(n) => content.extend_from_slice(&buffer[..n]),
            Err(e) => return Err(e).context("Failed to read /proc/cpuinfo"),
        }
    }

    let content = String::from_utf8(content).context("Failed to parse /proc/cpuinfo as UTF-8")?;

    let r = Regex::new(r"Serial\s*:\s*(\S+)").context("Failed to compile regex")?;

    if let Some(matches) = r.captures(&content)
        && matches.len() >= 2
    {
        return Ok(matches[1].to_string());
    }

    bail!("No serial number found in /proc/cpuinfo")
}

pub fn get_device_id() -> String {
    DEVICE_ID
        .get_or_init(|| match extract_serial_number() {
            Ok(serial) => {
                debug!("Extracted device serial number: {}", serial);
                serial
            }
            Err(e) => {
                warn!("Unknown serial number, the program likely not running on RV1106: {}", e);
                "unknown_device_id".to_string()
            }
        })
        .clone()
}

pub async fn run_watchdog(cancel_token: CancellationToken) -> Result<()> {
    let file = match open("/dev/watchdog", OFlags::WRONLY, Mode::empty()) {
        Ok(file) => file,
        Err(e) => {
            warn!("Unable to open /dev/watchdog, skipping watchdog reset: {}", e);
            return Ok(());
        }
    };

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = write(&file, &[0]) {
                    warn!("Error writing to /dev/watchdog, system may reboot: {}", e);
                }
            }
            _ = cancel_token.cancelled() => {
                if let Err(e) = write(&file, b"V") {
                    warn!("Failed to disarm watchdog, system may reboot: {}", e);
                }
                return Ok(());
            }
        }
    }
}

pub fn disarm_watchdog() -> Result<()> {
    let file = open("/dev/watchdog", OFlags::WRONLY, Mode::empty())
        .context("Failed to open watchdog device")?;

    write(&file, b"V").context("Failed to disarm watchdog")?;

    Ok(())
}

const PSTORE_DIR: &str = "/sys/fs/pstore";

/// Lines of a pstore kernel log that identify a crash: what failed, in which task,
/// and where. Capped so one record stays a few log lines.
pub fn summarize_kernel_crash(log: &str) -> Vec<&str> {
    const MARKERS: [&str; 8] = [
        "Internal error",
        "Kernel panic",
        "BUG:",
        "corruption",
        "Unable to handle",
        "Comm:",
        "pc :",
        "lr :",
    ];
    log.lines()
        .map(str::trim_end)
        .filter(|line| MARKERS.iter().any(|m| line.contains(m)))
        .take(8)
        .collect()
}

/// Logs the kernel crash records kept in pstore (ramoops) from earlier boots, so a
/// crash that took HID or the whole device down is visible in the app log.
pub fn log_previous_kernel_crashes() {
    let Ok(entries) = std::fs::read_dir(PSTORE_DIR) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("dmesg-") {
            continue;
        }
        let recorded = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
            .unwrap_or_else(|_| "unknown".to_string());
        let Ok(log) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let summary = summarize_kernel_crash(&log);
        if summary.is_empty() {
            continue;
        }
        warn!(
            record = %name,
            recorded = %recorded,
            "kernel crash from an earlier boot (full log in {PSTORE_DIR}): {}",
            summary.join(" | ")
        );
    }
}
