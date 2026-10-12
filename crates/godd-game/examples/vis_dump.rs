//! Print the regions visible from a region's centre (debug aid): `vis_dump LIPS11 117 255`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (level, r, turn): (&str, usize, f32) = (&a[1], a[2].parse().unwrap(), a[3].parse().unwrap());
    let vfs = godd_game::Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{level}.XRB")).unwrap(), vfs.read(&format!("{level}.XPG")).unwrap()).unwrap();
    let w = &lvl.world;
    let m = &godd_game::scene::build(w)[r];
    let (mut cx, mut cy, mut n) = (0.0, 0.0, 0.0);
    for t in &m.triangles { for &i in t { cx += w.vertices[i as usize].0; cy += w.vertices[i as usize].1; n += 1.0; } }
    let cam = godd_game::render::Camera { x: cx / n, y: cy / n, z: w.regions[r].floor + 48.0, angle: turn.to_radians(), pitch: 0.0, region: r };
    println!("camera {:.0},{:.0},{:.0}; point_in_region {}", cam.x, cam.y, cam.z, w.point_in_region(cam.x, cam.y, r));
    let vis = godd_game::scene::visible_regions(w, &cam, (0.6 * 1.6, 0.45), false);
    for (i, wins) in vis.iter().enumerate().filter(|(_, v)| !v.is_empty()) {
        let reg = &w.regions[i];
        println!("region {i}: floor {} ceil {} windows {} first {:?}", reg.floor, reg.ceiling, wins.len(), wins[0].iter().map(|p| [(p[0]*100.0).round()/100.0, (p[1]*100.0).round()/100.0]).collect::<Vec<_>>());
    }
}
