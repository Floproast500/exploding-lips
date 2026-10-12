//! Uncompressed 8-bit Windows BMP files (title screens such as `lips.bmp`).

use crate::reader::Le;
use crate::{Error, Result};

pub struct Bmp {
    pub width: usize,
    pub height: usize,
    pub palette: [[u8; 3]; 256],
    /// Top-down rows of palette indices.
    pub pixels: Vec<u8>,
}

impl Bmp {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let magic = data.bytes_at(0, 2, "BMP magic")?;
        if magic != b"BM" {
            return Err(Error::BadMagic { expected: b"BM", found: magic.to_vec() });
        }
        let pixel_offset = data.u32_at(10, "BMP")? as usize;
        let header_len = data.u32_at(14, "BMP")? as usize;
        let width = data.i32_at(18, "BMP")?;
        let height = data.i32_at(22, "BMP")?;
        let bpp = data.u16_at(28, "BMP")?;
        let compression = data.u32_at(30, "BMP")?;
        if bpp != 8 || compression != 0 || width <= 0 || height == 0 {
            return Err(Error::Invalid(format!("unsupported BMP: {width}x{height} {bpp}bpp compression {compression}")));
        }
        let colors = match data.u32_at(46, "BMP")? {
            0 => 256,
            n => (n as usize).min(256),
        };
        let mut palette = [[0u8; 3]; 256];
        let table = data.bytes_at(14 + header_len, colors * 4, "BMP palette")?;
        for (dst, bgra) in palette.iter_mut().zip(table.as_chunks::<4>().0) {
            *dst = [bgra[2], bgra[1], bgra[0]];
        }
        let (width, bottom_up) = (width as usize, height > 0);
        let height = height.unsigned_abs() as usize;
        let stride = width.div_ceil(4) * 4;
        let mut pixels = Vec::with_capacity(width * height);
        for row in 0..height {
            let src_row = if bottom_up { height - 1 - row } else { row };
            pixels.extend_from_slice(data.bytes_at(pixel_offset + src_row * stride, width, "BMP pixels")?);
        }
        Ok(Self { width, height, palette, pixels })
    }
}
