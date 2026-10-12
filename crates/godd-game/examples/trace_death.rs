//! Trace the death chain on LIPS00 (debug aid).
use godd_game::{Game, Input, Options, Vfs};

fn main() {
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let s = game.session_mut().unwrap();
    let pt = s.objects.list.iter().position(|o| o.name == "PLAYTRN1").unwrap();
    println!("PLAYTRN1 #{pt} mover={:?}", s.objects.list[pt].mover.as_ref().map(|m| (m.active, m.waiting_tag, m.finished)));
    s.player.x = 17188.0;
    s.player.y = 12928.0;
    s.player.z = -128.0;
    s.player.region = 126;
    s.previous = s.player;
    for t in 0..180 {
        game.tick(&vfs, &Input::default());
        let s = game.session().unwrap();
        if t % 20 == 0 {
            let m = s.objects.list[pt].mover.as_ref().map(|m| (m.active, m.waiting_tag, m.finished));
            println!("{t}: pos=({:.0},{:.0}) region={} deaths={} playtrn={m:?}", s.player.x, s.player.y, s.player.region, s.script.states[11]);
        }
    }
}
