use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::descriptors::{
    ABSOLUTE_MOUSE_REPORT_DESC, KEYBOARD_REPORT_DESC, RELATIVE_MOUSE_REPORT_DESC,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GadgetConfig {
    pub vendor_id: String,
    pub product_id: String,
    pub serial_number: String,
    pub manufacturer: String,
    pub product: String,
    pub strict_mode: bool,
}

impl Default for GadgetConfig {
    fn default() -> Self {
        Self {
            vendor_id: "0x1d6b".to_string(),
            product_id: "0x0104".to_string(),
            serial_number: String::new(),
            manufacturer: "RustKVM".to_string(),
            product: "RustKVM USB Emulation Device".to_string(),
            strict_mode: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub absolute_mouse: bool,
    pub relative_mouse: bool,
    pub keyboard: bool,
    pub mass_storage: bool,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self { absolute_mouse: true, relative_mouse: true, keyboard: true, mass_storage: true }
    }
}

#[derive(Debug, Clone)]
struct GadgetConfigItem {
    order: u32,
    path: Vec<String>,
    attrs: HashMap<String, String>,
    config_attrs: HashMap<String, String>,
    config_path: Option<Vec<String>>,
    report_desc: Option<Vec<u8>>,
}

pub struct UsbGadget {
    name: String,
    udc: String,
    kvm_gadget_path: PathBuf,
    config_c1_path: PathBuf,
    config_map: HashMap<String, GadgetConfigItem>,
    custom_config: GadgetConfig,
    enabled_devices: DeviceConfig,
}

impl UsbGadget {
    pub fn new(name: String, enabled_devices: DeviceConfig, config: GadgetConfig) -> Result<Self> {
        let udc = Self::get_udc()?;
        let kvm_gadget_path = PathBuf::from("/sys/kernel/config/usb_gadget").join(&name);
        let config_c1_path = kvm_gadget_path.join("configs/c.1");

        let mut gadget = Self {
            name,
            udc,
            kvm_gadget_path,
            config_c1_path,
            config_map: Self::create_default_config_map(),
            custom_config: config,
            enabled_devices,
        };

        gadget.load_gadget_config();
        Ok(gadget)
    }

    pub fn init(&self) -> Result<()> {
        let udcs = Self::get_udcs();
        if udcs.is_empty() {
            return Err(anyhow!("no UDC found, skipping USB stack init"));
        }

        Self::force_unbind_conflicting_gadgets(&self.udc, &self.name);

        self.configure_usb_gadget(false)?;
        info!("USB gadget initialized successfully");
        Ok(())
    }

    pub fn get_usb_state(&self) -> String {
        let state_file = PathBuf::from("/sys/class/udc").join(&self.udc).join("state");

        std::fs::read_to_string(&state_file)
            .map(|content| content.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    pub fn unbind_udc(&self) -> Result<()> {
        let udc_file = self.kvm_gadget_path.join("UDC");
        std::fs::write(&udc_file, "").with_context(|| {
            format!("failed to unbind gadget from UDC via {}", udc_file.display())
        })?;
        Ok(())
    }

    pub fn get_udc_name(&self) -> &str {
        &self.udc
    }

    fn get_udc() -> Result<String> {
        let udcs = Self::get_udcs();
        udcs.into_iter().next().ok_or_else(|| anyhow!("no UDC found"))
    }

    fn get_udcs() -> Vec<String> {
        let mut udcs: Vec<String> = Vec::new();

        if let Ok(entries) = std::fs::read_dir("/sys/devices/platform/usbdrd") {
            for e in entries.flatten() {
                if let Ok(ft) = e.file_type()
                    && ft.is_dir()
                    && let Some(name) = e.file_name().to_str()
                    && name.ends_with(".usb")
                {
                    udcs.push(name.to_string());
                }
            }
            if !udcs.is_empty() {
                return udcs;
            }
        }

        if let Ok(entries) = std::fs::read_dir("/sys/class/udc") {
            for e in entries.flatten() {
                if let Some(name) = e.file_name().to_str() {
                    udcs.push(name.to_string());
                }
            }
        }

        udcs
    }

    fn create_default_config_map() -> HashMap<String, GadgetConfigItem> {
        let mut config_map = HashMap::new();

        config_map.insert(
            "base".to_string(),
            GadgetConfigItem {
                order: 0,
                path: Vec::new(),
                attrs: HashMap::from([
                    ("bcdUSB".to_string(), "0x0320".to_string()),
                    ("idVendor".to_string(), "0x1d6b".to_string()),
                    ("idProduct".to_string(), "0x0104".to_string()),
                    ("bcdDevice".to_string(), "0x0100".to_string()),
                ]),
                config_attrs: HashMap::new(),
                config_path: None,
                report_desc: None,
            },
        );

        config_map.insert(
            "base_info".to_string(),
            GadgetConfigItem {
                order: 1,
                path: vec!["strings".to_string(), "0x409".to_string()],
                attrs: HashMap::from([
                    ("serialnumber".to_string(), String::new()),
                    ("manufacturer".to_string(), "RustKVM".to_string()),
                    ("product".to_string(), "RustKVM USB Emulation Device".to_string()),
                ]),
                config_attrs: HashMap::from([(
                    "configuration".to_string(),
                    "Config 1: HID".to_string(),
                )]),
                config_path: Some(vec!["strings".to_string(), "0x409".to_string()]),
                report_desc: None,
            },
        );

        config_map.insert(
            "config_c1".to_string(),
            GadgetConfigItem {
                order: 2,
                path: Vec::new(),
                attrs: HashMap::new(),
                config_attrs: HashMap::from([("MaxPower".to_string(), "900".to_string())]),
                config_path: Some(Vec::new()),
                report_desc: None,
            },
        );

        config_map.insert(
            "keyboard".to_string(),
            GadgetConfigItem {
                order: 1000,
                path: vec!["functions".to_string(), "hid.usb0".to_string()],
                attrs: HashMap::from([
                    ("protocol".to_string(), "1".to_string()),
                    ("subclass".to_string(), "1".to_string()),
                    ("report_length".to_string(), "8".to_string()),
                    ("no_out_endpoint".to_string(), "0".to_string()),
                ]),
                config_attrs: HashMap::new(),
                config_path: Some(vec!["hid.usb0".to_string()]),
                report_desc: Some(KEYBOARD_REPORT_DESC.to_vec()),
            },
        );

        config_map.insert(
            "absolute_mouse".to_string(),
            GadgetConfigItem {
                order: 1001,
                path: vec!["functions".to_string(), "hid.usb1".to_string()],
                attrs: HashMap::from([
                    ("protocol".to_string(), "2".to_string()),
                    ("subclass".to_string(), "0".to_string()),
                    ("report_length".to_string(), "6".to_string()),
                    ("no_out_endpoint".to_string(), "1".to_string()),
                ]),
                config_attrs: HashMap::new(),
                config_path: Some(vec!["hid.usb1".to_string()]),
                report_desc: Some(ABSOLUTE_MOUSE_REPORT_DESC.to_vec()),
            },
        );

        config_map.insert(
            "relative_mouse".to_string(),
            GadgetConfigItem {
                order: 1002,
                path: vec!["functions".to_string(), "hid.usb2".to_string()],
                attrs: HashMap::from([
                    ("protocol".to_string(), "2".to_string()),
                    ("subclass".to_string(), "1".to_string()),
                    ("report_length".to_string(), "4".to_string()),
                    ("no_out_endpoint".to_string(), "1".to_string()),
                ]),
                config_attrs: HashMap::new(),
                config_path: Some(vec!["hid.usb2".to_string()]),
                report_desc: Some(RELATIVE_MOUSE_REPORT_DESC.to_vec()),
            },
        );

        config_map.insert(
            "mass_storage_base".to_string(),
            GadgetConfigItem {
                order: 3000,
                path: vec!["functions".to_string(), "mass_storage.usb0".to_string()],
                attrs: HashMap::from([("stall".to_string(), "1".to_string())]),
                config_attrs: HashMap::new(),
                config_path: Some(vec!["mass_storage.usb0".to_string()]),
                report_desc: None,
            },
        );

        config_map.insert(
            "mass_storage_lun0".to_string(),
            GadgetConfigItem {
                order: 3001,
                path: vec![
                    "functions".to_string(),
                    "mass_storage.usb0".to_string(),
                    "lun.0".to_string(),
                ],
                attrs: HashMap::from([
                    ("cdrom".to_string(), "1".to_string()),
                    ("ro".to_string(), "1".to_string()),
                    ("removable".to_string(), "1".to_string()),
                    ("file".to_string(), "\n".to_string()),
                    ("inquiry_string".to_string(), "RustKVM  Virtual Media".to_string()),
                ]),
                config_attrs: HashMap::new(),
                config_path: None,
                report_desc: None,
            },
        );

        config_map
    }

    fn load_gadget_config(&mut self) {
        if self.custom_config.strict_mode {
            return;
        }

        if let Some(base) = self.config_map.get_mut("base") {
            base.attrs.insert("idVendor".to_string(), self.custom_config.vendor_id.clone());
            base.attrs.insert("idProduct".to_string(), self.custom_config.product_id.clone());
        }

        if let Some(base_info) = self.config_map.get_mut("base_info") {
            base_info
                .attrs
                .insert("serialnumber".to_string(), self.custom_config.serial_number.clone());
            base_info
                .attrs
                .insert("manufacturer".to_string(), self.custom_config.manufacturer.clone());
            base_info.attrs.insert("product".to_string(), self.custom_config.product.clone());
        }
    }

    fn is_gadget_config_item_enabled(&self, item_key: &str) -> bool {
        match item_key {
            "absolute_mouse" => self.enabled_devices.absolute_mouse,
            "relative_mouse" => self.enabled_devices.relative_mouse,
            "keyboard" => self.enabled_devices.keyboard,
            "mass_storage_base" => self.enabled_devices.mass_storage,
            "mass_storage_lun0" => self.enabled_devices.mass_storage,
            _ => true,
        }
    }

    fn configure_usb_gadget(&self, reset_usb: bool) -> Result<()> {
        self.mount_configfs()?;
        self.write_gadget_config()?;

        if reset_usb {
            self.rebind_usb(true)?;
        }

        Ok(())
    }

    fn mount_configfs(&self) -> Result<()> {
        let configfs_path = Path::new("/sys/kernel/config");
        if !configfs_path.exists() {
            std::fs::create_dir_all(configfs_path).with_context(|| {
                format!("failed to create configfs directory: {}", configfs_path.display())
            })?;
        }

        let mounted = std::fs::read_to_string("/proc/mounts")
            .ok()
            .map(|s| {
                s.lines().any(|line| {
                    let mut parts = line.split_whitespace();
                    let _src = parts.next();
                    let mnt = parts.next().unwrap_or("");
                    let fstype = parts.next().unwrap_or("");
                    mnt == "/sys/kernel/config" && fstype == "configfs"
                })
            })
            .unwrap_or(false);

        if !mounted {
            let status = Command::new("mount")
                .args(["-t", "configfs", "none", "/sys/kernel/config"])
                .status()
                .with_context(|| "failed to execute mount for configfs")?;
            if !status.success() {
                return Err(anyhow!("mount configfs failed with status: {}", status));
            }
            info!("configfs mounted at /sys/kernel/config");
        }

        Ok(())
    }

    fn create_config_path(&self) -> Result<()> {
        std::fs::create_dir_all(&self.config_c1_path).with_context(|| {
            format!("failed to create config path: {}", self.config_c1_path.display())
        })?;

        info!("config path created: {}", self.config_c1_path.display());
        Ok(())
    }

    fn write_gadget_config(&self) -> Result<()> {
        std::fs::create_dir_all(&self.kvm_gadget_path).with_context(|| {
            format!("failed to create gadget path: {}", self.kvm_gadget_path.display())
        })?;

        let _ = std::fs::write(self.kvm_gadget_path.join("UDC"), "");

        let mut ordered_items = Vec::with_capacity(self.config_map.len());
        ordered_items.extend(self.config_map.iter());
        ordered_items.sort_by_key(|(_, item)| item.order);

        for (_, item) in ordered_items.iter() {
            if item.order <= 1 {
                self.write_gadget_item_config(item)?;
            }
        }

        self.create_config_path()?;

        for (key, item) in ordered_items {
            if !self.is_gadget_config_item_enabled(key) {
                self.disable_gadget_item_config(item)?;
                continue;
            }

            self.write_gadget_item_config(item)?;
        }

        self.reorder_config_symlinks()?;

        self.write_udc()?;

        Ok(())
    }

    fn disable_gadget_item_config(&self, item: &GadgetConfigItem) -> Result<()> {
        if let Some(config_path) = &item.config_path {
            let full_config_path =
                self.build_path_from_components(&self.config_c1_path, config_path);
            if full_config_path.exists() {
                let meta = std::fs::metadata(&full_config_path)?;
                if meta.is_dir() {
                    std::fs::remove_dir_all(&full_config_path).with_context(|| {
                        format!("failed to remove config dir: {}", full_config_path.display())
                    })?;
                } else {
                    std::fs::remove_file(&full_config_path).with_context(|| {
                        format!("failed to remove config: {}", full_config_path.display())
                    })?;
                }
                debug!("disabled gadget config: {}", full_config_path.display());
            }
        }
        Ok(())
    }

    fn write_gadget_item_config(&self, item: &GadgetConfigItem) -> Result<()> {
        if let Some(config_path) = &item.config_path
            && item.config_attrs.is_empty()
        {
            let config_link_path =
                self.build_path_from_components(&self.config_c1_path, config_path);
            if config_link_path.exists() {
                let meta = std::fs::symlink_metadata(&config_link_path)?;
                if meta.file_type().is_symlink() || meta.is_file() {
                    std::fs::remove_file(&config_link_path)?;
                } else if meta.is_dir() {
                    std::fs::remove_dir_all(&config_link_path)?;
                }
                debug!("temporarily removed config link: {}", config_link_path.display());
            }
        }

        let gadget_item_path = self.build_path_from_components(&self.kvm_gadget_path, &item.path);
        if gadget_item_path != self.kvm_gadget_path {
            std::fs::create_dir_all(&gadget_item_path).with_context(|| {
                format!("failed to create gadget item directory: {}", gadget_item_path.display())
            })?;
        }

        let is_hid = item.path.last().map(|s| s.starts_with("hid.usb")).unwrap_or(false);
        if is_hid {
            if let Some(v) = item.attrs.get("subclass") {
                self.write_file_content(&gadget_item_path.join("subclass"), v)?;
            }
            if let Some(v) = item.attrs.get("protocol") {
                self.write_file_content(&gadget_item_path.join("protocol"), v)?;
            }
            if let Some(v) = item.attrs.get("report_length") {
                self.write_file_content(&gadget_item_path.join("report_length"), v)?;
            }
            if let Some(report_desc) = &item.report_desc {
                self.write_file_content_bytes(&gadget_item_path.join("report_desc"), report_desc)?;
            }
            for (attr_name, attr_value) in &item.attrs {
                if ["protocol", "subclass", "report_length"].contains(&attr_name.as_str()) {
                    continue;
                }
                let attr_path = gadget_item_path.join(attr_name);
                self.write_file_content(&attr_path, attr_value)?;
            }
        } else {
            for (attr_name, attr_value) in &item.attrs {
                let attr_path = gadget_item_path.join(attr_name);
                self.write_file_content(&attr_path, attr_value)?;
            }
            if let Some(report_desc) = &item.report_desc {
                self.write_file_content_bytes(&gadget_item_path.join("report_desc"), report_desc)?;
            }
        }

        if let Some(config_path) = &item.config_path
            && !item.config_attrs.is_empty()
        {
            let config_item_path =
                self.build_path_from_components(&self.config_c1_path, config_path);
            if config_item_path != self.config_c1_path {
                std::fs::create_dir_all(&config_item_path).with_context(|| {
                    format!(
                        "failed to create config item directory: {}",
                        config_item_path.display()
                    )
                })?;
            }
            for (attr_name, attr_value) in &item.config_attrs {
                let attr_path = config_item_path.join(attr_name);
                self.write_file_content(&attr_path, attr_value)?;
            }
        }

        if let Some(config_path) = &item.config_path
            && item.config_attrs.is_empty()
        {
            let config_link_path =
                self.build_path_from_components(&self.config_c1_path, config_path);
            let gadget_link_target =
                self.build_path_from_components(&self.kvm_gadget_path, &item.path);

            std::os::unix::fs::symlink(&gadget_link_target, &config_link_path).with_context(
                || {
                    format!(
                        "failed to create symlink: {} -> {}",
                        config_link_path.display(),
                        gadget_link_target.display()
                    )
                },
            )?;
        }

        Ok(())
    }

    fn reorder_config_symlinks(&self) -> Result<()> {
        let mut ordered_items = Vec::with_capacity(self.config_map.len());
        ordered_items.extend(self.config_map.iter());
        ordered_items.sort_by_key(|(_, item)| item.order);

        let mut expected: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (key, item) in ordered_items {
            if !self.is_gadget_config_item_enabled(key) {
                continue;
            }
            if let Some(cfg_path) = &item.config_path {
                if !item.config_attrs.is_empty() {
                    continue;
                }
                let link = self.build_path_from_components(&self.config_c1_path, cfg_path);
                let target = self.build_path_from_components(&self.kvm_gadget_path, &item.path);
                expected.push((link, target));
            }
        }

        if self.config_c1_path.exists() {
            for entry in std::fs::read_dir(&self.config_c1_path)
                .with_context(|| "failed to read configs/c.1 directory")?
            {
                let entry = entry?;
                let ftype = entry.file_type()?;
                if ftype.is_symlink() {
                    std::fs::remove_file(entry.path()).with_context(|| {
                        format!("failed to remove symlink: {}", entry.path().display())
                    })?;
                }
            }
        }

        for (link, target) in expected {
            std::os::unix::fs::symlink(&target, &link).with_context(|| {
                format!(
                    "failed to create symlink in order: {} -> {}",
                    link.display(),
                    target.display()
                )
            })?;
        }

        Ok(())
    }

    fn build_path_from_components(&self, base: &Path, components: &[String]) -> PathBuf {
        if components.is_empty() {
            return base.to_path_buf();
        }

        let mut path = base.to_path_buf();
        path.reserve(components.iter().map(|s| s.len()).sum::<usize>() + components.len());
        for component in components {
            path.push(component);
        }
        path
    }

    fn write_udc(&self) -> Result<()> {
        Self::force_unbind_conflicting_gadgets(&self.udc, &self.name);
        let udc_path = self.kvm_gadget_path.join("UDC");
        self.write_file_content(&udc_path, &self.udc)?;
        info!("UDC bound: {}", self.udc);
        Ok(())
    }

    fn rebind_usb(&self, ignore_unbind_error: bool) -> Result<()> {
        let unbind_path = PathBuf::from("/sys/bus/platform/drivers/dwc3/unbind");
        if let Err(e) = std::fs::write(&unbind_path, &self.udc) {
            if !ignore_unbind_error {
                return Err(e).with_context(|| "failed to unbind UDC");
            }
            warn!("failed to unbind UDC (ignored): {}", e);
        }

        let bind_path = PathBuf::from("/sys/bus/platform/drivers/dwc3/bind");
        std::fs::write(&bind_path, &self.udc).with_context(|| "failed to bind UDC")?;

        info!("USB gadget rebound successfully");
        Ok(())
    }

    fn write_file_content(&self, path: &Path, content: &str) -> Result<()> {
        std::fs::write(path, content)
            .with_context(|| format!("failed to write content to: {}", path.display()))?;

        Ok(())
    }

    fn write_file_content_bytes(&self, path: &Path, content: &[u8]) -> Result<()> {
        std::fs::write(path, content)
            .with_context(|| format!("failed to write content to: {}", path.display()))?;

        Ok(())
    }

    fn force_unbind_conflicting_gadgets(udc: &str, our_name: &str) {
        let root = Path::new("/sys/kernel/config/usb_gadget");
        if let Ok(entries) = fs::read_dir(root) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name == our_name {
                    continue;
                }
                let udc_file = e.path().join("UDC");
                if let Ok(s) = fs::read_to_string(&udc_file)
                    && s.trim() == udc
                {
                    let _ = fs::write(&udc_file, "");
                    info!("force-unbound conflicting gadget '{}' from UDC {}", name, udc);
                }
            }
        }
    }
}

impl Drop for UsbGadget {
    fn drop(&mut self) {
        if let Err(e) = self.unbind_udc() {
            warn!("failed to unbind UDC during cleanup: {}", e);
        }
    }
}
