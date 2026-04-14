//! USB HID keyboard/mouse control.
//!
//! - Keyboard output report writer and LED state reader (`/dev/hidg0`)
//! - Absolute mouse (`/dev/hidg1`) and relative mouse (`/dev/hidg2`)
//! - Keyboard LED state notifications via callback
//!
//! Safety:
//! - All file IO is best-effort and error-propagating
//! - Background readers are cancellable to avoid thread leaks

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{trace, warn};

/// Keyboard LED bit masks (USB HID spec)
pub const KEYBOARD_LED_MASK_NUM_LOCK: u8 = 1 << 0;
pub const KEYBOARD_LED_MASK_CAPS_LOCK: u8 = 1 << 1;
pub const KEYBOARD_LED_MASK_SCROLL_LOCK: u8 = 1 << 2;
pub const KEYBOARD_LED_MASK_COMPOSE: u8 = 1 << 3;
pub const KEYBOARD_LED_MASK_KANA: u8 = 1 << 4;
pub const KEYBOARD_LED_VALID_MASKS: u8 = KEYBOARD_LED_MASK_NUM_LOCK
    | KEYBOARD_LED_MASK_CAPS_LOCK
    | KEYBOARD_LED_MASK_SCROLL_LOCK
    | KEYBOARD_LED_MASK_COMPOSE
    | KEYBOARD_LED_MASK_KANA;

/// Keyboard LED state
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct KeyboardState {
    pub num_lock: bool,
    pub caps_lock: bool,
    pub scroll_lock: bool,
    pub compose: bool,
    pub kana: bool,
}

fn parse_keyboard_state(byte: u8) -> KeyboardState {
    KeyboardState {
        num_lock: byte & KEYBOARD_LED_MASK_NUM_LOCK != 0,
        caps_lock: byte & KEYBOARD_LED_MASK_CAPS_LOCK != 0,
        scroll_lock: byte & KEYBOARD_LED_MASK_SCROLL_LOCK != 0,
        compose: byte & KEYBOARD_LED_MASK_COMPOSE != 0,
        kana: byte & KEYBOARD_LED_MASK_KANA != 0,
    }
}

type KeyboardStateCallback = dyn Fn(KeyboardState) + Send + Sync + 'static;

/// HID device manager
///
/// Files:
/// - Keyboard: `/dev/hidg0`
/// - Absolute mouse: `/dev/hidg1`
/// - Relative mouse: `/dev/hidg2`
pub struct Hid {
    keyboard_path: String,
    abs_mouse_path: String,
    rel_mouse_path: String,

    keyboard_file: Mutex<Option<std::fs::File>>,  // RDWR
    abs_mouse_file: Mutex<Option<std::fs::File>>, // RDWR
    rel_mouse_file: Mutex<Option<std::fs::File>>, // RDWR

    keyboard_reader_cancel: Mutex<CancellationToken>,
    keyboard_reader_handle: Mutex<Option<JoinHandle<()>>>,

    keyboard_state: Arc<RwLock<KeyboardState>>,
    on_keyboard_state_change: RwLock<Option<Arc<KeyboardStateCallback>>>,

    last_user_input: Mutex<Instant>,
}

impl Default for Hid {
    fn default() -> Self {
        Self::new("/dev/hidg0".to_string(), "/dev/hidg1".to_string(), "/dev/hidg2".to_string())
    }
}

impl Hid {
    /// Create a new HID manager with explicit device paths
    pub fn new(keyboard_path: String, abs_mouse_path: String, rel_mouse_path: String) -> Self {
        Self {
            keyboard_path,
            abs_mouse_path,
            rel_mouse_path,
            keyboard_file: Mutex::new(None),
            abs_mouse_file: Mutex::new(None),
            rel_mouse_file: Mutex::new(None),
            keyboard_reader_cancel: Mutex::new(CancellationToken::new()),
            keyboard_reader_handle: Mutex::new(None),
            keyboard_state: Arc::new(RwLock::new(KeyboardState::default())),
            on_keyboard_state_change: RwLock::new(None),
            last_user_input: Mutex::new(Instant::now()),
        }
    }

    /// Set callback for keyboard LED state changes
    pub fn set_on_keyboard_state_change<F>(&self, f: F)
    where
        F: Fn(KeyboardState) + Send + Sync + 'static,
    {
        *self.on_keyboard_state_change.write() = Some(Arc::new(f));
    }

    /// Get last known keyboard LED state
    pub fn get_keyboard_state(&self) -> KeyboardState {
        *self.keyboard_state.read()
    }

    /// Open `/dev/hidg0` and start background reader for LED output reports
    pub fn open_keyboard_hid_file(&self) -> anyhow::Result<()> {
        if self.keyboard_file.lock().is_some() {
            return Ok(());
        }

        if !Path::new(&self.keyboard_path).exists() {
            anyhow::bail!("keyboard HID path not found: {}", self.keyboard_path);
        }

        let file = OpenOptions::new().read(true).write(true).open(&self.keyboard_path)?;
        *self.keyboard_file.lock() = Some(file);

        // Cancel previous reader if any and replace token
        let mut cancel_guard = self.keyboard_reader_cancel.lock();
        cancel_guard.cancel();
        *cancel_guard = CancellationToken::new();
        let reader_cancel = cancel_guard.clone();

        // Clear previous handle if present
        let _ = self.keyboard_reader_handle.lock().take();

        let mut file_for_read = OpenOptions::new().read(true).open(&self.keyboard_path)?;
        let on_change = self.on_keyboard_state_change.read().clone();
        let state_ref = Arc::clone(&self.keyboard_state);

        let handle = tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 8];
            loop {
                if reader_cancel.is_cancelled() {
                    trace!("keyboard reader cancelled");
                    break;
                }
                match read_exact_at_least_one(&mut file_for_read, &mut buf) {
                    Ok(n) => {
                        trace!("read from keyboard hid: n={}", n);
                        if n != 1 {
                            continue;
                        }
                        let b = buf[0];
                        if b & !KEYBOARD_LED_VALID_MASKS != 0 {
                            trace!("ignored invalid LED bits: {:02x}", b);
                            continue;
                        }
                        let new_state = parse_keyboard_state(b);
                        let mut changed = false;
                        {
                            let mut guard = state_ref.write();
                            if *guard != new_state {
                                *guard = new_state;
                                changed = true;
                            }
                        }
                        if changed && let Some(cb) = &on_change {
                            cb(new_state);
                        }
                    }
                    Err(e) => {
                        warn!("keyboard reader error: {}", e);
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    }
                }
            }
        });

        *self.keyboard_reader_handle.lock() = Some(handle);
        Ok(())
    }

    /// Send keyboard report: modifier + up to 6 keys (padded with zeros)
    pub fn keyboard_report(&self, modifier: u8, keys: &[u8]) -> anyhow::Result<()> {
        // Ensure file is opened without holding the lock during open to avoid self-deadlock
        {
            let guard = self.keyboard_file.lock();
            if guard.is_none() {
                drop(guard);
                self.open_keyboard_hid_file()?;
            }
        }
        let mut guard = self.keyboard_file.lock();
        let file = guard.as_mut().expect("keyboard file after open");

        let mut report = [0u8; 8];
        report[0] = modifier;
        report[1] = 0; // reserved
        for (i, k) in keys.iter().take(6).enumerate() {
            report[2 + i] = *k;
        }
        file.write_all(&report)?;
        self.reset_user_input_time();
        Ok(())
    }

    /// Absolute mouse report for `/dev/hidg1` (Report ID 1 and 2 for wheel)
    pub fn abs_mouse_report(&self, x: i32, y: i32, buttons: u8) -> anyhow::Result<()> {
        // Open lazily
        {
            let mut guard = self.abs_mouse_file.lock();
            if guard.is_none() {
                let file = OpenOptions::new().read(true).write(true).open(&self.abs_mouse_path)?;
                *guard = Some(file);
            }
            let file = guard.as_mut().expect("abs mouse file opened");

            let mut data = [0u8; 6];
            data[0] = 1; // Report ID 1
            data[1] = buttons;
            data[2] = (x as u16 & 0x00FF) as u8;
            data[3] = ((x as u16 >> 8) & 0x00FF) as u8;
            data[4] = (y as u16 & 0x00FF) as u8;
            data[5] = ((y as u16 >> 8) & 0x00FF) as u8;
            file.write_all(&data)?;
        }
        self.reset_user_input_time();
        Ok(())
    }

    /// Absolute mouse wheel report
    pub fn abs_mouse_wheel_report(&self, wheel_y: i8) -> anyhow::Result<()> {
        if wheel_y == 0 {
            return Ok(());
        }
        {
            let mut guard = self.abs_mouse_file.lock();
            if guard.is_none() {
                let file = OpenOptions::new().read(true).write(true).open(&self.abs_mouse_path)?;
                *guard = Some(file);
            }
            let file = guard.as_mut().expect("abs mouse file opened");
            let data = [2u8, wheel_y as u8];
            file.write_all(&data)?;
        }
        self.reset_user_input_time();
        Ok(())
    }

    /// Relative mouse (`/dev/hidg2`) report
    pub fn rel_mouse_report(&self, dx: i8, dy: i8, buttons: u8) -> anyhow::Result<()> {
        {
            let mut guard = self.rel_mouse_file.lock();
            if guard.is_none() {
                let file = OpenOptions::new().read(true).write(true).open(&self.rel_mouse_path)?;
                *guard = Some(file);
            }
            let file = guard.as_mut().expect("rel mouse file opened");
            let data = [buttons, dx as u8, dy as u8, 0u8];
            file.write_all(&data)?;
        }
        self.reset_user_input_time();
        Ok(())
    }

    /// Last user input timestamp for display wake heuristics
    pub fn get_last_user_input_time(&self) -> Instant {
        *self.last_user_input.lock()
    }

    fn reset_user_input_time(&self) {
        *self.last_user_input.lock() = Instant::now();
    }
}

fn read_exact_at_least_one(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut tmp = [0u8; 8];
    match file.read(&mut tmp) {
        Ok(0) => Ok(0),
        Ok(n) => {
            buf[..n].copy_from_slice(&tmp[..n]);
            Ok(n)
        }
        Err(e) => Err(e),
    }
}
