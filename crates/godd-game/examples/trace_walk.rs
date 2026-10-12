//! Print the player's state tick by tick while walking north from the LIPS00 start.
use godd_game::{Action, Game, Input, Options, Vfs};

fn main() {
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let mut input = Input::default();
    for t in 0..90 {
        input.set(Action::Forward, t < 28);
        game.tick(&vfs, &input);
        input.end_tick();
        let p = game.session().unwrap().player;
        println!("{t:3} y={:8.1} z={:6.1} vz={:7.1} region={}", p.y, p.z, p.vz, p.region);
    }
}
