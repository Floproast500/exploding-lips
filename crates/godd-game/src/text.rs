//! Bitmap text and simple UI primitives drawn straight into the palettised frame.

use crate::Frame;
use engine_core::UiCmd;
use font8x8::UnicodeFonts;

pub const GLYPH: usize = 8;

/// CP437 control-range symbols the HUD uses (IBM BIOS 8x8 shapes), stored LSB-left like
/// font8x8: heart (0x03), diamond (0x04), spade (0x06).
fn cp437_symbol(ch: char) -> Option<[u8; 8]> {
    let msb_left: [u8; 8] = match ch as u32 {
        0x03 => [0x6C, 0xFE, 0xFE, 0xFE, 0x7C, 0x38, 0x10, 0x00],
        0x04 => [0x10, 0x38, 0x7C, 0xFE, 0x7C, 0x38, 0x10, 0x00],
        0x06 => [0x10, 0x38, 0x7C, 0xFE, 0xFE, 0x7C, 0x10, 0x38],
        _ => return None,
    };
    Some(msb_left.map(u8::reverse_bits))
}

pub const HEART: char = '\u{3}';
pub const DIAMOND: char = '\u{4}';
pub const SPADE: char = '\u{6}';

/// Draw `text` with its top-left corner at (x, y), each glyph pixel scaled by `scale`.
/// Returns the width drawn in pixels.
pub fn draw_text(frame: &mut Frame, x: i32, y: i32, scale: usize, color: u8, text: &str) -> usize {
    let mut cx = x;
    for ch in text.chars() {
        if let Some(glyph) = cp437_symbol(ch).or_else(|| font8x8::BASIC_FONTS.get(ch)).or_else(|| font8x8::LATIN_FONTS.get(ch)) {
            if let Some(list) = &mut frame.ui_list {
                if glyph != [0; 8] {
                    list.push(UiCmd::Glyph { x: cx, y, scale: scale as u32, color, rows: glyph });
                }
                cx += (GLYPH * scale) as i32;
                continue;
            }
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..GLYPH {
                    if bits & (1 << col) != 0 {
                        fill_rect(frame, cx + (col * scale) as i32, y + (row * scale) as i32, scale, scale, color);
                    }
                }
            }
        }
        cx += (GLYPH * scale) as i32;
    }
    (cx - x) as usize
}

/// Text with a one-pixel drop shadow for legibility over the 3D view.
pub fn draw_text_shadow(frame: &mut Frame, x: i32, y: i32, scale: usize, color: u8, shadow: u8, text: &str) -> usize {
    draw_text(frame, x + scale as i32, y + scale as i32, scale, shadow, text);
    draw_text(frame, x, y, scale, color, text)
}

pub fn text_width(text: &str, scale: usize) -> usize {
    text.chars().count() * GLYPH * scale
}

pub fn fill_rect(frame: &mut Frame, x: i32, y: i32, w: usize, h: usize, color: u8) {
    if let Some(list) = &mut frame.ui_list {
        list.push(UiCmd::Rect { x, y, w: w as u32, h: h as u32, color });
        return;
    }
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = ((x + w as i32).max(0) as usize).min(frame.width);
    let y1 = ((y + h as i32).max(0) as usize).min(frame.height);
    for row in y0..y1 {
        for x in x0..x1 {
            frame.put(row * frame.width + x, color);
        }
    }
}

pub fn frame_rect(frame: &mut Frame, x: i32, y: i32, w: usize, h: usize, color: u8) {
    fill_rect(frame, x, y, w, 1, color);
    fill_rect(frame, x, y + h as i32 - 1, w, 1, color);
    fill_rect(frame, x, y, 1, h, color);
    fill_rect(frame, x + w as i32 - 1, y, 1, h, color);
}

/// Draw an image with index-0 transparency at a whole scale.
pub fn blit(frame: &mut Frame, width: usize, height: usize, pixels: &[u8], x: i32, y: i32, s: usize) {
    if let Some(list) = &mut frame.ui_list {
        list.push(UiCmd::Image { x, y, scale: s as u32, width: width as u32, height: height as u32, pixels: pixels[..width * height].to_vec() });
        return;
    }
    for v in 0..height * s {
        for u in 0..width * s {
            let texel = pixels[(v / s) * width + u / s];
            let (px, py) = (x + u as i32, y + v as i32);
            if texel != 0 && px >= 0 && py >= 0 && (px as usize) < frame.width && (py as usize) < frame.height {
                frame.put(py as usize * frame.width + px as usize, texel);
            }
        }
    }
}

/// Index of the palette colour closest to `rgb`.
pub fn nearest_color(palette: &[[u8; 3]; 256], rgb: [u8; 3]) -> u8 {
    let dist = |c: &[u8; 3]| -> i32 { (0..3).map(|i| (c[i] as i32 - rgb[i] as i32).pow(2)).sum() };
    (0..256).min_by_key(|&i| dist(&palette[i])).unwrap_or(0) as u8
}
