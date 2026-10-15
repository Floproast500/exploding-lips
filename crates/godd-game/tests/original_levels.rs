//! Load every shipped level into the engine world. Skipped without the game files.

use godd_formats::xrb::{RecordExt, Section};
use godd_game::level::Level;
use std::path::{Path, PathBuf};

fn original_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    dir.is_dir().then_some(dir)
}

fn levels(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().to_string_lossy().to_string();
            name.to_ascii_uppercase().strip_suffix(".XRB").map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

#[test]
fn every_level_loads_and_locates_objects() {
    let Some(dir) = original_dir() else { return };
    for name in levels(&dir) {
        let xrb = std::fs::read(dir.join(format!("{name}.XRB"))).unwrap();
        let xpg = std::fs::read(dir.join(format!("{name}.XPG"))).unwrap();
        let level = Level::load(&xrb, &xpg).unwrap_or_else(|e| panic!("{name}: {e}"));
        let world = &level.world;
        assert!(!world.regions.is_empty(), "{name}");
        assert!(world.sky.is_some(), "{name}: sky");

        // Point location must agree with the region stored for every placed object.
        let (mut checked, mut wrong) = (0, Vec::new());
        for (i, o) in level.orb.records(Section::Objects).iter().enumerate() {
            let (Ok(point), Ok(region)) = (usize::try_from(o.le_i32(0x14)), usize::try_from(o.le_i32(0xfc))) else { continue };
            let Some(&(x, y)) = world.vertices.get(point) else { continue };
            if &o[0x58..0x60] == b"NETPLAYR" {
                continue; // network template, not placed in any region list
            }
            if !world.regions[region].walls.is_empty() {
                checked += 1;
                if world.region_at_z(x, y, Some(o.fixed(0x2c))) != Some(region) {
                    wrong.push(i);
                }
            }
        }
        assert!(wrong.is_empty(), "{name}: {} of {checked} objects located in the wrong region: {:?}", wrong.len(), &wrong[..wrong.len().min(10)]);
    }
}

#[test]
fn every_level_script_starts_with_original_states() {
    use godd_game::script::{LIFE, MAGIC, Script};
    let Some(dir) = original_dir() else { return };
    let startup = String::from_utf8_lossy(&std::fs::read(dir.join("LIPSTART.INI")).unwrap()).into_owned();
    for name in levels(&dir) {
        let text = String::from_utf8_lossy(&std::fs::read(dir.join(format!("{name}.INI"))).unwrap()).into_owned();
        let mut script = Script::new(7);
        for (ini, section) in [(&startup, "nostrap9999"), (&startup, "commands9999"), (&text, "nostrap9999"), (&text, "commands9999")] {
            script.run_section(ini, section);
        }
        assert_eq!(script.states[LIFE], 100, "{name}");
        assert_eq!(script.states[MAGIC], 0, "{name}");
        if name != "LIPSTART" {
            assert_eq!(script.thresholds.max[LIFE], 100, "{name}: SETMAXOS");
            assert!(script.available_tools != 0, "{name}: AVAILTOOLS");
        }
    }
}

/// Golden values probed from the original (tests/pilot/start_lips00.txt): from the LIPS00
/// start, walking north for ~0.5 s drops the player off the start pad through room 635's
/// see-through floor into region 628 at z = 0, about 307 units further north.
#[test]
fn lips00_walk_off_start_pad_matches_original() {
    use godd_game::{Action, Game, Input, Options, TICK_RATE, Vfs};
    let Some(dir) = original_dir() else { return };
    let vfs = Vfs::from_dir(&dir).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let start = game.session().unwrap().player;
    assert_eq!((start.x, start.y, start.z, start.region), (11008.0, 15616.0, 160.0, 640));
    let mut input = Input::default();
    input.set(Action::Forward, true);
    for _ in 0..(TICK_RATE * 0.46) as usize {
        game.tick(&vfs, &input);
        input.end_tick();
    }
    input.set(Action::Forward, false);
    for _ in 0..(TICK_RATE * 1.0) as usize {
        game.tick(&vfs, &input);
        input.end_tick();
    }
    let p = game.session().unwrap().player;
    assert_eq!(p.region, 628, "fell into the region below");
    assert_eq!(p.z, 0.0);
    assert!((p.y - 15922.6).abs() < 40.0, "y = {}", p.y);
    assert_eq!(p.x, 11008.0);
}

/// RTDOOR (world-effects.md §3.1): LIPS00 region 5 is a closed door on tag 53, opening to
/// 128 at speed 200 (units / 1.2 s → 0.77 s) and closing by itself after 5 s.
#[test]
fn lips00_door_opens_on_tag_and_closes_after_wait() {
    use godd_game::{Game, Input, Options, TICK_RATE, Vfs};
    let Some(dir) = original_dir() else { return };
    let vfs = Vfs::from_dir(&dir).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let ceiling = |g: &Game| g.session().unwrap().level.world.regions[5].ceiling;
    assert_eq!(ceiling(&game), 0.0, "stored closed");
    game.session_mut().unwrap().fire_tag(53);
    let input = Input::default();
    let ticks = |s: f64| (s * TICK_RATE) as usize;
    for _ in 0..ticks(0.5) {
        game.tick(&vfs, &input);
    }
    let mid = ceiling(&game);
    assert!(mid > 60.0 && mid < 128.0, "opening: {mid}");
    for _ in 0..ticks(0.5) {
        game.tick(&vfs, &input);
    }
    assert_eq!(ceiling(&game), 128.0, "fully open after ~0.77 s");
    for _ in 0..ticks(6.0) {
        game.tick(&vfs, &input);
    }
    assert_eq!(ceiling(&game), 0.0, "closed again after the 5 s wait");
}

/// Touching a death bot kills the player (movers.md §7): "GOTCHA!!!", tag 661 counts a death
/// and restores Life/Armor (INI ONTAG[661]), PLAYTRN1 spins the view and fires tag 3, and the
/// DEATH01 teleporter (destination: the RBIRTH01 marker object) puts the player at the
/// rebirth spot.
#[test]
fn lips00_death_bot_kills_and_respawns_player() {
    use godd_game::script::{DEATHS, LIFE};
    use godd_game::{Game, Input, Options, TICK_RATE, Vfs};
    let Some(dir) = original_dir() else { return };
    let vfs = Vfs::from_dir(&dir).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let input = Input::default();
    let s = game.session_mut().unwrap();
    s.script.states[LIFE] = 40;
    // Stand right next to DETHBOT1 (object 24 at 17168,12928 in region 126, floor -128).
    let start = (s.player.x, s.player.y);
    s.player.x = 17168.0 + 20.0;
    s.player.y = 12928.0;
    s.player.z = -128.0;
    s.player.region = 126;
    s.previous = s.player;
    let mut saw_gotcha = false;
    for _ in 0..(TICK_RATE * 3.0) as usize {
        game.tick(&vfs, &input);
        saw_gotcha |= game.session().unwrap().messages.iter().any(|m| m.text.contains("GOTCHA"));
    }
    let s = game.session().unwrap();
    assert!(saw_gotcha, "bot's ON BP message");
    assert_eq!(s.script.states[DEATHS], 1, "STATEVAL(11 + 1) in ONTAG[661]");
    assert_eq!(s.script.states[LIFE], 100, "INCTOOS restores Life");
    let _ = start;
    let marker = s.objects.list.iter().find(|o| o.name == "RBIRTH01").expect("RBIRTH01 marker");
    assert_eq!((s.player.x, s.player.y), (marker.x, marker.y), "respawned at the rebirth marker");
}

/// Level exits (OSTRAP, object-effects.md): tag 500 fires LIPS00's TOLIPS01 exit, the next
/// tick loads LIPS01 and the player's states carry over.
#[test]
fn lips00_exit_loads_lips01_with_states() {
    use godd_game::script::MONEY;
    use godd_game::{Game, Input, Options, Vfs};
    let Some(dir) = original_dir() else { return };
    let vfs = Vfs::from_dir(&dir).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    game.session_mut().unwrap().script.states[MONEY] = 777;
    game.session_mut().unwrap().fire_tag(500);
    game.tick(&vfs, &Input::default());
    game.tick(&vfs, &Input::default());
    let s = game.session().unwrap();
    assert_eq!(s.name, "LIPS01");
    assert_eq!(s.script.states[MONEY], 777, "states carry over");
}

/// Combat (combat.md, movers.md): DETHBOT1 has SH(1); the first shotgun hit runs its HP block,
/// the next event runs HK — enemies-killed +1 and the bot becomes its DEADBOT1 pickup.
#[test]
fn lips00_shooting_a_bot_kills_it() {
    use godd_game::script::KILLS;
    use godd_game::{Action, Game, Input, Options, TICK_RATE, Vfs};
    let Some(dir) = original_dir() else { return };
    let vfs = Vfs::from_dir(&dir).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    let idle = Input::default();
    for _ in 0..30 {
        game.tick(&vfs, &idle); // start-pad tool pickup selects the shotgun
    }
    assert_eq!(game.session().unwrap().tool, 5);
    let s = game.session_mut().unwrap();
    // Face DETHBOT1 (17168, 12928, region 126) from 120 units west.
    s.player.x = 17168.0 - 120.0;
    s.player.y = 12928.0;
    s.player.z = -128.0;
    s.player.region = s.level.world.region_at_z(s.player.x, s.player.y, Some(-127.0)).unwrap();
    s.player.angle = 0.0;
    s.previous = s.player;
    let mut fire = Input::default();
    fire.set(Action::Use, true);
    for _ in 0..(TICK_RATE * 4.0) as usize {
        game.tick(&vfs, &fire);
        let s = game.session_mut().unwrap();
        // Keep facing the bot even if hits push things around.
        s.player.angle = 0.0;
        if s.script.states[KILLS] > 0 {
            break;
        }
    }
    let s = game.session().unwrap();
    assert_eq!(s.script.states[KILLS], 1, "bot killed");
    let bot = &s.objects.list[24];
    assert_eq!(bot.effect, s.objects.list.iter().find(|o| o.name == "DEADBOT1").unwrap().effect, "changed class into the pickup");
}

fn start(level: &str) -> Option<(godd_game::Vfs, godd_game::Game)> {
    let dir = original_dir()?;
    let vfs = godd_game::Vfs::from_dir(&dir).unwrap();
    let game = godd_game::Game::new(&vfs, &godd_game::Options { start_level: Some(level.into()), save: None }).unwrap();
    Some((vfs, game))
}

fn run(game: &mut godd_game::Game, vfs: &godd_game::Vfs, seconds: f64) {
    let idle = godd_game::Input::default();
    for _ in 0..(seconds * godd_game::TICK_RATE) as usize {
        game.tick(vfs, &idle);
    }
}

/// RROTATE (world-effects.md §3.6): LIPS02 DOOR01 swings 90° about its pivot per tag 28.
#[test]
fn lips02_rotating_door_swings_ninety_degrees() {
    let Some((vfs, mut game)) = start("LIPS02") else { return };
    let s = game.session().unwrap();
    let world = &s.level.world;
    let pivot = world.vertices[374];
    // The endpoint farthest from the hinge (the pivot itself does not move).
    let v = world.regions[24]
        .walls
        .iter()
        .flat_map(|&w| [world.walls[w].a, world.walls[w].b])
        .max_by(|&a, &b| {
            let d = |v: usize| (world.vertices[v].0 - pivot.0).hypot(world.vertices[v].1 - pivot.1);
            d(a).total_cmp(&d(b))
        })
        .unwrap();
    let before = world.vertices[v];
    game.session_mut().unwrap().fire_tag(28);
    run(&mut game, &vfs, 2.5);
    let after = game.session().unwrap().level.world.vertices[v];
    let angle = |p: (f32, f32)| (p.1 - pivot.1).atan2(p.0 - pivot.0);
    let turned = (angle(after) - angle(before)).rem_euclid(std::f32::consts::TAU).to_degrees();
    let turned = turned.min(360.0 - turned);
    assert!((turned - 90.0).abs() < 3.0, "turned {turned}°");
}

/// RSLANT (§3.8): on LIPS01 RAMP02's strips the player stands at the ramp height.
#[test]
fn lips01_ramp_lifts_the_player() {
    let Some((vfs, mut game)) = start("LIPS01") else { return };
    let s = game.session_mut().unwrap();
    let strips: Vec<usize> = (0..s.level.world.regions.len()).filter(|&r| s.effects.ramp_limits(r, 0.0, 0.0).is_some()).collect();
    assert!(strips.len() >= 3, "ramp strips found: {strips:?}");
    let r = strips[strips.len() / 2];
    let ((x0, y0), (x1, y1)) = s.level.world.regions[r].bounds;
    let (x, y) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (low, high) = s.effects.ramp_limits(r, x, y).unwrap();
    s.player.x = x;
    s.player.y = y;
    s.player.region = r;
    s.player.z = s.level.world.regions[r].floor;
    s.previous = s.player;
    run(&mut game, &vfs, 0.2);
    let p = game.session().unwrap().player;
    assert!(p.z >= low - 0.5 && p.z <= high + 0.5, "z {} within ramp {low}..{high}", p.z);
    assert!(low > game.session().unwrap().level.world.regions[r].floor - 0.5);
}

/// RMOVE + OFOLLOW (§3.7): LIPS00's raft region travels with RAFTMAN1 along RAFTPATH.
#[test]
fn lips00_raft_moves_with_its_carrier() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    let before = game.session().unwrap().level.world.regions[415].bounds;
    run(&mut game, &vfs, 3.0);
    let after = game.session().unwrap().level.world.regions[415].bounds;
    let moved = ((after.0.0 - before.0.0).powi(2) + (after.0.1 - before.0.1).powi(2)).sqrt();
    assert!(moved > 20.0, "raft moved {moved}");
}

/// OWTOW: tag 93 in LIPS00 gives wall DARK08 the look of LIGHT08.
#[test]
fn lips00_wall_copy_changes_the_look() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    let find = |g: &godd_game::Game, name: &str| {
        let s = g.session().unwrap();
        s.level.orb.records(godd_formats::xrb::Section::Walls).iter().position(|w| w[0xb0..0xb8].starts_with(name.as_bytes())).unwrap()
    };
    let (target, source) = (find(&game, "DARK08"), find(&game, "LIGHT08"));
    game.session_mut().unwrap().fire_tag(93);
    run(&mut game, &vfs, 0.1);
    let w = &game.session().unwrap().level.world.walls;
    assert_eq!(w[target].middle.map(|s| s.image), w[source].middle.map(|s| s.image));
    assert_eq!(w[target].upper.map(|s| s.image), w[source].upper.map(|s| s.image));
}

/// OFLYON / OFLYOFF: tag 1 starts flying (Jump rises above the floor), tag 2 ends it.
#[test]
fn flying_on_and_off() {
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    game.session_mut().unwrap().fire_tag(1);
    assert!(game.session().unwrap().player.flying);
    let floor_z = game.session().unwrap().player.z;
    let mut up = Input::default();
    up.set(Action::Jump, true);
    for _ in 0..30 {
        game.tick(&vfs, &up);
    }
    assert!(game.session().unwrap().player.z > floor_z + 20.0, "rose while flying");
    game.session_mut().unwrap().fire_tag(2);
    assert!(!game.session().unwrap().player.flying);
}

/// OSOUND with count -1 loops (LIPS15 alarm, tag 9); OSNDSEQ chains its slots (LIPS02 tag 9).
#[test]
fn looping_and_chained_sounds() {
    use godd_game::AudioEvent;
    let Some((vfs, mut game)) = start("LIPS15") else { return };
    game.drain_audio();
    game.session_mut().unwrap().fire_tag(9);
    run(&mut game, &vfs, 0.1);
    assert!(game.drain_audio().iter().any(|e| matches!(e, AudioEvent::Loop { name, .. } if name == "ALARM01")));

    let Some((vfs, mut game)) = start("LIPS02") else { return };
    game.drain_audio();
    game.session_mut().unwrap().fire_tag(9);
    let mut names = Vec::new();
    for _ in 0..(8.0 * godd_game::TICK_RATE) as usize {
        game.tick(&vfs, &godd_game::Input::default());
        for e in game.drain_audio() {
            if let AudioEvent::Sound { name, .. } = e {
                names.push(name);
            }
        }
    }
    // int0 = 2 → KNOCK02 three times, then KLANG01 three times.
    assert_eq!(names.iter().filter(|n| *n == "KNOCK02").count(), 3, "{names:?}");
    assert_eq!(names.iter().filter(|n| *n == "KLANG01").count(), 3, "{names:?}");
}

/// WTV + OVIEWON: LIPS00 switches TVVIEW on at start; its walls show the live view.
#[test]
fn lips00_tv_shows_a_live_view() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let mut frame = godd_game::Frame::new(320, 200);
    game.render(&mut frame, 1.0);
    let s = game.session().unwrap();
    let tv = s.tvs.iter().find(|t| t.name == "TVVIEW").expect("TVVIEW");
    assert!(tv.on, "switched on by the tag-9999 OVIEWON");
    for (wall, _) in &tv.walls {
        assert_eq!(s.level.world.walls[*wall].middle.map(|m| m.image), Some(tv.image));
    }
    let img = &s.level.world.images[tv.image];
    let distinct: std::collections::HashSet<u8> = img.pixels.iter().copied().collect();
    assert!(distinct.len() > 8, "rendered picture, not a blank");
}

/// ODROP: a carried KEY2 is placed back into the world by tag 118, firing tag 117.
#[test]
fn lips00_drop_places_a_carried_item() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    let s = game.session_mut().unwrap();
    let key = s.objects.list.iter().position(|o| o.name == "KEY2").unwrap();
    s.objects.list[key].hidden = true;
    s.objects.inventory.push(key);
    s.fire_tag(118);
    run(&mut game, &vfs, 0.1);
    let s = game.session().unwrap();
    assert!(!s.objects.list[key].hidden);
    assert!(!s.objects.inventory.contains(&key));
}

/// OFLASH: tag 98 in LIPS00 strobes full bright and then ends; OTEXOVER: tag 65 in LIPS12
/// shows the rain overlay.
#[test]
fn lightning_and_rain_overlay() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    game.session_mut().unwrap().fire_tag(98);
    assert!(game.session().unwrap().flash);
    run(&mut game, &vfs, 3.0);
    assert!(!game.session().unwrap().flash, "flashes end");

    let Some((vfs, mut game)) = start("LIPS12") else { return };
    game.session_mut().unwrap().fire_tag(65);
    run(&mut game, &vfs, 0.1);
    assert_eq!(game.session().unwrap().overlays.len(), 1);
    game.session_mut().unwrap().fire_tag(65);
    run(&mut game, &vfs, 0.1);
    assert!(game.session().unwrap().overlays.is_empty(), "toggles off");
}

/// OEFFSW → OCOUNT (k54 = 7) → STRTDONE timer (LIPS14): the 7th relayed tag fires it.
#[test]
fn lips14_counter_fires_on_seventh_event() {
    let Some((vfs, mut game)) = start("LIPS14") else { return };
    let timer = |g: &godd_game::Game| g.session().unwrap().objects.list.iter().position(|o| o.name == "STRTDONE").unwrap();
    let t = timer(&game);
    for k in 0..7 {
        game.session_mut().unwrap().fire_tag(if k % 2 == 0 { 30 } else { 31 });
        run(&mut game, &vfs, 0.05);
    }
    // The timer fires after its 4 s period; before that its effect target (DODONE) waits.
    let deadline_set = game.session().unwrap().objects.timer_running(t);
    assert!(deadline_set, "STRTDONE started by the counter");
}

/// OOTOO: tag 79 in LIPS00 turns the inert placeholder NOBRONZE into the BRONZE pickup.
#[test]
fn lips00_morph_reveals_a_pickup() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    assert_eq!(game.session().unwrap().objects.list[544].effect, -1);
    game.session_mut().unwrap().fire_tag(79);
    run(&mut game, &vfs, 0.1);
    let o = &game.session().unwrap().objects.list[544];
    assert_eq!(o.effect, 130, "now an OPICKUP");
    assert_eq!(o.name, "NOBRONZE", "keeps its name");
}

/// A bomb-style blast (2.5 s, weapon) hits a bot several times through its 240 ms debounce:
/// a DETHBOT4 (SH(3)) dies inside one blast.
#[test]
fn lips00_bomb_blast_hits_repeatedly() {
    use godd_game::script::KILLS;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.5);
    let s = game.session_mut().unwrap();
    let bot = s.objects.list.iter().position(|o| o.name == "DETHBOT4" && o.region.is_some()).expect("a DETHBOT4");
    let (x, y, z, region) = {
        let o = &s.objects.list[bot];
        (o.x, o.y, o.z, o.region)
    };
    // Keep the player far away so it only reacts to the blast.
    s.player.x += 5000.0;
    s.objects.spawn_explosion((x, y, z), 48.0, 64.0, 2.5, region);
    run(&mut game, &vfs, 2.6);
    let s = game.session().unwrap();
    assert_eq!(s.script.states[KILLS], 1, "killed by repeated blast hits");
}

/// Measured on lips.exe under Wine (tests/pilot/shots_to_kill.txt, godd-launch probes of the
/// mover hit counter and the kill count): the first fired shotgun shot takes DETHBOT1's SH(1)
/// counter to 0 without a kill; a press during the 1.44 s use animation is ignored; the next
/// fired shot runs HK. An SH(n) bot dies on shot n+1.
#[test]
fn lips00_shots_to_kill_match_the_original() {
    use godd_game::script::KILLS;
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.5); // start-pad pickup selects the shotgun
    let s = game.session_mut().unwrap();
    s.player.x = 17168.0 + 640.0; // the probe position: 640 units east, facing west
    s.player.y = 12928.0;
    s.player.z = -128.0;
    s.player.region = 126;
    s.player.angle = std::f32::consts::PI;
    s.previous = s.player;
    let mut fire = Input::default();
    fire.set(Action::Use, true);
    let tap = |game: &mut godd_game::Game| {
        game.tick(&vfs, &fire);
        run(game, &vfs, 0.6);
        let s = game.session().unwrap();
        (s.objects.list[24].mover.as_ref().unwrap().hits_left, s.script.states[KILLS])
    };
    assert_eq!(tap(&mut game), (0, 0), "first shot: counter 0, HP block only");
    assert_eq!(tap(&mut game), (0, 0), "press 0.6 s later is ignored (use animation running)");
    run(&mut game, &vfs, 0.5);
    let mut fired = tap(&mut game);
    for _ in 0..3 {
        if fired.1 == 1 {
            break;
        }
        fired = tap(&mut game);
    }
    assert_eq!(fired.1, 1, "second fired shot kills");
}

/// Gamepad tool switching: ToolNext/ToolPrev step through the selectable tools and wrap.
#[test]
fn lips00_tool_cycling_wraps() {
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.5);
    let tool = |game: &godd_game::Game| game.session().unwrap().tool;
    assert_eq!(tool(&game), 5);
    let press = |game: &mut godd_game::Game, action| {
        let mut input = Input::default();
        input.set(action, true);
        game.tick(&vfs, &input);
        run(game, &vfs, 0.05);
    };
    press(&mut game, Action::ToolNext);
    assert_eq!(tool(&game), 6);
    press(&mut game, Action::ToolPrev);
    press(&mut game, Action::ToolPrev);
    assert_eq!(tool(&game), 4);
    for _ in 0..5 {
        press(&mut game, Action::ToolPrev);
    }
    assert_eq!(tool(&game), 9, "wraps from 0 back to 9 (AVAILTOOLS 3FF)");
}

/// Mouse look is 1:1: motion shows in the camera before the tick consumes it, and the view
/// doesn't move again when the tick applies it (no interpolation lag, no double turn).
#[test]
fn lips00_mouse_look_is_one_to_one() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let a0 = game.view_camera(1.0).unwrap().angle;
    let mut input = godd_game::Input::default();
    input.mouse = (200.0, -80.0);
    game.preview_look(&input);
    let shown = game.view_camera(0.3).unwrap();
    let yaw = 200.0 * game.options.mouse_sensitivity;
    let wrap = |a: f32| (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    assert!(wrap(shown.angle - (a0 - yaw)).abs() < 1e-4, "pending motion shown at once");
    game.tick(&vfs, &input);
    input.end_tick();
    game.preview_look(&input);
    for alpha in [0.0, 0.5, 1.0] {
        let c = game.view_camera(alpha).unwrap();
        assert!(wrap(c.angle - shown.angle).abs() < 1e-4, "no jump after the tick at alpha {alpha}");
        assert!((c.pitch - shown.pitch).abs() < 1e-4);
    }
    assert!(shown.pitch > 0.0, "mouse up looks up");
}

/// Autorun inverts Run, toggle crouch latches Crouch on each press.
#[test]
fn lips00_autorun_and_toggle_crouch() {
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let walk = |game: &mut godd_game::Game| {
        let y0 = game.session().unwrap().player.y;
        let mut i = Input::default();
        i.set(Action::Forward, true);
        for _ in 0..10 {
            game.tick(&vfs, &i);
        }
        game.session().unwrap().player.y - y0
    };
    let slow = walk(&mut game);
    game.cvars.set("cl_autorun", "1").unwrap();
    let fast = walk(&mut game);
    assert!((fast / slow - 2.0).abs() < 0.05, "autorun doubles speed: {slow} vs {fast}");
    game.cvars.set("cl_togglecrouch", "1").unwrap();
    let mut press = Input::default();
    press.set(Action::Crouch, true);
    game.tick(&vfs, &press);
    run(&mut game, &vfs, 0.1);
    assert!(game.session().unwrap().player.crouching, "stays crouched after release");
    let mut press = Input::default();
    press.set(Action::Crouch, true);
    game.tick(&vfs, &press);
    run(&mut game, &vfs, 0.1);
    assert!(!game.session().unwrap().player.crouching);
}

/// Every region of every level triangulates: on a sample grid, a point is covered by the
/// region's triangles exactly when the engine's own point-in-region test (even-odd over the
/// region's walls) says it is inside. Regions with unclosed stray edges (walls that name a
/// region they don't bound, e.g. LIPS07 region 132's lone wall 64 units east of its box) break
/// that even-odd test, so for them the oracle is the closed outline loops instead.
#[test]
fn every_region_triangulates() {
    use engine_scene::polygon::contains;
    let Some(dir) = original_dir() else { return };
    let vfs = godd_game::Vfs::from_dir(&dir).unwrap();
    let (mut regions, mut samples, mut bad, mut stray) = (0, 0usize, Vec::new(), 0);
    for level in ["LIPSTART"].into_iter().map(String::from).chain((0..18).map(|i| format!("LIPS{i:02}"))) {
        let (Ok(xrb), Ok(xpg)) = (vfs.read(&format!("{level}.XRB")), vfs.read(&format!("{level}.XPG"))) else { continue };
        let Ok(lvl) = godd_game::level::Level::load(xrb, xpg) else { continue };
        let w = &lvl.world;
        let pts = godd_game::scene::points(w);
        for (r, m) in godd_game::scene::build(w).iter().enumerate() {
            regions += 1;
            stray += (m.outline.open_edges > 0) as usize;
            let ((x0, y0), (x1, y1)) = w.regions[r].bounds;
            let (mut wrong, mut n) = (0, 0);
            const STEPS: usize = 24;
            for i in 0..STEPS {
                for j in 0..STEPS {
                    // Cell centres, nudged off the 1-unit grid the maps are drawn on.
                    let x = x0 as f64 + (x1 - x0) as f64 * (i as f64 + 0.5) / STEPS as f64 + 0.013;
                    let y = y0 as f64 + (y1 - y0) as f64 * (j as f64 + 0.5) / STEPS as f64 + 0.017;
                    let world_says = if m.outline.open_edges == 0 {
                        w.point_in_region(x as f32, y as f32, r)
                    } else {
                        m.outline.parts.iter().any(|p| contains(&pts, &p.outer, [x, y]) && !p.holes.iter().any(|h| contains(&pts, h, [x, y])))
                    };
                    let covered = m.triangles.iter().any(|t| contains(&pts, t, [x, y]));
                    n += 1;
                    wrong += (world_says != covered) as usize;
                }
            }
            samples += n;
            if wrong * 50 > n {
                bad.push(format!("{level} region {r}: {wrong}/{n} samples disagree (open edges {})", m.outline.open_edges));
            }
        }
    }
    assert!(regions > 5000, "levels loaded ({regions} regions)");
    assert!(bad.is_empty(), "{} of {regions} regions fail ({samples} samples):\n{}", bad.len(), bad.join("\n"));
    assert!(stray < regions / 50, "{stray} regions have stray edges");
}

/// Modern audio: a door's sounds come from the door region (positional), and every frame tells
/// the front end where the listener is and how much the room reverberates.
#[test]
fn lips00_door_sounds_are_positional() {
    use godd_game::AudioEvent;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let _ = game.drain_audio();
    let before: Vec<f32> = game.session().unwrap().level.world.regions.iter().map(|r| r.ceiling).collect();
    game.session_mut().unwrap().fire_tag(53);
    run(&mut game, &vfs, 0.1);
    let events = game.drain_audio();
    let regions = &game.session().unwrap().level.world.regions;
    let moving: Vec<_> = regions.iter().zip(&before).filter(|(r, c)| r.ceiling != **c).map(|(r, _)| r.bounds).collect();
    let positional: Vec<(&str, [f32; 3])> = events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::Sound { name, pos: Some(p), .. } => Some((name.as_str(), *p)),
            _ => None,
        })
        .collect();
    let inside = |p: &[f32; 3], ((x0, y0), (x1, y1)): &((f32, f32), (f32, f32))| (*x0..=*x1).contains(&p[0]) && (*y0..=*y1).contains(&p[1]);
    assert!(!moving.is_empty(), "tag 53 opens doors");
    assert!(positional.iter().any(|(_, p)| moving.iter().any(|b| inside(p, b))), "a sound from an opening door {moving:?}: {positional:?}");
    let reverb = events.iter().find_map(|e| match e {
        AudioEvent::Listener { reverb, .. } => Some(*reverb),
        _ => None,
    });
    assert!(reverb.is_some_and(|r| (0.0..=1.0).contains(&r)), "listener with reverb {reverb:?}");
}

/// Save slots: saving records level, position and states plus a thumbnail of the view; the
/// file text round-trips; loading restores them after moving and changing states.
#[test]
fn lips00_save_slot_round_trip() {
    use godd_game::{Action, Input, script::MONEY};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    let at = |g: &godd_game::Game| {
        let p = g.session().unwrap().player;
        (p.x, p.y)
    };
    let saved_at = at(&game);
    game.session_mut().unwrap().script.states[MONEY] = 1234;
    game.save_slot(2);
    let files = game.take_slots();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, 2);
    let slot = godd_game::saves::SaveSlot::parse(&files[0].1).unwrap();
    assert!(slot.label.starts_with("LIPS00"), "{}", slot.label);
    assert!(slot.thumb.iter().filter(|&&t| t != 0).count() > 1000, "the thumbnail shows the view");
    // Walk away and spend the money, then load.
    let mut fwd = Input::default();
    fwd.set(Action::Forward, true);
    for _ in 0..30 {
        game.tick(&vfs, &fwd);
    }
    game.session_mut().unwrap().script.states[MONEY] = 0;
    assert_ne!(at(&game), saved_at);
    let mut restored = start("LIPS00").unwrap().1;
    restored.set_slot(2, &files[0].1);
    restored.load_slot(&vfs, 2);
    assert!((at(&restored).0 - saved_at.0).abs() < 1.0 && (at(&restored).1 - saved_at.1).abs() < 1.0);
    assert_eq!(restored.session().unwrap().script.states[MONEY], 1234);
}

/// Exploding Lips runs through the generic GameModule interface too (the one the Quake-family
/// viewer implements): it ticks from ModuleInput and produces GPU views.
#[test]
fn lips00_as_a_game_module() {
    use engine_scene::module::{GameModule, ModuleInput};
    let Some((vfs, game)) = start("LIPS00") else { return };
    let mut m = godd_game::module::GoddModule::new(game, vfs);
    assert!(m.title().contains("Mortuary"));
    let mut ui = godd_game::Frame::new(320, 200);
    let before = m.view(&mut ui, 1.0).unwrap().camera;
    for _ in 0..30 {
        m.tick(&ModuleInput { forward: 1.0, look: (0.01, 0.0), ..Default::default() });
    }
    let after = m.view(&mut ui, 1.0).unwrap().camera;
    assert!(((after.pos[0] - before.pos[0]).powi(2) + (after.pos[1] - before.pos[1]).powi(2)).sqrt() > 50.0, "walked");
    assert!((after.angle - before.angle).abs() > 0.2, "turned left");
    assert!(!m.images().is_empty() && m.shade_rows().len() == 16);
}

/// Rumble: firing gives a short kick, losing life a heavier one; `joy_rumble 0` silences both.
#[test]
fn lips00_shots_and_damage_rumble() {
    use godd_game::{Action, Input, script::LIFE};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.5); // the start pad hands over the shotgun
    let _ = game.drain_haptics();
    let mut fire = Input::default();
    fire.set(Action::Use, true);
    for _ in 0..30 {
        game.tick(&vfs, &fire);
    }
    let shots = game.drain_haptics();
    assert!(shots.iter().any(|h| h.high > h.low && h.seconds < 0.2), "a shot kick: {shots:?}");
    run(&mut game, &vfs, 1.0);
    let _ = game.drain_haptics();
    game.session_mut().unwrap().script.states[LIFE] -= 20;
    run(&mut game, &vfs, 0.05);
    let hurt = game.drain_haptics();
    assert!(hurt.iter().any(|h| h.low >= 0.3 && h.seconds >= 0.2), "a damage rumble: {hurt:?}");
    game.cvars.set("joy_rumble", "0").unwrap();
    game.session_mut().unwrap().script.states[LIFE] -= 20;
    run(&mut game, &vfs, 0.05);
    assert!(game.drain_haptics().is_empty());
}

/// Occlusion: portal hops grow away from the listener's room; positional sounds carry a
/// muffle amount from them (0 nearby, up to 1 several rooms away).
#[test]
fn lips00_sounds_behind_walls_are_muffled() {
    use godd_game::AudioEvent;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let s = game.session().unwrap();
    let hops = s.portal_hops(s.player.region);
    assert_eq!(hops[s.player.region], 0);
    let reachable = |hops: &[u8]| hops.iter().filter(|&&h| h != u8::MAX).count();
    let closed = reachable(&hops);
    assert!(closed > 5 && hops.iter().any(|&h| (3..u8::MAX).contains(&h)), "{closed} reachable regions");
    let _ = game.drain_audio();
    game.session_mut().unwrap().fire_tag(53);
    run(&mut game, &vfs, 0.1);
    // A shut door (ceiling down on the floor) stops sound reaching through it.
    let next = hops.iter().position(|&h| h == 1).unwrap();
    let s = game.session_mut().unwrap();
    let saved = s.level.world.regions[next].ceiling;
    s.level.world.regions[next].ceiling = s.level.world.regions[next].floor;
    let shut = s.portal_hops(s.player.region);
    assert_eq!(shut[next], u8::MAX);
    assert!(reachable(&shut) < closed);
    s.level.world.regions[next].ceiling = saved;
    let muffles: Vec<f32> = game
        .drain_audio()
        .iter()
        .filter_map(|e| match e {
            AudioEvent::Sound { pos: Some(_), muffle, .. } => Some(*muffle),
            _ => None,
        })
        .collect();
    assert!(!muffles.is_empty() && muffles.iter().all(|m| (0.0..=1.0).contains(m)), "{muffles:?}");
}

/// HD HUD: on the GPU path the HUD text is recorded for the GPU (not rasterised into the UI
/// layer); with `hud_hd 0` it is drawn as pixels like before.
#[test]
fn lips00_hud_is_recorded_for_the_gpu() {
    use engine_core::UiCmd;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let (w, h) = game.ui_layer_size(1920, 1080);
    let mut ui = godd_game::Frame::new(w, h);
    game.gpu_frame(&mut ui, 1.0).unwrap();
    let list = ui.ui_list.as_ref().expect("recording");
    assert!(list.iter().filter(|c| matches!(c, UiCmd::Glyph { .. })).count() > 10, "HUD glyphs");
    assert!(list.iter().filter(|c| matches!(c, UiCmd::Rect { .. })).count() >= 4, "Magic and Experience bars");
    let drawn = |ui: &godd_game::Frame| ui.coverage.as_ref().unwrap().iter().filter(|&&c| c != 0).count();
    let hd_pixels = drawn(&ui);
    game.cvars.set("hud_hd", "0").unwrap();
    game.gpu_frame(&mut ui, 1.0).unwrap();
    assert!(ui.ui_list.is_none());
    assert!(drawn(&ui) > hd_pixels + 1000, "the HUD is in the pixel layer: {} vs {hd_pixels}", drawn(&ui));
}

/// The automap on the GPU path: walls as recorded lines and the player as a triangle over the
/// pixel backdrop and floors.
#[test]
fn lips00_automap_lines_are_recorded() {
    use engine_core::UiCmd;
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let mut open = Input::default();
    open.set(Action::Map, true);
    game.tick(&vfs, &open);
    let (w, h) = game.ui_layer_size(1920, 1080);
    let mut ui = godd_game::Frame::new(w, h);
    game.gpu_frame(&mut ui, 1.0).unwrap();
    let list = ui.ui_list.as_ref().unwrap();
    assert!(list.iter().filter(|c| matches!(c, UiCmd::Line { .. })).count() > 10, "walls");
    assert!(list.iter().any(|c| matches!(c, UiCmd::Triangle { .. })), "player arrow");
    assert!(ui.coverage.as_ref().unwrap().iter().all(|&c| c != 0), "backdrop covers the view");
}

/// Progression guides in LIPS00: the start room's way out is the switch at the end of its
/// north corridor; it is a guide (the ledge's two inert teleporter objects are not), the exit
/// switch is green, and guides glow in the GPU view (without any helper text).
#[test]
fn lips00_guides_show_the_way_out() {
    use godd_game::guides::{self, GuideKind};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    let s = game.session().unwrap();
    let found = guides::find(s);
    let near = |kind: GuideKind, x: f32, y: f32| found.iter().any(|g| g.kind == kind && (g.pos[0] - x).abs() < 40.0 && (g.pos[1] - y).abs() < 40.0);
    assert!(near(GuideKind::Switch, 11712.0, 16768.0), "start corridor switch");
    assert!(!found.iter().any(|g| g.kind == GuideKind::Teleport && (g.pos[1] - 16496.0).abs() < 40.0), "inert ledge objects are not guides");
    assert_eq!(found.iter().filter(|g| g.kind == GuideKind::Exit).count(), 1, "one exit switch");
    assert!(s.messages.is_empty(), "no helper text");
    let (w, h) = game.ui_layer_size(1280, 720);
    let mut ui = godd_game::Frame::new(w, h);
    let lights = game.gpu_frame(&mut ui, 1.0).unwrap().scene.lights.len();
    assert!(lights >= 1, "{lights} lights");
    game.cvars.set("hud_guides", "0").unwrap();
    assert_eq!(game.gpu_frame(&mut ui, 1.0).unwrap().scene.lights.len(), 0);
}

/// LIPS00's BSP world (re/engine/bsp-world.md §6.3): the start-room switch (tag 132) moves the
/// player into LIPS00C.BSP; the player lands on the floor there; reaching COFFIN1's exit cube
/// brings them back onto COFFIN1 in the orb, whose floor then rises out of the pit.
#[test]
fn lips00_bsp_world_round_trip() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    assert!(game.session().unwrap().bsp.is_some(), "LIPS00C.BSP loaded");
    game.session_mut().unwrap().fire_tag(132);
    let s = game.session().unwrap();
    assert!(s.in_bsp, "in the BSP world");
    let p = s.bsp.as_ref().unwrap().player;
    assert_eq!(p.pos, [2640.0, 832.0, 448.1]);
    assert!(s.player.angle.abs() < 1e-3 || (s.player.angle - std::f32::consts::TAU).abs() < 1e-3, "facing +X");
    let orb_before = (s.player.x, s.player.y);
    run(&mut game, &vfs, 0.5);
    let s = game.session().unwrap();
    let p = s.bsp.as_ref().unwrap().player;
    assert!((p.pos[2] - 424.0).abs() < 1.0 && !p.airborne, "landed on the floor: {:?}", p.pos);
    assert_eq!((s.player.x, s.player.y), orb_before, "the orb is frozen");
    // Walking moves the BSP player, not the orb one.
    let mut walk = godd_game::Input::default();
    walk.set(godd_game::Action::Forward, true);
    for _ in 0..30 {
        game.tick(&vfs, &walk);
    }
    let moved = game.session().unwrap().bsp.as_ref().unwrap().player.pos;
    assert!((moved[0] - 2640.0).abs() > 50.0 || (moved[1] - 832.0).abs() > 50.0, "walked: {moved:?}");
    // Step into COFFIN1's exit cube.
    let floor_before = {
        let s = game.session().unwrap();
        s.level.world.regions[26].floor
    };
    let entering = game.drain_audio();
    assert!(entering.iter().any(|e| matches!(e, godd_game::AudioEvent::Loop { name, .. } if name.eq_ignore_ascii_case("DUNGEON6"))), "catacombs background");
    game.session_mut().unwrap().bsp.as_mut().unwrap().player.pos = [-2384.0, -1120.0, 424.0];
    run(&mut game, &vfs, 0.1);
    let back = game.drain_audio();
    assert!(back.iter().any(|e| matches!(e, godd_game::AudioEvent::Music { name: Some(n), .. } if n.eq_ignore_ascii_case("TOCATTA"))), "COFFIN1's region music: {back:?}");
    let s = game.session().unwrap();
    assert!(!s.in_bsp, "back in the orb");
    assert_eq!((s.player.x, s.player.y, s.player.region), (17272.0, 16784.0, 26));
    assert!((s.player.angle - std::f32::consts::PI).abs() < 1e-3, "angle 1024: {}", s.player.angle);
    run(&mut game, &vfs, 1.0);
    let s = game.session().unwrap();
    assert!(s.level.world.regions[26].floor > floor_before + 10.0, "COFFIN1 rises: {} -> {}", floor_before, s.level.world.regions[26].floor);
}

/// The BSP world can be crossed on foot: a planner using the player's own ClipMove finds a
/// path from where ORPOLY puts the player to COFFIN1's exit, and steering along it with plain
/// forward movement gets there (stairs, shafts, ramps).
#[test]
fn lips00_bsp_world_walkable_to_the_exit() {
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    game.session_mut().unwrap().fire_tag(132);
    run(&mut game, &vfs, 0.5);
    let route = game.session().unwrap().bsp.as_ref().unwrap().path_to_exit().expect("a walkable path to the exit");
    println!("path: {} steps", route.len());
    let mut walk = Input::default();
    walk.set(Action::Forward, true);
    let idle = Input::default();
    let mut ticks = 0;
    'route: for (k, &target) in route.iter().enumerate().skip(1) {
        loop {
            let s = game.session_mut().unwrap();
            if !s.in_bsp {
                break 'route;
            }
            let w = s.bsp.as_mut().unwrap();
            let p = w.player.pos;
            let (dx, dy) = (target[0] - p[0], target[1] - p[1]);
            if dx.hypot(dy) < 6.0 {
                break;
            }
            // Face the next spot (forward is (−sin yaw, cos yaw)); no steering while falling.
            w.look((-dx).atan2(dy) - w.player.yaw, 0.0);
            let falling = w.player.airborne;
            game.tick(&vfs, if falling { &idle } else { &walk });
            ticks += 1;
            assert!(ticks < 60 * 300, "stuck before step {k} {target:?} at {p:?}");
        }
    }
    let s = game.session().unwrap();
    assert!(!s.in_bsp, "reached the exit");
    assert_eq!(s.player.region, 26);
    println!("crossed the BSP world in {:.1} s", ticks as f32 / 60.0);
}

/// Modern HUD: the score stats (time, tool, treasures, deaths, kills) appear only with the
/// automap open; the classic HUD keeps them on screen.
#[test]
fn lips00_score_panel_only_on_the_automap() {
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let texts = |game: &mut godd_game::Game| {
        let (w, h) = game.ui_layer_size(1920, 1080);
        let mut ui = godd_game::Frame::new(w, h);
        game.gpu_frame(&mut ui, 1.0).unwrap();
        ui.ui_list.unwrap().iter().filter(|c| matches!(c, engine_core::UiCmd::Glyph { .. })).count()
    };
    let plain = texts(&mut game);
    let mut open = Input::default();
    open.set(Action::Map, true);
    game.tick(&vfs, &open);
    let with_map = texts(&mut game);
    assert!(with_map > plain + 40, "stats panel on the map: {plain} -> {with_map}");
    game.cvars.set("hud_style", "classic").unwrap();
    game.tick(&vfs, &open);
    let classic = texts(&mut game);
    assert!(classic > plain + 40, "classic HUD shows the stats: {classic}");
}

/// LIPS00's music DUNGN10 ships only as a WAV recording: it loops as a sound instead of the
/// (missing) MIDI; a level start first stops the previous level's recorded music.
#[test]
fn lips00_recorded_music_plays() {
    use godd_game::AudioEvent;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.1);
    let events = game.drain_audio();
    assert!(events.iter().any(|e| matches!(e, AudioEvent::Loop { name, .. } if name.eq_ignore_ascii_case("DUNGN10"))), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, AudioEvent::Stop { .. })));
}

/// One background at a time, as in the original: the level's own (LIPS00: the DUNGN10
/// recording) until a region names another (FIRE01), and `BACKSND` regions bring it back.
#[test]
fn lips00_region_backgrounds_replace_each_other() {
    use godd_formats::xrb::{RecordExt, Section};
    use godd_game::AudioEvent;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.1);
    let s = game.session().unwrap();
    let named = |want: &str| {
        let sounds = s.level.orb.records(Section::Sounds);
        s.level.orb.records(Section::Regions).iter().position(|r| {
            usize::try_from(r.le_i32(0x100)).ok().and_then(|k| sounds.get(k)).is_some_and(|rec| String::from_utf8_lossy(&rec[0x0c..0x18]).trim_end_matches('\0').eq_ignore_ascii_case(want))
        })
    };
    let (fire, back) = (named("FIRE01").unwrap(), named("BACKSND").unwrap());
    let _ = game.drain_audio();
    let s = game.session_mut().unwrap();
    s.region_background(fire);
    let events = std::mem::take(&mut s.audio);
    assert!(events.iter().any(|e| matches!(e, AudioEvent::Stop { channel: 999 })), "the level's background stops: {events:?}");
    assert!(events.iter().any(|e| matches!(e, AudioEvent::Loop { name, .. } if name == "FIRE01")));
    s.region_background(fire);
    assert!(s.audio.is_empty(), "the same background is not restarted");
    s.region_background(back);
    let events = std::mem::take(&mut s.audio);
    assert!(events.iter().any(|e| matches!(e, AudioEvent::Loop { name, .. } if name.eq_ignore_ascii_case("DUNGN10"))), "BACKSND restores the level's own: {events:?}");
}

/// A door opened by walking into its face (LIPS00 wall 460, tag 30) opens even though the
/// player is standing on the open doorway line behind it (wall 39): every wall in contact is
/// touched, not only the nearest (world-effects.md §1.1). Found by the play bot.
#[test]
fn lips00_door_face_opens_from_a_doorway() {
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    let s = game.session_mut().unwrap();
    (s.player.x, s.player.y, s.player.z) = (17920.0, 16640.0, 0.0);
    s.player.region = s.level.world.region_at_z(17920.0, 16640.0, Some(1.0)).unwrap();
    s.player.angle = -std::f32::consts::FRAC_PI_2;
    s.previous = s.player;
    let mut walk = Input::default();
    walk.set(Action::Forward, true);
    for _ in 0..120 {
        game.tick(&vfs, &walk);
    }
    let s = game.session().unwrap();
    assert!(s.level.world.regions[81].ceiling > 64.0, "door 81 opened");
    assert!(s.player.y < 16500.0, "walked through: {}", s.player.y);
}

/// Put the player at (x, y) on the floor there, facing `angle`.
fn place(game: &mut godd_game::Game, x: f32, y: f32, angle: f32) {
    let s = game.session_mut().unwrap();
    let w = &s.level.world;
    let r = w.region_at(x, y).unwrap();
    s.player.x = x;
    s.player.y = y;
    s.player.region = r;
    s.player.z = w.regions[r].floor;
    s.player.vz = 0.0;
    s.player.angle = angle;
    s.previous = s.player;
}

/// Hold `actions` for `ticks`, keeping the facing.
fn hold(game: &mut godd_game::Game, vfs: &godd_game::Vfs, actions: &[godd_game::Action], ticks: usize, angle: f32) {
    let mut input = godd_game::Input::default();
    for &a in actions {
        input.set(a, true);
    }
    for _ in 0..ticks {
        game.session_mut().unwrap().player.angle = angle;
        game.tick(vfs, &input);
    }
}

/// Solid walls hold the body radius + 12 away (physics.md §8.1), so LIPS00's 16-deep elevator
/// button alcove (r134) cannot be stepped into: pressing "up" from the elevator rides it up
/// instead of leaving the player stranded under it.
#[test]
fn lips00_elevator_button_keeps_the_player_on_the_elevator() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    game.session_mut().unwrap().fire_tag(24); // elevator down
    run(&mut game, &vfs, 14.0);
    place(&mut game, 16800.0, 15936.0, 0.0);
    hold(&mut game, &vfs, &[Action::Forward], 60, 0.0);
    let s = game.session().unwrap();
    assert!(s.player.x < 16896.0, "x = {}", s.player.x);
    assert_eq!(s.player.region, 76, "on the elevator, not in the alcove");
    run(&mut game, &vfs, 12.0);
    let s = game.session().unwrap();
    assert!(s.player.z > -100.0, "rode up with the elevator: z = {}", s.player.z);
}

/// A body standing right against a blocking line is pushed back to the side it came from,
/// never through the walls beyond (it used to pop 100 units north into LIPS00's pit r249).
#[test]
fn lips00_pushes_never_tunnel_through_walls() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    place(&mut game, 17793.5, 16927.7, std::f32::consts::FRAC_PI_2);
    hold(&mut game, &vfs, &[Action::Forward], 30, std::f32::consts::FRAC_PI_2);
    let s = game.session().unwrap();
    assert!(s.player.y < 16928.0, "y = {}", s.player.y);
    assert_eq!(s.player.region, 0);
}

/// A plain OWALK bot fires its own tag when the player bumps it (object-effects.md
/// FUN_0045fe91): LIPS00's tag-71 walker sends the player to the raft (OTELE on tag 71).
#[test]
fn lips00_plain_walker_fires_its_tag_when_bumped() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    let (x, y) = {
        let o = &game.session().unwrap().objects.list[243];
        assert_eq!((o.effect, o.tag), (5, 71));
        (o.x, o.y)
    };
    place(&mut game, x - 60.0, y, 0.0);
    hold(&mut game, &vfs, &[Action::Forward], 40, 0.0);
    run(&mut game, &vfs, 0.5);
    // CHEIRAFT rides the raft path, so only its row is fixed.
    let p = game.session().unwrap().player;
    assert!((p.x - x).hypot(p.y - y) > 1000.0 && (p.y - 16736.0).abs() < 64.0, "teleported to CHEIRAFT: at ({}, {})", p.x, p.y);
}

/// Teleporting re-homes the player and sends leave/enter region events: LIPS00's pit alcove
/// r483 opens SECRET2 (r12, tag 57) when left, and the death teleport is the way out of the pit.
#[test]
fn lips00_teleport_out_of_a_region_fires_its_leave_tag() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    place(&mut game, 16768.0, 15808.0, 0.0);
    run(&mut game, &vfs, 0.2);
    assert_eq!(game.session().unwrap().player.region, 483);
    game.session_mut().unwrap().fire_tag(3); // DEATH01 → RBIRTH01
    run(&mut game, &vfs, 4.0);
    let s = game.session().unwrap();
    assert_ne!(s.player.region, 483);
    let r = &s.level.world.regions[12];
    assert!(r.ceiling - r.floor > 56.0, "SECRET2 open: {} / {}", r.floor, r.ceiling);
}

/// Crossing onto a raised region nested in another (LIPS00's ELIXIR02 pedestal, 48 up)
/// re-homes the body onto it once a jump has it within step height (physics.md §8.2).
#[test]
fn lips00_jump_onto_the_elixir_pedestal() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    let west = std::f32::consts::PI;
    place(&mut game, 16240.0, 14816.0, west);
    hold(&mut game, &vfs, &[Action::Forward, Action::Jump], 2, west);
    hold(&mut game, &vfs, &[Action::Forward], 40, west);
    let s = game.session().unwrap();
    assert_eq!(s.player.region, 611, "at ({}, {}, {})", s.player.x, s.player.y, s.player.z);
    assert!((s.player.z + 112.0).abs() < 1.0);
}

/// LIPS00's FALLZ trap: walking straight in, the sinking floor carries the player down (no
/// landing, so no death tag), and a jump gets them out at the far end.
#[test]
fn lips00_fallz_rides_down_and_jumps_out() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    let south = -std::f32::consts::FRAC_PI_2;
    place(&mut game, 16544.0, 16100.0, south);
    hold(&mut game, &vfs, &[Action::Forward], 100, south);
    let s = game.session().unwrap();
    assert_eq!(s.player.region, 466, "still in FALLZ, not sent back to the start");
    assert!((s.player.z + 448.0).abs() < 1.0);
    hold(&mut game, &vfs, &[Action::Forward, Action::Jump], 40, south);
    assert_eq!(game.session().unwrap().player.region, 178);
}

/// An OSETF fired while its region is still moving is queued and runs after the move
/// (object-effects.md OSETF "pending"): LIPS00's elevator called up mid-descent comes back up.
#[test]
fn lips00_elevator_call_during_a_ride_is_queued() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    game.session_mut().unwrap().fire_tag(24); // down
    run(&mut game, &vfs, 1.0);
    game.session_mut().unwrap().fire_tag(25); // up, while still going down
    run(&mut game, &vfs, 30.0);
    let r = &game.session().unwrap().level.world.regions[76];
    assert!(r.floor.abs() < 0.5, "elevator floor {}", r.floor);
}

/// g_fixes (default on): LIPS00's FALLZ floor, lowered by stepping onto its trigger strip and
/// back out, rises again once empty; classic behaviour leaves it down for good.
#[test]
fn lips00_fallz_trap_rearms_with_fixes() {
    use godd_game::Action;
    for fixes in [true, false] {
        let Some((vfs, mut game)) = start("LIPS00") else { return };
        assert!(game.exec_config(&format!("set g_fixes {}", fixes as u8)).is_empty());
        run(&mut game, &vfs, 0.3);
        let south = -std::f32::consts::FRAC_PI_2;
        place(&mut game, 16544.0, 16020.0, south);
        hold(&mut game, &vfs, &[Action::Forward], 6, south); // onto the trigger strip
        hold(&mut game, &vfs, &[Action::Back], 30, south); // and back out
        run(&mut game, &vfs, 1.0);
        assert!(game.session().unwrap().level.world.regions[466].floor < -440.0, "trap sprung");
        run(&mut game, &vfs, 8.0);
        let floor = game.session().unwrap().level.world.regions[466].floor;
        if fixes {
            assert!((floor + 384.0).abs() < 0.5, "re-armed: {floor}");
        } else {
            assert!(floor < -440.0, "classic stays down: {floor}");
        }
    }
}

/// g_fixes: walking through LIPS00's door r458 (a switch face on each side) no longer touches
/// the far face shut; classic behaviour closes it on the way through.
#[test]
fn lips00_door_far_face_does_not_shut_it_with_fixes() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    game.session_mut().unwrap().fire_tag(86);
    run(&mut game, &vfs, 3.0);
    let open = |g: &godd_game::Game| { let r = &g.session().unwrap().level.world.regions[458]; r.ceiling - r.floor };
    assert!(open(&game) > 56.0, "opened: {}", open(&game));
    // Touch the far face (wall 364, door side r184) the way walking through does.
    game.session_mut().unwrap().effects.on_player_events(&[godd_game::player::PlayerEvent::TouchedWall(364)]);
    run(&mut game, &vfs, 1.0);
    assert!(open(&game) > 56.0, "still open with fixes: {}", open(&game));
}

/// The pause menu (tag 620, Esc) has an "Options" item (above Quit) that opens the options screen.
#[test]
fn pause_menu_opens_options() {
    use godd_game::{Action, Input};
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    let press = |game: &mut godd_game::Game, a: Action| {
        let mut i = Input::default();
        i.set(a, true);
        game.tick(&vfs, &i);
        game.tick(&vfs, &Input::default());
    };
    press(&mut game, Action::Menu);
    assert!(game.session().unwrap().menus.is_open(), "pause menu open");
    press(&mut game, Action::MenuUp); // wraps to the last item, Quit
    press(&mut game, Action::MenuUp); // Options
    press(&mut game, Action::MenuSelect);
    run(&mut game, &vfs, 0.1);
    assert!(game.options_menu.open, "options screen open");
}

/// Bodies collide with the walls of their own region only and change region by crossing its
/// portal lines (physics.md §8.1/8.2): LIPS12's corridor r37 runs under a raised walkway that
/// is solid from the room around it, and the player walks its length to door r94.
#[test]
fn lips12_corridor_under_the_walkway() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS12") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (15340.0, 15650.0, 0.0, 37);
        s.player.angle = 0.0;
        s.previous = s.player;
    }
    hold(&mut game, &vfs, &[Action::Forward], 120, 0.0);
    let p = game.session().unwrap().player;
    assert!(p.x > 16000.0, "walked under the walkway: x = {} region {}", p.x, p.region);
}

/// g_fixes: LIPS02's sealed west room (pickup for door r13) is reached through the block r24
/// in the corridor wall; classic data keeps its walls solid (the level cannot be finished).
#[test]
fn lips02_secret_passage_with_fixes() {
    use godd_game::Action;
    for fixes in [true, false] {
        let Some((vfs, mut game)) = start("LIPS02") else { return };
        assert!(game.exec_config(&format!("set g_fixes {}", fixes as u8)).is_empty());
        run(&mut game, &vfs, 0.3);
        let west = std::f32::consts::PI;
        {
            let s = game.session_mut().unwrap();
            (s.player.x, s.player.y, s.player.z, s.player.region) = (26272.0, 18112.0, 128.0, 25);
            s.previous = s.player;
        }
        hold(&mut game, &vfs, &[Action::Forward], 60, west);
        let p = game.session().unwrap().player;
        if fixes {
            assert!(p.x < 26100.0 && p.region == 25, "through the passage: ({}, {}) r{}", p.x, p.y, p.region);
        } else {
            assert!(p.x > 26176.0, "classic: still in the corridor at x = {}", p.x);
        }
    }
}

/// A body on a region's corner (off its polygon) never drops into a region under it whose
/// space it does not fit: LIPS00's start room corner used to hand the player to the cellar
/// r521 below, and the ceiling clamp dragged them 88 units down through the floor.
#[test]
fn lips00_corner_never_drops_into_the_cellar() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (17856.2, 16960.0, 0.0, 0);
        s.previous = s.player;
    }
    let angle = (16992.0f32 - 16960.0).atan2(17888.0 - 17856.2);
    hold(&mut game, &vfs, &[godd_game::Action::Forward], 30, angle);
    let p = game.session().unwrap().player;
    assert!(p.z > -1.0 && p.region != 521, "at ({}, {}, {}) r{}", p.x, p.y, p.z, p.region);
}

/// LIPS06's green lock: walking into r71 with GREENKEY fires tag 62 (DROPGRN), which opens
/// the green door r147.
#[test]
fn lips06_green_key_opens_the_green_door() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS06") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        let key = s.objects.list.iter().position(|o| o.name == "GREENKEY").unwrap();
        s.objects.list[key].hidden = true;
        s.objects.inventory.push(key);
        let z = s.level.world.regions[77].floor;
        (s.player.x, s.player.y, s.player.z, s.player.region) = (15200.0, 14950.0, z, 77);
        s.previous = s.player;
    }
    hold(&mut game, &vfs, &[Action::Forward], 60, 0.0);
    run(&mut game, &vfs, 3.0);
    let s = game.session().unwrap();
    let door = &s.level.world.regions[147];
    assert!(door.ceiling > door.floor + 60.0, "green door r147: floor {} ceiling {}; player r{}", door.floor, door.ceiling, s.player.region);
}

/// LIPS09's door r533 (toggle, tag 22) is opened by the thin trigger strip r532 in front of it
/// and by r257 behind; walking at it from r169 gets the player through.
#[test]
fn lips09_thin_strip_door_lets_the_player_through() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS09") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (13248.0, 18740.0, 4.0, 169);
        s.previous = s.player;
    }
    let south = -std::f32::consts::FRAC_PI_2;
    hold(&mut game, &vfs, &[Action::Forward], 120, south);
    let p = game.session().unwrap().player;
    assert!(p.y < 18680.0, "through the door: ({}, {}) r{}", p.x, p.y, p.region);
}

/// LIPS09's door r543 (toggle, tag 25): its trigger strip r542 is a step (32) up from the
/// room r540 and only 16 deep; walking at it opens the door and the player gets through (to
/// the teleporter behind it).
#[test]
fn lips09_raised_strip_door_lets_the_player_through() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS09") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (15888.0, 19060.0, 0.0, 540);
        s.previous = s.player;
    }
    let south = -std::f32::consts::FRAC_PI_2;
    hold(&mut game, &vfs, &[Action::Forward], 120, south);
    let p = game.session().unwrap().player;
    assert!(p.y < 18960.0 || p.region != 540, "through the door: ({}, {}) r{}", p.x, p.y, p.region);
}

/// Running at LIPS09's strip r532 (16 deep, door r533 shut behind it): the body crosses into
/// the strip and the door pushes it back out within one tick, which is still a re-home into
/// it, so its enter tag opens the door (world-effects.md §1: enter fires on every re-home).
#[test]
fn lips09_running_at_a_thin_strip_opens_its_door() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS09") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (13248.0, 18900.0, 0.0, 170);
        s.previous = s.player;
    }
    hold(&mut game, &vfs, &[Action::Forward, Action::Run], 120, -std::f32::consts::FRAC_PI_2);
    let p = game.session().unwrap().player;
    assert!(p.y < 18680.0, "ran through the door: ({}, {}) r{}", p.x, p.y, p.region);
}

/// g_fixes: firing in LIPS00's BSP world plays the tool's use animation (the orb, and its
/// tool, is frozen there, so in the original nothing at all shows); classic keeps it still.
#[test]
fn lips00_firing_in_the_bsp_world_animates_the_tool_with_fixes() {
    for fixes in [true, false] {
        let Some((vfs, mut game)) = start("LIPS00") else { return };
        assert!(game.exec_config(&format!("set g_fixes {}", fixes as u8)).is_empty());
        run(&mut game, &vfs, 0.2);
        game.session_mut().unwrap().fire_tag(132);
        run(&mut game, &vfs, 0.5);
        let s = game.session().unwrap();
        assert!(s.in_bsp);
        let idle = s.objects.weapon_pic(&s.level);
        let mut fire = godd_game::Input::default();
        fire.set(godd_game::Action::Use, true);
        let mut changed = false;
        for _ in 0..60 {
            game.tick(&vfs, &fire);
            let s = game.session().unwrap();
            changed |= s.objects.weapon_pic(&s.level) != idle;
        }
        assert_eq!(changed, fixes, "fixes {fixes}: tool animated {changed}");
    }
}

/// Death bots' growls (their mover script's OSOUND) come from the bot, not flat in the
/// player's ear: back from LIPS00's BSP world, the bots in the rooms round the coffin shaft
/// growl every 1.2 s (no line-of-sight test, movers.md), which used to play at full volume.
#[test]
fn lips00_bot_growls_are_positional() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    // (Classic: with g_fixes, bots behind walls out of earshot are not heard at all.)
    assert!(game.exec_config("set g_fixes 0").is_empty());
    run(&mut game, &vfs, 0.2);
    game.session_mut().unwrap().fire_tag(132);
    run(&mut game, &vfs, 0.5);
    game.session_mut().unwrap().bsp.as_mut().unwrap().player.pos = [-2384.0, -1120.0, 424.0];
    run(&mut game, &vfs, 4.0);
    let audio = game.drain_audio();
    let growls: Vec<_> = audio.iter().filter_map(|e| match e {
        godd_game::AudioEvent::Sound { name, pos, .. } if name == "GROWL07" || name == "GROWL14" => Some(*pos),
        _ => None,
    }).collect();
    assert!(!growls.is_empty(), "the bots growl");
    assert!(growls.iter().all(Option::is_some), "every growl has a position: {growls:?}");
}

/// LIPS06's corridor r53 is 64 wide with a chamfered corner (wall 103): walking into it, the
/// two walls' pushes fight, and the body used to be shoved out through the diagonal (the view
/// then sees out of the level). A push that lands outside the region undoes the step.
#[test]
fn lips06_chamfered_corner_never_pushes_the_body_out() {
    use godd_game::Action;
    // From inside the corridor, and from r80 through the foot of the portal (the playbot's
    // way: re-homed into r53 beyond its diagonal).
    for (from, target) in [((14880.0f32, 16300.0f32, 16.0f32, 53usize), (14850.0f32, 16050.0f32)), ((14880.0, 16300.0, 16.0, 53), (14840.0, 16100.0)), ((14944.0, 16081.0, 0.0, 80), (14887.0, 16078.0)), ((14944.0, 16081.0, 0.0, 80), (14860.0, 16060.0))] {
        let Some((vfs, mut game)) = start("LIPS06") else { return };
        run(&mut game, &vfs, 0.3);
        {
            let s = game.session_mut().unwrap();
            (s.player.x, s.player.y, s.player.z, s.player.region) = from;
            s.previous = s.player;
        }
        for _ in 0..60 {
            let p = game.session().unwrap().player;
            let angle = (target.1 - p.y).atan2(target.0 - p.x);
            hold(&mut game, &vfs, &[Action::Forward, Action::Run], 1, angle);
            let s = game.session().unwrap();
            let p = s.player;
            assert!(s.level.world.point_in_region(p.x, p.y, p.region), "toward {target:?}: out of r{} at ({}, {})", p.region, p.x, p.y);
        }
    }
}

/// Letting go of crouch in a space too low to stand (LIPS12's 39-high pickup nook r160) keeps
/// the body crouched (physics.md §5.4), so the eye never pokes out above the ceiling.
#[test]
fn lips12_no_standing_up_under_a_low_ceiling() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS12") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (16384.0, 15552.0, 33.0, 160);
        s.player.crouching = true;
        s.previous = s.player;
    }
    hold(&mut game, &vfs, &[Action::Crouch], 10, 0.0);
    run(&mut game, &vfs, 1.0);
    let s = game.session().unwrap();
    let eye = s.player.eye_z(&s.physics_override.unwrap_or_default());
    assert!(s.player.crouching && eye < s.level.world.regions[160].ceiling, "eye {eye} under the ceiling {}", s.level.world.regions[160].ceiling);
}

/// g_fixes: death bots out of earshot (behind walls; near bots attack through them with no
/// line-of-sight test, movers.md) are not heard. At LIPS02's start four of them growled and
/// fired every 1.2 s forever; classic keeps them.
#[test]
fn lips02_bots_behind_walls_are_not_heard_with_fixes() {
    for fixes in [true, false] {
        let Some((vfs, mut game)) = start("LIPS02") else { return };
        assert!(game.exec_config(&format!("set g_fixes {}", fixes as u8)).is_empty());
        run(&mut game, &vfs, 10.0);
        let growls = game.drain_audio().iter().filter(|e| matches!(e, godd_game::AudioEvent::Sound { name, .. } if name.starts_with("GROWL"))).count();
        assert_eq!(growls == 0, fixes, "fixes {fixes}: {growls} growls");
    }
}

/// LIPS01's doors r8 and r39 are locked from the way in: their faces (walls 112 and 231) only
/// knock until DUCK3 is picked up, whose tag 108 makes OWTOWs copy the door switches (DOR1OPEN,
/// DOORYES) onto them. Walking into each face does nothing before that and opens its door after.
#[test]
fn lips01_door_faces_unlock_with_the_duck() {
    use godd_game::Action;
    // (start x, y, region, facing, door)
    let cases = [(14784.0f32, 16080.0f32, 49usize, std::f32::consts::FRAC_PI_2, 8usize), (15472.0, 15808.0, 51, std::f32::consts::PI, 39)];
    for duck in [false, true] {
        for &(x, y, region, angle, door) in &cases {
            let Some((vfs, mut game)) = start("LIPS01") else { return };
            run(&mut game, &vfs, 0.3);
            {
                let s = game.session_mut().unwrap();
                if duck {
                    s.fire_tag(108);
                }
                let z = s.level.world.regions[region].floor;
                (s.player.x, s.player.y, s.player.z, s.player.region) = (x, y, z, region);
                s.previous = s.player;
            }
            run(&mut game, &vfs, 0.1);
            hold(&mut game, &vfs, &[Action::Forward], 40, angle);
            run(&mut game, &vfs, 1.0);
            let s = game.session().unwrap();
            let d = &s.level.world.regions[door];
            assert_eq!(d.ceiling > d.floor + 48.0, duck, "duck {duck}, door r{door}: floor {} ceiling {}", d.floor, d.ceiling);
        }
    }
}

/// LIPS01's pickup #257 (the one starting the niche chain) lies in a corner of a 24-deep niche
/// behind door r309: walls keep the body 20 off it one way and 12 the other. Things that stay put
/// are touched by the original's |dx|, |dy| box, so walking in picks it up.
#[test]
fn lips01_niche_pickup_reached() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS01") else { return };
    game.session_mut().unwrap().fire_tag(118);
    run(&mut game, &vfs, 5.0);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (14984.0, 16964.0, 672.0, 310);
        s.previous = s.player;
    }
    hold(&mut game, &vfs, &[Action::Forward], 40, 0.0);
    assert!(game.session().unwrap().objects.inventory.contains(&257), "#257 picked up");
}

/// LIPS14's rotating door r21 (the only way between the two halves of r15) turns when its face
/// is walked into, with fixes on too: the g_fixes rule that an open door's own face does not
/// shut it is for sliding doors only (a rotating door is always full height, so it counted as
/// open and its faces never turned it).
#[test]
fn lips14_rotating_door_turns_from_its_face() {
    use godd_game::Action;
    for fixes in [true, false] {
        let Some((vfs, mut game)) = start("LIPS14") else { return };
        assert!(game.exec_config(&format!("set g_fixes {}", fixes as u8)).is_empty());
        run(&mut game, &vfs, 0.3);
        let before = {
            let s = game.session_mut().unwrap();
            (s.player.x, s.player.y, s.player.z, s.player.region) = (16704.0, 18640.0, 32.0, 15);
            s.previous = s.player;
            let w = &s.level.world;
            w.vertices[w.walls[101].b]
        };
        hold(&mut game, &vfs, &[Action::Forward], 70, std::f32::consts::FRAC_PI_2);
        run(&mut game, &vfs, 6.0);
        let w = &game.session().unwrap().level.world;
        let after = w.vertices[w.walls[101].b];
        assert!((after.0 - before.0).hypot(after.1 - before.1) > 32.0, "fixes {fixes}: door r21 turned ({before:?} -> {after:?})");
    }
}

/// OSETJUMP in a level's INI sets the jump (physics.md 5): LIPS03 jumps at 250, others at 200.
/// LIPS03's pickup #12 (it opens door r397 on the way to the GOLD that lowers the exit bars)
/// hangs 112 over the floor of r9: a 250 jump reaches it, a 200 one does not.
#[test]
fn osetjump_sets_the_level_jump() {
    for (level, speed) in [("LIPS03", 250.0f32), ("LIPS00", 200.0)] {
        let Some((vfs, mut game)) = start(level) else { return };
        run(&mut game, &vfs, 0.3);
        let s = game.session().unwrap();
        assert_eq!(s.physics_override.unwrap_or_default().jump_speed, speed, "{level}");
    }
    // Jumping under #12 picks it up.
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS03") else { return };
    run(&mut game, &vfs, 0.3);
    {
        let s = game.session_mut().unwrap();
        (s.player.x, s.player.y, s.player.z, s.player.region) = (16096.0, 15240.0, 0.0, 9);
        s.previous = s.player;
    }
    for _ in 0..3 {
        hold(&mut game, &vfs, &[Action::Jump], 2, 0.0);
        run(&mut game, &vfs, 1.0);
    }
    assert!(game.session().unwrap().objects.inventory.contains(&12), "#12 picked up by jumping");
}

/// The route guide follows the player: entering the region of the next step, then pressing the
/// wall switch of the one after, moves it on (LIPS04's way: the lift region r16, the exit 1342).
#[test]
fn route_guide_follows_progress() {
    use godd_game::route::{Route, RouteStep, RouteTarget};
    let Some((vfs, mut game)) = start("LIPS04") else { return };
    let route = Route {
        steps: vec![
            RouteStep { target: RouteTarget::Region(16), pos: [16000.0, 16256.0, -488.0], trail: vec![] },
            RouteStep { target: RouteTarget::Wall(1342), pos: [9664.0, 15400.0, 0.0], trail: vec![] },
        ],
    };
    game.routes.insert("LIPS04".into(), route);
    run(&mut game, &vfs, 0.3);
    assert_eq!(game.route_next(), Some(0));
    // (As walking in would: the region's enter event.)
    game.session_mut().unwrap().effects.enter_log.push(16);
    run(&mut game, &vfs, 0.1);
    assert_eq!(game.route_next(), Some(1), "entered r16");
    game.session_mut().unwrap().effects.touch_log.push(1342);
    run(&mut game, &vfs, 0.1);
    assert_eq!(game.route_next(), None, "pressed the exit: route done");
}

/// Placed where there is no room to stand (a quicksave loaded in LIPS00's 48-high vent r425),
/// the player is crouched, not standing with the eye above the ceiling (seeing out of the map).
#[test]
fn placed_in_a_vent_the_player_crouches() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    run(&mut game, &vfs, 0.2);
    game.session_mut().unwrap().restore_position(16547.93, 16388.0, -400.0, 5.02);
    run(&mut game, &vfs, 0.5);
    let s = game.session().unwrap();
    assert_eq!(s.player.region, 425);
    assert!(s.player.crouching, "crouched in the vent");
    let eye = s.player.eye_z(&s.physics_override.unwrap_or_default());
    assert!(eye < s.level.world.regions[425].ceiling, "the eye under the ceiling: {eye}");
}

/// LIPS00's coffin (r26) is seen from the room behind it through many small openings, more
/// than the window cap: its ceiling showed a black coffin-shaped hole. Every part of that
/// ceiling in view must fall in one of its windows.
#[test]
fn lips00_coffin_ceiling_inside_its_windows() {
    let Some(dir) = original_dir() else { return };
    let xrb = std::fs::read(dir.join("LIPS00.XRB")).unwrap();
    let xpg = std::fs::read(dir.join("LIPS00.XPG")).unwrap();
    let level = Level::load(&xrb, &xpg).unwrap();
    let w = &level.world;
    let (x, y, z, angle) = (17408.0f32, 16800.0f32, 48.0f32, 6.1359224 + std::f32::consts::PI);
    let region = w.region_at_z(x, y, Some(1.0)).unwrap();
    let tan_v = (48.9f32.to_radians() / 2.0).tan();
    let tan_h = tan_v * 3440.0 / 1440.0;
    let pitch = 0.8f32; // looking up at it (a tangent, as the camera's)
    let cam = godd_game::render::Camera { x, y, z, angle, pitch, region };
    let windows = godd_game::scene::visible_regions(w, &cam, (tan_h, tan_v), true);
    let coffin = &w.regions[26];
    let (sin, cos) = angle.sin_cos();
    let (psin, pcos) = pitch.atan().sin_cos();
    let project = |px: f32, py: f32, pz: f32| {
        let (rx, ry) = (px - x, py - y);
        let (f, up) = (rx * cos + ry * sin, pz - z);
        let (f, up) = (f * pcos + up * psin, up * pcos - f * psin);
        [(rx * sin - ry * cos) / f / tan_h, up / f / tan_v]
    };
    let inside = |win: &Vec<[f32; 2]>, p: [f32; 2]| {
        (0..win.len()).all(|i| {
            let (a, b) = (win[i], win[(i + 1) % win.len()]);
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-4
        })
    };
    let ((x0, y0), (x1, y1)) = coffin.bounds;
    let mut checked = 0;
    for i in 1..20 {
        for j in 1..20 {
            let (px, py) = (x0 + (x1 - x0) * i as f32 / 20.0, y0 + (y1 - y0) * j as f32 / 20.0);
            if w.region_at_z(px, py, Some(coffin.ceiling - 1.0)) != Some(26) {
                continue;
            }
            let p = project(px, py, coffin.ceiling);
            if p[0].abs() > 1.0 || p[1].abs() > 1.0 {
                continue;
            }
            checked += 1;
            assert!(windows[26].iter().any(|win| inside(win, p)), "coffin ceiling at ({px}, {py}) -> {p:?} outside its {} windows", windows[26].len());
        }
    }
    assert!(checked > 20, "the coffin ceiling is in view ({checked} points)");
}

/// LIPS00's street seen past the canteen sign's post: the streets beyond are reached through
/// regions already holding the most windows, and were not walked on from (a black slab under
/// the sign). The far street (r83) must have a window there.
#[test]
fn lips00_street_past_the_canteen_post() {
    let Some(dir) = original_dir() else { return };
    let xrb = std::fs::read(dir.join("LIPS00.XRB")).unwrap();
    let xpg = std::fs::read(dir.join("LIPS00.XPG")).unwrap();
    let level = Level::load(&xrb, &xpg).unwrap();
    let w = &level.world;
    let (x, y) = (18183.863f32, 17053.113f32);
    let region = w.region_at_z(x, y, Some(1.0)).unwrap();
    let tan_v = (48.9f32.to_radians() / 2.0).tan();
    let cam = godd_game::render::Camera { x, y, z: 48.0, angle: 0.785, pitch: 0.0, region };
    let windows = godd_game::scene::visible_regions(w, &cam, (tan_v * 1400.0 / 590.0, tan_v), true);
    let p = [0.37f32, 0.3];
    let inside = |win: &Vec<[f32; 2]>| {
        (0..win.len()).all(|i| {
            let (a, b) = (win[i], win[(i + 1) % win.len()]);
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= 0.0
        })
    };
    assert!(windows[83].iter().any(inside), "r83 behind the post: {:?}", windows[83]);
}
