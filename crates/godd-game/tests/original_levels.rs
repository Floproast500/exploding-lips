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
