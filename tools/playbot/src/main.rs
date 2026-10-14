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
/// Tags that send the player back to the start (PLAYTRN1's 661 → 3, DEATH01).
const BASE_DEATH_TAGS: [i32; 2] = [3, 661];
/// Most planner states explored per plan.
const MAX_NODES: usize = 150_000;
/// Marker target: a part-way spot on a long region route, not a thing to use.
const WAYPOINT: usize = usize::MAX;
const TICK_RATE: f32 = 60.0;

/// Object effects (re/engine/object-effects.md).
const OWALK: i32 = 5;
const OTELE: i32 = 15;
const OTAGSW: i32 = 20;
const OBUMPSW: i32 = 22;
const OPICKUP: i32 = 130;

/// A search that found nothing: (start cell x, y, z band, region), goals.
type FailedPlan = ((i32, i32, i32, usize), Vec<Target>);

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
/// Grid spot: x, y, height band and region (overlapping regions at one height are different
/// places: a corridor under a walkway that is solid from the room around it).
type Cell = (i32, i32, i32, usize);

/// A move that failed: from cell, to cell (x, y in grid steps), doors key at the time.
type BadMove = ((i32, i32), (i32, i32), u64);

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
    /// Height expected there (None: any), so a spot above or below does not count.
    z: Option<f32>,
    jump: bool,
    /// Crouch the whole way (crawlspaces).
    crouch: bool,
    /// The region the plan expected to be in there (None: any).
    region: Option<usize>,
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
    /// The last old goals chosen (repeating one or two of them: shut in).
    recent_old: Vec<Target>,
    /// Respawns since the last new thing.
    idle_respawns: u32,
    /// Set when a tick moved the player far (a teleport).
    teleported: bool,
    /// When each thing was last used (ticks).
    last_used: HashMap<Target, u64>,
    /// Waypoint walks that failed in a row.
    waypoint_fails: u32,
    /// Portal lines whose sides are not in the regions they join (a stray line off a region's
    /// outline): no way through, for the region routes.
    false_portals: std::cell::OnceCell<HashSet<usize>>,
    /// Tags that kill: the base ones and any firing a teleport back to a start point (LIPS05's
    /// pits fire 75, DETH2 → RBIRTH02).
    death_tags: std::cell::OnceCell<HashSet<i32>>,
    /// Route targets (regions) whose waypoint could not be planned to; skipped until progress.
    route_fails: HashSet<usize>,
    /// Searches that found nothing: (start cell, goals); cleared when the world changes.
    failed_plans: HashSet<FailedPlan>,
    /// Camera-outside spots already reported (PLAYBOT_CAMCHECK).
    cam_reported: HashSet<(i32, i32, usize)>,
    /// The last level's boss (object, look): killing it ends the game.
    boss: Option<(usize, Option<usize>)>,
    /// Regions the last failed plan got to (seeds for the region-level route).
    reached: HashSet<usize>,
    /// Cell-to-cell moves that failed in the real game.
    /// (forgotten whenever something new is used: a door may have changed).
    bad_moves: HashSet<BadMove>,
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
        let Some(tag) = s.objects.list.iter().find(|o| o.effect == OTELE && BASE_DEATH_TAGS.contains(&o.tag)).map(|o| o.tag) else { return false };
        let before = (s.player.x, s.player.y);
        // A fresh start: what failed from elsewhere may work from here.
        self.bad_moves.clear();
        // (From the start, places that could not be reached from where we were may be.)
        self.unreachable.clear();
        self.failed_plans.clear();
        self.route_fails.clear();
        self.game.session_mut().expect("session").fire_tag(tag as u32);
        let p = self.session().player;
        (p.x - before.0).hypot(p.y - before.1) > 64.0
    }

    /// The switches (among `switches`) that open the first shut door on the region route to
    /// the nearest unreached non-switch goal. Doors count as open for routing; steps up to a
    /// jump and any drop are allowed.
    fn door_on_route(&self, goals: &[Goal], switches: &[Goal]) -> Vec<Goal> {
        use std::collections::VecDeque;
        let s = self.session();
        let w = &s.level.world;
        let doors: HashMap<usize, i32> = s.effects.doors().into_iter().collect();
        let shut = |r: usize| w.regions[r].ceiling - w.regions[r].floor < 56.0;
        let span = |r: usize| {
            let reg = &w.regions[r];
            let floor = reg.floor;
            (floor, if doors.contains_key(&r) { reg.ceiling.max(reg.floor + 128.0) } else { reg.ceiling })
        };
        let start = s.player.region;
        let mut prev = vec![usize::MAX; w.regions.len()];
        prev[start] = start;
        let mut queue = VecDeque::from([start]);
        while let Some(r) = queue.pop_front() {
            for &wi in &w.regions[r].walls {
                let x = &w.walls[wi];
                let Some(b) = x.back.filter(|&b| b != x.front) else { continue };
                let o = if x.front == r { b } else { x.front };
                if prev[o] != usize::MAX || x.blocking && !doors.contains_key(&o) && !doors.contains_key(&r) {
                    continue;
                }
                let ((ff, fc), (tf, tc)) = (span(r), span(o));
                if fc.min(tc) - ff.max(tf) >= 20.0 && tf - ff <= 48.0 {
                    prev[o] = r;
                    queue.push_back(o);
                }
            }
        }
        // Nearest (by route length) unreached thing.
        let route_len = |mut r: usize| {
            let mut n = 0;
            while prev[r] != r && n < 10_000 {
                r = prev[r];
                n += 1;
            }
            n
        };
        let target = goals
            .iter()
            .filter(|g| !matches!(g.target, Target::Wall(_)) && !self.used.contains(&g.target))
            .filter_map(|g| w.region_at_z(g.at[0], g.at[1], Some(g.at[2] + 1.0)).filter(|&r| prev[r] != usize::MAX).map(|r| (route_len(r), r)))
            .min();
        if self.verbose {
            println!("     route target: {:?}", target);
        }
        let Some((_, mut r)) = target else { return Vec::new() };
        // Walk back to the first shut door on the route.
        let mut first = None;
        while prev[r] != r {
            if doors.contains_key(&r) && shut(r) {
                first = Some(r);
            }
            r = prev[r];
        }
        let Some(door) = first else { return Vec::new() };
        let tag = doors[&door];
        // The door's tag, or a lock's (an ODROP firing it) while we carry the lock's key.
        let carried: Vec<&str> = s.objects.inventory.iter().map(|&k| s.objects.list[k].name.as_str()).collect();
        let tags: HashSet<i32> = std::iter::once(tag)
            .chain(s.objects.list.iter().filter(|o| {
                o.effect == 131 && o.tag > 0 && o.aux.as_ref().is_some_and(|a| a.ints[0] == tag && a.slot(9).is_some_and(|item| carried.iter().any(|c| c.eq_ignore_ascii_case(item))))
            }).map(|o| o.tag))
            .collect();
        let opens: HashSet<Target> = s
            .effects
            .switches()
            .iter()
            .filter(|sw| tags.iter().any(|t| sw.tags.contains(t)))
            .map(|sw| Target::Wall(sw.wall))
            .chain(s.objects.list.iter().enumerate().filter(|(_, o)| matches!(o.effect, OTAGSW | OBUMPSW) && tags.contains(&o.tag)).map(|(i, _)| Target::Object(i)))
            .collect();
        switches.iter().chain(goals.iter()).filter(|g| opens.contains(&g.target)).copied().collect()
    }

    fn death_tags(&self) -> &HashSet<i32> {
        self.death_tags.get_or_init(|| {
            let s = self.session();
            BASE_DEATH_TAGS
                .iter()
                .copied()
                .chain(s.objects.list.iter().filter(|o| o.effect == OTELE && o.tag > 0 && o.aux.as_ref().and_then(|a| a.slot(9)).is_some_and(|t| t.to_ascii_uppercase().starts_with("RBIRTH"))).map(|o| o.tag))
                .collect()
        })
    }

    /// After pressing switch wall `wi`: if it opens a door that closes on a timer, wait until
    /// it is open enough to pass, then walk to a spot just beyond it (the side away from us).
    fn through_timed_door(&mut self, wi: usize) {
        let (door, beyond) = {
            let s = self.session();
            let w = &s.level.world;
            let Some(sw) = s.effects.switches().into_iter().find(|x| x.wall == wi) else { return };
            let waits: Vec<usize> = s.effects.door_waits().iter().map(|d| d.0).collect();
            let Some(&(door, _)) = s.effects.doors().iter().find(|(d, t)| sw.tags.contains(t) && waits.contains(d)) else { return };
            let p = s.player;
            // The door's portal lines; the far one is the farthest from us.
            let far = w.regions[door].walls.iter().filter_map(|&x| {
                let wall = &w.walls[x];
                let o = if wall.front == door { wall.back? } else { wall.front };
                if o == door {
                    return None;
                }
                let ((ax, ay), (bx, by)) = (w.vertices[wall.a], w.vertices[wall.b]);
                let len = (bx - ax).hypot(by - ay).max(1e-3);
                let mid = ((ax + bx) / 2.0, (ay + by) / 2.0);
                // (48 past the line, or less into a thin strip beyond the door.)
                [48.0f32, 24.0, 8.0].into_iter().flat_map(|dist| [1.0f32, -1.0].map(|side| (mid.0 - (by - ay) / len * dist * side, mid.1 + (bx - ax) / len * dist * side))).find(|q| w.point_in_region(q.0, q.1, o)).map(|q| (q, o))
            }).max_by(|a, b| (a.0.0 - p.x).hypot(a.0.1 - p.y).total_cmp(&(b.0.0 - p.x).hypot(b.0.1 - p.y)));
            let Some((q, o)) = far else { return };
            if o == p.region {
                return;
            }
            (door, [q.0, q.1, w.regions[o].floor])
        };
        let physics = self.session().physics_override.unwrap_or_default();
        let idle = Input::default();
        for _ in 0..(TICK_RATE as usize * 3) {
            let r = &self.session().level.world.regions[door];
            if r.ceiling - r.floor >= physics.height + physics.head_clearance + 1.0 || self.level_done() {
                break;
            }
            self.tick(&idle);
        }
        let goal = Goal { target: Target::Region(WAYPOINT), at: beyond, face: None, shoot: false };
        match self.plan(&[goal]) {
            Some((_, steps)) => {
                self.note(format!("     through the door r{door} before it shuts ({} steps)", steps.len()));
                let _ = self.walk(&steps);
            }
            None => self.note(format!("     no way through the door r{door} to ({:.0}, {:.0})", beyond[0], beyond[1])),
        }
    }

    /// A spot just out of region `r` into a neighbour that borders a door opened by `tag`.
    fn door_side_exit(&self, r: usize, tag: i32) -> Option<(f32, f32)> {
        let s = self.session();
        let w = &s.level.world;
        let doors: Vec<usize> = s.effects.doors().iter().filter(|(_, t)| *t == tag).map(|d| d.0).collect();
        if doors.is_empty() {
            return None;
        }
        let joins = |a: usize, b: usize| w.regions[a].walls.iter().any(|&wi| {
            let x = &w.walls[wi];
            (x.front == a && x.back == Some(b)) || (x.front == b && x.back == Some(a))
        });
        w.regions[r].walls.iter().find_map(|&wi| {
            let x = &w.walls[wi];
            let o = if x.front == r { x.back? } else if x.back == Some(r) { x.front } else { return None };
            if o == r || !doors.iter().any(|&d| d == o || joins(o, d)) {
                return None;
            }
            let ((ax, ay), (bx, by)) = (w.vertices[x.a], w.vertices[x.b]);
            let len = (bx - ax).hypot(by - ay).max(1e-3);
            let mid = ((ax + bx) / 2.0, (ay + by) / 2.0);
            // 40 units past the line, on the side of the neighbour.
            [1.0f32, -1.0].into_iter().map(|side| (mid.0 - (by - ay) / len * 40.0 * side, mid.1 + (bx - ax) / len * 40.0 * side)).find(|p| w.point_in_region(p.0, p.1, o))
        })
    }

    /// Switch walls and objects that fire a lock (ODROP) whose key we carry and that has not
    /// taken it yet.
    fn key_locks(&self) -> HashSet<Target> {
        let s = self.session();
        let carried: Vec<&str> = s.objects.inventory.iter().map(|&k| s.objects.list[k].name.as_str()).collect();
        let tags: HashSet<i32> = s
            .objects
            .list
            .iter()
            .filter(|o| o.effect == 131 && o.tag > 0 && o.aux.as_ref().and_then(|a| a.slot(9)).is_some_and(|item| carried.iter().any(|c| c.eq_ignore_ascii_case(item))))
            .map(|o| o.tag)
            .collect();
        s.effects
            .switches()
            .iter()
            .filter(|sw| tags.iter().any(|t| sw.tags.contains(t)))
            .map(|sw| Target::Wall(sw.wall))
            .chain(s.objects.list.iter().enumerate().filter(|(_, o)| matches!(o.effect, OTAGSW | OBUMPSW) && tags.contains(&o.tag)).map(|(i, _)| Target::Object(i)))
            .collect()
    }

    /// A spot about 24 regions along the region-level route (doors and floors as they are
    /// now; steps up to a jump) to the nearest unreached thing, when that is further.
    fn route_waypoint(&self, goals: &[Goal]) -> Option<(Goal, usize)> {
        use std::collections::VecDeque;
        if self.waypoint_fails >= 3 {
            return None;
        }
        let s = self.session();
        let w = &s.level.world;
        // Regions that kill on entering or on landing (a drop into them) are no way on.
        let triggers = s.effects.triggers();
        let deadly_enter: HashSet<usize> = triggers.iter().filter(|t| self.death_tags().contains(&t.1[0])).map(|t| t.0).collect();
        let deadly_land: HashSet<usize> = triggers.iter().filter(|t| self.death_tags().contains(&t.1[2])).map(|t| t.0).collect();
        let door_regions: HashSet<usize> = s.effects.doors().iter().map(|d| d.0).collect();
        let false_portals = self.false_portals.get_or_init(|| {
            w.walls
                .iter()
                .enumerate()
                .filter(|(_, x)| {
                    // (Doors' lines are real however their regions are drawn.)
                    let Some(b) = x.back.filter(|&b| b != x.front && !door_regions.contains(&b) && !door_regions.contains(&x.front)) else { return false };
                    let ((ax, ay), (bx, by)) = (w.vertices[x.a], w.vertices[x.b]);
                    let len = (bx - ax).hypot(by - ay);
                    if len < 1.0 {
                        return false;
                    }
                    let (mx, my, nx, ny) = ((ax + bx) / 2.0, (ay + by) / 2.0, -(by - ay) / len * 2.0, (bx - ax) / len * 2.0);
                    let inside = |x: f32, y: f32, r: usize| w.point_in_region(x, y, r);
                    let (p, q) = ((mx + nx, my + ny), (mx - nx, my - ny));
                    !((inside(p.0, p.1, x.front) && inside(q.0, q.1, b)) || (inside(p.0, p.1, b) && inside(q.0, q.1, x.front)))
                })
                .map(|(i, _)| i)
                .collect()
        });
        // From everywhere the grid planner could get to (it knows jumps across gaps).
        let mut prev = vec![usize::MAX; w.regions.len()];
        let mut queue = VecDeque::new();
        for &r in self.reached.iter().chain(std::iter::once(&s.player.region)) {
            if r < prev.len() && prev[r] == usize::MAX {
                prev[r] = r;
                queue.push_back(r);
            }
        }
        // Shut doors and the regions their openers sit in (switch walls, trigger regions,
        // switch objects, pickups firing the tag); None when some opener is of another kind
        // (a lock, a timer): then the door counts as passable.
        let openers: HashMap<usize, Option<Vec<usize>>> = s
            .effects
            .doors()
            .iter()
            .filter(|(d, _)| w.regions[*d].ceiling - w.regions[*d].floor < 20.0)
            .map(|&(d, tag)| {
                let mut regions: Vec<usize> = Vec::new();
                for sw in s.effects.switches().iter().filter(|sw| sw.tags.contains(&tag)) {
                    let wall = &w.walls[sw.wall];
                    regions.push(wall.front);
                    regions.extend(wall.back);
                }
                regions.extend(s.effects.triggers().iter().filter(|(_, t)| t.contains(&tag)).map(|(r, _)| *r));
                let mut other = false;
                for o in s.objects.list.iter() {
                    let fires = o.tag == tag && matches!(o.effect, OTAGSW | OBUMPSW) || o.effect == OPICKUP && o.aux.as_ref().is_some_and(|a| a.ints[0] == tag);
                    if fires {
                        match o.region {
                            Some(r) => regions.push(r),
                            None => other = true,
                        }
                    } else if o.effect == 131 && o.aux.as_ref().is_some_and(|a| a.ints[0] == tag) {
                        other = true;
                    }
                }
                (d, (!other && !regions.is_empty()).then_some(regions))
            })
            .collect();
        while let Some(r) = queue.pop_front() {
            for &wi in &w.regions[r].walls {
                let x = &w.walls[wi];
                // (Lines flagged solid block, as they are now: wall copies change them; door
                // lines carry the flag too.)
                let solid = |y: &godd_game::world::Wall| y.blocking && !door_regions.contains(&y.front) && !y.back.is_some_and(|b| door_regions.contains(&b));
                let Some(b) = x.back.filter(|&b| b != x.front && !solid(x) && !false_portals.contains(&wi)) else { continue };
                let o = if x.front == r { b } else { x.front };
                if prev[o] != usize::MAX || deadly_enter.contains(&o) {
                    continue;
                }
                // Not into a shut door none of whose openers we can get to (yet): a door that
                // only opens from its other side.
                if let Some(Some(list)) = openers.get(&o)
                    && !list.iter().any(|&q| q < prev.len() && prev[q] != usize::MAX)
                {
                    continue;
                }
                // Not through an opening narrower than the body (the bars of a grate): all the
                // portal lines between the two regions together.
                let opening: f32 = w.regions[r]
                    .walls
                    .iter()
                    .map(|&wj| &w.walls[wj])
                    .filter(|y| !solid(y) && ((y.front == r && y.back == Some(o)) || (y.front == o && y.back == Some(r))))
                    .map(|y| {
                        let (pa, pb) = (w.vertices[y.a], w.vertices[y.b]);
                        (pa.0 - pb.0).hypot(pa.1 - pb.1)
                    })
                    .sum();
                if opening < 32.0 {
                    continue;
                }
                // Drawn floors (a narrow pit with a see-through floor can be run across; the
                // grid planner works out whether it can), but no waypoint in such a pit.
                let (a, c) = (&w.regions[r], &w.regions[o]);
                let (af, cf) = (a.floor, c.floor);
                if a.ceiling.min(c.ceiling) - af.max(cf) >= 20.0 && cf - af <= 48.0 {
                    prev[o] = r;
                    queue.push_back(o);
                }
            }
        }
        if let Ok(list) = std::env::var("PLAYBOT_ROUTECHK") {
            let got: Vec<String> = list.split(',').filter_map(|x| x.parse::<usize>().ok()).map(|r| format!("r{r}{}", if prev.get(r).is_some_and(|&p| p != usize::MAX) { "+" } else { "-" })).collect();
            println!("     route reach {}", got.join(" "));
        }
        let len = |mut r: usize| {
            let mut n = 0;
            while prev[r] != r && n < 100_000 {
                r = prev[r];
                n += 1;
            }
            n
        };
        let (n, target) = goals
            .iter()
            .filter(|g| !self.used.contains(&g.target))
            // (An object's own region: the lookup may pick a level stacked above it.)
            .filter_map(|g| {
                let own = if let Target::Object(i) = g.target { s.objects.list.get(i).and_then(|o| o.region) } else { None };
                own.or_else(|| w.region_at_z(g.at[0], g.at[1], Some(g.at[2] + 1.0))).filter(|&r| prev[r] != usize::MAX && !self.route_fails.contains(&r)).map(|r| (len(r), r))
            })
            .min()?;
        let _ = n;
        // Walk back from the target to the region `n - 24` steps from it... i.e. 24 from us.
        let mut path = vec![target];
        while let Some(&r) = path.last() {
            if prev[r] == r {
                break;
            }
            path.push(prev[r]);
        }
        path.reverse();
        // A region a few steps along from where the planner got to (the route starts there),
        // the furthest whose middle lies in it and is not the one we stand in.
        let here = s.player.region;
        // (The route's first region is one the planner reached: certainly within its budget.)
        (0..=2.min(path.len() - 1)).filter(|&k| path[k] != here).find_map(|k| {
            let r = path[k];
            let reg = &w.regions[r];
            let ((x0, y0), (x1, y1)) = reg.bounds;
            let c = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            // (Not in a region where landing kills: the walk there may end in a drop.)
            (reg.below.is_none() && !deadly_land.contains(&r) && w.region_at_z(c.0, c.1, Some(reg.floor + 1.0)) == Some(r)).then_some((Goal { target: Target::Region(WAYPOINT), at: [c.0, c.1, reg.floor], face: None, shoot: false }, target))
        })
    }

    /// Trigger regions whose tags move a floor or ceiling (elevators), used like switches.
    fn mover_regions(&self) -> HashSet<usize> {
        let s = self.session();
        let mover_tags: HashSet<i32> = s.objects.list.iter().filter(|o| matches!(o.effect, 41 | 49) && o.tag > 0).map(|o| o.tag).collect();
        s.effects.triggers().into_iter().filter(|(_, tags)| tags.iter().any(|x| mover_tags.contains(x))).map(|(r, _)| r).collect()
    }

    /// A planned move that the real game would not do: never plan it again (the planner's
    /// simulation leaves out objects and the exact spot it walks from).
    fn mark_bad(&mut self, steps: &[Step], k: usize) {
        let cell = |p: (f32, f32)| ((p.0 / STEP).round() as i32, (p.1 / STEP).round() as i32);
        let to = cell(steps[k].to);
        let from = if k == 0 { let p = self.session().player; cell((p.x, p.y)) } else { cell(steps[k - 1].to) };
        self.bad_moves.insert((from, to, 0));
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
        let locks = self.key_locks();
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
                // (A lock pressed before we had its key is worth pressing again now.)
                None if self.used.contains(&Target::Wall(sw.wall)) && !locks.contains(&Target::Wall(sw.wall)) => continue,
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
            let spots: Vec<((f32, f32), usize)> = [1.0f32, -1.0]
                .into_iter()
                .map(|side| {
                    let n = (-dy / len * side, dx / len * side);
                    ((mid.0 + n.0 * 40.0, mid.1 + n.1 * 40.0), side)
                })
                // At the switch's own height (another level may lie above or below). A region
                // with an open outline is not found by the lookup: the spot on the wall's right
                // (its front side) then lies in its front region.
                // (A one-sided wall is only faced from its front: behind it lies no level.)
                .filter(|&(_, side)| wall.back.is_some() || side < 0.0)
                .filter_map(|(at, side)| {
                    let ((x0, y0), (x1, y1)) = w.regions[wall.front].bounds;
                    let front = (side < 0.0 && at.0 >= x0 && at.0 <= x1 && at.1 >= y0 && at.1 <= y1).then_some(wall.front);
                    w.region_at_z(at.0, at.1, Some(w.regions[wall.front].floor + 1.0)).or(front).filter(|&r| open(r)).map(|r| (at, r))
                })
                // Nothing solid (a wall, a shut door) between the spot and the switch.
                .filter(|&(at, _)| {
                    let to = (mid.0 - (mid.0 - at.0) * 0.02, mid.1 - (mid.1 - at.1) * 0.02);
                    !w.walls.iter().enumerate().any(|(wi, x)| {
                        if wi == sw.wall {
                            return false;
                        }
                        // Only walls at this height (not a level above or below).
                        let zf = w.regions[wall.front].floor;
                        let at_height = |r: usize| w.regions.get(r).is_some_and(|g| g.floor < zf + 48.0 && g.ceiling > zf);
                        if !at_height(x.front) && !x.back.is_some_and(at_height) {
                            return false;
                        }
                        // One-sided, or next to somewhere too low to stand (a shut door). (The
                        // "blocking" flag marks door lines too, so it says nothing here.)
                        let solid = x.back.is_none() || [Some(x.front), x.back].into_iter().flatten().any(|r| !open(r));
                        solid && segments_cross(at, to, w.vertices[x.a], w.vertices[x.b])
                    })
                })
                .collect();
            // A two-sided line (a door face) works from either side: offer both and let the
            // planner take the one it can reach. A one-sided switch: the wall's own region,
            // else one opening onto it (a button alcove's room).
            let two_sided = wall.back.is_some_and(|b| b != wall.front);
            let chosen: Vec<((f32, f32), usize)> = if two_sided {
                spots
            } else {
                spots.into_iter().min_by_key(|&(_, r)| if r == wall.front { 0 } else if joined(r, wall.front) { 1 } else { 2 }).into_iter().collect()
            };
            for (at, r) in chosen {
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
                // A lock placed in the world (ODROP fires on contact too): walk into it with
                // its key.
                131 => o.aux.as_ref().and_then(|a| a.slot(9)).is_some_and(|item| s.objects.inventory.iter().any(|&k| s.objects.list[k].name.eq_ignore_ascii_case(item))),
                OTELE => o.name.is_empty() && o.aux.is_some(),
                // (Tag switches out of reach overhead are shot: projectiles hit them.)
                OTAGSW => o.region.is_some_and(|r| o.z - w.regions[r].floor < 72.0),
                OBUMPSW => true,
                // Plain walking bots fire their tag when bumped (or shot, if out of reach).
                OWALK => o.mover.is_none() && o.tag > 0 && o.region.is_some_and(|r| o.z - w.regions[r].floor < 40.0),
                _ => false,
            };
            if walk_into {
                goals.push(Goal { target: Target::Object(i), at: [o.x, o.y, o.z], face: None, shoot: false });
            } else if o.effect == OTAGSW
                || (o.name.starts_with("KEYBOT") && o.mover.is_some()) || (o.effect == OWALK && o.mover.is_none() && o.tag > 0) || self.boss.is_some_and(|(b, _)| b == i) {
                goals.push(Goal { target: Target::Object(i), at: [o.x, o.y, o.z], face: None, shoot: true });
            }
        }
        // Traps: tags that move the floor of a region where landing kills (LIPS00's FALLZ sinks
        // when you step in front of it; walking straight on rides it down, stepping in and back
        // out first leaves a deadly drop).
        let names = s.level.region_names();
        let triggers = s.effects.triggers();
        let deadly_land: HashSet<usize> = triggers.iter().filter(|t| self.death_tags().contains(&t.1[2])).map(|t| t.0).collect();
        let trap_tags: HashSet<i32> = s
            .objects
            .list
            .iter()
            .filter(|o| o.effect == 41)
            .filter(|o| o.aux.as_ref().and_then(|a| a.slot(9)).and_then(|n| names.iter().position(|rn| rn.eq_ignore_ascii_case(n))).is_some_and(|r| deadly_land.contains(&r)))
            .map(|o| o.tag)
            .collect();
        for &(r, tags) in &triggers {
            if tags.iter().all(|t| *t <= 0 || self.death_tags().contains(t) || trap_tags.contains(t)) {
                continue;
            }
            let ((x0, y0), (x1, y1)) = w.regions[r].bounds;
            let c = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            // (A trigger only on the ceiling, a water surface: up there.)
            let ceiling_only = tags[3] > 0 && tags[..3].iter().all(|t| *t <= 0);
            let z = if ceiling_only { w.regions[r].ceiling - 64.0 } else { w.regions[r].floor };
            if w.region_at(c.0, c.1) == Some(r) {
                goals.push(Goal { target: Target::Region(r), at: [c.0, c.1, z], face: None, shoot: false });
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
        // And a lock (a trigger or switch firing an ODROP) once we carry its key.
        let carried: Vec<&str> = s.objects.inventory.iter().map(|&k| s.objects.list[k].name.as_str()).collect();
        let key_tags: HashSet<i32> = s
            .objects
            .list
            .iter()
            .filter(|o| o.effect == 131 && o.tag > 0 && o.aux.as_ref().and_then(|a| a.slot(9)).is_some_and(|item| carried.iter().any(|c| c.eq_ignore_ascii_case(item))))
            .map(|o| o.tag)
            .collect();
        let lock = |r: usize| triggers.iter().any(|(t, tags)| *t == r && tags.iter().any(|x| key_tags.contains(x)));
        // Likewise bump/shoot switch objects (they toggle their door).
        let reswitch = |i: usize| s.objects.list.get(i).is_some_and(|o| matches!(o.effect, OTAGSW | OBUMPSW) && shut_tags.contains(&o.tag));
        // Trigger regions that move a floor or ceiling (an elevator's): like a switch, always
        // usable again.
        // (Only movers not yet where they would go: a raised bridge is done.)
        let mover_tags: HashSet<i32> = s
            .objects
            .list
            .iter()
            .filter(|o| matches!(o.effect, 41 | 49) && o.tag > 0)
            .filter(|o| {
                let Some(a) = o.aux.as_ref() else { return true };
                let Some(r) = a.slot(9).and_then(|n| names.iter().position(|rn| rn.eq_ignore_ascii_case(n))) else { return true };
                let reg = &w.regions[r];
                let at = if o.effect == 41 { reg.floor } else { reg.ceiling };
                (at - a.ints[0] as f32).abs() > 1.0
            })
            .map(|o| o.tag)
            .collect();
        let moves = |r: usize| triggers.iter().any(|(t, tags)| *t == r && tags.iter().any(|x| mover_tags.contains(x)));
        // And trigger regions that teleport (not back to a start): a way on, again and again.
        let tele_tags: HashSet<i32> = s.objects.list.iter().filter(|o| o.effect == OTELE && o.tag > 0 && !self.death_tags().contains(&o.tag)).map(|o| o.tag).collect();
        let teleports = |r: usize| triggers.iter().any(|(t, tags)| *t == r && tags.iter().any(|x| tele_tags.contains(x)));
        goals.retain(|g| (matches!(g.target, Target::Wall(_)) || matches!(g.target, Target::Region(r) if reopens(r) || moves(r) || lock(r) || teleports(r)) || matches!(g.target, Target::Object(i) if reswitch(i) || s.objects.list.get(i).is_some_and(|o| o.effect == OTELE || (matches!(o.effect, OTAGSW | OBUMPSW) && key_tags.contains(&o.tag)))) || !self.used.contains(&g.target)) && !self.unreachable.contains(&g.target) && self.failures.get(&g.target).is_none_or(|&n| n < 3));
        goals
    }

    /// Simulate a player walking (running, optionally jumping) from `p` toward `to`; the state
    /// where it arrives, if it does.
    fn try_move(world: &godd_game::world::World, physics: &PhysicsParams, pads: &[Option<f32>], mut p: Player, to: (f32, f32), jump: bool, crouch: bool) -> Option<Player> {
        // Jump pads relaunch a body on their floor that is not rising, plus the jump speed when
        // Jump is held at contact (as the session does after each tick).
        let pad = |p: &mut Player, jump: bool| {
            let floor = world.solid_floor(p.region).1;
            if p.on_ground && p.vz <= 0.0 && !p.flying
                && let Some(vz) = pads.get(floor).copied().flatten()
            {
                p.vz = vz + if jump { physics.jump_speed } else { 0.0 };
                p.on_ground = false;
                // (Lifted off the floor, as the session does: a Jump next tick keeps the speed.)
                p.z += 0.05;
            }
        };
        // A move that starts on a pad's contact starts with its launch (that tick's bounce).
        let on_pad = pads.get(world.solid_floor(p.region).1).copied().flatten().is_some();
        pad(&mut p, jump);
        // (A strong pad keeps us up for seconds: LIPS05's shaft of 600 pads.)
        let ticks = if on_pad { 360 } else { 72 };
        for t in 0..ticks {
            let before = (p.x - to.0).hypot(p.y - to.1);
            // Run while far, walk the last stretch; arrived once the spot is within one step.
            let mut i = Input::default();
            i.set(Action::Forward, true);
            // Walk the short hops (a run overshoots small ledges), run the long jumps.
            i.set(Action::Run, before > 48.0 && (!jump || before > 100.0));
            i.set(Action::Jump, jump && t == 0);
            // Stay down while there is no room to stand.
            // (Or heading into a gap too low to stand in: crouch before it, not in it.)
            let low_at = |r: Option<usize>| r.and_then(|r| world.regions.get(r)).is_some_and(|r| r.ceiling - r.floor < physics.height + physics.head_clearance);
            let low = low_at(Some(p.region)) || low_at(world.region_at_z(to.0, to.1, Some(p.z + 1.0)));
            i.set(Action::Crouch, crouch || low);
            p.angle = facing((p.x, p.y), to);
            let (x0, y0) = (p.x, p.y);
            p.tick(world, &i, physics);
            let moved = (p.x - x0).hypot(p.y - y0);
            let after = (p.x - to.0).hypot(p.y - to.1);
            let arrived = after < 6.0 || (before <= moved + 1.0 && moved > 0.5);
            // (Touching a pad where we were going counts as there: the next move starts from
            // the contact, bouncing.)
            if !(arrived && p.on_ground) {
                pad(&mut p, i.held(Action::Jump));
            }
            // Arrived: on the spot, about to pass it, or held up just short of it.
            if arrived || (after < 16.0 && moved < 0.5 && (p.on_ground || p.flying)) {
                // Flying (swimming): no ground to come to rest on.
                if p.flying {
                    return Some(p);
                }
                // Settle: come to rest on the ground.
                let idle = Input::default();
                for _ in 0..300 {
                    if p.on_ground {
                        break;
                    }
                    p.tick(world, &idle, physics);
                }
                // (On a pad, that contact is where we stand.)
                return p.on_ground.then_some(p);
            }
        }
        None
    }

    /// A* over standing spots from the player to the nearest goal. Each step is a simulated
    /// walk (or jump) to a neighbouring grid spot; results are cached while the regions at
    /// both ends keep their heights.
    fn plan(&mut self, goals: &[Goal]) -> Option<(Goal, Vec<Step>)> {
        // A search that failed from this spot for these goals fails again until something
        // changes (the world, or where we are): skip it.
        let p = self.session().player;
        let mut targets: Vec<Target> = goals.iter().map(|g| g.target).collect();
        targets.sort_by_key(|t| format!("{t:?}"));
        let memo = (((p.x / STEP).round() as i32, (p.y / STEP).round() as i32, (p.z / 24.0).round() as i32, p.region), targets);
        if self.failed_plans.contains(&memo) {
            return None;
        }
        let started = std::time::Instant::now();
        let r = self.plan_search(goals);
        if r.is_none() {
            self.failed_plans.insert(memo);
        }
        let took = started.elapsed().as_secs_f32();
        if self.verbose && took > 5.0 {
            println!("     slow plan: {took:.0} s for {} goals ({:?}...) -> {:?}", goals.len(), goals.iter().take(3).map(|g| g.target).collect::<Vec<_>>(), r.as_ref().map(|r| r.1.len()));
        }
        r
    }

    fn plan_search(&mut self, goals: &[Goal]) -> Option<(Goal, Vec<Step>)> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let s = self.game.session().expect("session");
        let world = &s.level.world;
        let physics = s.physics_override.unwrap_or_default();
        let start = s.player;
        let key = |p: &Player| ((p.x / STEP).round() as i32, (p.y / STEP).round() as i32, (p.z / 24.0).round() as i32, p.region);
        // Lifts move: reaching their floor area at any height counts.
        let near_goal = |p: &Player| {
            goals
                .iter()
                .find(|g| {
                    let d = (p.x - g.at[0]).hypot(p.y - g.at[1]);
                    if g.shoot {
                        // In range, not right underneath (aim is limited to about 60 degrees).
                        let dz = g.at[2] - p.z;
                        d < 320.0 && d > 32.0 && dz.abs() < d * 1.7 + 32.0
                    } else {
                        // (A lock set in a doorway is touched from the wall's distance.)
                        let reach = if matches!(g.target, Target::Object(i) if s.objects.list.get(i).is_some_and(|o| o.effect == 131)) { 40.0 } else { STEP * 0.75 };
                        // (A pickup hanging up to a jump above us counts: the approach jumps for it.)
                        let pickup_above = matches!(g.target, Target::Object(i) if s.objects.list.get(i).is_some_and(|o| o.effect == OPICKUP)) && g.at[2] > p.z && g.at[2] - p.z < 96.0;
                        d < reach && (matches!(g.target, Target::Lift(r) if p.region == r) || (p.z - g.at[2]).abs() < 72.0 || pickup_above)
                    }
                })
                .copied()
        };
        let heuristic = |p: &Player| goals.iter().map(|g| (p.x - g.at[0]).hypot(p.y - g.at[1])).fold(f32::MAX, f32::min) / STEP;
        let heights = |r: usize| world.regions.get(r).map_or((0, 0), |reg| (reg.floor.to_bits(), reg.ceiling.to_bits()));
        let doors = self.doors_key();
        let pads: Vec<Option<f32>> = (0..world.regions.len()).map(|r| s.effects.bounce_speed(r).filter(|&v| v > 0.0)).collect();
        let triggers = s.effects.triggers();
        let deadly_enter: HashSet<usize> = triggers.iter().filter(|t| self.death_tags().contains(&t.1[0])).map(|t| t.0).collect();
        let deadly_land: HashSet<usize> = triggers.iter().filter(|t| self.death_tags().contains(&t.1[2])).map(|t| t.0).collect();
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
                if !goal.shoot {
                    let crouch = steps.last().is_some_and(|s: &Step| s.crouch);
                    // (Swimming: up or down to the thing's own height.)
                    let z = came.get(&k).is_some_and(|e| e.0.flying).then_some(goal.at[2]);
                    steps.push(Step { to: (goal.at[0], goal.at[1]), z, crouch, jump: false, region: None });
                }
                return Some((goal, steps));
            }
            expanded += 1;
            if expanded > MAX_NODES {
                self.reached = came.keys().map(|c| c.3).collect();
                return None;
            }
            const MOVES: [(f32, f32); 40] = [
                (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0),
                // Jumps with a run-up onto ledges too high to jump from right next to them.
                (2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0), (2.0, 2.0), (2.0, -2.0), (-2.0, 2.0), (-2.0, -2.0),
                (3.0, 0.0), (-3.0, 0.0), (0.0, 3.0), (0.0, -3.0),
                // Across wider gaps (only tried over a drop, see below).
                (4.0, 0.0), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0),
                (5.0, 0.0), (-5.0, 0.0), (0.0, 5.0), (0.0, -5.0),
                (6.0, 0.0), (-6.0, 0.0), (0.0, 6.0), (0.0, -6.0),
                (7.0, 0.0), (-7.0, 0.0), (0.0, 7.0), (0.0, -7.0),
                (8.0, 0.0), (-8.0, 0.0), (0.0, 8.0), (0.0, -8.0),
            ];
            // Straight one-cell moves that worked from here (diagonals need both beside them:
            // no cutting through a corner where two rooms only touch at a point).
            let mut straight_ok: HashSet<(i32, i32)> = HashSet::new();
            // Long running jumps across a pit (LIPS05's death-pit field: 640 to the pad r520).
            const LONG: [(f32, f32); 32] = [
                (10.0, 0.0), (-10.0, 0.0), (0.0, 10.0), (0.0, -10.0),
                (12.0, 0.0), (-12.0, 0.0), (0.0, 12.0), (0.0, -12.0),
                (14.0, 0.0), (-14.0, 0.0), (0.0, 14.0), (0.0, -14.0),
                (16.0, 0.0), (-16.0, 0.0), (0.0, 16.0), (0.0, -16.0),
                (18.0, 0.0), (-18.0, 0.0), (0.0, 18.0), (0.0, -18.0),
                (20.0, 0.0), (-20.0, 0.0), (0.0, 20.0), (0.0, -20.0),
                (22.0, 0.0), (-22.0, 0.0), (0.0, 22.0), (0.0, -22.0),
                (24.0, 0.0), (-24.0, 0.0), (0.0, 24.0), (0.0, -24.0),
            ];
            for (dx, dy) in MOVES.into_iter().chain(LONG) {
                if dx != 0.0 && dy != 0.0 && dx.abs() == 1.0 && dy.abs() == 1.0 && !(straight_ok.contains(&(dx as i32, 0)) && straight_ok.contains(&(0, dy as i32))) {
                    continue;
                }
                let to = (((p.x / STEP).round() + dx) * STEP, ((p.y / STEP).round() + dy) * STEP);
                let long = dx.abs() > 1.0 || dy.abs() > 1.0;
                if long {
                    // Only onto a ledge too high to step onto, or across a gap to ground about
                    // level with us (an elevator top to its landing).
                    let floor_at = |x: f32, y: f32| world.region_at_z(x, y, Some(p.z + 1.0)).or_else(|| world.region_at(x, y)).map(|r| world.solid_floor(r).0);
                    let mid = ((p.x + to.0) / 2.0, (p.y + to.1) / 2.0);
                    let target = floor_at(to.0, to.1);
                    let ledge = target.is_some_and(|f| f > p.z + physics.step_height);
                    let gap = target.is_some_and(|f| (f - p.z).abs() <= physics.step_height) && floor_at(mid.0, mid.1).is_some_and(|f| f < p.z - physics.step_height);
                    let wide = dx.abs() > 3.0 || dy.abs() > 3.0;
                    // (The longest: also down onto ground somewhat lower, over a pit.)
                    let far = dx.abs() > 8.0 || dy.abs() > 8.0;
                    let pit_jump = far && target.is_some_and(|f| f <= p.z + physics.step_height && f >= p.z - 160.0) && floor_at(mid.0, mid.1).is_some_and(|f| f < p.z - 160.0);
                    // From a jump pad: bounced high, onto ground well above, further off too.
                    let on_pad = pads.get(world.solid_floor(p.region).1).copied().flatten().is_some();
                    // (Not too far: where a bounce touches down drifts, so launch from the pad's
                    // edge nearest the goal, as a player would.)
                    let pad_hop = on_pad && dx.abs().max(dy.abs()) <= 10.0 && target.is_some_and(|f| f > p.z - 160.0);
                    if far && !pit_jump && !pad_hop {
                        continue;
                    }
                    if !far && !pad_hop && ((!ledge && !gap) || (wide && !gap)) {
                        continue;
                    }
                }
                let edge = (k, (dx as i32, dy as i32));
                let cached = self.edges.get(&edge).filter(|e| e.doors == doors && heights(e.from_region) == e.from_heights && e.to.is_none_or(|(q, _)| heights(q.region) == e.to_heights)).map(|e| e.to);
                let result = match cached {
                    Some(r) => r,
                    None => {
                        // (A crouched jump gets into a low duct up a ledge.)
                        let kinds: &[(bool, bool)] = if long { &[(true, false), (true, true)] } else { &[(false, false), (true, false), (false, true), (true, true)] };
                        let r = kinds.iter().copied().find_map(|(jump, crouch)| Self::try_move(world, &physics, &pads, p, to, jump, crouch).map(|q| (q, (jump, crouch))));
                        self.edges.insert(edge, Edge { doors, from_region: p.region, from_heights: heights(p.region), to: r, to_heights: r.map_or((0, 0), |(q, _)| heights(q.region)) });
                        r
                    }
                };
                let Some((q, (jump, crouch))) = result else { continue };
                let cell_of = |x: f32, y: f32| ((x / STEP).round() as i32, (y / STEP).round() as i32);
                if self.bad_moves.contains(&(cell_of(p.x, p.y), cell_of(q.x, q.y), 0)) {
                    continue;
                }
                if (dx == 0.0 || dy == 0.0) && dx.abs() + dy.abs() == 1.0 {
                    straight_ok.insert((dx as i32, dy as i32));
                }
                // Never into a region that kills on entry (tag 3 sends you back to the start),
                // nor jump where landing does.
                // (Landing kills: no jump into it, and no drop of more than a step either.)
                let falls_in = deadly_land.contains(&q.region) && (jump || p.z - world.regions[q.region].floor > physics.step_height);
                if deadly_enter.contains(&q.region) || falls_in {
                    continue;
                }
                if avoid.iter().any(|&(x, y, z, r)| (q.x - x).abs() < r && (q.y - y).abs() < r && (q.z - z).abs() < 64.0) {
                    continue;
                }
                let qk = key(&q);
                // Corners cut off a deadly-landing region (FALLZ's chamfers) belong to its
                // container: entering the trap from there comes too late to ride its floor down.
                // (Only for a trap floor level with us, one that sinks under you; not a pit.)
                let corner = deadly_land.iter().any(|&r| {
                    let reg = &world.regions[r];
                    let ((x0, y0), (x1, y1)) = reg.bounds;
                    q.region != r && (reg.floor - q.z).abs() < 1.0 && q.x > x0 - 8.0 && q.x < x1 + 8.0 && q.y > y0 - 8.0 && q.y < y1 + 8.0
                });
                // Spots on the brink of a deadly drop: the walk drifts a few units off the plan.
                let brink = [(14.0f32, 0.0f32), (-14.0, 0.0), (0.0, 14.0), (0.0, -14.0), (10.0, 10.0), (10.0, -10.0), (-10.0, 10.0), (-10.0, -10.0)].iter().any(|&(ox, oy)| {
                    world.region_at_z(q.x + ox, q.y + oy, Some(q.z + 1.0)).is_some_and(|r| r != q.region && deadly_land.contains(&r) && q.z - world.regions[r].floor > physics.step_height)
                });
                let c = cost + dx.hypot(dy) + if corner { 8.0 } else { 0.0 } + if brink { 24.0 } else { 0.0 };
                if came.get(&qk).is_none_or(|e| c < e.1) {
                    came.insert(qk, (q, c, Some((k, Step { to: (q.x, q.y), z: Some(q.z), jump, crouch, region: Some(q.region) }))));
                    open.push((Reverse(((c + 1.5 * heuristic(&q)) * 1000.0) as i64), qk));
                }
            }
            // Flying (swimming): also straight up (Jump held) or down (Crouch held), a band of
            // 48 at a time.
            if p.flying {
                for up in [true, false] {
                    let mut q = p;
                    let mut i = Input::default();
                    i.set(if up { Action::Jump } else { Action::Crouch }, true);
                    for _ in 0..90 {
                        let z0 = q.z;
                        q.tick(world, &i, &physics);
                        if (q.z - p.z).abs() >= 48.0 || (q.z - z0).abs() < 0.01 {
                            break;
                        }
                    }
                    if (q.z - p.z).abs() < 12.0 || !q.flying {
                        continue;
                    }
                    let qk = key(&q);
                    let c = cost + 1.5;
                    if came.get(&qk).is_none_or(|e| c < e.1) {
                        came.insert(qk, (q, c, Some((k, Step { to: (q.x, q.y), z: Some(q.z), jump: up, crouch: !up, region: Some(q.region) }))));
                        open.push((Reverse(((c + 1.5 * heuristic(&q)) * 1000.0) as i64), qk));
                    }
                }
            }
        }
        self.reached = came.keys().map(|c| c.3).collect();
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
            // The camera must stay inside the level: the eye under its region's ceiling and
            // over its floor, and the spot inside the region (else the view sees out).
            if std::env::var("PLAYBOT_CAMCHECK").is_ok() && !s.in_bsp {
                let p = &s.player;
                let w = &s.level.world;
                let physics = s.physics_override.unwrap_or_default();
                let eye = p.eye_z(&physics);
                let (ceiling, _) = w.solid_ceiling(p.region);
                let (floor, _) = w.solid_floor(p.region);
                let inside = w.point_in_region(p.x, p.y, p.region);
                if eye > ceiling - 0.5 || eye < floor || !inside {
                    let key = ((p.x / 64.0) as i32, (p.y / 64.0) as i32, p.region);
                    if self.cam_reported.insert(key) {
                        println!("CAMOUT t={:.1}s ({:.0},{:.0}) eye {:.0} r{} floor {} ceil {} inside {} crouch {}", self.ticks as f32 / TICK_RATE, p.x, p.y, eye, p.region, floor, ceiling, inside, p.crouching);
                    }
                }
            }
            // PLAYBOT_TRACE=from-to: every tick in that range (PLAYBOT_TRACE0: the first 600).
            let range = std::env::var("PLAYBOT_TRACE").ok().and_then(|v| v.split_once('-').and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?))))
                .or_else(|| std::env::var("PLAYBOT_TRACE0").is_ok().then_some((0, 600)));
            if range.is_some_and(|(a, b)| (a..b).contains(&self.ticks)) {
                println!("       trace t{} r{} ({:.0},{:.0},{:.0}) vz {:.0} ground {} jump {} fwd {}", self.ticks, s.player.region, s.player.x, s.player.y, s.player.z, s.player.vz, s.player.on_ground, input.held(Action::Jump), input.held(Action::Forward));
            }
        }
        if let (Some(a), Some(b)) = (before, self.game.session().map(|s| s.player))
            && (a.x - b.x).hypot(a.y - b.y) > 200.0
        {
            self.teleported = true;
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
        let Some(s) = self.game.session() else { return true };
        if !s.name.eq_ignore_ascii_case(&self.level) {
            return true;
        }
        // The last level has no exit: it is done once its boss is killed (it turns into its
        // dead version, XN).
        self.boss.is_some_and(|(i, tex)| s.objects.list.get(i).is_none_or(|o| o.destroyed || o.tex != tex))
    }

    /// In a level without an exit (the last one): its boss (DETHBOT9) and its look.
    fn find_boss(&self) -> Option<(usize, Option<usize>)> {
        let s = self.session();
        if s.objects.list.iter().any(|o| o.effect == 29) {
            return None;
        }
        s.objects.list.iter().position(|o| o.name.eq_ignore_ascii_case("DETHBOT9") && o.mover.is_some()).map(|i| (i, s.objects.list[i].tex))
    }

    /// Walk the planned steps in the real game; false if the player gets stuck.
    fn walk(&mut self, steps: &[Step]) -> bool {
        self.walk_to(steps, 12.0, None)
    }

    /// `walk` with how close the last step must get.
    /// `inside`: arrived as soon as the player is in this region (trigger regions: stepping
    /// in is the point, and an elevator floor starts moving at once).
    fn walk_to(&mut self, steps: &[Step], close_last: f32, inside: Option<usize>) -> bool {
        let last = steps.len().saturating_sub(1);
        for (k, step) in steps.iter().enumerate() {
            // Intermediate spots only need passing near; the goal itself a little closer; and
            // right before a change of level (a drop or a jump planned from this very spot)
            // close enough to start it from where the planner did.
            // (Or into another region: membership follows the lines actually crossed, so a
            // corner cut short leaves us in another region than the plan, an open outline's.)
            let next_level_change = steps.get(k + 1).is_some_and(|n| n.jump || matches!((step.z, n.z), (Some(a), Some(b)) if (a - b).abs() > 24.0) || matches!((step.region, n.region), (Some(a), Some(b)) if a != b));
            let close = if k == last { close_last } else if next_level_change { 5.5 } else { 16.0 };
            let mut best = f32::MAX;
            let mut stalled = 0;
            let mut launched = false;
            // A jump needs ground under us: come down first (off a stair, a hop).
            if step.jump {
                let idle = Input::default();
                for _ in 0..120 {
                    let s = self.session();
                    let p = s.player;
                    let pad = s.effects.bounce_speed(s.level.world.solid_floor(p.region).1).is_some_and(|v| v > 0.0);
                    if p.on_ground || p.flying || pad || self.teleported || self.level_done() {
                        break;
                    }
                    self.tick(&idle);
                }
            }
            loop {
                if self.level_done() {
                    return true;
                }
                if self.teleported {
                    return false;
                }
                if inside.is_some_and(|r| self.session().player.region == r) {
                    return true;
                }
                let p = self.session().player;
                let d = (p.x - step.to.0).hypot(p.y - step.to.1);
                // Arrived only at about the planned height (a step 16 up in the next region is
                // not the spot); far off it, the plan went another way: give up on it.
                let dz = step.z.map_or(0.0, |z| (p.z - z).abs());
                // (Flying, height is steered: arrived only at it.)
                let level = dz < 12.0 || (!p.on_ground && !p.flying);
                // Held up just short of a precise spot (a wall in the way from this side, up to
                // its contact distance: a spot planned on a ledge's edge we dropped off): close
                // enough. Not before a jump, which needs its run-up spot.
                let next_jump = steps.get(k + 1).is_some_and(|n| n.jump);
                // (A region goal counts only from inside: a thin trigger strip lies within the
                // "close enough" of its centre.)
                let need_inside = k == last && inside.is_some();
                // Before a hop off a jump pad: arrive as the bounce comes down onto the pad (the
                // hop's Jump then catches the very next contact, here).
                // (A long hop off the pad, not a bounce up to a ledge right above.)
                let next_far = steps.get(k + 1).is_some_and(|n| (n.to.0 - step.to.0).hypot(n.to.1 - step.to.1) > 96.0);
                let pad_launch = next_jump && next_far && {
                    let s = self.session();
                    let floor = s.level.world.solid_floor(p.region);
                    // (Only for a spot on the pad itself, not a ledge it carries us up to.)
                    let on_pad_spot = step.z.is_some_and(|z| (z - floor.0).abs() < 24.0);
                    s.effects.bounce_speed(floor.1).is_some_and(|v| v > 0.0) && on_pad_spot && !(p.vz <= 0.0 && p.z - floor.0 < 24.0)
                };
                if (d < close || (stalled > 30 && d < if next_jump { 8.0 } else { 30.0 })) && level && !need_inside && !pad_launch {
                    break;
                }
                // Held just short of a thin strip a shut door pushes us out of: back off and take
                // a run-up (crossing into it at all fires it); then let it count.
                if need_inside && stalled == 30 && d < 24.0 {
                    let away = facing(step.to, (p.x, p.y));
                    for (ticks, angle, run) in [(20, away, false), (40, facing((p.x, p.y), step.to), true)] {
                        for _ in 0..ticks {
                            let s = self.game.session_mut().expect("session");
                            s.player.angle = angle;
                            let mut input = Input::default();
                            input.set(Action::Forward, true);
                            input.set(Action::Run, run);
                            self.tick(&input);
                            if run && inside.is_some_and(|r| self.session().player.region == r) {
                                return true;
                            }
                        }
                    }
                }
                if need_inside && stalled > 80 && d < 24.0 {
                    return true;
                }
                // (Above it: a drop still to come right ahead, so keep walking.)
                let drop_ahead = step.z.is_some_and(|z| z < p.z - 40.0);
                if d < close && dz > 40.0 && p.on_ground && k > 0 && !drop_ahead {
                    if self.verbose {
                        println!("     at step {k}/{last} but {dz:.0} off the planned height: player r{} ({:.0}, {:.0}, {:.0}), step ({:.0}, {:.0}, {:?})", p.region, p.x, p.y, p.z, step.to.0, step.to.1, step.z);
                    }
                    self.mark_bad(steps, k);
                    return false;
                }
                // (Flying, rising or sinking toward the planned height is progress too.)
                let progress = if p.flying { d + dz } else { d };
                if progress < best - 0.5 {
                    best = progress;
                    stalled = 0;
                } else if !p.on_ground && p.vz > 0.0 {
                    // Still rising (pressed against a wall a jump goes over): not stuck yet.
                } else {
                    stalled += 1;
                    if stalled > 90 {
                        if self.verbose {
                            let w = &self.session().level.world;
                            let to = w.region_at(step.to.0, step.to.1);
                            if k > 0 {
                                let pr = &steps[k - 1];
                                println!("     planned from ({:.1}, {:.1}, {:?}) jump {} → ({:.1}, {:.1}, {:?}) jump {} crouch {}", pr.to.0, pr.to.1, pr.z, pr.jump, step.to.0, step.to.1, step.z, step.jump, step.crouch);
                            }
                            println!("     stalled at step {k}/{last}: player r{} z {:.0} → ({:.0}, {:.0}) r{:?} floor {:?} ceil {:?}, d {d:.0}", p.region, p.z, step.to.0, step.to.1, to, to.map(|r| w.regions[r].floor), to.map(|r| w.regions[r].ceiling));
                        }
                        // Within reach of the goal counts.
                        if k == last && d < 32.0 && inside.is_none() {
                            return true;
                        }
                        self.mark_bad(steps, k);
                        return false;
                    }
                }
                let angle = facing((p.x, p.y), step.to);
                let s = self.game.session_mut().expect("session");
                s.player.angle = angle;
                let mut input = Input::default();
                input.set(Action::Forward, true);
                input.set(Action::Run, d > 48.0 && (!step.jump || d > 100.0));
                let low = {
                    let s = self.session();
                    let physics = s.physics_override.unwrap_or_default();
                    let w = &s.level.world;
                    let low_at = |r: Option<usize>| r.and_then(|r| w.regions.get(r)).is_some_and(|r| r.ceiling - r.floor < physics.height + physics.head_clearance);
                    // (Or heading into a gap too low to stand in: crouch before it.)
                    low_at(Some(p.region)) || low_at(w.region_at_z(step.to.0, step.to.1, Some(p.z + 1.0)))
                };
                input.set(Action::Crouch, step.crouch || low);
                // The planned jump, or a hop when stuck against a ledge.
                let deadly = self.session().effects.triggers().iter().any(|t| t.0 == p.region && self.death_tags().contains(&t.1[2]));
                // Planned jumps land somewhere safe; unplanned hops not where landing kills.
                // (A planned jump is pressed until it happens: a step down a stair just before it leaves
                // us in the air for a tick or two.)
                // (No unplanned hop right before a planned jump: it would start that jump in the air;
                // nor where the spot is not above us: a hop up onto a ledge beside the way leaves
                // the plan.)
                input.set(Action::Jump, (step.jump && !launched && best == progress) || (!deadly && !next_jump && step.z.is_none_or(|z| z > p.z + 8.0) && (stalled == 30 || stalled == 60)));
                // Over a jump pad on a hop: hold Jump, so the landing that relaunches us adds it
                // (the pad fires on the tick of contact).
                {
                    let s = self.session();
                    let pad_here = s.effects.bounce_speed(s.level.world.solid_floor(p.region).1).is_some_and(|v| v > 0.0);
                    if pad_here && (step.jump || (next_jump && !next_far)) {
                        input.set(Action::Jump, true);
                        // Not off the pad's edge before it has launched us (a long hop).
                        if step.jump && d > 96.0 && !p.on_ground && p.vz <= 0.0 {
                            input.set(Action::Forward, false);
                            input.set(Action::Run, false);
                        }
                    }
                }
                // Flying (swimming): hold Jump to rise, Crouch to sink to the planned height;
                // straight up or down, no forward.
                if p.flying && let Some(z) = step.z {
                    input.set(Action::Jump, z > p.z + 6.0);
                    input.set(Action::Crouch, z < p.z - 6.0);
                    if d < 6.0 {
                        input.set(Action::Forward, false);
                    }
                }
                self.tick(&input);
                let q = self.session().player;
                if step.jump && !q.on_ground && q.vz > 0.0 {
                    launched = true;
                }
            }
            // A jump that got there in the air: come down here (as the plan's simulation does,
            // with no forward) before the next step, rather than running on past the spot.
            if step.jump && k < last {
                let idle = Input::default();
                for _ in 0..120 {
                    let s = self.session();
                    let p = s.player;
                    // (Over a jump pad there is no coming to rest: its own steering handles it.)
                    let pad = s.effects.bounce_speed(s.level.world.solid_floor(p.region).1).is_some_and(|v| v > 0.0);
                    if p.on_ground || p.flying || pad || self.teleported || self.level_done() {
                        break;
                    }
                    self.tick(&idle);
                }
            }
        }
        true
    }

    /// Use the goal once there: walk into the wall, or shoot the bot.
    fn interact(&mut self, goal: &Goal) {
        if let Target::Region(r) = goal.target {
            // Swimming up into a ceiling trigger (a water surface): rise until it fires.
            if self.session().player.flying && self.session().effects.triggers().iter().any(|(t, tags)| *t == r && tags[3] > 0) {
                let mut up = Input::default();
                up.set(Action::Jump, true);
                // (Up to 1000 above us: swimming rises about 150 a second.)
                for _ in 0..(TICK_RATE as usize * 12) {
                    if self.teleported || self.level_done() || !self.session().player.flying {
                        break;
                    }
                    self.tick(&up);
                }
            }
            // A trigger region may start something under us (a floor that is an elevator):
            // stand still while the floor here moves.
            let idle = Input::default();
            let mut still = 0;
            let mut last = None;
            for _ in 0..(TICK_RATE as usize * 12) {
                if self.level_done() {
                    return;
                }
                self.tick(&idle);
                let s = self.session();
                let r = &s.level.world.regions[s.player.region];
                let now = (s.player.region, r.floor.to_bits());
                // (Also anything it set moving elsewhere: a bridge that rises ahead of us.)
                let busy = s.objects.any_height_motion() || s.effects.doors_moving();
                still = if Some(now) == last && !busy { still + 1 } else { 0 };
                last = Some(now);
                if still > 20 {
                    break;
                }
            }
            return;
        }
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
            let counter = self.session().objects.list.get(i).map_or(0, |o| o.counter);
            // For the boss, the strongest ranged tool we have (higher numbers first; missing
            // ones do nothing). Not for switches and walkers: a long shot can hit a trap switch
            // further off.
            let boss = self.boss.is_some_and(|(b, _)| b == i);
            // A switch overhead needs an aimed tool (the starting ones are lobbed: their shots
            // stay low); missing tools do nothing, so the last available one pressed wins.
            let overhead = self.session().objects.list.get(i).is_some_and(|o| o.effect == OTAGSW);
            let picks: Vec<Action> = if boss {
                Action::TOOLS.iter().rev().take(8).copied().collect()
            } else if overhead {
                [9, 7, 5, 4, 2].iter().filter_map(|&k| Action::TOOLS.get(k).copied()).collect()
            } else {
                Vec::new()
            };
            for tool in picks {
                let mut pick = Input::default();
                pick.set(tool, true);
                self.tick(&pick);
                self.tick(&Input::default());
            }
            let mut fire = Input::default();
            fire.set(Action::Use, true);
            for _ in 0..(TICK_RATE as usize * if boss { 20 } else { 6 }) {
                if self.level_done() {
                    return;
                }
                let s = self.session();
                let Some(o) = s.objects.list.get(i) else { break };
                if o.destroyed || o.hidden || (o.name.starts_with("KEYBOT") && o.mover.is_none()) || o.counter != counter {
                    break;
                }
                let angle = facing((s.player.x, s.player.y), (o.x, o.y));
                // Aim at its middle: projectiles that fly follow the view pitch.
                let physics = s.physics_override.unwrap_or_default();
                let dist = (o.x - s.player.x).hypot(o.y - s.player.y).max(1.0);
                let slope = (o.z + o.height / 2.0 - s.player.eye_z(&physics)) / dist;
                let look = (slope * 220.2 * 256.0 / 200.0).clamp(-512.0, 512.0);
                let p = &mut self.game.session_mut().expect("session").player;
                p.angle = angle;
                p.look = look;
                self.tick(&fire);
            }
            if let Some(s) = self.game.session_mut() {
                s.player.look = 0.0;
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
            let shut_door = |r: usize| doors.contains_key(&r) && w.regions[r].ceiling - w.regions[r].floor < 20.0;
            let (inside, out) = if inside_set.contains(&wall.front) && (!inside_set.contains(&b) || shut_door(b)) {
                (wall.front, b)
            } else if inside_set.contains(&b) && (!inside_set.contains(&wall.front) || shut_door(wall.front)) {
                (b, wall.front)
            } else {
                continue;
            };
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
        self.boss = self.find_boss();
        while self.ticks < max_ticks {
            if self.level_done() {
                let next = self.game.session().map_or_else(String::new, |s| s.name.clone());
                return Ok(if next.eq_ignore_ascii_case(&self.level) { "the end (boss killed)".into() } else { next });
            }
            if self.session().in_bsp {
                self.cross_bsp()?;
                continue;
            }
            let key = self.world_key();
            if key != self.world_key {
                self.world_key = key;
                self.unreachable.clear();
                self.failed_plans.clear();
            }
            let goals = self.goals();
            if goals.is_empty() {
                return Err("nothing left to use".into());
            }
            // Whatever the last switch opened up comes first (before a door closes again), then
            // things to collect and places to visit; switches and lifts change the world (an
            // elevator's other button would undo the ride), so only when nothing else is reachable.
            if let Ok(v) = std::env::var("PLAYBOT_HAS") {
                let n: Vec<[f32; 3]> = goals.iter().filter(|g| format!("{:?}", g.target) == v).map(|g| g.at).collect();
                println!("     has {v}: {n:?}");
            }
            let w = &self.session().level.world;
            // (Things, not switches: a door's other face is right behind it and would shut it.)
            let fresh: Vec<Goal> = goals.iter().filter(|g| !matches!(g.target, Target::Wall(_)) && w.region_at_z(g.at[0], g.at[1], Some(g.at[2] + 1.0)).is_some_and(|r| self.opened.contains(&r))).copied().collect();
            let movers = self.mover_regions();
            let tele_regions: HashSet<usize> = {
                let s = self.session();
                let tags: HashSet<i32> = s.objects.list.iter().filter(|o| o.effect == OTELE && o.tag > 0 && !self.death_tags().contains(&o.tag)).map(|o| o.tag).collect();
                s.effects.triggers().iter().filter(|(_, t)| t.iter().any(|x| tags.contains(x))).map(|(r, _)| *r).collect()
            };
            // (A used teleporter, object or trigger region, only when nothing new is left.)
            let teleporter = |g: &Goal| self.used.contains(&g.target) && (matches!(g.target, Target::Object(i) if self.session().objects.list.get(i).is_some_and(|o| o.effect == OTELE)) || matches!(g.target, Target::Region(r) if tele_regions.contains(&r)));
            let (firsts, switches): (Vec<Goal>, Vec<Goal>) = goals.iter().partition(|g| !matches!(g.target, Target::Wall(_) | Target::Lift(_)) && !matches!(g.target, Target::Region(r) if movers.contains(&r)) && !teleporter(g));
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
            // (A lock switch pressed before we had its key is new again once we carry it.)
            let locks = self.key_locks();
            let (new, old): (Vec<Goal>, Vec<Goal>) = switches.iter().partition(|g| !self.used.contains(&g.target) || locks.contains(&g.target));
            // (A switch that moves the floor we stand on, the down button of the lift we just
            // rode up, only once nothing else is reachable.)
            let under_us: HashSet<Target> = {
                let s = self.session();
                let w = &s.level.world;
                let floor = w.solid_floor(s.player.region).1;
                let names = s.level.region_names();
                let tags: HashSet<i32> = s.objects.list.iter().filter(|o| matches!(o.effect, 41 | 49) && o.tag > 0 && o.aux.as_ref().and_then(|a| a.slot(9)).and_then(|n| names.iter().position(|rn| rn.eq_ignore_ascii_case(n))) == Some(floor)).map(|o| o.tag).collect();
                s.effects.switches().iter().filter(|sw| tags.iter().any(|t| sw.tags.contains(t))).map(|sw| Target::Wall(sw.wall)).collect()
            };
            let (new_last, new_first): (Vec<Goal>, Vec<Goal>) = new.iter().partition(|g| under_us.contains(&g.target));
            let found = found.or_else(|| if new_first.is_empty() { None } else { self.plan(&new_first) });
            let found = found.or_else(|| if new_last.is_empty() { None } else { self.plan(&new_last) });
            // Only old switches left: open the next shut door on the way to the nearest thing
            // not yet reached.
            let found = found.or_else(|| {
                let pick = self.door_on_route(&goals, &old);
                if self.verbose {
                    println!("     route switches: {:?}", pick.iter().map(|g| g.target).collect::<Vec<_>>());
                }
                if pick.is_empty() { None } else { self.plan(&pick) }
            });
            // Too far for the grid search (a big maze): head for a region part-way along a
            // region-level route to the nearest unreached thing, then plan again from there.
            let found = found.or_else(|| {
                let mut wp = self.route_waypoint(&goals);
                // (Every route set aside: give them another go, from here.)
                if wp.is_none() && !self.route_fails.is_empty() {
                    self.route_fails.clear();
                    wp = self.route_waypoint(&goals);
                }
                if self.verbose {
                    println!("     waypoint {:?} (reached {} regions)", wp.map(|g| g.0.at), self.reached.len());
                }
                let (wp, target) = wp?;
                let r = self.plan(&[wp]);
                if self.verbose {
                    println!("     waypoint plan {:?}", r.as_ref().map(|r| r.1.len()));
                }
                // No way to it (a moat the route crosses): try the routes to other things.
                if r.is_none() {
                    self.route_fails.insert(target);
                }
                // Only worth it far away (the search ran out); a waypoint next to us means
                // something else (a door) is in the way.
                r.filter(|r| r.1.len() >= 10)
            });
            // Among old switches, the one pressed longest ago first (the nearest ones would
            // just cycle, e.g. an elevator's buttons).
            let found = found.or_else(|| {
                let mut old = old.clone();
                old.sort_by_key(|g| self.last_used.get(&g.target).copied().unwrap_or(0));
                let mut k = 0;
                let mut tries = 0;
                while k < old.len() && tries < 3 {
                    tries += 1;
                    let stamp = self.last_used.get(&old[k].target).copied().unwrap_or(0);
                    let batch: Vec<Goal> = old[k..].iter().take_while(|g| self.last_used.get(&g.target).copied().unwrap_or(0) == stamp).copied().collect();
                    k += batch.len();
                    if let Some(f) = self.plan(&batch) {
                        return Some(f);
                    }
                }
                // The rest together (one search).
                if k < old.len() { self.plan(&old[k..]) } else { None }
            });
            let Some((goal, steps)) = found else {
                // Nothing reachable now: wait for moving parts (a few times), then give up.
                let idle = Input::default();
                for _ in 0..TICK_RATE as usize {
                    self.tick(&idle);
                }
                self.waits += 1;
                let stuck_at = self.session().player;
                if self.waits > 8 && self.respawn() {
                    // Trapped (a pit with no way up): a player gets out by dying, which sends them
                    // back to the start; do the same.
                    let p = stuck_at;
                    self.note(format!("     no way on from ({:.0}, {:.0}, {:.0}) r{}: back to the start (death teleport)", p.x, p.y, p.z, p.region));
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
                        for g in goals.iter().filter(|g| (g.at[0] - p.x).hypot(g.at[1] - p.y) < 600.0) {
                            let w = &self.session().level.world;
                            println!("  near target {:?} at ({:.0}, {:.0}, {:.0}) region {:?}", g.target, g.at[0], g.at[1], g.at[2], w.region_at(g.at[0], g.at[1]));
                        }
                    }
                    let mut left: Vec<&Goal> = goals.iter().filter(|g| !self.used.contains(&g.target)).collect();
                    left.sort_by_key(|g| matches!(g.target, Target::Wall(_)));
                    let left: Vec<String> = left.iter().take(24).map(|g| format!("{:?}@r{:?}", g.target, self.session().level.world.region_at_z(g.at[0], g.at[1], Some(g.at[2] + 1.0)))).collect();
                    return Err(format!("stuck at ({:.0}, {:.0}, {:.0}) region {}; {} targets left. Unreached: {}\n  The reachable area ends at:\n    {}", p.x, p.y, p.z, p.region, goals.len(), left.join(" "), self.boundary_report()));
                }
                continue;
            };
            self.waits = 0;
            // Only ever re-pressing old switches: nothing new is reachable.
            if self.used.contains(&goal.target) || goal.target == Target::Region(WAYPOINT) {
                self.repeats += 1;
                // The same one or two old goals again and again (a door that leads nowhere new, two
                // teleporters back and forth): as shut in.
                self.recent_old.push(goal.target);
                if self.recent_old.len() > 10 {
                    self.recent_old.remove(0);
                }
                let distinct: HashSet<Target> = self.recent_old.iter().copied().collect();
                if self.recent_old.len() >= 10 && distinct.len() <= 2 {
                    self.repeats = self.repeats.max(61);
                    self.recent_old.clear();
                }
                // Shut in with only old switches (a one-way area): back to the start, as a
                // player would get out by dying; give up if that changes nothing five times.
                if self.repeats > 60 && self.idle_respawns < 5 && self.respawn() {
                    let p = self.session().player;
                    self.note(format!("     only old switches here: back to the start at ({:.0}, {:.0})", p.x, p.y));
                    self.idle_respawns += 1;
                    self.repeats = 0;
                    self.edges.clear();
                    continue;
                }
                if self.repeats > 60 {
                    let p = self.session().player;
                    let mut left: Vec<&Goal> = goals.iter().filter(|g| !self.used.contains(&g.target)).collect();
                    left.sort_by_key(|g| matches!(g.target, Target::Wall(_)));
                    let left: Vec<String> = left.iter().take(24).map(|g| format!("{:?}@r{:?}", g.target, self.session().level.world.region_at_z(g.at[0], g.at[1], Some(g.at[2] + 1.0)))).collect();
                    return Err(format!("only old switches left at ({:.0}, {:.0}, {:.0}) region {}. Unreached: {}\n  The reachable area ends at:\n    {}", p.x, p.y, p.z, p.region, left.join(" "), self.boundary_report()));
                }
            } else {
                self.repeats = 0;
                self.idle_respawns = 0;
                self.recent_old.clear();
            }
            self.note(format!("t={:.0}s → {:?} ({} steps)", self.ticks as f32 / TICK_RATE, goal.target, steps.len()));
            let start = { let p = self.session().player; (p.x, p.y) };
            // Switch objects fire on touching; stop at the edge rather than circling inside
            // (leaving and re-entering contact would toggle them again).
            let close = match goal.target {
                Target::Object(i) if self.session().objects.list.get(i).is_some_and(|o| matches!(o.effect, OTAGSW | OBUMPSW)) => 20.0,
                _ => 12.0,
            };
            self.teleported = false;
            if goal.target == Target::Region(WAYPOINT) {
                self.note(format!("t={:.0}s → waypoint ({:.0}, {:.0}) ({} steps)", self.ticks as f32 / TICK_RATE, goal.at[0], goal.at[1], steps.len()));
                self.teleported = false;
                if !self.walk(&steps) {
                    self.edges.clear();
                    self.waypoint_fails += 1;
                } else {
                    self.waypoint_fails = 0;
                }
                continue;
            }
            if std::env::var("PLAYBOT_PATH").is_ok_and(|v| v == format!("{:?}", goal.target)) {
                let path: Vec<String> = steps.iter().map(|st| format!("({:.0},{:.0},{:?}{}{}) r{:?}", st.to.0, st.to.1, st.z, if st.jump { " J" } else { "" }, if st.crouch { " C" } else { "" }, st.region)).collect();
                println!("     path to {:?}: {}", goal.target, path.join(" "));
            }
            let inside = if let Target::Region(r) = goal.target { Some(r) } else { None };
            let mut reached = self.walk_to(&steps, close, inside);
            // Teleported on the way (a trigger region or teleporter did its thing): that goal
            // is spent; plan afresh from wherever we landed.
            if self.teleported && !reached {
                self.teleported = false;
                // (A pickup still lying there is not spent: we fell into a trap short of it.)
                let still_there = matches!(goal.target, Target::Object(i) if self.session().objects.list.get(i).is_some_and(|o| o.effect == OPICKUP && !o.hidden && !o.destroyed));
                if still_there {
                    *self.failures.entry(goal.target).or_default() += 1;
                } else {
                    self.used.insert(goal.target);
                }
                self.edges.clear();
                self.note("     teleported on the way".into());
                if self.verbose {
                    let s = self.session();
                    let w = &s.level.world;
                    let inv: Vec<&str> = s.objects.inventory.iter().map(|&k| s.objects.list[k].name.as_str()).collect();
                    let path: Vec<String> = steps.iter().map(|st| format!("({:.0},{:.0},{:?}{}) r{:?}", st.to.0, st.to.1, st.z, if st.jump { " J" } else { "" }, w.region_at_z(st.to.0, st.to.1, st.z.map(|z| z + 1.0)))).collect();
                    println!("       carrying {inv:?}; path {}", path.join(" "));
                }
                continue;
            }
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
                    // Swimming: rise or sink to it as well.
                    let (rise, sink) = if s.player.flying { (o.z > s.player.z + 4.0, o.z < s.player.z - 4.0) } else { (false, false) };
                    // (Up on a ledge too high to step onto, as in a niche: jump to it.)
                    let ledge = !s.player.flying && s.player.on_ground && o.z - s.player.z > 24.0;
                    self.game.session_mut().expect("session").player.angle = angle;
                    let mut step_input = if near { Input::default() } else { input.clone() };
                    step_input.set(Action::Jump, rise || ledge);
                    step_input.set(Action::Crouch, sink);
                    self.tick(&step_input);
                }
                let o = &self.session().objects.list[i];
                reached = o.hidden || o.destroyed || self.level_done();
            }
            if reached {
                let before = self.area();
                self.interact(&goal);
                // A switch that set the floor under us moving (an elevator's own button): ride it
                // to the end before planning on (up to 15 s).
                if matches!(goal.target, Target::Wall(_) | Target::Object(_)) {
                    let idle = Input::default();
                    for _ in 0..(TICK_RATE as usize * 15) {
                        let s = self.session();
                        let under = s.level.world.solid_floor(s.player.region).1;
                        if !s.objects.region_moving(under) || self.level_done() {
                            break;
                        }
                        self.tick(&idle);
                    }
                }
                // A switch that opens a door which shuts itself again (RTDOOR with a wait): go
                // through at once, before it does (planning a long way round first, it is shut
                // by the time we get there).
                if let Target::Wall(wi) = goal.target {
                    self.through_timed_door(wi);
                }
                // A switch may have set a door moving: plan once it is open (or shut), not
                // while it is still a closed doorway (up to 4 s).
                if matches!(goal.target, Target::Wall(_) | Target::Object(_)) {
                    let idle = Input::default();
                    for _ in 0..(TICK_RATE as usize * 4) {
                        if !self.session().effects.doors_moving() || self.level_done() {
                            break;
                        }
                        self.tick(&idle);
                    }
                }
                // A trigger that fires on leaving: step back out the way we came; but where the
                // leave opens a door, out on the door's side (the door then stands open in front
                // of us; going back would mean crossing the region again after it shut).
                if let Target::Region(r) = goal.target
                    && let Some(leave) = self.session().effects.triggers().iter().find(|(t, tags)| *t == r && tags[1] > 0).map(|(_, tags)| tags[1])
                {
                    let back = steps.len().checked_sub(2).map_or(start, |k| steps[k].to);
                    let out = self.door_side_exit(r, leave).unwrap_or(back);
                    let _ = self.walk(&[Step { to: out, z: None, jump: false, crouch: false, region: None }]);
                }
                if matches!(goal.target, Target::Wall(_) | Target::Lift(_)) {
                    let after = self.area();
                    self.opened = after.difference(&before).copied().collect();
                } else {
                    self.opened.clear();
                }
                if !self.used.contains(&goal.target) {
                    self.bad_moves.clear();
                }
                self.used.insert(goal.target);
                self.last_used.insert(goal.target, self.ticks);
                self.waypoint_fails = 0;
                self.failures.clear(); // something changed: try the others again
                self.route_fails.clear();
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
        let mut bot = Bot { vfs: &vfs, game, level: level.clone(), used: HashSet::new(), unreachable: HashSet::new(), failures: HashMap::new(), waits: 0, opened: HashSet::new(), repeats: 0, recent_old: Vec::new(), idle_respawns: 0, teleported: false, last_used: HashMap::new(), waypoint_fails: 0, false_portals: std::cell::OnceCell::new(), death_tags: std::cell::OnceCell::new(), route_fails: HashSet::new(), failed_plans: HashSet::new(), cam_reported: HashSet::new(), boss: None, reached: HashSet::new(), bad_moves: HashSet::new(), visited: HashSet::new(), edges: HashMap::new(), world_key: 0, ticks: 0, verbose, god, log: Vec::new() };
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

/// Whether segments p1-p2 and q1-q2 properly intersect.
fn segments_cross(p1: (f32, f32), p2: (f32, f32), q1: (f32, f32), q2: (f32, f32)) -> bool {
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let (d1, d2) = (cross(q1, q2, p1), cross(q1, q2, p2));
    let (d3, d4) = (cross(p1, p2, q1), cross(p1, p2, q2));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}
