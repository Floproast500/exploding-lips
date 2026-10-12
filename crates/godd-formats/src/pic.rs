//! Standalone `.PIC` images (`PC2`), as loaded by GetPic (`FUN_00488146`).

use crate::reader::Le;
use crate::{Error, Result};

const MAGIC: &[u8] = b"PC2\0";
const HEADER_LEN: usize = 14;

/// An 8-bit palettised image. Like XRB pics, `.PIC` pixels are stored column-major: the
/// first header size is rows per column (height), the second the column count (width).
/// [`Pic::parse`] converts to row-major `pixels`.
pub struct Pic {
    pub width: u16,
    pub height: u16,
    /// Header fields after width/height whose meaning is not yet confirmed
    /// (likely origin x, origin y and flags).
    pub extra: [u16; 3],
    pub pixels: Vec<u8>,
}

impl Pic {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let magic = data.bytes_at(0, 4, "PIC magic")?;
        if magic != MAGIC {
            return Err(Error::BadMagic { expected: MAGIC, found: magic.to_vec() });
        }
        let height = data.u16_at(4, "PIC rows per column")?;
        let width = data.u16_at(6, "PIC columns")?;
        let extra = [data.u16_at(8, "PIC")?, data.u16_at(10, "PIC")?, data.u16_at(12, "PIC")?];
        let (w, h) = (width as usize, height as usize);
        let columns = data.bytes_at(HEADER_LEN, w * h, "PIC pixels")?;
        let mut pixels = vec![0u8; w * h];
        for x in 0..w {
            for y in 0..h {
                pixels[y * w + x] = columns[x * h + y];
            }
        }
        Ok(Self { width, height, extra, pixels })
    }
}
