//! Jump pads (RFBOUNCE) of a level with their floors and launch speeds (debug aid):
//! `cargo run --example bounce_pads LIPS05`
use godd_game::{Game, Options, Vfs};

fn main() {
    let level = std::env::args().nth(1).unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(level), save: None }).unwrap();
    let s = game.session().unwrap();
    let w = &s.level.world;
    for r in 0..w.regions.len() {
        if let Some(v) = s.effects.bounce_speed(r) {
            let reg = &w.regions[r];
            println!("r{r}: floor {} ceil {} speed {v} bounds {:?}", reg.floor, reg.ceiling, reg.bounds);
        }
    }
}
