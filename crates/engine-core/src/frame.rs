/// A frame buffer of palette indices (`pixels`), optionally with a true-colour layer (`rgb`).
///
/// In true-colour mode the 3D renderer writes `rgb` directly with continuous lighting; UI
/// drawing goes through [`Frame::put`], which keeps both layers in sync, so presenters can use
/// `rgb` whenever it is present and the indices otherwise.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    /// 0x00RRGGBB per pixel when true-colour output is enabled.
    pub rgb: Option<Vec<u32>>,
    /// Palette used to keep `rgb` in sync with index writes.
    pub palette: [[u8; 3]; 256],
    /// Displayed height of one pixel relative to its width (1.2 for 320x200 on a 4:3 screen).
    pub pixel_aspect: f32,
    /// When tracked: 255 where something was drawn since the last clear, 0 elsewhere. Lets a UI
    /// layer be composited over another renderer's view (index 0 is a real colour, not "empty").
    pub coverage: Option<Vec<u8>>,
    /// When recording: UI drawing (text, boxes, menu images) is collected here instead of
    /// rasterised, for a GPU to draw at full display resolution over the pixel layer.
    pub ui_list: Option<Vec<UiCmd>>,
}

/// A recorded UI drawing command, in frame pixel coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum UiCmd {
    Rect { x: i32, y: i32, w: u32, h: u32, color: u8 },
    /// An 8x8 bitmap glyph (one byte per row, bit 0 leftmost) enlarged `scale` times.
    Glyph { x: i32, y: i32, scale: u32, color: u8, rows: [u8; 8] },
    /// Palette indices (0 transparent) enlarged `scale` times.
    Image { x: i32, y: i32, scale: u32, width: u32, height: u32, pixels: Vec<u8> },
}

impl Frame {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![0; width * height], rgb: None, palette: [[0; 3]; 256], pixel_aspect: 1.0, coverage: None, ui_list: None }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.pixels = vec![0; width * height];
            if self.rgb.is_some() {
                self.rgb = Some(vec![0; width * height]);
            }
            if self.coverage.is_some() {
                self.coverage = Some(vec![0; width * height]);
            }
        }
    }

    /// Start or stop recording which pixels are drawn (see [`Frame::coverage`]).
    pub fn track_coverage(&mut self, enabled: bool) {
        self.coverage = enabled.then(|| vec![0; self.width * self.height]);
    }

    /// Start or stop recording UI drawing into [`Frame::ui_list`].
    pub fn record_ui(&mut self, enabled: bool) {
        if enabled != self.ui_list.is_some() {
            self.ui_list = enabled.then(Vec::new);
        }
    }

    pub fn set_true_color(&mut self, enabled: bool) {
        match (enabled, self.rgb.is_some()) {
            (true, false) => self.rgb = Some(vec![0; self.width * self.height]),
            (false, true) => self.rgb = None,
            _ => {}
        }
    }

    /// Width / height of the frame as displayed.
    pub fn display_aspect(&self) -> f32 {
        self.width as f32 / (self.height as f32 * self.pixel_aspect)
    }

    pub fn clear(&mut self, index: u8) {
        self.pixels.fill(index);
        let rgb = pack(self.palette[index as usize]);
        if let Some(buf) = &mut self.rgb {
            buf.fill(rgb);
        }
        if let Some(c) = &mut self.coverage {
            c.fill(0);
        }
        if let Some(l) = &mut self.ui_list {
            l.clear();
        }
    }

    /// Write a palette index (and its colour in true-colour mode).
    #[inline]
    pub fn put(&mut self, offset: usize, index: u8) {
        self.pixels[offset] = index;
        if let Some(buf) = &mut self.rgb {
            buf[offset] = pack(self.palette[index as usize]);
        }
        if let Some(c) = &mut self.coverage {
            c[offset] = 255;
        }
    }

    /// Write a colour directly (true-colour mode only).
    #[inline]
    pub fn put_rgb(&mut self, offset: usize, rgb: [u8; 3]) {
        if let Some(buf) = &mut self.rgb {
            buf[offset] = pack(rgb);
        }
        if let Some(c) = &mut self.coverage {
            c[offset] = 255;
        }
    }

    /// Expand to RGBA8 (used by front ends without a palette shader).
    pub fn to_rgba(&self, palette: &[[u8; 3]; 256], out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.pixels.len() * 4);
        match &self.rgb {
            Some(buf) => {
                for &c in buf {
                    out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
                }
            }
            None => {
                for &i in &self.pixels {
                    let [r, g, b] = palette[i as usize];
                    out.extend_from_slice(&[r, g, b, 255]);
                }
            }
        }
    }
}

#[inline]
pub fn pack([r, g, b]: [u8; 3]) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_marks_drawn_pixels_and_clears() {
        let mut f = Frame::new(4, 2);
        f.track_coverage(true);
        f.clear(0);
        f.put(1, 0); // index 0 drawn on purpose still counts as covered
        f.put(6, 9);
        assert_eq!(f.coverage.as_deref(), Some(&[0, 255, 0, 0, 0, 0, 255, 0][..]));
        f.clear(3);
        assert!(f.coverage.as_ref().unwrap().iter().all(|&c| c == 0));
        f.resize(2, 2);
        assert_eq!(f.coverage.as_ref().map(Vec::len), Some(4));
        f.track_coverage(false);
        assert!(f.coverage.is_none());
    }
}
