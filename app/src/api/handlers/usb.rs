use anyhow::{Result, anyhow};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use crate::hardware::usb as usb_mod;

static USB_STATE: once_cell::sync::OnceCell<RwLock<String>> = once_cell::sync::OnceCell::new();

pub fn get_usb_state() -> Result<String> {
    let cell = USB_STATE.get_or_init(|| parking_lot::RwLock::new("unknown".to_string()));
    Ok(cell.read().clone())
}

pub fn set_usb_state(state: String) -> Result<Value> {
    let cell = USB_STATE.get_or_init(|| parking_lot::RwLock::new("unknown".to_string()));
    *cell.write() = state;
    Ok(Value::Null)
}

#[derive(Serialize, Deserialize)]
pub struct UsbDevicesResponse {
    pub absolute_mouse: bool,
    pub relative_mouse: bool,
    pub keyboard: bool,
    pub mass_storage: bool,
}

pub fn get_usb_devices() -> Result<UsbDevicesResponse> {
    Ok(UsbDevicesResponse {
        absolute_mouse: true,
        relative_mouse: false,
        keyboard: true,
        mass_storage: true,
    })
}

#[derive(Deserialize)]
pub struct SetUsbDevicesParams {
    pub devices: UsbDevicesResponse,
}

pub fn set_usb_devices(_params: SetUsbDevicesParams) -> Result<Value> {
    info!("USB devices config updated");
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct UsbDeviceStateParams {
    pub device: String,
    pub enabled: bool,
}

pub fn set_usb_device_state(params: UsbDeviceStateParams) -> Result<Value> {
    match params.device.as_str() {
        "absoluteMouse" | "relativeMouse" | "keyboard" | "massStorage" => {
            info!("USB device {} state set to: {}", params.device, params.enabled);
            Ok(Value::Null)
        }
        _ => Err(anyhow!("Invalid USB device: {}", params.device)),
    }
}

pub fn get_usb_emulation_state() -> Result<bool> {
    if let Some(mgr) = usb_mod::get_usb_manager() {
        let mgr_guard = mgr.read();
        let udc = mgr_guard.get_udc_name();
        let path = format!("/sys/bus/platform/drivers/dwc3/{}", udc);
        return Ok(std::path::Path::new(&path).exists());
    }
    Ok(false)
}

#[derive(Deserialize)]
pub struct UsbEmulationParams {
    pub enabled: bool,
}

pub async fn set_usb_emulation_state(params: UsbEmulationParams) -> Result<Value> {
    let udc = {
        let mgr =
            usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
        let mgr_guard = mgr.read();
        mgr_guard.get_udc_name().to_string()
    };
    let path = if params.enabled {
        "/sys/bus/platform/drivers/dwc3/bind"
    } else {
        "/sys/bus/platform/drivers/dwc3/unbind"
    };
    tokio::fs::write(path, &udc).await.map_err(|e| {
        if params.enabled {
            anyhow!("error binding UDC: {}", e)
        } else {
            anyhow!("error unbinding UDC: {}", e)
        }
    })?;
    info!("USB emulation state set to: {}", params.enabled);
    Ok(Value::Null)
}

#[derive(Serialize, Deserialize)]
pub struct UsbConfigResponse {
    pub absolute_mouse: bool,
    pub relative_mouse: bool,
    pub keyboard: bool,
    pub mass_storage: bool,
    pub serial_console: bool,
}

pub fn get_usb_config() -> Result<UsbConfigResponse> {
    Ok(UsbConfigResponse {
        absolute_mouse: true,
        relative_mouse: true,
        keyboard: true,
        mass_storage: true,
        serial_console: false,
    })
}

#[derive(Deserialize)]
pub struct SetUsbConfigParams {
    #[serde(rename = "usbConfig")]
    pub usb_config: UsbConfigResponse,
}

pub fn set_usb_config(_params: SetUsbConfigParams) -> Result<Value> {
    info!("USB config updated");
    Ok(Value::Null)
}
