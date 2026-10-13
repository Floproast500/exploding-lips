//! Region effects in a range (debug aid): `cargo run --example region_kinds LIPS05 480 600`
use godd_game::{Game, Options, Vfs};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (a, b): (usize, usize) = (args[2].parse().unwrap(), args[3].parse().unwrap());
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(args[1].clone()), save: None }).unwrap();
    let s = game.session().unwrap();
    for r in a..=b.min(s.level.world.regions.len() - 1) {
        if let Some(k) = s.effects.region_kind(r) {
            let reg = &s.level.world.regions[r];
            println!("r{r}: {k} floor {} ceil {} bounds {:?}", reg.floor, reg.ceiling, reg.bounds);
        }
    }
}
