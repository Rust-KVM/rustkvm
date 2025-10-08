use std::collections::HashMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// Constants for keyboard macro limits
pub const MAX_MACROS_PER_DEVICE: usize = 25;
pub const MAX_STEPS_PER_MACRO: usize = 10;
pub const MAX_KEYS_PER_STEP: usize = 10;
pub const MIN_STEP_DELAY: u32 = 50;
pub const MAX_STEP_DELAY: u32 = 2000;

/// Wake-on-LAN device configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WakeOnLanDevice {
    pub name: String,
    #[serde(rename = "macAddress", alias = "mac_address")]
    pub mac_address: String,
}

/// Keyboard macro step configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyboardMacroStep {
    pub keys: Vec<String>,
    pub modifiers: Vec<String>,
    pub delay: u32,
}

impl KeyboardMacroStep {
    /// Validate and normalize the step configuration
    pub fn validate(&mut self) -> Result<(), String> {
        if self.keys.len() > MAX_KEYS_PER_STEP {
            return Err(format!("Too many keys in step (max {})", MAX_KEYS_PER_STEP));
        }

        self.delay = self.delay.clamp(MIN_STEP_DELAY, MAX_STEP_DELAY);

        Ok(())
    }
}

/// Keyboard macro configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyboardMacro {
    pub id: String,
    pub name: String,
    pub steps: Vec<KeyboardMacroStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<u32>,
}

impl KeyboardMacro {
    /// Validate the macro configuration
    pub fn validate(&mut self) -> anyhow::Result<()> {
        if self.name.trim().is_empty() {
            bail!("Macro name cannot be empty");
        }

        if self.steps.is_empty() {
            bail!("Macro must have at least one step");
        }

        if self.steps.len() > MAX_STEPS_PER_MACRO {
            bail!("Too many steps in macro (max {})", MAX_STEPS_PER_MACRO);
        }

        for (i, step) in self.steps.iter_mut().enumerate() {
            if let Err(e) = step.validate() {
                bail!("Invalid step {}: {}", i + 1, e);
            }
        }

        Ok(())
    }
}

/// USB gadget configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsbConfig {
    pub vendor_id: String,
    pub product_id: String,
    pub serial_number: String,
    pub manufacturer: String,
    pub product: String,
}

impl Default for UsbConfig {
    fn default() -> Self {
        Self {
            vendor_id: "0x1d6b".to_string(),  // The Linux Foundation
            product_id: "0x0104".to_string(), // Multifunction Composite Gadget
            serial_number: String::new(),
            manufacturer: "RustKVM".to_string(),
            product: "USB Emulation Device".to_string(),
        }
    }
}

/// USB device capabilities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsbDevices {
    pub absolute_mouse: bool,
    pub relative_mouse: bool,
    pub keyboard: bool,
    pub mass_storage: bool,
}

impl Default for UsbDevices {
    fn default() -> Self {
        Self { absolute_mouse: true, relative_mouse: true, keyboard: true, mass_storage: true }
    }
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkConfig {
    #[serde(default, deserialize_with = "deserialize_null_as_none")]
    pub hostname: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_as_none")]
    pub http_proxy: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_as_none")]
    pub domain: Option<String>,
    #[serde(default = "default_ipv4_mode")]
    pub ipv4_mode: String,
    #[serde(default = "default_ipv6_mode")]
    pub ipv6_mode: String,
    #[serde(default = "default_lldp_mode")]
    pub lldp_mode: String,
    #[serde(default)]
    pub lldp_tx_tlvs: Vec<String>,
    #[serde(default = "default_mdns_mode")]
    pub mdns_mode: String,
    #[serde(default = "default_time_sync_mode")]
    pub time_sync_mode: String,
    #[serde(default)]
    pub time_sync_ordering: Vec<String>,
    #[serde(default)]
    pub time_sync_disable_fallback: bool,
    #[serde(default = "default_time_sync_parallel")]
    pub time_sync_parallel: u32,
    #[serde(flatten)]
    pub additional_settings: HashMap<String, serde_json::Value>,
}

fn deserialize_null_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::deserialize(deserializer)?.flatten())
}

fn deserialize_null_default_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let opt = Option::<Vec<T>>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

fn default_ipv4_mode() -> String {
    "dhcp".to_string()
}
fn default_ipv6_mode() -> String {
    "slaac".to_string()
}
fn default_lldp_mode() -> String {
    "basic".to_string()
}
fn default_mdns_mode() -> String {
    "auto".to_string()
}
fn default_time_sync_mode() -> String {
    "ntp_and_http".to_string()
}
fn default_time_sync_parallel() -> u32 {
    4
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            hostname: None,
            http_proxy: None,
            domain: None,
            ipv4_mode: default_ipv4_mode(),
            ipv6_mode: default_ipv6_mode(),
            lldp_mode: default_lldp_mode(),
            lldp_tx_tlvs: vec![
                "chassis".to_string(),
                "port".to_string(),
                "system".to_string(),
                "vlan".to_string(),
            ],
            mdns_mode: default_mdns_mode(),
            time_sync_mode: default_time_sync_mode(),
            time_sync_ordering: vec!["ntp".to_string(), "http".to_string()],
            time_sync_disable_fallback: false,
            time_sync_parallel: default_time_sync_parallel(),
            additional_settings: HashMap::new(),
        }
    }
}

/// Main application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // Cloud settings
    pub cloud_url: String,
    pub cloud_app_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_identity: Option<String>,

    // Feature flags
    pub jiggler_enabled: bool,
    pub auto_update_enabled: bool,
    pub include_pre_release: bool,

    // Authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hashed_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_auth_token: Option<String>,
    #[serde(alias = "localAuthMode")]
    pub local_auth_mode: String,
    pub local_loopback_only: bool,

    // Device configuration
    #[serde(default, deserialize_with = "deserialize_null_default_vec")]
    pub wake_on_lan_devices: Vec<WakeOnLanDevice>,
    #[serde(default)]
    pub keyboard_macros: Vec<KeyboardMacro>,
    pub keyboard_layout: String,
    #[serde(skip_serializing_if = "Option::is_none", alias = "hdmi_edid_string")]
    pub edid_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_extension: Option<String>,

    // Display settings
    pub display_rotation: String,
    pub display_max_brightness: u32,
    pub display_dim_after_sec: u32,
    pub display_off_after_sec: u32,

    // TLS configuration
    pub tls_mode: String,

    // USB configuration
    pub usb_config: UsbConfig,
    pub usb_devices: UsbDevices,

    // Network configuration
    pub network_config: NetworkConfig,

    // Logging
    pub default_log_level: String,

    // Device identity
    pub device_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cloud_url: "https://api.rustkvm.com".to_string(),
            cloud_app_url: "https://app.rustkvm.com".to_string(),
            cloud_token: None,
            google_identity: None,
            jiggler_enabled: false,
            auto_update_enabled: true,
            include_pre_release: false,
            hashed_password: None,
            local_auth_token: None,
            local_auth_mode: "noPassword".to_string(),
            local_loopback_only: false,
            wake_on_lan_devices: Vec::new(),
            keyboard_macros: Vec::new(),
            keyboard_layout: "en_US".to_string(),
            edid_string: None,
            active_extension: None,
            display_rotation: "270".to_string(),
            display_max_brightness: 64,
            display_dim_after_sec: 120,  // 2 minutes
            display_off_after_sec: 1800, // 30 minutes
            tls_mode: String::new(),
            usb_config: UsbConfig::default(),
            usb_devices: UsbDevices::default(),
            network_config: NetworkConfig::default(),
            default_log_level: "INFO".to_string(),
            device_id: crate::hardware::hw::get_device_id(),
        }
    }
}

impl Config {
    /// Validate the entire configuration
    pub fn validate(&mut self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Validate keyboard macros
        if self.keyboard_macros.len() > MAX_MACROS_PER_DEVICE {
            errors.push(format!("Too many macros (max {})", MAX_MACROS_PER_DEVICE));
        }

        for (i, macro_item) in self.keyboard_macros.iter_mut().enumerate() {
            if let Err(e) = macro_item.validate() {
                errors.push(format!("Invalid macro {}: {}", i + 1, e));
            }
        }

        // Validate display settings
        if self.display_max_brightness > 255 {
            errors.push("Display max brightness cannot exceed 255".to_string());
        }

        // Validate auth mode
        if !["password", "noPassword"].contains(&self.local_auth_mode.as_str()) {
            errors.push("Invalid auth mode, must be 'password' or 'noPassword'".to_string());
        }

        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }

    /// Check if device requires setup
    pub fn is_setup_required(&self) -> bool {
        self.local_auth_mode.is_empty()
    }
}

/// Developer mode state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevModeState {
    pub enabled: bool,
}
