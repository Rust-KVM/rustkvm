use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::get_config_manager;
use crate::hardware::usb::get_usb_manager;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JigglerConfig {
    pub inactivity_limit_seconds: i32,
    pub jitter_percentage: i32,
    pub schedule_cron_tab: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl Default for JigglerConfig {
    fn default() -> Self {
        Self {
            inactivity_limit_seconds: 60,
            jitter_percentage: 25,
            schedule_cron_tab: "0 * * * * *".to_string(),
            timezone: Some("UTC".to_string()),
        }
    }
}

static JIGGLER_CONFIG: OnceCell<Mutex<JigglerConfig>> = OnceCell::new();

static JIGGLER_CANCEL: OnceCell<Mutex<Option<CancellationToken>>> = OnceCell::new();

fn parse_cron_interval(cron_tab: &str) -> Duration {
    let cleaned = if let Some(idx) = cron_tab.find("TZ=") {
        if let Some(rest) = cron_tab[idx..].find(' ') {
            &cron_tab[idx + rest + 1..]
        } else {
            cron_tab
        }
    } else {
        cron_tab
    };

    let secs_field = cleaned.split_whitespace().next().unwrap_or("*");

    let secs = if let Some(stripped) = secs_field.strip_prefix("*/") {
        u64::from_str(stripped).unwrap_or(60).max(1)
    } else {
        60
    };

    Duration::from_secs(secs)
}

fn calculate_jitter_duration(base_interval: Duration, jitter_pct: i32) -> Duration {
    if jitter_pct <= 0 {
        return Duration::ZERO;
    }
    let pct = (jitter_pct.min(100) as f64) / 100.0;
    let seed =
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos() as u64;
    let fraction: f64 = ((seed % 100) as f64 / 100.0) * pct;
    base_interval.mul_f64(fraction)
}

fn do_jiggle() {
    let mgr = match get_usb_manager() {
        Some(m) => m,
        None => {
            debug!("USB manager not available, skipping jiggle");
            return;
        }
    };

    let hid = mgr.read().hid();

    fn simple_random(max: i8) -> i8 {
        let ns = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
        ((ns % (max as u32 * 2)) as i8 % max) + 1
    }
    let dx: i8 = {
        let v = simple_random(3);
        if SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
            .is_multiple_of(2)
        {
            -v
        } else {
            v
        }
    };
    let dy: i8 = {
        let v = simple_random(3);
        if SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
            .is_multiple_of(2)
        {
            -v
        } else {
            v
        }
    };

    if let Err(e) = hid.rel_mouse_report(dx, dy, 0) {
        warn!(error = %e, "Failed to jiggle mouse");
    } else {
        debug!(dx, dy, "Mouse jiggled");
    }
}

async fn run_jiggler_loop(cancel: CancellationToken) {
    let config = get_jiggler_config();
    let interval_dur = parse_cron_interval(&config.schedule_cron_tab);
    let inactivity_limit = Duration::from_secs(config.inactivity_limit_seconds as u64);
    let jitter_pct = config.jitter_percentage;
    drop(config);

    info!(
        interval_ms = interval_dur.as_millis(),
        inactivity_secs = %inactivity_limit.as_secs(),
        jitter_pct,
        "Jiggler loop started"
    );

    let mut ticker = interval(interval_dur);
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let jitter = calculate_jitter_duration(interval_dur, jitter_pct);
                if !jitter.is_zero() {
                    tokio::time::sleep(jitter).await;
                }

                let enabled = get_jiggler_state().await;
                if !enabled {
                    debug!("Jiggler disabled, skipping tick");
                    continue;
                }

                let elapsed = get_usb_manager()
                    .map(|m| m.read().hid().get_last_user_input_time().elapsed())
                    .unwrap_or(Duration::MAX);

                debug!(elapsed_ms = elapsed.as_millis(), "Time since last user input");

                if elapsed > inactivity_limit {
                    do_jiggle();
                } else {
                    debug!("User active recently, skipping jiggle");
                }
            }
            _ = cancel.cancelled() => {
                info!("Jiggler loop cancelled");
                break;
            }
        }
    }
}

pub async fn get_jiggler_state() -> bool {
    get_config_manager().get().await.jiggler_enabled
}

pub async fn set_jiggler_state(enabled: bool) -> Result<()> {
    get_config_manager()
        .update(|cfg| {
            cfg.jiggler_enabled = enabled;
        })
        .await?;

    info!(enabled, "Jiggler state updated");

    Ok(())
}

pub fn get_jiggler_config() -> JigglerConfig {
    JIGGLER_CONFIG.get_or_init(|| Mutex::new(JigglerConfig::default())).lock().clone()
}

pub async fn set_jiggler_config(cfg: JigglerConfig) -> Result<()> {
    info!(
        inactivity_limit_seconds = cfg.inactivity_limit_seconds,
        jitter_percentage = cfg.jitter_percentage,
        schedule = %cfg.schedule_cron_tab,
        timezone = ?cfg.timezone,
        "Setting jiggler config"
    );

    *JIGGLER_CONFIG.get_or_init(|| Mutex::new(JigglerConfig::default())).lock() = cfg;

    cancel_jiggler_cron();
    run_jiggler_cron_tab().await;

    Ok(())
}

pub async fn run_jiggler_cron_tab() {
    cancel_jiggler_cron();

    let cancel = CancellationToken::new();
    let child = cancel.clone();

    *JIGGLER_CANCEL.get_or_init(|| Mutex::new(None)).lock() = Some(cancel);

    tokio::spawn(async move {
        run_jiggler_loop(child).await;
    });

    info!("Jiggler cron scheduled");
}

pub fn cancel_jiggler_cron() {
    let token = JIGGLER_CANCEL.get_or_init(|| Mutex::new(None)).lock().take();

    if let Some(t) = token {
        t.cancel();
        info!("Jiggler cron cancelled");
    }
}
