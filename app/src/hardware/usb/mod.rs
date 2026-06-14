use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace};

pub mod descriptors;
pub mod gadget;
pub mod hid;
pub use gadget::{DeviceConfig, GadgetConfig, UsbGadget};
pub use hid::{Hid, KeyboardState};
pub mod storage;

#[derive(Debug, Clone)]
pub struct UsbState {
    pub state: String,
}

impl Default for UsbState {
    fn default() -> Self {
        Self { state: "unknown".to_string() }
    }
}

pub struct UsbManager {
    udc_name: String,
    udc_state_path: String,
    state: Arc<RwLock<UsbState>>,
    poll_cancel: CancellationToken,
    poll_handle: RwLock<Option<JoinHandle<()>>>,
    hid: Arc<Hid>,
    gadget: Option<Arc<UsbGadget>>,
}

impl UsbManager {
    pub fn new(udc_name: String) -> Self {
        let udc_state_path = format!("/sys/class/udc/{}/state", udc_name);
        Self {
            udc_name,
            udc_state_path,
            state: Arc::new(RwLock::new(UsbState::default())),
            poll_cancel: CancellationToken::new(),
            poll_handle: RwLock::new(None),
            hid: Arc::new(Hid::default()),
            gadget: None,
        }
    }

    pub fn init_gadget(
        &mut self,
        name: String,
        devices: DeviceConfig,
        config: GadgetConfig,
    ) -> anyhow::Result<()> {
        info!("initializing USB gadget: {}", name);

        let gadget = UsbGadget::new(name, devices, config)?;
        gadget.init()?;

        self.gadget = Some(Arc::new(gadget));
        Ok(())
    }

    pub fn start_polling(&self) {
        if self.poll_handle.read().is_some() {
            return;
        }

        let state_path = self.udc_state_path.clone();
        let cancel = self.poll_cancel.clone();
        let state_ref = Arc::clone(&self.state);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last_state = String::new();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        trace!("usb state poll cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        if !Path::new(&state_path).exists() {
                            continue;
                        }

                        let new_state = match read_trimmed(&state_path).await {
                            Ok(state) => state,
                            Err(_) => "unknown".to_string(),
                        };

                        if new_state != last_state {
                            let prev_state = std::mem::replace(&mut last_state, new_state.clone());
                            *state_ref.write() = UsbState { state: new_state.clone() };

                            info!(from = %prev_state, to = %new_state, "USB state changed");

                            crate::api::broadcast_usb_state(new_state).await;
                            let _ = crate::hardware::display::request_display_update(true).await;
                        }
                    }
                }
            }
        });
        *self.poll_handle.write() = Some(handle);
    }

    pub fn get_usb_state(&self) -> String {
        self.state.read().state.clone()
    }

    pub fn hid(&self) -> Arc<Hid> {
        self.hid.clone()
    }

    pub fn get_udc_name(&self) -> &str {
        &self.udc_name
    }
}

pub fn get_current_usb_state() -> String {
    if let Some(mgr) = USB_MANAGER.get() {
        return mgr.read().get_usb_state();
    }
    "unknown".to_string()
}

async fn read_trimmed(path: &str) -> anyhow::Result<String> {
    if !Path::new(path).exists() {
        anyhow::bail!("path not found: {}", path);
    }
    let content = tokio::fs::read_to_string(path).await.map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(content.trim().to_string())
}

fn usb_serial() -> String {
    let mut hex = String::with_capacity(16);

    if let Ok(s) = crate::hardware::hw::extract_serial_number() {
        for &byte in s.as_bytes() {
            if byte.is_ascii_hexdigit() {
                hex.push(byte.to_ascii_uppercase() as char);
                if hex.len() == 16 {
                    break;
                }
            }
        }
    }

    if hex.is_empty() {
        let mut buf = uuid::Uuid::encode_buffer();
        let u = uuid::Uuid::new_v4().as_simple().encode_upper(&mut buf);
        hex.push_str(&u[..u.len().min(16)]);
    }

    let mut out = String::with_capacity(8 + hex.len());
    out.push_str("RUSTKVM-");
    out.push_str(&hex);
    out
}

static USB_MANAGER: OnceCell<Arc<RwLock<UsbManager>>> = OnceCell::new();

pub fn init_usb() -> Option<&'static Arc<RwLock<UsbManager>>> {
    USB_MANAGER
        .get_or_try_init(|| -> anyhow::Result<Arc<RwLock<UsbManager>>> {
            let udc_name = std::fs::read_dir("/sys/class/udc")
                .ok()
                .and_then(|it| it.flatten().next())
                .and_then(|e| e.file_name().into_string().ok())
                .unwrap_or_else(|| {
                    tracing::warn!(
                        "no UDC found; USB emulation will be disabled until UDC appears"
                    );
                    "unknown".to_string()
                });

            let mut mgr = UsbManager::new(udc_name);

            let devices = DeviceConfig::default();
            let config = GadgetConfig { serial_number: usb_serial(), ..Default::default() };
            if let Err(err) = mgr.init_gadget("rustkvm".to_string(), devices, config) {
                tracing::warn!("failed to initialize USB gadget: {}", err);
            }

            let mgr = Arc::new(RwLock::new(mgr));

            let hid = mgr.read().hid();

            hid.set_on_keyboard_state_change(|state| {
                tokio::spawn(async move {
                    crate::api::broadcast_keyboard_led_state(state).await;
                });
            });

            if let Err(e) = hid.open_keyboard_hid_file() {
                tracing::warn!("failed to open keyboard HID file: {}", e);
            }

            mgr.read().start_polling();
            Ok(mgr)
        })
        .ok()
}

pub fn get_usb_manager() -> Option<&'static Arc<RwLock<UsbManager>>> {
    USB_MANAGER.get()
}
