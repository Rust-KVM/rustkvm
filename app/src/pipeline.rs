use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use tracing::{debug, error, info, warn};

use crate::video::VideoInputState;

#[derive(Debug, Clone)]
pub struct VideoConfig {
    pub device: String,
    pub encoder: crate::cli::VideoEncoder,
    pub bitrate: u32,
    pub max_bitrate: u32,
    pub gop_size: Option<i32>,
    pub rc_mode: crate::cli::RateControlMode,
    pub level: String,
    pub profile: Option<String>,
    pub fps: u32,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            device: "/dev/video0".to_string(),
            encoder: crate::cli::VideoEncoder::default(),
            bitrate: 20_000_000,
            max_bitrate: 60_000_000,
            gop_size: None,
            rc_mode: crate::cli::RateControlMode::default(),
            level: "5.2".to_string(),
            profile: None,
            fps: 60,
        }
    }
}

impl VideoConfig {
    pub fn from_cli(args: &crate::cli::VideoArgs, quality: f32) -> Self {
        let q = quality.clamp(0.1, 2.0);
        Self {
            device: args.video_device.clone(),
            encoder: args.video_encoder,
            bitrate: (args.video_bitrate as f32 * q) as u32,
            max_bitrate: (args.video_max_bitrate as f32 * q) as u32,
            gop_size: args.video_gop,
            rc_mode: args.video_rc_mode,
            level: args.video_level.clone(),
            profile: args.video_profile.clone(),
            fps: args.video_fps.max(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub bitrate: i32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { device: "hw:0,0".to_string(), sample_rate: 48000, channels: 2, bitrate: 64000 }
    }
}

impl AudioConfig {
    pub fn from_cli(args: &crate::cli::AudioArgs) -> Self {
        Self {
            device: args.audio_device.clone(),
            sample_rate: args.audio_sample_rate,
            channels: args.audio_channels,
            bitrate: args.audio_bitrate,
        }
    }
}

pub struct VideoPipeline {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    encoder: gst::Element,
    running: Arc<AtomicBool>,
}

impl VideoPipeline {
    pub fn new(config: VideoConfig) -> Result<Self> {
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

        let v4l2src = gst::ElementFactory::make("v4l2src")
            .property("device", &config.device)
            .build()
            .context("Failed to create v4l2src")?;

        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .context("Failed to create videoconvert")?;

        let nv12_caps = gst::ElementFactory::make("capsfilter")
            .name("nv12_capsfilter")
            .build()
            .context("Failed to create nv12 capsfilter")?;

        let nv12_caps_spec =
            gst_video::VideoCapsBuilder::new().format(gst_video::VideoFormat::Nv12).build();
        nv12_caps.set_property("caps", &nv12_caps_spec);

        let videorate = gst::ElementFactory::make("videorate")
            .property("drop-only", true)
            .build()
            .context("Failed to create videorate")?;

        let rate_caps = gst::ElementFactory::make("capsfilter")
            .name("rate_capsfilter")
            .build()
            .context("Failed to create framerate capsfilter")?;
        let rate_caps_spec = gst_video::VideoCapsBuilder::new()
            .framerate(gst::Fraction::new(config.fps as i32, 1))
            .build();
        rate_caps.set_property("caps", &rate_caps_spec);

        let queue = gst::ElementFactory::make("queue")
            .property("max-size-buffers", 2u32)
            .property_from_str("leaky", "downstream")
            .build()
            .context("Failed to create queue")?;

        let encoder_name = config.encoder.gst_element_name();
        info!("Creating {} encoder for {}", encoder_name, config.encoder.codec_name());

        let mut encoder_builder = gst::ElementFactory::make(encoder_name)
            .name(encoder_name)
            .property_from_str("rc-mode", config.rc_mode.as_gst_str())
            .property_from_str("level", &config.level)
            .property("bps", config.bitrate)
            .property("bps-max", config.max_bitrate);

        if let Some(ref profile) = config.profile {
            encoder_builder = encoder_builder.property_from_str("profile", profile);
        }

        let encoder = encoder_builder
            .build()
            .with_context(|| format!("Failed to create {}", encoder_name))?;

        if let Some(gop) = config.gop_size {
            encoder.set_property("gop", gop);
        }

        let parser_name = config.encoder.parser_element_name();
        let parser = gst::ElementFactory::make(parser_name)
            .property("config-interval", -1i32)
            .property("disable-passthrough", true)
            .build()
            .with_context(|| format!("Failed to create {}", parser_name))?;

        let appsink = gst_app::AppSink::builder()
            .name("video_appsink")
            .sync(false)
            .buffer_list(false)
            .max_buffers(2)
            .drop(true)
            .build();

        pipeline
            .add_many([
                &v4l2src,
                &videoconvert,
                &nv12_caps,
                &videorate,
                &rate_caps,
                &queue,
                &encoder,
                &parser,
                appsink.upcast_ref(),
            ])
            .context("Failed to add elements to pipeline")?;

        gst::Element::link_many([
            &v4l2src,
            &videoconvert,
            &nv12_caps,
            &videorate,
            &rate_caps,
            &queue,
            &encoder,
            &parser,
            appsink.upcast_ref(),
        ])
        .context("Failed to link video pipeline elements")?;

        let running = Arc::new(AtomicBool::new(false));
        let encoder_for_state = encoder.clone();

        if config.gop_size.is_none() {
            let encoder_clone = encoder;
            let pipeline_weak = pipeline.downgrade();

            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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

    pub fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        self.pipeline
            .set_state(gst::State::Null)
            .context("Failed to set pipeline to NULL state")?;

        info!("Video pipeline stopped");
        Ok(())
    }

    pub fn set_frame_callback<F>(&self, mut callback: F)
    where
        F: FnMut(bytes::Bytes, u64) + Send + 'static,
    {
        tracing::info!("Setting video frame callback on appsink");

        let start_time = std::time::Instant::now();
        let first_frame = Arc::new(AtomicBool::new(false));

        self.appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    tracing::debug!("Video appsink new_sample callback triggered");
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer_owned().ok_or(gst::FlowError::Error)?;

                    if !first_frame.swap(true, Ordering::Relaxed) {
                        tracing::info!("First video frame received from GStreamer");
                    }

                    let pts_us = if let Some(pts) = buffer.pts() {
                        pts.nseconds() / 1000
                    } else {
                        start_time.elapsed().as_micros() as u64
                    };

                    let mapped =
                        buffer.into_mapped_buffer_readable().map_err(|_| gst::FlowError::Error)?;
                    let bytes = bytes::Bytes::from_owner(mapped);

                    crate::observability::metrics::VIDEO_FRAMES_TOTAL.inc();
                    callback(bytes, pts_us);

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
    }

    pub fn set_state_callback<F>(&self, callback: F)
    where
        F: FnMut(VideoInputState) + Send + 'static,
    {
        let bus = self.pipeline.bus().expect("Pipeline should have a bus");
        let running = self.running.clone();
        let encoder = self.encoder.clone();
        let callback = Arc::new(parking_lot::Mutex::new(callback));

        let encoder_poll = encoder;
        let callback_poll = callback.clone();
        tokio::spawn(async move {
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
                    debug!(
                        "Video state extracted (active poll): {}x{} @ {:.1}fps",
                        state.width, state.height, state.frame_per_second
                    );
                    let mut cb = callback_poll.lock();
                    cb(state);
                }
            })
            .await
            .ok();
        });

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
                            let mut cb = callback.lock();
                            cb(VideoInputState {
                                ready: false,
                                error: Some(format!("{}", err.error())),
                                width: 0,
                                height: 0,
                                frame_per_second: 0.0,
                            });
                            drop(cb);
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

pub struct AudioPipeline {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    running: Arc<AtomicBool>,
}

impl AudioPipeline {
    pub fn new(config: AudioConfig) -> Result<Self> {
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

        let alsasrc = gst::ElementFactory::make("alsasrc")
            .property("device", &config.device)
            .property("buffer-time", 20000i64)
            .property("latency-time", 10000i64)
            .build()
            .context("Failed to create alsasrc")?;

        let audioconvert = gst::ElementFactory::make("audioconvert")
            .build()
            .context("Failed to create audioconvert")?;

        let audioresample = gst::ElementFactory::make("audioresample")
            .build()
            .context("Failed to create audioresample")?;

        let queue = gst::ElementFactory::make("queue")
            .property("max-size-buffers", 2u32)
            .property("max-size-bytes", 0u32)
            .property("max-size-time", 0u64)
            .property_from_str("leaky", "downstream")
            .build()
            .context("Failed to create audio queue")?;

        let opusenc = gst::ElementFactory::make("opusenc")
            .property("bitrate", config.bitrate)
            .property_from_str("frame-size", "10")
            .property("complexity", 4i32)
            .property("inband-fec", true)
            .property("dtx", false)
            .property_from_str("audio-type", "restricted-lowdelay")
            .build()
            .context("Failed to create opusenc")?;

        let appsink = gst_app::AppSink::builder()
            .name("audio_appsink")
            .sync(false)
            .buffer_list(false)
            .max_buffers(1)
            .drop(true)
            .build();

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

    pub fn start(&self) -> Result<()> {
        self.pipeline
            .set_state(gst::State::Playing)
            .context("Failed to set audio pipeline to PLAYING state")?;

        self.running.store(true, Ordering::SeqCst);
        info!("Audio pipeline started");

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

    pub fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        self.pipeline
            .set_state(gst::State::Null)
            .context("Failed to set audio pipeline to NULL state")?;

        info!("Audio pipeline stopped");
        Ok(())
    }

    pub fn set_callback<F>(&self, mut callback: F)
    where
        F: FnMut(bytes::Bytes) + Send + 'static,
    {
        let first_frame = Arc::new(AtomicBool::new(false));

        self.appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer_owned().ok_or(gst::FlowError::Error)?;

                    if !first_frame.swap(true, Ordering::Relaxed) {
                        tracing::info!("First audio frame received from GStreamer");
                    }

                    let mapped =
                        buffer.into_mapped_buffer_readable().map_err(|_| gst::FlowError::Error)?;
                    let bytes = bytes::Bytes::from_owner(mapped);

                    crate::observability::metrics::AUDIO_FRAMES_TOTAL.inc();
                    callback(bytes);

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

pub struct PipelineManager {
    video: VideoPipeline,
    audio: Option<AudioPipeline>,
}

impl PipelineManager {
    pub fn new(video_config: VideoConfig, audio_config: Option<AudioConfig>) -> Result<Self> {
        gst::init().context("Failed to initialize GStreamer")?;

        let video = VideoPipeline::new(video_config)?;
        let audio = audio_config.map(AudioPipeline::new).transpose()?;

        Ok(Self { video, audio })
    }

    pub fn start(&self) -> Result<()> {
        self.video.start()?;
        if let Some(audio) = &self.audio {
            audio.start()?;
        }
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.video.stop()?;
        if let Some(audio) = &self.audio {
            audio.stop()?;
        }
        Ok(())
    }

    pub fn set_video_callback<F>(&self, callback: F)
    where
        F: FnMut(bytes::Bytes, u64) + Send + 'static,
    {
        self.video.set_frame_callback(callback);
    }

    pub fn set_state_callback<F>(&self, callback: F)
    where
        F: FnMut(VideoInputState) + Send + 'static,
    {
        self.video.set_state_callback(callback);
    }

    pub fn set_audio_callback<F>(&self, callback: F)
    where
        F: FnMut(bytes::Bytes) + Send + 'static,
    {
        if let Some(audio) = &self.audio {
            audio.set_callback(callback);
        }
    }

    pub fn set_video_bitrate(&self, bitrate: u32, max_bitrate: u32) -> Result<()> {
        if let Some(encoder) = self
            .video
            .pipeline
            .by_name("mpph264enc")
            .or_else(|| self.video.pipeline.by_name("mpph265enc"))
        {
            encoder.set_property("bps", bitrate);
            encoder.set_property("bps-max", max_bitrate);
            Ok(())
        } else {
            anyhow::bail!("Video encoder not found in pipeline")
        }
    }

    pub fn force_keyframe(&self) {
        let event = gst_video::UpstreamForceKeyUnitEvent::builder().all_headers(true).build();
        if !self.video.encoder.send_event(event) {
            warn!("force-keyframe event was not handled by encoder");
        }
    }
}
