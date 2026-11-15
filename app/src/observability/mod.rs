pub mod diagnostics;
pub mod health;
pub mod log_buffer;
pub mod log_file;
pub mod metrics;

use anyhow::{Result, anyhow};
pub use diagnostics::get_system_snapshot;
pub use metrics::init_prometheus;
use once_cell::sync::OnceCell;
use tracing_subscriber::Registry;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::reload::Handle;

type ReloadHandle = Handle<EnvFilter, Registry>;
static LOG_RELOAD: OnceCell<ReloadHandle> = OnceCell::new();

pub fn install_log_reload_handle(handle: ReloadHandle) {
    let _ = LOG_RELOAD.set(handle);
}

/// The filter for a plain level name: that level for our own crates, `info`
/// for dependencies. The WebRTC stack's ICE, SCTP and peer-connection modules
/// log a dozen warnings on every ordinary disconnect ("agent is closed",
/// "conn is closed"), so only their errors are kept.
pub fn default_filter_directive(level: &str) -> String {
    format!(
        "rustkvm={level},rkvm_core={level},rkvm_net={level},rkvm_vision={level},\
         webrtc_ice=error,webrtc_sctp=error,webrtc::peer_connection=error,info"
    )
}

pub fn set_log_filter(directive: &str) -> Result<()> {
    let handle = LOG_RELOAD.get().ok_or_else(|| anyhow!("log reload handle not initialized"))?;
    let trimmed = directive.trim();
    let filter = match trimmed.to_ascii_lowercase().as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => {
            EnvFilter::try_new(default_filter_directive(&trimmed.to_ascii_lowercase()))
                .map_err(|e| anyhow!("invalid log level: {e}"))?
        }
        _ => EnvFilter::try_new(trimmed).map_err(|e| anyhow!("invalid log directive: {e}"))?,
    };
    handle.reload(filter).map_err(|e| anyhow!("failed to apply log filter: {e}"))?;
    Ok(())
}
