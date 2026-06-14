use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tracing::{info, warn};

use super::MQTT_MANAGER;
use super::manager::MqttManager;
use super::stubs::{get_active_extension, get_current_atx_state, get_dc_state, get_device_id};
use super::types::{MqttConfig, MqttStatusPayload};

const MQTT_CONFIG_PATH: &str = "/userdata/rustkvm/mqtt-config.toml";

pub(super) async fn save_mqtt_config(cfg: &MqttConfig) -> Result<()> {
    let data = toml::to_string_pretty(cfg).context("Failed to serialize MQTT config to TOML")?;
    crate::config::persistence::write_atomic_secret(Path::new(MQTT_CONFIG_PATH), data.as_bytes())
        .await
        .context("Failed to write MQTT config file")?;
    info!("MQTT configuration saved");
    Ok(())
}

pub(super) async fn get_mqtt_config() -> MqttConfig {
    match tokio::fs::read_to_string(MQTT_CONFIG_PATH).await {
        Ok(data) => toml::from_str(&data).unwrap_or_else(|e| {
            warn!("Failed to parse MQTT TOML config: {}, using defaults", e);
            MqttConfig::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => MqttConfig::default(),
        Err(e) => {
            warn!("Failed to read MQTT config file: {}, using defaults", e);
            MqttConfig::default()
        }
    }
}

pub async fn start_mqtt() {
    let cfg = get_mqtt_config().await;

    if !cfg.enabled {
        info!("MQTT is disabled");
        return;
    }

    let device_id = get_device_id();

    match MqttManager::new(&cfg, &device_id).await {
        Ok(manager) => {
            let cell = MQTT_MANAGER.get_or_init(|| async { tokio::sync::Mutex::new(None) }).await;
            let mut guard = cell.lock().await;
            *guard = Some(manager);
            drop(guard);

            start_periodic_status_updates(Duration::from_secs(15)).await;

            info!("MQTT started");
        }
        Err(e) => {
            warn!("failed to start MQTT: {}", e);
        }
    }
}

pub async fn restart_mqtt(settings: &MqttConfig) -> Result<()> {
    {
        let guard =
            MQTT_MANAGER.get().ok_or_else(|| anyhow!("MQTT manager not initialized"))?.lock().await;
        if let Some(mgr) = guard.as_ref() {
            mgr.close().await;
        }
    }

    if let Some(cell) = MQTT_MANAGER.get() {
        let mut guard = cell.lock().await;
        *guard = None;
    }

    if !settings.enabled {
        info!("MQTT is disabled");
        return Ok(());
    }

    let device_id = get_device_id();
    let manager = MqttManager::new(settings, &device_id).await?;

    let cell = MQTT_MANAGER.get_or_init(|| async { tokio::sync::Mutex::new(None) }).await;
    let mut guard = cell.lock().await;
    *guard = Some(manager);
    drop(guard);

    start_periodic_status_updates(Duration::from_secs(15)).await;

    info!("MQTT restarted");
    Ok(())
}

async fn start_periodic_status_updates(interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;

            let guard = match MQTT_MANAGER.get() {
                Some(g) => g.lock().await,
                None => break,
            };

            let mgr = match guard.as_ref() {
                Some(m) => m,
                None => break,
            };

            if !mgr.is_connected() {
                drop(guard);
                continue;
            }

            mgr.publish_json(&mgr.topic(&["status"]), &MqttStatusPayload { online: true }, true)
                .await;

            let active_ext = get_active_extension();
            match active_ext.as_str() {
                "atx-power" => {
                    let state = get_current_atx_state();
                    mgr.publish_atx_state(&state).await;
                }
                "dc-power" => {
                    let state = get_dc_state();
                    mgr.publish_dc_state(&state).await;
                }
                _ => {}
            }

            mgr.publish_extended_states().await;

            drop(guard);
        }
    });
}
