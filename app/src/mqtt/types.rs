use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    pub enabled: bool,
    pub broker: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    #[serde(default = "default_base_topic")]
    pub base_topic: String,
    pub use_tls: bool,
    pub tls_insecure: bool,
    pub enable_ha_discovery: bool,
    pub enable_actions: bool,
    #[serde(default)]
    pub debounce_ms: u64,
}

fn default_base_topic() -> String {
    "rustkvm".to_string()
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            broker: String::new(),
            port: 1883,
            username: String::new(),
            password: String::new(),
            base_topic: "rustkvm".to_string(),
            use_tls: false,
            tls_insecure: false,
            enable_ha_discovery: true,
            enable_actions: false,
            debounce_ms: 0,
        }
    }
}

impl MqttConfig {
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.broker.is_empty() {
            return Err(anyhow!("broker address is required when MQTT is enabled"));
        }
        if self.port == 0 {
            return Err(anyhow!("port must be between 1 and 65535"));
        }
        if self.base_topic.contains('+') || self.base_topic.contains('#') {
            return Err(anyhow!("base topic must not contain MQTT wildcards (+ or #)"));
        }
        if self.base_topic.contains(' ') {
            return Err(anyhow!("base topic must not contain spaces"));
        }
        if self.base_topic.is_empty() {
            return Err(anyhow!("base topic must not be empty"));
        }
        Ok(())
    }
}

pub const MQTT_PASSWORD_MASK: &str = "********";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttStatusPayload {
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttVideoState {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub width: u32,
    pub height: u32,
    #[serde(rename = "fps")]
    pub fps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttUsbState {
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttCloudState {
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttSessionsState {
    pub active_sessions: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttJigglerState {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttNetworkState {
    pub ip_address: String,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttUpdateState {
    pub installed_version: String,
    pub latest_version: String,
    pub in_progress: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_percentage: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttSystemState {
    pub cpu_load: f64,
    pub temperature: f64,
    pub memory_used: u64,
    pub memory_total: u64,
    pub storage_used: i64,
    pub storage_free: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttVirtualMediaState {
    pub mounted_image: String,
    pub source: String,
}

pub use crate::power::{AtxState, DcPowerState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaDevice {
    pub identifiers: Vec<String>,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sw_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HaDiscoveryPayload {
    pub name: String,
    pub unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_template: Option<String>,
    pub availability_topic: String,
    pub availability_template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<HaDevice>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_measurement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_off: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_press: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_by_default: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_attributes_topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_attributes_template: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version_topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_install: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttStatusResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttTestResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(super) const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(600);

pub(super) const TEST_MQTT_CONNECTION_TIMEOUT: u64 = 5;

pub(super) fn build_topic(base: &str, parts: &[&str]) -> String {
    let mut result = base.to_string();
    for part in parts {
        use std::fmt::Write;
        write!(&mut result, "/{}", part).ok();
    }
    result
}
