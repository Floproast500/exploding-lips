//! `playbot <GAME_DIR> --optimise LEVEL [ROUTES_DIR]`: make a recorded route a clear-cut way
//! through. A route recorded from a run keeps everything that run did: things used that the
//! level does not need yet (or at all), detours, ways walked twice.
//!
//! The steps here are the level's own sequence, worked out from its data (`chains`): only what
//! finishing it needs, each after whatever it needs first (a door's switch before the door, a
//! key before its lock), so nothing leads where it cannot be opened yet. Only steps the
//! recorded run actually used are allowed (where the data offers several ways, the one shown
//! to work). The trail to each step is the recorded walk between them, its loops cut.

use crate::{Target, chains, fresh_bot};
use godd_game::route::{Route, RouteStep, RouteTarget};
use godd_game::{Game, Options, Vfs};
use std::collections::HashSet;

fn target_of(t: RouteTarget) -> Target {
    match t {
        RouteTarget::Wall(w) => Target::Wall(w),
        RouteTarget::Object(o) => Target::Object(o),
        RouteTarget::Region(r) => Target::Region(r),
    }
}

/// Whether the straight line from `a` to `b` crosses a solid line of the map (one-sided, or a
/// blocking two-sided one).
fn solid_between(world: &godd_game::world::World, a: [f32; 3], b: [f32; 3]) -> bool {
    world.walls.iter().any(|w| {
        if w.back.is_some_and(|k| k != w.front) && !w.blocking {
            return false;
        }
        let (p, q) = (world.vertices[w.a], world.vertices[w.b]);
        let (dx, dy, ex, ey) = (b[0] - a[0], b[1] - a[1], q.0 - p.0, q.1 - p.1);
        let den = dx * ey - dy * ex;
        if den.abs() < 1e-9 {
            return false;
        }
        let t = ((p.0 - a[0]) * ey - (p.1 - a[1]) * ex) / den;
        let u = ((p.0 - a[0]) * dy - (p.1 - a[1]) * dx) / den;
        (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
    })
}

/// `untangle` that cuts a loop only where the shortcut crosses no solid line of the map (the
/// way back past a pillar is not a way through it).
fn untangle_in(world: &godd_game::world::World, trail: &[[f32; 3]], near: f32) -> Vec<[f32; 3]> {
    let mut out: Vec<[f32; 3]> = Vec::new();
    for &p in trail {
        let close = |q: &[f32; 3]| (q[0] - p[0]).hypot(q[1] - p[1]) < near && (q[2] - p[2]).abs() < 64.0 && !solid_between(world, *q, p);
        match out.iter().position(close) {
            Some(k) => out.truncate(k + 1),
            None => out.push(p),
        }
    }
    out
}

/// A walk through `path` (regions, in order) from `from` to `to`: through the middle of the
/// widest opening between each two, pulled straight wherever no solid line is in the way,
/// a point every 48 units.
fn walk_through(world: &godd_game::world::World, path: &[usize], from: [f32; 3], to: [f32; 3]) -> Vec<[f32; 3]> {
    let mut pts = vec![from];
    for pair in path.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let opening = world.regions[a]
            .walls
            .iter()
            .map(|&wi| &world.walls[wi])
            .filter(|w| (w.front == a && w.back == Some(b)) || (w.front == b && w.back == Some(a)))
            .map(|w| (world.vertices[w.a], world.vertices[w.b]))
            .max_by(|x, y| ((x.1 .0 - x.0 .0).hypot(x.1 .1 - x.0 .1)).total_cmp(&(y.1 .0 - y.0 .0).hypot(y.1 .1 - y.0 .1)));
        if let Some((p, q)) = opening {
            let z = world.regions[a].floor.max(world.regions[b].floor);
            pts.push([(p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0, z]);
        }
    }
    pts.push(to);
    // Pulled straight: from each point, on to the furthest one in plain line through the path's
    // own regions (not over a drop or past a stacked room into others).
    let on_path: HashSet<usize> = path.iter().copied().collect();
    let within = |a: [f32; 3], b: [f32; 3]| {
        world.walls.iter().all(|w| {
            let (p, q) = (world.vertices[w.a], world.vertices[w.b]);
            let (dx, dy, ex, ey) = (b[0] - a[0], b[1] - a[1], q.0 - p.0, q.1 - p.1);
            let den = dx * ey - dy * ex;
            if den.abs() < 1e-9 {
                return true;
            }
            let t = ((p.0 - a[0]) * ey - (p.1 - a[1]) * ex) / den;
            let u = ((p.0 - a[0]) * dy - (p.1 - a[1]) * dx) / den;
            if !((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)) {
                return true;
            }
            !w.blocking && w.back.is_some_and(|k| on_path.contains(&k)) && on_path.contains(&w.front)
        })
    };
    let mut pulled = vec![pts[0]];
    let mut k = 0;
    while k + 1 < pts.len() {
        let far = (k + 1..pts.len()).rev().find(|&m| m == k + 1 || within(pts[k], pts[m])).unwrap_or(k + 1);
        pulled.push(pts[far]);
        k = far;
    }
    // A point every 48 units (the guide shows trail points).
    let mut out = Vec::new();
    for seg in pulled.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / 48.0).ceil().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]);
        }
    }
    out.pop(); // (the step itself is not a trail point)
    out
}

/// The optimised route for `level` from its recorded one.
pub fn optimise(vfs: &Vfs, level: &str, route: &Route) -> Result<Route, String> {
    let all = &route.steps;
    if all.is_empty() {
        return Err("an empty route".into());
    }
    let game = Game::new(vfs, &Options { start_level: Some(level.to_string()), save: None }).map_err(|e| e.to_string())?;
    let bot = fresh_bot(vfs, game, level, false);
    let world = bot.session().level.world.clone();
    let start = {
        let p = bot.session().player;
        [p.x, p.y, p.z]
    };
    // Doors shut at the start, by region: the tag that opens each.
    let shut_doors: std::collections::HashMap<usize, i32> = bot.session().effects.doors().into_iter().filter(|(r, _)| world.regions[*r].ceiling - world.regions[*r].floor < 56.0).collect();
    let used: HashSet<Target> = all.iter().map(|st| target_of(st.target)).collect();
    // (A lift called is a region stepped into, in the recording.)
    let as_used = |t: Target| match t {
        Target::Lift(r) => Target::Region(r),
        t => t,
    };
    // The exit is the recorded route's last step: a sequence counts only if it ends there.
    let exit = target_of(all.last().expect("steps").target);
    let plan = |avoid: &HashSet<Target>| {
        let s = chains::sequence(bot.session(), bot.death_tags(), None, true, avoid);
        s.last().is_some_and(|st| as_used(st.target) == exit).then_some(s)
    };
    let mut avoid: HashSet<Target> = HashSet::new();
    let Some(mut seq) = plan(&avoid) else {
        let s = chains::sequence(bot.session(), bot.death_tags(), None, true, &HashSet::new());
        println!("{level}: the level's sequence ends at {:?}, the run at {exit:?}: only what changes nothing is left out", s.last().map(|st| (st.target, st.why.clone())));
        return Ok(safe_prune(&world, bot.session(), route));
    };
    // Steered round what the run never used, one at a time, while it still reaches the exit.
    let mut tried: HashSet<Target> = HashSet::new();
    while let Some(m) = seq.iter().map(|st| st.target).find(|&t| !used.contains(&as_used(t)) && !tried.contains(&t)) {
        tried.insert(m);
        let mut more = avoid.clone();
        more.insert(m);
        if let Some(s) = plan(&more) {
            (avoid, seq) = (more, s);
        }
    }
    // Each step where the run used it next (after the step before); its trail, the walk since.
    let mut out = Route::default();
    let mut at = 0usize; // first recorded step not yet passed
    for st in &seq {
        let t = as_used(st.target);
        if !used.contains(&t) {
            // (The run did this another way: the first thing it used after the step before
            // that fires the same tags, LIPS01's wall 112 for 302, both opening door r8.)
            let want = tags_of(bot.session(), t);
            if let Some(k) = (at..all.len()).find(|&k| !want.is_empty() && !tags_of(bot.session(), target_of(all[k].target)).is_disjoint(&want)) {
                let walked: Vec<[f32; 3]> = (at..=k).flat_map(|q| all[q].trail.iter().copied()).collect();
                out.steps.push(RouteStep { target: all[k].target, pos: all[k].pos, trail: untangle_in(&world, &walked, 96.0) });
                at = k + 1;
            }
            continue;
        }
        let next = (at..all.len()).find(|&j| target_of(all[j].target) == t);
        let j = match next {
            Some(j) => j,
            // (Used only earlier: from its own approach.)
            None => match (0..at).rev().find(|&j| target_of(all[j].target) == t) {
                Some(j) => {
                    out.steps.push(RouteStep { target: all[j].target, pos: all[j].pos, trail: untangle_in(&world, &all[j].trail, 96.0) });
                    continue;
                }
                None => continue,
            },
        };
        // The run's walk there, if it kept to the ways open then in the sequence (it may have
        // gone through a door a step left out had opened: then through the planned regions).
        let walked: Vec<[f32; 3]> = (at..=j).flat_map(|k| all[k].trail.iter().copied()).collect();
        // The run's walk is the way, unless it goes through a door that started shut and that
        // nothing kept so far (or a trigger crossed on the way) opens: then the planned way.
        let mut fired: HashSet<i32> = out.steps.iter().flat_map(|o| tags_of(bot.session(), target_of(o.target))).collect();
        let mut walk_ok = true;
        for p in &walked {
            let Some(r) = world.region_at_z(p[0], p[1], Some(p[2] + 1.0)).or_else(|| world.region_at(p[0], p[1])) else { continue };
            fired.extend(tags_of(bot.session(), Target::Region(r)));
            if let Some(t) = shut_doors.get(&r)
                && !fired.contains(t)
            {
                walk_ok = false;
                break;
            }
        }
        let trail = if walk_ok || st.path.len() < 2 {
            untangle_in(&world, &walked, 96.0)
        } else {
            let from = out.steps.last().map_or(start, |p| p.pos);
            walk_through(&world, &st.path, from, all[j].pos)
        };
        out.steps.push(RouteStep { target: all[j].target, pos: all[j].pos, trail });
        at = j + 1;
    }
    let length = |r: &Route| -> f32 { r.steps.iter().flat_map(|st| st.trail.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))).sum() };
    println!("{level}: {} steps → {} ({:.0} → {:.0} units of trail); the sequence: {:?}", all.len(), out.steps.len(), length(route), length(&out), seq.iter().map(|s| s.target).collect::<Vec<_>>());
    Ok(out)
}

/// Without the level's sequence (LIPS17's boss, a plan the data does not finish): the run's own
/// steps, less those that change nothing (a pickup that fires no tag and that no lock asks
/// for: treasure, health) and a step used again right after itself, the trail across them
/// with its loops cut.
fn safe_prune(world: &godd_game::world::World, s: &godd_game::session::Session, route: &Route) -> Route {
    let inert = inert_steps(s);
    let all = &route.steps;
    let mut out = Route::default();
    let mut walked: Vec<[f32; 3]> = Vec::new();
    for (k, st) in all.iter().enumerate() {
        walked.extend(st.trail.iter().copied());
        let last = k + 1 == all.len();
        let repeat = all.get(k + 1).is_some_and(|n| n.target == st.target);
        if !last && (inert(st) || repeat) {
            continue;
        }
        out.steps.push(RouteStep { target: st.target, pos: st.pos, trail: untangle_in(world, &std::mem::take(&mut walked), 96.0) });
    }
    let length = |r: &Route| -> f32 { r.steps.iter().flat_map(|st| st.trail.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))).sum() };
    println!("  {} steps → {} ({:.0} → {:.0} units of trail)", all.len(), out.steps.len(), length(route), length(&out));
    out
}

/// Steps that change nothing in the world: a pickup that fires no tag and that no lock asks for
/// (treasure, health).
fn inert_steps(s: &godd_game::session::Session) -> impl Fn(&RouteStep) -> bool + '_ {
    let list = &s.objects.list;
    let wanted: HashSet<String> = list.iter().filter(|o| o.effect == 131).filter_map(|o| o.aux.as_ref().and_then(|a| a.slot(9)).map(|n| n.to_ascii_uppercase())).collect();
    move |st: &RouteStep| match st.target {
        RouteTarget::Object(i) => list.get(i).is_some_and(|o| o.effect == 130 && o.aux.as_ref().is_none_or(|a| a.ints[0] <= 0) && !wanted.contains(&o.name.to_ascii_uppercase())),
        _ => false,
    }
}

/// The tags a step fires: a switch wall's, a pickup's or switch object's, a trigger region's.
fn tags_of(s: &godd_game::session::Session, t: Target) -> HashSet<i32> {
    match t {
        Target::Wall(w) => s.effects.switches().into_iter().filter(|sw| sw.wall == w).flat_map(|sw| sw.tags).filter(|&x| x > 0).collect(),
        Target::Object(i) => s.objects.list.get(i).map_or_else(HashSet::new, |o| {
            let mut set: HashSet<i32> = o.aux.as_ref().map(|a| a.ints[0]).into_iter().filter(|&x| x > 0).collect();
            if matches!(o.effect, 20 | 22) && o.tag > 0 {
                set.insert(o.tag);
            }
            set
        }),
        Target::Region(r) | Target::Lift(r) => s.effects.triggers().into_iter().filter(|(q, _)| *q == r).flat_map(|(_, tags)| tags).filter(|&x| x > 0).collect(),
    }
}
