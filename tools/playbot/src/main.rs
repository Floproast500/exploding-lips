//! `playbot`: plays Exploding Lips through the real game simulation, level by level, to find
//! anything that stops progress.
//!
//!     playbot <GAME_DIR> [LIPS00 LIPS01 ...] [--minutes N] [--verbose] [--mortal]
//!
//! For each level the bot repeatedly finds the nearest unused thing it can actually get to
//! (switch walls it walks into, pickups, working teleporters, shootable switches, region
//! triggers, key bots to shoot), walks there, uses it, and re-plans. Reachability is tested
//! with the player's own physics (walking, running, jumping, steps and drops), so a path the
//! bot finds is one a player can walk. It stops when the level's exit loads the next level,
//! or reports what is left when nothing new can be reached.

use godd_game::player::{PhysicsParams, Player};
use godd_game::{Action, Game, Input, Options, Vfs};
use std::collections::{HashMap, HashSet};
use std::path::Path;

type AnyResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Planner grid spacing (world units).
const STEP: f32 = 32.0;
/// Tags that only send the player back (3: DEATH01 teleport to the start; 661: death).
const DEATH_TAGS: [i32; 2] = [3, 661];
/// Most planner states explored per plan.
const MAX_NODES: usize = 40_000;
const TICK_RATE: f32 = 60.0;

/// Object effects (re/engine/object-effects.md).
const OWALK: i32 = 5;
const OTELE: i32 = 15;
const OTAGSW: i32 = 20;
const OBUMPSW: i32 = 22;
const OPICKUP: i32 = 130;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Target {
    Wall(usize),
    Object(usize),
    Region(usize),
    Lift(usize),
}

#[derive(Debug, Clone, Copy)]
struct Goal {
    target: Target,
    /// Where to stand.
    at: [f32; 3],
    /// Then face this way and walk forward (switch walls), or shoot (bots).
    face: Option<f32>,
    shoot: bool,
}

/// A planner grid spot: x and y in grid steps, z in 24-unit bands.
type Cell = (i32, i32, i32);

/// A cached planner step: where it starts and ends, and the heights it was tested with.
#[derive(Debug, Clone, Copy)]
struct Edge {
    /// Door and mover heights when tested (a door between the ends may have moved since).
    doors: u64,
    from_region: usize,
    from_heights: (u32, u32),
    to: Option<(Player, (bool, bool))>,
    to_heights: (u32, u32),
}

/// One planner step: the point walked to and whether it needed a jump.
#[derive(Debug, Clone, Copy)]
struct Step {
    to: (f32, f32),
    jump: bool,
    /// Crouch the whole way (crawlspaces).
    crouch: bool,
}

struct Bot<'a> {
    vfs: &'a Vfs,
    game: Game,
    level: String,
    used: HashSet<Target>,
    /// Targets that could not be reached since the world last changed.
    unreachable: HashSet<Target>,
    /// Failed attempts per target; after 3 it waits until something new has been used.
    failures: HashMap<Target, u32>,
    world_key: u64,
    /// Consecutive plans that found nothing reachable.
    waits: u32,
    /// Regions the last switch opened up (explored first).
    opened: HashSet<usize>,
    /// Consecutive plans that only re-used something.
    repeats: u32,
    /// Every region the player has stood in.
    visited: HashSet<usize>,
    /// Planner steps already simulated: (grid spot, direction) → result.
    edges: HashMap<(Cell, (i32, i32)), Edge>,
    ticks: u64,
    verbose: bool,
    /// Enemy shots do not send the player back (progression, not combat, is under test).
    god: bool,
    log: Vec<String>,
}

fn facing(from: (f32, f32), to: (f32, f32)) -> f32 {
    (to.1 - from.1).atan2(to.0 - from.0)
}

impl<'a> Bot<'a> {
    fn session(&self) -> &godd_game::session::Session {
        self.game.session().expect("a level is running")
    }

    /// Changes whenever doors, lifts and other moving parts have moved.
    fn world_key(&self) -> u64 {
        let w = &self.session().level.world;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for r in &w.regions {
            for v in [r.floor, r.ceiling] {
                h = (h ^ v.to_bits() as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    /// Fire the level's death teleport (an OTELE on a death tag), unless already standing where it
    /// leads; false if there is none.
    fn respawn(&mut self) -> bool {
        let s = self.session();
        let Some(tag) = s.objects.list.iter().find(|o| o.effect == OTELE && DEATH_TAGS.contains(&o.tag)).map(|o| o.tag) else { return false };
        let before = (s.player.x, s.player.y);
        self.game.session_mut().expect("session").fire_tag(tag as u32);
        let p = self.session().player;
        (p.x - before.0).hypot(p.y - before.1) > 64.0
    }

    /// Regions connected to the player's through open portals.
    fn area(&self) -> HashSet<usize> {
        let s = self.session();
        let hops = s.portal_hops(s.player.region);
        (0..hops.len()).filter(|&r| hops[r] != u8::MAX).collect()
    }

    /// Changes when a door or an OSETF/OSETC-moved region (elevator) changes height; perpetual
    /// lifts do not count.
    fn doors_key(&self) -> u64 {
        let s = self.session();
        let w = &s.level.world;
        let names = s.level.region_names();
        let mut regions: Vec<usize> = s.effects.doors().into_iter().map(|(r, _)| r).collect();
        regions.extend(s.objects.list.iter().filter(|o| matches!(o.effect, 41 | 49)).filter_map(|o| {
            let target = o.aux.as_ref()?.slot(9)?;
            names.iter().position(|n| n.eq_ignore_ascii_case(target))
        }));
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for r in regions {
            let reg = &w.regions[r];
            for v in [reg.floor, reg.ceiling] {
                // Coarse, so a door creeping by fractions does not flush everything.
                h = (h ^ (v / 16.0).round() as i64 as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
        // Walls copied over by OWTOW (wall doors) change which lines block.
        for (i, wall) in w.walls.iter().enumerate() {
            if wall.blocking && wall.back.is_some() {
                h = (h ^ i as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    /// Everything still worth walking to.
    fn goals(&self) -> Vec<Goal> {
        let s = self.session();
        let w = &s.level.world;
        let mut goals = Vec::new();
        // Tag 3 (DEATH01: back to the start) and 661 (death) only send the player back.
        let pointless = |tags: &std::ops::RangeInclusive<i32>| tags.clone().all(|t| t == 3 || t == 661);
        // A switch is worth using while it would change something: a door of its tags is
        // closed, or a floor/ceiling mover (OSETF/OSETC: elevators, the coffin) of its tags has
        // its region away from the target height. Others are used once.
        let doors = s.effects.doors();
        let names = s.level.region_names();
        let useful = |tags: &std::ops::RangeInclusive<i32>| -> Option<bool> {
            let door: Vec<usize> = doors.iter().filter(|(_, t)| tags.contains(t)).map(|&(r, _)| r).collect();
            let movers: Vec<bool> = s
                .objects
                .list
                .iter()
                .filter(|o| matches!(o.effect, 41 | 49) && tags.contains(&o.tag))
                .filter_map(|o| {
                    let a = o.aux.as_ref()?;
                    let target = a.slot(9)?;
                    let r = names.iter().position(|n| n.eq_ignore_ascii_case(target))?;
                    let reg = &w.regions[r];
                    let at = if o.effect == 41 { reg.floor } else { reg.ceiling };
                    Some((at - a.ints[0] as f32).abs() > 1.0)
                })
                .collect();
            if door.is_empty() && movers.is_empty() {
                return None;
            }
            Some(door.iter().any(|&r| w.regions[r].ceiling - w.regions[r].floor < 56.0) || movers.iter().any(|&m| m))
        };
        for sw in s.effects.switches().into_iter().filter(|sw| sw.usable && *sw.tags.start() > 0 && !pointless(&sw.tags)) {
            match useful(&sw.tags) {
                Some(false) => continue,
                Some(true) => {}
                None if self.used.contains(&Target::Wall(sw.wall)) => continue,
                None => {}
            }
            let wall = &w.walls[sw.wall];
            let (a, b) = (w.vertices[wall.a], w.vertices[wall.b]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = dx.hypot(dy).max(1e-3);
            let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            // Stand facing the wall where there is room: in front of it if possible, else on
            // either side with open space (a door face fires when walked into from outside).
            let open = |r: usize| w.regions.get(r).is_some_and(|reg| reg.ceiling - reg.floor > 40.0);
            let joined = |a: usize, b: usize| w.regions[b].walls.iter().chain(&w.regions[a].walls).any(|&wi| {
                let x = &w.walls[wi];
                (x.front == a && x.back == Some(b)) || (x.front == b && x.back == Some(a))
            });
            let spot = [1.0f32, -1.0]
                .into_iter()
                .map(|side| {
                    let n = (-dy / len * side, dx / len * side);
                    (mid.0 + n.0 * 40.0, mid.1 + n.1 * 40.0)
                })
                .filter_map(|at| w.region_at(at.0, at.1).filter(|&r| open(r)).map(|r| (at, r)))
                // The wall's own region, else one opening onto it (a button alcove's room).
                .min_by_key(|&(_, r)| if r == wall.front { 0 } else if joined(r, wall.front) { 1 } else { 2 });
            if let Some((at, r)) = spot {
                let z = w.regions[r].floor;
                goals.push(Goal { target: Target::Wall(sw.wall), at: [at.0, at.1, z], face: Some(facing(at, mid)), shoot: false });
            }
        }
        for (i, o) in s.objects.list.iter().enumerate() {
            if o.destroyed || o.hidden || o.region.is_none() {
                continue;
            }
            let walk_into = match o.effect {
                OPICKUP => true,
                OTELE => o.name.is_empty() && o.aux.is_some(),
                OTAGSW | OBUMPSW => true,
                // Plain walking bots fire their tag when bumped.
                OWALK => o.mover.is_none() && o.tag > 0,
                _ => false,
            };
            if walk_into {
                goals.push(Goal { target: Target::Object(i), at: [o.x, o.y, o.z], face: None, shoot: false });
            } else if o.name.starts_with("KEYBOT") && o.mover.is_some() {
                goals.push(Goal { target: Target::Object(i), at: [o.x, o.y, o.z], face: None, shoot: true });
            }
        }
        // Traps: tags that move the floor of a region where landing kills (LIPS00's FALLZ sinks
        // when you step in front of it; walking straight on rides it down, stepping in and back
        // out first leaves a deadly drop).
        let names = s.level.region_names();
        let triggers = s.effects.triggers();
        let deadly_land: HashSet<usize> = triggers.iter().filter(|t| DEATH_TAGS.contains(&t.1[2])).map(|t| t.0).collect();
        let trap_tags: HashSet<i32> = s
            .objects
            .list
            .iter()
            .filter(|o| o.effect == 41)
            .filter(|o| o.aux.as_ref().and_then(|a| a.slot(9)).and_then(|n| names.iter().position(|rn| rn.eq_ignore_ascii_case(n))).is_some_and(|r| deadly_land.contains(&r)))
            .map(|o| o.tag)
            .collect();
        for &(r, tags) in &triggers {
            if tags.iter().all(|t| *t <= 0 || DEATH_TAGS.contains(t) || trap_tags.contains(t)) {
                continue;
            }
            let ((x0, y0), (x1, y1)) = w.regions[r].bounds;
            let c = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            if w.region_at(c.0, c.1) == Some(r) {
                goals.push(Goal { target: Target::Region(r), at: [c.0, c.1, w.regions[r].floor], face: None, shoot: false });
            }
        }
        for (r, _) in s.effects.lifts() {
            let ((x0, y0), (x1, y1)) = w.regions[r].bounds;
            let c = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            if w.region_at(c.0, c.1) == Some(r) && (x1 - x0).min(y1 - y0) >= 40.0 {
                goals.push(Goal { target: Target::Lift(r), at: [c.0, c.1, w.regions[r].floor], face: None, shoot: false });
            }
        }
        // A used trigger region is worth another visit while a door it opens is shut again.
        let doors: Vec<(usize, i32)> = s.effects.doors();
        let shut_tags: HashSet<i32> = doors.iter().filter(|(r, _)| w.regions[*r].ceiling - w.regions[*r].floor < 20.0).map(|d| d.1).collect();
        let reopens = |r: usize| triggers.iter().any(|(t, tags)| *t == r && tags.iter().any(|x| shut_tags.contains(x)));
        let here = s.player.region;
        goals.retain(|g| !matches!(g.target, Target::Region(r) if r == here));
        // Likewise bump/shoot switch objects (they toggle their door).
        let reswitch = |i: usize| s.objects.list.get(i).is_some_and(|o| matches!(o.effect, OTAGSW | OBUMPSW) && shut_tags.contains(&o.tag));
        goals.retain(|g| (matches!(g.target, Target::Wall(_)) || matches!(g.target, Target::Region(r) if reopens(r)) || matches!(g.target, Target::Object(i) if reswitch(i)) || !self.used.contains(&g.target)) && !self.unreachable.contains(&g.target) && self.failures.get(&g.target).is_none_or(|&n| n < 3));
        goals
    }

    /// Simulate a player walking (running, optionally jumping) from `p` toward `to`; the state
    /// where it arrives, if it does.
    fn try_move(world: &godd_game::world::World, physics: &PhysicsParams, mut p: Player, to: (f32, f32), jump: bool, crouch: bool) -> Option<Player> {
        for t in 0..48 {
            let before = (p.x - to.0).hypot(p.y - to.1);
            // Run while far, walk the last stretch; arrived once the spot is within one step.
            let mut i = Input::default();
            i.set(Action::Forward, true);
            i.set(Action::Run, before > 48.0 && !jump);
            i.set(Action::Jump, jump && t == 0);
            i.set(Action::Crouch, crouch);
            p.angle = facing((p.x, p.y), to);
            let (x0, y0) = (p.x, p.y);
            p.tick(world, &i, physics);
            let moved = (p.x - x0).hypot(p.y - y0);
            // Arrived: on the spot, about to pass it, or held up just short of it.
            let after = (p.x - to.0).hypot(p.y - to.1);
            if after < 6.0 || (before <= moved + 1.0 && moved > 0.5) || (after < 16.0 && moved < 0.5 && p.on_ground) {
                // Settle: come to rest on the ground.
                let idle = Input::default();
                for _ in 0..300 {
                    if p.on_ground {
                        break;
                    }
                    p.tick(world, &idle, physics);
                }
                return p.on_ground.then_some(p);
            }
        }
        None
    }

    /// A* over standing spots from the player to the nearest goal. Each step is a simulated
    /// walk (or jump) to a neighbouring grid spot; results are cached while the regions at
    /// both ends keep their heights.
    fn plan(&mut self, goals: &[Goal]) -> Option<(Goal, Vec<Step>)> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let s = self.game.session().expect("session");
        let world = &s.level.world;
        let physics = s.physics_override.unwrap_or_default();
        let start = s.player;
        let key = |p: &Player| ((p.x / STEP).round() as i32, (p.y / STEP).round() as i32, (p.z / 24.0).round() as i32);
        // Lifts move: reaching their floor area at any height counts.
        let near_goal = |p: &Player| {
            goals.iter().find(|g| (p.x - g.at[0]).hypot(p.y - g.at[1]) < STEP * 0.75 && (matches!(g.target, Target::Lift(r) if p.region == r) || (p.z - g.at[2]).abs() < 72.0)).copied()
        };
        let heuristic = |p: &Player| goals.iter().map(|g| (p.x - g.at[0]).hypot(p.y - g.at[1])).fold(f32::MAX, f32::min) / STEP;
        let heights = |r: usize| world.regions.get(r).map_or((0, 0), |reg| (reg.floor.to_bits(), reg.ceiling.to_bits()));
        let doors = self.doors_key();
        let triggers = s.effects.triggers();
        let deadly_enter: HashSet<usize> = triggers.iter().filter(|t| DEATH_TAGS.contains(&t.1[0])).map(|t| t.0).collect();
        let deadly_land: HashSet<usize> = triggers.iter().filter(|t| DEATH_TAGS.contains(&t.1[2])).map(|t| t.0).collect();
        // Switch objects fire (and toggle doors back) on touch: walk around the ones that are
        // not the goal.
        let goal_objects: HashSet<usize> = goals.iter().filter_map(|g| if let Target::Object(i) = g.target { Some(i) } else { None }).collect();
        let avoid: Vec<(f32, f32, f32, f32)> = s
            .objects
            .list
            .iter()
            .enumerate()
            .filter(|(i, o)| matches!(o.effect, OTAGSW | OBUMPSW) && !o.hidden && !o.destroyed && o.region.is_some() && !goal_objects.contains(i))
            .map(|(_, o)| (o.x, o.y, o.z, o.radius + 16.0 + 2.0))
            .collect();
        // Spot → (player there, path cost, where it was reached from).
        type Visit = (Player, f32, Option<(Cell, Step)>);
        let mut came: HashMap<Cell, Visit> = HashMap::new();
        let mut open = BinaryHeap::new();
        came.insert(key(&start), (start, 0.0, None));
        open.push((Reverse((heuristic(&start) * 1000.0) as i64), key(&start)));
        let mut expanded = 0;
        while let Some((_, k)) = open.pop() {
            let (p, cost, _) = came[&k];
            if let Some(goal) = near_goal(&p) {
                let mut steps = Vec::new();
                let mut k = k;
                while let Some((_, _, Some((prev, step)))) = came.get(&k).copied() {
                    steps.push(step);
                    k = prev;
                }
                steps.reverse();
                let crouch = steps.last().is_some_and(|s: &Step| s.crouch);
                steps.push(Step { to: (goal.at[0], goal.at[1]), jump: false, crouch });
                return Some((goal, steps));
            }
            expanded += 1;
            if expanded > MAX_NODES {
                return None;
            }
            const MOVES: [(f32, f32); 20] = [
                (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0),
                // Jumps with a run-up onto ledges too high to jump from right next to them.
                (2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0), (2.0, 2.0), (2.0, -2.0), (-2.0, 2.0), (-2.0, -2.0),
                (3.0, 0.0), (-3.0, 0.0), (0.0, 3.0), (0.0, -3.0),
            ];
            for (dx, dy) in MOVES {
                let to = (((p.x / STEP).round() + dx) * STEP, ((p.y / STEP).round() + dy) * STEP);
                let long = dx.abs() > 1.0 || dy.abs() > 1.0;
                if long && !world.region_at(to.0, to.1).is_some_and(|r| world.solid_floor(r).0 > p.z + physics.step_height) {
                    continue;
                }
                let edge = (k, (dx as i32, dy as i32));
                let cached = self.edges.get(&edge).filter(|e| e.doors == doors && heights(e.from_region) == e.from_heights && e.to.is_none_or(|(q, _)| heights(q.region) == e.to_heights)).map(|e| e.to);
                let result = match cached {
                    Some(r) => r,
                    None => {
                        let kinds: &[(bool, bool)] = if long { &[(true, false)] } else { &[(false, false), (true, false), (false, true)] };
                        let r = kinds.iter().copied().find_map(|(jump, crouch)| Self::try_move(world, &physics, p, to, jump, crouch).map(|q| (q, (jump, crouch))));
                        self.edges.insert(edge, Edge { doors, from_region: p.region, from_heights: heights(p.region), to: r, to_heights: r.map_or((0, 0), |(q, _)| heights(q.region)) });
                        r
                    }
                };
                let Some((q, (jump, crouch))) = result else { continue };
                // Never into a region that kills on entry (tag 3 sends you back to the start),
                // nor jump where landing does.
                if deadly_enter.contains(&q.region) || (jump && deadly_land.contains(&q.region)) {
                    continue;
                }
                if avoid.iter().any(|&(x, y, z, r)| (q.x - x).abs() < r && (q.y - y).abs() < r && (q.z - z).abs() < 64.0) {
                    continue;
                }
                let qk = key(&q);
                // Corners cut off a deadly-landing region (FALLZ's chamfers) belong to its
                // container: entering the trap from there comes too late to ride its floor down.
                let corner = deadly_land.iter().any(|&r| {
                    let ((x0, y0), (x1, y1)) = world.regions[r].bounds;
                    q.region != r && q.x > x0 - 8.0 && q.x < x1 + 8.0 && q.y > y0 - 8.0 && q.y < y1 + 8.0
                });
                let c = cost + dx.hypot(dy) + if corner { 8.0 } else { 0.0 };
                if came.get(&qk).is_none_or(|e| c < e.1) {
                    came.insert(qk, (q, c, Some((k, Step { to: (q.x, q.y), jump, crouch }))));
                    open.push((Reverse(((c + heuristic(&q)) * 1000.0) as i64), qk));
                }
            }
        }
        None
    }

    fn tick(&mut self, input: &Input) {
        if let Some(s) = self.game.session_mut() {
            s.god = self.god;
        }
        let before = self.game.session().map(|s| s.player);
        self.game.tick(self.vfs, input);
        self.ticks += 1;
        if let Some(s) = self.game.session() {
            self.visited.insert(s.player.region);
        }
        if self.verbose
            && let (Some(a), Some(b)) = (before, self.game.session().map(|s| s.player))
            && (a.x - b.x).hypot(a.y - b.y) > 200.0
        {
            println!("     teleported from ({:.0},{:.0},{:.0}) r{} to ({:.0},{:.0},{:.0}) r{}", a.x, a.y, a.z, a.region, b.x, b.y, b.z, b.region);
        }
    }

    /// The level changed (exit taken, or a death sent us to another level).
    fn level_done(&self) -> bool {
        self.game.session().is_none_or(|s| !s.name.eq_ignore_ascii_case(&self.level))
    }

    /// Walk the planned steps in the real game; false if the player gets stuck.
    fn walk(&mut self, steps: &[Step]) -> bool {
        self.walk_to(steps, 12.0)
    }

    /// `walk` with how close the last step must get.
    fn walk_to(&mut self, steps: &[Step], close_last: f32) -> bool {
        let last = steps.len().saturating_sub(1);
        for (k, step) in steps.iter().enumerate() {
            // Intermediate spots only need passing near; the goal itself a little closer.
            let close = if k == last { close_last } else { 16.0 };
            let mut best = f32::MAX;
            let mut stalled = 0;
            loop {
                if self.level_done() {
                    return true;
                }
                let p = self.session().player;
                let d = (p.x - step.to.0).hypot(p.y - step.to.1);
                if d < close {
                    break;
                }
                if d < best - 0.5 {
                    best = d;
                    stalled = 0;
                } else {
                    stalled += 1;
                    if stalled > 90 {
                        if self.verbose {
                            let w = &self.session().level.world;
                            let to = w.region_at(step.to.0, step.to.1);
                            println!("     stalled at step {k}/{last}: player r{} z {:.0} → ({:.0}, {:.0}) r{:?} floor {:?} ceil {:?}, d {d:.0}", p.region, p.z, step.to.0, step.to.1, to, to.map(|r| w.regions[r].floor), to.map(|r| w.regions[r].ceiling));
                        }
                        // Within reach of the goal counts.
                        return k == last && d < 32.0;
                    }
                }
                let angle = facing((p.x, p.y), step.to);
                let s = self.game.session_mut().expect("session");
                s.player.angle = angle;
                let mut input = Input::default();
                input.set(Action::Forward, true);
                input.set(Action::Run, d > 48.0 && !step.jump);
                input.set(Action::Crouch, step.crouch);
                // The planned jump, or a hop when stuck against a ledge.
                let deadly = self.session().effects.triggers().iter().any(|t| t.0 == p.region && DEATH_TAGS.contains(&t.1[2]));
                // Planned jumps land somewhere safe; unplanned hops not where landing kills.
                input.set(Action::Jump, (step.jump && stalled == 0 && best == d) || (!deadly && (stalled == 30 || stalled == 60)));
                self.tick(&input);
            }
        }
        true
    }

    /// Use the goal once there: walk into the wall, or shoot the bot.
    fn interact(&mut self, goal: &Goal) {
        if let Some(face) = goal.face {
            let mut input = Input::default();
            input.set(Action::Forward, true);
            for _ in 0..30 {
                if self.level_done() {
                    return;
                }
                self.game.session_mut().expect("session").player.angle = face;
                self.tick(&input);
            }
            // Give doors time to open, and ride whatever the floor does (elevators): wait until
            // the region under the player has stopped moving for half a second.
            let idle = Input::default();
            let mut still = 0;
            let mut last = None;
            for t in 0..(TICK_RATE as usize * 8) {
                if self.level_done() {
                    return;
                }
                self.tick(&idle);
                let s = self.session();
                let r = &s.level.world.regions[s.player.region];
                let now = (s.player.region, r.floor, r.ceiling);
                still = if Some(now) == last { still + 1 } else { 0 };
                last = Some(now);
                if t > TICK_RATE as usize * 2 && still > 30 {
                    break;
                }
            }
            // A door face: walk straight through while it is open, as a player would (stepping
            // back and touching the face again would close it).
            if let Target::Wall(w) = goal.target {
                let s = self.session();
                let wall = &s.level.world.walls[w];
                let (front, back) = (wall.front, wall.back);
                if let Some(door) = back.filter(|b| s.effects.doors().iter().any(|d| d.0 == *b)) {
                    let mut input = Input::default();
                    input.set(Action::Forward, true);
                    for _ in 0..(TICK_RATE as usize * 3) {
                        if self.level_done() {
                            return;
                        }
                        let r = self.session().player.region;
                        if r != front && r != door {
                            break;
                        }
                        self.game.session_mut().expect("session").player.angle = face;
                        self.tick(&input);
                    }
                }
            }
        } else if goal.shoot {
            let Target::Object(i) = goal.target else { return };
            let mut fire = Input::default();
            fire.set(Action::Use, true);
            for _ in 0..(TICK_RATE as usize * 6) {
                if self.level_done() {
                    return;
                }
                let s = self.session();
                let Some(o) = s.objects.list.get(i) else { break };
                if o.destroyed || !o.name.starts_with("KEYBOT") {
                    break;
                }
                let angle = facing((s.player.x, s.player.y), (o.x, o.y));
                self.game.session_mut().expect("session").player.angle = angle;
                self.tick(&fire);
            }
        } else if let Target::Object(i) = goal.target
            && self.session().objects.list.get(i).is_some_and(|o| matches!(o.effect, OTAGSW | OBUMPSW))
        {
            // A switch object: let what it started (doors) finish before re-planning.
            let idle = Input::default();
            for _ in 0..(TICK_RATE as usize * 3) {
                if self.level_done() {
                    return;
                }
                self.tick(&idle);
            }
        } else if let Target::Lift(r) = goal.target {
            // Ride to the other end of the lift's travel, then plan from there.
            let idle = Input::default();
            let start = self.session().level.world.regions[r].floor;
            let mut far = start;
            for _ in 0..(TICK_RATE as usize * 40) {
                if self.level_done() {
                    return;
                }
                self.tick(&idle);
                let s = self.session();
                if s.player.region != r {
                    return; // pushed off
                }
                let f = s.level.world.regions[r].floor;
                if (f - start).abs() > (far - start).abs() {
                    far = f;
                } else if (far - start).abs() > 32.0 && (f - far).abs() > 2.0 {
                    return; // reached the far end and turned back
                }
            }
        } else {
            // Stand still a moment for pickups, teleports and triggers to happen.
            let idle = Input::default();
            for _ in 0..30 {
                self.tick(&idle);
            }
        }
    }

    /// Why the reachable area ends: its closed doors (tag, what fires it, whether used) and
    /// steps too high to climb.
    fn boundary_report(&self) -> String {
        let s = self.session();
        let w = &s.level.world;
        // Everywhere the player has stood, and what borders it unvisited.
        let inside_set = &self.visited;
        let doors: HashMap<usize, i32> = s.effects.doors().into_iter().collect();
        let switches = s.effects.switches();
        let triggers = s.effects.triggers();
        let mut seen = HashSet::new();
        let mut lines = Vec::new();
        for wall in &w.walls {
            let Some(b) = wall.back.filter(|&b| b != wall.front) else { continue };
            let (inside, out) = if inside_set.contains(&wall.front) && !inside_set.contains(&b) { (wall.front, b) } else if inside_set.contains(&b) && !inside_set.contains(&wall.front) { (b, wall.front) } else { continue };
            if !seen.insert(out) {
                continue;
            }
            let (ri, ro) = (&w.regions[inside], &w.regions[out]);
            let shut = ro.ceiling - ro.floor < 20.0;
            if let Some(&tag) = doors.get(&out).filter(|_| shut) {
                let by_walls: Vec<String> = switches.iter().filter(|sw| sw.tags.contains(&tag)).map(|sw| format!("wall {}{}", sw.wall, if self.used.contains(&Target::Wall(sw.wall)) { " (used)" } else { "" })).collect();
                // Objects that fire the tag: bump/shoot switches (own tag), pickups and drops
                // (int0), counters (int3).
                let by_objects: Vec<String> = s
                    .objects
                    .list
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| {
                        let int = |k: usize| o.aux.as_ref().map_or(0, |a| a.ints[k]);
                        match o.effect {
                            20 | 22 => o.tag == tag,
                            130 | 131 => int(0) == tag,
                            72 => int(3) == tag,
                            _ => false,
                        }
                    })
                    .map(|(i, o)| format!("obj {i} {} eff {} at ({:.0}, {:.0}, {:.0}) region {:?}{}{}", o.name, o.effect, o.x, o.y, o.z, o.region, if o.hidden { " hidden" } else { "" }, if self.used.contains(&Target::Object(i)) { " (used)" } else { "" }))
                    .collect();
                let by_triggers: Vec<String> = triggers.iter().filter(|(_, t)| t.contains(&tag)).map(|(r, _)| format!("trigger region {r}")).collect();
                lines.push(format!("door r{out} (tag {tag}) from r{inside}: fired by {:?} {:?} {:?}", by_walls, by_objects, by_triggers));
            } else if std::env::var("PLAYBOT_ALL_EDGES").is_ok() {
                lines.push(format!("unvisited r{out} from r{inside}: floor {} ceil {} vs floor {} ceil {}", ro.floor, ro.ceiling, ri.floor, ri.ceiling));
            }
        }
        lines.join("\n    ")
    }

    fn note(&mut self, line: String) {
        if self.verbose {
            println!("  {line}");
        }
        self.log.push(line);
    }

    /// Cross the BSP world (LIPS00's catacombs) to its exit along the planned walk.
    fn cross_bsp(&mut self) -> Result<(), String> {
        let route = self.session().bsp.as_ref().and_then(|b| b.path_to_exit()).ok_or("no walkable path out of the BSP world")?;
        self.note(format!("t={:.0}s → BSP world exit ({} steps)", self.ticks as f32 / TICK_RATE, route.len()));
        let mut walk = Input::default();
        walk.set(Action::Forward, true);
        let idle = Input::default();
        for target in route.iter().skip(1) {
            let mut left = TICK_RATE as u32 * 20;
            loop {
                let s = self.game.session_mut().expect("session");
                if !s.in_bsp {
                    return Ok(());
                }
                let w = s.bsp.as_mut().expect("bsp");
                let p = w.player.pos;
                let (dx, dy) = (target[0] - p[0], target[1] - p[1]);
                if dx.hypot(dy) < 6.0 {
                    break;
                }
                w.look((-dx).atan2(dy) - w.player.yaw, 0.0);
                let falling = w.player.airborne;
                self.tick(if falling { &idle } else { &walk });
                left -= 1;
                if left == 0 {
                    return Err(format!("stuck in the BSP world at {p:?}"));
                }
            }
        }
        // At the last spot: give the exit cube a moment.
        for _ in 0..30 {
            self.tick(&idle);
        }
        if self.session().in_bsp { Err("the BSP exit did not bring the player back".into()) } else { Ok(()) }
    }

    /// Play the level until it is left or nothing new is reachable; Ok(next level name).
    fn play(&mut self, max_ticks: u64) -> Result<String, String> {
        while self.ticks < max_ticks {
            if self.level_done() {
                return Ok(self.game.session().map_or_else(String::new, |s| s.name.clone()));
            }
            if self.session().in_bsp {
                self.cross_bsp()?;
                continue;
            }
            let key = self.world_key();
            if key != self.world_key {
                self.world_key = key;
                self.unreachable.clear();
            }
            let goals = self.goals();
            if goals.is_empty() {
                return Err("nothing left to use".into());
            }
            // Whatever the last switch opened up comes first (before a door closes again), then
            // things to collect and places to visit; switches and lifts change the world (an
            // elevator's other button would undo the ride), so only when nothing else is reachable.
            let w = &self.session().level.world;
            let fresh: Vec<Goal> = goals.iter().filter(|g| w.region_at(g.at[0], g.at[1]).is_some_and(|r| self.opened.contains(&r))).copied().collect();
            let (firsts, switches): (Vec<Goal>, Vec<Goal>) = goals.iter().partition(|g| !matches!(g.target, Target::Wall(_) | Target::Lift(_)));
            let mut found = if fresh.is_empty() { None } else { self.plan(&fresh) };
            if found.is_none() && !firsts.is_empty() {
                found = self.plan(&firsts);
            }
            if self.verbose && std::env::var("PLAYBOT_WHY").is_ok_and(|v| goals.iter().any(|g| format!("{:?}", g.target) == v)) {
                let v = std::env::var("PLAYBOT_WHY").unwrap();
                let g: Vec<Goal> = goals.iter().filter(|g| format!("{:?}", g.target) == v).copied().collect();
                let r = self.plan(&g);
                let p = self.session().player;
                println!("     why {v}: goal at {:?} region {:?}; from ({:.0},{:.0},{:.0}) r{} plan {:?}", g[0].at, self.session().level.world.region_at(g[0].at[0], g[0].at[1]), p.x, p.y, p.z, p.region, r.map(|r| r.1.len()));
            }
            if let Ok(rs) = std::env::var("PLAYBOT_REGIONS") {
                let w = &self.session().level.world;
                let info: Vec<String> = rs.split(',').filter_map(|x| x.parse::<usize>().ok()).map(|r| format!("r{r} {}/{}", w.regions[r].floor, w.regions[r].ceiling)).collect();
                println!("     regions {info:?}");
            }
            if let Ok(spots) = std::env::var("PLAYBOT_AT") {
                for spot in spots.split(';') {
                    let v: Vec<f32> = spot.split(',').filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 3 {
                        let g = Goal { target: Target::Region(usize::MAX), at: [v[0], v[1], v[2]], face: None, shoot: false };
                        let r = self.plan(&[g]).map(|r| r.1.len());
                        println!("     reach ({}, {}, {}): {:?}", v[0], v[1], v[2], r);
                    }
                }
            }
            // Switches never pressed before ones that were (toggling the same doors forever).
            let (new, old): (Vec<Goal>, Vec<Goal>) = switches.iter().partition(|g| !self.used.contains(&g.target));
            let found = found.or_else(|| if new.is_empty() { None } else { self.plan(&new) });
            let found = found.or_else(|| self.plan(&old));
            let Some((goal, steps)) = found else {
                // Nothing reachable now: wait for moving parts (a few times), then give up.
                let idle = Input::default();
                for _ in 0..TICK_RATE as usize {
                    self.tick(&idle);
                }
                self.waits += 1;
                if self.waits > 8 && self.respawn() {
                    // Trapped (a pit with no way up): a player gets out by dying, which sends them
                    // back to the start; do the same.
                    self.note("     no way on: back to the start (death teleport)".into());
                    self.waits = 0;
                    self.edges.clear();
                    continue;
                }
                if self.waits > 8 {
                    let left: Vec<String> = goals.iter().map(|g| format!("{:?} at ({:.0}, {:.0}, {:.0})", g.target, g.at[0], g.at[1], g.at[2])).collect();
                    let p = self.session().player;
                    let _ = left;
                    if self.verbose {
                        println!("  failures: {:?}", self.failures.iter().filter(|e| *e.1 > 0).collect::<Vec<_>>());
                        println!("  player {:?} override {:?}", self.session().player, self.session().physics_override);
                        let w = &self.session().level.world;
                        println!("  door236 floor {} ceil {}; r0 floor {} ceil {}; r234 floor {} ceil {}", w.regions[236].floor, w.regions[236].ceiling, w.regions[0].floor, w.regions[0].ceiling, w.regions[234].floor, w.regions[234].ceiling);
                        for g in goals.iter().filter(|g| (g.at[0] - p.x).hypot(g.at[1] - p.y) < 600.0) {
                            let w = &self.session().level.world;
                            println!("  near target {:?} at ({:.0}, {:.0}, {:.0}) region {:?}", g.target, g.at[0], g.at[1], g.at[2], w.region_at(g.at[0], g.at[1]));
                        }
                    }
                    return Err(format!("stuck at ({:.0}, {:.0}, {:.0}) region {}; {} targets left. The reachable area ends at:\n    {}", p.x, p.y, p.z, p.region, goals.len(), self.boundary_report()));
                }
                continue;
            };
            self.waits = 0;
            // Only ever re-pressing old switches: nothing new is reachable.
            if self.used.contains(&goal.target) {
                self.repeats += 1;
                if self.repeats > 60 {
                    let p = self.session().player;
                    return Err(format!("only old switches left at ({:.0}, {:.0}, {:.0}) region {}. The reachable area ends at:\n    {}", p.x, p.y, p.z, p.region, self.boundary_report()));
                }
            } else {
                self.repeats = 0;
            }
            self.note(format!("t={:.0}s → {:?} ({} steps)", self.ticks as f32 / TICK_RATE, goal.target, steps.len()));
            let start = { let p = self.session().player; (p.x, p.y) };
            // Switch objects fire on touching; stop at the edge rather than circling inside
            // (leaving and re-entering contact would toggle them again).
            let close = match goal.target {
                Target::Object(i) if self.session().objects.list.get(i).is_some_and(|o| matches!(o.effect, OTAGSW | OBUMPSW)) => 20.0,
                _ => 12.0,
            };
            let mut reached = self.walk_to(&steps, close);
            // A pickup only counts once it is actually picked up (the walk's "close enough"
            // may stop just out of touch, or mid-hop above it).
            if reached && let Target::Object(i) = goal.target
                && self.session().objects.list.get(i).is_some_and(|o| o.effect == OPICKUP)
            {
                let mut input = Input::default();
                input.set(Action::Forward, true);
                for _ in 0..(TICK_RATE as usize) {
                    let s = self.session();
                    let o = &s.objects.list[i];
                    if o.hidden || o.destroyed || self.level_done() {
                        break;
                    }
                    let angle = facing((s.player.x, s.player.y), (o.x, o.y));
                    let near = (s.player.x - o.x).hypot(s.player.y - o.y) < 4.0;
                    self.game.session_mut().expect("session").player.angle = angle;
                    let idle = Input::default();
                    self.tick(if near { &idle } else { &input });
                }
                let o = &self.session().objects.list[i];
                reached = o.hidden || o.destroyed || self.level_done();
            }
            if reached {
                let before = self.area();
                self.interact(&goal);
                // A trigger that fires on leaving: step back out the way we came.
                if let Target::Region(r) = goal.target
                    && self.session().effects.triggers().iter().any(|(t, tags)| *t == r && tags[1] > 0)
                {
                    let back = steps.len().checked_sub(2).map_or(start, |k| steps[k].to);
                    let _ = self.walk(&[Step { to: back, jump: false, crouch: false }]);
                }
                if matches!(goal.target, Target::Wall(_) | Target::Lift(_)) {
                    let after = self.area();
                    self.opened = after.difference(&before).copied().collect();
                } else {
                    self.opened.clear();
                }
                self.used.insert(goal.target);
                self.failures.clear(); // something changed: try the others again
            } else {
                self.unreachable.insert(goal.target);
                *self.failures.entry(goal.target).or_default() += 1;
                // Cached moves are per grid cell, not per exact spot; recompute them.
                self.edges.clear();
                let p = self.session().player;
                self.note(format!("   could not follow the path to {:?} (at {:.0}, {:.0})", goal.target, p.x, p.y));
            }
        }
        Err(format!("out of time after {} s", self.ticks as f32 / TICK_RATE))
    }
}

fn main() -> AnyResult {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        eprintln!("usage: playbot <GAME_DIR> [LIPSnn ...] [--minutes N] [--verbose] [--mortal]");
        std::process::exit(2);
    };
    let verbose = args.iter().any(|a| a == "--verbose");
    let minutes: f64 = args.iter().position(|a| a == "--minutes").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(30.0);
    let god = !args.iter().any(|a| a == "--mortal");
    let mut levels: Vec<String> = args[1..].iter().filter(|a| a.starts_with("LIPS")).cloned().collect();
    if levels.is_empty() {
        levels = (0..18).map(|i| format!("LIPS{i:02}")).collect();
    }
    let vfs = Vfs::from_dir(Path::new(dir))?;
    let mut failures = 0;
    for level in &levels {
        let game = Game::new(&vfs, &Options { start_level: Some(level.clone()), save: None })?;
        let mut bot = Bot { vfs: &vfs, game, level: level.clone(), used: HashSet::new(), unreachable: HashSet::new(), failures: HashMap::new(), waits: 0, opened: HashSet::new(), repeats: 0, visited: HashSet::new(), edges: HashMap::new(), world_key: 0, ticks: 0, verbose, god, log: Vec::new() };
        let started = std::time::Instant::now();
        let result = bot.play((minutes * 60.0 * TICK_RATE as f64) as u64);
        let game_time = bot.ticks as f32 / TICK_RATE;
        match result {
            Ok(next) => println!("{level}: completed in {game_time:.0} s of play ({} things used) → {next} [{:.1} s]", bot.used.len(), started.elapsed().as_secs_f32()),
            Err(why) => {
                failures += 1;
                println!("{level}: BLOCKED after {game_time:.0} s ({} things used): {why}", bot.used.len());
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}
