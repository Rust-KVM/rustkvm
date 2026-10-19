use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{trace, warn};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct KeyboardState {
    pub num_lock: bool,
    pub caps_lock: bool,
    pub scroll_lock: bool,
    pub compose: bool,
    pub kana: bool,
}

pub const HID_KEY_BUFFER_SIZE: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeysDownState {
    pub modifier: u8,
    pub keys: [u8; HID_KEY_BUFFER_SIZE],
}

fn modifier_mask(key: u8) -> Option<u8> {
    if (0xE0..=0xE7).contains(&key) { Some(1u8 << (key - 0xE0)) } else { None }
}

fn apply_keypress(state: &mut KeysDownState, key: u8, press: bool) {
    if let Some(mask) = modifier_mask(key) {
        if press {
            state.modifier |= mask;
        } else {
            state.modifier &= !mask;
        }
        return;
    }
    if key == 0 {
        return;
    }

    let buf = &mut state.keys;
    let mut target: Option<usize> = None;
    for (i, slot) in buf.iter().enumerate() {
        if *slot == key || *slot == 0 {
            target = Some(i);
            break;
        }
    }

    match target {
        Some(i) if press => buf[i] = key,
        Some(i) if buf[i] != 0 => {
            for j in i..buf.len() - 1 {
                buf[j] = buf[j + 1];
            }
            buf[buf.len() - 1] = 0;
        }
        Some(_) => {}
        None => {
            if press {
                buf.fill(0x01);
            }
        }
    }
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

/// Depth of the report queue feeding the writer thread. Sized to absorb a
/// burst of pointer events while the interrupt endpoint drains (~8ms/report
/// worst case) without replaying seconds of stale input after a host stall.
const HID_WRITE_QUEUE_DEPTH: usize = 64;

/// Upper bound for a single gadget write. The f_hid function blocks writes
/// until the host polls the interrupt endpoint, which never happens while the
/// host is suspended — without this bound a single report can hang forever.
const HID_WRITE_TIMEOUT: Duration = Duration::from_millis(100);

const HID_WARN_INTERVAL: Duration = Duration::from_secs(5);

const HID_DEV_KEYBOARD: usize = 0;
const HID_DEV_ABS_MOUSE: usize = 1;
const HID_DEV_REL_MOUSE: usize = 2;

enum HidWriteCmd {
    Keyboard([u8; 8]),
    AbsMouse([u8; 6]),
    AbsWheel([u8; 2]),
    RelMouse([u8; 4]),
}

impl HidWriteCmd {
    fn device_and_payload(&self) -> (usize, &[u8]) {
        match self {
            Self::Keyboard(b) => (HID_DEV_KEYBOARD, b.as_slice()),
            Self::AbsMouse(b) => (HID_DEV_ABS_MOUSE, b.as_slice()),
            Self::AbsWheel(b) => (HID_DEV_ABS_MOUSE, b.as_slice()),
            Self::RelMouse(b) => (HID_DEV_REL_MOUSE, b.as_slice()),
        }
    }
}

struct HidWriter {
    paths: [String; 3],
    files: [Option<std::fs::File>; 3],
    last_warn: Option<Instant>,
    suppressed: u64,
}

impl HidWriter {
    fn run(mut self, rx: mpsc::Receiver<HidWriteCmd>) {
        while let Ok(cmd) = rx.recv() {
            let (dev, payload) = cmd.device_and_payload();
            if let Err(e) = self.write_report(dev, payload) {
                self.files[dev] = None;
                let mut dropped = 0usize;
                while rx.try_recv().is_ok() {
                    dropped += 1;
                }
                self.warn_rate_limited(dev, &e, dropped);
            }
        }
        trace!("hid writer thread exiting");
    }

    fn write_report(&mut self, dev: usize, payload: &[u8]) -> anyhow::Result<()> {
        let file = match &mut self.files[dev] {
            Some(file) => file,
            None => {
                let path = &self.paths[dev];
                if !Path::new(path).exists() {
                    anyhow::bail!("HID path not found: {path}");
                }
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(path)?;
                self.files[dev].insert(file)
            }
        };

        let deadline = Instant::now() + HID_WRITE_TIMEOUT;
        loop {
            match file.write(payload) {
                Ok(n) if n == payload.len() => return Ok(()),
                Ok(n) => anyhow::bail!("short HID write: {n}/{}", payload.len()),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let remain = deadline.saturating_duration_since(Instant::now());
                    if remain.is_zero() || !poll_writable(file.as_raw_fd(), remain)? {
                        anyhow::bail!("HID write timed out (host not reading reports?)");
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn warn_rate_limited(&mut self, dev: usize, err: &anyhow::Error, dropped: usize) {
        self.suppressed += 1 + dropped as u64;
        if self.last_warn.is_none_or(|t| t.elapsed() >= HID_WARN_INTERVAL) {
            warn!(
                "HID write to {} failed ({err}); {} report(s) dropped since last warning",
                self.paths[dev], self.suppressed
            );
            self.last_warn = Some(Instant::now());
            self.suppressed = 0;
        }
    }
}

fn poll_writable(fd: std::os::fd::RawFd, timeout: Duration) -> std::io::Result<bool> {
    let mut pfd = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
    // SAFETY: `pfd` is a valid pollfd for the writer's open gadget fd and we
    // pass nfds=1 matching the single-element array.
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if rc < 0 {
        let e = std::io::Error::last_os_error();
        if e.kind() == std::io::ErrorKind::Interrupted {
            // Treat EINTR as "retry": the write loop re-checks the deadline.
            return Ok(true);
        }
        return Err(e);
    }
    Ok(rc > 0)
}

pub struct Hid {
    keyboard_path: String,

    write_tx: mpsc::SyncSender<HidWriteCmd>,
    last_full_warn: Mutex<Option<Instant>>,

    keyboard_reader_cancel: Mutex<CancellationToken>,
    keyboard_reader_handle: Mutex<Option<JoinHandle<()>>>,

    keyboard_state: Arc<RwLock<KeyboardState>>,
    on_keyboard_state_change: RwLock<Option<Arc<KeyboardStateCallback>>>,

    keys_down: Mutex<KeysDownState>,

    last_user_input: Mutex<Instant>,
}

impl Default for Hid {
    fn default() -> Self {
        Self::new("/dev/hidg0".to_string(), "/dev/hidg1".to_string(), "/dev/hidg2".to_string())
    }
}

impl Hid {
    pub fn new(keyboard_path: String, abs_mouse_path: String, rel_mouse_path: String) -> Self {
        let (write_tx, write_rx) = mpsc::sync_channel(HID_WRITE_QUEUE_DEPTH);
        let writer = HidWriter {
            paths: [keyboard_path.clone(), abs_mouse_path, rel_mouse_path],
            files: [None, None, None],
            last_warn: None,
            suppressed: 0,
        };
        std::thread::Builder::new()
            .name("hid-writer".to_string())
            .spawn(move || writer.run(write_rx))
            .expect("failed to spawn hid-writer thread");

        Self {
            keyboard_path,
            write_tx,
            last_full_warn: Mutex::new(None),
            keyboard_reader_cancel: Mutex::new(CancellationToken::new()),
            keyboard_reader_handle: Mutex::new(None),
            keyboard_state: Arc::new(RwLock::new(KeyboardState::default())),
            on_keyboard_state_change: RwLock::new(None),
            keys_down: Mutex::new(KeysDownState::default()),
            last_user_input: Mutex::new(Instant::now()),
        }
    }

    fn enqueue(&self, cmd: HidWriteCmd) -> anyhow::Result<()> {
        self.reset_user_input_time();
        match self.write_tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                let mut last = self.last_full_warn.lock();
                if last.is_none_or(|t| t.elapsed() >= HID_WARN_INTERVAL) {
                    *last = Some(Instant::now());
                    warn!("HID write queue full; dropping input report (host stalled?)");
                }
                Ok(())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("HID writer thread terminated")
            }
        }
    }

    pub fn set_on_keyboard_state_change<F>(&self, f: F)
    where
        F: Fn(KeyboardState) + Send + Sync + 'static,
    {
        *self.on_keyboard_state_change.write() = Some(Arc::new(f));
    }

    pub fn open_keyboard_hid_file(&self) -> anyhow::Result<()> {
        if self.keyboard_reader_handle.lock().is_some() {
            return Ok(());
        }

        if !Path::new(&self.keyboard_path).exists() {
            anyhow::bail!("keyboard HID path not found: {}", self.keyboard_path);
        }

        let mut cancel_guard = self.keyboard_reader_cancel.lock();
        cancel_guard.cancel();
        *cancel_guard = CancellationToken::new();
        let reader_cancel = cancel_guard.clone();

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

    pub fn keyboard_report(&self, modifier: u8, keys: &[u8]) -> anyhow::Result<()> {
        {
            let mut state = self.keys_down.lock();
            state.modifier = modifier;
            state.keys.fill(0);
            for (slot, k) in state.keys.iter_mut().zip(keys.iter().take(HID_KEY_BUFFER_SIZE)) {
                *slot = *k;
            }
        }
        self.write_keyboard_report(modifier, keys)
    }

    pub fn keypress_report(&self, key: u8, press: bool) -> anyhow::Result<()> {
        let (modifier, keys) = {
            let mut state = self.keys_down.lock();
            apply_keypress(&mut state, key, press);
            (state.modifier, state.keys)
        };
        self.write_keyboard_report(modifier, &keys)
    }

    pub fn get_keys_down_state(&self) -> KeysDownState {
        *self.keys_down.lock()
    }

    fn write_keyboard_report(&self, modifier: u8, keys: &[u8]) -> anyhow::Result<()> {
        let mut report = [0u8; 8];
        report[0] = modifier;
        report[1] = 0;
        for (i, k) in keys.iter().take(HID_KEY_BUFFER_SIZE).enumerate() {
            report[2 + i] = *k;
        }
        self.enqueue(HidWriteCmd::Keyboard(report))
    }

    pub fn abs_mouse_report(&self, x: i32, y: i32, buttons: u8) -> anyhow::Result<()> {
        let mut data = [0u8; 6];
        data[0] = 1;
        data[1] = buttons;
        data[2] = (x as u16 & 0x00FF) as u8;
        data[3] = ((x as u16 >> 8) & 0x00FF) as u8;
        data[4] = (y as u16 & 0x00FF) as u8;
        data[5] = ((y as u16 >> 8) & 0x00FF) as u8;
        self.enqueue(HidWriteCmd::AbsMouse(data))
    }

    pub fn abs_mouse_wheel_report(&self, wheel_y: i8) -> anyhow::Result<()> {
        if wheel_y == 0 {
            return Ok(());
        }
        self.enqueue(HidWriteCmd::AbsWheel([2u8, wheel_y as u8]))
    }

    pub fn rel_mouse_report(&self, dx: i8, dy: i8, buttons: u8) -> anyhow::Result<()> {
        self.enqueue(HidWriteCmd::RelMouse([buttons, dx as u8, dy as u8, 0u8]))
    }

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
