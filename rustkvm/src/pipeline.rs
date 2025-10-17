// GStreamer pipeline implementation for RustKVM
// Handles video and audio capture with hardware acceleration

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use tracing::{debug, error, info, warn};
use {gstreamer as gst, gstreamer_app as gst_app, gstreamer_video as gst_video};

use crate::video::VideoInputState;

/// Video pipeline configuration
#[derive(Debug, Clone)]
pub struct VideoConfig {
    pub device: String,
    pub bitrate: u32,
    pub max_bitrate: u32,
    pub gop_size: Option<i32>, // None = auto, -1 = FPS, positive = specific GOP
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            device: "/dev/video0".to_string(),
            bitrate: 20_000_000,
            max_bitrate: 60_000_000,
            gop_size: None,
        }
    }
}

impl VideoConfig {
    /// Create a new configuration with default values
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set device path (builder pattern)
    #[inline]
    pub fn with_device(mut self, device: impl Into<String>) -> Self {
        self.device = device.into();
        self
    }

    /// Set bitrate (builder pattern)
    #[inline]
    pub fn with_bitrate(mut self, bitrate: u32, max_bitrate: u32) -> Self {
        self.bitrate = bitrate;
        self.max_bitrate = max_bitrate;
        self
    }

    /// Set GOP size (builder pattern)
    ///
    /// * `-1` = use FPS as GOP (default behavior)
    /// * positive value = specific GOP size
    #[inline]
    pub fn with_gop_size(mut self, gop: i32) -> Self {
        self.gop_size = Some(gop);
        self
    }

    /// Adjust bitrate by quality factor (builder pattern)
    pub fn with_quality(mut self, quality: f32) -> Self {
        let q = quality.clamp(0.1, 2.0);
        self.bitrate = (20_000_000.0 * q) as u32;
        self.max_bitrate = (60_000_000.0 * q) as u32;
        self
    }
}

/// Audio pipeline configuration
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub bitrate: i32, // Opus bitrate range: 4000-650000
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { device: "hw:0,0".to_string(), sample_rate: 48000, channels: 2, bitrate: 64000 }
    }
}

impl AudioConfig {
    /// Create a new configuration with default values
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set device path (builder pattern)
    #[inline]
    pub fn with_device(mut self, device: impl Into<String>) -> Self {
        self.device = device.into();
        self
    }

    /// Set sample rate (builder pattern)
    #[inline]
    pub fn with_sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// Set channels (builder pattern)
    #[inline]
    pub fn with_channels(mut self, channels: u32) -> Self {
        self.channels = channels;
        self
    }

    /// Set bitrate (builder pattern)
    ///
    /// Valid range for Opus: 4000-650000 bps
    #[inline]
    pub fn with_bitrate(mut self, bitrate: i32) -> Self {
        self.bitrate = bitrate;
        self
    }
}

/// Video pipeline with H.264 encoding
pub struct VideoPipeline {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    encoder: gst::Element,
    running: Arc<AtomicBool>,
}

impl VideoPipeline {
    /// Create new video pipeline
    pub fn new(config: VideoConfig) -> Result<Self> {
        // Validate configuration
        if config.bitrate == 0 || config.max_bitrate == 0 {
            anyhow::bail!("Bitrate must be non-zero");
        }
        if config.max_bitrate < config.bitrate {
            anyhow::bail!("Max bitrate must be >= bitrate");
        }
        if let Some(gop) = config.gop_size
            && gop == 0
        {
            anyhow::bail!("GOP size must be non-zero (-1 for FPS, positive for specific value)");
        }
        let pipeline = gst::Pipeline::new();

        // Create elements
        let v4l2src = gst::ElementFactory::make("v4l2src")
            .property("device", &config.device)
            .build()
            .context("Failed to create v4l2src")?;

        // Video converter (will handle any format -> NV12)
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .context("Failed to create videoconvert")?;

        // NV12 caps filter (force NV12 for encoder, but preserve source resolution/fps)
        let nv12_caps = gst::ElementFactory::make("capsfilter")
            .name("nv12_capsfilter")
            .build()
            .context("Failed to create nv12 capsfilter")?;

        // Only specify format, let resolution and framerate pass through from source
        let nv12_caps_spec =
            gst_video::VideoCapsBuilder::new().format(gst_video::VideoFormat::Nv12).build();
        nv12_caps.set_property("caps", &nv12_caps_spec);

        // Queue for buffering (minimal for low latency)
        let queue = gst::ElementFactory::make("queue")
            .property("max-size-buffers", 2u32)
            .property_from_str("leaky", "downstream") // Drop old frames if queue is full
            .build()
            .context("Failed to create queue")?;

        // H.264 encoder (Rockchip MPP)
        // GOP will be set dynamically based on source framerate if not specified
        let encoder = gst::ElementFactory::make("mpph264enc")
            .name("mpph264enc") // Named for dynamic access
            .property_from_str("rc-mode", "vbr")
            .property_from_str("level", "5.2")
            .property("bps", config.bitrate)
            .property("bps-max", config.max_bitrate)
            .build()
            .context("Failed to create mpph264enc")?;

        // Set GOP if specified, otherwise it will use encoder default or be set dynamically
        if let Some(gop) = config.gop_size {
            encoder.set_property("gop", gop);
        }

        // H.264 parser
        let h264parse = gst::ElementFactory::make("h264parse")
            .property("config-interval", -1i32)
            .property("disable-passthrough", true)
            .build()
            .context("Failed to create h264parse")?;

        // AppSink to receive encoded data
        let appsink = gst_app::AppSink::builder()
            .name("video_appsink")
            .sync(false)
            .buffer_list(false)
            .max_buffers(2)
            .drop(true)
            .build();

        // Add all elements to pipeline
        pipeline
            .add_many([
                &v4l2src,
                &videoconvert,
                &nv12_caps,
                &queue,
                &encoder,
                &h264parse,
                appsink.upcast_ref(),
            ])
            .context("Failed to add elements to pipeline")?;

        // Link elements (no src_caps, direct from v4l2src)
        gst::Element::link_many([
            &v4l2src,
            &videoconvert,
            &nv12_caps,
            &queue,
            &encoder,
            &h264parse,
            appsink.upcast_ref(),
        ])
        .context("Failed to link video pipeline elements")?;

        let running = Arc::new(AtomicBool::new(false));
        let encoder_for_state = encoder.clone();

        // Setup dynamic GOP adjustment based on source framerate
        if config.gop_size.is_none() {
            let encoder_clone = encoder.clone();
            let pipeline_weak = pipeline.downgrade();

            // Monitor state changes to set GOP after caps negotiation
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                // GStreamer operations must run in blocking context
                tokio::task::spawn_blocking(move || {
                    if let Some(_pipeline) = pipeline_weak.upgrade()
                        && let Some(src_pad) = encoder_clone.static_pad("sink")
                        && let Some(caps) = src_pad.current_caps()
                        && let Ok(info) = gst_video::VideoInfo::from_caps(&caps)
                    {
                        let fps = info.fps();
                        let gop = fps.numer() / fps.denom().max(1);
                        encoder_clone.set_property("gop", gop);
                        info!(
                            "Set dynamic GOP to {} based on source {:.1}fps",
                            gop,
                            fps.numer() as f64 / fps.denom() as f64
                        );
                    }
                })
                .await
                .ok();
            });
        }

        Ok(Self { pipeline, appsink, encoder: encoder_for_state, running })
    }

    /// Start the pipeline
    pub fn start(&self) -> Result<()> {
        info!("Setting video pipeline to PLAYING state...");

        let state_result = self.pipeline.set_state(gst::State::Playing);
        match state_result {
            Ok(_) => info!("Video pipeline state change to PLAYING succeeded"),
            Err(e) => {
                error!("Video pipeline state change failed: {:?}", e);
                anyhow::bail!("Failed to set pipeline to PLAYING state");
            }
        }

        self.running.store(true, Ordering::SeqCst);
        info!("Video pipeline started, appsink should start receiving frames");
        Ok(())
    }

    /// Stop the pipeline
    pub fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        self.pipeline
            .set_state(gst::State::Null)
            .context("Failed to set pipeline to NULL state")?;

        info!("Video pipeline stopped");
        Ok(())
    }

    /// Set callback to receive encoded H.264 frames
    pub fn set_frame_callback<F>(&self, mut callback: F)
    where
        F: FnMut(Vec<u8>, u64) + Send + 'static,
    {
        tracing::info!("Setting video frame callback on appsink");

        // Use std::time in GStreamer callback context (not tokio runtime)
        let start_time = std::time::Instant::now();
        let first_frame = Arc::new(AtomicBool::new(false));

        self.appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    tracing::debug!("Video appsink new_sample callback triggered");
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;

                    // Log first frame
                    if !first_frame.swap(true, Ordering::Relaxed) {
                        tracing::info!("First video frame received from GStreamer");
                    }

                    // Map buffer for reading
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let data = map.as_slice().to_vec();

                    // Calculate PTS in microseconds
                    let pts_us = if let Some(pts) = buffer.pts() {
                        pts.nseconds() / 1000
                    } else {
                        start_time.elapsed().as_micros() as u64
                    };

                    // Call user callback
                    callback(data, pts_us);

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
    }

    /// Set callback to receive video state updates
    ///
    /// Uses active polling strategy similar to GOP adjustment for reliability
    pub fn set_state_callback<F>(&self, callback: F)
    where
        F: FnMut(VideoInputState) + Send + 'static,
    {
        let bus = self.pipeline.bus().expect("Pipeline should have a bus");
        let running = self.running.clone();
        let encoder = self.encoder.clone();
        let callback = Arc::new(std::sync::Mutex::new(callback));

        // Strategy: Use both active polling AND bus monitoring for reliability

        // 1. Active polling task (same as GOP adjustment)
        let encoder_poll = encoder.clone();
        let callback_poll = callback.clone();
        tokio::spawn(async move {
            // Wait for caps negotiation (same delay as GOP)
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            tokio::task::spawn_blocking(move || {
                if let Some(pad) = encoder_poll.static_pad("sink")
                    && let Some(caps) = pad.current_caps()
                    && let Ok(info) = gst_video::VideoInfo::from_caps(&caps)
                {
                    let state = VideoInputState {
                        ready: true,
                        error: None,
                        width: info.width(),
                        height: info.height(),
                        frame_per_second: {
                            let fps = info.fps();
                            fps.numer() as f64 / fps.denom() as f64
                        },
                    };
                    info!(
                        "Video state extracted (active poll): {}x{} @ {:.1}fps",
                        state.width, state.height, state.frame_per_second
                    );
                    if let Ok(mut cb) = callback_poll.lock() {
                        cb(state);
                    }
                }
            })
            .await
            .ok();
        });

        // 2. Bus monitoring for errors
        tokio::task::spawn_blocking(move || {
            while running.load(Ordering::SeqCst) {
                if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
                    match msg.view() {
                        gst::MessageView::Error(err) => {
                            error!(
                                "GStreamer error from {:?}: {} ({:?})",
                                err.src().map(|s| s.path_string()),
                                err.error(),
                                err.debug()
                            );
                            if let Ok(mut cb) = callback.lock() {
                                cb(VideoInputState {
                                    ready: false,
                                    error: Some(format!("{}", err.error())),
                                    width: 0,
                                    height: 0,
                                    frame_per_second: 0.0,
                                });
                            }
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                        gst::MessageView::Warning(w) => {
                            warn!(
                                "GStreamer warning from {:?}: {}",
                                w.src().map(|s| s.path_string()),
                                w.error()
                            );
                        }
                        gst::MessageView::Eos(_) => {
                            info!("GStreamer pipeline EOS");
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        });
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Audio pipeline with Opus encoding
pub struct AudioPipeline {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    running: Arc<AtomicBool>,
}

impl AudioPipeline {
    /// Create new audio pipeline
    pub fn new(config: AudioConfig) -> Result<Self> {
        // Validate configuration
        if config.sample_rate == 0 {
            anyhow::bail!("Sample rate must be non-zero");
        }
        if config.channels == 0 {
            anyhow::bail!("Channels must be non-zero");
        }
        if config.bitrate < 4000 || config.bitrate > 650000 {
            anyhow::bail!("Opus bitrate must be in range 4000-650000");
        }
        let pipeline = gst::Pipeline::new();

        // Create elements
        let alsasrc = gst::ElementFactory::make("alsasrc")
            .property("device", &config.device)
            .property("buffer-time", 20000i64) // 20ms buffer
            .property("latency-time", 10000i64) // 10ms latency
            .build()
            .context("Failed to create alsasrc")?;

        // Audio converter
        let audioconvert = gst::ElementFactory::make("audioconvert")
            .build()
            .context("Failed to create audioconvert")?;

        // Audio resampler
        let audioresample = gst::ElementFactory::make("audioresample")
            .build()
            .context("Failed to create audioresample")?;

        // Queue for buffering (small for low latency)
        let queue = gst::ElementFactory::make("queue")
            .property("max-size-buffers", 2u32)
            .property("max-size-bytes", 0u32)
            .property("max-size-time", 0u64)
            .property_from_str("leaky", "downstream") // Drop old buffers under backpressure
            .build()
            .context("Failed to create audio queue")?;

        // Opus encoder (optimized for low latency)
        let opusenc = gst::ElementFactory::make("opusenc")
            .property("bitrate", config.bitrate) // i32: 4000-650000
            .property_from_str("frame-size", "10") // Enum: 10ms frames for low latency
            .property("complexity", 4i32) // i32: 0-10, balance between quality and CPU
            .property("inband-fec", true) // Boolean: forward error correction
            .property("dtx", false) // Boolean: disable DTX for consistent latency
            .property_from_str("audio-type", "restricted-lowdelay") // Enum: low-latency mode
            .build()
            .context("Failed to create opusenc")?;

        // AppSink to receive encoded data
        let appsink = gst_app::AppSink::builder()
            .name("audio_appsink")
            .sync(false)
            .buffer_list(false)
            .max_buffers(1)
            .drop(true)
            .build();

        // Add all elements to pipeline
        pipeline
            .add_many([
                &alsasrc,
                &audioconvert,
                &audioresample,
                &queue,
                &opusenc,
                appsink.upcast_ref(),
            ])
            .context("Failed to add elements to audio pipeline")?;

        // Link elements
        gst::Element::link_many([
            &alsasrc,
            &audioconvert,
            &audioresample,
            &queue,
            &opusenc,
            appsink.upcast_ref(),
        ])
        .context("Failed to link audio pipeline elements")?;

        let running = Arc::new(AtomicBool::new(false));

        Ok(Self { pipeline, appsink, running })
    }

    /// Start the pipeline
    pub fn start(&self) -> Result<()> {
        self.pipeline
            .set_state(gst::State::Playing)
            .context("Failed to set audio pipeline to PLAYING state")?;

        self.running.store(true, Ordering::SeqCst);
        info!("Audio pipeline started");

        // Monitor bus for errors/hotplug issues to avoid buffer buildup
        let running = self.running.clone();
        let bus = self.pipeline.bus().expect("Audio pipeline should have a bus");
        let pipeline_for_bus = self.pipeline.clone();
        let recovering = Arc::new(AtomicBool::new(false));
        let warn_count = Arc::new(AtomicU32::new(0));
        let last_warn_reset = Arc::new(AtomicU64::new(0));
        let rt_handle = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            while running.load(Ordering::SeqCst) {
                if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
                    match msg.view() {
                        gst::MessageView::Error(err) => {
                            error!(
                                "GStreamer audio error from {:?}: {} ({:?})",
                                err.src().map(|s| s.path_string()),
                                err.error(),
                                err.debug()
                            );
                            if !recovering.swap(true, Ordering::SeqCst) {
                                let _ = pipeline_for_bus.set_state(gst::State::Null);
                                let running_retry = running.clone();
                                let pipeline_retry = pipeline_for_bus.clone();
                                let recovering_flag = recovering.clone();
                                rt_handle.spawn(async move {
                                    while running_retry.load(Ordering::SeqCst) {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                                        match pipeline_retry.set_state(gst::State::Playing) {
                                            Ok(_) => {
                                                info!("Audio pipeline recovered to PLAYING after error");
                                                recovering_flag.store(false, Ordering::SeqCst);
                                                break;
                                            }
                                            Err(e) => warn!("Audio restart failed, will retry: {:?}", e),
                                        }
                                    }
                                });
                            }
                        }
                        gst::MessageView::Warning(w) => {
                            debug!(
                                "GStreamer audio warning from {:?}: {}",
                                w.src().map(|s| s.path_string()),
                                w.error()
                            );
                            // Fast path: check if it's alsasrc underrun
                            if let Some(src) = w.src() {
                                let src_path = src.path_string();
                                if src_path.contains("GstAlsaSrc") {
                                    let err_str = format!("{}", w.error());
                                    if err_str.contains("Can't record audio fast enough") {
                                        let now_ms = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis()
                                            as u64;

                                        // Reset counter every 2 seconds
                                        let last_reset = last_warn_reset.load(Ordering::SeqCst);
                                        if now_ms - last_reset > 2000 {
                                            warn_count.store(0, Ordering::SeqCst);
                                            last_warn_reset.store(now_ms, Ordering::SeqCst);
                                        }

                                        let count = warn_count.fetch_add(1, Ordering::SeqCst) + 1;
                                        if count > 10 && !recovering.load(Ordering::SeqCst) {
                                            warn!(
                                                "Audio underrun detected ({} warnings), triggering recovery",
                                                count
                                            );
                                            recovering.store(true, Ordering::SeqCst);
                                            let _ = pipeline_for_bus.set_state(gst::State::Null);
                                            let running_retry = running.clone();
                                            let pipeline_retry = pipeline_for_bus.clone();
                                            let recovering_flag = recovering.clone();
                                            rt_handle.spawn(async move {
                                                while running_retry.load(Ordering::SeqCst) {
                                                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                                                    if pipeline_retry.set_state(gst::State::Playing).is_ok() {
                                                        info!("Audio pipeline recovered to PLAYING after underrun warnings");
                                                        recovering_flag.store(false, Ordering::SeqCst);
                                                        break;
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        gst::MessageView::Eos(_) => {
                            info!("GStreamer audio pipeline EOS");
                            if !recovering.swap(true, Ordering::SeqCst) {
                                let _ = pipeline_for_bus.set_state(gst::State::Null);
                                let running_retry = running.clone();
                                let pipeline_retry = pipeline_for_bus.clone();
                                let recovering_flag = recovering.clone();
                                rt_handle.spawn(async move {
                                    while running_retry.load(Ordering::SeqCst) {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(
                                            1000,
                                        ))
                                        .await;
                                        if pipeline_retry.set_state(gst::State::Playing).is_ok() {
                                            info!("Audio pipeline recovered to PLAYING after EOS");
                                            recovering_flag.store(false, Ordering::SeqCst);
                                            break;
                                        }
                                    }
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
        Ok(())
    }

    /// Stop the pipeline
    pub fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        self.pipeline
            .set_state(gst::State::Null)
            .context("Failed to set audio pipeline to NULL state")?;

        info!("Audio pipeline stopped");
        Ok(())
    }

    /// Set callback to receive encoded Opus audio
    pub fn set_callback<F>(&self, mut callback: F)
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let first_frame = Arc::new(AtomicBool::new(false));

        self.appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;

                    // Log first frame
                    if !first_frame.swap(true, Ordering::Relaxed) {
                        tracing::info!("First audio frame received from GStreamer");
                    }

                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let data = map.as_slice().to_vec();

                    callback(data);

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Unified pipeline manager for video and audio
pub struct PipelineManager {
    video: VideoPipeline,
    audio: Option<AudioPipeline>,
}

impl PipelineManager {
    /// Create new pipeline manager
    pub fn new(video_config: VideoConfig, audio_config: Option<AudioConfig>) -> Result<Self> {
        // Initialize GStreamer
        gst::init().context("Failed to initialize GStreamer")?;

        let video = VideoPipeline::new(video_config)?;
        let audio = audio_config.map(AudioPipeline::new).transpose()?;

        Ok(Self { video, audio })
    }

    /// Start all pipelines
    pub fn start(&self) -> Result<()> {
        self.video.start()?;
        if let Some(audio) = &self.audio {
            audio.start()?;
        }
        Ok(())
    }

    /// Stop all pipelines
    pub fn stop(&self) -> Result<()> {
        self.video.stop()?;
        if let Some(audio) = &self.audio {
            audio.stop()?;
        }
        Ok(())
    }

    /// Set video frame callback
    pub fn set_video_callback<F>(&self, callback: F)
    where
        F: FnMut(Vec<u8>, u64) + Send + 'static,
    {
        self.video.set_frame_callback(callback);
    }

    /// Set video state callback
    pub fn set_state_callback<F>(&self, callback: F)
    where
        F: FnMut(VideoInputState) + Send + 'static,
    {
        self.video.set_state_callback(callback);
    }

    /// Set audio callback
    pub fn set_audio_callback<F>(&self, callback: F)
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        if let Some(audio) = &self.audio {
            audio.set_callback(callback);
        }
    }

    /// Dynamically update video bitrate
    pub fn set_video_bitrate(&self, bitrate: u32, max_bitrate: u32) -> Result<()> {
        if let Some(encoder) = self.video.pipeline.by_name("mpph264enc") {
            encoder.set_property("bps", bitrate);
            encoder.set_property("bps-max", max_bitrate);
            Ok(())
        } else {
            anyhow::bail!("Video encoder not found in pipeline")
        }
    }
}
