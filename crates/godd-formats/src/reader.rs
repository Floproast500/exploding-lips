use crate::{Error, Result};

/// Bounds-checked little-endian access into a byte slice.
pub(crate) trait Le {
    fn bytes_at(&self, offset: usize, len: usize, what: &'static str) -> Result<&[u8]>;

    fn u16_at(&self, offset: usize, what: &'static str) -> Result<u16> {
        let b = self.bytes_at(offset, 2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32_at(&self, offset: usize, what: &'static str) -> Result<u32> {
        let b = self.bytes_at(offset, 4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32_at(&self, offset: usize, what: &'static str) -> Result<i32> {
        Ok(self.u32_at(offset, what)? as i32)
    }
}

impl Le for [u8] {
    fn bytes_at(&self, offset: usize, len: usize, what: &'static str) -> Result<&[u8]> {
        offset
            .checked_add(len)
            .and_then(|end| self.get(offset..end))
            .ok_or(Error::Truncated { what, offset })
    }
}

/// Decode a fixed-size, NUL-padded name field.
pub(crate) fn name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| b as char).collect()
}
