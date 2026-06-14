use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use libc::{CLOCK_REALTIME, clock_settime, timespec};
use reqwest::header;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{info, warn};

use crate::error::{Error, Result};

static LAST_SYNC_UNIX_SECS: AtomicI64 = AtomicI64::new(0);

pub fn is_time_synced() -> bool {
    LAST_SYNC_UNIX_SECS.load(Ordering::Relaxed) > 0
}

pub fn last_sync_unix_secs() -> i64 {
    LAST_SYNC_UNIX_SECS.load(Ordering::Relaxed)
}

fn system_time_to_timespec(st: SystemTime) -> Result<timespec> {
    let dur = st.duration_since(UNIX_EPOCH)?;
    Ok(timespec {
        tv_sec: dur.as_secs() as libc::time_t,
        tv_nsec: dur.subsec_nanos() as libc::c_long,
    })
}

fn set_system_time(dt: DateTime<Utc>) -> Result<()> {
    let ts = system_time_to_timespec(SystemTime::from(dt))?;
    // SAFETY: `clock_settime` reads a single `timespec` through the pointer; `ts` is
    // a fully-initialised local that outlives the call, and `CLOCK_REALTIME` is a
    // valid clock id. The return code is checked below.
    let rc = unsafe { clock_settime(CLOCK_REALTIME, &ts as *const timespec) };
    if rc != 0 {
        return Err(Error::ClockSettime(std::io::Error::last_os_error()));
    }
    Ok(())
}

async fn http_date_with_rtt(
    url: &str,
    client: &reqwest::Client,
    method: &str,
) -> Result<DateTime<Utc>> {
    let t0 = SystemTime::now();
    let resp = match method {
        "HEAD" => client.head(url).send().await?,
        _ => client.get(url).send().await?,
    };
    let t1 = SystemTime::now();

    let date_hdr = resp
        .headers()
        .get(header::DATE)
        .ok_or(Error::MissingDateHeader)?
        .to_str()
        .map_err(Error::InvalidDateHeader)?;

    let srv_time = httpdate::parse_http_date(date_hdr)
        .map_err(|source| Error::ParseDate { date: date_hdr.to_string(), source })?;

    let rtt = t1.duration_since(t0).unwrap_or(Duration::ZERO);
    let corrected = srv_time + rtt / 2;

    Ok(DateTime::<Utc>::from(corrected))
}

pub async fn sync_time_once() -> Result<()> {
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .https_only(false)
        .timeout(Duration::from_secs(3))
        .tcp_keepalive(Duration::from_secs(30))
        .user_agent("rustkvm/timesync")
        .build()?;

    let urls_env = std::env::var("TIME_SYNC_URLS").ok();
    let urls: Vec<String> = urls_env
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_else(|| {
            vec![
                "http://www.gstatic.com/generate_204".to_string(),
                "http://cp.cloudflare.com/".to_string(),
                "http://edge-http.microsoft.com/captiveportal/generate_204".to_string(),
            ]
        });

    for u in urls.iter() {
        match http_date_with_rtt(u, &client, "HEAD").await {
            Ok(dt) => {
                set_system_time(dt)?;
                LAST_SYNC_UNIX_SECS.store(dt.timestamp(), Ordering::Relaxed);
                info!("Time synced via HTTP Date(+1/2RTT): {} (url: {})", dt, u);
                return Ok(());
            }
            Err(e1) => match http_date_with_rtt(u, &client, "GET").await {
                Ok(dt) => {
                    set_system_time(dt)?;
                    LAST_SYNC_UNIX_SECS.store(dt.timestamp(), Ordering::Relaxed);
                    info!("Time synced via HTTP Date(+1/2RTT): {} (url: {})", dt, u);
                    return Ok(());
                }
                Err(e2) => {
                    warn!("HTTP time source failed: {} (HEAD: {}; GET: {})", u, e1, e2);
                }
            },
        }
    }

    Err(Error::AllSourcesFailed)
}

pub fn spawn_periodic_resync(period: Duration) {
    tokio::spawn(async move {
        let mut tick = interval(period);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        tick.tick().await;

        loop {
            tick.tick().await;
            if let Err(e) = sync_time_once().await {
                warn!("periodic time sync failed: {}", e);
            }
        }
    });
}
