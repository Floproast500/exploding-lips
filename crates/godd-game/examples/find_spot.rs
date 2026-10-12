//! Find standing spots around an LIPS00 object for scripted original-vs-ours tests (debug aid).
use godd_game::{Game, Options, Vfs};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or("24".into());
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let s = game.session().unwrap();
    let idx = arg.parse().unwrap_or_else(|_| s.objects.list.iter().position(|o| o.name == arg && o.region.is_some()).expect("object"));
    let o = &s.objects.list[idx];
    let w = &s.level.world;
    println!("#{idx} {} at ({},{},{}) region {:?}", o.name, o.x, o.y, o.z, o.region);
    if let Some(r) = o.region {
        println!("region {r}: bounds {:?} floor {} ceil {}", w.regions[r].bounds, w.regions[r].floor, w.regions[r].ceiling);
    }
    for d in [96.0f32, 160.0, 240.0] {
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let (x, y) = (o.x + d * a.cos(), o.y + d * a.sin());
            if let Some(r) = w.region_at_z(x, y, Some(o.z + 1.0)) {
                // angle facing the object, 2048 units / turn, counter-clockwise from +X
                let face = ((a + std::f32::consts::PI) / std::f32::consts::TAU * 2048.0).rem_euclid(2048.0) as i32;
                println!("d={d} ({x:.0},{y:.0}) region {r} floor {} face {face}", w.regions[r].floor);
            }
        }
    }
}
