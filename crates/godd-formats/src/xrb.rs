//! Compiled orb (`.XRB`) and its pixel blob (`.XPG`), as read by ReadCompiledOrb (`FUN_004d5968`).
//!
//! Only the header, section table, textures and pics are decoded so far; see `re/formats/XRB.md`.

use crate::reader::{Le, name};
use crate::{Error, Result};

const MAGIC: &[u8] = b"O11\0";
const HEADER_LEN: usize = 0x68;
/// The engine refuses orbs compiled before this build stamp.
const MIN_BUILD: u32 = 0x0705_9901;
const SECTION_COUNT: usize = 15;
const DESCRIPTOR_LEN: usize = 20;
const TEX_RECORD_LEN: usize = 100;
const PIC_RECORD_LEN: usize = 40;
/// Blocks between ObjectStates and Views: 6x16 object indices, then 512 object indices.
const OBJECT_GROUP_TABLE_LEN: usize = 0x180;
const OBJECT_INDEX_TABLE_LEN: usize = 0x800;
/// Entry size of the per-orb sample table (count at header +0x40).
const SAMPLE_ENTRY_LEN: usize = 0x3c;
const TAIL_LEN: usize = 0x7c;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Texs,
    Pics,
    Points,
    Paths,
    Regions,
    Walls,
    Objects,
    ObjectStates,
    Views,
    Auxs,
    Sounds,
    Texts,
    Windows,
    Overlays,
    ActionNodes,
}

impl Section {
    pub const ALL: [Section; SECTION_COUNT] = [
        Section::Texs,
        Section::Pics,
        Section::Points,
        Section::Paths,
        Section::Regions,
        Section::Walls,
        Section::Objects,
        Section::ObjectStates,
        Section::Views,
        Section::Auxs,
        Section::Sounds,
        Section::Texts,
        Section::Windows,
        Section::Overlays,
        Section::ActionNodes,
    ];
}

#[derive(Debug, Clone, Copy)]
pub struct SectionDescriptor {
    pub max: u32,
    /// Number of live records.
    pub used: u32,
    /// Number of records stored in the file (live + spare).
    pub allocated: u32,
    pub record_len: u32,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub source_path: String,
    pub xpg_size: u32,
    pub build: u32,
}

/// A texture definition. Its record is followed in the file by `data`
/// (`n*12` bytes of entries plus `5 * (w << shift)` bytes).
#[derive(Debug, Clone)]
pub struct Tex {
    pub kind: u32,
    pub name: String,
    pub record: Vec<u8>,
    pub data: Vec<u8>,
}

/// An image stored in the `.XPG` blob. Pixels are column-major: column `x` occupies
/// `height` consecutive bytes starting at `x * height`.
#[derive(Debug, Clone)]
pub struct PicRef {
    pub kind: u32,
    pub name: String,
    /// Columns (record +0x10).
    pub width: u16,
    /// Rows per column (record +0x0e).
    pub height: u16,
    /// Byte offset into the `.XPG`, or `None` when the pic is not stored there.
    pub xpg_offset: Option<u32>,
    pub record: Vec<u8>,
}

impl PicRef {
    /// The pic's raw (column-major) pixels inside `xpg`, if stored there and fully in range.
    pub fn pixels<'a>(&self, xpg: &'a [u8]) -> Option<&'a [u8]> {
        let start = self.xpg_offset? as usize;
        xpg.get(start..start + self.width as usize * self.height as usize)
    }

    /// The pic converted to row-major order (`out[y * width + x]`).
    pub fn pixels_row_major(&self, xpg: &[u8]) -> Option<Vec<u8>> {
        let src = self.pixels(xpg)?;
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = vec![0u8; w * h];
        for x in 0..w {
            for y in 0..h {
                out[y * w + x] = src[x * h + y];
            }
        }
        Some(out)
    }
}

pub struct Orb {
    pub header: Header,
    /// The raw 0x68-byte file header (globals copied out by the loader).
    pub header_raw: Vec<u8>,
    pub sections: [SectionDescriptor; SECTION_COUNT],
    pub texs: Vec<Tex>,
    pub pics: Vec<PicRef>,
    /// Live records of every section from Points onwards, indexed by `Section as usize`
    /// (Texs/Pics entries are empty; use `texs`/`pics`). Field layouts: re/formats/XRB-records.md.
    pub records: Vec<Vec<Vec<u8>>>,
    /// Wall indices referenced by regions (`Regions` +0x110 = start, +0xbc = count).
    pub wall_list: Vec<i32>,
    /// Trailing 0x7c-byte block: level title, sky texture slot (+0x48), music sound (+0x78).
    pub tail: Vec<u8>,
    /// Body offset just past the tail (equals the body length for shipped files).
    pub end_offset: usize,
}

impl Orb {
    /// An i32 from the raw header (e.g. +0x4c start object, +0x50 initial tool mask).
    pub fn header_word(&self, offset: usize) -> i32 {
        self.header_raw.get(offset..offset + 4).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn records(&self, section: Section) -> &[Vec<u8>] {
        &self.records[section as usize]
    }
}

/// Little-endian field readers for raw records.
pub trait RecordExt {
    fn le_u16(&self, offset: usize) -> u16;
    fn le_i32(&self, offset: usize) -> i32;
    /// The integer part of a 16.16 fixed-point field.
    fn fixed_int(&self, offset: usize) -> i32 {
        self.le_i32(offset) >> 16
    }
    /// A 16.16 fixed-point field as a float.
    fn fixed(&self, offset: usize) -> f32 {
        self.le_i32(offset) as f32 / 65536.0
    }
}

impl RecordExt for [u8] {
    fn le_u16(&self, offset: usize) -> u16 {
        u16::from_le_bytes([self[offset], self[offset + 1]])
    }
    fn le_i32(&self, offset: usize) -> i32 {
        i32::from_le_bytes([self[offset], self[offset + 1], self[offset + 2], self[offset + 3]])
    }
}

impl Orb {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let magic = data.bytes_at(0, 4, "XRB magic")?;
        if magic != MAGIC {
            return Err(Error::BadMagic { expected: MAGIC, found: magic.to_vec() });
        }
        let header = Header {
            source_path: name(data.bytes_at(0x04, 0x1c, "XRB source path")?),
            xpg_size: data.u32_at(0x34, "XRB xpg size")?,
            build: data.u32_at(0x5c, "XRB build")?,
        };
        if header.build < MIN_BUILD {
            return Err(Error::UnsupportedBuild(header.build));
        }

        let body = data.get(HEADER_LEN..).ok_or(Error::Truncated { what: "XRB body", offset: HEADER_LEN })?;
        let mut sections = [SectionDescriptor { max: 0, used: 0, allocated: 0, record_len: 0 }; SECTION_COUNT];
        for (i, s) in sections.iter_mut().enumerate() {
            let o = i * DESCRIPTOR_LEN;
            *s = SectionDescriptor {
                max: body.u32_at(o, "descriptor")?,
                used: body.u32_at(o + 4, "descriptor")?,
                allocated: body.u32_at(o + 8, "descriptor")?,
                record_len: body.u32_at(o + 12, "descriptor")?,
            };
        }
        let mut offset = SECTION_COUNT * DESCRIPTOR_LEN;

        let (texs, next) = parse_texs(body, offset, &sections[Section::Texs as usize])?;
        offset = next;
        let (pics, next) = parse_pics(body, offset, &sections[Section::Pics as usize])?;
        offset = next;

        let mut records = vec![Vec::new(); SECTION_COUNT];
        let take = |records: &mut Vec<Vec<Vec<u8>>>, section: Section, offset: &mut usize| -> Result<()> {
            let d = &sections[section as usize];
            let len = d.record_len as usize;
            records[section as usize] = (0..d.used as usize)
                .map(|i| body.bytes_at(*offset + i * len, len, "section record").map(<[u8]>::to_vec))
                .collect::<Result<Vec<_>>>()?;
            *offset += d.allocated as usize * len;
            Ok(())
        };
        // File order follows the loader, not the descriptor order (re/formats/XRB-records.md).
        for section in [Section::Points, Section::Paths, Section::Regions, Section::Walls, Section::Objects, Section::ObjectStates] {
            take(&mut records, section, &mut offset)?;
        }
        offset += OBJECT_GROUP_TABLE_LEN + OBJECT_INDEX_TABLE_LEN;
        for section in [Section::Views, Section::Auxs, Section::Sounds] {
            take(&mut records, section, &mut offset)?;
        }
        offset += data.i32_at(0x40, "XRB sample count")?.max(0) as usize * SAMPLE_ENTRY_LEN;
        take(&mut records, Section::Texts, &mut offset)?;
        for text in &records[Section::Texts as usize] {
            if text.le_i32(0x3c) > 0 {
                offset += text.le_i32(0x50).max(0) as usize * 4 + text.le_i32(0x40).max(0) as usize;
            }
        }
        for section in [Section::Windows, Section::Overlays, Section::ActionNodes] {
            take(&mut records, section, &mut offset)?;
        }
        let wall_list_bytes = data.i32_at(0x24, "XRB wall list size")?.max(0) as usize;
        let wall_list_used = data.i32_at(0x20, "XRB wall list count")?.max(0) as usize;
        let wall_list = body
            .bytes_at(offset, wall_list_bytes, "region wall list")?
            .as_chunks::<4>()
            .0
            .iter()
            .take(wall_list_used)
            .map(|b| i32::from_le_bytes(*b))
            .collect();
        offset += wall_list_bytes;
        let tail = body.bytes_at(offset, TAIL_LEN, "XRB tail")?.to_vec();
        offset += TAIL_LEN;

        let header_raw = data[..HEADER_LEN].to_vec();
        Ok(Self { header, header_raw, sections, texs, pics, records, wall_list, tail, end_offset: offset })
    }
}

fn check_record_len(desc: &SectionDescriptor, expected: usize, section: &str) -> Result<()> {
    if desc.record_len as usize != expected {
        return Err(Error::Invalid(format!(
            "{section} record length {} (expected {expected})",
            desc.record_len
        )));
    }
    Ok(())
}

fn parse_texs(body: &[u8], start: usize, desc: &SectionDescriptor) -> Result<(Vec<Tex>, usize)> {
    check_record_len(desc, TEX_RECORD_LEN, "Texs")?;
    let mut data_offset = start + desc.allocated as usize * TEX_RECORD_LEN;
    let mut texs = Vec::with_capacity((desc.used as usize).min(body.len() / TEX_RECORD_LEN));
    for i in 0..desc.used as usize {
        let record = body.bytes_at(start + i * TEX_RECORD_LEN, TEX_RECORD_LEN, "Tex record")?;
        let entries = record.u16_at(0x1a, "Tex")? as usize;
        let width = record.u16_at(0x10, "Tex")? as usize;
        let shift = record.u16_at(0x14, "Tex")? as u32;
        let size = record.u32_at(0x50, "Tex")? as usize;
        let expected = entries * 12 + 5 * (width << (shift & 31));
        if size != expected {
            return Err(Error::Invalid(format!("Tex {i} data size {size}, expected {expected}")));
        }
        texs.push(Tex {
            kind: record.u32_at(0, "Tex")?,
            name: name(&record[4..12]),
            record: record.to_vec(),
            data: body.bytes_at(data_offset, size, "Tex data")?.to_vec(),
        });
        data_offset += size;
    }
    Ok((texs, data_offset))
}

fn parse_pics(body: &[u8], start: usize, desc: &SectionDescriptor) -> Result<(Vec<PicRef>, usize)> {
    check_record_len(desc, PIC_RECORD_LEN, "Pics")?;
    let mut pics = Vec::with_capacity((desc.used as usize).min(body.len() / PIC_RECORD_LEN));
    for i in 0..desc.used as usize {
        let record = body.bytes_at(start + i * PIC_RECORD_LEN, PIC_RECORD_LEN, "Pic record")?;
        let xpg_offset = record.i32_at(0x24, "Pic")?;
        pics.push(PicRef {
            kind: record.u32_at(0, "Pic")?,
            name: name(&record[4..12]),
            width: record.u16_at(0x10, "Pic")?,
            height: record.u16_at(0x0e, "Pic")?,
            xpg_offset: u32::try_from(xpg_offset).ok(),
            record: record.to_vec(),
        });
    }
    Ok((pics, start + desc.allocated as usize * PIC_RECORD_LEN))
}
