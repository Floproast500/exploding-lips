//! The tags a trigger region fires (enter, leave, land, ceiling) and what listens to each
//! (debug aid): `cargo run --example trigger_tags LIPS05 114 286`
use godd_game::{Game, Options, Vfs};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(args[1].clone()), save: None }).unwrap();
    let s = game.session().unwrap();
    let triggers = s.effects.triggers();
    for r in args[2..].iter().filter_map(|a| a.parse::<usize>().ok()) {
        let Some((_, tags)) = triggers.iter().find(|(t, _)| *t == r) else {
            println!("r{r}: no trigger");
            continue;
        };
        println!("r{r}: enter {} leave {} land {} ceiling {}", tags[0], tags[1], tags[2], tags[3]);
        for &t in tags.iter().filter(|&&t| t > 0) {
            for (i, o) in s.objects.list.iter().enumerate().filter(|(_, o)| o.tag == t) {
                let target = o.aux.as_ref().and_then(|a| a.slot(9)).unwrap_or("");
                println!("  tag {t}: #{i} {} eff {} -> {target}", o.name, o.effect);
            }
            for (d, _) in s.effects.doors().iter().filter(|(_, dt)| *dt == t) {
                println!("  tag {t}: door r{d}");
            }
        }
    }
}
