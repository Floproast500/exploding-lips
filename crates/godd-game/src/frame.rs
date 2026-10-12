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
}

impl Frame {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![0; width * height], rgb: None, palette: [[0; 3]; 256], pixel_aspect: 1.0 }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.pixels = vec![0; width * height];
            if self.rgb.is_some() {
                self.rgb = Some(vec![0; width * height]);
            }
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
    }

    /// Write a palette index (and its colour in true-colour mode).
    #[inline]
    pub fn put(&mut self, offset: usize, index: u8) {
        self.pixels[offset] = index;
        if let Some(buf) = &mut self.rgb {
            buf[offset] = pack(self.palette[index as usize]);
        }
    }

    /// Write a colour directly (true-colour mode only).
    #[inline]
    pub fn put_rgb(&mut self, offset: usize, rgb: [u8; 3]) {
        if let Some(buf) = &mut self.rgb {
            buf[offset] = pack(rgb);
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
