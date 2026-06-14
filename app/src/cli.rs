use std::path::PathBuf;

use clap::{Parser, ValueEnum};

fn quality_in_range(s: &str) -> Result<f32, String> {
    let value: f32 = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if (0.1..=2.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("Quality must be between 0.1 and 2.0, got {}", value))
    }
}

fn audio_channels_in_range(s: &str) -> Result<u32, String> {
    let value: u32 = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if (1..=2).contains(&value) {
        Ok(value)
    } else {
        Err(format!("Audio channels must be 1 or 2, got {}", value))
    }
}

fn audio_bitrate_in_range(s: &str) -> Result<i32, String> {
    let value: i32 = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if (4000..=650000).contains(&value) {
        Ok(value)
    } else {
        Err(format!("Audio bitrate must be between 4000 and 650000, got {}", value))
    }
}

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
    #[command(flatten)]
    pub video: VideoArgs,

    #[command(flatten)]
    pub audio: AudioArgs,

    #[command(flatten)]
    pub network: NetworkArgs,

    #[command(flatten)]
    pub logging: LoggingArgs,

    #[arg(
        long,
        short = 'q',
        value_name = "QUALITY",
        default_value = "1.0",
        value_parser = quality_in_range,
        env = "RUSTKVM_QUALITY"
    )]
    pub quality: f32,

    #[arg(long, short = 'c', value_name = "FILE", env = "RUSTKVM_CONFIG")]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Default)]
pub enum VideoEncoder {
    #[default]
    #[value(name = "h264")]
    H264,
    #[value(name = "h265")]
    H265,
}

impl VideoEncoder {
    #[inline]
    pub const fn gst_element_name(self) -> &'static str {
        match self {
            Self::H264 => "mpph264enc",
            Self::H265 => "mpph265enc",
        }
    }

    #[inline]
    pub const fn parser_element_name(self) -> &'static str {
        match self {
            Self::H264 => "h264parse",
            Self::H265 => "h265parse",
        }
    }

    #[inline]
    pub const fn codec_name(self) -> &'static str {
        match self {
            Self::H264 => "H.264/AVC",
            Self::H265 => "H.265/HEVC",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Default)]
pub enum RateControlMode {
    #[default]
    #[value(name = "vbr")]
    Vbr,
    #[value(name = "cbr")]
    Cbr,
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

#[derive(Parser, Debug)]
#[command(next_help_heading = "Video Options")]
pub struct VideoArgs {
    #[arg(
        long,
        value_name = "DEVICE",
        default_value = "/dev/video0",
        env = "RUSTKVM_VIDEO_DEVICE"
    )]
    pub video_device: String,

    #[arg(long, value_name = "ENCODER", default_value = "h264", env = "RUSTKVM_VIDEO_ENCODER")]
    pub video_encoder: VideoEncoder,

    #[arg(long, value_name = "BPS", default_value = "20000000", env = "RUSTKVM_VIDEO_BITRATE")]
    pub video_bitrate: u32,

    #[arg(long, value_name = "BPS", default_value = "60000000", env = "RUSTKVM_VIDEO_MAX_BITRATE")]
    pub video_max_bitrate: u32,

    #[arg(long, value_name = "MODE", default_value = "vbr", env = "RUSTKVM_VIDEO_RC_MODE")]
    pub video_rc_mode: RateControlMode,

    #[arg(long, value_name = "FRAMES", allow_hyphen_values = true, env = "RUSTKVM_VIDEO_GOP")]
    pub video_gop: Option<i32>,

    #[arg(long, value_name = "LEVEL", default_value = "5.2", env = "RUSTKVM_VIDEO_LEVEL")]
    pub video_level: String,

    #[arg(long, value_name = "PROFILE", env = "RUSTKVM_VIDEO_PROFILE")]
    pub video_profile: Option<String>,

    #[arg(long, value_name = "FPS", default_value = "60", env = "RUSTKVM_VIDEO_FPS")]
    pub video_fps: u32,
}

#[derive(Parser, Debug)]
#[command(next_help_heading = "Audio Options")]
pub struct AudioArgs {
    #[arg(long, value_name = "DEVICE", default_value = "hw:0,0", env = "RUSTKVM_AUDIO_DEVICE")]
    pub audio_device: String,

    #[arg(long, value_name = "HZ", default_value = "48000", env = "RUSTKVM_AUDIO_SAMPLE_RATE")]
    pub audio_sample_rate: u32,

    #[arg(
        long,
        value_name = "COUNT",
        default_value = "2",
        value_parser = audio_channels_in_range,
        env = "RUSTKVM_AUDIO_CHANNELS"
    )]
    pub audio_channels: u32,

    #[arg(
        long,
        value_name = "BPS",
        default_value = "64000",
        value_parser = audio_bitrate_in_range,
        env = "RUSTKVM_AUDIO_BITRATE"
    )]
    pub audio_bitrate: i32,
}

#[derive(Parser, Debug)]
#[command(next_help_heading = "Network Options")]
pub struct NetworkArgs {
    #[arg(long, value_name = "ADDR", default_value = "0.0.0.0:8000", env = "RUSTKVM_HTTP_ADDR")]
    pub http_addr: String,

    #[arg(long, value_name = "ADDR", default_value = "0.0.0.0:8443", env = "RUSTKVM_HTTPS_ADDR")]
    pub https_addr: String,

    #[arg(long, value_name = "HOSTNAME", default_value = "rustkvm", env = "RUSTKVM_MDNS_HOSTNAME")]
    pub mdns_hostname: String,

    #[arg(long, env = "RUSTKVM_MDNS_DISABLE")]
    pub mdns_disable: bool,

    #[arg(long, env = "RUSTKVM_CLOUD_DISABLE")]
    pub cloud_disable: bool,
}

#[derive(Parser, Debug)]
#[command(next_help_heading = "Logging Options")]
pub struct LoggingArgs {
    #[arg(long, short = 'v', value_name = "LEVEL", default_value = "info", env = "RUST_LOG")]
    pub log_level: String,

    #[arg(long, env = "RUSTKVM_LOG_JSON")]
    pub log_json: bool,

    #[arg(long, env = "RUSTKVM_LOG_NO_COLOR")]
    pub log_no_color: bool,

    #[arg(long, value_name = "FILE", env = "RUSTKVM_LOG_FILE")]
    pub log_file: Option<PathBuf>,
}

impl Cli {
    #[inline]
    pub fn parse_args() -> Self {
        Self::parse()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.video.video_max_bitrate < self.video.video_bitrate {
            anyhow::bail!(
                "Max bitrate ({}) must be >= target bitrate ({})",
                self.video.video_max_bitrate,
                self.video.video_bitrate
            );
        }

        if let Some(gop) = self.video.video_gop
            && gop == 0
        {
            anyhow::bail!("GOP size cannot be 0 (use -1 for auto or positive value)");
        }

        if !(0.1..=2.0).contains(&self.quality) {
            anyhow::bail!("Quality must be between 0.1 and 2.0");
        }

        Ok(())
    }

    pub fn print_summary(&self) {
        tracing::info!(
            video.device = %self.video.video_device,
            video.encoder = self.video.video_encoder.codec_name(),
            video.bitrate = self.video.video_bitrate,
            video.max_bitrate = self.video.video_max_bitrate,
            video.rc_mode = ?self.video.video_rc_mode,
            video.gop = self.video.video_gop.unwrap_or(-1),
            video.level = self.video.video_level,
            video.profile = ?self.video.video_profile,
            quality_factor = self.quality,
            audio.device = %self.audio.audio_device,
            audio.sample_rate_hz = self.audio.audio_sample_rate,
            audio.channels = self.audio.audio_channels,
            audio.bitrate = self.audio.audio_bitrate,
            net.http = %self.network.http_addr,
            net.https = %self.network.https_addr,
            net.mdns = if self.network.mdns_disable {
                "disabled".to_string()
            } else {
                format!("{}.local", self.network.mdns_hostname)
            },
            "rustkvm configuration"
        );
    }
}
