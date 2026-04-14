use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chrono::{DateTime, Utc};
use libc::{CLOCK_REALTIME, clock_settime, timespec};
use reqwest::header;
use tracing::{info, warn};

fn system_time_to_timespec(st: SystemTime) -> anyhow::Result<timespec> {
    let dur = st.duration_since(UNIX_EPOCH).context("system time before UNIX_EPOCH")?;
    Ok(timespec {
        tv_sec: dur.as_secs() as libc::time_t,
        tv_nsec: dur.subsec_nanos() as libc::c_long,
    })
}

fn set_system_time(dt: DateTime<Utc>) -> anyhow::Result<()> {
    let ts = system_time_to_timespec(SystemTime::from(dt))?;
    let rc = unsafe { clock_settime(CLOCK_REALTIME, &ts as *const timespec) };
    if rc != 0 {
        return Err(anyhow::anyhow!("clock_settime failed: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

async fn http_date_with_rtt(
    url: &str,
    client: &reqwest::Client,
    method: &str,
) -> anyhow::Result<DateTime<Utc>> {
    let t0 = SystemTime::now();
    let resp = match method {
        "HEAD" => client.head(url).send().await?,
        _ => client.get(url).send().await?,
    };
    let t1 = SystemTime::now();

    let date_hdr = resp
        .headers()
        .get(header::DATE)
        .ok_or_else(|| anyhow::anyhow!("missing Date header"))?
        .to_str()
        .context("invalid Date header bytes")?;

    let srv_time = httpdate::parse_http_date(date_hdr)
        .with_context(|| format!("parse_http_date failed: {}", date_hdr))?;

    let rtt = t1.duration_since(t0).unwrap_or(Duration::ZERO);
    let corrected = srv_time + rtt / 2;

    Ok(DateTime::<Utc>::from(corrected))
}

pub async fn sync_time_once() -> anyhow::Result<()> {
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
        // Try HEAD first
        match http_date_with_rtt(u, &client, "HEAD").await {
            Ok(dt) => {
                set_system_time(dt)?;
                info!("Time synced via HTTP Date(+1/2RTT): {} (url: {})", dt, u);
                return Ok(());
            }
            Err(e1) => {
                // Fallback to GET
                match http_date_with_rtt(u, &client, "GET").await {
                    Ok(dt) => {
                        set_system_time(dt)?;
                        info!("Time synced via HTTP Date(+1/2RTT): {} (url: {})", dt, u);
                        return Ok(());
                    }
                    Err(e2) => {
                        warn!("HTTP time source failed: {} (HEAD: {}; GET: {})", u, e1, e2);
                    }
                }
            }
        }
    }

    Err(anyhow::anyhow!("all HTTP time sources failed"))
}
