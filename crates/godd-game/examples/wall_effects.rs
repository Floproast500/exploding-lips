//! Wall effects in use, by effect index, with sample walls (debug aid):
//! `cargo run --example wall_effects LIPS00 [wall...]`
use godd_formats::xrb::{RecordExt, Section};
use godd_game::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let level = &args[1];
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{level}.XRB")).unwrap(), vfs.read(&format!("{level}.XPG")).unwrap()).unwrap();
    let walls: Vec<usize> = args[2..].iter().filter_map(|a| a.parse().ok()).collect();
    let mut counts = std::collections::BTreeMap::new();
    for (i, w) in lvl.orb.records(Section::Walls).iter().enumerate() {
        let e = w.le_i32(0xd0);
        if walls.contains(&i) {
            println!("wall {i}: effect {e} tag {} aux {}", w.le_u16(0xa6) as i16, w.le_i32(0xdc));
        }
        counts.entry(e).or_insert_with(Vec::new).push(i);
    }
    if walls.is_empty() {
        for (e, ws) in counts {
            println!("effect {e}: {} walls, e.g. {:?}", ws.len(), &ws[..ws.len().min(6)]);
        }
    }
}
