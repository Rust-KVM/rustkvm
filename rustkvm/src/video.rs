use std::sync::{Arc, OnceLock};
use std::time::Duration as StdDuration;

use once_cell::sync::Lazy;
use prometheus::Encoder;
use serde_json as serde_json_crate;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, info, warn};
use webrtc::media::Sample;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::jsonrpc::{JsonRpcProcessor, create_default_registry};

/// Video input state
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VideoInputState {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>, // no_signal, no_lock, out_of_range
    pub width: i32,
    pub height: i32,
    #[serde(rename = "fps")]
    pub frame_per_second: f64,
}

/// Last video state
static LAST_VIDEO_STATE: RwLock<VideoInputState> = RwLock::const_new(VideoInputState {
    ready: false,
    error: None,
    width: 0,
    height: 0,
    frame_per_second: 0.0,
});

// ----- Native video FFI bridge (authoritative path) -----

static VIDEO_SINK: tokio::sync::RwLock<Option<Arc<TrackLocalStaticSample>>> =
    tokio::sync::RwLock::const_new(None);
type FramePacket = (Vec<u8>, u64);
static VIDEO_FRAME_TX: tokio::sync::OnceCell<mpsc::Sender<FramePacket>> =
    tokio::sync::OnceCell::const_new();

// FFI pipeline starter (for direct frame ingress)
static VIDEO_PIPELINE_STARTED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

static VIDEO_STATE_TX: OnceLock<mpsc::UnboundedSender<VideoInputState>> = OnceLock::new();

// Prometheus drop counter
static VIDEO_DROP_COUNTER: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "rustkvm_video_frame_drops_total",
        "Total number of dropped video frames in RustKVM pipeline"
    )
    .expect("register rustkvm_video_frame_drops_total")
});

pub async fn init_video_state_updater() -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<VideoInputState>();
    VIDEO_STATE_TX.set(tx).map_err(|_| anyhow::anyhow!("video state tx already set"))?;

    tokio::spawn(async move {
        while let Some(st) = rx.recv().await {
            *LAST_VIDEO_STATE.write().await = st.clone();
            info!(
                "Video state updated: {}x{} @ {:.2}fps, ready={}",
                st.width, st.height, st.frame_per_second, st.ready
            );
            trigger_video_state_update_rpc().await;
        }
    });

    Ok(())
}

pub async fn ensure_video_pipeline_started() -> anyhow::Result<()> {
    VIDEO_PIPELINE_STARTED
        .get_or_try_init(|| async {
            let (tx, rx) = mpsc::channel::<FramePacket>(32);
            let _ = VIDEO_FRAME_TX.set(tx);
            tokio::spawn(video_frame_writer(rx));

            Ok(())
        })
        .await
        .map(|_| ())
}

pub async fn attach_webrtc_sink(track: Arc<TrackLocalStaticSample>) {
    *VIDEO_SINK.write().await = Some(track);
}

pub async fn detach_webrtc_sink() {
    *VIDEO_SINK.write().await = None;
}

async fn video_frame_writer(mut rx: mpsc::Receiver<FramePacket>) {
    let mut last_pts_us: Option<u64> = None;
    while let Some((data, pts_us)) = rx.recv().await {
        let dur = match last_pts_us {
            Some(prev) if pts_us > prev => {
                let delta_us = pts_us - prev;
                let capped = delta_us.min(1_000_000);
                StdDuration::from_micros(capped)
            }
            _ => StdDuration::from_micros(16_666),
        };
        last_pts_us = Some(pts_us);

        if let Some(track) = VIDEO_SINK.read().await.clone() {
            let sample = Sample { data: data.into(), duration: dur, ..Default::default() };
            if let Err(e) = track.write_sample(&sample).await {
                warn!("error writing video sample: {}", e);
            }
        }
    }
}

// C -> Rust H.264 frame ingress (called per encoded frame)
#[unsafe(no_mangle)]
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers. The caller must ensure:
/// - `data` is a valid pointer to `len` bytes of memory
/// - `len` is not larger than the actual allocated memory
/// - The memory remains valid for the duration of this function call
pub unsafe extern "C" fn rustkvm_on_h264_frame(data: *const u8, len: usize, pts_us: u64) {
    if data.is_null() || len == 0 {
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    if let Some(tx) = VIDEO_FRAME_TX.get() {
        let mut owned = Vec::with_capacity(len);
        owned.extend_from_slice(slice);
        if tx.try_send((owned, pts_us)).is_err() {
            VIDEO_DROP_COUNTER.inc();
        }
    }
}

#[unsafe(no_mangle)]
/// # Safety
///
/// This function is called from C code via FFI and is therefore `unsafe`.
/// The caller must ensure the following preconditions are met:
/// - `error` is either a null pointer or a valid NUL-terminated C string
///   that remains valid for the duration of this call.
/// - The values of `width`, `height`, and `fps` reflect the current detected
///   video format and are consistent with the produced H.264 stream.
/// - The function may be called from non-Rust threads; it must not be called
///   after the Rust runtime has been torn down.
///
/// Inside this function we immediately convert the C string (if present) into
/// an owned `String` and then forward an owned `VideoInputState` via an
/// unbounded channel, avoiding holding raw pointers across threads.
pub unsafe extern "C" fn rustkvm_on_video_state_changed(
    ready: i32,
    width: u16,
    height: u16,
    fps: f64,
    error: *const std::os::raw::c_char,
) {
    let error_str = if error.is_null() {
        None
    } else {
        let c_str = unsafe { std::ffi::CStr::from_ptr(error) };
        c_str.to_str().ok().map(|s| s.to_string())
    };

    let st = VideoInputState {
        ready: ready != 0,
        error: error_str,
        width: width as i32,
        height: height as i32,
        frame_per_second: fps,
    };

    if let Some(tx) = VIDEO_STATE_TX.get() {
        let _ = tx.send(st);
    } else {
        debug!("video state updater not initialized");
    }
}

// FFI to control native (C) video pipeline
unsafe extern "C" {
    fn rustkvm_native_video_init() -> i32;
    fn rustkvm_native_video_start();
    fn rustkvm_native_video_stop();
    fn rustkvm_native_video_shutdown();
    fn rustkvm_native_video_set_quality(q: f32);
}

pub async fn start_native_video(q: Option<f32>) -> anyhow::Result<()> {
    ensure_video_pipeline_started().await?;
    unsafe {
        if let Some(v) = q {
            rustkvm_native_video_set_quality(v);
        }
        if rustkvm_native_video_init() != 0 {
            anyhow::bail!("native video init failed");
        }
        rustkvm_native_video_start();
    }
    Ok(())
}

pub fn stop_native_video() {
    unsafe {
        rustkvm_native_video_stop();
        rustkvm_native_video_shutdown();
    }
}

/// Graceful shutdown of video pipeline
pub async fn shutdown_video_pipeline() {
    info!("Shutting down video pipeline...");
    stop_native_video();

    // Clear video sink
    *VIDEO_SINK.write().await = None;

    info!("Video pipeline shutdown complete");
}

/// Update video quality dynamically
pub fn update_video_quality(quality: f32) -> anyhow::Result<()> {
    if !(0.0..=1.0).contains(&quality) {
        anyhow::bail!("Quality must be between 0.0 and 1.0");
    }

    unsafe {
        rustkvm_native_video_set_quality(quality);
    }
    Ok(())
}

/// Get video pipeline statistics
pub async fn get_video_stats() -> anyhow::Result<VideoStats> {
    let state = get_video_state().await;
    let stats = VideoStats {
        ready: state.ready,
        error: state.error.clone(),
        width: state.width,
        height: state.height,
        fps: state.frame_per_second,
        has_sink: VIDEO_SINK.read().await.is_some(),
        pipeline_started: VIDEO_PIPELINE_STARTED.get().is_some(),
    };
    Ok(stats)
}

/// Video pipeline statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct VideoStats {
    pub ready: bool,
    pub error: Option<String>,
    pub width: i32,
    pub height: i32,
    pub fps: f64,
    pub has_sink: bool,
    pub pipeline_started: bool,
}

/// Get current video state
pub async fn get_video_state() -> VideoInputState {
    LAST_VIDEO_STATE.read().await.clone()
}

/// Update video state
pub async fn handle_video_state_message(video_state: VideoInputState) {
    *LAST_VIDEO_STATE.write().await = video_state.clone();
    trigger_video_state_update_rpc().await;
    // Update LVGL display to reflect new video state
    let _ = crate::hardware::display::request_display_update(true).await;
}

/// Public interface for video state update (called from webrtc.rs or native events)
pub async fn trigger_video_state_update_rpc() {
    let video_state = LAST_VIDEO_STATE.read().await.clone();
    debug!("Triggering video state update: {:?}", video_state);

    if let Some(session_id) = crate::webrtc::get_current_session().await {
        let mut session = crate::session::Session::new(session_id.clone());
        if let Some(rpc_channel) = crate::webrtc::get_rpc_channel(&session_id).await {
            // Optional: check channel open state like webrtc.rs does
            if rpc_channel.ready_state()
                != webrtc::data_channel::data_channel_state::RTCDataChannelState::Open
            {
                // Avoid spamming logs on normal race
                return;
            }
            session.rpc_channel = Some(rpc_channel);
        } else {
            return;
        }

        let processor = JsonRpcProcessor::new(std::sync::Arc::new(create_default_registry()));
        let params = match serde_json_crate::to_value(&video_state) {
            Ok(v) => Some(v),
            Err(e) => {
                warn!("Failed to serialize video state: {}", e);
                None
            }
        };
        if let Err(e) = processor.send_event("videoInputState", params, &session).await {
            warn!("Failed to send videoInputState event: {}", e);
        }
    }
}

pub fn video_metrics_snapshot() -> String {
    let metric_families = prometheus::gather();
    let mut buf = Vec::new();
    let encoder = prometheus::TextEncoder::new();
    if let Err(e) = encoder.encode(&metric_families, &mut buf) {
        tracing::warn!("Failed to encode metrics: {}", e);
        return "# ERROR: Failed to encode metrics\n".to_string();
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Write control action
pub async fn write_ctrl_action(action: &str) -> anyhow::Result<()> {
    use crate::hardware::native::socket::call_ctrl_action;
    info!("Writing control action: {}", serde_json::json!({ "action": action }));
    let _ = call_ctrl_action(action, None).await?;
    Ok(())
}
