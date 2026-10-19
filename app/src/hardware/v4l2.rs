//! The HDMI receiver's live state and its one recovery lever.
//!
//! After the host wakes from a long sleep the RK3588 receiver can fall into a
//! relock loop: every lock ends in `hdmirx_format_change`, which queues a
//! V4L2 source-change event even when nothing changed; v4l2src answers each
//! event by restarting the stream; the restart relocks and queues the next
//! event. The loop runs about once a second, frames are cut mid-DMA (green
//! and magenta stripes), and it does not end on its own. A replug ends it, so
//! the recovery is a software replug: drop hot-plug detect and raise it again.

use std::collections::VecDeque;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Getter, Opcode, Setter, Updater, ioctl, opcode};

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

/// `struct v4l2_event_subscription`.
#[repr(C)]
struct V4l2EventSubscription {
    ev_type: u32,
    id: u32,
    flags: u32,
    reserved: [u32; 5],
}

/// `struct v4l2_event`: the union holds an `s64`, so it starts at offset 8
/// and the struct is 136 bytes on 64-bit targets.
#[repr(C, align(8))]
struct V4l2Event {
    ev_type: u32,
    _pad: u32,
    u: [u8; 64],
    pending: u32,
    sequence: u32,
    timestamp: [i64; 2],
    id: u32,
    reserved: [u32; 8],
}

const _: () = assert!(std::mem::size_of::<V4l2Format>() == 208);
const _: () = assert!(std::mem::size_of::<V4l2DvTimings>() == 132);
const _: () = assert!(std::mem::size_of::<V4l2EventSubscription>() == 32);
const _: () = assert!(std::mem::size_of::<V4l2Event>() == 136);

const V4L2_EVENT_SOURCE_CHANGE: u32 = 5;
const VIDIOC_DQEVENT: Opcode = opcode::read::<V4l2Event>(b'V', 89);
const VIDIOC_SUBSCRIBE_EVENT: Opcode = opcode::write::<V4l2EventSubscription>(b'V', 90);

/// The rockchip_hdmirx sysfs node: writing `off` drops hot-plug detect and
/// `on` raises it again.
const HDMIRX_HPD_NODE: &str = "/sys/class/hdmirx/hdmirx/status";

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

/// A non-streaming handle on the capture node, kept open across polls. It
/// subscribes to source-change events so it sees every relock the driver
/// reports, alongside whatever is streaming.
pub struct CaptureProbe {
    fd: OwnedFd,
    events: bool,
}

impl CaptureProbe {
    pub fn open(device: &str) -> Result<Self> {
        let fd = open(device, OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())
            .with_context(|| format!("open {device}"))?;
        let sub = V4l2EventSubscription {
            ev_type: V4L2_EVENT_SOURCE_CHANGE,
            id: 0,
            flags: 0,
            reserved: [0; 5],
        };
        // SAFETY: VIDIOC_SUBSCRIBE_EVENT's opcode is built from
        // V4l2EventSubscription, which matches the kernel's 32-byte struct;
        // the kernel only reads it.
        let events = unsafe {
            ioctl(&fd, Setter::<VIDIOC_SUBSCRIBE_EVENT, V4l2EventSubscription>::new(sub))
        }
        .is_ok();
        Ok(Self { fd, events })
    }

    /// Whether the driver accepted the source-change subscription.
    pub fn has_events(&self) -> bool {
        self.events
    }

    /// Dequeues and counts pending source-change events. The kernel keeps
    /// only the newest one per subscription, so this counts at most one per
    /// call unless several arrived in between; callers poll often enough.
    pub fn take_source_changes(&self) -> u32 {
        if !self.events {
            return 0;
        }
        let mut n = 0;
        while n < 1024 {
            // SAFETY: VIDIOC_DQEVENT's opcode is built from V4l2Event, which
            // matches the kernel's 136-byte struct v4l2_event.
            match unsafe { ioctl(&self.fd, Getter::<VIDIOC_DQEVENT, V4l2Event>::new()) } {
                Ok(ev) if ev.ev_type == V4L2_EVENT_SOURCE_CHANGE => n += 1,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        n
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

/// Drops hot-plug detect for `low_for`, then raises it, so the source sees an
/// unplug and renegotiates the link from scratch. HPD is always raised again,
/// with retries: leaving it low would blank the input for good.
pub fn pulse_hotplug(low_for: Duration) -> Result<()> {
    std::fs::write(HDMIRX_HPD_NODE, "off")
        .with_context(|| format!("write off to {HDMIRX_HPD_NODE}"))?;
    std::thread::sleep(low_for);
    let mut last = None;
    for _ in 0..5 {
        match std::fs::write(HDMIRX_HPD_NODE, "on") {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    match last {
        Some(e) => Err(e).with_context(|| format!("write on to {HDMIRX_HPD_NODE}")),
        None => Ok(()),
    }
}

/// What the latest source-change events mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelockVerdict {
    Quiet,
    /// A relock storm, but the last replug was too recent to try again.
    Storm {
        events: u32,
    },
    /// A relock storm: replug now. `attempt` counts replugs in this episode.
    Replug {
        events: u32,
        attempt: u32,
    },
}

/// Recognises the relock loop from source-change events. A healthy input
/// reports none, a real mode change or a replug a few; the loop reports one
/// a second without end. Replugs back off while the loop persists.
#[derive(Debug, Default)]
pub struct RelockWatch {
    recent: VecDeque<(Instant, u32)>,
    last_replug: Option<Instant>,
    attempts: u32,
}

impl RelockWatch {
    /// Window the events are counted over.
    pub const WINDOW: Duration = Duration::from_secs(10);
    /// Events within the window that make a storm.
    pub const THRESHOLD: u32 = 6;
    /// Quiet time after which an episode is over and backoff resets.
    pub const SETTLE: Duration = Duration::from_secs(300);

    fn backoff(attempts: u32) -> Duration {
        Duration::from_secs(30)
            .saturating_mul(1u32 << attempts.min(5))
            .min(Duration::from_secs(600))
    }

    pub fn observe(&mut self, now: Instant, events: u32) -> RelockVerdict {
        if events > 0 {
            self.recent.push_back((now, events));
        }
        while self.recent.front().is_some_and(|&(t, _)| now.duration_since(t) > Self::WINDOW) {
            self.recent.pop_front();
        }
        let total: u32 = self.recent.iter().map(|&(_, n)| n).sum();
        if total < Self::THRESHOLD {
            if self.last_replug.is_some_and(|t| now.duration_since(t) >= Self::SETTLE) {
                self.attempts = 0;
                self.last_replug = None;
            }
            return RelockVerdict::Quiet;
        }
        let due = self.attempts == 0
            || self
                .last_replug
                .is_none_or(|t| now.duration_since(t) >= Self::backoff(self.attempts - 1));
        if !due {
            return RelockVerdict::Storm { events: total };
        }
        self.attempts += 1;
        self.last_replug = Some(now);
        self.recent.clear();
        RelockVerdict::Replug { events: total, attempt: self.attempts }
    }
}
