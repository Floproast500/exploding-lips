//! Portal lines whose sides are not in the regions they join (debug aid):
//! `cargo run --example false_portals LIPS07`
use godd_game::Vfs;

fn main() {
    let level = std::env::args().nth(1).unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{level}.XRB")).unwrap(), vfs.read(&format!("{level}.XPG")).unwrap()).unwrap();
    let w = &lvl.world;
    let mut n = 0;
    for (i, x) in w.walls.iter().enumerate() {
        let Some(b) = x.back.filter(|&b| b != x.front) else { continue };
        let ((ax, ay), (bx, by)) = (w.vertices[x.a], w.vertices[x.b]);
        let len = (bx - ax).hypot(by - ay);
        if len < 1.0 {
            continue;
        }
        let (mx, my, nx, ny) = ((ax + bx) / 2.0, (ay + by) / 2.0, -(by - ay) / len * 2.0, (bx - ax) / len * 2.0);
        let (p, q) = ((mx + nx, my + ny), (mx - nx, my - ny));
        let ok = (w.point_in_region(p.0, p.1, x.front) && w.point_in_region(q.0, q.1, b)) || (w.point_in_region(p.0, p.1, b) && w.point_in_region(q.0, q.1, x.front));
        if !ok {
            n += 1;
            if std::env::args().nth(2).is_some() {
                println!("wall {i}: r{} / r{b} len {len:.0}", x.front);
            }
        }
    }
    println!("{level}: {n} false portals of {}", w.walls.len());
}
