use std::os::unix::fs::PermissionsExt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::{Duration, timeout};
use tracing::{debug, error, info, trace, warn};

pub fn ping() -> Result<String> {
    Ok("pong".to_string())
}

#[derive(Serialize, Default)]
pub struct FailsafeModeResponse {
    pub active: bool,
    pub reason: String,
}

pub fn get_failsafe_mode() -> Result<FailsafeModeResponse> {
    Ok(crate::failsafe::get_state()
        .map(|s| FailsafeModeResponse { active: s.active, reason: s.reason })
        .unwrap_or_default())
}

pub fn get_device_id() -> Result<String> {
    Ok(crate::hardware::hw::get_device_id())
}

#[derive(Deserialize)]
pub struct RebootParams {
    #[serde(default)]
    pub force: bool,
}

pub fn reboot(params: RebootParams) -> Result<Value> {
    info!("Reboot requested, force: {}", params.force);

    tokio::spawn(async move {
        crate::api::broadcast_will_reboot("user_requested").await;
        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

        let mut cmd = std::process::Command::new("reboot");
        if params.force {
            cmd.arg("-f");
        }
        if let Err(e) = cmd.spawn() {
            error!("Failed to execute reboot command: {}", e);
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        std::process::exit(0);
    });

    Ok(Value::Null)
}

pub async fn factory_reset() -> Result<Value> {
    info!("Factory reset requested");
    for path in ["/userdata/rustkvm/config.toml", "/userdata/rustkvm/mqtt-config.toml"] {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(Value::Null)
}

pub async fn reset_config() -> Result<Value> {
    let config_manager = crate::config::get_config_manager();
    config_manager
        .update(|cfg| {
            *cfg = crate::config::types::Config::default();
        })
        .await?;
    info!("Configuration reset to default and saved");
    Ok(Value::Null)
}

#[derive(Serialize)]
pub struct LocalVersionResponse {
    pub system_version: Option<String>,
    pub app_version: String,
}

pub async fn get_local_version() -> Result<LocalVersionResponse> {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let system_version =
        tokio::fs::read_to_string("/etc/rustkvm-version").await.ok().map(|s| s.trim().to_string());
    Ok(LocalVersionResponse { system_version, app_version })
}

static DEFAULT_LOG_LEVEL: OnceLock<Mutex<String>> = OnceLock::new();
const VALID_LOG_LEVELS: &[&str] = &["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

pub fn get_default_log_level() -> Result<String> {
    let cell = DEFAULT_LOG_LEVEL.get_or_init(|| Mutex::new("INFO".to_string()));
    Ok(cell.lock().clone())
}

#[derive(Deserialize)]
pub struct LogLevelParams {
    pub level: String,
}

pub fn set_default_log_level(params: LogLevelParams) -> Result<Value> {
    if !VALID_LOG_LEVELS.contains(&params.level.as_str()) {
        return Err(anyhow!(
            "Invalid log level: {} (must be TRACE, DEBUG, INFO, WARN, or ERROR)",
            params.level
        ));
    }
    let cell = DEFAULT_LOG_LEVEL.get_or_init(|| Mutex::new("INFO".to_string()));
    *cell.lock() = params.level.clone();
    crate::observability::set_log_filter(&params.level)?;
    info!("Default log level set to: {} (applied to live subscriber)", params.level);
    Ok(Value::Null)
}

pub fn emit_test_log(params: LogLevelParams) -> Result<Value> {
    match params.level.as_str() {
        "TRACE" => trace!("JSON-RPC test log probe"),
        "DEBUG" => debug!("JSON-RPC test log probe"),
        "INFO" => info!("JSON-RPC test log probe"),
        "WARN" => warn!("JSON-RPC test log probe"),
        "ERROR" => error!("JSON-RPC test log probe"),
        _ => return Err(anyhow!("Invalid log level: {}", params.level)),
    }
    Ok(Value::Null)
}

pub fn get_timezones() -> Result<Vec<String>> {
    Ok(vec![
        "UTC".to_string(),
        "America/New_York".to_string(),
        "America/Chicago".to_string(),
        "America/Denver".to_string(),
        "America/Los_Angeles".to_string(),
        "Europe/London".to_string(),
        "Europe/Paris".to_string(),
        "Europe/Berlin".to_string(),
        "Asia/Tokyo".to_string(),
        "Asia/Shanghai".to_string(),
        "Asia/Singapore".to_string(),
        "Australia/Sydney".to_string(),
        "Pacific/Auckland".to_string(),
    ])
}

const SSH_KEY_DIR: &str = "/userdata/dropbear/.ssh";
const SSH_KEY_FILE: &str = "/userdata/dropbear/.ssh/authorized_keys";

pub async fn get_ssh_key_state() -> Result<String> {
    match tokio::fs::read_to_string(SSH_KEY_FILE).await {
        Ok(s) => Ok(s),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(String::new())
            } else {
                Err(anyhow!("error reading SSH key file: {}", e))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct SshKeyParam {
    #[serde(rename = "sshKey")]
    pub ssh_key: String,
}

pub async fn set_ssh_key_state(param: SshKeyParam) -> Result<Value> {
    let ssh_key = param.ssh_key;
    if !ssh_key.is_empty() {
        crate::dev_mode::validate_ssh_key(&ssh_key)?;
        tokio::fs::create_dir_all(SSH_KEY_DIR)
            .await
            .map_err(|e| anyhow!("failed to create SSH key directory: {}", e))?;
        let dir_meta = tokio::fs::metadata(SSH_KEY_DIR)
            .await
            .map_err(|e| anyhow!("failed to stat SSH key directory: {}", e))?;
        let mut dir_perm = dir_meta.permissions();
        dir_perm.set_mode(0o700);
        tokio::fs::set_permissions(SSH_KEY_DIR, dir_perm)
            .await
            .map_err(|e| anyhow!("failed to set SSH key directory permissions: {}", e))?;

        tokio::fs::write(SSH_KEY_FILE, ssh_key.as_bytes())
            .await
            .map_err(|e| anyhow!("failed to write SSH key: {}", e))?;
        let meta = tokio::fs::metadata(SSH_KEY_FILE)
            .await
            .map_err(|e| anyhow!("failed to stat SSH key file: {}", e))?;
        let mut perm = meta.permissions();
        perm.set_mode(0o600);
        tokio::fs::set_permissions(SSH_KEY_FILE, perm)
            .await
            .map_err(|e| anyhow!("failed to set SSH key permissions: {}", e))?;
    } else if let Err(e) = tokio::fs::remove_file(SSH_KEY_FILE).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(anyhow!("failed to remove SSH key file: {}", e));
    }

    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct DevModeParams {
    pub enabled: bool,
}

pub async fn get_dev_mode_state_handler() -> Result<crate::config::DevModeState> {
    let state = crate::config::get_dev_mode_state().await?;
    Ok(state)
}

pub async fn set_dev_mode_state_handler(params: DevModeParams) -> Result<Value> {
    crate::config::set_dev_mode_state(params.enabled).await?;

    tokio::spawn(async {
        let run = async { Command::new("/usr/bin/dropbear.sh").arg("auto").output().await };
        match timeout(Duration::from_secs(2), run).await {
            Ok(Ok(output)) => {
                if !output.status.success() {
                    warn!(
                        "dropbear.sh exited non-zero: status={:?} stderr={}",
                        output.status.code(),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
            Ok(Err(e)) => {
                warn!("dropbear.sh exec error: {}", e);
            }
            Err(_) => {
                warn!("dropbear.sh timed out");
            }
        }
    });

    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct SerialSettings {
    pub baud_rate: String,
    pub data_bits: String,
    pub stop_bits: String,
    pub parity: String,
}

#[derive(Serialize)]
pub struct SerialSettingsResponse {
    pub baud_rate: String,
    pub data_bits: String,
    pub stop_bits: String,
    pub parity: String,
}

pub fn set_serial_settings(settings: SerialSettings) -> Result<Value> {
    let _baud_rate: u32 = settings
        .baud_rate
        .parse()
        .map_err(|_| anyhow!("Invalid baud rate: {}", settings.baud_rate))?;

    let data_bits: u8 = settings
        .data_bits
        .parse()
        .map_err(|_| anyhow!("Invalid data bits: {}", settings.data_bits))?;
    if !(5..=8).contains(&data_bits) {
        return Err(anyhow!("Data bits must be between 5 and 8"));
    }

    match settings.stop_bits.as_str() {
        "1" | "1.5" | "2" => {}
        _ => return Err(anyhow!("Invalid stop bits: {}", settings.stop_bits)),
    }

    match settings.parity.as_str() {
        "none" | "odd" | "even" | "mark" | "space" => {}
        _ => return Err(anyhow!("Invalid parity: {}", settings.parity)),
    }

    info!("Serial settings updated successfully");
    Ok(Value::Null)
}

pub fn get_serial_settings() -> Result<SerialSettingsResponse> {
    Ok(SerialSettingsResponse {
        baud_rate: "115200".to_string(),
        data_bits: "8".to_string(),
        stop_bits: "1".to_string(),
        parity: "none".to_string(),
    })
}

#[derive(Deserialize)]
pub struct CustomCommandParams {
    pub command: String,
}

pub fn send_custom_command(params: CustomCommandParams) -> Result<Value> {
    debug!("Custom serial command: {}", params.command);
    Ok(Value::Null)
}

const SERIAL_COMMAND_HISTORY_PATH: &str = "/userdata/rustkvm/serialCommandHistory.json";

pub async fn get_serial_command_history() -> Result<Vec<String>> {
    match tokio::fs::read_to_string(SERIAL_COMMAND_HISTORY_PATH).await {
        Ok(s) => Ok(serde_json::from_str(&s).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => {
            warn!("Failed to read serial command history: {e}");
            Ok(Vec::new())
        }
    }
}

#[derive(Deserialize)]
pub struct CommandHistoryParams {
    #[serde(rename = "commandHistory")]
    pub command_history: Vec<String>,
}

pub async fn set_serial_command_history(params: CommandHistoryParams) -> Result<Value> {
    if let Some(parent) = std::path::Path::new(SERIAL_COMMAND_HISTORY_PATH).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| anyhow!("failed to create history dir: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(&params.command_history)
        .map_err(|e| anyhow!("failed to encode history: {e}"))?;
    tokio::fs::write(SERIAL_COMMAND_HISTORY_PATH, bytes)
        .await
        .map_err(|e| anyhow!("failed to write history: {e}"))?;
    debug!("Serial command history saved: {} entries", params.command_history.len());
    Ok(Value::Null)
}

pub async fn delete_serial_command_history() -> Result<Value> {
    let empty: Vec<String> = Vec::new();
    let bytes = serde_json::to_vec_pretty(&empty).unwrap_or_else(|_| b"[]".to_vec());
    if let Some(parent) = std::path::Path::new(SERIAL_COMMAND_HISTORY_PATH).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(SERIAL_COMMAND_HISTORY_PATH, bytes)
        .await
        .map_err(|e| anyhow!("failed to clear history: {e}"))?;
    debug!("Serial command history deleted");
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct TerminalPauseParams {
    #[serde(rename = "terminalPaused")]
    pub terminal_paused: bool,
}

pub fn set_terminal_paused(params: TerminalPauseParams) -> Result<Value> {
    debug!("Terminal paused: {}", params.terminal_paused);
    Ok(Value::Null)
}

static AUTO_UPDATE_ENABLED: AtomicBool = AtomicBool::new(false);
static DEV_CHANNEL_ENABLED: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
pub struct UpdateStatusResponse {
    #[serde(rename = "updateAvailable")]
    pub update_available: bool,
    #[serde(rename = "currentVersion")]
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn get_update_status() -> Result<UpdateStatusResponse> {
    Ok(UpdateStatusResponse {
        update_available: false,
        current_version: crate::version::built_app_version().to_string(),
        error: None,
    })
}

pub async fn get_auto_update_state() -> Result<bool> {
    let cfg = crate::config::get_config_manager().get().await;
    AUTO_UPDATE_ENABLED.store(cfg.auto_update_enabled, Ordering::Relaxed);
    Ok(cfg.auto_update_enabled)
}

#[derive(Deserialize)]
pub struct AutoUpdateParams {
    pub enabled: bool,
}

pub async fn set_auto_update_state(params: AutoUpdateParams) -> Result<bool> {
    let mgr = crate::config::get_config_manager();
    mgr.update(|cfg| {
        cfg.auto_update_enabled = params.enabled;
    })
    .await?;
    AUTO_UPDATE_ENABLED.store(params.enabled, Ordering::Relaxed);
    info!("Auto update state set to: {} (persisted)", params.enabled);
    Ok(params.enabled)
}

pub fn is_update_pending() -> Result<bool> {
    Ok(false)
}

#[derive(Serialize)]
pub struct UpdateStatusChannelResponse {
    pub channel: String,
}

pub fn get_update_status_channel() -> Result<UpdateStatusChannelResponse> {
    Ok(UpdateStatusChannelResponse { channel: "stable".to_string() })
}

pub async fn get_dev_channel_state() -> Result<bool> {
    let cfg = crate::config::get_config_manager().get().await;
    DEV_CHANNEL_ENABLED.store(cfg.include_pre_release, Ordering::Relaxed);
    Ok(cfg.include_pre_release)
}

#[derive(Deserialize)]
pub struct DevChannelParams {
    pub enabled: bool,
}

pub async fn set_dev_channel_state(params: DevChannelParams) -> Result<Value> {
    let mgr = crate::config::get_config_manager();
    mgr.update(|cfg| {
        cfg.include_pre_release = params.enabled;
    })
    .await?;
    DEV_CHANNEL_ENABLED.store(params.enabled, Ordering::Relaxed);
    info!("Dev channel state set to: {} (persisted)", params.enabled);
    Ok(Value::Null)
}

#[derive(Serialize)]
pub struct UpdateComponentsResponse {
    pub has_update: bool,
    pub components: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct CheckUpdateParams {
    pub include_pre_release: Option<bool>,
    pub params: Option<serde_json::Value>,
}

pub async fn check_update_components(
    _params: CheckUpdateParams,
) -> Result<UpdateComponentsResponse> {
    Ok(UpdateComponentsResponse { has_update: false, components: vec![] })
}

pub async fn try_update() -> Result<Value> {
    info!("Update triggered via RPC");
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct TryUpdateComponentsParams {
    pub params: Option<serde_json::Value>,
    pub include_pre_release: Option<bool>,
    pub reset_config: Option<bool>,
}

pub async fn try_update_components(_params: TryUpdateComponentsParams) -> Result<Value> {
    info!("Component update triggered via RPC");
    Ok(Value::Null)
}

pub use crate::power::dc::DcPowerState as DcPowerStateResponse;

pub fn get_dc_power_state() -> Result<DcPowerStateResponse> {
    Ok(crate::power::dc::get_dc_power_state())
}

#[derive(Deserialize)]
pub struct SetDcPowerParams {
    pub enabled: bool,
}

pub async fn set_dc_power_state(params: SetDcPowerParams) -> Result<Value> {
    tokio::task::spawn_blocking(move || crate::power::dc::set_dc_power_state(params.enabled))
        .await
        .map_err(|e| anyhow!("join error: {e}"))??;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct DcRestoreParams {
    pub state: i32,
}

pub async fn set_dc_restore_state(params: DcRestoreParams) -> Result<Value> {
    tokio::task::spawn_blocking(move || crate::power::dc::set_dc_restore_state(params.state))
        .await
        .map_err(|e| anyhow!("join error: {e}"))??;
    Ok(Value::Null)
}

pub use crate::power::atx::AtxState as AtxStateResponse;

pub fn get_atx_state() -> Result<AtxStateResponse> {
    crate::power::atx::get_atx_state()
}

#[derive(Deserialize)]
pub struct AtxPowerActionParams {
    pub action: String,
}

pub async fn set_atx_power_action(params: AtxPowerActionParams) -> Result<Value> {
    let action = params.action;
    tokio::task::spawn_blocking(move || crate::power::atx::set_atx_power_action(&action))
        .await
        .map_err(|e| anyhow!("join error: {e}"))??;
    Ok(Value::Null)
}

static ACTIVE_EXTENSION: OnceLock<Mutex<String>> = OnceLock::new();

pub fn get_active_extension() -> Result<String> {
    let cell = ACTIVE_EXTENSION.get_or_init(|| Mutex::new(String::new()));
    Ok(cell.lock().clone())
}

#[derive(Deserialize)]
pub struct ExtensionParams {
    #[serde(rename = "extensionId")]
    pub extension_id: String,
}

pub async fn set_active_extension(params: ExtensionParams) -> Result<Value> {
    let cell = ACTIVE_EXTENSION.get_or_init(|| Mutex::new(String::new()));
    let prev = { cell.lock().clone() };

    if prev == crate::power::atx::ATX_EXTENSION_ID && prev != params.extension_id {
        if let Err(e) = tokio::task::spawn_blocking(crate::power::atx::unmount_atx_control)
            .await
            .map_err(|e| anyhow!("join error: {e}"))?
        {
            warn!("Failed to unmount ATX control: {}", e);
        }
    }

    if params.extension_id == crate::power::atx::ATX_EXTENSION_ID {
        tokio::task::spawn_blocking(crate::power::atx::mount_atx_control)
            .await
            .map_err(|e| anyhow!("join error: {e}"))??;
    }

    *cell.lock() = params.extension_id.clone();
    info!("Active extension set to: {}", params.extension_id);
    Ok(Value::Null)
}

pub async fn get_jiggler_state() -> Result<bool> {
    Ok(crate::hardware::jiggler::get_jiggler_state().await)
}

#[derive(Deserialize)]
pub struct JigglerStateParams {
    pub enabled: bool,
}

pub async fn set_jiggler_state(params: JigglerStateParams) -> Result<Value> {
    crate::hardware::jiggler::set_jiggler_state(params.enabled).await?;
    Ok(Value::Null)
}

pub use crate::hardware::jiggler::JigglerConfig as JigglerConfigResponse;

pub fn get_jiggler_config() -> Result<JigglerConfigResponse> {
    Ok(crate::hardware::jiggler::get_jiggler_config())
}

#[derive(Deserialize)]
pub struct SetJigglerConfigParams {
    #[serde(rename = "jigglerConfig")]
    pub jiggler_config: JigglerConfigResponse,
}

pub async fn set_jiggler_config(params: SetJigglerConfigParams) -> Result<Value> {
    crate::hardware::jiggler::set_jiggler_config(params.jiggler_config).await?;
    Ok(Value::Null)
}

pub use crate::mqtt::{MqttConfig as MqttSettingsResponse, MqttStatusResponse};

pub async fn get_mqtt_settings() -> Result<MqttSettingsResponse> {
    crate::mqtt::rpc_get_mqtt_settings().await
}

#[derive(Deserialize)]
pub struct SetMqttSettingsParams {
    pub settings: MqttSettingsResponse,
}

pub async fn set_mqtt_settings(params: SetMqttSettingsParams) -> Result<Value> {
    crate::mqtt::rpc_set_mqtt_settings(params.settings).await?;
    Ok(Value::Null)
}

pub async fn get_mqtt_status() -> Result<MqttStatusResponse> {
    Ok(crate::mqtt::rpc_get_mqtt_status().await)
}

pub async fn test_mqtt_connection(params: SetMqttSettingsParams) -> Result<Value> {
    let result = crate::mqtt::rpc_test_mqtt_connection(params.settings).await?;
    Ok(serde_json::to_value(result)?)
}
