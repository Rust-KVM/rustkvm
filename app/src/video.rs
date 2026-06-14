use std::sync::{Arc, OnceLock};

use prometheus::Encoder;
use serde_json as serde_json_crate;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{Duration, interval};
use tracing::{debug, info, warn};
use webrtc::media::Sample;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::api::{JsonRpcProcessor, default_registry};
use crate::pipeline::{AudioConfig, PipelineManager, VideoConfig};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VideoInputState {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub width: u32,
    pub height: u32,
    #[serde(rename = "fps")]
    pub frame_per_second: f64,
}

static LAST_VIDEO_STATE: RwLock<VideoInputState> = RwLock::const_new(VideoInputState {
    ready: false,
    error: None,
    width: 0,
    height: 0,
    frame_per_second: 0.0,
});

static VIDEO_SINK: tokio::sync::RwLock<Option<Arc<TrackLocalStaticSample>>> =
    tokio::sync::RwLock::const_new(None);
static AUDIO_SINK: tokio::sync::RwLock<Option<Arc<TrackLocalStaticSample>>> =
    tokio::sync::RwLock::const_new(None);

static AUDIO_FRAME_TX: tokio::sync::OnceCell<mpsc::Sender<bytes::Bytes>> =
    tokio::sync::OnceCell::const_new();

static VIDEO_PIPELINE_STARTED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

static PIPELINE_MANAGER: tokio::sync::Mutex<Option<PipelineManager>> =
    tokio::sync::Mutex::const_new(None);

static VIDEO_STATE_TX: OnceLock<mpsc::UnboundedSender<VideoInputState>> = OnceLock::new();

type VideoFrame = (bytes::Bytes, u64);
type AudioFrame = bytes::Bytes;

static VIDEO_FRAME_TX: tokio::sync::OnceCell<mpsc::Sender<VideoFrame>> =
    tokio::sync::OnceCell::const_new();

fn video_writer_loop(mut rx: mpsc::Receiver<VideoFrame>) {
    tokio::spawn(async move {
        let mut last_pts: Option<u64> = None;
        let mut current_track: Option<Arc<TrackLocalStaticSample>> = None;
        let mut track_refresh = interval(Duration::from_secs(1));
        track_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = track_refresh.tick() => {
                    current_track = VIDEO_SINK.read().await.clone();
                }
                maybe_frame = rx.recv() => {
                    let Some((data, pts)) = maybe_frame else { break; };

                    let dur = match last_pts {
                        Some(prev) if pts > prev => Duration::from_micros(pts - prev),
                        _ => Duration::from_millis(16),
                    };
                    last_pts = Some(pts);

                    if current_track.is_none() {
                        current_track = VIDEO_SINK.read().await.clone();
                    }
                    if let Some(track) = &current_track {
                        let sample = Sample {
                            data,
                            duration: dur,
                            ..Default::default()
                        };
                        if let Err(e) = track.write_sample(&sample).await {
                            warn!("Failed to write video sample: {}", e);
                            current_track = None;
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
            let (video_tx, video_rx) = mpsc::channel::<VideoFrame>(2);
            VIDEO_FRAME_TX
                .set(video_tx)
                .map_err(|_| anyhow::anyhow!("video frame tx already set"))?;
            video_writer_loop(video_rx);

            let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(32);
            let _ = AUDIO_FRAME_TX.set(audio_tx);
            tokio::spawn(audio_frame_writer(audio_rx));

            Ok::<(), anyhow::Error>(())
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
    const AUDIO_FRAME_DURATION: Duration = Duration::from_millis(10);

    let mut current_track: Option<Arc<TrackLocalStaticSample>> = None;
    let mut track_check_interval = interval(Duration::from_secs(1));
    track_check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = track_check_interval.tick() => {
                current_track = AUDIO_SINK.read().await.clone();
            }
            frame = rx.recv() => {
                match frame {
                    Some(data) => {
                        if let Some(track) = &current_track {
                            let sample = Sample {
                                data,
                                duration: AUDIO_FRAME_DURATION,
                                ..Default::default()
                            };
                            if let Err(e) = track.write_sample(&sample).await {
                                warn!("error writing audio sample: {}", e);
                                current_track = AUDIO_SINK.read().await.clone();
                            }
                        } else {
                            current_track = AUDIO_SINK.read().await.clone();
                        }
                    }
                    None => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        current_track = AUDIO_SINK.read().await.clone();
                        continue;
                    }
                }
            }
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn start_native_video_with_cli(cli: &crate::cli::Cli) -> anyhow::Result<()> {
    ensure_video_pipeline_started().await?;

    let video_config = VideoConfig::from_cli(&cli.video, cli.quality);

    let audio_config =
        if cli.audio_enabled { Some(AudioConfig::from_cli(&cli.audio)) } else { None };

    let manager = PipelineManager::new(video_config, audio_config)?;

    manager.set_video_callback(move |data, pts_us| {
        if let Some(tx) = VIDEO_FRAME_TX.get() {
            let _ = tx.try_send((data, pts_us));
        }
    });

    if let Some(tx) = AUDIO_FRAME_TX.get() {
        let tx_clone = tx.clone();
        manager.set_audio_callback(move |data| {
            let _ = tx_clone.try_send(data);
        });
    }

    if let Some(state_tx) = VIDEO_STATE_TX.get() {
        let state_tx_clone = state_tx.clone();
        manager.set_state_callback(move |state| {
            let _ = state_tx_clone.send(state);
        });
    }

    manager.start()?;

    *PIPELINE_MANAGER.lock().await = Some(manager);

    info!(
        "GStreamer pipeline started: {} encoder, quality={:.2}",
        cli.video.video_encoder.codec_name(),
        cli.quality
    );
    Ok(())
}

pub async fn shutdown_video_pipeline() {
    info!("Shutting down video pipeline...");

    if let Some(manager) = PIPELINE_MANAGER.lock().await.take() {
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || manager.stop()),
        )
        .await
        {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => warn!("Error stopping pipeline: {}", e),
            Ok(Err(e)) => warn!("Pipeline stop task panicked: {}", e),
            Err(_) => warn!("Pipeline stop timed out after 5s; continuing shutdown"),
        }
    }

    *VIDEO_SINK.write().await = None;

    info!("Video pipeline shutdown complete");
}

pub async fn force_video_keyframe() {
    let manager = PIPELINE_MANAGER.lock().await;
    if let Some(manager) = manager.as_ref() {
        manager.force_keyframe();
    }
}

pub async fn update_video_quality(quality: f32) -> anyhow::Result<()> {
    if !(0.0..=1.0).contains(&quality) {
        anyhow::bail!("Quality must be between 0.0 and 1.0");
    }

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

pub async fn get_video_state() -> VideoInputState {
    LAST_VIDEO_STATE.read().await.clone()
}

pub async fn handle_video_state_message(video_state: VideoInputState) {
    *LAST_VIDEO_STATE.write().await = video_state.clone();
    trigger_video_state_update_rpc().await;
    let _ = crate::hardware::display::request_display_update(true).await;
}

pub async fn trigger_video_state_update_rpc() {
    let video_state = LAST_VIDEO_STATE.read().await.clone();
    debug!("Triggering video state update: {:?}", video_state);

    if let Some(session_id) = crate::webrtc::get_current_session().await {
        let mut session = crate::session::Session::new(session_id.clone());
        if let Some(rpc_channel) = crate::webrtc::get_rpc_channel(&session_id).await {
            if rpc_channel.ready_state()
                != webrtc::data_channel::data_channel_state::RTCDataChannelState::Open
            {
                return;
            }
            session.rpc_channel = Some(rpc_channel);
        } else {
            return;
        }

        let processor = JsonRpcProcessor::new(default_registry());
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

pub async fn write_ctrl_action(action: &str) -> anyhow::Result<()> {
    use crate::hardware::native::socket::call_ctrl_action;
    info!("Writing control action: {}", serde_json::json!({ "action": action }));
    let _ = call_ctrl_action(action, None).await?;
    Ok(())
}
