//! Player movement and collision against the region/wall world.
//!
//! Constants and the vertical model come from re/engine/physics.md. The original runs one
//! logic frame per ~33 ms (30 fps) with speeds in "units per 1.2 s"; this simulation runs at
//! [`crate::TICK_RATE`] and converts, so per-frame quantities are scaled by
//! `FRAMES_PER_TICK`.

use crate::input::{Action, Input};
use crate::world::World;

/// Original logic frames (30 fps) per simulation tick.
const FRAMES_PER_TICK: f32 = 30.0 / crate::TICK_RATE as f32;
/// Original speeds are units per 1.2 s.
const SPEED_UNIT_SECONDS: f32 = 1.2;
/// Angle units per turn in the original.
const ANGLE_UNITS: f32 = 2048.0;

#[derive(Debug, Clone, Copy)]
pub struct PhysicsParams {
    /// Units per 1.2 s (original default 800; running doubles it).
    pub walk_speed: f32,
    /// Angle units (2048/turn) per original frame (keyboard turning: 24).
    pub turn_per_frame: f32,
    /// Look shear units per frame (24) and limit (±512); horizon = (look+127)·H/256.
    pub look_per_frame: f32,
    pub look_limit: f32,
    /// OSETJUMP: launch speed (200) and fall gravity (200 → 50 per frame).
    pub jump_speed: f32,
    pub jump_fall_gravity: f32,
    /// Speed decrease per frame while rising (G scaled: 200 → 10 per frame).
    pub rise_decel_per_frame: f32,
    /// Fall speed (msz, 200): falling gains msz/4 per frame and starts at -msz at the apex.
    pub fall_speed: f32,
    pub radius: f32,
    /// Standing height; the eye is at the top.
    pub height: f32,
    pub crouch_eye: f32,
    /// Step-up/step-down height (32).
    pub step_height: f32,
    /// Clearance kept below ceilings (8).
    pub head_clearance: f32,
}

impl Default for PhysicsParams {
    fn default() -> Self {
        Self {
            walk_speed: 800.0,
            turn_per_frame: 24.0,
            look_per_frame: 24.0,
            look_limit: 512.0,
            jump_speed: 200.0,
            jump_fall_gravity: 200.0,
            rise_decel_per_frame: 10.0,
            fall_speed: 200.0,
            radius: 16.0,
            height: 48.0,
            crouch_eye: 12.0,
            step_height: 32.0,
            head_clearance: 8.0,
        }
    }
}

impl PhysicsParams {
    /// Map units moved per tick at `speed` (units per 1.2 s).
    fn per_tick(speed: f32) -> f32 {
        speed / SPEED_UNIT_SECONDS / crate::TICK_RATE as f32
    }

    /// Original height change per frame is vz / 40 at the default 30 fps.
    fn dz(vz: f32) -> f32 {
        vz / 40.0 * FRAMES_PER_TICK
    }

    /// Look shear (−limit..limit) as the renderer's pitch (fraction of the focal length) for
    /// the original 320x200 view: horizon offset = look·200/256 rows, focal 220.2.
    pub fn look_to_pitch(look: f32) -> f32 {
        look * 200.0 / 256.0 / 220.2
    }

    /// Look units for a pitch change of `radians` (through the shear).
    pub fn pitch_to_look(radians: f32) -> f32 {
        radians.tan() * 220.2 * 256.0 / 200.0
    }
}

/// Walls tracked in contact at once.
const MAX_CONTACTS: usize = 6;

#[derive(Debug, Clone, Copy)]
pub struct Player {
    pub x: f32,
    pub y: f32,
    /// Feet height.
    pub z: f32,
    /// Vertical speed in original units (per 1.2 s, applied as vz/40 per frame).
    pub vz: f32,
    pub angle: f32,
    /// Look shear in original units (−512..512).
    pub look: f32,
    pub region: usize,
    pub on_ground: bool,
    pub crouching: bool,
    jumping: bool,
    /// Flying (OFLYON): Jump rises, Crouch sinks, no gravity.
    pub flying: bool,
    /// Last wall touched; a wall fires once per continuous contact (world-effects.md §1.1).
    /// Walls in contact at the last update (each fires once per continuous contact).
    touching: [u32; MAX_CONTACTS],
    /// Regions re-homed into during the last tick, in order (physics.md §8.2: every crossing
    /// re-homes, and each re-home sends leave/enter, world-effects.md §1: a body bounced back
    /// out of a thin region within one tick still entered it).
    crossed: [u32; MAX_CROSSINGS],
    crossed_n: u8,
    /// Yaw (radians) and look (units) the last tick took from mouse/stick look. The camera
    /// shows that part immediately instead of interpolating it, so look stays 1:1.
    pub direct_turn: f32,
    pub direct_look: f32,
}

/// Contacts the player made during a tick, for wall/region effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEvent {
    /// Event 0x200: touched a wall (new contact).
    TouchedWall(usize),
    /// Event 0x1000 / 0x2000.
    EnteredRegion(usize),
    LeftRegion(usize),
    /// Event 0x400: landed on this region's floor.
    Landed(usize),
    /// Event 0x800: head hit this region's ceiling.
    HitCeiling(usize),
}

/// Extra distance beyond the radius at which walls count as touched.
const CONTACT_MARGIN: f32 = 12.0;
/// Flying with no up/down input: sink speed (u per 1.2 s), physics.md 5.5.
const FLY_IDLE_SINK: f32 = 50.0;
/// Region changes recorded per tick (a tick moves at most a few sub-steps).
const MAX_CROSSINGS: usize = 8;

impl Player {
    pub fn new(world: &World, x: f32, y: f32, angle: f32) -> Self {
        let region = world.region_at(x, y).unwrap_or(0);
        Self::in_region(world, x, y, angle, region)
    }

    pub fn in_region(world: &World, x: f32, y: f32, angle: f32, region: usize) -> Self {
        let z = world.regions.get(region).map_or(0.0, |r| r.floor);
        Self { x, y, z, vz: 0.0, angle, look: 0.0, region, on_ground: true, crouching: false, jumping: false, flying: false, touching: [u32::MAX; MAX_CONTACTS], crossed: [0; MAX_CROSSINGS], crossed_n: 0, direct_turn: 0.0, direct_look: 0.0 }
    }

    pub fn eye_z(&self, params: &PhysicsParams) -> f32 {
        self.z + if self.crouching { params.crouch_eye } else { params.height }
    }

    pub fn pitch(&self) -> f32 {
        PhysicsParams::look_to_pitch(self.look)
    }

    /// Advance one tick; contacts with walls and regions are appended to `events`.
    pub fn tick_with_events(&mut self, world: &World, input: &Input, params: &PhysicsParams, events: &mut Vec<PlayerEvent>) {
        let region_before = self.region;
        let airborne_before = !self.on_ground;
        let vz_before = self.vz;
        self.tick(world, input, params);
        let mut at = region_before;
        for &r in &self.crossed[..self.crossed_n as usize] {
            let r = r as usize;
            if r != at {
                events.push(PlayerEvent::LeftRegion(at));
                events.push(PlayerEvent::EnteredRegion(r));
                at = r;
            }
        }
        // (Changes made outside the moves, or past the record's end.)
        if self.region != at {
            events.push(PlayerEvent::LeftRegion(at));
            events.push(PlayerEvent::EnteredRegion(self.region));
        }
        if airborne_before && self.on_ground {
            events.push(PlayerEvent::Landed(world.solid_floor(self.region).1));
        }
        if vz_before > 0.0 && self.vz <= 0.0 && !self.on_ground {
            let (ceiling, owner) = world.solid_ceiling(self.region);
            if self.z + params.height + params.head_clearance >= ceiling - 0.5 {
                events.push(PlayerEvent::HitCeiling(owner));
            }
        }
        // Wall contact (world-effects.md §1.1): every wall within radius + margin that overlaps
        // our height: blocking walls, closed portals and passable lines being approached. Each
        // fires once per continuous contact; a closed door face touched while standing on an
        // open doorway line is still touched.
        // Walls hold the body exactly radius + margin away, so allow a unit for rounding (the
        // original tests the attempted position, before the push-out).
        let reach = params.radius + CONTACT_MARGIN + 1.0;
        let height = if self.crouching { params.crouch_eye } else { params.height };
        let mut near: Vec<(f32, usize)> = Vec::new();
        for (wi, wall) in world.walls.iter().enumerate() {
            let (ax, ay) = world.vertices[wall.a];
            let (bx, by) = world.vertices[wall.b];
            if self.x < ax.min(bx) - reach || self.x > ax.max(bx) + reach || self.y < ay.min(by) - reach || self.y > ay.max(by) + reach {
                continue;
            }
            let overlaps = |r: usize| world.regions.get(r).is_some_and(|reg| self.z < reg.ceiling && self.z + height > reg.floor);
            if !overlaps(wall.front) && !wall.back.is_some_and(overlaps) {
                continue;
            }
            let (px, py) = closest_point(ax, ay, bx, by, self.x, self.y);
            let d = ((self.x - px).powi(2) + (self.y - py).powi(2)).sqrt();
            if d <= reach {
                near.push((d, wi));
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut now = [u32::MAX; MAX_CONTACTS];
        for (slot, &(_, wi)) in now.iter_mut().zip(&near) {
            *slot = wi as u32;
            if !self.touching.contains(&(wi as u32)) {
                events.push(PlayerEvent::TouchedWall(wi));
            }
        }
        self.touching = now;
    }

    pub fn tick(&mut self, world: &World, input: &Input, params: &PhysicsParams) {
        self.crossed_n = 0;
        let strafe_mod = input.held(Action::StrafeModifier);
        let mut turn = 0.0;
        let mut strafe = input.move_axis.0;
        let mut forward = input.move_axis.1;
        if input.held(Action::Forward) {
            forward += 1.0;
        }
        if input.held(Action::Back) {
            forward -= 1.0;
        }
        for (action, sign) in [(Action::TurnLeft, 1.0), (Action::TurnRight, -1.0)] {
            if input.held(action) {
                if strafe_mod { strafe -= sign } else { turn += sign }
            }
        }
        if input.held(Action::StrafeLeft) {
            strafe -= 1.0;
        }
        if input.held(Action::StrafeRight) {
            strafe += 1.0;
        }
        let turn_rad = params.turn_per_frame * FRAMES_PER_TICK / ANGLE_UNITS * std::f32::consts::TAU;
        self.angle = (self.angle + turn * turn_rad - input.look_delta.0).rem_euclid(std::f32::consts::TAU);

        let look_step = params.look_per_frame * FRAMES_PER_TICK;
        if input.held(Action::LookUp) {
            self.look += look_step;
        }
        if input.held(Action::LookDown) {
            self.look -= look_step;
        }
        if input.pressed(Action::CenterView) {
            self.look = 0.0;
        }
        // Mouse look delta is in radians of pitch; convert through the shear.
        let before = self.look.clamp(-params.look_limit, params.look_limit);
        self.look -= PhysicsParams::pitch_to_look(input.look_delta.1);
        self.look = self.look.clamp(-params.look_limit, params.look_limit);
        self.direct_look = self.look - before;
        self.direct_turn = -input.look_delta.0;

        // Released, the body only stands up where there is room: Height <= ceil - floor - 8
        // (physics.md §5.4, you stay crouched under low ceilings; else the eye pokes out above
        // the ceiling).
        let no_room = self.crouching && world.regions.get(self.region).is_some_and(|_| world.solid_ceiling(self.region).0 - self.z < params.height + params.head_clearance);
        self.crouching = (input.held(Action::Crouch) || no_room) && !self.flying;
        let speed = PhysicsParams::per_tick(params.walk_speed) * if input.held(Action::Run) { 2.0 } else { 1.0 };
        let len = (forward * forward + strafe * strafe).sqrt();
        if len > 1.0 {
            forward /= len;
            strafe /= len;
        }
        let (sin, cos) = self.angle.sin_cos();
        // Strafe right is clockwise from the facing direction.
        let dx = (cos * forward + sin * strafe) * speed;
        let dy = (sin * forward - cos * strafe) * speed;
        self.slide_move(world, params, dx, dy);
        self.vertical(world, input, params);
    }

    /// Vertical update (physics.md §5.2), per tick.
    fn vertical(&mut self, world: &World, input: &Input, params: &PhysicsParams) {
        if self.region >= world.regions.len() {
            return;
        }
        // Stacked regions: see-through floors/ceilings are not solid.
        let (floor, _) = world.solid_floor(self.region);
        let (ceiling, _) = world.solid_ceiling(self.region);

        if self.flying {
            // Fly speeds: up/down at the original fall speed scale, no gravity.
            let fly = params.fall_speed;
            // With neither held, a body above the floor sinks slowly (physics.md 5.5: idle sink
            // ext+0x20c, 50): a swimmer drifts down when Jump is let go.
            self.vz = if input.held(Action::Jump) {
                fly
            } else if input.held(Action::Crouch) {
                -fly
            } else if self.z - floor > 4.0 {
                -FLY_IDLE_SINK
            } else {
                0.0
            };
            self.z += PhysicsParams::dz(self.vz);
            let height = params.height;
            self.z = self.z.clamp(floor, (ceiling - height - params.head_clearance).max(floor));
            self.on_ground = (self.z - floor).abs() < 0.01;
            for _ in 0..8 {
                let r = &world.regions[self.region];
                match (r.below, r.above) {
                    (Some(b), _) if self.z < r.floor => self.region = b,
                    (_, Some(a)) if self.z >= r.ceiling => self.region = a,
                    _ => break,
                }
            }
            return;
        }
        if input.pressed(Action::Jump) && (self.z - floor).abs() < 0.01 {
            self.vz = params.jump_speed;
            self.jumping = true;
        }
        if self.vz > 0.0 {
            // Rising: decelerate; at the apex snap to the fall speed.
            self.vz -= params.rise_decel_per_frame * FRAMES_PER_TICK;
            if self.vz < 0.0 {
                self.vz = -params.fall_speed;
            }
        } else if self.z - floor > params.step_height {
            // Airborne below the apex.
            let gravity = if self.jumping { params.jump_fall_gravity } else { params.fall_speed };
            self.vz -= gravity / 4.0 * FRAMES_PER_TICK;
        } else if !self.jumping || self.vz == 0.0 {
            // Within step height of the floor: snap down (step-down).
            self.z = floor;
        }
        self.z += PhysicsParams::dz(self.vz);

        let height = if self.crouching { params.crouch_eye } else { params.height };
        if self.z + height + params.head_clearance > ceiling {
            self.z = (ceiling - height - params.head_clearance).max(floor);
            self.vz = self.vz.min(0.0);
        }
        if self.z <= floor {
            // Landing. (The generic object bounce in physics.md §5.3 does not apply to the
            // player: probes of the original show it at rest on the floor right after a fall.)
            self.z = floor;
            self.vz = 0.0;
            self.jumping = false;
        }
        self.on_ground = (self.z - floor).abs() < 0.01;
        // Move between stacked regions when crossing a see-through floor or ceiling.
        for _ in 0..8 {
            let r = &world.regions[self.region];
            match (r.below, r.above) {
                (Some(b), _) if self.z < r.floor => self.region = b,
                (_, Some(a)) if self.z >= r.ceiling => self.region = a,
                _ => break,
            }
        }
    }

    /// Move, sliding along blocking walls; split into small steps to avoid tunnelling.
    fn slide_move(&mut self, world: &World, params: &PhysicsParams, dx: f32, dy: f32) {
        let dist = (dx * dx + dy * dy).sqrt();
        if dist == 0.0 {
            return;
        }
        let steps = (dist / (params.radius * 0.5)).ceil().max(1.0) as usize;
        let (sx, sy) = (dx / steps as f32, dy / steps as f32);
        for _ in 0..steps {
            let (mut nx, mut ny) = (self.x + sx, self.y + sy);
            // Push out of every blocking wall to radius + 12 (physics.md §8.1: solid walls hold
            // the body that far away; two passes settle corners).
            let keep = params.radius + CONTACT_MARGIN;
            // Only the walls of the region the body is in count (physics.md §8.1: the region's
            // wall array), so a corridor may pass under or beside unrelated geometry.
            for _ in 0..2 {
                for &wi in &world.regions[self.region].walls {
                    let wall = &world.walls[wi];
                    if !self.wall_blocks(world, params, wi) {
                        continue;
                    }
                    let (ax, ay) = world.vertices[wall.a];
                    let (bx, by) = world.vertices[wall.b];
                    if nx < ax.min(bx) - keep || nx > ax.max(bx) + keep || ny < ay.min(by) - keep || ny > ay.max(by) + keep {
                        continue;
                    }
                    let (ex, ey) = (bx - ax, by - ay);
                    let len = ex.hypot(ey);
                    let t = if len > 0.0 { ((nx - ax) * ex + (ny - ay) * ey) / (len * len) } else { 0.0 };
                    if len > 0.0 && (0.0..=1.0).contains(&t) {
                        // Along the wall: back out to `keep` on the side the body came from, so
                        // it can never be pushed through (physics.md §8.1).
                        let (ux, uy) = (-ey / len, ex / len);
                        let side_of = |x: f32, y: f32| (x - ax) * ux + (y - ay) * uy;
                        let old = side_of(self.x, self.y);
                        let new = side_of(nx, ny);
                        let sign = if old.abs() > 1e-3 { old.signum() } else if new != 0.0 { new.signum() } else { 1.0 };
                        if new * sign < keep {
                            let push = sign * keep - new;
                            nx += ux * push;
                            ny += uy * push;
                        }
                    } else {
                        // Past an end: push radially off the corner.
                        let (px, py) = closest_point(ax, ay, bx, by, nx, ny);
                        let (ox, oy) = (nx - px, ny - py);
                        let d2 = ox * ox + oy * oy;
                        if d2 < keep * keep && d2 > 1e-8 {
                            let d = d2.sqrt();
                            let push = keep - d;
                            nx += ox / d * push;
                            ny += oy / d * push;
                        }
                    }
                }
            }
            // A gap too narrow for the body: the whole step is undone (unless already squeezed,
            // e.g. placed there, so the body can always get out).
            if self.near_blocking_wall(world, params, nx, ny, keep - 1.0) && !self.near_blocking_wall(world, params, self.x, self.y, keep - 1.0) {
                continue;
            }
            let current = &world.regions[self.region];
            // Re-home on crossing one of the region's portal lines (physics.md §8.2), the first
            // one along the step; stacked levels and lost bodies fall back to a point lookup.
            let crossed = (current.below.is_none() && current.above.is_none())
                .then(|| {
                    current
                        .walls
                        .iter()
                        .filter_map(|&wi| {
                            let w = &world.walls[wi];
                            let back = w.back.filter(|&b| b != w.front)?;
                            let other = if w.front == self.region { back } else if back == self.region { w.front } else { return None };
                            let t = segment_param(world.vertices[w.a], world.vertices[w.b], (self.x, self.y), (nx, ny))?;
                            Some((t, other))
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .map(|(_, r)| r)
                })
                .flatten();
            let stacked = current.below.is_some() || current.above.is_some();
            let lost = !stacked && !world.point_in_region(self.x, self.y, self.region) && !world.point_in_region(nx, ny, self.region);
            let target = if let Some(r) = crossed {
                // (Likewise a crossing that lands beyond the new region's own solid lines: the
                // end of a portal where the region behind it is cut off by a diagonal.)
                (world.point_in_region(nx, ny, r) || world.region_at(nx, ny).is_some() || !world.point_in_region(self.x, self.y, self.region)).then_some(r)
            } else if !stacked && !lost {
                // No line crossed: still in the same region, unless the push put the body out
                // of the level through a solid line (two walls of a narrow corner pushing against
                // each other): a push that lands outside every region undoes the step
                // (physics.md §8.1).
                (world.point_in_region(nx, ny, self.region) || world.region_at(nx, ny).is_some() || !world.point_in_region(self.x, self.y, self.region)).then_some(self.region)
            } else {
                // The region the point is in, if the body can step onto its floor (a raised
                // region nested in another, like a pedestal: crossing onto it re-homes the body
                // and raises it, physics.md §8.2); else the one at our height (stacked levels).
                // Only a region the body fits in (a lookup that finds none would hand back
                // whatever region lies under the point, e.g. a cellar below, and the ceiling
                // clamp would drag the body down into it); else the step is undone, as one
                // that would land outside any region (physics.md §8.1).
                let fits = |r: usize| {
                    let reg = &world.regions[r];
                    reg.floor - self.z <= params.step_height && self.z < reg.ceiling
                };
                world.region_at(nx, ny).filter(|&r| fits(r)).or_else(|| world.region_at_z(nx, ny, Some(self.z)).filter(|&r| fits(r)))
            };
            if let Some(region) = target {
                let floor = world.solid_floor(region).0;
                if floor - self.z <= params.step_height {
                    self.x = nx;
                    self.y = ny;
                    if region != self.region {
                        self.region = region;
                        if (self.crossed_n as usize) < MAX_CROSSINGS {
                            self.crossed[self.crossed_n as usize] = region as u32;
                            self.crossed_n += 1;
                        }
                        if floor > self.z {
                            self.z = floor; // step up
                        }
                    }
                }
            }
        }
    }

    /// Whether a blocking wall of the current region is closer than `dist` to (x, y).
    fn near_blocking_wall(&self, world: &World, params: &PhysicsParams, x: f32, y: f32, dist: f32) -> bool {
        world.regions[self.region].walls.iter().map(|&wi| (wi, &world.walls[wi])).any(|(wi, wall)| {
            let (ax, ay) = world.vertices[wall.a];
            let (bx, by) = world.vertices[wall.b];
            if x < ax.min(bx) - dist || x > ax.max(bx) + dist || y < ay.min(by) - dist || y > ay.max(by) + dist {
                return false;
            }
            let (px, py) = closest_point(ax, ay, bx, by, x, y);
            (x - px).hypot(y - py) < dist && self.wall_blocks(world, params, wi)
        })
    }

    fn wall_blocks(&self, world: &World, params: &PhysicsParams, wall: usize) -> bool {
        let w = &world.walls[wall];
        // Walls only collide where their regions' height range overlaps the body (stacked
        // levels share x/y but not z).
        let height = if self.crouching { params.crouch_eye } else { params.height };
        let overlaps = |r: usize| world.regions.get(r).is_some_and(|reg| self.z < reg.ceiling && self.z + height > reg.floor);
        if !overlaps(w.front) && !w.back.is_some_and(overlaps) {
            return false;
        }
        if w.blocking {
            return true;
        }
        let Some(back) = w.back else { return true };
        if back == w.front {
            return false;
        }
        let (Some(front), Some(back)) = (world.regions.get(w.front), world.regions.get(back)) else { return true };
        // The far side is whichever region we are not standing in.
        let other = if w.front == self.region { back } else { front };
        let height = if self.crouching { params.crouch_eye } else { params.height };
        // physics.md §8.2: the step is at most 32 and the body (without the head clearance kept
        // under ceilings) fits under the far ceiling: z + Height < nextCeiling.
        other.floor - self.z > params.step_height || other.floor.max(self.z) + height >= other.ceiling
    }
}

/// Where along p1→p2 (0..1) it crosses the segment a-b, if it does.
fn segment_param(a: (f32, f32), b: (f32, f32), p1: (f32, f32), p2: (f32, f32)) -> Option<f32> {
    let (rx, ry) = (p2.0 - p1.0, p2.1 - p1.1);
    let (sx, sy) = (b.0 - a.0, b.1 - a.1);
    let denom = rx * sy - ry * sx;
    if denom.abs() < 1e-9 {
        return None;
    }
    let (qx, qy) = (a.0 - p1.0, a.1 - p1.1);
    let t = (qx * sy - qy * sx) / denom;
    let u = (qx * ry - qy * rx) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

fn closest_point(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> (f32, f32) {
    let (ex, ey) = (bx - ax, by - ay);
    let len2 = ex * ex + ey * ey;
    if len2 == 0.0 {
        return (ax, ay);
    }
    let t = (((px - ax) * ex + (py - ay) * ey) / len2).clamp(0.0, 1.0);
    (ax + ex * t, ay + ey * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walls_stop_the_player() {
        let world = World::test_room(256.0, 256.0, 128.0);
        let params = PhysicsParams::default();
        let mut player = Player::new(&world, 128.0, 128.0, 0.0);
        let mut input = Input::default();
        input.set(Action::Forward, true);
        for _ in 0..200 {
            player.tick(&world, &input, &params);
        }
        assert!((player.x - (256.0 - params.radius - CONTACT_MARGIN)).abs() < 1.0, "x = {}", player.x);
        assert!((player.y - 128.0).abs() < 0.01);
    }

    #[test]
    fn walk_speed_matches_original() {
        // 800 units per 1.2 s.
        let world = World::test_room(4096.0, 256.0, 128.0);
        let params = PhysicsParams::default();
        let mut player = Player::new(&world, 100.0, 128.0, 0.0);
        let mut input = Input::default();
        input.set(Action::Forward, true);
        for _ in 0..(crate::TICK_RATE * 1.2) as usize {
            player.tick(&world, &input, &params);
        }
        assert!((player.x - 900.0).abs() < 2.0, "x = {}", player.x);
    }

    #[test]
    fn jump_peaks_near_one_player_height() {
        let world = World::test_room(256.0, 256.0, 512.0);
        let params = PhysicsParams::default();
        let mut player = Player::new(&world, 128.0, 128.0, 0.0);
        let mut input = Input::default();
        input.set(Action::Jump, true);
        let mut peak: f32 = 0.0;
        for i in 0..200 {
            player.tick(&world, &input, &params);
            if i == 0 {
                input.end_tick();
                input.set(Action::Jump, false);
            }
            peak = peak.max(player.z);
        }
        assert!((45.0..=56.0).contains(&peak), "peak {peak}");
        assert_eq!(player.z, 0.0);
        assert!(player.on_ground);
    }
}
