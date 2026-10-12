//! Character save file (`LIPSTART.GAM`): 6 records of 0x4D4 bytes — record 0 a header kept
//! verbatim, records 1–5 the character slots (re/formats/GAM.md).
//!
//! The original writes the file in C text mode, so every 0x0A byte is stored as 0x0D 0x0A;
//! [`SaveFile::parse`] folds CR LF back and [`SaveFile::to_bytes`] expands it again.

use crate::{Error, Result};

pub const RECORD_LEN: usize = 0x4D4;
pub const SLOT_COUNT: usize = 5;
pub const INVENTORY_LEN: usize = 64;
const RECORDS: usize = SLOT_COUNT + 1;

pub const FLAG_ACTIVE: u32 = 1;
pub const FLAG_CREATED: u32 = 2;
pub const FLAG_SAVED: u32 = 4;

const NAME: (usize, usize) = (0x18, 20);
const ORB: (usize, usize) = (0x30, 24);
const POSITION: usize = 0x48;
const ANGLE: usize = 0x74;
const INVENTORY: usize = 0x78;
const ITEM_LEN: usize = 12;
const STATES: usize = 0x37C;
const TOOL: usize = 0x3C0;

/// One character slot. Unknown fields stay in `raw` and are written back unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub raw: Vec<u8>,
}

impl Slot {
    fn u32_at(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.raw[o..o + 4].try_into().unwrap())
    }
    fn set_u32(&mut self, o: usize, v: u32) {
        self.raw[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn string(&self, (o, len): (usize, usize)) -> String {
        let b = &self.raw[o..o + len];
        b[..b.iter().position(|&c| c == 0).unwrap_or(len)].iter().map(|&c| c as char).collect()
    }
    fn set_string(&mut self, (o, len): (usize, usize), s: &str) {
        let field = &mut self.raw[o..o + len];
        field.fill(0);
        for (d, c) in field.iter_mut().zip(s.bytes().take(len - 1)) {
            *d = c;
        }
    }

    pub fn empty(index: u32) -> Self {
        let mut slot = Self { raw: vec![0; RECORD_LEN] };
        slot.set_u32(4, index);
        slot
    }

    pub fn flags(&self) -> u32 {
        self.u32_at(0)
    }
    pub fn set_flags(&mut self, flags: u32) {
        self.set_u32(0, flags);
    }
    pub fn is_active(&self) -> bool {
        self.flags() & FLAG_ACTIVE != 0
    }
    pub fn is_saved(&self) -> bool {
        self.flags() & FLAG_SAVED != 0
    }
    pub fn index(&self) -> u32 {
        self.u32_at(4)
    }
    pub fn name(&self) -> String {
        self.string(NAME)
    }
    pub fn set_name(&mut self, name: &str) {
        self.set_string(NAME, name);
    }
    pub fn orb(&self) -> String {
        self.string(ORB)
    }
    pub fn set_orb(&mut self, orb: &str) {
        self.set_string(ORB, orb);
    }
    /// Position (16.16 values as floats).
    pub fn position(&self) -> (f32, f32, f32) {
        let f = |k: usize| self.u32_at(POSITION + 4 * k) as i32 as f32 / 65536.0;
        (f(0), f(1), f(2))
    }
    pub fn set_position(&mut self, (x, y, z): (f32, f32, f32)) {
        for (k, v) in [x, y, z].into_iter().enumerate() {
            self.set_u32(POSITION + 4 * k, (v * 65536.0) as i32 as u32);
        }
    }
    /// Facing angle, 2048 units per turn.
    pub fn angle(&self) -> u16 {
        u16::from_le_bytes([self.raw[ANGLE], self.raw[ANGLE + 1]])
    }
    pub fn set_angle(&mut self, angle: u16) {
        self.raw[ANGLE..ANGLE + 2].copy_from_slice(&angle.to_le_bytes());
    }
    pub fn inventory(&self) -> Vec<String> {
        (0..INVENTORY_LEN).map(|i| self.string((INVENTORY + i * ITEM_LEN, ITEM_LEN))).filter(|s| !s.is_empty()).collect()
    }
    pub fn set_inventory(&mut self, items: &[String]) {
        for i in 0..INVENTORY_LEN {
            self.set_string((INVENTORY + i * ITEM_LEN, ITEM_LEN), items.get(i).map_or("", String::as_str));
        }
    }
    pub fn states(&self) -> [i32; 16] {
        std::array::from_fn(|i| self.u32_at(STATES + 4 * i) as i32)
    }
    pub fn set_states(&mut self, states: &[i32; 16]) {
        for (i, v) in states.iter().enumerate() {
            self.set_u32(STATES + 4 * i, *v as u32);
        }
    }
    pub fn tool(&self) -> u32 {
        self.u32_at(TOOL)
    }
    pub fn set_tool(&mut self, tool: u32) {
        self.set_u32(TOOL, tool);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveFile {
    /// Record 0, preserved verbatim.
    pub header: Vec<u8>,
    /// Slots 1–5 (`slots[0]` is slot 1).
    pub slots: Vec<Slot>,
}

impl Default for SaveFile {
    fn default() -> Self {
        Self { header: vec![0; RECORD_LEN], slots: (1..=SLOT_COUNT as u32).map(Slot::empty).collect() }
    }
}

impl SaveFile {
    pub fn parse(file: &[u8]) -> Result<Self> {
        let data = fold_crlf(file);
        if data.len() < RECORDS * RECORD_LEN {
            return Err(Error::Truncated { what: "GAM records", offset: data.len() });
        }
        let record = |i: usize| data[i * RECORD_LEN..(i + 1) * RECORD_LEN].to_vec();
        let slots = (1..RECORDS)
            .map(|i| {
                let slot = Slot { raw: record(i) };
                // Records with an out-of-range index are treated as empty (as the loader does).
                if (1..=SLOT_COUNT as u32).contains(&slot.index()) { slot } else { Slot::empty(i as u32) }
            })
            .collect();
        Ok(Self { header: record(0), slots })
    }

    /// Bytes as the original writes them (text mode: LF → CR LF).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut logical = self.header.clone();
        for slot in &self.slots {
            logical.extend_from_slice(&slot.raw);
        }
        expand_lf(&logical)
    }

    /// First slot that is not active (GAMENEW), as a 0-based index.
    pub fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.is_active())
    }
}

/// Undo C text-mode output: CR LF → LF.
pub fn fold_crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x0D && data.get(i + 1) == Some(&0x0A) {
            out.push(0x0A);
            i += 2;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// C text-mode output: LF → CR LF.
pub fn expand_lf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    for &b in data {
        if b == 0x0A {
            out.push(0x0D);
        }
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_fields_round_trip_with_newline_bytes() {
        let mut save = SaveFile::default();
        let slot = &mut save.slots[0];
        slot.set_flags(FLAG_ACTIVE | FLAG_CREATED);
        slot.set_name("tester");
        slot.set_orb("lips00");
        // 10 (0x0A) in a state value must survive the CR LF translation.
        let mut states = [0; 16];
        states[1] = 10;
        states[4] = 0x0A0A;
        slot.set_states(&states);
        slot.set_inventory(&["KEY2".to_string()]);
        let bytes = save.to_bytes();
        assert!(bytes.len() > RECORDS * RECORD_LEN, "LF bytes expanded");
        let back = SaveFile::parse(&bytes).unwrap();
        assert_eq!(back, save);
        assert_eq!(back.slots[0].name(), "tester");
        assert_eq!(back.slots[0].states()[4], 0x0A0A);
        assert_eq!(back.free_slot(), Some(1));
    }
}
