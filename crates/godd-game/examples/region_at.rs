//! The region at a spot (debug aid): `cargo run --example region_at LIPS03 17088 16456 [z]`
use godd_game::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (x, y): (f32, f32) = (args[2].parse().unwrap(), args[3].parse().unwrap());
    let z: Option<f32> = args.get(4).and_then(|v| v.parse().ok());
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{}.XRB", args[1])).unwrap(), vfs.read(&format!("{}.XPG", args[1])).unwrap()).unwrap();
    println!("{:?}", lvl.world.region_at_z(x, y, z));
}
