//! USB gadget subsystem facade.
//!
//! - Periodically poll USB state from kernel `udc` state file
//! - Expose HID operations (keyboard/mouse)
//! - Provide keyboard LED state callback registration
//! - Upper-layer hooks for event broadcast
//! - USB gadget configuration and management

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

/// Simple USB state reader for UDC
#[derive(Debug, Clone)]
pub struct UsbState {
    pub state: String,
}

impl Default for UsbState {
    fn default() -> Self {
        Self { state: "unknown".to_string() }
    }
}

/// USB manager combining UDC state polling and HID access
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
    /// Create with UDC name and default HID device paths
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

    /// Initialize USB gadget with specified configuration
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

    /// Get USB gadget if initialized
    pub fn get_gadget(&self) -> Option<&Arc<UsbGadget>> {
        self.gadget.as_ref()
    }

    /// Start background UDC state polling loop
    pub fn start_polling(&self) {
        if self.poll_handle.read().is_some() {
            return;
        }

        let state_path = self.udc_state_path.clone();
        let cancel = self.poll_cancel.clone();
        let state_ref = Arc::clone(&self.state);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            let mut last_state = String::new();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        trace!("usb state poll cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        // Optimize: only read file if path exists
                        if !Path::new(&state_path).exists() {
                            continue;
                        }

                        let new_state = match read_trimmed(&state_path) {
                            Ok(state) => state,
                            Err(_) => "unknown".to_string(),
                        };

                        if new_state != last_state {
                            let prev_state = std::mem::replace(&mut last_state, new_state.clone());
                            *state_ref.write() = UsbState { state: new_state.clone() };

                            info!(from = %prev_state, to = %new_state, "USB state changed");

                            // Broadcast via RPC and request display update
                            crate::jsonrpc::broadcast_usb_state(new_state).await;
                            let _ = crate::hardware::display::request_display_update(true).await;
                        }
                    }
                }
            }
        });
        *self.poll_handle.write() = Some(handle);
    }

    /// Stop polling loop
    pub async fn stop_polling(&self) {
        self.poll_cancel.cancel();
        let handle_opt = { self.poll_handle.write().take() };
        if let Some(h) = handle_opt {
            let _ = h.await;
        }
    }

    /// Get current USB state string
    pub fn get_usb_state(&self) -> String {
        self.state.read().state.clone()
    }

    /// Access HID
    pub fn hid(&self) -> Arc<Hid> {
        self.hid.clone()
    }

    /// Get the UDC name associated with this manager
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

fn read_trimmed(path: &str) -> anyhow::Result<String> {
    if !Path::new(path).exists() {
        anyhow::bail!("path not found: {}", path);
    }
    let content = std::fs::read_to_string(path)?;
    Ok(content.trim().to_string())
}

static USB_MANAGER: OnceCell<Arc<RwLock<UsbManager>>> = OnceCell::new();

/// Initialize global USB manager, start polling, and wire keyboard LED to RPC
pub fn init_usb() -> Option<&'static Arc<RwLock<UsbManager>>> {
    USB_MANAGER
        .get_or_try_init(|| -> anyhow::Result<Arc<RwLock<UsbManager>>> {
            // Optimize: use more efficient UDC detection, but tolerate absence
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

            // Initialize USB gadget with default configuration. Do not fail overall if this errors.
            let devices = DeviceConfig::default();
            let config = GadgetConfig::default();
            if let Err(err) = mgr.init_gadget("rustkvm".to_string(), devices, config) {
                tracing::warn!("failed to initialize USB gadget: {}", err);
            }

            let mgr = Arc::new(RwLock::new(mgr));

            // Optimize: reduce lock contention by getting HID reference once
            let hid = mgr.read().hid();

            // Set keyboard LED state change callback
            hid.set_on_keyboard_state_change(|state| {
                tokio::spawn(async move {
                    crate::jsonrpc::broadcast_keyboard_led_state(state).await;
                });
            });

            // Open keyboard HID file with better error handling
            if let Err(e) = hid.open_keyboard_hid_file() {
                tracing::warn!("failed to open keyboard HID file: {}", e);
                // Continue initialization even if HID file fails
            }

            // Start polling
            mgr.read().start_polling();
            Ok(mgr)
        })
        .ok()
}

/// Get global USB manager if initialized
pub fn get_usb_manager() -> Option<&'static Arc<RwLock<UsbManager>>> {
    USB_MANAGER.get()
}

/// Initialize USB gadget with custom configuration
pub fn init_usb_gadget(
    name: String,
    devices: DeviceConfig,
    config: GadgetConfig,
) -> anyhow::Result<()> {
    if let Some(manager) = get_usb_manager() {
        let mut mgr = manager.write();
        mgr.init_gadget(name, devices, config)?;
        Ok(())
    } else {
        Err(anyhow::anyhow!("USB manager not initialized"))
    }
}
