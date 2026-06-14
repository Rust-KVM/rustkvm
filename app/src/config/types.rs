use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const MAX_MACROS_PER_DEVICE: usize = 25;
pub const MAX_STEPS_PER_MACRO: usize = 10;
pub const MAX_KEYS_PER_STEP: usize = 10;
pub const MIN_STEP_DELAY: u32 = 50;
pub const MAX_STEP_DELAY: u32 = 2000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WakeOnLanDevice {
    pub name: String,
    #[serde(rename = "macAddress", alias = "mac_address")]
    pub mac_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyboardMacroStep {
    pub keys: Vec<String>,
    pub modifiers: Vec<String>,
    pub delay: u32,
}

impl KeyboardMacroStep {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.keys.len() > MAX_KEYS_PER_STEP {
            return Err(format!("Too many keys in step (max {})", MAX_KEYS_PER_STEP));
        }

        self.delay = self.delay.clamp(MIN_STEP_DELAY, MAX_STEP_DELAY);

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyboardMacro {
    pub id: String,
    pub name: String,
    pub steps: Vec<KeyboardMacroStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<u32>,
}

impl KeyboardMacro {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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
            vendor_id: "0x1d6b".to_string(),
            product_id: "0x0104".to_string(),
            serial_number: String::new(),
            manufacturer: "RustKVM".to_string(),
            product: "USB Emulation Device".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub http_proxy: Option<String>,
    #[serde(default)]
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
        }
    }
}

fn default_cloud_url() -> String {
    "https://api.rustkvm.com".to_string()
}
fn default_cloud_app_url() -> String {
    "https://app.rustkvm.com".to_string()
}
fn default_log_level() -> String {
    "INFO".to_string()
}
fn default_device_id() -> String {
    crate::hardware::hw::get_device_id()
}
fn default_true() -> bool {
    true
}

/// Accepts a USB vendor/product id as `0x1234` or bare `1234` hex (1-4 digits).
fn is_valid_usb_id(s: &str) -> bool {
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    !hex.is_empty() && hex.len() <= 4 && u16::from_str_radix(hex, 16).is_ok()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_cloud_url")]
    pub cloud_url: String,
    #[serde(default = "default_cloud_app_url")]
    pub cloud_app_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_identity: Option<String>,
    pub jiggler_enabled: bool,
    pub auto_update_enabled: bool,
    pub include_pre_release: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hashed_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_auth_token: Option<String>,
    #[serde(alias = "localAuthMode")]
    pub local_auth_mode: String,
    pub local_loopback_only: bool,
    pub keyboard_layout: String,
    #[serde(skip_serializing_if = "Option::is_none", alias = "hdmi_edid_string")]
    pub edid_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_extension: Option<String>,
    pub display_rotation: String,
    pub display_max_brightness: u8,
    pub display_dim_after_sec: u32,
    pub display_off_after_sec: u32,
    pub tls_mode: String,
    #[serde(default = "default_log_level")]
    pub default_log_level: String,
    #[serde(default = "default_device_id")]
    pub device_id: String,

    #[serde(default)]
    pub wake_on_lan_devices: Vec<WakeOnLanDevice>,
    #[serde(default)]
    pub keyboard_macros: Vec<KeyboardMacro>,

    pub usb_config: UsbConfig,
    pub usb_devices: UsbDevices,
    pub network_config: NetworkConfig,

    #[serde(default = "default_true")]
    pub audio_enabled: bool,
    #[serde(default)]
    pub host_display_disable_when_idle: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cloud_url: default_cloud_url(),
            cloud_app_url: default_cloud_app_url(),
            cloud_token: None,
            google_identity: None,
            jiggler_enabled: false,
            auto_update_enabled: true,
            include_pre_release: false,
            hashed_password: None,
            local_auth_token: None,
            local_auth_mode: String::new(),
            local_loopback_only: false,
            keyboard_layout: "en_US".to_string(),
            edid_string: None,
            active_extension: None,
            display_rotation: "270".to_string(),
            display_max_brightness: 64,
            display_dim_after_sec: 120,
            display_off_after_sec: 1800,
            tls_mode: String::new(),
            default_log_level: default_log_level(),
            device_id: default_device_id(),
            wake_on_lan_devices: Vec::new(),
            keyboard_macros: Vec::new(),
            usb_config: UsbConfig::default(),
            usb_devices: UsbDevices::default(),
            network_config: NetworkConfig::default(),
            audio_enabled: true,
            host_display_disable_when_idle: false,
        }
    }
}

impl Config {
    pub fn validate(&mut self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.keyboard_macros.len() > MAX_MACROS_PER_DEVICE {
            errors.push(format!("Too many macros (max {})", MAX_MACROS_PER_DEVICE));
        }

        for (i, macro_item) in self.keyboard_macros.iter_mut().enumerate() {
            if let Err(e) = macro_item.validate() {
                errors.push(format!("Invalid macro {}: {}", i + 1, e));
            }
        }

        if !["0", "90", "180", "270"].contains(&self.display_rotation.as_str()) {
            errors.push(format!(
                "Invalid display rotation '{}', must be 0/90/180/270",
                self.display_rotation
            ));
        }

        if !["", "self-signed", "custom"].contains(&self.tls_mode.as_str()) {
            errors.push(format!(
                "Invalid tls_mode '{}', must be '', 'self-signed' or 'custom'",
                self.tls_mode
            ));
        }

        if !is_valid_usb_id(&self.usb_config.vendor_id) {
            errors.push(format!("Invalid USB vendor_id '{}'", self.usb_config.vendor_id));
        }
        if !is_valid_usb_id(&self.usb_config.product_id) {
            errors.push(format!("Invalid USB product_id '{}'", self.usb_config.product_id));
        }

        if !self.local_auth_mode.is_empty()
            && !["password", "noPassword"].contains(&self.local_auth_mode.as_str())
        {
            errors.push("Invalid auth mode, must be 'password' or 'noPassword'".to_string());
        }

        self.network_config.time_sync_parallel =
            self.network_config.time_sync_parallel.clamp(1, 16);

        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }

    pub fn is_setup_required(&self) -> bool {
        self.local_auth_mode.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevModeState {
    pub enabled: bool,
}
