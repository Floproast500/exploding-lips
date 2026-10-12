//! Save slots (modern addition next to the original's character slots): a snapshot of the
//! level, position, states and inventory, a label, and a small thumbnail of the view.
//!
//! File format (text):
//! ```text
//! LIPSSAVE 1
//! <label>
//! <snapshot>
//! <thumbnail: THUMB_W*THUMB_H palette indices in hex>
//! ```

pub const SLOTS: usize = 8;
pub const THUMB_W: usize = 96;
pub const THUMB_H: usize = 60;

#[derive(Debug, Clone, PartialEq)]
pub struct SaveSlot {
    pub label: String,
    /// Same format as the quicksave: `LEVEL x y z angle states... | inventory...`.
    pub snapshot: String,
    /// `THUMB_W x THUMB_H` palette indices.
    pub thumb: Vec<u8>,
}

impl SaveSlot {
    pub fn to_text(&self) -> String {
        let hex: String = self.thumb.iter().map(|b| format!("{b:02x}")).collect();
        format!("LIPSSAVE 1\n{}\n{}\n{hex}\n", self.label.replace('\n', " "), self.snapshot.replace('\n', " "))
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != "LIPSSAVE 1" {
            return None;
        }
        let label = lines.next()?.to_string();
        let snapshot = lines.next()?.to_string();
        let hex = lines.next().unwrap_or("");
        let thumb: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()).collect();
        let thumb = if thumb.len() == THUMB_W * THUMB_H { thumb } else { vec![0; THUMB_W * THUMB_H] };
        Some(Self { label, snapshot, thumb })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_damage() {
        let s = SaveSlot { label: "LIPS03  Mortuary II".into(), snapshot: "LIPS03 1 2 3 0.5 | KEY02".into(), thumb: (0..THUMB_W * THUMB_H).map(|i| i as u8).collect() };
        assert_eq!(SaveSlot::parse(&s.to_text()), Some(s.clone()));
        assert!(SaveSlot::parse("garbage").is_none());
        let short = SaveSlot::parse("LIPSSAVE 1\nlabel\nsnap\nabc").unwrap();
        assert_eq!(short.thumb.len(), THUMB_W * THUMB_H, "a damaged thumbnail is replaced");
    }
}
