use anyhow::{Result, anyhow};
use tokio::sync::OnceCell;
use tracing::info;

pub mod manager;
pub mod persistence;
pub mod types;

pub use manager::ConfigManager;
pub use types::{Config, DevModeState, KeyboardMacro, KeyboardMacroStep, WakeOnLanDevice};

static CONFIG_MANAGER: OnceCell<ConfigManager> = OnceCell::const_new();

pub async fn init_config() -> Result<()> {
    let manager = ConfigManager::new();
    manager.load().await?;

    if !manager.config_exists() {
        info!("Configuration file doesn't exist, creating with default values");
        manager.save().await?;
        info!("Default configuration saved to {:?}", manager.config_path());
    }

    CONFIG_MANAGER
        .set(manager)
        .map_err(|_| anyhow::anyhow!("Config manager already initialized"))?;

    info!("Configuration manager initialized");
    Ok(())
}

pub fn get_config_manager() -> &'static ConfigManager {
    CONFIG_MANAGER.get().expect("Config manager not initialized. Call init_config() first.")
}

pub async fn get_dev_mode_state() -> Result<DevModeState> {
    let path = "/userdata/rustkvm/devmode.enable";
    let enabled = std::path::Path::new(path).exists();
    Ok(DevModeState { enabled })
}

pub async fn set_dev_mode_state(enabled: bool) -> Result<()> {
    let path = "/userdata/rustkvm/devmode.enable";
    let p = std::path::Path::new(path);
    if enabled {
        if let Some(dir) = p.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| anyhow!("mkdir failed: {}", e))?;
        }
        tokio::fs::write(p, []).await.map_err(|e| anyhow!("create file failed: {}", e))?;
    } else if tokio::fs::try_exists(p).await.unwrap_or(false) {
        tokio::fs::remove_file(p).await.map_err(|e| anyhow!("remove file failed: {}", e))?;
    }
    Ok(())
}

pub async fn validate_hardware_paths() -> Vec<String> {
    use std::path::Path;

    let critical_paths: &[(&str, &str)] = &[
        ("/dev/video0", "Video capture device"),
        ("/dev/video1", "Secondary video capture device"),
        ("/sys/class/backlight/backlight/brightness", "Backlight brightness control"),
        ("/sys/class/backlight/backlight/max_brightness", "Backlight max brightness"),
        ("/sys/class/gpio", "GPIO sysfs interface"),
        ("/sys/class/udc", "USB Device Controller sysfs"),
        ("/sys/bus/platform/drivers/dwc3", "DWC3 USB driver"),
        ("/proc/cpuinfo", "CPU information"),
        ("/userdata/rustkvm", "RustKVM user data directory"),
        ("/etc/rustkvm-version", "System version file"),
        ("/usr/bin/dropbear.sh", "Dropbear SSH control script"),
    ];

    let mut warnings = Vec::new();

    for (path, description) in critical_paths {
        let p = Path::new(path);
        if !tokio::fs::try_exists(p).await.unwrap_or(false) {
            let is_optional = path.contains("video1")
                || path.contains("rustkvm-version")
                || path.contains("dropbear.sh");
            if is_optional {
                tracing::debug!("Optional hardware path not found: {} ({})", path, description);
            } else {
                let msg = format!("Hardware path not found: {} ({})", path, description);
                tracing::warn!("{}", msg);
                warnings.push(msg);
            }
        } else {
            tracing::debug!("Hardware path found: {} ({})", path, description);
        }
    }

    let userdata = Path::new("/userdata/rustkvm");
    if tokio::fs::try_exists(userdata).await.unwrap_or(false) {
        let test_file = userdata.join(".write_test");
        match tokio::fs::write(&test_file, b"test").await {
            Ok(_) => {
                let _ = tokio::fs::remove_file(&test_file).await;
                tracing::debug!("/userdata/rustkvm is writable");
            }
            Err(e) => {
                let msg = format!("/userdata/rustkvm is not writable: {}", e);
                tracing::warn!("{}", msg);
                warnings.push(msg);
            }
        }
    }

    warnings
}
