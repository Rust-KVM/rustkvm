use std::time::Duration;

use anyhow::{Result, anyhow};

use super::MQTT_MANAGER;
use super::lifecycle::{get_mqtt_config, restart_mqtt, save_mqtt_config};
use super::stubs::get_device_id;
use super::tls::build_tls_config;
use super::types::{
    MQTT_PASSWORD_MASK, MqttConfig, MqttStatusResponse, MqttTestResult,
    TEST_MQTT_CONNECTION_TIMEOUT,
};

pub fn validate_base_topic(topic: &str) -> Result<()> {
    if topic.contains('+') || topic.contains('#') {
        return Err(anyhow!("base topic must not contain MQTT wildcards (+ or #)"));
    }
    if topic.contains(' ') {
        return Err(anyhow!("base topic must not contain spaces"));
    }
    if topic.is_empty() {
        return Err(anyhow!("base topic must not be empty"));
    }
    Ok(())
}

pub async fn rpc_get_mqtt_settings() -> Result<MqttConfig> {
    let cfg = get_mqtt_config().await;
    let mut masked = cfg;
    if !masked.password.is_empty() {
        masked.password = MQTT_PASSWORD_MASK.to_string();
    }
    Ok(masked)
}

pub async fn rpc_set_mqtt_settings(mut settings: MqttConfig) -> Result<()> {
    if settings.enabled && settings.broker.is_empty() {
        return Err(anyhow!("broker address is required when MQTT is enabled"));
    }
    if settings.port == 0 {
        settings.port = 1883;
    }
    if settings.base_topic.is_empty() {
        settings.base_topic = "rustkvm".to_string();
    }
    validate_base_topic(&settings.base_topic)?;

    if settings.password == MQTT_PASSWORD_MASK {
        let current = get_mqtt_config().await;
        settings.password = current.password;
    }

    let old_config = get_mqtt_config().await;
    let old_enabled = old_config.enabled;
    let old_ha_discovery = old_config.enable_ha_discovery;

    if let Some(guard) = MQTT_MANAGER.get() {
        let mgr = guard.lock().await;
        if let Some(mgr) = mgr.as_ref() {
            if old_enabled && !settings.enabled {
                mgr.cleanup_all_topics().await;
            } else if old_ha_discovery && !settings.enable_ha_discovery {
                mgr.remove_all_discovery().await;
            }
        }
    }

    save_mqtt_config(&settings).await?;

    restart_mqtt(&settings).await?;

    Ok(())
}

pub async fn rpc_get_mqtt_status() -> MqttStatusResponse {
    let guard = match MQTT_MANAGER.get() {
        Some(g) => g.lock().await,
        None => {
            return MqttStatusResponse { connected: false, error: None };
        }
    };

    match guard.as_ref() {
        Some(mgr) => MqttStatusResponse {
            connected: mgr.is_connected(),
            error: Some(mgr.last_error().await).filter(|e| !e.is_empty()),
        },
        None => MqttStatusResponse { connected: false, error: None },
    }
}

pub async fn rpc_test_mqtt_connection(settings: MqttConfig) -> Result<MqttTestResult> {
    if settings.broker.is_empty() {
        return Ok(MqttTestResult {
            success: false,
            error: Some("broker address is required".to_string()),
        });
    }

    let mut test_settings = settings;
    if test_settings.password == MQTT_PASSWORD_MASK {
        let current = get_mqtt_config().await;
        test_settings.password = current.password;
    }

    let port = if test_settings.port == 0 { 1883 } else { test_settings.port };
    let client_id = format!("rustkvm-{}-test", get_device_id());
    let test_timeout = Duration::from_secs(TEST_MQTT_CONNECTION_TIMEOUT);

    let mut mqtt_options = rumqttc::MqttOptions::new(&client_id, &test_settings.broker, port);
    mqtt_options.set_clean_session(true);
    mqtt_options.set_keep_alive(Duration::from_secs(10));

    if !test_settings.username.is_empty() {
        mqtt_options.set_credentials(&test_settings.username, &test_settings.password);
    }

    if test_settings.use_tls {
        let _tls_config = build_tls_config(test_settings.tls_insecure).await?;
    }

    let (client, _eventloop) = rumqttc::AsyncClient::new(mqtt_options, 10);

    match tokio::time::timeout(test_timeout, async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = client.publish("rustkvm/test", rumqttc::QoS::AtLeastOnce, false, b"test").await;
        client.disconnect().await.ok();
    })
    .await
    {
        Ok(()) => Ok(MqttTestResult { success: true, error: None }),
        Err(_) => {
            Ok(MqttTestResult { success: false, error: Some("connection timed out".to_string()) })
        }
    }
}
