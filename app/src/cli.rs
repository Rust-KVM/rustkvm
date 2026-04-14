// CLI argument definitions for RustKVM
// Provides comprehensive command-line interface for video/audio pipeline control

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Custom value parser for quality factor (0.1-2.0)
fn quality_in_range(s: &str) -> Result<f32, String> {
    let value: f32 = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if (0.1..=2.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("Quality must be between 0.1 and 2.0, got {}", value))
    }
}

/// Custom value parser for audio channels (1-2)
fn audio_channels_in_range(s: &str) -> Result<u32, String> {
    let value: u32 = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if (1..=2).contains(&value) {
        Ok(value)
    } else {
        Err(format!("Audio channels must be 1 or 2, got {}", value))
    }
}

/// Custom value parser for audio bitrate (4000-650000)
fn audio_bitrate_in_range(s: &str) -> Result<i32, String> {
    let value: i32 = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if (4000..=650000).contains(&value) {
        Ok(value)
    } else {
        Err(format!("Audio bitrate must be between 4000 and 650000, got {}", value))
    }
}

/// RustKVM - High-performance KVM over IP solution with hardware-accelerated encoding
#[derive(Parser, Debug)]
#[command(
    name = "rustkvm",
    version,
    author,
    about,
    long_about = "RustKVM provides KVM-over-IP functionality with hardware-accelerated video encoding \
                  using Rockchip MPP on RK3588 platforms. Supports H.264/H.265 encoding with WebRTC streaming."
)]
pub struct Cli {
    /// Video source configuration
    #[command(flatten)]
    pub video: VideoArgs,

    /// Audio source configuration
    #[command(flatten)]
    pub audio: AudioArgs,

    /// Network and service configuration
    #[command(flatten)]
    pub network: NetworkArgs,

    /// Logging configuration
    #[command(flatten)]
    pub logging: LoggingArgs,

    /// Enable audio pipeline
    #[arg(long, default_value = "true", env = "RUSTKVM_AUDIO_ENABLED")]
    pub audio_enabled: bool,

    /// Video quality factor (0.1-2.0, default 1.0)
    #[arg(
        long,
        short = 'q',
        value_name = "QUALITY",
        default_value = "1.0",
        value_parser = quality_in_range,
        env = "RUSTKVM_QUALITY"
    )]
    pub quality: f32,

    /// Configuration file path (optional, overrides CLI args)
    #[arg(long, short = 'c', value_name = "FILE", env = "RUSTKVM_CONFIG")]
    pub config: Option<PathBuf>,

    /// Enable dry-run mode (validate config without starting services)
    #[arg(long)]
    pub dry_run: bool,
}

/// Video encoder type selection
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Default)]
pub enum VideoEncoder {
    /// H.264/AVC encoder (Rockchip MPP)
    #[default]
    #[value(name = "h264")]
    H264,
    /// H.265/HEVC encoder (Rockchip MPP)
    #[value(name = "h265")]
    H265,
}

impl VideoEncoder {
    /// Get GStreamer element name for this encoder
    #[inline]
    pub const fn gst_element_name(self) -> &'static str {
        match self {
            Self::H264 => "mpph264enc",
            Self::H265 => "mpph265enc",
        }
    }

    /// Get parser element name for this encoder
    #[inline]
    pub const fn parser_element_name(self) -> &'static str {
        match self {
            Self::H264 => "h264parse",
            Self::H265 => "h265parse",
        }
    }

    /// Get codec name for display/logging
    #[inline]
    pub const fn codec_name(self) -> &'static str {
        match self {
            Self::H264 => "H.264/AVC",
            Self::H265 => "H.265/HEVC",
        }
    }
}

/// Video encoding rate control mode
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Default)]
pub enum RateControlMode {
    /// Variable Bitrate (default, best quality)
    #[default]
    #[value(name = "vbr")]
    Vbr,
    /// Constant Bitrate (stable bandwidth)
    #[value(name = "cbr")]
    Cbr,
    /// Average Bitrate
    #[value(name = "avbr")]
    Avbr,
}

impl RateControlMode {
    #[inline]
    pub const fn as_gst_str(self) -> &'static str {
        match self {
            Self::Vbr => "vbr",
            Self::Cbr => "cbr",
            Self::Avbr => "avbr",
        }
    }
}

/// Video pipeline arguments
#[derive(Parser, Debug)]
#[command(next_help_heading = "Video Options")]
pub struct VideoArgs {
    /// Video capture device path
    #[arg(
        long,
        value_name = "DEVICE",
        default_value = "/dev/video0",
        env = "RUSTKVM_VIDEO_DEVICE"
    )]
    pub video_device: String,

    /// Video encoder type (h264 or h265)
    #[arg(long, value_name = "ENCODER", default_value = "h264", env = "RUSTKVM_VIDEO_ENCODER")]
    pub video_encoder: VideoEncoder,

    /// Target video bitrate in bps
    #[arg(long, value_name = "BPS", default_value = "20000000", env = "RUSTKVM_VIDEO_BITRATE")]
    pub video_bitrate: u32,

    /// Maximum video bitrate in bps
    #[arg(long, value_name = "BPS", default_value = "60000000", env = "RUSTKVM_VIDEO_MAX_BITRATE")]
    pub video_max_bitrate: u32,

    /// Rate control mode
    #[arg(long, value_name = "MODE", default_value = "vbr", env = "RUSTKVM_VIDEO_RC_MODE")]
    pub video_rc_mode: RateControlMode,

    /// GOP size (Group of Pictures)
    /// Special values: -1 = auto (use FPS), positive = specific value
    #[arg(long, value_name = "FRAMES", allow_hyphen_values = true, env = "RUSTKVM_VIDEO_GOP")]
    pub video_gop: Option<i32>,

    /// H.264/H.265 level (e.g., "5.2", "5.1")
    #[arg(long, value_name = "LEVEL", default_value = "5.2", env = "RUSTKVM_VIDEO_LEVEL")]
    pub video_level: String,

    /// H.264/H.265 profile (baseline, main, high for H.264)
    #[arg(long, value_name = "PROFILE", env = "RUSTKVM_VIDEO_PROFILE")]
    pub video_profile: Option<String>,
}

/// Audio pipeline arguments
#[derive(Parser, Debug)]
#[command(next_help_heading = "Audio Options")]
pub struct AudioArgs {
    /// Audio capture device (ALSA device name)
    #[arg(long, value_name = "DEVICE", default_value = "hw:0,0", env = "RUSTKVM_AUDIO_DEVICE")]
    pub audio_device: String,

    /// Audio sample rate in Hz
    #[arg(long, value_name = "HZ", default_value = "48000", env = "RUSTKVM_AUDIO_SAMPLE_RATE")]
    pub audio_sample_rate: u32,

    /// Audio channels (1=mono, 2=stereo)
    #[arg(
        long,
        value_name = "COUNT",
        default_value = "2",
        value_parser = audio_channels_in_range,
        env = "RUSTKVM_AUDIO_CHANNELS"
    )]
    pub audio_channels: u32,

    /// Opus encoding bitrate (4000-650000 bps)
    #[arg(
        long,
        value_name = "BPS",
        default_value = "64000",
        value_parser = audio_bitrate_in_range,
        env = "RUSTKVM_AUDIO_BITRATE"
    )]
    pub audio_bitrate: i32,
}

/// Network and service arguments
#[derive(Parser, Debug)]
#[command(next_help_heading = "Network Options")]
pub struct NetworkArgs {
    /// HTTP server bind address
    #[arg(long, value_name = "ADDR", default_value = "0.0.0.0:8000", env = "RUSTKVM_HTTP_ADDR")]
    pub http_addr: String,

    /// HTTPS server bind address
    #[arg(long, value_name = "ADDR", default_value = "0.0.0.0:8443", env = "RUSTKVM_HTTPS_ADDR")]
    pub https_addr: String,

    /// mDNS hostname (without .local suffix)
    #[arg(long, value_name = "HOSTNAME", default_value = "rustkvm", env = "RUSTKVM_MDNS_HOSTNAME")]
    pub mdns_hostname: String,

    /// Disable mDNS service
    #[arg(long, env = "RUSTKVM_MDNS_DISABLE")]
    pub mdns_disable: bool,

    /// Disable cloud connection
    #[arg(long, env = "RUSTKVM_CLOUD_DISABLE")]
    pub cloud_disable: bool,
}

/// Logging configuration arguments
#[derive(Parser, Debug)]
#[command(next_help_heading = "Logging Options")]
pub struct LoggingArgs {
    /// Log level (trace, debug, info, warn, error)
    #[arg(long, short = 'v', value_name = "LEVEL", default_value = "info", env = "RUST_LOG")]
    pub log_level: String,

    /// Enable JSON structured logging
    #[arg(long, env = "RUSTKVM_LOG_JSON")]
    pub log_json: bool,

    /// Disable ANSI colors in logs
    #[arg(long, env = "RUSTKVM_LOG_NO_COLOR")]
    pub log_no_color: bool,

    /// Log output file (default: stderr)
    #[arg(long, value_name = "FILE", env = "RUSTKVM_LOG_FILE")]
    pub log_file: Option<PathBuf>,
}

impl Cli {
    /// Parse CLI arguments from environment
    #[inline]
    pub fn parse_args() -> Self {
        Self::parse()
    }

    /// Validate configuration consistency
    pub fn validate(&self) -> anyhow::Result<()> {
        // Validate bitrate constraints
        if self.video.video_max_bitrate < self.video.video_bitrate {
            anyhow::bail!(
                "Max bitrate ({}) must be >= target bitrate ({})",
                self.video.video_max_bitrate,
                self.video.video_bitrate
            );
        }

        // Validate GOP size
        if let Some(gop) = self.video.video_gop
            && gop == 0
        {
            anyhow::bail!("GOP size cannot be 0 (use -1 for auto or positive value)");
        }

        // Validate quality factor
        if !(0.1..=2.0).contains(&self.quality) {
            anyhow::bail!("Quality must be between 0.1 and 2.0");
        }

        Ok(())
    }

    /// Print configuration summary
    pub fn print_summary(&self) {
        tracing::info!("=== RustKVM Configuration ===");
        tracing::info!("Video:");
        tracing::info!("  Device: {}", self.video.video_device);
        tracing::info!(
            "  Encoder: {} ({})",
            self.video.video_encoder.codec_name(),
            self.video.video_encoder.gst_element_name()
        );
        tracing::info!(
            "  Bitrate: {} bps (max: {} bps)",
            self.video.video_bitrate,
            self.video.video_max_bitrate
        );
        tracing::info!("  Rate Control: {:?}", self.video.video_rc_mode);
        tracing::info!("  GOP Size: {:?}", self.video.video_gop.unwrap_or(-1));
        tracing::info!("  Level: {}", self.video.video_level);
        if let Some(ref profile) = self.video.video_profile {
            tracing::info!("  Profile: {}", profile);
        }
        tracing::info!("  Quality Factor: {}", self.quality);

        if self.audio_enabled {
            tracing::info!("Audio:");
            tracing::info!("  Device: {}", self.audio.audio_device);
            tracing::info!("  Sample Rate: {} Hz", self.audio.audio_sample_rate);
            tracing::info!("  Channels: {}", self.audio.audio_channels);
            tracing::info!("  Bitrate: {} bps", self.audio.audio_bitrate);
        } else {
            tracing::info!("Audio: Disabled");
        }

        tracing::info!("Network:");
        tracing::info!("  HTTP: {}", self.network.http_addr);
        tracing::info!("  HTTPS: {}", self.network.https_addr);
        if !self.network.mdns_disable {
            tracing::info!("  mDNS: {}.local", self.network.mdns_hostname);
        }
        tracing::info!("=============================");
    }
}
