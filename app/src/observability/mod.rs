pub mod diagnostics;
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

pub fn set_log_filter(directive: &str) -> Result<()> {
    let handle = LOG_RELOAD.get().ok_or_else(|| anyhow!("log reload handle not initialized"))?;
    let trimmed = directive.trim();
    let filter = match trimmed.to_ascii_lowercase().as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => {
            let lvl = trimmed.to_ascii_lowercase();
            EnvFilter::try_new(format!("rustkvm={lvl},rkvm_core={lvl},rkvm_net={lvl},info"))
                .map_err(|e| anyhow!("invalid log level: {e}"))?
        }
        _ => EnvFilter::try_new(trimmed).map_err(|e| anyhow!("invalid log directive: {e}"))?,
    };
    handle.reload(filter).map_err(|e| anyhow!("failed to apply log filter: {e}"))?;
    Ok(())
}
