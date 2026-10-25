//! PP-OCR (DB) text detection post-processing, tiling and box merging.

use crate::types::Rect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DbParams {
    /// Probability above which a pixel is text.
    pub thresh: f32,
    /// Mean probability a region needs to count as a text box.
    pub box_thresh: f32,
    /// How far the shrunk text kernel is grown back (PP-OCR's unclip ratio).
    pub unclip_ratio: f32,
    /// Kernels narrower or shorter than this many pixels are noise.
    pub min_size: f32,
}

impl Default for DbParams {
    fn default() -> Self {
        Self { thresh: 0.3, box_thresh: 0.6, unclip_ratio: 1.5, min_size: 3.0 }
    }
}

/// An axis-aligned text box with fractional edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub score: f32,
}

impl TextBox {
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    fn center_y(&self) -> f32 {
        (self.y0 + self.y1) / 2.0
    }

    fn union(&self, other: &TextBox) -> TextBox {
        TextBox {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
            score: self.score.max(other.score),
        }
    }

    /// Whether two boxes are pieces of the same text line: they share most of
    /// the shorter one's height and touch or overlap horizontally. Tiles
    /// overlap, so a line cut by a tile edge shows up as overlapping pieces;
    /// separate columns of a table keep a gap and stay apart.
    pub fn same_line(&self, other: &TextBox) -> bool {
        let min_h = self.height().min(other.height());
        if min_h <= 0.0 {
            return false;
        }
        let v_overlap = self.y1.min(other.y1) - self.y0.max(other.y0);
        let h_gap = self.x0.max(other.x0) - self.x1.min(other.x1);
        v_overlap >= 0.6 * min_h && h_gap <= 0.1 * min_h
    }

    /// Integer pixel rectangle, clamped to a `width`×`height` frame.
    pub fn to_rect(&self, width: u32, height: u32) -> Option<Rect> {
        let x0 = self.x0.max(0.0).floor() as u32;
        let y0 = self.y0.max(0.0).floor() as u32;
        let x1 = self.x1.max(0.0).ceil() as u32;
        let y1 = self.y1.max(0.0).ceil() as u32;
        Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)).clamp_to(width, height)
    }
}

/// Finds text boxes in a DB probability map of `width`×`height`.
pub fn db_boxes(prob: &[f32], width: usize, height: usize, params: &DbParams) -> Vec<TextBox> {
    let n = width * height;
    if prob.len() < n {
        return Vec::new();
    }
    let mut seen = vec![false; n];
    let mut stack = Vec::new();
    let mut boxes = Vec::new();
    for start in 0..n {
        if seen[start] || prob[start] <= params.thresh {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0, usize::MAX, 0);
        let (mut sum, mut count) = (0.0f32, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % width, i / width);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
            sum += prob[i];
            count += 1;
            let mut visit = |j: usize| {
                if !seen[j] && prob[j] > params.thresh {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < width {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - width);
            }
            if y + 1 < height {
                visit(i + width);
            }
        }
        let w = (max_x - min_x + 1) as f32;
        let h = (max_y - min_y + 1) as f32;
        let score = sum / count as f32;
        if w.min(h) < params.min_size || score < params.box_thresh {
            continue;
        }
        let d = w * h * params.unclip_ratio / (2.0 * (w + h));
        boxes.push(TextBox {
            x0: min_x as f32 - d,
            y0: min_y as f32 - d,
            x1: (max_x + 1) as f32 + d,
            y1: (max_y + 1) as f32 + d,
            score,
        });
    }
    boxes
}

/// Covers a `width`×`height` area with tiles of at most `tile_w`×`tile_h`
/// that overlap by at least `overlap` pixels.
pub fn plan_tiles(width: u32, height: u32, tile_w: u32, tile_h: u32, overlap: u32) -> Vec<Rect> {
    let xs = axis_positions(width, tile_w, overlap);
    let ys = axis_positions(height, tile_h, overlap);
    ys.iter().flat_map(|&(y, h)| xs.iter().map(move |&(x, w)| Rect::new(x, y, w, h))).collect()
}

fn axis_positions(len: u32, tile: u32, overlap: u32) -> Vec<(u32, u32)> {
    if len == 0 || tile == 0 {
        return Vec::new();
    }
    if len <= tile {
        return vec![(0, len)];
    }
    let step = tile.saturating_sub(overlap).max(1);
    let count = (len - overlap.min(len)).div_ceil(step).max(2);
    let span = u64::from(len - tile);
    (0..count)
        .map(|i| {
            let pos = (span * u64::from(i) + u64::from(count - 1) / 2) / u64::from(count - 1);
            (pos as u32, tile)
        })
        .collect()
}

/// Joins pieces of the same line (from tile overlaps or split kernels).
pub fn merge_boxes(mut boxes: Vec<TextBox>) -> Vec<TextBox> {
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < boxes.len() {
            let mut j = i + 1;
            while j < boxes.len() {
                if boxes[i].same_line(&boxes[j]) {
                    let other = boxes.swap_remove(j);
                    boxes[i] = boxes[i].union(&other);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            return boxes;
        }
    }
}

/// Sorts boxes top to bottom, and left to right within a row.
pub fn reading_order(boxes: &mut Vec<TextBox>) {
    boxes.sort_by(|a, b| a.center_y().total_cmp(&b.center_y()));
    let mut rows: Vec<Vec<TextBox>> = Vec::new();
    for b in boxes.drain(..) {
        match rows.last_mut() {
            Some(row)
                if row.first().is_some_and(|first| {
                    (b.center_y() - first.center_y()).abs() < 0.5 * first.height().min(b.height())
                }) =>
            {
                row.push(b)
            }
            _ => rows.push(vec![b]),
        }
    }
    for mut row in rows {
        row.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        boxes.extend(row);
    }
}

/// Detector passes cost about this many input pixels each on top of their
/// size (input conversion, output copy, post-processing).
const PASS_OVERHEAD_PIXELS: u64 = 50_000;

/// Picks the detector input shape that covers a `width`×`height` area
/// soonest on `cores` parallel NPU cores: tiles run in rounds of `cores`,
/// and a round takes as long as one tile of that shape.
pub fn choose_shape(
    shapes: &[(u32, u32)],
    width: u32,
    height: u32,
    overlap: u32,
    cores: usize,
) -> Option<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, (w, h))| *w > overlap && *h > overlap)
        .min_by_key(|&(_, &(w, h))| {
            let tiles = plan_tiles(width, height, w, h, overlap).len();
            let rounds = tiles.div_ceil(cores.max(1)) as u64;
            (rounds * (u64::from(w) * u64::from(h) + PASS_OVERHEAD_PIXELS), tiles)
        })
        .map(|(i, _)| i)
}
