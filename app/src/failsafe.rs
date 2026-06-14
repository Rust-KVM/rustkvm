use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

use parking_lot::RwLock;
use serde::Serialize;
use tracing::warn;

const FAILSAFE_DEFAULT_LAST_CRASH_PATH: &str = "/userdata/rustkvm/crashdump/last-crash.log";
const FAILSAFE_FILE: &str = "/userdata/rustkvm/.enablefailsafe";
const FAILSAFE_LAST_CRASH_ENV: &str = "RUSTKVM_LAST_ERROR_PATH";
const FAILSAFE_ENV: &str = "RUSTKVM_FORCE_FAILSAFE";
const FAILSAFE_VIDEO_PATTERNS: &[&str] = &["video_max_restart_attempts_reached", "panic in video"];

#[derive(Debug, Clone, Default, Serialize)]
pub struct FailsafeState {
    pub active: bool,
    pub reason: String,
    pub crash_log: String,
}

static STATE: RwLock<Option<FailsafeState>> = RwLock::new(None);
static CHECKED: OnceLock<()> = OnceLock::new();

#[derive(Serialize)]
pub struct FailsafeNotification {
    pub active: bool,
    pub reason: String,
}

pub fn check_failsafe_reason() {
    if CHECKED.set(()).is_err() {
        return;
    }

    if std::env::var(FAILSAFE_ENV).ok().as_deref() == Some("1") {
        *STATE.write() = Some(FailsafeState {
            active: true,
            reason: "failsafe_env_set".into(),
            crash_log: "".into(),
        });
        return;
    }

    if Path::new(FAILSAFE_FILE).exists() {
        *STATE.write() = Some(FailsafeState {
            active: true,
            reason: "failsafe_file_exists".into(),
            crash_log: "".into(),
        });
        let _ = fs::remove_file(FAILSAFE_FILE);
        return;
    }

    let last_crash_path = std::env::var(FAILSAFE_LAST_CRASH_ENV)
        .unwrap_or_else(|_| FAILSAFE_DEFAULT_LAST_CRASH_PATH.into());

    let meta = match fs::symlink_metadata(&last_crash_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            warn!(path = %last_crash_path, "failed to stat last crash log: {e}");
            return;
        }
    };
    if !meta.file_type().is_symlink() {
        warn!(path = %last_crash_path, "last crash log is not a symlink, ignoring");
        return;
    }

    let content = match read_file_tail(&last_crash_path, 50 * 1024) {
        Ok(s) => s,
        Err(e) => {
            warn!(path = %last_crash_path, "failed to read last crash log: {e}");
            return;
        }
    };

    let _ = fs::remove_file(&last_crash_path);

    let lower = content.to_ascii_lowercase();
    let reason = if FAILSAFE_VIDEO_PATTERNS.iter().any(|p| lower.contains(p)) {
        "video"
    } else if lower.contains("sigsegv") || lower.contains("sigill") || lower.contains("sigbus") {
        "signal"
    } else {
        "unknown"
    };

    *STATE.write() =
        Some(FailsafeState { active: true, reason: reason.into(), crash_log: content });
}

fn read_file_tail(path: &str, max_bytes: u64) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let size = f.metadata()?.len();
    if size > max_bytes {
        f.seek(SeekFrom::Start(size - max_bytes))?;
    }
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

pub fn get_state() -> Option<FailsafeState> {
    STATE.read().clone()
}

pub fn is_active() -> bool {
    STATE.read().as_ref().is_some_and(|s| s.active)
}

pub fn notification() -> Option<FailsafeNotification> {
    STATE
        .read()
        .as_ref()
        .filter(|s| s.active)
        .map(|s| FailsafeNotification { active: true, reason: s.reason.clone() })
}
