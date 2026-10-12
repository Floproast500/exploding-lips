//! The BSP world (re/engine/bsp-world.md): a Quake-1 map the player is moved into from a sector
//! level and back. In the shipped game only LIPS00 has one (`LIPS00C.BSP`): its start-room
//! switch sends the player there (ORPOLY) and an exit cube (ORORB "COFFIN1") brings them back.
//!
//! The original runs BSP physics once per drawn frame (≤ 30 fps) with a fixed step of 5 "ticks";
//! the per-frame numbers below come from that, and this module steps it at 30 Hz.

use crate::world::Image;
use quake_bsp::Bsp;
use quake_bsp::collision::{Hulls, SOLID};

/// One original BSP frame.
pub const FRAME_SECONDS: f32 = 1.0 / 30.0;
/// Ticks per frame (the fixed `dt`).
const DT: f32 = 5.0;
const ACCEL: f32 = 0.6;
const MAX_SPEED: f32 = 1.5;
const FRICTION: f32 = 0.3;
const JUMP: f32 = 3.5;
const GRAVITY: f32 = 0.07;
const TURN_RATE: f32 = 0.039_269_9;
const TURN_DECAY: f32 = 0.019_635;
/// Clip hull of the 32x32x56 player box (origin 24 above the feet).
const HULL: usize = 1;
/// Ground snap and step-up limits of ClipMove.
const SNAP: f32 = 2.0;
const STEP: f32 = 20.0;
/// UpDist looks this far up for open space.
const UP_RANGE: f32 = 100.0;
/// Footstep (tag 996) distance per axis, and a fall that hurts.
const FOOTSTEP: f32 = 100.0;
const HARD_FALL: f32 = 100.0;
const TAU: f32 = std::f32::consts::TAU;

/// Tags the BSP code fires (orb objects listen).
pub mod tags {
    pub const JUMP: u32 = 940;
    pub const CROUCH: u32 = 941;
    pub const FIRE: u32 = 943;
    pub const FOOTSTEP: u32 = 996;
    pub const LANDED: u32 = 997;
}

/// An axis-aligned cube in BSP space owned by an orb object (ORORB exits, OPTAGSW switches).
#[derive(Debug, Clone)]
pub struct Cube {
    pub centre: [f32; 3],
    pub half: f32,
    pub object: usize,
    pub tag: i32,
    /// The player was inside last frame (fires the tag once per entry).
    inside: bool,
}

impl Cube {
    pub fn new(ints: [i32; 4], object: usize, tag: i32) -> Self {
        let half = if ints[3] < 1 { 20.0 } else { ints[3] as f32 };
        Self { centre: [ints[0] as f32, ints[1] as f32, ints[2] as f32], half, object, tag, inside: false }
    }

    /// Inside or on the cube's surface.
    fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|k| (p[k] - self.centre[k]).abs() <= self.half)
    }
}

/// The BSP player (entity type 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BspPlayer {
    /// Hull-1 origin (24 above the feet); the eye is here too.
    pub pos: [f32; 3],
    /// Velocity in the player's frame: right, forward, up (units per tick).
    vel: [f32; 3],
    /// Radians; the forward direction is (−sin yaw, cos yaw).
    pub yaw: f32,
    yaw_rate: f32,
    /// Radians, positive up.
    pub pitch: f32,
    pub airborne: bool,
    fall_start: Option<f32>,
    step_from: Option<(f32, f32)>,
    jumped: bool,
    jump_held: bool,
}

impl BspPlayer {
    fn at(pos: [f32; 3], yaw: f32) -> Self {
        Self { pos, vel: [0.0; 3], yaw, yaw_rate: 0.0, pitch: 0.0, airborne: false, fall_start: None, step_from: None, jumped: false, jump_held: false }
    }

    /// The facing as an orb angle (radians, counter-clockwise from +X).
    pub fn orb_angle(&self) -> f32 {
        (self.yaw - 1.5 * std::f32::consts::PI).rem_euclid(TAU)
    }
}

/// What a BSP frame asks the orb to do.
#[derive(Debug, Clone, PartialEq)]
pub enum BspEvent {
    Tag(u32),
    /// Back to the orb onto this object (ORORB).
    Exit(usize),
}

/// Movement wanted for one frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct BspInput {
    pub forward: f32,
    pub strafe: f32,
    pub turn: f32,
    pub jump: bool,
    pub fire: bool,
}

pub struct BspWorld {
    pub bsp: Bsp,
    pub player: BspPlayer,
    exits: Vec<Cube>,
    switches: Vec<Cube>,
    /// Game time not yet stepped (seconds).
    pending: f32,
    /// GODDPOLY.PAL colours the textures are indexed into.
    pub palette: [[u8; 3]; 256],
    /// Textures re-indexed into the orb palette (built on first use).
    images: Option<Vec<Image>>,
    /// The world's triangles (static).
    pub scene: engine_scene::scene::SceneFrame,
    /// Planned way to the exit (guides), from where it was planned.
    route: Option<Vec<[f32; 3]>>,
    /// GODDPOLY's light tables (for [`relight_levels`]).
    pub colormap: Vec<[u8; 256]>,
    relit: bool,
}

impl BspWorld {
    /// `exits` / `switches`: ORORB and OPTAGSW cubes of the level (rebuilt per level load).
    pub fn new(bsp: Bsp, palette: [[u8; 3]; 256], exits: Vec<Cube>, mut switches: Vec<Cube>) -> Self {
        // Every ORORB is also a tag switch with the same cube.
        switches.extend(exits.iter().cloned());
        let (start, angle) = bsp.player_start();
        let player = BspPlayer::at([start[0], start[1], start[2] + 0.01], angle.to_radians());
        let scene = quake_bsp::viewer::build_scene(&bsp);
        Self { bsp, player, exits, switches, pending: 0.0, palette, images: None, scene, route: None, colormap: Vec::new(), relit: false }
    }

    pub fn exits(&self) -> &[Cube] {
        &self.exits
    }

    /// ORPOLY: put the player at a BSP position with an orb heading (2048 units per turn). Speed
    /// and airborne state are kept, as in the original.
    pub fn place(&mut self, pos: [i32; 3], heading: i32) {
        self.player.pos = [pos[0] as f32, pos[1] as f32, pos[2] as f32 + 0.1];
        let mut yaw = heading as f32 / 2048.0 * TAU + 1.5 * std::f32::consts::PI;
        if yaw > TAU {
            yaw -= TAU;
        }
        if yaw < -TAU {
            yaw += TAU;
        }
        self.player.yaw = yaw;
        for c in &mut self.switches {
            c.inside = false;
        }
    }

    /// Mouse/stick look in radians (positive yaw turns left, positive pitch looks up); applied
    /// at once, like the original's mouse.
    pub fn look(&mut self, yaw: f32, pitch: f32) {
        let p = &mut self.player;
        p.yaw = (p.yaw + yaw) % TAU;
        p.pitch = (p.pitch + pitch).clamp(-std::f32::consts::FRAC_PI_2 + 0.01, std::f32::consts::FRAC_PI_2 - 0.01);
    }

    /// Advance `seconds` of game time in whole original frames; events in order.
    pub fn tick(&mut self, seconds: f32, input: &BspInput) -> Vec<BspEvent> {
        self.pending += seconds;
        let mut events = Vec::new();
        while self.pending >= FRAME_SECONDS {
            self.pending -= FRAME_SECONDS;
            self.frame(input, &mut events);
            if events.iter().any(|e| matches!(e, BspEvent::Exit(_))) {
                self.pending = 0.0;
                break;
            }
        }
        events
    }

    /// One original frame: input, friction, gravity, integration, ClipMove, landing,
    /// footsteps, then the tag-switch and exit tests.
    fn frame(&mut self, input: &BspInput, events: &mut Vec<BspEvent>) {
        let p = &mut self.player;
        // Input (accelerate, turn, jump).
        if input.forward != 0.0 {
            p.vel[1] += input.forward.signum() * DT * ACCEL;
        }
        if input.strafe != 0.0 {
            p.vel[0] += input.strafe.signum() * DT * ACCEL;
        }
        if input.turn != 0.0 {
            p.yaw_rate = input.turn.signum() * TURN_RATE;
        }
        if input.jump {
            if !p.jump_held {
                events.push(BspEvent::Tag(tags::JUMP));
                p.jump_held = true;
            }
            if !p.airborne {
                p.vel[2] = JUMP;
                p.jumped = true;
            }
        } else {
            p.jump_held = false;
        }
        if input.fire {
            events.push(BspEvent::Tag(tags::FIRE));
        }
        // Friction (x/y only) and the turn decay.
        for v in &mut p.vel[..2] {
            if *v > 0.0 {
                *v = (*v - DT * FRICTION).clamp(0.0, MAX_SPEED);
            } else if *v < 0.0 {
                *v = (*v + DT * FRICTION).clamp(-MAX_SPEED, 0.0);
            }
        }
        p.yaw_rate = if p.yaw_rate > 0.0 { (p.yaw_rate - TURN_DECAY).max(0.0) } else { (p.yaw_rate + TURN_DECAY).min(0.0) };
        if p.airborne {
            p.vel[2] -= DT * GRAVITY;
        }
        // Integrate in the player's frame.
        let old = p.pos;
        let (s, c) = p.yaw.sin_cos();
        let target = [old[0] + (c * p.vel[0] - s * p.vel[1]) * DT, old[1] + (s * p.vel[0] + c * p.vel[1]) * DT, old[2] + p.vel[2] * DT];
        p.yaw = (p.yaw + p.yaw_rate * DT) % TAU;
        let (pos, airborne) = clip_move(&self.bsp.hulls, old, target);
        p.pos = pos;
        // Landing, falls and the landing tag.
        if airborne {
            if !p.airborne {
                p.fall_start = Some(p.pos[2]);
            }
            p.airborne = true;
        } else if p.airborne {
            p.airborne = false;
            p.vel[2] = 0.0;
            if p.jumped {
                p.jumped = false;
                events.push(BspEvent::Tag(tags::LANDED));
            } else if p.fall_start.is_some_and(|z| z - p.pos[2] > HARD_FALL) {
                events.push(BspEvent::Tag(tags::LANDED));
            }
            p.fall_start = None;
        }
        // Footsteps.
        if p.vel[0] == 0.0 && p.vel[1] == 0.0 {
            p.step_from.get_or_insert((p.pos[0], p.pos[1]));
        } else if !p.airborne {
            let from = *p.step_from.get_or_insert((p.pos[0], p.pos[1]));
            if (p.pos[0] - from.0).abs() > FOOTSTEP || (p.pos[1] - from.1).abs() > FOOTSTEP {
                events.push(BspEvent::Tag(tags::FOOTSTEP));
                p.step_from = Some((p.pos[0], p.pos[1]));
            }
        }
        // Tag switches: at most one fires per frame.
        let pos = self.player.pos;
        for c in &mut self.switches {
            if !c.contains(pos) {
                c.inside = false;
            } else if !c.inside {
                c.inside = true;
                if c.tag > 0 {
                    events.push(BspEvent::Tag(c.tag as u32));
                }
                break;
            }
        }
        if let Some(exit) = self.exits.iter().find(|c| c.contains(pos)) {
            events.push(BspEvent::Exit(exit.object));
        }
    }

    /// The next few spots of the way to the exit (planned once per stay, then followed from
    /// the nearest point), for guide lights.
    pub fn route_ahead(&mut self, count: usize) -> Vec<[f32; 3]> {
        if self.route.is_none() {
            self.route = Some(self.path_to_exit().unwrap_or_default());
        }
        let route = self.route.as_deref().unwrap_or_default();
        let p = self.player.pos;
        let dist = |q: &[f32; 3]| (q[0] - p[0]).hypot(q[1] - p[1]) + (q[2] - p[2]).abs();
        let Some(nearest) = (0..route.len()).min_by(|&a, &b| dist(&route[a]).total_cmp(&dist(&route[b]))) else { return Vec::new() };
        // Beacons about 130 units apart, starting about 200 units ahead, at knee height.
        route[nearest..].iter().skip(12).step_by(8).take(count).map(|p| [p[0], p[1], p[2] - 12.0]).collect()
    }

    /// Forget the planned route (a new stay starts somewhere else).
    pub fn reset_route(&mut self) {
        self.route = None;
    }

    /// The textures in the orb palette, once built by [`BspWorld::images`].
    pub fn cached_images(&self) -> &[Image] {
        self.images.as_deref().unwrap_or_default()
    }

    /// The images shown in the BSP world: its textures in the orb palette (each GODDPOLY index
    /// becomes the nearest orb colour), then the level's own images (weapon and HUD sprites keep
    /// their pictures, at index + [`BspWorld::texture_count`]).
    pub fn images(&mut self, orb: &[[u8; 3]; 256], level_images: &[Image]) -> &[Image] {
        if self.images.is_none() {
            let map: Vec<u8> = self.palette.iter().map(|&c| crate::text::nearest_color(orb, c)).collect();
            let textures = self.bsp.textures.iter().map(|t| Image { width: t.width, height: t.height, pixels: t.pixels.iter().map(|&i| map[i as usize]).collect() });
            self.images = Some(textures.chain(level_images.iter().cloned()).collect());
        }
        self.images.as_deref().unwrap_or_default()
    }

    /// Re-express the scene's light levels in the orb's shade tables, matching the original's
    /// BSP brightness (once).
    pub fn relight(&mut self, map: impl Fn(f32) -> f32) {
        if self.relit {
            return;
        }
        for v in &mut self.scene.vertices {
            v.light = map(v.light);
        }
        self.relit = true;
    }

    /// Images before the level's own in [`BspWorld::images`].
    pub fn texture_count(&self) -> usize {
        self.bsp.textures.len()
    }
}

/// The BSP world's palette: GODDPOLY.PAL, 768 bytes of 6-bit RGB (grey ramp when missing).
pub fn goddpoly_palette(vfs: &crate::Vfs) -> [[u8; 3]; 256] {
    let pal = vfs.read("GODDPOLY.PAL").ok().filter(|p| p.len() >= 768);
    std::array::from_fn(|i| match pal {
        Some(p) => [p[i * 3], p[i * 3 + 1], p[i * 3 + 2]].map(|v| ((v.min(63) as u16 * 255) / 63) as u8),
        None => [i as u8; 3],
    })
}

/// GODDPOLY.PAL's light tables: 32 rows of 256 (after the palette and two count bytes), row 0
/// brightest.
pub fn goddpoly_colormap(vfs: &crate::Vfs) -> Vec<[u8; 256]> {
    let Ok(d) = vfs.read("GODDPOLY.PAL") else { return Vec::new() };
    d.get(770..).unwrap_or_default().as_chunks::<256>().0.iter().take(32).copied().collect()
}

/// Shade levels as the original lights the BSP: `light` (0 bright .. 15 dark, from the
/// lightmap) picks a GODDPOLY colormap row; return the orb shade level (fractional, as in
/// `orb_rows`) that is about as bright, comparing average luminance over all colours.
pub fn relight_levels(god_pal: &[[u8; 3]; 256], god_rows: &[[u8; 256]], orb_pal: &[[u8; 3]; 256], orb_rows: &[[u8; 256]]) -> impl Fn(f32) -> f32 {
    let lum = |c: [u8; 3]| 0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32;
    let avg = |pal: &[[u8; 3]; 256], row: &[u8; 256]| row.iter().map(|&i| lum(pal[i as usize])).sum::<f32>() / 256.0;
    let god: Vec<f32> = god_rows.iter().map(|r| avg(god_pal, r)).collect();
    let orb: Vec<f32> = orb_rows.iter().map(|r| avg(orb_pal, r)).collect();
    let (g0, o0) = (god.first().copied().unwrap_or(1.0).max(1e-3), orb.first().copied().unwrap_or(1.0).max(1e-3));
    move |light: f32| {
        if god.len() < 2 || orb.len() < 2 {
            return light;
        }
        let row = (light / 15.0 * (god.len() - 1) as f32).clamp(0.0, (god.len() - 1) as f32);
        let (r0, t) = (row.floor() as usize, row.fract());
        let want = (god[r0] + (god[(r0 + 1).min(god.len() - 1)] - god[r0]) * t) / g0;
        // First orb level at or below the wanted brightness, interpolated.
        for l in 0..orb.len() - 1 {
            let (a, b) = (orb[l] / o0, orb[l + 1] / o0);
            if b <= want {
                let f = if (a - b).abs() > 1e-6 { ((a - want) / (a - b)).clamp(0.0, 1.0) } else { 0.0 };
                return (l as f32 + f).min(15.0);
            }
        }
        (orb.len() - 1) as f32
    }
}

/// The BSP file a level loads (`[startup] loadbspfilename` in its INI).
pub fn level_bsp_path(level_ini: &str) -> Option<String> {
    let ini = godd_formats::ini::Ini::parse(level_ini);
    let file = ini.section("startup")?.get("loadbspfilename")?.to_string();
    Some(if file.contains('.') { file } else { format!("{file}.BSP") })
}

/// A level's BSP textures as shown in play (in the orb palette), with their names: for texture
/// packs. Empty when the level has no BSP world.
pub fn level_textures(vfs: &crate::Vfs, level: &str, orb: &[[u8; 3]; 256]) -> Vec<(String, Image)> {
    let ini = vfs.read(&format!("{level}.INI")).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    let Some(path) = level_bsp_path(&ini) else { return Vec::new() };
    let Some(bsp) = vfs.read(&path).ok().and_then(|d| Bsp::parse(d).ok()) else { return Vec::new() };
    let mut world = BspWorld::new(bsp, goddpoly_palette(vfs), Vec::new(), Vec::new());
    let count = world.texture_count();
    let names: Vec<String> = world.bsp.textures.iter().map(|t| t.name.clone()).collect();
    names.into_iter().zip(world.images(orb, &[])[..count].iter().cloned()).collect()
}

/// Grid spacing of the walk planner.
const PLAN_STEP: f32 = 16.0;

impl BspWorld {
    /// A walkable path from the player's position to any exit cube, through standing spots
    /// 16 units apart. Each step is the player's own ClipMove (so step-ups, slopes and drops
    /// behave as in play); falls land where a downward sweep lands. `None` when unreachable.
    pub fn path_to_exit(&self) -> Option<Vec<[f32; 3]>> {
        let h = &self.bsp.hulls;
        let start = settle(h, self.player.pos)?;
        let key = |p: [f32; 3]| ((p[0] / PLAN_STEP).round() as i32, (p[1] / PLAN_STEP).round() as i32, (p[2] / 8.0).round() as i32);
        // Grid cell → (standing spot, the cell it was reached from).
        type Cell = (i32, i32, i32);
        let mut came: std::collections::HashMap<Cell, ([f32; 3], Option<Cell>)> = std::collections::HashMap::new();
        let mut queue = std::collections::VecDeque::from([start]);
        came.insert(key(start), (start, None));
        while let Some(p) = queue.pop_front() {
            if self.exits.iter().any(|c| c.contains(p)) {
                let mut path = vec![p];
                let mut k = came[&key(p)].1;
                while let Some(prev) = k {
                    let (pos, before) = came[&prev];
                    path.push(pos);
                    k = before;
                }
                path.reverse();
                return Some(path);
            }
            if came.len() > 400_000 {
                return None;
            }
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                // Two half steps, like the player's per-frame moves.
                let mut q = p;
                for _ in 0..2 {
                    q = clip_move(h, q, [q[0] + dx * PLAN_STEP / 2.0, q[1] + dy * PLAN_STEP / 2.0, q[2]]).0;
                }
                if (q[0] - p[0]).hypot(q[1] - p[1]) < PLAN_STEP * 0.6 {
                    continue; // blocked
                }
                let Some(q) = settle(h, q) else { continue };
                let k = key(q);
                if let std::collections::hash_map::Entry::Vacant(e) = came.entry(k) {
                    e.insert((q, Some(key(p))));
                    queue.push_back(q);
                }
            }
        }
        None
    }
}

/// Where a player at `p` comes to rest: on the floor below (within a long fall).
fn settle(h: &Hulls, p: [f32; 3]) -> Option<[f32; 3]> {
    if h.contents(HULL, p) == SOLID {
        return None;
    }
    let t = h.trace(HULL, p, [p[0], p[1], p[2] - 2048.0]);
    (t.fraction < 1.0 && t.normal[2] > 0.5).then_some([t.end[0], t.end[1], t.end[2] + 0.01])
}

/// Distance up from `p` to open space (0 when `p` is open, `UP_RANGE` when solid all the way).
fn up_dist(h: &Hulls, p: [f32; 3]) -> f32 {
    if h.contents(HULL, p) != SOLID {
        return 0.0;
    }
    let at = |dz: f32| h.contents(HULL, [p[0], p[1], p[2] + dz]) != SOLID;
    let Some(k) = (1..=UP_RANGE as i32).find(|&k| at(k as f32)) else { return UP_RANGE };
    // Refine between the last solid and the first open sample.
    let (mut lo, mut hi) = ((k - 1) as f32, k as f32);
    for _ in 0..8 {
        let mid = (lo + hi) / 2.0;
        if at(mid) { hi = mid } else { lo = mid }
    }
    hi
}

/// The ground probe of ClipMove: how far `target` sits in the floor (0 = in the air).
fn ground(h: &Hulls, target: [f32; 3]) -> f32 {
    up_dist(h, [target[0], target[1], target[2] - SNAP])
}

/// ClipMove: step up small ledges, sweep `pos` → `target`, and slide once along what is hit.
/// Returns the new position and whether the player is airborne.
pub fn clip_move(h: &Hulls, pos: [f32; 3], target: [f32; 3]) -> ([f32; 3], bool) {
    let (end, airborne) = clip_move_unchecked(h, pos, target);
    // Never end inside solid (a step-up under a low ceiling, a slide into a corner): keep the
    // last open position instead, so nothing can tunnel through floors.
    if h.contents(HULL, end) == SOLID && h.contents(HULL, pos) != SOLID {
        let still = ground(h, pos) == 0.0;
        return (pos, still);
    }
    (end, airborne)
}

fn clip_move_unchecked(h: &Hulls, mut pos: [f32; 3], mut target: [f32; 3]) -> ([f32; 3], bool) {
    let up = ground(h, target);
    let airborne = up == 0.0;
    if up > SNAP && up < STEP {
        let lift = up - SNAP + 0.01;
        pos[2] += lift;
        target[2] += lift;
    }
    let t = h.trace(HULL, pos, target);
    if t.fraction >= 1.0 {
        return (target, airborne);
    }
    if t.start_solid {
        // Raised into solid by the step-up: no move this frame.
        return (pos, airborne);
    }
    // Back off from the impact, then put the target just in front of the plane hit.
    let v = [target[0] - pos[0], target[1] - pos[1], target[2] - pos[2]];
    let backed = [t.end[0] - 0.01 * v[0], t.end[1] - 0.01 * v[1], t.end[2] - 0.01 * v[2]];
    let n = t.normal;
    let dot = |a: [f32; 3]| a[0] * n[0] + a[1] * n[1] + a[2] * n[2];
    let plane = dot(t.end) - 0.031_25;
    let d = dot(target) - plane - 0.01;
    let mut slid = [target[0] - d * n[0], target[1] - d * n[1], target[2] - d * n[2]];
    let up = ground(h, slid);
    let airborne = up == 0.0;
    if up > SNAP && up < STEP {
        slid[2] += up - SNAP + 0.01;
    }
    if h.contents(HULL, slid) == SOLID { (backed, airborne) } else { (slid, airborne) }
}
