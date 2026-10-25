//! PP-OCR (CTC) text recognition: dictionary, decoding and long-line splitting.

/// Reads a PP-OCR character dictionary: one character per line.
pub fn load_dict(text: &str) -> Vec<String> {
    text.lines().map(|l| l.trim_end_matches('\r').to_string()).filter(|l| !l.is_empty()).collect()
}

/// Greedy CTC decoding of `steps`×`classes` probabilities.
///
/// Class 0 is the CTC blank and class `i` is `dict[i - 1]`. PP-OCR models
/// trained with `use_space_char` have one class more than the dictionary,
/// the space, as the last class.
pub fn ctc_decode(probs: &[f32], steps: usize, classes: usize, dict: &[String]) -> (String, f32) {
    let mut text = String::new();
    let (mut score_sum, mut kept) = (0.0f32, 0usize);
    let mut prev = 0usize;
    for step in probs.chunks_exact(classes).take(steps) {
        let (best, p) = step
            .iter()
            .enumerate()
            .fold((0usize, f32::MIN), |acc, (i, &p)| if p > acc.1 { (i, p) } else { acc });
        if best != 0 && best != prev {
            if let Some(ch) = dict.get(best - 1) {
                text.push_str(ch);
            } else if best == dict.len() + 1 {
                text.push(' ');
            }
            score_sum += p;
            kept += 1;
        }
        prev = best;
    }
    let score = if kept == 0 { 0.0 } else { score_sum / kept as f32 };
    (text.trim().to_string(), score)
}

/// A horizontal slice of a text line, in columns of the line's crop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    /// The gap before this segment is wide enough to be a word space.
    pub space_before: bool,
}

/// Splits a line wider than the recognizer accepts into segments of at most
/// `max_width` columns, cutting at blank columns where possible so no
/// character is split. Gaps of at least `space_min` columns are word spaces.
pub fn split_line(blank: &[bool], max_width: usize, space_min: usize) -> Vec<Segment> {
    let n = blank.len();
    let max_width = max_width.max(1);
    let mut segments = Vec::new();
    let mut start = 0;
    let mut space_before = false;
    loop {
        while start < n && blank[start] {
            start += 1;
        }
        if start >= n {
            return segments;
        }
        if n - start <= max_width {
            segments.push(Segment { start, end: n, space_before });
            return segments;
        }
        let limit = start + max_width;
        let earliest = start + max_width / 2;
        match (earliest..=limit).rev().find(|&c| blank[c]) {
            Some(c) => {
                let mut gap_start = c;
                while gap_start > start && blank[gap_start - 1] {
                    gap_start -= 1;
                }
                let mut gap_end = c;
                while gap_end < n && blank[gap_end] {
                    gap_end += 1;
                }
                segments.push(Segment { start, end: gap_start, space_before });
                space_before = gap_end - gap_start >= space_min;
                start = gap_end;
            }
            None => {
                segments.push(Segment { start, end: limit, space_before });
                space_before = false;
                start = limit;
            }
        }
    }
}

/// The narrowest recognizer input width that fits `needed` columns, or the
/// widest one if none does. `widths` must be sorted ascending.
pub fn pick_width(widths: &[u32], needed: u32) -> Option<u32> {
    widths.iter().copied().find(|&w| w >= needed).or_else(|| widths.last().copied())
}
