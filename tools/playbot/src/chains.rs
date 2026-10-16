//! Cause and effect from the level data: what to use, now, so that a tag eventually fires.
//!
//! Lips levels are chains: a switch turns placeholder flowers into a pickup (OOTOO), picking it
//! up fires a tag that opens a door, a lock (ODROP) fires its tag only when you bring the right
//! item, and a teleporter's trigger region is the only way into a room. The bot's walk-to-the-
//! nearest-new-thing search finds such chains only by trying everything; this works them out
//! backwards from the tag that is needed.

use crate::{OBUMPSW, OPICKUP, OTAGSW, OTELE, OWALK, Target};
use godd_game::session::Session;
use std::collections::HashSet;

/// OOTOO: morph objects named slot 9 into the template named slot 8.
const OOTOO: i32 = 66;
const OWTOW: i32 = 67;
/// ODROP: a lock that takes item slot 9 and fires int0 when its own tag fires.
const ODROP: i32 = 131;
/// How many links back a chain is followed.
const MAX_DEPTH: u32 = 6;

/// Things usable now (in the world, not hidden, not a death trap) that fire `tag`, directly or
/// through a chain of morphs and locks.
pub fn producers(s: &Session, tag: i32, death: &HashSet<i32>) -> HashSet<Target> {
    let mut out = HashSet::new();
    let mut seen = HashSet::new();
    collect(s, tag, death, 0, &mut seen, &mut out);
    out
}

fn collect(s: &Session, tag: i32, death: &HashSet<i32>, depth: u32, seen: &mut HashSet<i32>, out: &mut HashSet<Target>) {
    if tag <= 0 || depth > MAX_DEPTH || death.contains(&tag) || !seen.insert(tag) {
        return;
    }
    // Switch walls and trigger regions (entered, left, landed in, head against the ceiling).
    for sw in s.effects.switches().iter().filter(|sw| sw.usable && sw.tags.contains(&tag)) {
        out.insert(Target::Wall(sw.wall));
    }
    for (r, t) in s.effects.triggers() {
        if t.contains(&tag) {
            out.insert(Target::Region(r));
        }
    }
    let list = &s.objects.list;
    let live = |o: &godd_game::objects::Obj| !o.hidden && !o.destroyed && o.region.is_some();
    for (i, o) in list.iter().enumerate() {
        let fires = match o.effect {
            OTAGSW | OBUMPSW => o.tag == tag,
            OWALK => o.mover.is_none() && o.tag == tag,
            OPICKUP => o.aux.as_ref().is_some_and(|a| a.ints[0] == tag),
            _ => false,
        };
        if fires && live(o) {
            out.insert(Target::Object(i));
        }
    }
    // A pickup that fires it but is only a template (off the map): whatever is morphed into it.
    for o in list.iter().filter(|o| o.effect == OPICKUP && o.region.is_none() && o.aux.as_ref().is_some_and(|a| a.ints[0] == tag)) {
        morphs_into(s, &o.name, death, depth, seen, out);
    }
    // A lock that fires it: its own tag while we carry its item, or first the item.
    for o in list.iter().filter(|o| o.effect == ODROP && o.aux.as_ref().is_some_and(|a| a.ints[0] == tag)) {
        let Some(item) = o.aux.as_ref().and_then(|a| a.slot(9)) else { continue };
        let carried = s.objects.inventory.iter().any(|&k| list[k].name.eq_ignore_ascii_case(item));
        if carried {
            collect(s, o.tag, death, depth + 1, seen, out);
        } else {
            item_sources(s, item, death, depth + 1, seen, out);
        }
    }
}

/// Where to get item `name`: a pickup of that name lying in the world, or what morphs one in.
fn item_sources(s: &Session, name: &str, death: &HashSet<i32>, depth: u32, seen: &mut HashSet<i32>, out: &mut HashSet<Target>) {
    let list = &s.objects.list;
    let lying: Vec<usize> = (0..list.len()).filter(|&i| {
        let o = &list[i];
        o.effect == OPICKUP && o.name.eq_ignore_ascii_case(name) && !o.hidden && !o.destroyed && o.region.is_some()
    }).collect();
    if lying.is_empty() {
        morphs_into(s, name, death, depth, seen, out);
    }
    out.extend(lying.into_iter().map(Target::Object));
}

/// The tags of the OOTOO switches that turn something in the world into template `name`.
fn morphs_into(s: &Session, name: &str, death: &HashSet<i32>, depth: u32, seen: &mut HashSet<i32>, out: &mut HashSet<Target>) {
    let list = &s.objects.list;
    for o in list.iter().filter(|o| o.effect == OOTOO && o.tag > 0) {
        let Some(a) = &o.aux else { continue };
        let (Some(template), Some(placeholder)) = (a.slot(8), a.slot(9)) else { continue };
        // (A placeholder must exist in the world to become it.)
        if template.eq_ignore_ascii_case(name) && list.iter().any(|p| p.name.eq_ignore_ascii_case(placeholder) && !p.hidden && !p.destroyed && p.region.is_some()) {
            collect(s, o.tag, death, depth + 1, seen, out);
        }
    }
}

/// Teleports as region links: (from region, to region, what to use). A trigger region whose
/// tag fires a teleporter (LIPS00's pool r468 → the swim hall r303), and teleporter objects one
/// walks into. Teleporters home (death tags) are left out.
pub fn teleports(s: &Session, death: &HashSet<i32>) -> Vec<(usize, usize, Target)> {
    let list = &s.objects.list;
    let dest = |o: &godd_game::objects::Obj| -> Option<usize> {
        let name = o.aux.as_ref()?.slot(9)?;
        let d = list.iter().find(|d| d.name.eq_ignore_ascii_case(name))?;
        d.region.or_else(|| s.level.world.region_at_z(d.x, d.y, Some(d.z + 1.0)))
    };
    let mut out = Vec::new();
    for (r, t) in s.effects.triggers() {
        for o in list.iter().filter(|o| o.effect == OTELE && o.tag > 0 && !death.contains(&o.tag) && t.contains(&o.tag)) {
            if let Some(d) = dest(o) {
                out.push((r, d, Target::Region(r)));
            }
        }
    }
    for (i, o) in list.iter().enumerate() {
        if o.effect == OTELE && o.name.is_empty() && !o.hidden && !death.contains(&o.tag)
            && let (Some(from), Some(d)) = (o.region, dest(o))
        {
            out.push((from, d, Target::Object(i)));
        }
    }
    out
}

/// A thing to do on the way: (region, door tag, a teleport to take and where from).
type Call = (usize, i32, Option<(Target, usize)>);

/// Planner debug output on (PLAYBOT_SEQDBG, or set by the bot for one decision).
pub static DEBUG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn debugging() -> bool {
    DEBUG.load(std::sync::atomic::Ordering::Relaxed) || std::env::var("PLAYBOT_SEQDBG").is_ok()
}

/// One step of a level's sequence: what to use, and why.
#[derive(Debug, Clone)]
pub struct Step {
    pub target: Target,
    pub why: String,
    /// The region it is used from.
    pub at: usize,
}

/// The abstract state a sequence is planned over: which doors are open, what placeholders a
/// morph turned into pickups (index → (template name, tag it fires)), what is carried, what is
/// used, and the regions reachable now.
#[derive(Clone)]
struct Plan<'a> {
    s: &'a Session,
    death: &'a HashSet<i32>,
    open: Vec<bool>,
    morphed: std::collections::HashMap<usize, (String, i32)>,
    carried: Vec<String>,
    used: HashSet<Target>,
    start: usize,
    steps: Vec<Step>,
    tele: Vec<(usize, usize, Target)>,
    doors: std::collections::HashMap<usize, i32>,
    depth: u32,
    /// Tags being worked out now (a door's own far face needs the door open: not again).
    active: HashSet<i32>,
    /// Perpetual lifts (RFUP1): entered and left at any height of theirs (r405 caught on its
    /// way up, its floor over r210's ceiling).
    perpetual: HashSet<usize>,
    /// What the OSETF/OSETC movers of each region do: (tag, ceiling?, target height, after
    /// the ride: a mover a trigger-by-name of the same tag queues, LIPS00's R2DOWN taking
    /// the riser r251 back down once up).
    movers: std::collections::HashMap<usize, Vec<(i32, bool, f32, bool)>>,
    /// Their floor and ceiling in the plan (LIPS00's ELEV02 goes down on tag 24 and up on 25;
    /// the coffin r26 rises on its own wall 127; the riser r251 on entering it).
    heights: std::collections::HashMap<usize, (f32, f32)>,
    /// Walls an OWTOW copy changed in the plan: blocking or not (LIPS00's DARK01..08, made
    /// passable like LIGHT01..08 once the lamp is set down at r470, tag 93).
    blocking: std::collections::HashMap<usize, bool>,
    /// Walls an OWTOW tag makes passable: passed like shut doors on the way, the tag fired
    /// first.
    gates: std::collections::HashMap<usize, i32>,
    /// The region pairs such a gate stands between (lower, higher) → its wall: while it is
    /// shut, nothing between them is passed but on the way being worked out.
    gate_pairs: std::collections::HashMap<(usize, usize), usize>,
    /// Gate tags that could not be fired for the way being worked out: another way, then.
    closed_gates: HashSet<i32>,
    /// Doors that could not be opened for the way being worked out: another way, then (LIPS00's
    /// front door r5 shuts 5 s after BRONZE opens it; from the hall, no switch opens it again).
    closed_doors: HashSet<usize>,
    /// Tie-breaking for other orders (0: none).
    seed: u64,
    rng: std::cell::Cell<u64>,
    /// The longest run of steps carried out anywhere in the search, shared by every copy of
    /// the plan (a branch that comes to nothing still did each of its steps in order).
    longest: std::rc::Rc<std::cell::RefCell<Vec<Step>>>,
    /// Rides being worked out (a ride's switch reached by another ride: not too deep).
    riding: u32,
    first: Option<&'a HashSet<Target>>,
    /// Doors that shut again by themselves (RTDOOR wait) and have a switch: passed as open,
    /// their switch pressed right before the step that goes through (LIPS00's r81 and r253:
    /// opened early, they were shut again by the time the bot came back that way).
    timed: HashSet<usize>,
    /// Where dying (the DEATH01 teleporter, tag 3) puts the player, what was done kept: the
    /// way out of LIPS00's one-way west rooms.
    home: Option<usize>,
    may_die: bool,
    /// Switch walls with somewhere to stand and press them (not LIPS00's 1841, behind a wall).
    pressable: HashSet<usize>,
    /// Trigger regions firing a death tag: (on entering, on landing). Not entered, or not
    /// dropped into (LIPS00's sunk FALLZ r466: walked onto level it is safe).
    deadly: std::collections::HashMap<usize, (bool, bool)>,
}

/// A plan, from the data alone, of what to use in which order to reach the level's exit
/// (a tag 500 switch), starting from the player's region and state now. Each step is a switch,
/// pickup, trigger region or tag switch object; the walking between them is left to the bot.
/// Without a whole way to the exit, the steps worked out up to there (the last is then not
/// "the exit").
/// `first` limits the first step to what can be used now (the bot's goals); `None` for any.
/// `may_die`: going back to the start by dying is a way from the first step on (not before
/// anything was done: the level's opening is played, not skipped).
pub fn sequence(s: &Session, death: &HashSet<i32>, first: Option<&HashSet<Target>>, may_die: bool) -> Vec<Step> {
    let w = &s.level.world;
    let doors: std::collections::HashMap<usize, i32> = s.effects.doors().into_iter().collect();
    let tops: std::collections::HashMap<usize, f32> = s.effects.door_tops().into_iter().collect();
    let open = (0..w.regions.len()).map(|r| !doors.contains_key(&r) || w.regions[r].ceiling - w.regions[r].floor >= 56.0 || tops.get(&r).is_some_and(|&t| w.regions[r].ceiling >= t - 1.0)).collect();
    let carried = s.objects.inventory.iter().map(|&k| s.objects.list[k].name.to_ascii_uppercase()).collect();
    let movers: Vec<&godd_game::objects::Obj> = s.objects.list.iter().filter(|o| matches!(o.effect, 41 | 49)).collect();
    let mut by_region: std::collections::HashMap<usize, Vec<(i32, bool, f32, bool)>> = Default::default();
    for o in movers.iter() {
        let Some(a) = &o.aux else { continue };
        let Some(r) = a.slot(9).and_then(|n| s.region_named(n)).filter(|_| a.ints[1] > 0) else { continue };
        if o.tag > 0 {
            by_region.entry(r).or_default().push((o.tag, o.effect == 49, a.ints[0] as f32, false));
        }
        for t in s.objects.list.iter().filter(|t| t.effect == 80 && t.tag > 0 && t.aux.as_ref().and_then(|ta| ta.slot(9)).is_some_and(|n| !o.name.is_empty() && n.eq_ignore_ascii_case(&o.name))) {
            by_region.entry(r).or_default().push((t.tag, o.effect == 49, a.ints[0] as f32, true));
        }
    }
    let heights = by_region.keys().map(|&r| (r, (w.regions[r].floor, w.regions[r].ceiling))).collect();
    let perpetual = s.effects.lifts().into_iter().map(|(r, _)| r).collect();
    let mut p = Plan { s, death, open, morphed: Default::default(), carried, used: HashSet::new(), start: s.player.region, steps: Vec::new(), tele: teleports(s, death), doors, depth: 0, active: HashSet::new(), perpetual, movers: by_region, heights, seed: 0, rng: std::cell::Cell::new(1), longest: Default::default(), blocking: Default::default(), gates: Default::default(), gate_pairs: Default::default(), closed_gates: HashSet::new(), closed_doors: HashSet::new(), riding: 0, first, timed: HashSet::new(), home: None, may_die, pressable: HashSet::new(), deadly: Default::default() };
    p.pressable = s.effects.switches().iter().filter(|sw| sw.usable && !crate::switch_spots(w, sw.wall).0.is_empty()).map(|sw| sw.wall).collect();
    p.deadly = s.effects.triggers().into_iter().filter_map(|(r, t)| {
        let (enter, land) = (death.contains(&t[0]), death.contains(&t[2]));
        (enter || land).then_some((r, (enter, land)))
    }).collect();
    // Teleporters a tag fires, from wherever that tag is fired: a switch, a trigger region, a
    // walker shot from beside it (LIPS00's #243 fires 71: out to CHEIRAFT).
    {
        let list = &s.objects.list;
        for o in list.iter().filter(|o| o.effect == OTELE && o.tag > 0 && !death.contains(&o.tag)) {
            let t = o.tag;
            let Some(d) = o.aux.as_ref().and_then(|a| a.slot(9)).and_then(|n| list.iter().find(|d| d.name.eq_ignore_ascii_case(n))) else { continue };
            let Some(dest) = d.region.or_else(|| w.region_at_z(d.x, d.y, Some(d.z + 1.0))) else { continue };
            let mut from: Vec<(usize, Target)> = Vec::new();
            for sw in s.effects.switches().iter().filter(|sw| sw.usable && sw.tags.contains(&t)) {
                let x = &w.walls[sw.wall];
                from.extend(std::iter::once(x.front).chain(x.back).map(|r| (r, Target::Wall(sw.wall))));
            }
            for (i, so) in list.iter().enumerate().filter(|(_, so)| matches!(so.effect, OWALK | OTAGSW | OBUMPSW) && so.tag == t && !so.hidden && !so.destroyed) {
                let Some(r) = so.region.or_else(|| w.region_at_z(so.x, so.y, Some(so.z + 1.0))) else { continue };
                from.push((r, Target::Object(i)));
                if so.effect == OWALK {
                    for &wi in &w.regions[r].walls {
                        let x = &w.walls[wi];
                        if let Some(b) = x.back.filter(|&b| b != x.front) {
                            from.push((if x.front == r { b } else { x.front }, Target::Object(i)));
                        }
                    }
                }
            }
            for (r, target) in from {
                if !p.tele.iter().any(|e| e.0 == r && e.1 == dest) {
                    p.tele.push((r, dest, target));
                }
            }
        }
    }
    p.home = s.objects.list.iter().filter(|o| o.effect == OTELE && matches!(o.tag, 3 | 661)).find_map(|o| {
        let d = s.objects.list.iter().find(|d| o.aux.as_ref().and_then(|a| a.slot(9)).is_some_and(|n| d.name.eq_ignore_ascii_case(n)))?;
        d.region.or_else(|| w.region_at_z(d.x, d.y, Some(d.z + 1.0)))
    });
    for o in s.objects.list.iter().filter(|o| o.effect == OWTOW && o.tag > 0) {
        let Some(a) = &o.aux else { continue };
        if let (Some(to), Some(from)) = (a.slot(9).and_then(|n| s.wall_named(n)), a.slot(8).and_then(|n| s.wall_named(n))) && w.walls[to].blocking && !w.walls[from].blocking {
            p.gates.insert(to, o.tag);
            if let Some(b) = w.walls[to].back {
                let f = w.walls[to].front;
                p.gate_pairs.insert((f.min(b), f.max(b)), to);
            }
        }
    }
    let switch_tags: HashSet<i32> = s.effects.switches().iter().filter(|sw| sw.usable).flat_map(|sw| sw.tags.clone()).collect();
    p.timed = s.effects.door_waits().into_iter().map(|(r, _)| r).filter(|r| p.doors.get(r).is_some_and(|t| switch_tags.contains(t))).collect();
    let exits: Vec<usize> = s.effects.switches().iter().filter(|sw| sw.tags.contains(&500)).map(|sw| sw.wall).collect();
    // Other orders when one comes to nothing: ties in the ways and the choices broken by
    // seed (LIPS00: BRONZE opens the front door once, for 20 s; an order that spends that trip
    // outside before GOLD is fetched there has no second one).
    let mut best: Vec<Step> = Vec::new();
    for seed in 0..48u64 {
        let mut q = p.clone();
        q.longest = Default::default();
        q.rng.set(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        q.seed = seed;
        for &wall in &exits {
            let x = &w.walls[wall];
            let regions: Vec<usize> = std::iter::once(x.front).chain(x.back).collect();
            // (Not undone on failure here: the steps so far are the partial plan.)
            if regions.iter().any(|&r| q.reach_region_inner(r)) {
                q.steps.push(Step { target: Target::Wall(wall), why: "the exit".into(), at: x.front });
                return q.steps;
            }
        }
        if debugging() {
            for (k, st) in q.steps.iter().enumerate() {
                println!("       seq: (failed, order {seed}) {}: {:?} ({})", k + 1, st.target, st.why);
            }
        }
        let longest = q.longest.borrow().clone();
        if longest.len() > best.len() {
            best = longest;
        }
    }
    // (No whole way worked out: the longest run of steps found, the way they go is still right;
    // the bot works the plan out again after each.)
    best
}

/// The planner's region route from the player to region `to` with the world as it is now
/// (doors as they are, timed ones passed, lifts at any height): the regions in order, empty
/// when there is none.
pub fn route_to(s: &Session, death: &HashSet<i32>, to: usize) -> Vec<usize> {
    let w = &s.level.world;
    let doors: std::collections::HashMap<usize, i32> = s.effects.doors().into_iter().collect();
    let tops: std::collections::HashMap<usize, f32> = s.effects.door_tops().into_iter().collect();
    let open = (0..w.regions.len()).map(|r| !doors.contains_key(&r) || w.regions[r].ceiling - w.regions[r].floor >= 56.0 || tops.get(&r).is_some_and(|&t| w.regions[r].ceiling >= t - 1.0)).collect();
    let mut p = Plan {
        s, death, open, morphed: Default::default(), carried: Vec::new(), used: HashSet::new(), start: s.player.region, steps: Vec::new(), tele: teleports(s, death), doors, depth: 0, active: HashSet::new(),
        perpetual: s.effects.lifts().into_iter().map(|(r, _)| r).collect(), movers: Default::default(), heights: Default::default(), seed: 0, rng: std::cell::Cell::new(1), longest: Default::default(),
        blocking: Default::default(), gates: Default::default(), gate_pairs: Default::default(), closed_gates: HashSet::new(), closed_doors: HashSet::new(), riding: 0, first: None, timed: HashSet::new(), home: None, may_die: false, pressable: HashSet::new(), deadly: Default::default(),
    };
    p.timed = s.effects.door_waits().into_iter().map(|(r, _)| r).collect();
    let (prev, set) = p.reachable(false);
    if !set.contains(&to) {
        return Vec::new();
    }
    let mut path = vec![to];
    let mut q = to;
    while prev[q] != q {
        q = prev[q];
        path.push(q);
    }
    path.reverse();
    path
}

impl Plan<'_> {
    /// Regions reachable from the start with doors as they are in the plan.
    fn reachable(&self, through_shut: bool) -> (Vec<usize>, HashSet<usize>) {
        self.reachable_as(through_shut, false)
    }

    /// `loose`: floors that movers move are taken at any height (the way, rides left for
    /// later: from the bottom of LIPS00's ELEV02, what is down there before riding up).
    fn reachable_as(&self, through_shut: bool, loose: bool) -> (Vec<usize>, HashSet<usize>) {
        use std::collections::VecDeque;
        let w = &self.s.level.world;
        let n = w.regions.len();
        let mut prev = vec![usize::MAX; n];
        prev[self.start] = self.start;
        let mut queue = VecDeque::from([self.start]);
        while let Some(r) = queue.pop_front() {
            let reg = &w.regions[r];
            let (rf, rc) = self.fc(r);
            let mut next: Vec<usize> = reg.below.into_iter().collect();
            next.extend(self.tele.iter().filter(|e| e.0 == r).map(|e| e.1));
            for &wi in &reg.walls {
                let x = &w.walls[wi];
                let Some(b) = x.back.filter(|&b| b != x.front) else { continue };
                let o = if x.front == r { b } else { x.front };
                let door = self.doors.contains_key(&o);
                let gate_shut = |g: usize| self.blocking.get(&g).copied().unwrap_or(w.walls[g].blocking);
                if let Some(&g) = self.gate_pairs.get(&(r.min(o), r.max(o))) && gate_shut(g) && (!through_shut || self.closed_gates.contains(&self.gates[&g])) {
                    continue;
                }
                let blocking = self.blocking.get(&wi).copied().unwrap_or(x.blocking) && !(through_shut && self.gates.get(&wi).is_some_and(|t| !self.closed_gates.contains(t)));
                if (blocking && !door && !self.doors.contains_key(&r)) || (door && !self.open[o] && ((!through_shut && !self.timed.contains(&o)) || self.closed_doors.contains(&o))) {
                    continue;
                }
                let (ff, fcl) = self.fc(o);
                // (A door is passed open, from either side.)
                let top = if door { fcl.max(ff + 128.0) } else { fcl };
                let here = if self.doors.contains_key(&r) { rc.max(rf + 128.0) } else { rc };
                if let Some(&(enter, land)) = self.deadly.get(&o) && (enter || (land && ff < rf - 8.0)) {
                    continue;
                }
                let lift = self.perpetual.contains(&r) || self.perpetual.contains(&o) || (loose && (self.movers.contains_key(&r) || self.movers.contains_key(&o)));
                if lift || (here.min(top) - rf.max(ff) >= 24.0 && ff - rf <= 80.0) {
                    next.push(o);
                }
            }
            // (Off a raised riser onto a ledge across the neighbour beside it: LIPS00's r57, a
            // pillar alone in the pit r214, risen to 0.)
            if let Some(list) = self.movers.get(&r) {
                let top = if loose { list.iter().filter(|m| !m.1).map(|m| m.2).fold(rf, f32::max) } else { rf };
                let ((ax0, ay0), (ax1, ay1)) = reg.bounds;
                for &wi in &reg.walls {
                    let x = &w.walls[wi];
                    let Some(b) = x.back.filter(|&b| b != x.front) else { continue };
                    let nb = if x.front == r { b } else { x.front };
                    for &wj in &w.regions[nb].walls {
                        let y = &w.walls[wj];
                        let Some(c) = y.back.filter(|&c| c != y.front) else { continue };
                        let m = if y.front == nb { c } else { y.front };
                        let ((bx0, by0), (bx1, by1)) = w.regions[m].bounds;
                        let (mf, mc) = self.fc(m);
                        let gap = (bx0 - ax1).max(ax0 - bx1).max(0.0).hypot((by0 - ay1).max(ay0 - by1).max(0.0));
                        let shut = self.doors.contains_key(&m) && !self.open[m] && !self.timed.contains(&m) && !through_shut;
                        if m != r && nb != m && !shut && mf <= top + 32.0 && mf > w.regions[nb].floor + 80.0 && mc - mf >= 24.0 && gap <= 128.0 {
                            next.push(m);
                        }
                    }
                }
            }
            if self.seed != 0 {
                self.shuffle(&mut next);
            }
            for o in next {
                if o < n && prev[o] == usize::MAX {
                    prev[o] = r;
                    queue.push_back(o);
                }
            }
        }
        let set = (0..n).filter(|&r| prev[r] != usize::MAX).collect();
        (prev, set)
    }

    /// Make region `r` reachable: open the shut doors on a way there, in order. On failure the
    /// plan is as it was (no steps of a way that came to nothing).
    fn reach_region(&mut self, r: usize) -> bool {
        // (At the top, a failure keeps what was worked out before it: the partial plan.)
        let saved = (self.depth > 0).then(|| self.clone());
        let ok = self.reach_region_inner(r);
        if !ok && let Some(saved) = saved {
            *self = saved;
        }
        ok
    }

    fn reach_region_inner(&mut self, r: usize) -> bool {
        let dbg = debugging();
        for _ in 0..12 {
            let (prev, now) = self.reachable(false);
            if now.contains(&r) {
                // Open the timed doors and take the teleports on the way.
                let w = &self.s.level.world;
                let mut q = r;
                let mut calls = Vec::new();
                while prev[q] != q {
                    let p0 = prev[q];
                    let walked = w.regions[p0].below == Some(q) || w.regions[p0].walls.iter().any(|&wi| {
                        let x = &w.walls[wi];
                        (x.front == p0 && x.back == Some(q)) || (x.front == q && x.back == Some(p0))
                    });
                    if !walked && let Some(&(_, _, target)) = self.tele.iter().find(|e| e.0 == p0 && e.1 == q) {
                        calls.push((q, 0, Some((target, p0))));
                    }
                    if self.timed.contains(&q) && !self.open[q] {
                        calls.push((q, self.doors[&q], None));
                    }
                    q = p0;
                }
                // (A timed door with no switch on our side, first: what opens it otherwise
                    // comes before walking off toward it. LIPS00: OCTRED in the swim hall, for
                    // BRONZE and the front door, before leaving the hall.)
                let calls: Vec<Call> = calls.into_iter().rev().collect();
                let mut settled = false;
                for &(q, t, tele) in &calls {
                    if tele.is_none() && self.timed_switch(q, t, prev[q]).is_none() {
                        self.timed.remove(&q);
                        let ok = self.need_tag(t, &format!("opens door r{q}"));
                        self.timed.insert(q);
                        if !ok {
                            if self.closed_doors.insert(q) {
                                let ok = self.reach_region(r);
                                self.closed_doors.remove(&q);
                                return ok;
                            }
                            return false;
                        }
                        settled = true;
                        break;
                    }
                }
                if settled {
                    continue;
                }
                for (q, t, tele) in calls {
                    // (A teleport: the bot walks into it as a step; LIPS00's pool r468 to the
                    // swim hall, its ceiling r305 back out.)
                    if let Some((target, from)) = tele {
                        self.steps.push(Step { target, why: format!("teleports to r{q}"), at: from });
                        continue;
                    }
                    if dbg {
                        let mut path = vec![r];
                        let mut z = r;
                        while prev[z] != z { z = prev[z]; path.push(z); }
                        println!("       seq: timed door r{q} from r{} on the way to r{r}: {:?}", prev[q], path.iter().rev().collect::<Vec<_>>());
                    }
                    if !self.open_timed(q, t, prev[q]) {
                        if self.closed_doors.insert(q) {
                            let ok = self.reach_region(r);
                            self.closed_doors.remove(&q);
                            return ok;
                        }
                        return false;
                    }
                }
                return true;
            }
            let (prev, all) = self.reachable_as(true, true);
            // The first shut door on the way, if any; else a ride brings it in reach.
            // (Door regions as usize; a gate wall's tag as the door's tag, region usize::MAX.)
            let (door, gate) = if all.contains(&r) {
                let w = &self.s.level.world;
                let mut q = r;
                let (mut first, mut gate) = (None, None);
                while prev[q] != q {
                    let p0 = prev[q];
                    if self.doors.contains_key(&q) && !self.open[q] && !self.timed.contains(&q) {
                        (first, gate) = (Some(q), None);
                    } else {
                        // Only blocking walls between, one of them a gate.
                        let between: Vec<usize> = w.regions[q].walls.iter().copied().filter(|&wi| {
                            let x = &w.walls[wi];
                            (x.front == q && x.back == Some(p0)) || (x.front == p0 && x.back == Some(q))
                        }).collect();
                        // (A shut gate between them, even beside an open line: LIPS00's dark
                        // r425 to r437 is crossed only once the lamp lights it, DARK08.)
                        let shut = |wi: &usize| self.blocking.get(wi).copied().unwrap_or(w.walls[*wi].blocking);
                        if let Some(&t) = between.iter().filter(|wi| shut(wi)).find_map(|wi| self.gates.get(wi)).filter(|t| !self.closed_gates.contains(t)) {
                            (first, gate) = (None, Some(t));
                        }
                    }
                    q = p0;
                }
                (first, gate)
            } else {
                (None, None)
            };
            if let Some(t) = gate {
                if dbg { println!("       seq: to r{r}, a wall opened by tag {t}"); }
                if self.need_tag(t, &format!("opens the way to r{r}")) {
                    continue;
                }
                // (Not now: another way round, LIPS00's lamp lying up the way it would light.)
                if self.closed_gates.insert(t) {
                    let ok = self.reach_region(r);
                    self.closed_gates.remove(&t);
                    return ok;
                }
                return false;
            }
            if door.is_none() && all.contains(&r) && self.riding < 3 {
                let all = self.reachable(true).1;
                if let Some((target, at, e, h, t)) = self.find_ride(r, &all) {
                    if dbg { println!("       seq: to r{r}: ride r{e} to {h} with {target:?}"); }
                    self.riding += 1;
                    let ok = self.reach_region(at);
                    self.riding -= 1;
                    if ok {
                        let w = &self.s.level.world;
                        // (A button in a recess beside the car: the player rides in the car.)
                        let beside = at != e && w.regions[at].walls.iter().any(|&wi| w.walls[wi].front == e || w.walls[wi].back == Some(e));
                        self.use_target(target, at, &format!("moves r{e} to {h}"));
                        if beside {
                            self.start = e;
                        }
                        self.ride_done(t, r);
                        continue;
                    }
                }
                // (No source of the ride yet: make one, as for a door. LIPS00's lift r357 down
                // to the west rooms goes on SUMTHING, a morph of NOTHING once LIPZ is taken.)
                let mut tags: Vec<(i32, usize)> = self.movers.iter().flat_map(|(&e, l)| l.iter().map(move |m| (m.0, e))).collect();
                tags.sort();
                tags.dedup_by_key(|t| t.0);
                let mut made = false;
                for (t, e) in tags {
                    let after = self.moved(t);
                    if after == self.heights || self.death.contains(&t) || self.active.contains(&t) {
                        continue;
                    }
                    let saved = (std::mem::replace(&mut self.heights, after.clone()), self.start);
                    self.start = e;
                    let reaches = self.reachable(true).1.contains(&r);
                    (self.heights, self.start) = saved;
                    if !reaches {
                        continue;
                    }
                    if dbg { println!("       seq: to r{r}: ride r{e} on tag {t}, made first"); }
                    self.riding += 1;
                    let ok = self.need_tag(t, &format!("moves r{e} to {}", after[&e].0));
                    self.riding -= 1;
                    if ok {
                        self.ride_done(t, r);
                        made = true;
                        break;
                    }
                }
                if made {
                    continue;
                }
                return false;
            }
            // (Shut in: back to the start, if the way is from there.)
            // (Only once something was done: not to skip the level's opening.)
            if !all.contains(&r) && (self.may_die || !self.steps.is_empty()) && let Some(h) = self.home.filter(|&h| h != self.start) {
                let here = std::mem::replace(&mut self.start, h);
                if self.reachable_as(true, true).1.contains(&r) {
                    // First what the way from there needs that is in reach here (LIPS00's
                    // #559 in the west rooms, the door r50 it opens wanted later).
                    let mut dry = self.clone();
                    let base = dry.steps.len();
                    self.start = here;
                    if dry.reach_region(r) {
                        let now = self.reachable(false).1;
                        let pre: Vec<Step> = dry.steps[base..].iter().filter(|st| {
                            now.contains(&st.at) && !self.used.contains(&st.target) && st.why != "back to the start" && !st.why.ends_with("(it shuts again)") && !st.why.starts_with("moves r")
                        }).cloned().collect();
                        for st in pre {
                            if dbg { println!("       seq: before going back, {:?} ({})", st.target, st.why); }
                            if self.reach_region(st.at) {
                                self.use_target(st.target, st.at, &st.why);
                            }
                        }
                    }
                    self.start = h;
                    if dbg { println!("       seq: to r{r}: back to the start r{h} from r{here}"); }
                    self.steps.push(Step { target: Target::Region(h), why: "back to the start".into(), at: h });
                    for &q in &self.timed {
                        self.open[q] = false;
                    }
                    continue;
                }
                self.start = here;
            }
            if !all.contains(&r) {
                if dbg {
                    let mut v: Vec<&usize> = all.iter().collect();
                    v.sort();
                    println!("       seq: r{r} not reachable even through doors (from r{}): {v:?}", self.start);
                    let w = &self.s.level.world;
                    for &q in &all {
                        for &wi in &w.regions[q].walls {
                            let x = &w.walls[wi];
                            let Some(b) = x.back.filter(|&b| b != x.front) else { continue };
                            let o = if x.front == q { b } else { x.front };
                            if !all.contains(&o) { println!("         r{q} [{}..{}] -> r{o} [{}..{}] blocking {} door {:?}", w.regions[q].floor, w.regions[q].ceiling, w.regions[o].floor, w.regions[o].ceiling, x.blocking, self.doors.get(&o)); }
                        }
                    }
                }
                return false;
            }
            let Some(door) = door else { return false };
            if dbg {
                let mut path = vec![r];
                let mut z = r;
                while prev[z] != z { z = prev[z]; path.push(z); }
                path.reverse();
                println!("       seq: to r{r}, door r{door} (tag {}) on {path:?}", self.doors[&door]);
            }
            if !self.need_tag(self.doors[&door], &format!("opens door r{door}")) {
                if dbg { println!("       seq: nothing fires tag {} for door r{door}", self.doors[&door]); }
                if self.closed_doors.insert(door) {
                    let ok = self.reach_region(r);
                    self.closed_doors.remove(&door);
                    return ok;
                }
                return false;
            }
        }
        false
    }

    /// Shuffle by the plan's seed (xorshift).
    fn shuffle<T>(&self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let mut x = self.rng.get();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.rng.set(x);
            v.swap(i, (x % (i as u64 + 1)) as usize);
        }
    }

    /// A region's floor and ceiling in the plan.
    fn fc(&self, r: usize) -> (f32, f32) {
        self.heights.get(&r).copied().unwrap_or_else(|| {
            let g = &self.s.level.world.regions[r];
            (g.floor, g.ceiling)
        })
    }

    /// The heights after tag `t`'s movers (`back`: the ones queued after, instead).
    fn moved_by(&self, t: i32, back: bool) -> std::collections::HashMap<usize, (f32, f32)> {
        let mut h = self.heights.clone();
        for (r, list) in &self.movers {
            for &(tag, ceiling, target, after) in list {
                if tag == t && after == back && let Some(v) = h.get_mut(r) {
                    if ceiling { v.1 = target } else { v.0 = target }
                }
            }
        }
        h
    }

    fn moved(&self, t: i32) -> std::collections::HashMap<usize, (f32, f32)> {
        self.moved_by(t, false)
    }

    /// After a ride on tag `t` toward `r`: off at `r` if it is in reach now, then the movers
    /// queued after it take the floor back.
    fn ride_done(&mut self, t: i32, r: usize) {
        // (Off as far toward `r` as no shut door stops us: the floor goes back after.)
        let (prev, all) = self.reachable(true);
        let now = self.reachable(false).1;
        if all.contains(&r) {
            let w = &self.s.level.world;
            let mut path = vec![r];
            let mut q = r;
            while prev[q] != q {
                q = prev[q];
                path.push(q);
            }
            path.reverse();
            // (Up to the first teleport on the way: it is a step of its own, taken next.)
            let mut stop = path[0];
            for pair in path.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let walked = w.regions[a].below == Some(b) || w.regions[a].walls.iter().any(|&wi| {
                    let x = &w.walls[wi];
                    (x.front == a && x.back == Some(b)) || (x.front == b && x.back == Some(a))
                });
                if !walked || !now.contains(&b) {
                    break;
                }
                stop = b;
            }
            self.start = stop;
        }
        self.heights = self.moved_by(t, true);
    }

    /// A ride that brings region `r` in reach: (what fires it, where from, the region moved,
    /// its new floor). Sources among `all` (reachable through doors), trigger regions or
    /// usable switches.
    fn find_ride(&mut self, r: usize, all: &HashSet<usize>) -> Option<(Target, usize, usize, f32, i32)> {
        let s = self.s;
        let w = &s.level.world;
        let mut tags: Vec<(i32, usize)> = self.movers.iter().flat_map(|(&e, l)| l.iter().map(move |m| (m.0, e))).collect();
        tags.sort();
        tags.dedup_by_key(|t| t.0);
        let triggers = s.effects.triggers();
        let switches = s.effects.switches();
        for (t, e) in tags {
            let after = self.moved(t);
            if after == self.heights || self.death.contains(&t) {
                continue;
            }
            let mut sources: Vec<(Target, usize)> = Vec::new();
            for (q, tt) in &triggers {
                if (tt[0] == t || tt[2] == t) && all.contains(q) {
                    sources.push((Target::Region(*q), *q));
                }
            }
            for sw in switches.iter().filter(|sw| sw.usable && sw.tags.contains(&t) && self.pressable.contains(&sw.wall)) {
                let x = &w.walls[sw.wall];
                if let Some(q) = std::iter::once(x.front).chain(x.back).find(|q| all.contains(q)) {
                    sources.push((Target::Wall(sw.wall), q));
                }
            }
            // (And switch objects: LIPS00's #531 takes the lift r357 down to the west rooms.)
            for (i, o) in s.objects.list.iter().enumerate() {
                let fires = match o.effect {
                    OTAGSW | OBUMPSW => o.tag == t,
                    OWALK => o.mover.is_none() && o.tag == t,
                    OPICKUP => o.aux.as_ref().is_some_and(|a| a.ints[0] == t),
                    _ => false,
                } || self.morphed.get(&i).is_some_and(|m| m.1 == t);
                if fires && !o.hidden && !o.destroyed && !self.used.contains(&Target::Object(i)) && let Some(q) = o.region.filter(|q| all.contains(q)) {
                    sources.push((Target::Object(i), q));
                }
            }
            sources.retain(|(target, _)| !(self.steps.is_empty() && self.first.is_some_and(|ok| !ok.contains(target))));
            for (target, at) in sources {
                let beside = at != e && w.regions[at].walls.iter().any(|&wi| w.walls[wi].front == e || w.walls[wi].back == Some(e));
                let saved = (std::mem::replace(&mut self.heights, after.clone()), self.start);
                self.start = if beside { e } else { at };
                let reaches = self.reachable(true).1.contains(&r);
                (self.heights, self.start) = saved;
                if reaches {
                    return Some((target, at, e, after[&e].0, t));
                }
            }
        }
        None
    }

    /// Open timed door `q` (tag `t`) from region `from` just before going through: a switch
    /// on that side if any, else one reachable now. Not marked used: it is pressed again the
    /// next time.
    fn open_timed(&mut self, q: usize, t: i32, from: usize) -> bool {
        if let Some(wall) = self.timed_switch(q, t, from) {
            self.steps.push(Step { target: Target::Wall(wall), why: format!("opens door r{q} (it shuts again)"), at: from });
            self.fire(t);
            return true;
        }
        // (No switch to hand: whatever else opens it. LIPS00's front door r5 from r24 opens
        // on BRONZE; its switch 1841 there stands behind a wall.)
        self.timed.remove(&q);
        let ok = self.need_tag(t, &format!("opens door r{q}"));
        self.timed.insert(q);
        ok
    }

    /// The switch that opens timed door `q` (tag `t`) for one coming from region `from`.
    fn timed_switch(&mut self, q: usize, t: i32, from: usize) -> Option<usize> {
        let s = self.s;
        let w = &s.level.world;
        // (Reachable with this door shut: not its far face through itself.)
        self.timed.remove(&q);
        let now = self.reachable(false).1;
        self.timed.insert(q);
        let mut cands: Vec<(usize, bool)> = s.effects.switches().iter().filter(|sw| sw.usable && sw.tags.contains(&t) && self.pressable.contains(&sw.wall)).filter_map(|sw| {
            let x = &w.walls[sw.wall];
            let sides: Vec<usize> = std::iter::once(x.front).chain(x.back).collect();
            let near = sides.contains(&from);
            (near || sides.iter().any(|r| now.contains(r) && !self.timed.contains(r))).then_some((sw.wall, near))
        }).collect();
        if self.steps.is_empty() && let Some(ok) = self.first {
            cands.retain(|(wall, _)| ok.contains(&Target::Wall(*wall)));
        }
        cands.sort_by_key(|c| !c.1);
        cands.first().map(|c| c.0)
    }

    /// Fire tag `t`: use something that fires it (reaching it first), or what makes one.
    fn need_tag(&mut self, t: i32, why: &str) -> bool {
        if self.depth > MAX_DEPTH * 2 || self.death.contains(&t) || !self.active.insert(t) {
            return false;
        }
        let saved = (self.depth > 0).then(|| self.clone());
        self.depth += 1;
        let ok = self.need_tag_inner(t, why);
        self.depth -= 1;
        self.active.remove(&t);
        if !ok && let Some(saved) = saved {
            *self = saved;
            self.active.remove(&t);
        }
        ok
    }

    fn need_tag_inner(&mut self, t: i32, why: &str) -> bool {
        let s = self.s;
        let w = &s.level.world;
        let list = &s.objects.list;
        // Direct ones: (target, the region it is used from).
        let mut direct: Vec<(Target, usize)> = Vec::new();
        for sw in s.effects.switches().iter().filter(|sw| sw.usable && sw.tags.contains(&t) && self.pressable.contains(&sw.wall)) {
            let x = &w.walls[sw.wall];
            for r in std::iter::once(x.front).chain(x.back) {
                direct.push((Target::Wall(sw.wall), r));
            }
        }
        for (r, tags) in s.effects.triggers() {
            if tags.contains(&t) {
                direct.push((Target::Region(r), r));
            }
        }
        for (i, o) in list.iter().enumerate() {
            let fires = match o.effect {
                OTAGSW | OBUMPSW => o.tag == t,
                OWALK => o.mover.is_none() && o.tag == t,
                OPICKUP => o.aux.as_ref().is_some_and(|a| a.ints[0] == t),
                _ => false,
            } || self.morphed.get(&i).is_some_and(|m| m.1 == t);
            // (Some placed objects have no region recorded: the one at their spot.)
            let at = o.region.or_else(|| (o.effect != OPICKUP).then(|| w.region_at_z(o.x, o.y, Some(o.z + 1.0))).flatten());
            if fires && !o.hidden && !o.destroyed && let Some(r) = at {
                direct.push((Target::Object(i), r));
                // (Walkers and tag switches answer shots: usable from the regions beside theirs,
                // LIPS00's walker #243 behind its bars, shot for the spray can.)
                if matches!(o.effect, OWALK | OTAGSW) {
                    for &wi in &w.regions[r].walls {
                        let x = &w.walls[wi];
                        for q in std::iter::once(x.front).chain(x.back).filter(|&q| q != r) {
                            direct.push((Target::Object(i), q));
                        }
                    }
                }
            }
        }
        // (A walker answers every shot: LIPS00's #243, each time a way back outside.)
        direct.retain(|(target, _)| !self.used.contains(target) || matches!(target, Target::Object(i) if list[*i].effect == OWALK));
        // (The first step only from what can be used now: LIPS00's 1841, the front door's face
        // from r24, is no goal of the bot's while BRONZE opens it.)
        if self.steps.is_empty() && let Some(ok) = self.first {
            let now = self.reachable(false).1;
            direct.retain(|(target, r)| ok.contains(target) || !now.contains(r));
        }
        if debugging() {
            println!("       seq: tag {t}: direct {:?}", direct.iter().map(|d| d.0).collect::<Vec<_>>());
        }
        // Reachable now first; else the first one a way can be opened to.
        let now = self.reachable(false).1;
        if self.seed != 0 {
            self.shuffle(&mut direct);
        }
        direct.sort_by_key(|(_, r)| !now.contains(r));
        for (target, r) in direct {
            if self.reach_region(r) && self.clear_bars(target) && self.reach_region(r) {
                self.use_target(target, r, why);
                return true;
            }
        }
        // A template pickup that fires it: morph a placeholder into it, then take that.
        for o in list.iter().filter(|o| o.effect == OPICKUP && o.region.is_none() && o.aux.as_ref().is_some_and(|a| a.ints[0] == t)) {
            for m in list.iter().filter(|m| m.effect == OOTOO && m.tag > 0) {
                let Some(a) = &m.aux else { continue };
                let (Some(template), Some(placeholder)) = (a.slot(8), a.slot(9)) else { continue };
                if !template.eq_ignore_ascii_case(&o.name) {
                    continue;
                }
                if debugging() {
                    println!("       seq: tag {t}: template {template} from {placeholder} needs tag {}", m.tag);
                }
                if !self.need_tag(m.tag, &format!("turns {placeholder} into {template}")) {
                    continue;
                }
                let made: Vec<usize> = self.morphed.iter().filter(|(_, v)| v.0.eq_ignore_ascii_case(template)).map(|(&k, _)| k).collect();
                for i in made {
                    if let Some(r) = list[i].region && self.reach_region(r) && self.clear_bars(Target::Object(i)) && self.reach_region(r) {
                        self.use_target(Target::Object(i), r, why);
                        return true;
                    }
                }
            }
        }
        // A lock that fires it: bring its item, then fire the lock's own tag.
        for o in list.iter().filter(|o| o.effect == ODROP && o.aux.as_ref().is_some_and(|a| a.ints[0] == t)) {
            let Some(item) = o.aux.as_ref().and_then(|a| a.slot(9)).map(str::to_ascii_uppercase) else { continue };
            if !self.carried.contains(&item) && !self.get_item(&item) {
                continue;
            }
            if self.need_tag(o.tag, &format!("a lock takes {item} ({why})")) {
                return true;
            }
        }
        false
    }

    /// Lower what stands raised around an object first (LIPS00's #559 inside five bars,
    /// BAR1DOWN..BAR5DOWN, lowered by tag 121 once ELIXIR02 is set down in r462).
    fn clear_bars(&mut self, target: Target) -> bool {
        let Target::Object(i) = target else { return true };
        let s = self.s;
        let o = &s.objects.list[i];
        let Some(home) = o.region else { return true };
        let floor = self.fc(home).0;
        let w = &s.level.world;
        let mut tags: Vec<i32> = Vec::new();
        for (&r, list) in &self.movers {
            let ((x0, y0), (x1, y1)) = w.regions[r].bounds;
            let near = (x0 - o.x).max(o.x - x1).max(0.0).hypot((y0 - o.y).max(o.y - y1).max(0.0)) < 64.0;
            if !near || r == home || self.fc(r).0 <= floor + 24.0 {
                continue;
            }
            for m in list.iter().filter(|m| !m.1 && !m.3 && m.2 <= floor + 24.0) {
                if !tags.contains(&m.0) {
                    tags.push(m.0);
                }
            }
        }
        for t in tags {
            if self.fc(home).0 + 24.0 >= self.movers.iter().filter(|(_, l)| l.iter().any(|m| m.0 == t)).map(|(r, _)| self.fc(*r).0).fold(f32::MIN, f32::max) {
                continue;
            }
            if !self.need_tag(t, &format!("lowers what stands around {target:?}")) {
                return false;
            }
        }
        true
    }

    /// Pick up item `name` (one lying in the world, or one a morph makes).
    fn get_item(&mut self, name: &str) -> bool {
        let list = &self.s.objects.list;
        for (i, o) in list.iter().enumerate() {
            if o.effect == OPICKUP && o.name.eq_ignore_ascii_case(name) && !o.hidden && !o.destroyed && !self.used.contains(&Target::Object(i))
                && let Some(r) = o.region
                && self.reach_region(r)
                && self.clear_bars(Target::Object(i))
                && self.reach_region(r)
            {
                self.use_target(Target::Object(i), r, &format!("take {name}"));
                return true;
            }
        }
        false
    }

    /// Record a step and what it does: tags fired (doors toggle, morphs), items taken.
    /// `at`: the region it is used from, where the player is afterwards.
    fn use_target(&mut self, target: Target, at: usize, why: &str) {
        let s = self.s;
        self.start = at;
        let list = &s.objects.list;
        self.used.insert(target);
        self.steps.push(Step { target, why: why.to_string(), at });
        if self.steps.len() > self.longest.borrow().len() {
            *self.longest.borrow_mut() = self.steps.clone();
        }
        let tags: Vec<i32> = match target {
            Target::Wall(wi) => s.effects.switches().iter().find(|sw| sw.wall == wi).map(|sw| sw.tags.clone().collect()).unwrap_or_default(),
            // (Entering, landing, then leaving; not a leave that moves a floor back: LIPS00's
            // riser r57 goes down again when left.)
            Target::Region(r) => s.effects.triggers().iter().find(|t| t.0 == r).map(|t| {
                let moves = |x: i32| self.movers.values().any(|l| l.iter().any(|m| m.0 == x));
                [t.1[0], t.1[2], t.1[1]].into_iter().enumerate().filter(|&(k, x)| x > 0 && (k < 2 || !moves(x))).map(|(_, x)| x).collect()
            }).unwrap_or_default(),
            Target::Object(i) => {
                let o = &list[i];
                if o.effect == OPICKUP || self.morphed.contains_key(&i) {
                    self.carried.push(self.morphed.get(&i).map_or_else(|| o.name.to_ascii_uppercase(), |m| m.0.to_ascii_uppercase()));
                    vec![self.morphed.get(&i).map_or_else(|| o.aux.as_ref().map_or(0, |a| a.ints[0]), |m| m.1)]
                } else {
                    vec![o.tag]
                }
            }
            _ => Vec::new(),
        };
        // (Timed doors passed on the way have shut again by now.)
        for &q in &self.timed {
            self.open[q] = false;
        }
        for t in tags {
            self.fire(t);
        }
    }

    fn fire(&mut self, t: i32) {
        if t <= 0 {
            return;
        }
        let list = &self.s.objects.list;
        // (A teleporter the tag fires takes the player along: LIPS00's walker #243, shot for
        // the spray can, fires 71, whose #555 sends the player out to CHEIRAFT, the way back
        // outside once BRONZE's trip is spent.)
        if !self.death.contains(&t) {
            let w = &self.s.level.world;
            for o in list.iter().filter(|o| o.effect == OTELE && o.tag == t) {
                let dest = o.aux.as_ref().and_then(|a| a.slot(9)).and_then(|n| list.iter().find(|d| d.name.eq_ignore_ascii_case(n)));
                if let Some(d) = dest && let Some(r) = d.region.or_else(|| w.region_at_z(d.x, d.y, Some(d.z + 1.0))) {
                    self.start = r;
                }
            }
        }
        for (&r, &tag) in &self.doors {
            if tag == t {
                self.open[r] = !self.open[r];
            }
        }
        self.heights = self.moved(t);
        // (Wall copies: a wall takes another's blocking.)
        let w = &self.s.level.world;
        for o in list.iter().filter(|o| o.effect == OWTOW && o.tag == t) {
            let Some(a) = &o.aux else { continue };
            if let (Some(to), Some(from)) = (a.slot(9).and_then(|n| self.s.wall_named(n)), a.slot(8).and_then(|n| self.s.wall_named(n))) {
                let b = self.blocking.get(&from).copied().unwrap_or(w.walls[from].blocking);
                self.blocking.insert(to, b);
            }
        }
        for m in list.iter().filter(|m| m.effect == OOTOO && m.tag == t) {
            let Some(a) = &m.aux else { continue };
            let (Some(template), Some(placeholder)) = (a.slot(8), a.slot(9)) else { continue };
            let Some(tpl) = list.iter().find(|o| o.name.eq_ignore_ascii_case(template)) else { continue };
            let fires = tpl.aux.as_ref().map_or(0, |a| a.ints[0]);
            for (i, o) in list.iter().enumerate() {
                if o.name.eq_ignore_ascii_case(placeholder) && o.region.is_some() {
                    self.morphed.insert(i, (template.to_string(), fires));
                }
            }
        }
        // A lock fired with its item carried takes it and fires its own tag (LIPS00's #96:
        // wall 1725's tag 118 with KEY2 fires 117, the door r233).
        let locks: Vec<(String, i32)> = list.iter().filter(|o| o.effect == ODROP && o.tag == t).filter_map(|o| {
            let a = o.aux.as_ref()?;
            Some((a.slot(9)?.to_ascii_uppercase(), a.ints[0]))
        }).collect();
        for (item, then) in locks {
            if let Some(k) = self.carried.iter().position(|c| *c == item) {
                self.carried.remove(k);
                if then != t {
                    self.fire(then);
                }
            }
        }
    }
}
