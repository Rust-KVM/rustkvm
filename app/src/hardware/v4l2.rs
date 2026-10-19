//! Read-only queries of the HDMI capture node's live state, used to notice a
//! source change (resolution, timings or pixel layout) while the GStreamer
//! pipeline keeps streaming with the format it negotiated at start.

use std::os::fd::OwnedFd;

use anyhow::{Context, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Getter, Opcode, Updater, ioctl, opcode};

const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE: u32 = 9;

/// `struct v4l2_format`: the union holds pointers, so it starts at offset 8
/// and the struct is 208 bytes on 64-bit targets.
#[repr(C, align(8))]
struct V4l2Format {
    buf_type: u32,
    _pad: u32,
    fmt: [u8; 200],
}

/// `struct v4l2_dv_timings` (packed): a type and a 128-byte union whose
/// `v4l2_bt_timings` member is what HDMI receivers report.
#[repr(C)]
struct V4l2DvTimings {
    timing_type: u32,
    data: [u8; 128],
}

const _: () = assert!(std::mem::size_of::<V4l2Format>() == 208);
const _: () = assert!(std::mem::size_of::<V4l2DvTimings>() == 132);

const VIDIOC_G_FMT: Opcode = opcode::read_write::<V4l2Format>(b'V', 4);
const VIDIOC_QUERY_DV_TIMINGS: Opcode = opcode::read::<V4l2DvTimings>(b'V', 99);

/// The pixel layout the driver currently writes into capture buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFormat {
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub bytesperline: u32,
    pub sizeimage: u32,
}

impl CaptureFormat {
    pub fn fourcc_str(&self) -> String {
        self.fourcc
            .to_le_bytes()
            .iter()
            .map(|&b| if b.is_ascii_graphic() { b as char } else { '?' })
            .collect()
    }
}

/// The signal timings the HDMI receiver currently detects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DvTimings {
    pub width: u32,
    pub height: u32,
    pub interlaced: bool,
    pub pixelclock: u64,
}

/// What the capture node reports at one point in time. `timings` is `None`
/// when no stable signal is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureSignal {
    pub format: Option<CaptureFormat>,
    pub timings: Option<DvTimings>,
}

impl std::fmt::Display for CaptureSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.format {
            Some(c) => write!(
                f,
                "format {} {}x{} stride {} size {}",
                c.fourcc_str(),
                c.width,
                c.height,
                c.bytesperline,
                c.sizeimage
            )?,
            None => write!(f, "format unknown")?,
        }
        match self.timings {
            Some(t) => write!(
                f,
                ", signal {}x{}{} {:.2} MHz",
                t.width,
                t.height,
                if t.interlaced { "i" } else { "p" },
                t.pixelclock as f64 / 1e6
            ),
            None => write!(f, ", no signal"),
        }
    }
}

fn u32_at(buf: &[u8], offset: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&buf[offset..offset + 4]);
    u32::from_ne_bytes(b)
}

fn u64_at(buf: &[u8], offset: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[offset..offset + 8]);
    u64::from_ne_bytes(b)
}

/// Parses the `fmt` union of `struct v4l2_format` for a capture buffer type:
/// `v4l2_pix_format_mplane` (first plane) or `v4l2_pix_format`.
pub fn parse_pix_format(mplane: bool, fmt: &[u8; 200]) -> CaptureFormat {
    let (bytesperline, sizeimage) = if mplane {
        (u32_at(fmt, 24), u32_at(fmt, 20))
    } else {
        (u32_at(fmt, 16), u32_at(fmt, 20))
    };
    CaptureFormat {
        width: u32_at(fmt, 0),
        height: u32_at(fmt, 4),
        fourcc: u32_at(fmt, 8),
        bytesperline,
        sizeimage,
    }
}

/// Parses the `v4l2_bt_timings` member of `struct v4l2_dv_timings`.
pub fn parse_bt_timings(data: &[u8; 128]) -> DvTimings {
    DvTimings {
        width: u32_at(data, 0),
        height: u32_at(data, 4),
        interlaced: u32_at(data, 8) != 0,
        pixelclock: u64_at(data, 16),
    }
}

/// A non-streaming handle on the capture node, kept open across polls.
pub struct CaptureProbe {
    fd: OwnedFd,
}

impl CaptureProbe {
    pub fn open(device: &str) -> Result<Self> {
        let fd = open(device, OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())
            .with_context(|| format!("open {device}"))?;
        Ok(Self { fd })
    }

    pub fn query(&self) -> CaptureSignal {
        CaptureSignal { format: self.format().ok(), timings: self.timings().ok() }
    }

    fn format(&self) -> Result<CaptureFormat> {
        for (buf_type, mplane) in
            [(V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, true), (V4L2_BUF_TYPE_VIDEO_CAPTURE, false)]
        {
            let mut req = V4l2Format { buf_type, _pad: 0, fmt: [0; 200] };
            // SAFETY: VIDIOC_G_FMT's opcode is built from V4l2Format, whose
            // layout matches the kernel's 208-byte struct v4l2_format; the
            // kernel only writes within it.
            let rc = unsafe { ioctl(&self.fd, Updater::<VIDIOC_G_FMT, V4l2Format>::new(&mut req)) };
            if rc.is_ok() {
                return Ok(parse_pix_format(mplane, &req.fmt));
            }
        }
        anyhow::bail!("VIDIOC_G_FMT failed for both capture buffer types")
    }

    fn timings(&self) -> Result<DvTimings> {
        // SAFETY: VIDIOC_QUERY_DV_TIMINGS's opcode is built from
        // V4l2DvTimings, which matches the kernel's packed 132-byte struct.
        let t = unsafe { ioctl(&self.fd, Getter::<VIDIOC_QUERY_DV_TIMINGS, V4l2DvTimings>::new()) }
            .context("VIDIOC_QUERY_DV_TIMINGS")?;
        if t.timing_type != 0 {
            anyhow::bail!("unsupported DV timing type {}", t.timing_type);
        }
        Ok(parse_bt_timings(&t.data))
    }
}

/// What one poll of the capture node means for the running pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceObservation {
    /// First reading with a signal; it becomes the reference.
    Baseline,
    Steady,
    NoSignal,
    /// Differs from the reference once; confirmed on the next poll.
    Pending,
    /// Differed on two polls in a row: the pipeline no longer matches the
    /// source and must be rebuilt. Carries the old reference.
    Changed {
        from: CaptureSignal,
    },
}

/// Tracks the capture node's reported state against the state it had when
/// the pipeline started. Losing the signal is not a change by itself; the
/// signal coming back different is.
#[derive(Debug, Default)]
pub struct SourceWatch {
    baseline: Option<CaptureSignal>,
    pending: Option<CaptureSignal>,
}

impl SourceWatch {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn observe(&mut self, now: CaptureSignal) -> SourceObservation {
        if now.timings.is_none() {
            self.pending = None;
            return SourceObservation::NoSignal;
        }
        let Some(baseline) = self.baseline else {
            self.baseline = Some(now);
            return SourceObservation::Baseline;
        };
        if now == baseline {
            self.pending = None;
            return SourceObservation::Steady;
        }
        if self.pending == Some(now) {
            self.reset();
            return SourceObservation::Changed { from: baseline };
        }
        self.pending = Some(now);
        SourceObservation::Pending
    }
}
