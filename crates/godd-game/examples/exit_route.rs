//! Is the exit walkable from the start, doors open? (debug aid)
//! `cargo run --example exit_route LIPS05`: a region route from the player's start to the region
//! of the wall switch firing the exit (tag 500), through no solid line and no death pit, stepping
//! up at most 48 (jump pads: their bounce height; lifts: anything). Without one, it prints where
//! the reachable area ends.
use godd_game::{Game, Options, Vfs};
use std::collections::VecDeque;

fn main() {
    let level = std::env::args().nth(1).unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let mut game = Game::new(&vfs, &Options { start_level: Some(level.clone()), save: None }).unwrap();
    game.tick(&vfs, &godd_game::Input::default());
    let s = game.session().unwrap();
    let w = &s.level.world;
    let from = s.player.region;
    let exits: Vec<usize> = s.effects.switches().iter().filter(|sw| sw.tags.contains(&500)).map(|sw| w.walls[sw.wall].front).collect();
    let death: Vec<i32> = s.objects.list.iter().filter(|o| o.effect == 15 && o.tag > 0 && o.aux.as_ref().and_then(|a| a.slot(9)).is_some_and(|t| t.to_ascii_uppercase().starts_with("RBIRTH"))).map(|o| o.tag).collect();
    let deadly: Vec<usize> = s.effects.triggers().iter().filter(|(_, t)| death.contains(&t[0]) || death.contains(&t[2])).map(|(r, _)| *r).collect();
    let lifts: Vec<usize> = s.effects.lifts().iter().map(|l| l.0).collect();
    let doors: Vec<usize> = s.effects.doors().iter().map(|d| d.0).collect();
    // Floors a region can have: its own, and every height an OSETF moving it sets (lifts,
    // rising bridges).
    let names = s.level.region_names();
    let mut floors: Vec<Vec<f32>> = w.regions.iter().map(|r| vec![r.floor]).collect();
    for o in s.objects.list.iter().filter(|o| o.effect == 41) {
        let Some(a) = o.aux.as_ref() else { continue };
        if let Some(r) = a.slot(9).and_then(|n| names.iter().position(|rn| rn.eq_ignore_ascii_case(n))) {
            floors[r].push(a.ints[0] as f32);
        }
    }
    let rise = |r: usize, o: usize| -> f32 { floors[o].iter().flat_map(|fo| floors[r].iter().map(move |fr| fo - fr)).fold(f32::MAX, f32::min) };
    let climb = |r: usize| -> f32 {
        if lifts.contains(&r) {
            return f32::MAX;
        }
        match s.effects.bounce_speed(r) {
            // A jump on a pad launches at speed + 200; a jump at 200 peaks at ~48 (h ~ v^2).
            Some(v) => 48.0 * ((v + 200.0) / 200.0).powi(2),
            None => 48.0,
        }
    };
    // Why the step from region r into o is not possible (None: it is).
    let blocked = |r: usize, o: usize, x: &godd_game::world::Wall| -> Option<&'static str> {
        let door = doors.contains(&o) || doors.contains(&r);
        if x.blocking && !door {
            Some("solid")
        } else if deadly.contains(&o) {
            Some("death pit")
        } else if !door && !lifts.contains(&o) && rise(r, o) > climb(r) {
            Some("too high")
        } else if !door && w.regions[o].ceiling - w.regions[o].floor < 12.0 {
            Some("no headroom")
        } else {
            None
        }
    };
    let links = |r: usize| -> Vec<(usize, &godd_game::world::Wall)> {
        w.walls
            .iter()
            .filter_map(|x| {
                let b = x.back.filter(|&b| b != x.front)?;
                if x.front == r { Some((b, x)) } else if b == r { Some((x.front, x)) } else { None }
            })
            .collect()
    };
    let search = |starts: &[usize], forward: bool| -> Vec<bool> {
        let mut seen = vec![false; w.regions.len()];
        let mut queue: VecDeque<usize> = starts.iter().copied().collect();
        for &st in starts {
            seen[st] = true;
        }
        while let Some(r) = queue.pop_front() {
            for (o, x) in links(r) {
                let ok = if forward { blocked(r, o, x).is_none() } else { blocked(o, r, x).is_none() };
                if ok && !seen[o] {
                    seen[o] = true;
                    queue.push_back(o);
                }
            }
        }
        seen
    };
    let ahead = search(&[from], true);
    let behind = search(&exits, false);
    let reached = ahead.iter().filter(|&&a| a).count();
    match exits.iter().find(|&&e| ahead[e]) {
        Some(e) => println!("{level}: exit r{e} reachable from r{from} ({reached} regions)"),
        None => {
            println!("{level}: exit {exits:?} NOT reachable from r{from} ({reached} regions reached, {} lead to the exit). Chokepoints:", behind.iter().filter(|&&b| b).count());
            // The shortest way across, rules ignored, from the reachable area to the exit's.
            let mut prev = vec![usize::MAX; w.regions.len()];
            let mut queue: VecDeque<usize> = (0..w.regions.len()).filter(|&r| ahead[r]).collect();
            for &r in &queue {
                prev[r] = r;
            }
            let mut end = None;
            while let Some(r) = queue.pop_front() {
                if behind[r] {
                    end = Some(r);
                    break;
                }
                for (o, x) in links(r) {
                    if prev[o] == usize::MAX && !(x.blocking && !doors.contains(&o) && !doors.contains(&r)) {
                        prev[o] = r;
                        queue.push_back(o);
                    }
                }
            }
            let mut path = Vec::new();
            let mut r = end.unwrap_or(from);
            while prev[r] != r && prev[r] != usize::MAX {
                path.push((prev[r], r));
                r = prev[r];
            }
            path.reverse();
            println!("  shortest way across:");
            for (a, b) in path {
                let x = links(a).into_iter().find(|(o, _)| *o == b).map(|(_, x)| x).unwrap();
                println!("    r{a} (floor {}) -> r{b} (floor {} ceil {}){}", w.regions[a].floor, w.regions[b].floor, w.regions[b].ceiling, blocked(a, b, x).map_or(String::new(), |y| format!(": {y}")));
            }
            let mut seen = std::collections::HashSet::new();
            for r in (0..w.regions.len()).filter(|&r| ahead[r]) {
                for (o, x) in links(r) {
                    if behind[o] && seen.insert((r, o)) {
                        let why = blocked(r, o, x).unwrap_or("?");
                        println!("  r{r} (floor {}) -> r{o} (floor {} ceil {}): {why}", w.regions[r].floor, w.regions[o].floor, w.regions[o].ceiling);
                    }
                }
            }
        }
    }
}
