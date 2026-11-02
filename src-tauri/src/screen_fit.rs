//! Snaps a box onto what is actually drawn in it, for marks Aloe draws over the screen. The
//! backend finds a thing by asking a vision model (locate_marks.ts), which gets close but rarely
//! tight: a highlight a little too low, a box that cuts a word in half. Here the box's crop, plus
//! a margin, is split into background and ink, and the box becomes the ink it touches: the text
//! lines it overlaps, and the words in them it overlaps, whole, even where they run past it.
//!
//! Only on a plain background (most of the crop's edge one colour, as with text on a page). On a
//! photo or a busy figure there is no telling ink from background, so the box is left alone.

use std::collections::HashMap;

use image::RgbImage;

/// A box in image pixels; `x1` and `y1` are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxBox {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// How far (summed over r, g, b) a pixel is from the background to count as ink.
const INK_DISTANCE: i32 = 72;
/// The least share of the crop's edge the background colour must cover.
const MIN_BACKGROUND_SHARE: f64 = 0.55;
/// Gaps between letters are joined, gaps between words are not: a fraction of the line's height.
const LETTER_GAP: f64 = 0.18;

/// The most common colour on the image's edge, if it is common enough to be the background.
fn background(image: &RgbImage) -> Option<[u8; 3]> {
    let (w, h) = image.dimensions();
    let mut edge = Vec::with_capacity((2 * (w + h)) as usize);
    for x in 0..w {
        edge.push(image.get_pixel(x, 0).0);
        edge.push(image.get_pixel(x, h - 1).0);
    }
    for y in 0..h {
        edge.push(image.get_pixel(0, y).0);
        edge.push(image.get_pixel(w - 1, y).0);
    }
    // Colours in buckets of 16 per channel, so anti-aliasing and noise fall together.
    let mut buckets: HashMap<u16, (usize, [u64; 3])> = HashMap::new();
    for p in &edge {
        let key = (u16::from(p[0] >> 4) << 8) | (u16::from(p[1] >> 4) << 4) | u16::from(p[2] >> 4);
        let bucket = buckets.entry(key).or_insert((0, [0; 3]));
        bucket.0 += 1;
        for c in 0..3 {
            bucket.1[c] += u64::from(p[c]);
        }
    }
    let (count, sum) = buckets.into_values().max_by_key(|(count, _)| *count)?;
    if (count as f64) < edge.len() as f64 * MIN_BACKGROUND_SHARE {
        return None;
    }
    Some(sum.map(|s| (s / count as u64) as u8))
}

/// Runs of `true`, joining runs at most `gap` apart: `(start, end)`, end exclusive.
fn runs(marks: &[bool], gap: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, &mark) in marks.iter().enumerate() {
        if !mark {
            continue;
        }
        match out.last_mut() {
            Some(last) if i - last.1 <= gap => last.1 = i + 1,
            _ => out.push((i, i + 1)),
        }
    }
    out
}

/// The runs that overlap `low..high`, as one span. A run reaching the image's edge goes on past
/// what was captured, so it isn't what the box was for: that side keeps the box's own edge.
fn span(all: Vec<(usize, usize)>, low: u32, high: u32, size: u32) -> Option<(u32, u32)> {
    let hit: Vec<_> = all.into_iter().filter(|&(a, b)| a < high as usize && b > low as usize).collect();
    let start = hit.iter().map(|r| r.0).min()? as u32;
    let end = hit.iter().map(|r| r.1).max()? as u32;
    Some((if start == 0 { low } else { start }, if end >= size { high } else { end }))
}

/// `inner` (a box inside `image`, which is it plus a margin) snapped onto the ink it touches, or
/// None when the background isn't plain or there is no ink in it.
pub fn fit(image: &RgbImage, inner: PxBox) -> Option<PxBox> {
    let (w, h) = image.dimensions();
    if w < 3 || h < 3 || inner.x1 <= inner.x0 || inner.y1 <= inner.y0 || inner.x1 > w || inner.y1 > h {
        return None;
    }
    let bg = background(image)?;
    let ink = |x: u32, y: u32| {
        let p = image.get_pixel(x, y).0;
        (0..3).map(|c| (i32::from(p[c]) - i32::from(bg[c])).abs()).sum::<i32>() > INK_DISTANCE
    };
    // Lines: rows with ink under the box, over the whole crop's height.
    let rows: Vec<bool> = (0..h).map(|y| (inner.x0..inner.x1).any(|x| ink(x, y))).collect();
    let lines = runs(&rows, 1);
    let line = lines.iter().filter(|&&(a, b)| a < inner.y1 as usize && b > inner.y0 as usize).map(|(a, b)| b - a).min()?;
    let (y0, y1) = span(lines, inner.y0, inner.y1, h)?;
    // Words: columns with ink in those lines, over the whole crop's width.
    let cols: Vec<bool> = (0..w).map(|x| (y0..y1).any(|y| ink(x, y))).collect();
    let gap = ((line as f64 * LETTER_GAP).round() as usize).max(2);
    let (x0, x1) = span(runs(&cols, gap), inner.x0, inner.x1, w)?;
    (x1 - x0 >= 3 && y1 - y0 >= 3).then_some(PxBox { x0, y0, x1, y1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn page() -> RgbImage {
        RgbImage::from_pixel(300, 120, Rgb([250, 250, 250]))
    }
    fn ink(image: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x, y, Rgb([20, 20, 30]));
            }
        }
    }
    /// A word as letters 6 px wide with 2 px between them, on the line y 50–70.
    fn word(image: &mut RgbImage, x: u32, letters: u32) {
        for i in 0..letters {
            ink(image, x + i * 8, 50, x + i * 8 + 6, 70);
        }
    }

    #[test]
    fn a_loose_box_tightens_and_a_cut_word_grows_whole() {
        let mut image = page();
        word(&mut image, 40, 5); // x 40–78
        word(&mut image, 100, 4); // x 100–130, a word gap of 22 px away
        // Too tall, and cutting the first word at x 60.
        let fitted = fit(&image, PxBox { x0: 30, y0: 35, x1: 60, y1: 90 }).unwrap();
        assert_eq!(fitted, PxBox { x0: 40, y0: 50, x1: 78, y1: 70 });
    }

    #[test]
    fn a_busy_background_or_no_ink_is_left_alone() {
        let mut noisy = page();
        for y in 0..120 {
            for x in 0..300 {
                noisy.put_pixel(x, y, Rgb([(x * 7 % 256) as u8, (y * 13 % 256) as u8, ((x + y) * 5 % 256) as u8]));
            }
        }
        assert_eq!(fit(&noisy, PxBox { x0: 30, y0: 35, x1: 60, y1: 90 }), None);
        assert_eq!(fit(&page(), PxBox { x0: 30, y0: 35, x1: 60, y1: 90 }), None);
    }

    #[test]
    fn ink_running_off_the_crop_keeps_the_boxs_own_edge() {
        let mut image = page();
        ink(&mut image, 0, 50, 300, 54); // a rule across the whole crop
        let fitted = fit(&image, PxBox { x0: 100, y0: 40, x1: 200, y1: 60 }).unwrap();
        assert_eq!(fitted, PxBox { x0: 100, y0: 50, x1: 200, y1: 54 });
    }
}
