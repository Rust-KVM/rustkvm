use anyhow::{Result, bail};

use crate::types::Rect;

/// Outputs at least this large are resampled by several threads.
const PARALLEL_SAMPLE_PIXELS: usize = 256 * 1024;
const SAMPLE_THREADS: usize = 4;

/// A picture the engine can read: decoded RGB, or a captured NV12 frame read
/// in place.
pub trait Frame: Sync {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    /// Writes BGR pixels for one output row: `out[i]` is the bilinear sample
    /// at source column taps `xs[i]` on source row taps `y`.
    fn sample_row(&self, y: Tap, xs: &[Tap], out: &mut [u8]);
    /// Full-range luma of one pixel.
    fn luma(&self, x: usize, y: usize) -> u8;
}

/// Two neighbouring source indices and the weight of the second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    pub i0: usize,
    pub i1: usize,
    pub w: f32,
}

impl Tap {
    /// The bilinear tap for output index `o` of `n` covering `[from, to)`
    /// of a source axis `len` long.
    fn new(o: usize, n: usize, from: f32, to: f32, len: usize) -> Self {
        let step = (to - from) / n as f32;
        let f = (from + (o as f32 + 0.5) * step - 0.5).clamp(0.0, (len - 1) as f32);
        let i0 = f as usize;
        Self { i0, i1: (i0 + 1).min(len - 1), w: f - i0 as f32 }
    }

    /// The same position on an axis subsampled by two (NV12 chroma).
    fn half(&self, len: usize) -> Self {
        let f = ((self.i0 as f32 + self.w + 0.5) / 2.0 - 0.5).clamp(0.0, (len - 1) as f32);
        let i0 = f as usize;
        Self { i0, i1: (i0 + 1).min(len - 1), w: f - i0 as f32 }
    }
}

fn lerp(a: f32, b: f32, w: f32) -> f32 {
    a + (b - a) * w
}

/// A source rectangle in image pixels with fractional edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// Resamples `src` of `frame` bilinearly into a `dst_w`×`dst_h` BGR patch at
/// the top-left of `out`, whose rows are `out_stride` pixels long. Pixels of
/// `out` outside the patch are left as they are.
pub fn sample_bgr<F: Frame + ?Sized>(
    frame: &F,
    src: RectF,
    dst_w: usize,
    dst_h: usize,
    out: &mut [u8],
    out_stride: usize,
) {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    if dst_w == 0
        || dst_h == 0
        || w == 0
        || h == 0
        || out_stride < dst_w
        || out.len() < out_stride * 3 * (dst_h - 1) + dst_w * 3
    {
        return;
    }
    let xs: Vec<Tap> = (0..dst_w).map(|o| Tap::new(o, dst_w, src.x0, src.x1, w)).collect();
    let row_bytes = out_stride * 3;
    let rows = out[..row_bytes * (dst_h - 1) + dst_w * 3].chunks_mut(row_bytes);
    let sample = |oy: usize, row: &mut [u8]| {
        frame.sample_row(Tap::new(oy, dst_h, src.y0, src.y1, h), &xs, &mut row[..dst_w * 3]);
    };
    if dst_w * dst_h < PARALLEL_SAMPLE_PIXELS {
        rows.enumerate().for_each(|(oy, row)| sample(oy, row));
        return;
    }
    let mut rows: Vec<(usize, &mut [u8])> = rows.enumerate().collect();
    let per_thread = rows.len().div_ceil(SAMPLE_THREADS);
    std::thread::scope(|scope| {
        for chunk in rows.chunks_mut(per_thread) {
            scope.spawn(|| chunk.iter_mut().for_each(|(oy, row)| sample(*oy, row)));
        }
    });
}

/// For each column of `rect`, whether it holds no ink: its luma range is
/// small compared with the range of the whole rectangle.
pub fn blank_columns<F: Frame + ?Sized>(frame: &F, rect: Rect) -> Vec<bool> {
    let Some(rect) = rect.clamp_to(frame.width(), frame.height()) else {
        return Vec::new();
    };
    let (x0, n) = (rect.x as usize, rect.width as usize);
    let mut lo = vec![u8::MAX; n];
    let mut hi = vec![u8::MIN; n];
    for y in rect.y as usize..rect.bottom() as usize {
        for (i, (l, h)) in lo.iter_mut().zip(hi.iter_mut()).enumerate() {
            let v = frame.luma(x0 + i, y);
            *l = (*l).min(v);
            *h = (*h).max(v);
        }
    }
    let min = lo.iter().copied().min().unwrap_or(0);
    let max = hi.iter().copied().max().unwrap_or(0);
    let threshold = (u32::from(max - min) / 4).max(24);
    lo.iter().zip(&hi).map(|(&l, &h)| u32::from(h - l) < threshold).collect()
}

/// Interleaved 8-bit RGB pixels, rows packed without padding.
pub struct RgbImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl RgbImage {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = width as usize * height as usize * 3;
        if width == 0 || height == 0 || data.len() != expected {
            bail!("RGB image {width}x{height} needs {expected} bytes, got {}", data.len());
        }
        Ok(Self { width, height, data })
    }

    /// Converts a whole NV12 frame to RGB (see [`Nv12Frame`] to read one in
    /// place instead).
    pub fn from_nv12(frame: &Nv12Frame<'_>) -> Self {
        let (w, h) = (frame.width as usize, frame.height as usize);
        let mut data = vec![0u8; w * h * 3];
        for (y, row) in data.chunks_exact_mut(w * 3).enumerate() {
            for (x, px) in row.chunks_exact_mut(3).enumerate() {
                let (yy, u, v) = frame.yuv(x, y);
                px.copy_from_slice(&yuv_to_rgb(yy, u, v));
            }
        }
        Self { width: frame.width, height: frame.height, data }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn px(&self, x: usize, y: usize) -> &[u8] {
        let i = (y * self.width as usize + x) * 3;
        &self.data[i..i + 3]
    }
}

impl Frame for RgbImage {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn sample_row(&self, y: Tap, xs: &[Tap], out: &mut [u8]) {
        for (x, px) in xs.iter().zip(out.chunks_exact_mut(3)) {
            let (a, b, c, d) = (
                self.px(x.i0, y.i0),
                self.px(x.i1, y.i0),
                self.px(x.i0, y.i1),
                self.px(x.i1, y.i1),
            );
            for ch in 0..3 {
                let top = lerp(f32::from(a[ch]), f32::from(b[ch]), x.w);
                let bottom = lerp(f32::from(c[ch]), f32::from(d[ch]), x.w);
                px[2 - ch] = (lerp(top, bottom, y.w) + 0.5) as u8;
            }
        }
    }

    fn luma(&self, x: usize, y: usize) -> u8 {
        let p = self.px(x, y);
        ((u32::from(p[0]) * 77 + u32::from(p[1]) * 150 + u32::from(p[2]) * 29) >> 8) as u8
    }
}

/// A captured NV12 frame (BT.601, limited range) read in place: the planes
/// stay in the capture buffer and only the pixels the models need are
/// converted.
pub struct Nv12Frame<'a> {
    width: u32,
    height: u32,
    y: &'a [u8],
    y_stride: usize,
    uv: &'a [u8],
    uv_stride: usize,
}

impl<'a> Nv12Frame<'a> {
    /// The planes may carry row padding; `y_stride` and `uv_stride` are
    /// their row lengths in bytes.
    pub fn new(
        width: u32,
        height: u32,
        y: &'a [u8],
        y_stride: usize,
        uv: &'a [u8],
        uv_stride: usize,
    ) -> Result<Self> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
            bail!("NV12 frame must have even, non-zero dimensions, got {width}x{height}");
        }
        if y_stride < w || uv_stride < w {
            bail!("NV12 strides {y_stride}/{uv_stride} are shorter than the width {width}");
        }
        if y.len() < y_stride * (h - 1) + w || uv.len() < uv_stride * (h / 2 - 1) + w {
            bail!("NV12 planes are too small for {width}x{height}");
        }
        Ok(Self { width, height, y, y_stride, uv, uv_stride })
    }

    fn yuv(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let c = (y / 2) * self.uv_stride + (x & !1);
        (self.y[y * self.y_stride + x], self.uv[c], self.uv[c + 1])
    }
}

/// BT.601 limited-range YUV to RGB, in the fixed-point form of the encoder
/// path.
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = (i32::from(y) - 16).max(0) * 298;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    [
        ((c + 409 * e + 128) >> 8).clamp(0, 255) as u8,
        ((c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8,
        ((c + 516 * d + 128) >> 8).clamp(0, 255) as u8,
    ]
}

impl Frame for Nv12Frame<'_> {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn sample_row(&self, y: Tap, xs: &[Tap], out: &mut [u8]) {
        let (cw, ch) = (self.width as usize / 2, self.height as usize / 2);
        let cy = y.half(ch);
        let y0 = &self.y[y.i0 * self.y_stride..];
        let y1 = &self.y[y.i1 * self.y_stride..];
        let c0 = &self.uv[cy.i0 * self.uv_stride..];
        let c1 = &self.uv[cy.i1 * self.uv_stride..];
        let luma = |row: &[u8], x: &Tap| lerp(f32::from(row[x.i0]), f32::from(row[x.i1]), x.w);
        let chroma = |row: &[u8], x: &Tap, k: usize| {
            lerp(f32::from(row[2 * x.i0 + k]), f32::from(row[2 * x.i1 + k]), x.w)
        };
        for (x, px) in xs.iter().zip(out.chunks_exact_mut(3)) {
            let cx = x.half(cw);
            let l = lerp(luma(y0, x), luma(y1, x), y.w);
            let u = lerp(chroma(c0, &cx, 0), chroma(c1, &cx, 0), cy.w);
            let v = lerp(chroma(c0, &cx, 1), chroma(c1, &cx, 1), cy.w);
            let [r, g, b] = yuv_to_rgb((l + 0.5) as u8, (u + 0.5) as u8, (v + 0.5) as u8);
            px.copy_from_slice(&[b, g, r]);
        }
    }

    fn luma(&self, x: usize, y: usize) -> u8 {
        let l = i32::from(self.y[y * self.y_stride + x]);
        (((l - 16).max(0) * 298 + 128) >> 8).clamp(0, 255) as u8
    }
}
