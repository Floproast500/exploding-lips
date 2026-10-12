//! `PALETTE.PAL` (`PL3`): a 6-bit VGA palette followed by colormap (fade) tables.

use crate::reader::Le;
use crate::{Error, Result};

const MAGIC: &[u8] = b"PL3\0";
const COLORS_OFFSET: usize = 0x08;
const COLORMAP_OFFSET: usize = 0x328;

pub struct Palette {
    /// 256 colours expanded to 8-bit RGB.
    pub rgb: [[u8; 3]; 256],
    /// 272 colormap rows of 256 entries each. Rows 0 and 16 are near-identity and the rows in
    /// between darken progressively, suggesting 17 blocks of 16 shade levels (unconfirmed).
    pub colormaps: Vec<[u8; 256]>,
}

impl Palette {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let magic = data.bytes_at(0, 4, "PAL magic")?;
        if magic != MAGIC {
            return Err(Error::BadMagic { expected: MAGIC, found: magic.to_vec() });
        }
        let six_bit = data.bytes_at(COLORS_OFFSET, 768, "PAL colours")?;
        let mut rgb = [[0u8; 3]; 256];
        for (dst, src) in rgb.iter_mut().zip(six_bit.as_chunks::<3>().0) {
            for (d, &s) in dst.iter_mut().zip(src) {
                *d = expand_6bit(s);
            }
        }
        let colormaps = data
            .get(COLORMAP_OFFSET..)
            .unwrap_or_default()
            .as_chunks::<256>()
            .0
            .to_vec();
        Ok(Self { rgb, colormaps })
    }
}

fn expand_6bit(v: u8) -> u8 {
    ((v.min(63) as u16 * 255) / 63) as u8
}
