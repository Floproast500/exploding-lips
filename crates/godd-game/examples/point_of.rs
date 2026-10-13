//! Where a named Point is, and its region (debug aid): `cargo run --example point_of LIPS05 SMART1`
use godd_game::{Game, Options, Vfs};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(args[1].clone()), save: None }).unwrap();
    let s = game.session().unwrap();
    for name in &args[2..] {
        let p = s.objects.point(name);
        let r = p.and_then(|(x, y, z)| s.level.world.region_at_z(x, y, Some(z + 1.0)));
        println!("{name}: {p:?} r{r:?}");
    }
}
