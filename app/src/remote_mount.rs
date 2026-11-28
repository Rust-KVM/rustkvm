use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use once_cell::sync::Lazy;
use parking_lot::{Condvar, Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tracing::{debug, trace, warn};

use crate::hardware::usb::storage::{WebRtcReadHandler, set_webrtc_read_handler};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiskReadRequest {
    #[serde(rename = "Start")]
    start: u64,
    #[serde(rename = "End")]
    end: u64,
}

const INBOUND_HEADER_LEN: usize = 16;

const READ_TIMEOUT: Duration = Duration::from_secs(5);

type SendTextFn = dyn Fn(&str) -> Result<()> + Send + Sync + 'static;

struct Transport {
    sender: RwLock<Option<Arc<SendTextFn>>>,
    state: Mutex<InFlight>,
    cv: Condvar,
}

#[derive(Default)]
struct InFlight {
    in_flight: bool,
    buf: Vec<u8>,
}

static TRANSPORT: Lazy<Transport> = Lazy::new(|| Transport {
    sender: RwLock::new(None),
    state: Mutex::new(InFlight::default()),
    cv: Condvar::new(),
});

pub fn webrtc_disk_set_sender(f: Arc<SendTextFn>) {
    *TRANSPORT.sender.write() = Some(f);
    debug!("webrtc disk sender installed");
}

pub fn webrtc_disk_clear_sender() {
    *TRANSPORT.sender.write() = None;
    debug!("webrtc disk sender cleared");
}

pub fn webrtc_disk_on_message(payload: &[u8]) {
    let mut data = payload;
    if data.len() >= INBOUND_HEADER_LEN {
        data = &data[INBOUND_HEADER_LEN..];
    } else {
        warn!("webrtc disk message too short to trim header, len={}", data.len());
    }

    let mut st = TRANSPORT.state.lock();
    st.buf.extend_from_slice(data);
    TRANSPORT.cv.notify_all();
    trace!("webrtc disk inbound appended, buffered={}", st.buf.len());
}

struct WebRtcBridge;

impl WebRtcBridge {
    fn new() -> Self {
        Self
    }
}

impl WebRtcReadHandler for WebRtcBridge {
    fn read(&self, offset: i64, size: i64) -> Result<Vec<u8>> {
        if offset < 0 || size <= 0 {
            return Err(anyhow!("invalid range"));
        }
        let start = offset as u64;
        let end = start + size as u64;
        let need = (end - start) as usize;

        let sender =
            TRANSPORT.sender.read().clone().ok_or_else(|| anyhow!("disk channel not ready"))?;

        {
            let mut st = TRANSPORT.state.lock();
            if st.in_flight {
                return Err(anyhow!("concurrent disk read not supported"));
            }
            st.in_flight = true;
            st.buf.clear();
        }
        // Every exit below, errors included, must free the slot: a read that
        // left it taken made each later read fail as "concurrent".
        let _in_flight = InFlightGuard;

        let req = DiskReadRequest { start, end };
        let text = serde_json::to_string(&req).context("serialize DiskReadRequest")?;
        sender(&text).context("send webrtc disk request")?;
        trace!("webrtc disk request sent: {}", text);

        let deadline = Instant::now() + READ_TIMEOUT;
        let mut st = TRANSPORT.state.lock();
        while st.buf.len() < need {
            let remain = deadline.saturating_duration_since(Instant::now());
            if remain.is_zero() || TRANSPORT.cv.wait_for(&mut st, remain).timed_out() {
                return Err(anyhow!("timeout waiting for webrtc disk data"));
            }
        }
        Ok(st.buf.drain(..need).collect())
    }
}

struct InFlightGuard;

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        TRANSPORT.state.lock().in_flight = false;
    }
}

pub fn install_webrtc_disk_bridge() {
    set_webrtc_read_handler(Some(Arc::new(WebRtcBridge::new())));
    debug!("webrtc disk bridge installed");
}
