//! `playbot`: plays Exploding Lips through the real game simulation, level by level, to find
//! anything that stops progress.
//!
//!     playbot <GAME_DIR> [LIPS00 LIPS01 ...] [--minutes N] [--verbose]
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
/// Most planner states explored per plan.
const MAX_NODES: usize = 40_000;
const TICK_RATE: f32 = 60.0;

/// Object effects (re/engine/object-effects.md).
const OTELE: i32 = 15;
const OTAGSW: i32 = 20;
const OBUMPSW: i32 = 22;
const OPICKUP: i32 = 130;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Target {
    Wall(usize),
    Object(usize),
    Region(usize),
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
    from_region: usize,
    from_heights: (u32, u32),
    to: Option<(Player, bool)>,
    to_heights: (u32, u32),
}

/// One planner step: the point walked to and whether it needed a jump.
#[derive(Debug, Clone, Copy)]
struct Step {
    to: (f32, f32),
    jump: bool,
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
    /// Times the used switches were reset after getting stuck.
    retries: u32,
    /// Planner steps already simulated: (grid spot, direction) → result.
    edges: HashMap<(Cell, (i32, i32)), Edge>,
    ticks: u64,
    verbose: bool,
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

    /// Everything still worth walking to.
    fn goals(&self) -> Vec<Goal> {
        let s = self.session();
        let w = &s.level.world;
        let mut goals = Vec::new();
        for sw in s.effects.switches().into_iter().filter(|sw| sw.usable && *sw.tags.start() > 0) {
            let wall = &w.walls[sw.wall];
            let (a, b) = (w.vertices[wall.a], w.vertices[wall.b]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = dx.hypot(dy).max(1e-3);
            let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            // Stand facing the wall where there is room: in front of it if possible, else on
            // either side with open space (a door face fires when walked into from outside).
            let open = |r: usize| w.regions.get(r).is_some_and(|reg| reg.ceiling - reg.floor > 40.0);
            let spot = [1.0f32, -1.0]
                .into_iter()
                .map(|side| {
                    let n = (-dy / len * side, dx / len * side);
                    (mid.0 + n.0 * 28.0, mid.1 + n.1 * 28.0)
                })
                .filter_map(|at| w.region_at(at.0, at.1).filter(|&r| open(r)).map(|r| (at, r)))
                .min_by_key(|&(_, r)| usize::from(r != wall.front));
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
                _ => false,
            };
            if walk_into {
                goals.push(Goal { target: Target::Object(i), at: [o.x, o.y, o.z], face: None, shoot: false });
            } else if o.name.starts_with("KEYBOT") && o.mover.is_some() {
                goals.push(Goal { target: Target::Object(i), at: [o.x, o.y, o.z], face: None, shoot: true });
            }
        }
        for (r, _) in s.effects.triggers() {
            let ((x0, y0), (x1, y1)) = w.regions[r].bounds;
            let c = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            if w.region_at(c.0, c.1) == Some(r) {
                goals.push(Goal { target: Target::Region(r), at: [c.0, c.1, w.regions[r].floor], face: None, shoot: false });
            }
        }
        goals.retain(|g| !self.used.contains(&g.target) && !self.unreachable.contains(&g.target) && self.failures.get(&g.target).is_none_or(|&n| n < 3));
        goals
    }

    /// Simulate a player walking (running, optionally jumping) from `p` toward `to`; the state
    /// where it arrives, if it does.
    fn try_move(world: &godd_game::world::World, physics: &PhysicsParams, mut p: Player, to: (f32, f32), jump: bool) -> Option<Player> {
        for t in 0..48 {
            let before = (p.x - to.0).hypot(p.y - to.1);
            // Run while far, walk the last stretch; arrived once the spot is within one step.
            let mut i = Input::default();
            i.set(Action::Forward, true);
            i.set(Action::Run, before > 48.0);
            i.set(Action::Jump, jump && t == 0);
            p.angle = facing((p.x, p.y), to);
            let (x0, y0) = (p.x, p.y);
            p.tick(world, &i, physics);
            let moved = (p.x - x0).hypot(p.y - y0);
            if (p.x - to.0).hypot(p.y - to.1) < 6.0 || (before <= moved + 1.0 && moved > 0.5) {
                // Settle: come to rest on the ground.
                let idle = Input::default();
                for _ in 0..90 {
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
        let near_goal = |p: &Player| goals.iter().find(|g| (p.x - g.at[0]).hypot(p.y - g.at[1]) < STEP * 0.75 && (p.z - g.at[2]).abs() < 72.0).copied();
        let heuristic = |p: &Player| goals.iter().map(|g| (p.x - g.at[0]).hypot(p.y - g.at[1])).fold(f32::MAX, f32::min) / STEP;
        let heights = |r: usize| world.regions.get(r).map_or((0, 0), |reg| (reg.floor.to_bits(), reg.ceiling.to_bits()));
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
                steps.push(Step { to: (goal.at[0], goal.at[1]), jump: false });
                return Some((goal, steps));
            }
            expanded += 1;
            if expanded > MAX_NODES {
                return None;
            }
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                let to = (((p.x / STEP).round() + dx) * STEP, ((p.y / STEP).round() + dy) * STEP);
                let edge = (k, (dx as i32, dy as i32));
                let cached = self.edges.get(&edge).filter(|e| heights(e.from_region) == e.from_heights && e.to.is_none_or(|(q, _)| heights(q.region) == e.to_heights)).map(|e| e.to);
                let result = match cached {
                    Some(r) => r,
                    None => {
                        let r = [false, true].into_iter().find_map(|jump| Self::try_move(world, &physics, p, to, jump).map(|q| (q, jump)));
                        self.edges.insert(edge, Edge { from_region: p.region, from_heights: heights(p.region), to: r, to_heights: r.map_or((0, 0), |(q, _)| heights(q.region)) });
                        r
                    }
                };
                let Some((q, jump)) = result else { continue };
                let qk = key(&q);
                let c = cost + if dx != 0.0 && dy != 0.0 { 1.414 } else { 1.0 };
                if came.get(&qk).is_none_or(|e| c < e.1) {
                    came.insert(qk, (q, c, Some((k, Step { to, jump }))));
                    open.push((Reverse(((c + heuristic(&q)) * 1000.0) as i64), qk));
                }
            }
        }
        None
    }

    fn tick(&mut self, input: &Input) {
        self.game.tick(self.vfs, input);
        self.ticks += 1;
    }

    /// The level changed (exit taken, or a death sent us to another level).
    fn level_done(&self) -> bool {
        self.game.session().is_none_or(|s| !s.name.eq_ignore_ascii_case(&self.level))
    }

    /// Walk the planned steps in the real game; false if the player gets stuck.
    fn walk(&mut self, steps: &[Step]) -> bool {
        let last = steps.len().saturating_sub(1);
        for (k, step) in steps.iter().enumerate() {
            // Intermediate spots only need passing near; the goal itself a little closer.
            let close = if k == last { 12.0 } else { 16.0 };
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
                        // Within reach of the goal counts.
                        return k == last && d < 32.0;
                    }
                }
                let angle = facing((p.x, p.y), step.to);
                let s = self.game.session_mut().expect("session");
                s.player.angle = angle;
                let mut input = Input::default();
                input.set(Action::Forward, true);
                input.set(Action::Run, d > 48.0);
                // The planned jump, or a hop when stuck against a ledge.
                input.set(Action::Jump, (step.jump && stalled == 0 && best == d) || stalled == 30 || stalled == 60);
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
            let back = Input::default();
            for _ in 0..30 {
                self.tick(&back);
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
        } else {
            // Stand still a moment for pickups, teleports and triggers to happen.
            let idle = Input::default();
            for _ in 0..30 {
                self.tick(&idle);
            }
        }
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
            let Some((goal, steps)) = self.plan(&goals) else {
                // Nothing reachable now: wait for moving parts (a few times), then give up.
                let idle = Input::default();
                for _ in 0..TICK_RATE as usize {
                    self.tick(&idle);
                }
                self.waits += 1;
                // Switches can be touched again (doors close behind; toggles flip back).
                if self.waits == 4 && self.retries < 6 {
                    self.retries += 1;
                    self.used.retain(|t| !matches!(t, Target::Wall(_)));
                    self.failures.clear();
                    self.waits = 0;
                    self.note(format!("t={:.0}s: stuck, using switches again (retry {})", self.ticks as f32 / TICK_RATE, self.retries));
                    continue;
                }
                if self.waits == 1 && self.verbose {
                    let s = self.session();
                    let p = s.player;
                    println!("  no plan from {p:?} override {:?}", s.physics_override);
                    let physics = s.physics_override.unwrap_or_default();
                    for (dx, dy) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                        let to = (p.x + dx * STEP, p.y + dy * STEP);
                        let mut q = p;
                        let mut input = Input::default();
                        input.set(Action::Forward, true);
                        for _ in 0..48 {
                            q.angle = facing((q.x, q.y), to);
                            q.tick(&s.level.world, &input, &physics);
                        }
                        let reg = &s.level.world.regions[q.region];
                        println!("    move ({dx},{dy}) ends at ({:.1}, {:.1}, {:.1}) region {} (floor {} ceil {}) ground {}", q.x, q.y, q.z, q.region, reg.floor, reg.ceiling, q.on_ground);
                    }
                }
                if self.waits > 8 {
                    let left: Vec<String> = goals.iter().map(|g| format!("{:?} at ({:.0}, {:.0}, {:.0})", g.target, g.at[0], g.at[1], g.at[2])).collect();
                    let p = self.session().player;
                    return Err(format!("stuck at ({:.0}, {:.0}, {:.0}) region {}; unreachable: {}", p.x, p.y, p.z, p.region, left.join("; ")));
                }
                continue;
            };
            self.waits = 0;
            self.note(format!("t={:.0}s → {:?} ({} steps)", self.ticks as f32 / TICK_RATE, goal.target, steps.len()));
            if self.walk(&steps) {
                self.interact(&goal);
                if self.used.insert(goal.target) && !matches!(goal.target, Target::Wall(_)) {
                    self.retries = 0;
                }
                self.failures.clear(); // something changed: try the others again
            } else {
                self.unreachable.insert(goal.target);
                *self.failures.entry(goal.target).or_default() += 1;
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
        eprintln!("usage: playbot <GAME_DIR> [LIPSnn ...] [--minutes N] [--verbose]");
        std::process::exit(2);
    };
    let verbose = args.iter().any(|a| a == "--verbose");
    let minutes: u64 = args.iter().position(|a| a == "--minutes").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(30);
    let mut levels: Vec<String> = args[1..].iter().filter(|a| a.starts_with("LIPS")).cloned().collect();
    if levels.is_empty() {
        levels = (0..18).map(|i| format!("LIPS{i:02}")).collect();
    }
    let vfs = Vfs::from_dir(Path::new(dir))?;
    let mut failures = 0;
    for level in &levels {
        let game = Game::new(&vfs, &Options { start_level: Some(level.clone()), save: None })?;
        let mut bot = Bot { vfs: &vfs, game, level: level.clone(), used: HashSet::new(), unreachable: HashSet::new(), failures: HashMap::new(), waits: 0, retries: 0, edges: HashMap::new(), world_key: 0, ticks: 0, verbose, log: Vec::new() };
        let started = std::time::Instant::now();
        let result = bot.play(minutes * 60 * TICK_RATE as u64);
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
