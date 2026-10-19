use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use prometheus::Encoder;
use serde_json as serde_json_crate;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{Duration, Instant, interval};
use tracing::{debug, info, warn};
use webrtc::media::Sample;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::api::{JsonRpcProcessor, default_registry};
use crate::hardware::v4l2::{CaptureProbe, RelockVerdict, RelockWatch, pulse_hotplug};
use crate::pipeline::{AudioConfig, H264Profile, PipelineManager, VideoConfig};

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

static AUDIO_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_audio_enabled(enabled: bool) {
    AUDIO_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_audio_enabled() -> bool {
    AUDIO_ENABLED.load(Ordering::Relaxed)
}

static VIDEO_PIPELINE_STARTED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

static PIPELINE_MANAGER: tokio::sync::Mutex<Option<PipelineManager>> =
    tokio::sync::Mutex::const_new(None);

/// The H.264 profile the attached viewer negotiated. It overrides the
/// configured profile on the running encoder and on every rebuild, so the
/// bitstream always matches the payload type the browser decodes it as.
static H264_PROFILE: parking_lot::Mutex<Option<H264Profile>> = parking_lot::Mutex::new(None);

static PIPELINE_CONFIG: OnceLock<(VideoConfig, Option<AudioConfig>)> = OnceLock::new();
static SUPERVISOR_STOPPED: AtomicBool = AtomicBool::new(false);

const SUPERVISOR_PERIOD: Duration = Duration::from_secs(2);
/// No raw frame for this long while the receiver reports a signal means the
/// capture stream is wedged; repeat stalls back off like failed restarts.
const STALL_LIMIT: Duration = Duration::from_secs(10);
const RESTART_BACKOFF_BASE: Duration = Duration::from_secs(5);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(300);

static VIDEO_STATE_TX: OnceLock<mpsc::UnboundedSender<VideoInputState>> = OnceLock::new();

type VideoFrame = (bytes::Bytes, u64);
type AudioFrame = bytes::Bytes;

static VIDEO_FRAME_TX: tokio::sync::OnceCell<mpsc::Sender<VideoFrame>> =
    tokio::sync::OnceCell::const_new();

/// Writes encoded frames to the current session's track. The sink is read per
/// frame rather than cached: a cached track kept feeding a taken-over session
/// for up to a second after the new one attached, delaying its first frame.
fn video_writer_loop(mut rx: mpsc::Receiver<VideoFrame>) {
    tokio::spawn(async move {
        let mut last_pts: Option<u64> = None;
        while let Some((data, pts)) = rx.recv().await {
            let dur = match last_pts {
                Some(prev) if pts > prev => Duration::from_micros(pts - prev),
                _ => Duration::from_millis(16),
            };
            last_pts = Some(pts);

            let Some(track) = VIDEO_SINK.read().await.clone() else { continue };
            let sample = Sample { data, duration: dur, ..Default::default() };
            if let Err(e) = track.write_sample(&sample).await {
                warn!("Failed to write video sample: {}", e);
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

/// Makes the encoder emit `profile`, the H.264 profile a new viewer
/// negotiated. Called before the viewer's keyframe is requested, so its first
/// IDR already carries the new SPS.
pub async fn set_h264_profile(profile: H264Profile) {
    if H264_PROFILE.lock().replace(profile) == Some(profile) {
        return;
    }
    if let Some(manager) = PIPELINE_MANAGER.lock().await.as_ref() {
        manager.set_h264_profile(profile);
    }
    info!(profile = profile.as_gst_str(), "H.264 encoder profile set to match the viewer");
}

/// Switches video output to a new session's track. The keyframe for the new
/// viewer is requested once its connection is fully up (see webrtc.rs).
pub async fn attach_webrtc_sink(track: Arc<TrackLocalStaticSample>) {
    *VIDEO_SINK.write().await = Some(track);
}

pub async fn attach_audio_sink(track: Arc<TrackLocalStaticSample>) {
    *AUDIO_SINK.write().await = Some(track);
}

pub async fn detach_webrtc_sink() {
    *VIDEO_SINK.write().await = None;
    *AUDIO_SINK.write().await = None;
    set_encoding_active(false).await;
}

/// Encoding only runs while a session has a video track attached.
pub async fn set_encoding_active(active: bool) {
    if let Some(manager) = PIPELINE_MANAGER.lock().await.as_ref() {
        manager.set_encoding_active(active);
    }
}

async fn audio_frame_writer(mut rx: mpsc::Receiver<AudioFrame>) {
    const AUDIO_FRAME_DURATION: Duration = Duration::from_millis(10);

    // Read the sink per frame for the same reason as the video writer.
    while let Some(data) = rx.recv().await {
        if !is_audio_enabled() {
            continue;
        }
        let Some(track) = AUDIO_SINK.read().await.clone() else { continue };
        let sample = Sample { data, duration: AUDIO_FRAME_DURATION, ..Default::default() };
        if let Err(e) = track.write_sample(&sample).await {
            warn!("error writing audio sample: {}", e);
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn start_native_video_with_cli(cli: &crate::cli::Cli) -> anyhow::Result<()> {
    ensure_video_pipeline_started().await?;

    let video_config = VideoConfig::from_cli(&cli.video, cli.quality);

    let audio_enabled = crate::config::get_config_manager().get().await.audio_enabled;
    set_audio_enabled(audio_enabled);

    let audio_config = Some(AudioConfig::from_cli(&cli.audio));
    let _ = PIPELINE_CONFIG.set((video_config.clone(), audio_config.clone()));

    start_pipeline(video_config, audio_config).await?;

    info!(
        "GStreamer pipeline started: {} encoder, quality={:.2}",
        cli.video.video_encoder.codec_name(),
        cli.quality
    );
    Ok(())
}

async fn start_pipeline(
    mut video_config: VideoConfig,
    audio_config: Option<AudioConfig>,
) -> anyhow::Result<()> {
    if matches!(video_config.encoder, crate::cli::VideoEncoder::H264)
        && let Some(profile) = *H264_PROFILE.lock()
    {
        video_config.profile = Some(profile.as_gst_str().to_owned());
    }
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
    // A restarted pipeline comes up paused; resume it if a viewer is attached.
    manager.set_encoding_active(VIDEO_SINK.read().await.is_some());

    *PIPELINE_MANAGER.lock().await = Some(manager);
    Ok(())
}

async fn stop_pipeline() {
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
            Err(_) => warn!("Pipeline stop timed out after 5s; continuing"),
        }
    }
}

/// Tears down and rebuilds the capture/encode pipeline with the boot-time configuration,
/// keeping any attached WebRTC sink so connected viewers resume on the next keyframe.
pub async fn restart_video_pipeline() -> anyhow::Result<()> {
    let (video_config, audio_config) = PIPELINE_CONFIG
        .get()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("video pipeline was never configured"))?;
    stop_pipeline().await;
    crate::observability::metrics::VIDEO_PIPELINE_RESTARTS_TOTAL.inc();
    start_pipeline(video_config, audio_config).await?;
    info!("video pipeline restarted");
    Ok(())
}

pub async fn is_video_pipeline_running() -> bool {
    PIPELINE_MANAGER.lock().await.as_ref().is_some_and(PipelineManager::is_video_running)
}

pub fn restart_backoff(consecutive_failures: u32) -> Duration {
    RESTART_BACKOFF_BASE
        .saturating_mul(1u32 << consecutive_failures.min(16))
        .min(RESTART_BACKOFF_MAX)
}

/// How long hot-plug detect stays low on a software replug. Long enough for
/// the source to register an unplug.
const REPLUG_LOW: Duration = Duration::from_secs(2);

/// State the supervisor keeps between ticks.
struct LiveCheck {
    probe: Option<Arc<CaptureProbe>>,
    probe_error_logged: bool,
    relock_events: Option<Arc<AtomicU32>>,
    relock: RelockWatch,
    last_frames: u64,
    last_progress: Instant,
    stall_restarts: u32,
    caps_logged: bool,
}

impl LiveCheck {
    fn new() -> Self {
        Self {
            probe: None,
            probe_error_logged: false,
            relock_events: None,
            relock: RelockWatch::default(),
            last_frames: 0,
            last_progress: Instant::now(),
            stall_restarts: 0,
            caps_logged: false,
        }
    }

    /// Forgets everything tied to the pipeline that was just replaced.
    fn pipeline_replaced(&mut self) {
        self.last_frames = 0;
        self.last_progress = Instant::now();
        self.caps_logged = false;
    }

    fn probe(&mut self, device: &str) -> Option<Arc<CaptureProbe>> {
        if self.probe.is_none() {
            match CaptureProbe::open(device) {
                Ok(p) => {
                    let p = Arc::new(p);
                    if p.has_events() {
                        info!("watching the HDMI receiver for relock storms");
                        self.relock_events = Some(spawn_relock_counter(p.clone()));
                    } else {
                        warn!(
                            "capture node has no source-change events; relock storms go undetected"
                        );
                    }
                    self.probe = Some(p);
                }
                Err(e) => {
                    if !self.probe_error_logged {
                        warn!(error = %format!("{e:#}"), "cannot open capture node for monitoring");
                        self.probe_error_logged = true;
                    }
                }
            }
        }
        self.probe.clone()
    }

    /// Replugs the source when the receiver is stuck relocking.
    async fn check_relock(&mut self) {
        let Some(counter) = &self.relock_events else { return };
        let events = counter.swap(0, Ordering::Relaxed);
        if events > 0 {
            info!(events, "HDMI receiver reported a source change");
        }
        match self.relock.observe(std::time::Instant::now(), events) {
            RelockVerdict::Quiet => {}
            RelockVerdict::Storm { events } => {
                debug!(events, "HDMI receiver still relocking; waiting before the next replug");
            }
            RelockVerdict::Replug { events, attempt } => {
                warn!(events, attempt, "HDMI receiver is stuck relocking, replugging the source");
                match tokio::task::spawn_blocking(|| pulse_hotplug(REPLUG_LOW)).await {
                    Ok(Ok(())) => info!("software replug done"),
                    Ok(Err(e)) => warn!(error = %format!("{e:#}"), "software replug failed"),
                    Err(e) => warn!(error = %e, "software replug task failed"),
                }
                // The signal drops while HPD is low; don't count that as a stall.
                self.last_progress = Instant::now();
            }
        }
    }

    /// Returns why the running pipeline must be rebuilt, if it must.
    async fn check(&mut self, device: &str) -> Option<String> {
        let (frames, caps) = {
            let guard = PIPELINE_MANAGER.lock().await;
            let m = guard.as_ref()?;
            (m.raw_frames(), m.capture_caps())
        };
        if !self.caps_logged
            && let Some(caps) = caps
        {
            info!(%caps, "capture caps negotiated");
            if !caps.contains("NV12") {
                warn!("capture source is not NV12; frames are converted in software");
            }
            self.caps_logged = true;
        }

        let probe = self.probe(device);
        self.check_relock().await;

        if frames != self.last_frames {
            self.last_frames = frames;
            self.last_progress = Instant::now();
            self.stall_restarts = 0;
            return None;
        }
        let signal = match probe {
            Some(p) => tokio::task::spawn_blocking(move || p.query()).await.ok(),
            None => None,
        };
        // Without a signal there is nothing to capture; wait for it to return.
        if signal.is_some_and(|s| s.timings.is_none()) {
            self.last_progress = Instant::now();
            return None;
        }
        let limit = STALL_LIMIT.max(restart_backoff(self.stall_restarts));
        let stalled = self.last_progress.elapsed();
        if stalled >= limit {
            self.stall_restarts = self.stall_restarts.saturating_add(1);
            let source = signal.map_or_else(|| "unknown".to_owned(), |s| s.to_string());
            return Some(format!("no raw frame for {}s, source {source}", stalled.as_secs()));
        }
        None
    }
}

/// Counts the receiver's source-change events on a thread of its own: the
/// kernel keeps only the newest pending event, so a 2 s supervisor tick
/// would see at most one of a once-a-second storm.
fn spawn_relock_counter(probe: Arc<CaptureProbe>) -> Arc<AtomicU32> {
    let count = Arc::new(AtomicU32::new(0));
    let out = count.clone();
    let spawned = std::thread::Builder::new().name("hdmirx-events".into()).spawn(move || {
        while !SUPERVISOR_STOPPED.load(Ordering::Relaxed) {
            let n = probe.take_source_changes();
            if n > 0 {
                count.fetch_add(n, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    if let Err(e) = spawned {
        warn!(error = %e, "cannot start the HDMI event thread");
    }
    out
}

/// Keeps the capture pipeline healthy. It rebuilds the pipeline when its bus
/// loop has died (GStreamer error, EOS, or a failed start at boot) or when
/// capture stalls while a signal is present, and replugs the HDMI source in
/// software when the receiver is stuck in a relock loop. Failed restarts back
/// off exponentially.
pub fn spawn_video_supervisor() {
    tokio::spawn(async move {
        let mut tick = interval(SUPERVISOR_PERIOD);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut failures = 0u32;
        let mut live = LiveCheck::new();
        loop {
            tick.tick().await;
            if SUPERVISOR_STOPPED.load(Ordering::SeqCst) {
                return;
            }
            let Some((video_config, _)) = PIPELINE_CONFIG.get() else { continue };
            if is_video_pipeline_running().await {
                failures = 0;
                let Some(reason) = live.check(&video_config.device).await else { continue };
                warn!(%reason, "video capture stalled, rebuilding the pipeline");
            } else {
                warn!(failures, "video pipeline is down, restarting");
            }
            live.pipeline_replaced();
            match restart_video_pipeline().await {
                Ok(()) => failures = 0,
                Err(e) => {
                    let delay = restart_backoff(failures);
                    failures = failures.saturating_add(1);
                    warn!(error = %format!("{e:#}"), retry_in_secs = delay.as_secs(), "video pipeline restart failed");
                    tokio::time::sleep(delay).await;
                }
            }
        }
    });
}

pub async fn shutdown_video_pipeline() {
    info!("Shutting down video pipeline...");

    SUPERVISOR_STOPPED.store(true, Ordering::SeqCst);
    stop_pipeline().await;

    *VIDEO_SINK.write().await = None;

    info!("Video pipeline shutdown complete");
}

pub async fn capture_screenshot_jpeg() -> anyhow::Result<bytes::Bytes> {
    let rx = PIPELINE_MANAGER
        .lock()
        .await
        .as_ref()
        .map(PipelineManager::request_raw_frame)
        .ok_or_else(|| anyhow::anyhow!("Pipeline not initialized"))?;
    let sample = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .map_err(|_| anyhow::anyhow!("No video frame within 2s"))?
        .map_err(|_| anyhow::anyhow!("Video pipeline stopped before a frame arrived"))?;
    tokio::task::spawn_blocking(move || crate::pipeline::encode_jpeg(&sample))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
}

pub async fn force_video_keyframe() {
    let manager = PIPELINE_MANAGER.lock().await;
    if let Some(manager) = manager.as_ref() {
        manager.force_keyframe();
    }
}

/// Uncoalesced keyframe request for a viewer whose connection just came up.
pub async fn force_new_viewer_keyframe() {
    if let Some(manager) = PIPELINE_MANAGER.lock().await.as_ref() {
        manager.force_keyframe_now();
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
    use crate::hardware::native::socket::{call_ctrl_action, ctrl_client_connected};
    if !ctrl_client_connected() {
        debug!(action, "no native ctrl client attached; skipping control action");
        return Ok(());
    }
    info!("Writing control action: {}", serde_json::json!({ "action": action }));
    let _ = call_ctrl_action(action, None).await?;
    Ok(())
}
