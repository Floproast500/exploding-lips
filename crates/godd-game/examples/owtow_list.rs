//! OWTOW wall copies of a level: target wall, source wall, and whether the copy makes the target
//! passable (debug aid): `cargo run --example owtow_list LIPS03`
use godd_game::{Game, Options, Vfs};

fn main() {
    let level = std::env::args().nth(1).unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(level), save: None }).unwrap();
    let s = game.session().unwrap();
    let w = &s.level.world;
    for (i, o) in s.objects.list.iter().enumerate().filter(|(_, o)| o.effect == 67) {
        let Some(a) = o.aux.as_ref() else { continue };
        let (tn, sn) = (a.slot(9).unwrap_or("-"), a.slot(8).unwrap_or("-"));
        let (t, src) = (s.wall_named(tn), s.wall_named(sn));
        let desc = |x: Option<usize>| x.map_or("?".to_string(), |x| format!("w{x} (r{} / {:?}, blocking {})", w.walls[x].front, w.walls[x].back, w.walls[x].blocking));
        println!("#{i} tag {}: {tn} {} <- {sn} {}", o.tag, desc(t), desc(src));
    }
}
