//! Who fires a tag, and what listens (debug aid): `cargo run --example tag_sources LIPS06 62`
use godd_game::{Game, Options, Vfs};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tag: i32 = args[2].parse().unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(args[1].clone()), save: None }).unwrap();
    let s = game.session().unwrap();
    let w = &s.level.world;
    for sw in s.effects.switches().iter().filter(|sw| sw.tags.contains(&tag)) {
        let wall = &w.walls[sw.wall];
        println!("wall switch {} (front r{} back {:?}) tags {:?}", sw.wall, wall.front, wall.back, sw.tags);
    }
    for (r, tags) in s.effects.triggers().iter().filter(|(_, t)| t.contains(&tag)) {
        println!("trigger region r{r} tags {tags:?}");
    }
    for (r, t) in s.effects.doors().iter().filter(|(_, t)| *t == tag) {
        let wait = s.effects.door_waits().iter().find(|(d, _)| d == r).map(|(_, w)| *w);
        println!("door r{r} (tag {t}) wait {wait:?}");
    }
    for (i, o) in s.objects.list.iter().enumerate() {
        let Some(a) = o.aux.as_ref() else { continue };
        if a.ints.contains(&tag) || o.tag == tag {
            let drop = a.slot(8).map(|p| (p.to_string(), s.objects.point(p), s.objects.point(p).and_then(|(x, y, z)| w.region_at_z(x, y, Some(z + 1.0)))));
            println!("object #{i} {} eff {} tag {} r{:?} ints {:?} point {drop:?}", o.name, o.effect, o.tag, o.region, a.ints);
        }
    }
}
