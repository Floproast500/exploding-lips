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
    let exits: Vec<usize> = match std::env::var("TARGET").ok().and_then(|t| t.parse().ok()) {
        Some(t) => vec![t],
        None => s.effects.switches().iter().filter(|sw| sw.tags.contains(&500)).map(|sw| w.walls[sw.wall].front).collect(),
    };
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
    // Walls an OWTOW can overwrite with a passable one (wall doors) count as open.
    let opened: Vec<usize> = s
        .objects
        .list
        .iter()
        .filter(|o| o.effect == 67)
        .filter_map(|o| {
            let a = o.aux.as_ref()?;
            let (t, src) = (s.wall_named(a.slot(9)?)?, s.wall_named(a.slot(8)?)?);
            (!w.walls[src].blocking).then_some(t)
        })
        .collect();
    // Why the step from region r into o is not possible (None: it is).
    // CLOSED=4,7: doors that stay shut; TARGET=2: check that region instead of the exit.
    let closed: Vec<usize> = std::env::var("CLOSED").map(|v| v.split(',').filter_map(|c| c.parse().ok()).collect()).unwrap_or_default();
    let blocked = |r: usize, o: usize, x: &godd_game::world::Wall| -> Option<&'static str> {
        if closed.contains(&o) {
            return Some("closed door");
        }
        let door = doors.contains(&o) || doors.contains(&r);
        let wall_door = w.walls.iter().position(|y| std::ptr::eq(y, x)).is_some_and(|i| opened.contains(&i));
        if x.blocking && !door && !wall_door {
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
    // Teleports (OTELE onto an object, not back to a start point): from the region of a walk-in
    // teleporter, or of the trigger region / wall switch firing a tag-driven one, to the
    // destination's region; a flying destination template (LIPS00's swimmer) lets the player
    // climb anywhere until a teleport onto a walking one.
    let triggers = s.effects.triggers();
    let switches = s.effects.switches();
    let mut teleports: Vec<(usize, usize, bool)> = Vec::new();
    for o in s.objects.list.iter().filter(|o| o.effect == 15 && !death.contains(&o.tag)) {
        let Some(dest) = o.aux.as_ref().and_then(|a| a.slot(9)).and_then(|n| s.objects.list.iter().find(|d| d.name.eq_ignore_ascii_case(n))) else { continue };
        let Some(to) = dest.region else { continue };
        let flying = dest.flags & 2 != 0;
        let mut from_regions: Vec<usize> = o.region.into_iter().collect();
        if o.tag > 0 {
            from_regions.extend(triggers.iter().filter(|(_, t)| t[0] == o.tag || t[2] == o.tag).map(|(r, _)| *r));
            from_regions.extend(switches.iter().filter(|sw| sw.tags.contains(&o.tag)).map(|sw| w.walls[sw.wall].front));
        }
        for f in from_regions {
            teleports.push((f, to, flying));
        }
    }
    // Forward, with the flying state.
    let ahead = {
        let mut seen = vec![[false; 2]; w.regions.len()];
        let mut parent: Vec<Option<(usize, &'static str)>> = vec![None; w.regions.len()];
        seen[from][0] = true;
        let mut queue = VecDeque::from([(from, false)]);
        while let Some((r, fly)) = queue.pop_front() {
            let mut next: Vec<(usize, bool, &'static str)> = links(r)
                .into_iter()
                .filter(|(o, x)| match blocked(r, *o, x) {
                    None => true,
                    Some("too high") | Some("no headroom") => fly,
                    _ => false,
                })
                .map(|(o, x)| (o, fly, if doors.contains(&o) { "door" } else if x.blocking { "wall door" } else if lifts.contains(&o) || floors[o].len() > 1 { "lift" } else { "walk" }))
                .collect();
            next.extend(teleports.iter().filter(|t| t.0 == r).map(|t| (t.1, t.2, "teleport")));
            // Stacked levels: drop through a see-through floor; climb into the one above when
            // it is in reach.
            let reg = &w.regions[r];
            next.extend(reg.below.filter(|b| !deadly.contains(b)).map(|b| (b, fly, "drop")));
            next.extend(reg.above.filter(|&a| fly || rise(r, a) <= climb(r)).map(|a| (a, fly, "climb")));
            for (o, f, how) in next {
                if !seen[o][f as usize] {
                    seen[o][f as usize] = true;
                    if parent[o].is_none() && o != from {
                        parent[o] = Some((r, how));
                    }
                    queue.push_back((o, f));
                }
            }
        }
        // PATH_TO=1: the way found to the first target, with how each step is made (not walking).
        if std::env::var("PATH_TO").is_ok()
            && let Some(&t) = exits.first()
        {
            let mut steps = Vec::new();
            let mut r = t;
            while let Some((p, how)) = parent[r] {
                steps.push((p, r, how));
                r = p;
            }
            steps.reverse();
            for (p, r, how) in steps.iter().filter(|s| s.2 != "walk") {
                println!("  r{p} -> r{r}: {how}");
            }
        }
        seen.iter().map(|s| s[0] || s[1]).collect::<Vec<bool>>()
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
            if std::env::var("FRONTIER").is_ok() {
                for r in (0..w.regions.len()).filter(|&r| ahead[r]) {
                    for (o, x) in links(r) {
                        if !ahead[o] {
                            println!("  frontier r{r} (floor {}) -> r{o} (floor {}): {}", w.regions[r].floor, w.regions[o].floor, blocked(r, o, x).unwrap_or("?"));
                        }
                    }
                }
            }
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
