//! Print a region's walls (debug aid): `cargo run --example region_dump LIPS12 74`
use godd_game::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (level, r): (&str, usize) = (&args[1], args[2].parse().unwrap());
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{level}.XRB")).unwrap(), vfs.read(&format!("{level}.XPG")).unwrap()).unwrap();
    let w = &lvl.world;
    let reg = &w.regions[r];
    println!("region {r}: floor {} ceil {} bounds {:?} below {:?} above {:?}", reg.floor, reg.ceiling, reg.bounds, reg.below, reg.above);
    for &i in &reg.walls {
        let wall = &w.walls[i];
        println!("  wall {i}: {:?} -> {:?}  front {} back {:?} blocking {}", w.vertices[wall.a], w.vertices[wall.b], wall.front, wall.back, wall.blocking);
    }
    let m = &godd_game::scene::build(w)[r];
    println!("parts {} open {} tris {}", m.outline.parts.len(), m.outline.open_edges, m.triangles.len());
}
