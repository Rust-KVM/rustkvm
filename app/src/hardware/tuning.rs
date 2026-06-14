use std::path::Path;

use tokio::fs;
use tracing::{debug, info, warn};

pub const BIG_CORES: &[usize] = &[4, 5, 6, 7];

const BIG_CORE_AFFINITY_MASK: &str = "f0";

async fn detect_rk3588() -> bool {
    match fs::read("/proc/device-tree/compatible").await {
        Ok(bytes) => {
            let s = String::from_utf8_lossy(&bytes);
            s.split('\0').any(|t| t == "rockchip,rk3588")
        }
        Err(_) => false,
    }
}

pub async fn apply_rk3588_tuning() {
    if !detect_rk3588().await {
        debug!("not running on RK3588, skipping platform tuning");
        return;
    }
    info!("RK3588 detected — applying IRQ affinity and cpufreq tuning");

    pin_hot_irqs_to_big_cores().await;
    set_big_core_governor("performance").await;
}

async fn pin_hot_irqs_to_big_cores() {
    let data = match fs::read_to_string("/proc/interrupts").await {
        Ok(s) => s,
        Err(e) => {
            warn!("failed to read /proc/interrupts: {e}");
            return;
        }
    };

    let mut pinned = 0u32;
    for line in data.lines() {
        let Some((irq_field, rest)) = line.split_once(':') else {
            continue;
        };
        let Ok(irq) = irq_field.trim().parse::<u32>() else {
            continue;
        };
        if !is_hot_irq(rest) {
            continue;
        }

        let path = format!("/proc/irq/{irq}/smp_affinity");
        match fs::write(&path, BIG_CORE_AFFINITY_MASK).await {
            Ok(()) => {
                let driver = rest.split_whitespace().last().unwrap_or("?");
                info!(irq, driver, mask = BIG_CORE_AFFINITY_MASK, "pinned IRQ to A76");
                pinned += 1;
            }
            Err(e) => {
                warn!("failed to set affinity for IRQ {irq}: {e}");
            }
        }
    }
    if pinned == 0 {
        debug!("no hot IRQs matched — driver names may have changed");
    }
}

fn is_hot_irq(driver_field: &str) -> bool {
    driver_field.contains("rkvenc-core")     // H.264/H.265 video encoder
        || driver_field.contains("rkvdec-core")  // video decoder
        || driver_field.contains("rkjpeg")       // hardware JPEG
        || driver_field.trim_end().ends_with("dwc3") // USB-3 gadget controller
}

async fn set_big_core_governor(governor: &str) {
    for cpu in BIG_CORES {
        let path = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor");
        if !Path::new(&path).exists() {
            continue;
        }
        match fs::write(&path, governor).await {
            Ok(()) => info!(cpu, governor, "cpufreq governor applied"),
            Err(e) => warn!("failed to set cpu{cpu} governor: {e}"),
        }
    }
}
