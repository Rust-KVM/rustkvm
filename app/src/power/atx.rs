use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use once_cell::sync::OnceCell;
use rustix::fs::{Mode, OFlags, open};
use rustix::io::{read, write};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

pub const ATX_POWER_PIN: u32 = 124;

pub const ATX_RESET_PIN: u32 = 125;

pub const ATX_PWR_LED_PIN: u32 = 126;

pub const ATX_HDD_LED_PIN: u32 = 127;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtxState {
    pub power: bool,
    #[serde(rename = "hdd")]
    pub hdd: bool,
}

pub const ATX_EXTENSION_ID: &str = "atx-power";

static ATX_STATE: OnceCell<tokio::sync::RwLock<AtxState>> = OnceCell::new();

pub static LED_PWR_STATE: AtomicBool = AtomicBool::new(false);
pub static LED_HDD_STATE: AtomicBool = AtomicBool::new(false);

static ATX_MOUNTED: AtomicBool = AtomicBool::new(false);

fn gpio_path(pin: u32, file: &str) -> String {
    format!("/sys/class/gpio/gpio{}/{}", pin, file)
}

fn sysfs_write(path: &str, value: &str) -> Result<()> {
    let fd = open(path, OFlags::WRONLY, Mode::empty())
        .with_context(|| format!("Failed to open {path} for writing"))?;
    write(&fd, value.as_bytes()).with_context(|| format!("Failed to write '{value}' to {path}"))?;
    debug!(target: "atx_power", %path, %value, "sysfs write");
    Ok(())
}

fn sysfs_read_bool(path: &str) -> Result<bool> {
    let fd = open(path, OFlags::RDONLY, Mode::empty())
        .with_context(|| format!("Failed to open {path} for reading"))?;
    let mut buf = [0u8; 2];
    let n = read(&fd, &mut buf).with_context(|| format!("Failed to read {path}"))?;
    let s = std::str::from_utf8(&buf[..n])
        .with_context(|| format!("Non-UTF-8 content in {path}"))?
        .trim();
    match s {
        "1" => Ok(true),
        "0" => Ok(false),
        other => bail!("Unexpected GPIO value '{other}' in {path}"),
    }
}

fn gpio_export(pin: u32) -> Result<()> {
    sysfs_write("/sys/class/gpio/export", &pin.to_string())
}

fn gpio_unexport(pin: u32) {
    if let Err(e) = sysfs_write("/sys/class/gpio/unexport", &pin.to_string()) {
        warn!(target: "atx_power", pin, "Failed to unexport GPIO: {e}");
    }
}

fn gpio_set_direction(pin: u32, direction: &str) -> Result<()> {
    sysfs_write(&gpio_path(pin, "direction"), direction)
}

fn gpio_write(pin: u32, high: bool) -> Result<()> {
    sysfs_write(&gpio_path(pin, "value"), if high { "1" } else { "0" })
}

fn gpio_read(pin: u32) -> Result<bool> {
    sysfs_read_bool(&gpio_path(pin, "value"))
}

fn init_output_pin(pin: u32, name: &str) -> Result<()> {
    info!(target: "atx_power", pin, name, "Initialising output GPIO");
    gpio_export(pin).with_context(|| format!("Failed to export GPIO {pin} ({name})"))?;
    thread::sleep(Duration::from_millis(50));

    gpio_set_direction(pin, "out")
        .with_context(|| format!("Failed to set direction for GPIO {pin} ({name})"))?;
    gpio_write(pin, false)
        .with_context(|| format!("Failed to initialise GPIO {pin} ({name}) low"))?;
    Ok(())
}

fn init_input_pin(pin: u32, name: &str) -> Result<()> {
    info!(target: "atx_power", pin, name, "Initialising input GPIO");
    gpio_export(pin).with_context(|| format!("Failed to export GPIO {pin} ({name})"))?;
    thread::sleep(Duration::from_millis(50));

    gpio_set_direction(pin, "in")
        .with_context(|| format!("Failed to set direction for GPIO {pin} ({name})"))?;
    Ok(())
}

fn deinit_pin(pin: u32, name: &str) {
    debug!(target: "atx_power", pin, name, "De-initialising GPIO");
    gpio_unexport(pin);
}

pub fn mount_atx_control() -> Result<()> {
    if ATX_MOUNTED.swap(true, Ordering::SeqCst) {
        debug!(target: "atx_power", "ATX control already mounted, skipping");
        return Ok(());
    }

    info!(target: "atx_power", "Mounting ATX power control");

    init_output_pin(ATX_POWER_PIN, "power_button")?;
    init_output_pin(ATX_RESET_PIN, "reset_button")?;
    init_input_pin(ATX_PWR_LED_PIN, "power_led")?;
    init_input_pin(ATX_HDD_LED_PIN, "hdd_led")?;

    if let Ok(state) = read_hw_state() {
        LED_PWR_STATE.store(state.power, Ordering::SeqCst);
        LED_HDD_STATE.store(state.hdd, Ordering::SeqCst);
    }

    let _ =
        ATX_STATE.get_or_init(|| tokio::sync::RwLock::new(AtxState { power: false, hdd: false }));

    info!(target: "atx_power", "ATX power control mounted");
    Ok(())
}

pub fn unmount_atx_control() -> Result<()> {
    if !ATX_MOUNTED.swap(false, Ordering::SeqCst) {
        debug!(target: "atx_power", "ATX control not mounted, skipping unmount");
        return Ok(());
    }

    info!(target: "atx_power", "Unmounting ATX power control");

    let _ = gpio_write(ATX_POWER_PIN, false);
    let _ = gpio_write(ATX_RESET_PIN, false);

    deinit_pin(ATX_POWER_PIN, "power_button");
    deinit_pin(ATX_RESET_PIN, "reset_button");
    deinit_pin(ATX_PWR_LED_PIN, "power_led");
    deinit_pin(ATX_HDD_LED_PIN, "hdd_led");

    info!(target: "atx_power", "ATX power control unmounted");
    Ok(())
}

fn read_hw_state() -> Result<AtxState> {
    let pwr_raw = gpio_read(ATX_PWR_LED_PIN).context("Failed to read power LED GPIO")?;
    let hdd_raw = gpio_read(ATX_HDD_LED_PIN).context("Failed to read HDD LED GPIO")?;

    Ok(AtxState { power: !pwr_raw, hdd: !hdd_raw })
}

fn pulse_output_pin(pin: u32, name: &str, duration_ms: u64) -> Result<()> {
    debug!(target: "atx_power", pin, name, duration_ms, "Pulsing GPIO");
    gpio_write(pin, true).with_context(|| format!("Failed to set {name} GPIO {pin} HIGH"))?;
    thread::sleep(Duration::from_millis(duration_ms));
    gpio_write(pin, false).with_context(|| format!("Failed to set {name} GPIO {pin} LOW"))?;
    debug!(target: "atx_power", pin, name, "Pulse complete");
    Ok(())
}

pub fn press_atx_power_button(duration_ms: u64) -> Result<()> {
    info!(target: "atx_power", duration_ms, "Pressing ATX power button");
    pulse_output_pin(ATX_POWER_PIN, "power_button", duration_ms)
}

pub fn press_atx_reset_button(duration_ms: u64) -> Result<()> {
    info!(target: "atx_power", duration_ms, "Pressing ATX reset button");
    pulse_output_pin(ATX_RESET_PIN, "reset_button", duration_ms)
}

pub fn get_atx_state() -> Result<AtxState> {
    if !ATX_MOUNTED.load(Ordering::SeqCst) {
        bail!("ATX control not mounted – call mount_atx_control() first");
    }

    let state = read_hw_state()?;

    let prev_pwr = LED_PWR_STATE.swap(state.power, Ordering::SeqCst);
    let prev_hdd = LED_HDD_STATE.swap(state.hdd, Ordering::SeqCst);

    if prev_pwr != state.power || prev_hdd != state.hdd {
        debug!(target: "atx_power",
            power = state.power,
            hdd = state.hdd,
            prev_power = prev_pwr,
            prev_hdd = prev_hdd,
            "ATX state changed");
    }

    if let Some(cell) = ATX_STATE.get()
        && let Ok(mut guard) = cell.try_write()
    {
        *guard = state;
    }

    Ok(state)
}

pub fn set_atx_power_action(action: &str) -> Result<()> {
    debug!(target: "atx_power", %action, "Executing ATX power action");

    match action {
        "power-short" => {
            debug!(target: "atx_power", "Simulating short power button press");
            press_atx_power_button(200)
        }
        "power-long" => {
            debug!(target: "atx_power", "Simulating long power button press");
            press_atx_power_button(5_000)
        }
        "reset" => {
            debug!(target: "atx_power", "Simulating reset button press");
            press_atx_reset_button(200)
        }
        other => {
            error!(target: "atx_power", action = other, "Invalid ATX power action");
            bail!(
                "Invalid ATX power action: '{other}'. \
                 Valid actions: power-short, power-long, reset"
            )
        }
    }
}
