use std::fs;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use prometheus::{Gauge, IntGauge, Opts};
use tracing::{debug, info, warn};

use crate::config::get_config_manager;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DcPowerState {
    #[serde(rename = "isOn")]
    pub is_on: bool,
    pub voltage: f64,
    pub current: f64,
    pub power: f64,
    #[serde(rename = "restoreState")]
    pub restore_state: i32,
}

impl Default for DcPowerState {
    fn default() -> Self {
        Self { is_on: false, voltage: 0.0, current: 0.0, power: 0.0, restore_state: -1 }
    }
}

static DC_STATE: OnceCell<Mutex<DcPowerState>> = OnceCell::new();

pub fn init_dc_state() {
    DC_STATE.get_or_init(|| Mutex::new(DcPowerState::default()));
}

static DC_VOLTAGE: OnceCell<Gauge> = OnceCell::new();
static DC_CURRENT: OnceCell<Gauge> = OnceCell::new();
static DC_POWER: OnceCell<Gauge> = OnceCell::new();
static DC_STATE_METRIC: OnceCell<IntGauge> = OnceCell::new();
static METRICS_REGISTERED: OnceCell<()> = OnceCell::new();

pub fn register_dc_metrics() {
    METRICS_REGISTERED.get_or_init(|| {
        let voltage =
            Gauge::with_opts(Opts::new("rustkvm_dc_voltage_volts", "DC voltage in volts"))
                .expect("Failed to create voltage gauge");
        prometheus::default_registry()
            .register(Box::new(voltage.clone()))
            .expect("Failed to register voltage gauge");
        DC_VOLTAGE.set(voltage).ok();

        let current = Gauge::with_opts(Opts::new(
            "rustkvm_dc_current_amperes",
            "Current DC power consumption in amperes",
        ))
        .expect("Failed to create current gauge");
        prometheus::default_registry()
            .register(Box::new(current.clone()))
            .expect("Failed to register current gauge");
        DC_CURRENT.set(current).ok();

        let power =
            Gauge::with_opts(Opts::new("rustkvm_dc_power_watts", "DC power consumption in watts"))
                .expect("Failed to create power gauge");
        prometheus::default_registry()
            .register(Box::new(power.clone()))
            .expect("Failed to register power gauge");
        DC_POWER.set(power).ok();

        let state = IntGauge::with_opts(Opts::new(
            "rustkvm_dc_power_state",
            "DC power state (1 = on, 0 = off)",
        ))
        .expect("Failed to create state gauge");
        prometheus::default_registry()
            .register(Box::new(state.clone()))
            .expect("Failed to register state gauge");
        DC_STATE_METRIC.set(state).ok();

        debug!("DC power Prometheus metrics registered");
    });
}

fn update_dc_metrics(state: &DcPowerState) {
    if let Some(g) = DC_VOLTAGE.get() {
        g.set(state.voltage);
    }
    if let Some(g) = DC_CURRENT.get() {
        g.set(state.current);
    }
    if let Some(g) = DC_POWER.get() {
        g.set(state.power);
    }
    if let Some(g) = DC_STATE_METRIC.get() {
        g.set(if state.is_on { 1 } else { 0 });
    }
}

const SYSFS_POWER_SUPPLY_BASE: &str = "/sys/class/power_supply";
const ENV_DC_POWER_GPIO: &str = "RUSTKVM_DC_POWER_GPIO";
const DEFAULT_DC_POWER_GPIO: u32 = 0;

fn read_sysfs_file(path: &str) -> Result<String> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read sysfs file: {path}"))?;
    Ok(content.trim().to_string())
}

fn write_sysfs_file(path: &str, value: &str) -> Result<()> {
    fs::write(path, value.as_bytes())
        .with_context(|| format!("Failed to write '{value}' to {path}"))?;
    debug!("Wrote '{value}' to {path}");
    Ok(())
}

fn find_power_supply_dir() -> Result<String> {
    let base = std::path::Path::new(SYSFS_POWER_SUPPLY_BASE);
    if !base.exists() {
        return Err(anyhow!("{SYSFS_POWER_SUPPLY_BASE} does not exist"));
    }

    let entries = fs::read_dir(base)
        .with_context(|| format!("Failed to read directory {SYSFS_POWER_SUPPLY_BASE}"))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let type_path = path.join("type");
        if type_path.exists() {
            let ps_type = fs::read_to_string(&type_path).unwrap_or_default();
            let ps_type = ps_type.trim();
            if ps_type != "Mains" && ps_type != "USB" && ps_type != "Battery" && ps_type != "UPS" {
                continue;
            }
        }

        let voltage_path = path.join("voltage_now");
        let current_path = path.join("current_now");
        if voltage_path.exists() && current_path.exists() {
            return Ok(path.to_string_lossy().to_string());
        }
    }

    Err(anyhow!("No suitable power supply found in {SYSFS_POWER_SUPPLY_BASE}"))
}

fn read_power_supply_values(dir: &str) -> Result<(f64, f64, f64)> {
    let voltage_uv: f64 = read_sysfs_file(&format!("{dir}/voltage_now"))?
        .parse()
        .with_context(|| "Failed to parse voltage_now")?;

    let current_ua: f64 = read_sysfs_file(&format!("{dir}/current_now"))?
        .parse()
        .with_context(|| "Failed to parse current_now")?;

    let voltage = voltage_uv / 1_000_000.0;
    let current = current_ua / 1_000_000.0;
    let power = voltage * current;

    Ok((voltage, current, power))
}

fn read_dc_on_state(gpio_pin: u32) -> Result<bool> {
    let value_path = format!("/sys/class/gpio/gpio{gpio_pin}/value");
    let val = read_sysfs_file(&value_path)?;
    match val.as_str() {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(anyhow!("Unexpected GPIO value: '{val}'")),
    }
}

fn dc_power_gpio() -> u32 {
    if let Ok(val) = std::env::var(ENV_DC_POWER_GPIO)
        && let Ok(pin) = val.parse::<u32>()
    {
        debug!("Using DC power GPIO pin {pin} from {ENV_DC_POWER_GPIO}");
        return pin;
    }
    DEFAULT_DC_POWER_GPIO
}

fn ensure_gpio_exported(pin: u32) -> Result<()> {
    let gpio_dir = format!("/sys/class/gpio/gpio{pin}");
    if std::path::Path::new(&gpio_dir).exists() {
        return Ok(());
    }

    let export_path = "/sys/class/gpio/export";
    write_sysfs_file(export_path, &pin.to_string())?;

    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

fn set_gpio_direction(pin: u32, direction: &str) -> Result<()> {
    let dir_path = format!("/sys/class/gpio/gpio{pin}/direction");
    write_sysfs_file(&dir_path, direction)
}

fn write_gpio_value(pin: u32, value: bool) -> Result<()> {
    let val_path = format!("/sys/class/gpio/gpio{pin}/value");
    write_sysfs_file(&val_path, if value { "1" } else { "0" })
}

pub fn get_dc_power_state() -> DcPowerState {
    init_dc_state();

    let mut is_on = false;
    let mut voltage = 0.0_f64;
    let mut current = 0.0_f64;
    let mut power = 0.0_f64;

    match find_power_supply_dir() {
        Ok(dir) => match read_power_supply_values(&dir) {
            Ok((v, a, w)) => {
                voltage = v;
                current = a;
                power = w;
                debug!("Read power supply from {dir}: {v:.3}V, {a:.3}A, {w:.3}W");
            }
            Err(e) => {
                warn!("Failed to read power supply values from {dir}: {e}");
            }
        },
        Err(e) => {
            debug!("No power supply found: {e}");
        }
    }

    let gpio = dc_power_gpio();
    if gpio != 0 {
        match read_dc_on_state(gpio) {
            Ok(on) => is_on = on,
            Err(e) => warn!("Failed to read DC power GPIO state: {e}"),
        }
    }

    if gpio == 0 {
        let state = DC_STATE.get().expect("DC_STATE not initialized").lock();
        is_on = state.is_on;
    }

    let restore_state = {
        let state = DC_STATE.get().expect("DC_STATE not initialized").lock();
        state.restore_state
    };

    DcPowerState { is_on, voltage, current, power, restore_state }
}

pub fn set_dc_power_state(enabled: bool) -> Result<()> {
    info!("Setting DC power state: enabled={enabled}");

    let gpio = dc_power_gpio();
    if gpio == 0 {
        warn!(
            "No DC power GPIO configured (set {ENV_DC_POWER_GPIO}) – updating in-memory state only"
        );
    } else {
        ensure_gpio_exported(gpio)?;
        set_gpio_direction(gpio, "out")?;
        write_gpio_value(gpio, enabled)?;
    }

    {
        let state_lock = DC_STATE.get_or_init(|| Mutex::new(DcPowerState::default()));
        let mut state = state_lock.lock();
        state.is_on = enabled;
    }

    let state = get_dc_power_state();
    update_dc_metrics(&state);

    Ok(())
}

pub fn set_dc_restore_state(state: i32) -> Result<()> {
    if state != 0 && state != 1 && state != 2 {
        return Err(anyhow!("Invalid restore state: {state} (expected 0=off, 1=on, 2=last)"));
    }

    info!("Setting DC restore state to {state}");

    let gpio = dc_power_gpio();
    if gpio != 0 {
        match state {
            0 => write_gpio_value(gpio + 1, false)?,
            1 => write_gpio_value(gpio + 1, true)?,
            2 => {
                debug!("Restore mode 'last state' \u{2013} hardware will manage");
            }
            other => return Err(anyhow!("invalid DC restore state: {other}")),
        }
    }

    {
        let state_lock = DC_STATE.get_or_init(|| Mutex::new(DcPowerState::default()));
        let mut dc = state_lock.lock();
        dc.restore_state = state;
    }

    Ok(())
}

pub fn get_dc_restore_state() -> i32 {
    init_dc_state();
    let state = DC_STATE.get().expect("DC_STATE not initialized").lock();
    state.restore_state
}

pub async fn get_active_extension() -> String {
    let config = get_config_manager().get().await;
    config.active_extension.unwrap_or_default()
}

pub async fn set_active_extension(extension_id: &str) -> Result<()> {
    let current = get_active_extension().await;

    if current == extension_id {
        debug!("Active extension already set to '{extension_id}'");
        return Ok(());
    }

    match current.as_str() {
        "atx-power" => {
            info!("Unmounting ATX power control");
            if let Err(e) = super::atx::unmount_atx_control() {
                warn!("Failed to unmount ATX power control: {e}");
            }
        }
        "dc-power" => {
            info!("Unmounting DC power control");
            unmount_dc_control();
        }
        _ => {}
    }

    get_config_manager()
        .update(|config| {
            config.active_extension =
                if extension_id.is_empty() { None } else { Some(extension_id.to_string()) };
        })
        .await
        .context("Failed to save active_extension to config")?;

    match extension_id {
        "atx-power" => {
            info!("Mounting ATX power control");
            super::atx::mount_atx_control()?;
        }
        "dc-power" => {
            info!("Mounting DC power control");
            mount_dc_control()?;
        }
        "" => {
            info!("No extension active");
        }
        other => {
            warn!("Unknown extension ID: {other}");
            return Err(anyhow!("Unknown extension ID: {other}"));
        }
    }

    Ok(())
}

pub fn mount_dc_control() -> Result<()> {
    init_dc_state();
    register_dc_metrics();

    let gpio = dc_power_gpio();
    if gpio != 0 {
        ensure_gpio_exported(gpio).with_context(|| format!("Failed to export GPIO pin {gpio}"))?;
        set_gpio_direction(gpio, "out")
            .with_context(|| format!("Failed to set GPIO {gpio} direction"))?;
        info!("DC power control mounted on GPIO pin {gpio}");
    } else {
        info!("DC power control mounted (no GPIO configured – set {ENV_DC_POWER_GPIO})");
    }

    Ok(())
}

pub fn unmount_dc_control() {
    let gpio = dc_power_gpio();
    if gpio != 0 {
        let unexport_path = "/sys/class/gpio/unexport";
        if let Err(e) = write_sysfs_file(unexport_path, &gpio.to_string()) {
            warn!("Failed to unexport GPIO pin {gpio}: {e}");
        }
        info!("DC power control unmounted (GPIO pin {gpio} unexported)");
    } else {
        info!("DC power control unmounted");
    }
}
