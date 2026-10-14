//! List a level's objects (debug aid): `cargo run --example obj_dump LIPS06 [filter]`
use godd_game::{Game, Options, Vfs};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let filter = args.get(2).map(|s| s.to_ascii_uppercase());
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(args[1].clone()), save: None }).unwrap();
    let s = game.session().unwrap();
    for (i, o) in s.objects.list.iter().enumerate() {
        let aux = o.aux.as_ref().map(|a| format!("ints {:?} slots {:?} sounds {:?}", a.ints, (0..10).filter_map(|k| a.slot(k).map(|n| (k, n))).collect::<Vec<_>>(), a.sounds.iter().flatten().collect::<Vec<_>>())).unwrap_or_default();
        let line = format!("#{i} {} eff {} tag {} flags {:#x} at ({:.0},{:.0},{:.0}) r{:?} rad {} h {} hidden {} {aux}", o.name, o.effect, o.tag, o.flags, o.x, o.y, o.z, o.region, o.radius, o.height, o.hidden);
        if filter.as_ref().is_none_or(|f| line.to_ascii_uppercase().contains(f.as_str())) {
            println!("{line}");
        }
    }
}
