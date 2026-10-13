//! Region names, with their numbers (debug aid): `cargo run --example region_names LIPS05 [r...]`
use godd_game::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let only: Vec<usize> = args[2..].iter().filter_map(|a| a.parse().ok()).collect();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{}.XRB", args[1])).unwrap(), vfs.read(&format!("{}.XPG", args[1])).unwrap()).unwrap();
    for (r, n) in lvl.region_names().iter().enumerate() {
        if !n.is_empty() && (only.is_empty() || only.contains(&r)) {
            println!("r{r}: {n}");
        }
    }
}
