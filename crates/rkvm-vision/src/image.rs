use anyhow::{Result, bail};

use crate::types::Rect;

/// Interleaved 8-bit RGB pixels, rows packed without padding.
pub struct RgbImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

/// A source rectangle in image pixels with fractional edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl RgbImage {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = width as usize * height as usize * 3;
        if width == 0 || height == 0 || data.len() != expected {
            bail!("RGB image {width}x{height} needs {expected} bytes, got {}", data.len());
        }
        Ok(Self { width, height, data })
    }

    /// Converts NV12 (BT.601, limited range) to RGB. The planes may carry row
    /// padding; `y_stride` and `uv_stride` are their row lengths in bytes.
    pub fn from_nv12(
        width: u32,
        height: u32,
        y: &[u8],
        y_stride: usize,
        uv: &[u8],
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

        let mut data = vec![0u8; w * h * 3];
        let row_pairs = h / 2;
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4);
        let pairs_per_chunk = row_pairs.div_ceil(workers);
        std::thread::scope(|scope| {
            for (chunk_index, chunk) in data.chunks_mut(pairs_per_chunk * 2 * w * 3).enumerate() {
                scope.spawn(move || {
                    let first_pair = chunk_index * pairs_per_chunk;
                    for (pair_offset, rows) in chunk.chunks_mut(2 * w * 3).enumerate() {
                        let pair = first_pair + pair_offset;
                        let uv_row = &uv[pair * uv_stride..pair * uv_stride + w];
                        for (dy, out_row) in rows.chunks_mut(w * 3).enumerate() {
                            let row = pair * 2 + dy;
                            let y_row = &y[row * y_stride..row * y_stride + w];
                            convert_row(y_row, uv_row, out_row);
                        }
                    }
                });
            }
        });
        Ok(Self { width, height, data })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn rgb(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width as usize + x) * 3;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    fn luma(&self, x: usize, y: usize) -> u8 {
        let [r, g, b] = self.rgb(x, y);
        ((u32::from(r) * 77 + u32::from(g) * 150 + u32::from(b) * 29) >> 8) as u8
    }

    /// Resamples `src` bilinearly into a `dst_w`×`dst_h` BGR patch at the
    /// top-left of `out`, whose rows are `out_stride` pixels long. Pixels of
    /// `out` outside the patch are left as they are.
    pub fn sample_bgr(
        &self,
        src: RectF,
        dst_w: usize,
        dst_h: usize,
        out: &mut [u8],
        out_stride: usize,
    ) {
        if dst_w == 0
            || dst_h == 0
            || out_stride < dst_w
            || out.len() < out_stride * 3 * (dst_h - 1) + dst_w * 3
        {
            return;
        }
        let max_x = (self.width - 1) as f32;
        let max_y = (self.height - 1) as f32;
        let sx = (src.x1 - src.x0) / dst_w as f32;
        let sy = (src.y1 - src.y0) / dst_h as f32;
        for oy in 0..dst_h {
            let fy = (src.y0 + (oy as f32 + 0.5) * sy - 0.5).clamp(0.0, max_y);
            let y0 = fy as usize;
            let y1 = (y0 + 1).min(self.height as usize - 1);
            let wy = fy - y0 as f32;
            let row = &mut out[oy * out_stride * 3..(oy * out_stride + dst_w) * 3];
            for (ox, px) in row.chunks_exact_mut(3).enumerate() {
                let fx = (src.x0 + (ox as f32 + 0.5) * sx - 0.5).clamp(0.0, max_x);
                let x0 = fx as usize;
                let x1 = (x0 + 1).min(self.width as usize - 1);
                let wx = fx - x0 as f32;
                let (a, b, c, d) =
                    (self.rgb(x0, y0), self.rgb(x1, y0), self.rgb(x0, y1), self.rgb(x1, y1));
                for ch in 0..3 {
                    let top = f32::from(a[ch]) * (1.0 - wx) + f32::from(b[ch]) * wx;
                    let bottom = f32::from(c[ch]) * (1.0 - wx) + f32::from(d[ch]) * wx;
                    px[2 - ch] = (top * (1.0 - wy) + bottom * wy + 0.5) as u8;
                }
            }
        }
    }

    /// For each column of `rect`, whether it holds no ink: its luma range is
    /// small compared with the range of the whole rectangle.
    pub fn blank_columns(&self, rect: Rect) -> Vec<bool> {
        let Some(rect) = rect.clamp_to(self.width, self.height) else {
            return Vec::new();
        };
        let ranges: Vec<(u8, u8)> = (rect.x..rect.right())
            .map(|x| {
                (rect.y..rect.bottom()).fold((u8::MAX, u8::MIN), |(lo, hi), y| {
                    let l = self.luma(x as usize, y as usize);
                    (lo.min(l), hi.max(l))
                })
            })
            .collect();
        let lo = ranges.iter().map(|r| r.0).min().unwrap_or(0);
        let hi = ranges.iter().map(|r| r.1).max().unwrap_or(0);
        let threshold = (u32::from(hi - lo) / 4).max(24);
        ranges.iter().map(|&(l, h)| u32::from(h - l) < threshold).collect()
    }
}

fn convert_row(y_row: &[u8], uv_row: &[u8], out: &mut [u8]) {
    for (x, px) in out.chunks_exact_mut(3).enumerate() {
        let c = (i32::from(y_row[x]) - 16).max(0) * 298;
        let d = i32::from(uv_row[x & !1]) - 128;
        let e = i32::from(uv_row[x | 1]) - 128;
        px[0] = ((c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
        px[1] = ((c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
        px[2] = ((c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
    }
}
