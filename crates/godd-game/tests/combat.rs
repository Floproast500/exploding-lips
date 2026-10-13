//! Combat: bots, shots and hit boxes against the original game data. Tests skip themselves
//! without original/.

use std::path::PathBuf;

fn start(level: &str) -> Option<(godd_game::Vfs, godd_game::Game)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../original");
    if !dir.join("LIPS00.XRB").exists() {
        return None;
    }
    let vfs = godd_game::Vfs::from_dir(&dir).unwrap();
    let game = godd_game::Game::new(&vfs, &godd_game::Options { start_level: Some(level.into()), save: None }).unwrap();
    Some((vfs, game))
}

/// Bot shots fired in the first `seconds` (spawn spot, where each ended, its distance to the
/// player then).
fn bot_shots(level: &str, fixes: bool, seconds: f64) -> Vec<([f32; 2], [f32; 2], f32)> {
    bot_shots_with(level, fixes, seconds, |_| {})
}

fn bot_shots_with(level: &str, fixes: bool, seconds: f64, setup: impl FnOnce(&mut godd_game::Game)) -> Vec<([f32; 2], [f32; 2], f32)> {
    let Some((vfs, mut game)) = start(level) else { return Vec::new() };
    assert!(game.exec_config(&format!("set g_fixes {}", fixes as u8)).is_empty());
    game.tick(&vfs, &godd_game::Input::default());
    setup(&mut game);
    let mut live: std::collections::HashMap<usize, [f32; 2]> = std::collections::HashMap::new();
    let mut shots = Vec::new();
    for _ in 0..(seconds * godd_game::TICK_RATE) as usize {
        game.tick(&vfs, &godd_game::Input::default());
        let s = game.session().unwrap();
        for (i, o) in s.objects.list.iter().enumerate() {
            if o.name != "_PROJ_" {
                continue;
            }
            if !o.destroyed {
                live.entry(i).or_insert([o.x, o.y]);
            } else if let Some(from) = live.remove(&i) {
                shots.push((from, [o.x, o.y], (o.x - s.player.x).hypot(o.y - s.player.y)));
            }
        }
    }
    shots
}

/// g_fixes: bots shoot only at a player they can see. Near the starts of LIPS01 and LIPS02 the
/// original's bots (no line-of-sight test) fire through the walls every 1.2 s; none of those
/// shots can reach the player, they blow up on the walls in front of the bots.
#[test]
fn bots_hold_fire_without_line_of_sight_with_fixes() {
    for level in ["LIPS01", "LIPS02"] {
        let classic = bot_shots(level, false, 20.0);
        let fixed = bot_shots(level, true, 20.0);
        if classic.is_empty() && fixed.is_empty() && start(level).is_none() {
            return;
        }
        assert!(classic.len() > 5 && classic.iter().all(|s| s.2 > 64.0), "{level} classic: {} shots through walls", classic.len());
        assert!(fixed.is_empty(), "{level} fixes: {} shots", fixed.len());
    }
}

/// A bot that sees the player still shoots, and hits: stand in the open in front of LIPS01's
/// second death bot (#331).
#[test]
fn a_bot_in_view_still_shoots_with_fixes() {
    let shots = bot_shots_with("LIPS01", true, 10.0, |game| {
        let s = game.session_mut().unwrap();
        s.god = true;
        let b = &s.objects.list[331];
        assert_eq!(b.name, "DETHBOT1");
        let (x, y) = (b.x + 150.0, b.y + 120.0);
        let r = s.level.world.region_at(x, y).unwrap();
        let z = s.level.world.regions[r].floor;
        (s.player.x, s.player.y, s.player.z, s.player.region) = (x, y, z, r);
        s.previous = s.player;
    });
    if start("LIPS01").is_none() {
        return;
    }
    assert!(shots.iter().filter(|s| s.2 < 64.0).count() >= 3, "shots reaching the player: {:?}", shots.iter().map(|s| s.2 as i32).collect::<Vec<_>>());
}

/// Plain OWALKs (no script) random-walk (object-effects.md OWALK tick 4): LIPS00's vampire bats
/// fly about near the player instead of hanging still where they were placed.
#[test]
fn vampire_bats_fly_about() {
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    game.tick(&vfs, &godd_game::Input::default());
    let (bat, from) = {
        let s = game.session_mut().unwrap();
        s.god = true;
        let b = &s.objects.list[13];
        let from = (b.x, b.y, b.z);
        // Stand 300 units off, inside the activation distance.
        let (x, y) = (b.x - 300.0, b.y);
        if let Some(r) = s.level.world.region_at(x, y) {
            let z = s.level.world.regions[r].floor;
            (s.player.x, s.player.y, s.player.z, s.player.region) = (x, y, z, r);
            s.previous = s.player;
        }
        (13, from)
    };
    let mut moved: f32 = 0.0;
    let mut heights = (f32::MAX, f32::MIN);
    for _ in 0..(5.0 * godd_game::TICK_RATE) as usize {
        game.tick(&vfs, &godd_game::Input::default());
        let b = &game.session().unwrap().objects.list[bat];
        moved = moved.max((b.x - from.0).hypot(b.y - from.1));
        heights = (heights.0.min(b.z), heights.1.max(b.z));
    }
    assert!(moved > 64.0, "the bat flies about: moved {moved}");
}

/// Stand `d` units in front of LIPS01's death bot #331 (in the open, facing it) with every tool.
fn face_bot_331(game: &mut godd_game::Game, d: f32) {
    let s = game.session_mut().unwrap();
    s.god = true;
    s.objects.tools.available = u32::MAX;
    let b = &s.objects.list[331];
    let (x, y) = (b.x + d * 0.78, b.y + d * 0.62);
    let r = s.level.world.region_at(x, y).unwrap();
    (s.player.x, s.player.y, s.player.z, s.player.region) = (x, y, s.level.world.regions[r].floor, r);
    s.previous = s.player;
}

/// Fire tool 2 at bot #331 every `period` ticks for `seconds`, aiming at it; where each shot ended
/// (distance from the bot) and the enemies-killed count after it.
fn shoot_bot_331(fixes: bool, seconds: f64) -> Vec<(f32, i32)> {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS01") else { return Vec::new() };
    assert!(game.exec_config(&format!("set g_fixes {}", fixes as u8)).is_empty());
    game.tick(&vfs, &godd_game::Input::default());
    face_bot_331(&mut game, 190.0);
    let mut pick = godd_game::Input::default();
    pick.set(Action::Tool2, true);
    game.tick(&vfs, &pick);
    let mut live = std::collections::HashSet::new();
    let mut ends = Vec::new();
    for t in 0..(seconds * godd_game::TICK_RATE) as usize {
        let (bx, by) = {
            let b = &game.session().unwrap().objects.list[331];
            (b.x, b.y)
        };
        let s = game.session_mut().unwrap();
        s.player.angle = (by - s.player.y).atan2(bx - s.player.x);
        let mut input = godd_game::Input::default();
        input.set(Action::Use, t % 120 < 2);
        game.tick(&vfs, &input);
        let s = game.session().unwrap();
        for (i, o) in s.objects.list.iter().enumerate() {
            if o.name == "_PPROJ_" && !o.destroyed {
                live.insert(i);
            } else if o.name == "_PPROJ_" && live.remove(&i) {
                let b = &s.objects.list[331];
                ends.push(((o.x - b.x).hypot(o.y - b.y), s.script.states[12]));
            }
        }
    }
    ends
}

/// A one-hit death bot (DETHBOT1, SH(1)) dies to the second shot (movers.md 3.5: HK runs at
/// the event after the counter runs out, n + 1), and turns into its bone-pile pickup. With
/// g_fixes later shots fly on past the pile; the original bursts them on it, right in front of
/// the player.
#[test]
fn a_killed_bot_dies_on_the_second_shot_and_its_bones_do_not_stop_shots() {
    for fixes in [true, false] {
        let ends = shoot_bot_331(fixes, 7.0);
        if ends.is_empty() && start("LIPS01").is_none() {
            return;
        }
        assert!(ends.len() >= 4, "fixes {fixes}: shots {ends:?}");
        assert!(ends[0].0 < 30.0 && ends[1].0 < 30.0, "fixes {fixes}: the first two hit the bot: {ends:?}");
        // (Its HK block runs a few ticks after the second shot lands.)
        assert!(ends[1].1 == 0 && ends[2].1 == 1, "fixes {fixes}: killed by the second shot: {ends:?}");
        let later = &ends[2..];
        if fixes {
            assert!(later.iter().all(|e| e.0 > 60.0), "shots pass the bones: {ends:?}");
        } else {
            assert!(later.iter().all(|e| e.0 < 30.0), "classic: shots burst on the bones: {ends:?}");
        }
    }
}

/// LIPS07's tag switch #31 hangs 384 units up (r78): an aimed shot (tool 2, looking up) hits
/// it, which opens the gate (wall 589 copies GAT01OPN); the lobbed starting tools cannot reach.
#[test]
fn lips07_overhead_switch_is_shot_open() {
    use godd_game::Action;
    for (tool, opens) in [(Action::Tool2, true), (Action::Tool0, false)] {
        let Some((vfs, mut game)) = start("LIPS07") else { return };
        game.tick(&vfs, &godd_game::Input::default());
        {
            let s = game.session_mut().unwrap();
            s.god = true;
            s.objects.tools.available = u32::MAX;
            let r = s.level.world.region_at(14272.0, 19704.0).unwrap();
            (s.player.x, s.player.y, s.player.z, s.player.region) = (14272.0, 19704.0, s.level.world.regions[r].floor, r);
            s.player.angle = std::f32::consts::FRAC_PI_2;
            s.previous = s.player;
        }
        let mut pick = godd_game::Input::default();
        pick.set(tool, true);
        game.tick(&vfs, &pick);
        let mut fire = godd_game::Input::default();
        fire.set(Action::Use, true);
        for _ in 0..240 {
            game.session_mut().unwrap().player.look = 512.0;
            game.tick(&vfs, &fire);
        }
        let open = !game.session().unwrap().level.world.walls[589].blocking;
        assert_eq!(open, opens, "{tool:?}: gate open {open}");
    }
}

/// Bots keep their distance from walls like the player (radius + 12, physics.md 8.1): the
/// death bots creeping at the player through a wall at the starts of LIPS01/02 used to push
/// half their body into it.
#[test]
fn bots_stay_out_of_walls() {
    for level in ["LIPS00", "LIPS01", "LIPS02", "LIPS06", "LIPS09"] {
        let Some((vfs, mut game)) = start(level) else { return };
        game.session_mut().unwrap().god = true;
        for _ in 0..(15.0 * godd_game::TICK_RATE) as usize {
            game.tick(&vfs, &godd_game::Input::default());
            let s = game.session().unwrap();
            let w = &s.level.world;
            for (i, o) in s.objects.list.iter().enumerate().filter(|(_, o)| o.mover.is_some() && !o.hidden && !o.destroyed && o.region.is_some()) {
                let r = o.region.unwrap();
                let inside_wall = w.regions[r].walls.iter().any(|&wi| {
                    let wall = &w.walls[wi];
                    if wall.back.is_some() && !wall.blocking {
                        return false;
                    }
                    let (a, b) = (w.vertices[wall.a], w.vertices[wall.b]);
                    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
                    let t = (((o.x - a.0) * ex + (o.y - a.1) * ey) / (ex * ex + ey * ey)).clamp(0.0, 1.0);
                    (o.x - a.0 - t * ex).hypot(o.y - a.1 - t * ey) < o.radius * 0.5
                });
                assert!(!inside_wall, "{level}: bot #{i} {} half inside a wall at ({}, {})", o.name, o.x, o.y);
            }
        }
    }
}


/// OSNDGRP (object-effects.md): LIPS10's AMBICITY street ambience. Nothing in the shipped data
/// triggers it (dead data), so the test gives it a tag. Mode 0 plays all five slots together,
/// from the object; a trigger while they play is ignored; mode 1 plays one slot per trigger,
/// in turn.
#[test]
fn sound_groups_play_together_or_in_turn() {
    let Some((vfs, mut game)) = start("LIPS10") else { return };
    game.tick(&vfs, &godd_game::Input::default());
    let sounds = |game: &mut godd_game::Game| -> Vec<String> {
        game.drain_audio()
            .into_iter()
            .filter_map(|e| match e {
                godd_game::AudioEvent::Sound { name, pos: Some(_), .. } if name.starts_with("AMB") => Some(name),
                _ => None,
            })
            .collect()
    };
    let _ = game.drain_audio();
    let s = game.session_mut().unwrap();
    assert_eq!(s.objects.list[110].name, "AMBICITY");
    s.objects.list[110].tag = 990;
    s.fire_tag(990);
    game.tick(&vfs, &godd_game::Input::default());
    let mut all = sounds(&mut game);
    all.sort();
    assert_eq!(all, ["AMBSUB01", "AMBWLK01", "AMBWLK03", "AMBWLK04", "AMBWLK05"], "mode 0: all five");
    game.session_mut().unwrap().fire_tag(990);
    game.tick(&vfs, &godd_game::Input::default());
    assert!(sounds(&mut game).is_empty(), "ignored while playing");

    // Mode 1: wait for the group to end, then one slot per trigger.
    for _ in 0..(30.0 * godd_game::TICK_RATE) as usize {
        game.tick(&vfs, &godd_game::Input::default());
    }
    let _ = game.drain_audio();
    game.session_mut().unwrap().objects.list[110].counter = 1;
    let mut order = Vec::new();
    for _ in 0..2 {
        game.session_mut().unwrap().fire_tag(990);
        game.tick(&vfs, &godd_game::Input::default());
        order.extend(sounds(&mut game));
        for _ in 0..(10.0 * godd_game::TICK_RATE) as usize {
            game.tick(&vfs, &godd_game::Input::default());
        }
        let _ = game.drain_audio();
    }
    assert_eq!(order, ["AMBWLK01", "AMBWLK03"], "mode 1: slots in turn");
}

/// LIPS05's blue gem (it opens door r481, behind which is the rest of the level) lies on an
/// island in a field of death pits, reached by jump pads (RFBOUNCE, world-effects.md 3.3): from
/// a low pad (r524, -960) hop to r520 (-640), then on to the gem's pedestal (r368, -368). Pads
/// relaunch a body on their floor at every contact (not only on landing), and a jump started on
/// one adds the pad's speed; without those the island cannot be reached.
#[test]
fn lips05_jump_pads_carry_the_player_to_the_blue_gem() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS05") else { return };
    game.tick(&vfs, &godd_game::Input::default());
    {
        let s = game.session_mut().unwrap();
        s.god = true;
        let z = s.level.world.regions[524].floor;
        (s.player.x, s.player.y, s.player.z, s.player.region) = (12480.0, 9600.0, z, 524);
        s.previous = s.player;
    }
    let mut hop = godd_game::Input::default();
    hop.set(Action::Forward, true);
    hop.set(Action::Jump, true);
    let mut walk = godd_game::Input::default();
    walk.set(Action::Forward, true);
    // 0: hop to r520; 1: cross it (bouncing) to its north edge; 2: jump to the gem.
    let mut stage = 0;
    for _ in 0..900 {
        let p = game.session().unwrap().player;
        if p.region == 368 {
            break;
        }
        if stage == 0 && p.region == 520 {
            stage = 1;
        }
        let edge = (12864.0, 9140.0);
        // (Jump as the pad is touched, near its north edge.)
        if stage == 1 && p.region == 520 && (p.x - edge.0).hypot(p.y - edge.1) < 40.0 && p.z < -630.0 && p.vz <= 0.0 {
            stage = 2;
        }
        let (target, input) = match stage {
            0 => ((12864.0, 9280.0), &hop),
            1 => (edge, &walk),
            _ => ((12864.0, 8800.0), &hop),
        };
        game.session_mut().unwrap().player.angle = (target.1 - p.y).atan2(target.0 - p.x);
        game.tick(&vfs, input);
    }
    let p = game.session().unwrap().player;
    assert_eq!(p.region, 368, "on the gem's pedestal: ({}, {}, {}) r{}", p.x, p.y, p.z, p.region);
}


/// LIPS00's water room: landing in r468 fires tag 77, and WATER1 (OTELE) sends the player onto
/// the template SWIMMER1 in r303, taking on its body (MoveObjToObject, object-effects.md
/// OTELE): it flies (flag 0x2), so Jump swims up (the original rises to z = 454 in 3 s, measured
/// under Wine). WATER2 sends the player onto SWIMMER2, which walks again.
#[test]
fn lips00_water_teleport_makes_the_player_swim() {
    use godd_game::Action;
    let Some((vfs, mut game)) = start("LIPS00") else { return };
    game.tick(&vfs, &godd_game::Input::default());
    {
        let s = game.session_mut().unwrap();
        s.god = true;
        (s.player.x, s.player.y, s.player.z, s.player.region) = (17760.0, 14400.0, -100.0, 468);
        s.player.on_ground = false;
        s.previous = s.player;
    }
    for _ in 0..60 {
        game.tick(&vfs, &godd_game::Input::default());
    }
    let p = game.session().unwrap().player;
    assert_eq!(p.region, 303, "in the water room: ({}, {}, {}) r{}", p.x, p.y, p.z, p.region);
    assert!(p.flying, "swimming");
    let start_z = p.z;
    let mut up = godd_game::Input::default();
    up.set(Action::Jump, true);
    for _ in 0..(3.0 * godd_game::TICK_RATE) as usize {
        game.tick(&vfs, &up);
    }
    let z = game.session().unwrap().player.z;
    assert!((380.0..520.0).contains(&(z - start_z)), "swam up from {start_z} to {z} (original: about 450 in 3 s)");
    // Letting go of Jump, the swimmer drifts down slowly (idle sink 50 u per 1.2 s).
    for _ in 0..(1.2 * godd_game::TICK_RATE) as usize {
        game.tick(&vfs, &godd_game::Input::default());
    }
    let sunk = z - game.session().unwrap().player.z;
    assert!((35.0..65.0).contains(&sunk), "sank {sunk} in 1.2 s (original 50)");
    game.session_mut().unwrap().fire_tag(78);
    game.tick(&vfs, &godd_game::Input::default());
    let s = game.session().unwrap();
    assert!(!s.player.flying, "SWIMMER2 walks again");
}
