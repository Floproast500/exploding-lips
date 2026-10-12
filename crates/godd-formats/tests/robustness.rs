//! Parsers must reject damaged input with an error, never panic: every shipped file is fed
//! truncated at many lengths and with corrupted bytes (a deterministic stand-in for fuzzing).

use godd_formats::{bmp::Bmp, gam::SaveFile, ini::Ini, pal::Palette, pic::Pic, xrb::Orb};
use std::path::{Path, PathBuf};

fn original_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    dir.is_dir().then_some(dir)
}

/// Deterministic xorshift.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn variants(data: &[u8], rng: &mut Rng) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for k in 0..24 {
        let len = if k < 12 { data.len() * k / 12 } else { (rng.next() as usize) % (data.len() + 1) };
        out.push(data[..len].to_vec());
    }
    for _ in 0..24 {
        let mut d = data.to_vec();
        for _ in 0..8 {
            if d.is_empty() {
                break;
            }
            let i = (rng.next() as usize) % d.len();
            d[i] = rng.next() as u8;
        }
        out.push(d);
    }
    out
}

type Parser = fn(&[u8]);

#[test]
fn damaged_files_do_not_panic() {
    let Some(dir) = original_dir() else { return };
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let parsers: [(&str, Parser); 6] = [
        ("LIPSTART.XRB", |d| drop(Orb::parse(d))),
        ("PALETTE.PAL", |d| drop(Palette::parse(d))),
        ("gfx/hand01.pic", |d| drop(Pic::parse(d))),
        ("lips.bmp", |d| drop(Bmp::parse(d))),
        ("LIPSTART.GAM", |d| drop(SaveFile::parse(d))),
        ("LIPS00.INI", |d| drop(Ini::parse(&String::from_utf8_lossy(d)))),
    ];
    for (file, parse) in parsers {
        let data = std::fs::read(dir.join(file)).unwrap();
        for v in variants(&data, &mut rng) {
            parse(&v);
        }
    }
}

#[test]
fn absurd_section_counts_are_rejected() {
    let Some(dir) = original_dir() else { return };
    let data = std::fs::read(dir.join("LIPSTART.XRB")).unwrap();
    for section in 0..15 {
        for field in [4usize, 8, 12] {
            let mut d = data.clone();
            let at = 0x68 + section * 20 + field;
            d[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(Orb::parse(&d).is_err() || field == 12, "section {section} field {field}");
        }
    }
}
