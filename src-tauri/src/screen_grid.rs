//! A numbered grid drawn onto a screenshot, so a model can say where something is by reading cell
//! numbers instead of estimating coordinates, which general vision models do badly. The backend
//! asks which cells hold a thing, zooms into those cells, and asks again (see the backend's
//! locate_marks.ts). Cells are numbered from 1, left to right and then top to bottom.
//!
//! Labels use a built-in 3×5 digit font, so no font files or text-rendering crates are needed.

use image::{Rgb, RgbImage};

const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];
/// Bright and rare in real content, so the lines read as the grid and not as part of the screen.
const LINE: Rgb<u8> = Rgb([255, 0, 200]);
const TAG: Rgb<u8> = Rgb([0, 0, 0]);
const INK: Rgb<u8> = Rgb([255, 230, 0]);

fn fill(image: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: Rgb<u8>) {
    let (iw, ih) = image.dimensions();
    for py in y.min(ih)..(y + h).min(ih) {
        for px in x.min(iw)..(x + w).min(iw) {
            image.put_pixel(px, py, color);
        }
    }
}

/// `number` at (x, y) in a black tag, each font pixel `scale` image pixels.
fn label(image: &mut RgbImage, x: u32, y: u32, number: usize, scale: u32) {
    let text = number.to_string();
    let pad = scale;
    let width = text.len() as u32 * 4 * scale - scale + pad * 2;
    let height = 5 * scale + pad * 2;
    fill(image, x, y, width, height, TAG);
    for (i, ch) in text.bytes().enumerate() {
        let glyph = DIGITS[(ch - b'0') as usize];
        let left = x + pad + i as u32 * 4 * scale;
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    fill(image, left + col * scale, y + pad + row as u32 * scale, scale, scale, INK);
                }
            }
        }
    }
}

/// Draws `cols` × `rows` cells over the whole image, each numbered at its top-left corner.
pub fn draw(image: &mut RgbImage, cols: u32, rows: u32) {
    let (w, h) = image.dimensions();
    let (cols, rows) = (cols.clamp(1, 12), rows.clamp(1, 12));
    let scale = (w.max(h) / 300).max(3);
    let thickness = (scale / 2).max(1);
    for c in 1..cols {
        fill(image, c * w / cols, 0, thickness, h, LINE);
    }
    for r in 1..rows {
        fill(image, 0, r * h / rows, w, thickness, LINE);
    }
    for r in 0..rows {
        for c in 0..cols {
            label(image, c * w / cols + thickness + 1, r * h / rows + thickness + 1, (r * cols + c + 1) as usize, scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_every_cell_inside_the_image() {
        let mut image = RgbImage::from_pixel(800, 600, Rgb([255, 255, 255]));
        draw(&mut image, 8, 6);
        // A grid line between the first two columns, and the first cell's tag in its corner.
        assert_eq!(*image.get_pixel(100, 300), LINE);
        assert_eq!(*image.get_pixel(3, 3), TAG);
        // The last cell (48) is labelled too: its tag starts just past the last lines.
        assert_eq!(*image.get_pixel(7 * 800 / 8 + 3, 5 * 600 / 6 + 3), TAG);
    }
}

