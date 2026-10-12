//! Parses every shipped data file in `original/`. Skipped when the game files are absent
//! (they are not redistributed with this repository).

use godd_formats::{pal::Palette, pic::Pic, xrb::Orb};
use std::path::{Path, PathBuf};

fn original_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    dir.is_dir().then_some(dir)
}

fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
        .collect();
    files.sort();
    files
}

#[test]
fn palette() {
    let Some(dir) = original_dir() else { return };
    let pal = Palette::parse(&std::fs::read(dir.join("PALETTE.PAL")).unwrap()).unwrap();
    assert_eq!(pal.colormaps.len(), 272);
    // Rows 0 and 16 start their 16-row blocks and are near-identity (250/256 entries).
    for row in [0, 16] {
        let identical = pal.colormaps[row].iter().enumerate().filter(|&(i, &c)| c as usize == i).count();
        assert!(identical >= 250, "row {row}: {identical}");
    }
}

#[test]
fn pics() {
    let Some(dir) = original_dir() else { return };
    let pics = files_with_ext(&dir.join("gfx"), "pic");
    assert!(!pics.is_empty());
    for path in pics {
        let pic = Pic::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(pic.pixels.len(), pic.width as usize * pic.height as usize, "{path:?}");
    }
}

#[test]
fn orbs() {
    let Some(dir) = original_dir() else { return };
    let orbs = files_with_ext(&dir, "xrb");
    assert_eq!(orbs.len(), 19);
    for path in orbs {
        let orb = Orb::parse(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let xpg = std::fs::read(path.with_extension("XPG")).unwrap();
        assert_eq!(orb.header.xpg_size as usize, xpg.len(), "{path:?}");
        for pic in &orb.pics {
            if pic.xpg_offset.is_some() {
                assert!(pic.pixels(&xpg).is_some(), "{path:?}: pic {} out of range", pic.name);
            }
        }
        assert_eq!(orb.pics[0].name, "DEF_Flor");
        let body_len = std::fs::metadata(&path).unwrap().len() as usize - 0x68;
        assert_eq!(orb.end_offset, body_len, "{path:?}: layout must cover the whole file");
    }
}

#[test]
fn title_bitmaps() {
    let Some(dir) = original_dir() else { return };
    for name in ["lips.bmp", "lips2.bmp"] {
        let bmp = godd_formats::bmp::Bmp::parse(&std::fs::read(dir.join(name)).unwrap()).unwrap();
        assert_eq!((bmp.width, bmp.height), (640, 480), "{name}");
    }
}

#[test]
fn level_scripts() {
    let Some(dir) = original_dir() else { return };
    for path in files_with_ext(&dir, "ini") {
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        let ini = godd_formats::ini::Ini::parse(&text);
        if path.file_name().unwrap().to_string_lossy().to_ascii_uppercase().starts_with("LIPS") {
            let startup = ini.section("startup").unwrap_or_else(|| panic!("{path:?}: no [startup]"));
            assert!(startup.get("orbname").is_some(), "{path:?}");
        }
        for section in &ini.sections {
            for e in section.all("ONTAG") {
                assert!(e.index.as_deref().is_some_and(|i| i.parse::<u32>().is_ok()), "{path:?}:{}", e.line);
                assert!(!godd_formats::ini::parse_calls(&e.value).is_empty(), "{path:?}:{}", e.line);
            }
        }
    }
}

#[test]
fn save_file_matches_log_and_round_trips() {
    let Some(dir) = original_dir() else { return };
    let bytes = std::fs::read(dir.join("LIPSTART.GAM")).unwrap();
    let save = godd_formats::gam::SaveFile::parse(&bytes).unwrap();
    // GODDRUN.LOG: idx=1, name=rew, orb=lipstart, states= L=100 MA=0 A=50 MO=100 E=0
    let slot = &save.slots[0];
    assert!(slot.is_active());
    assert_eq!((slot.name().as_str(), slot.orb().as_str()), ("rew", "lipstart"));
    assert_eq!(&slot.states()[1..=5], &[100, 0, 50, 100, 0]);
    assert!(save.slots[1..].iter().all(|s| !s.is_active()));
    assert_eq!(save.to_bytes(), bytes, "byte-identical rewrite");
}
