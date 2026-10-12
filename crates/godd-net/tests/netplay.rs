//! Two games over loopback UDP: the client loads the host's level, each sees the other as a
//! sprite, and a world tag fired on one opens the door on both. Needs the game files.
#![cfg(feature = "game")]

use godd_game::{Game, Input, Options, Vfs};
use godd_net::session::NetSession;
use std::path::Path;

#[test]
fn host_and_client_share_players_and_doors() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    if !dir.is_dir() {
        return;
    }
    let vfs = Vfs::from_dir(&dir).unwrap();
    let mut host_game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let mut client_game = Game::new(&vfs, &Options::default()).unwrap();
    let mut host = NetSession::host(0, "LIPS00", "host").unwrap();
    let port = host.local_addr().unwrap().port();
    let mut client = NetSession::join(&format!("127.0.0.1:{port}"), "client").unwrap();
    let idle = Input::default();
    let mut pump = |frames: usize, host_game: &mut Game, client_game: &mut Game| {
        for _ in 0..frames {
            host.update(host_game, &vfs, 0.06);
            client.update(client_game, &vfs, 0.06);
            host_game.tick(&vfs, &idle);
            client_game.tick(&vfs, &idle);
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
    };
    pump(40, &mut host_game, &mut client_game);
    assert_eq!(client_game.session().map(|s| s.name.as_str()), Some("LIPS00"), "client loaded the host's level");
    let visible = |g: &Game| g.session().unwrap().level.world.sprites.iter().filter(|s| s.visible).count();
    let host_sprites = visible(&host_game);
    let client_sprites = visible(&client_game);

    // Host flips a world switch tag; the door (region 5, tag 53) opens on both sides.
    let s = host_game.session_mut().unwrap();
    s.outgoing_tags.push(53);
    s.apply_shared_tag(53);
    s.outgoing_tags.push(53);
    pump(90, &mut host_game, &mut client_game);
    let ceiling = |g: &Game| g.session().unwrap().level.world.regions[5].ceiling;
    assert_eq!(ceiling(&host_game), 128.0);
    assert_eq!(ceiling(&client_game), 128.0, "tag replicated to the client");
    assert!(host_sprites > 0 && client_sprites > 0);
}
