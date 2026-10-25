//! Luma fingerprints for telling whether the screen is still changing.

use crate::types::Rect;

/// Mean luma of a `cols`×`rows` grid over a `width`×`height` luma plane,
/// sampling every fourth pixel in each direction.
pub fn luma_grid(
    y: &[u8],
    stride: usize,
    width: usize,
    height: usize,
    cols: usize,
    rows: usize,
) -> Vec<u8> {
    if cols == 0
        || rows == 0
        || width < cols
        || height < rows
        || stride < width
        || y.len() < stride * (height - 1) + width
    {
        return Vec::new();
    }
    let mut grid = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        let (y0, y1) = (r * height / rows, (r + 1) * height / rows);
        for c in 0..cols {
            let (x0, x1) = (c * width / cols, (c + 1) * width / cols);
            let (mut sum, mut count) = (0u64, 0u64);
            for yy in (y0..y1).step_by(4) {
                let row = &y[yy * stride..yy * stride + width];
                for &l in row[x0..x1].iter().step_by(4) {
                    sum += u64::from(l);
                    count += 1;
                }
            }
            grid.push(sum.checked_div(count).unwrap_or(0) as u8);
        }
    }
    grid
}

/// Whether any cell's mean luma moved by more than `threshold`. Grids of
/// different sizes always count as changed.
pub fn grid_changed(a: &[u8], b: &[u8], threshold: u8) -> bool {
    a.len() != b.len() || a.iter().zip(b).any(|(&x, &y)| x.abs_diff(y) > threshold)
}

/// A hash of every pixel of `rect` in a luma plane with rows `stride`
/// bytes apart: equal only when that part of the picture is the same, down
/// to one pixel.
pub fn fingerprint(y: &[u8], stride: usize, rect: Rect) -> u64 {
    const K: u64 = 0x9e37_79b9_7f4a_7c15;
    let (x0, x1) = (rect.x as usize, rect.right() as usize);
    let mut hash = (u64::from(rect.x) << 48)
        ^ (u64::from(rect.y) << 32)
        ^ (u64::from(rect.width) << 16)
        ^ u64::from(rect.height);
    for row in y.chunks(stride.max(1)).skip(rect.y as usize).take(rect.height as usize) {
        let row = &row[x0.min(row.len())..x1.min(row.len())];
        let mut words = row.chunks_exact(8);
        for word in &mut words {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(word);
            hash = (hash ^ u64::from_le_bytes(bytes)).wrapping_mul(K).rotate_left(29);
        }
        for &b in words.remainder() {
            hash = (hash ^ u64::from(b)).wrapping_mul(K).rotate_left(29);
        }
    }
    hash
}
