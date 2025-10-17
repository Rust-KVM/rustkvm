use std::sync::{Arc, OnceLock};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use prometheus::Encoder;
use serde_json as serde_json_crate;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{Duration, interval};
use tracing::{debug, info, warn};
use webrtc::media::Sample;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::jsonrpc::{JsonRpcProcessor, create_default_registry};
use crate::pipeline::{AudioConfig, PipelineManager, VideoConfig};

/// Video input state
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VideoInputState {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>, // no_signal, no_lock, out_of_range
    pub width: u32,
    pub height: u32,
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

// ----- GStreamer pipeline bridge -----

static VIDEO_SINK: tokio::sync::RwLock<Option<Arc<TrackLocalStaticSample>>> =
    tokio::sync::RwLock::const_new(None);
static AUDIO_SINK: tokio::sync::RwLock<Option<Arc<TrackLocalStaticSample>>> =
    tokio::sync::RwLock::const_new(None);

static AUDIO_FRAME_TX: tokio::sync::OnceCell<mpsc::Sender<Vec<u8>>> =
    tokio::sync::OnceCell::const_new();

// Pipeline starter
static VIDEO_PIPELINE_STARTED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

// GStreamer pipeline manager
static PIPELINE_MANAGER: tokio::sync::Mutex<Option<PipelineManager>> =
    tokio::sync::Mutex::const_new(None);

static VIDEO_STATE_TX: OnceLock<mpsc::UnboundedSender<VideoInputState>> = OnceLock::new();

type VideoFrame = (Vec<u8>, u64); // pts in microseconds
type AudioFrame = Vec<u8>;

static LATEST_VIDEO: Lazy<Mutex<Option<VideoFrame>>> = Lazy::new(|| Mutex::new(None));

fn video_writer_loop() {
    tokio::spawn(async move {
        let mut last_pts: Option<u64> = None;
        let mut current_track = None;
        let mut check = interval(Duration::from_secs(1));

        // default to 60fps; will be adjusted by LAST_VIDEO_STATE
        let mut frame_interval = Duration::from_millis(16);
        let mut tick = interval(frame_interval);

        loop {
            tokio::select! {
                _ = check.tick() => {
                    current_track = VIDEO_SINK.read().await.clone();

                    // adjust interval by current fps if available
                    let fps = {
                        let st = LAST_VIDEO_STATE.read().await;
                        if st.frame_per_second.is_finite() && st.frame_per_second > 0.0 {
                            st.frame_per_second
                        } else {
                            60.0
                        }
                    };

                    let ms = (1000.0 / fps) as u64;
                    let actual_fps = 1000.0 / ms as f64;

                    info!(
                        "Frame rate: GStreamer={:.1}fps -> output={:.1}fps (interval={}ms)",
                        fps, actual_fps, ms
                    );

                    let desired = Duration::from_millis(ms);
                    if desired != frame_interval {
                        frame_interval = desired;
                        tick = interval(frame_interval);
                    }
                }
                _ = tick.tick() => {
                    let frame = { LATEST_VIDEO.lock().take() };
                    if let (Some((data, pts)), Some(track)) = (frame, &current_track) {
                        let dur = match last_pts {
                            Some(prev) if pts > prev => {
                                let interval_us = pts - prev;
                                if interval_us > 0 {
                                    let calculated_fps = 1_000_000.0 / interval_us as f64;
                                    info!(
                                        "PTS interval: {}us, calculated FPS: {:.1}",
                                        interval_us, calculated_fps
                                    );
                                }
                                Duration::from_micros(interval_us)
                            },
                            _ => frame_interval,
                        };
                        last_pts = Some(pts);
                        let sample = webrtc::media::Sample { data: data.into(), duration: dur, ..Default::default() };

                        if let Err(e) = track.write_sample(&sample).await {
                            warn!("Failed to write video sample: {}", e);
                            // Track might be closed, refresh it
                            current_track = VIDEO_SINK.read().await.clone();
                        }
                    }
                }
            }
        }
    });
}

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
            // Video channel
            video_writer_loop();

            // Audio channel
            let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(32);
            let _ = AUDIO_FRAME_TX.set(audio_tx);
            tokio::spawn(audio_frame_writer(audio_rx));

            Ok(())
        })
        .await
        .map(|_| ())
}

pub async fn attach_webrtc_sink(track: Arc<TrackLocalStaticSample>) {
    *VIDEO_SINK.write().await = Some(track);
}

pub async fn attach_audio_sink(track: Arc<TrackLocalStaticSample>) {
    *AUDIO_SINK.write().await = Some(track);
}

pub async fn detach_webrtc_sink() {
    *VIDEO_SINK.write().await = None;
    *AUDIO_SINK.write().await = None;
}

async fn audio_frame_writer(mut rx: mpsc::Receiver<AudioFrame>) {
    const AUDIO_FRAME_DURATION: Duration = Duration::from_millis(10); // 10ms Opus frames

    // Clone track once outside the loop to avoid repeated RwLock acquisition
    let mut current_track: Option<Arc<TrackLocalStaticSample>> = None;
    let mut track_check_interval = interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = track_check_interval.tick() => {
                // Periodically check if track has changed (rare)
                current_track = AUDIO_SINK.read().await.clone();
            }
            frame = rx.recv() => {
                match frame {
                    Some(data) => {
                        // Fast path: no lock needed
                        if let Some(track) = &current_track {
                            let sample = Sample {
                                data: data.into(),
                                duration: AUDIO_FRAME_DURATION,
                                ..Default::default()
                            };
                            if let Err(e) = track.write_sample(&sample).await {
                                warn!("error writing audio sample: {}", e);
                                // Track might be closed, refresh it
                                current_track = AUDIO_SINK.read().await.clone();
                            }
                        } else {
                            // No track yet, check once
                            current_track = AUDIO_SINK.read().await.clone();
                        }
                    }
                    None => {
                        // sender closed: backoff instead of exiting; allow pipeline rebuild and reattach
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        current_track = AUDIO_SINK.read().await.clone();
                        continue;
                    }
                }
            }
        }
    }
}

// Start GStreamer video pipeline
pub async fn start_native_video(q: Option<f32>) -> anyhow::Result<()> {
    ensure_video_pipeline_started().await?;

    // Create video configuration with quality
    let video_config = VideoConfig::default().with_quality(q.unwrap_or(1.0));

    // Optional audio configuration
    let audio_config = Some(AudioConfig::default());

    // Create pipeline manager
    let manager = PipelineManager::new(video_config, audio_config)?;

    // Set video frame callback
    manager.set_video_callback(move |data, pts_us| {
        let mut slot = LATEST_VIDEO.lock();
        *slot = Some((data, pts_us));
    });

    // Set audio frame callback
    if let Some(tx) = AUDIO_FRAME_TX.get() {
        let tx_clone = tx.clone();
        manager.set_audio_callback(move |data| {
            let _ = tx_clone.try_send(data);
        });
    }

    // Set video state callback
    if let Some(state_tx) = VIDEO_STATE_TX.get() {
        let state_tx_clone = state_tx.clone();
        manager.set_state_callback(move |state| {
            let _ = state_tx_clone.send(state);
        });
    }

    // Start all pipelines
    manager.start()?;

    // Store manager
    *PIPELINE_MANAGER.lock().await = Some(manager);

    info!("GStreamer pipeline started with quality factor: {:?}", q);
    Ok(())
}

// Stop GStreamer pipeline
pub fn stop_native_video() {
    tokio::spawn(async {
        if let Some(manager) = PIPELINE_MANAGER.lock().await.take()
            && let Err(e) = manager.stop()
        {
            warn!("Error stopping pipeline: {}", e);
        }
    });
}

/// Graceful shutdown of video pipeline
pub async fn shutdown_video_pipeline() {
    info!("Shutting down video pipeline...");
    stop_native_video();

    // Clear video sink
    *VIDEO_SINK.write().await = None;

    info!("Video pipeline shutdown complete");
}

/// Update video quality dynamically (adjusts bitrate)
pub async fn update_video_quality(quality: f32) -> anyhow::Result<()> {
    if !(0.0..=1.0).contains(&quality) {
        anyhow::bail!("Quality must be between 0.0 and 1.0");
    }

    // Access pipeline manager and update encoder bitrate
    let manager = PIPELINE_MANAGER.lock().await;
    if let Some(manager) = manager.as_ref() {
        let base_bitrate = 20_000_000u32;
        let base_max_bitrate = 60_000_000u32;
        let q = quality.clamp(0.1, 2.0);
        let new_bitrate = (base_bitrate as f32 * q) as u32;
        let new_max_bitrate = (base_max_bitrate as f32 * q) as u32;

        manager.set_video_bitrate(new_bitrate, new_max_bitrate)?;
        info!(
            "Updated video quality to {:.2}, bitrate: {}, max: {}",
            quality, new_bitrate, new_max_bitrate
        );
    } else {
        anyhow::bail!("Pipeline not initialized");
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
    pub width: u32,
    pub height: u32,
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

        let processor = JsonRpcProcessor::new(Arc::new(create_default_registry()));
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
