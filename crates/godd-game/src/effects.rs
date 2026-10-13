//! Wall and region effects (re/engine/world-effects.md): doors, lifts, jump pads, switches,
//! region triggers, scrolling and animated textures.
//!
//! The compiled XRB is a snapshot taken after every effect's init ran, so nothing is
//! initialised here beyond reading the stored state (doors are stored closed with their open
//! height in var C, speeds already 16.16, …).

use crate::anim::{self, Sequence};
use crate::level::Level;
use crate::player::PlayerEvent;
use crate::world::World;
use godd_formats::xrb::{RecordExt, Section};

/// Effect indices (row order of re/engine/effects.md).
pub mod index {
    pub const WTEXHOR: i32 = 201;
    pub const WANIM: i32 = 203;
    pub const WTAGSW: i32 = 206;
    pub const RFBOUNCE: i32 = 211;
    pub const RROTATE: i32 = 212;
    pub const RANIM: i32 = 214;
    pub const RFUP1: i32 = 216;
    pub const RTDOOR: i32 = 219;
    pub const RTAGSW: i32 = 223;
    pub const RSLANT: i32 = 228;
}

/// Angle units per turn in the original.
const ANGLE_UNITS: f32 = 2048.0;
/// Region effects that rotate run once per original 30 fps frame.
const FRAME_RATE: f32 = 30.0;

/// Original speeds are units per 1.2 s.
const SPEED_SECONDS: f32 = 1.2;
/// Switch lockout after a touch: 20 animation units of 12 ms.
const SWITCH_LOCKOUT: f32 = 0.24;
/// Wall flag: limited number of uses (count in +0xac).
const WALL_LIMITED_USES: u32 = 0x1000;

/// Requests produced by effects for the session to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectRequest {
    Tag(u32),
    /// A sound, from a region (its moving door/lift) when known.
    Sound(String, Option<usize>),
    /// RROTATE turned `region` by `angle` radians about `pivot`: objects on its floor follow.
    Rotated { region: usize, pivot: (f32, f32), angle: f32 },
}

/// The few aux-record fields world effects use.
#[derive(Debug, Clone, Default)]
struct Aux {
    params: [i32; 5],
    /// Sample names for sound slots 0–4 (start, loop, stop, …).
    sounds: [Option<String>; 5],
}

impl Aux {
    fn load(level: &Level, index: i32) -> Option<Self> {
        let record = level.orb.records(Section::Auxs).get(usize::try_from(index).ok()?)?;
        let sounds_section = level.orb.records(Section::Sounds);
        let sounds = std::array::from_fn(|k| {
            let sound = usize::try_from(record.le_i32(0x9c + 4 * k)).ok()?;
            let name = c_string(&sounds_section.get(sound)?[0x0c..0x18]);
            (!name.is_empty()).then_some(name)
        });
        Some(Self { params: std::array::from_fn(|k| record.le_i32(0x88 + 4 * k)), sounds })
    }
    fn p88(&self) -> i32 {
        self.params[0]
    }
    fn p8c(&self) -> i32 {
        self.params[1]
    }
    fn p90(&self) -> i32 {
        self.params[2]
    }
    fn p94(&self) -> i32 {
        self.params[3]
    }
    fn p98(&self) -> i32 {
        self.params[4]
    }
    fn sound(&self, slot: usize) -> Option<String> {
        self.sounds[slot].clone()
    }
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| b as char).collect::<String>().trim().to_string()
}

/// An animated texture slot: which surface it drives and its playback.
#[derive(Debug, Clone)]
struct AnimSlot {
    target: SlotTarget,
    tex: usize,
    player: anim::Player,
}

#[derive(Debug, Clone, Copy)]
enum SlotTarget {
    WallUpper(usize),
    WallMiddle(usize),
    WallLower(usize),
    Floor(usize),
    Ceiling(usize),
}

#[derive(Debug, Clone)]
enum WallKind {
    Switch { tag: i32, tag_end: i32, uses: Option<i32>, state: u8, lockout: f32 },
    Animated,
    Scroll { speed: f32, negative: bool },
}

#[derive(Debug, Clone)]
struct WallEffect {
    wall: usize,
    kind: WallKind,
    slots: Vec<AnimSlot>,
}

#[derive(Debug, Clone)]
enum RegionKind {
    Door { open: f32, speed: f32, moving: bool, close_at: Option<f64>, wait: Option<f64>, aux: Option<Aux> },
    Lift { bottom: f32, speed: f32 },
    Bounce { speed: f32, sound: Option<String> },
    Animated { aux: Option<Aux> },
    Trigger { aux: Option<Aux>, uses_left: Option<i32> },
    Rotate(Box<Rotation>),
}

/// RROTATE state (world-effects.md §3.6).
#[derive(Debug, Clone)]
struct Rotation {
    /// 0 idle, 1 running, 2 stop requested, other = always running.
    mode: i32,
    /// Angle units per frame (signed).
    step: i32,
    turned: i32,
    segments_done: i32,
    pivot: (f32, f32),
    segment: i32,
    segments: i32,
    ping_pong: bool,
    aux: Option<Aux>,
    /// Original positions of the region's wall endpoints (restored after even segments).
    saved: Vec<(usize, (f32, f32))>,
}

/// A ramp (RSLANT): the container's strips get z = low + slope · distance from the base wall.
#[derive(Debug, Clone)]
struct Ramp {
    along_y: bool,
    slope: f32,
    base: (f32, f32),
    low: f32,
    high: f32,
}

#[derive(Debug, Clone)]
struct RegionEffect {
    region: usize,
    tag: i32,
    kind: RegionKind,
    slots: Vec<AnimSlot>,
}

#[derive(Debug, Default)]
pub struct Effects {
    walls: Vec<WallEffect>,
    regions: Vec<RegionEffect>,
    /// region index → position in `regions`.
    by_region: Vec<Option<usize>>,
    by_wall: Vec<Option<usize>>,
    requests: Vec<EffectRequest>,
    /// RSLANT: strip region → ramp.
    ramps: Vec<Option<Ramp>>,
    frame_accum: f32,
    /// Original-game fixes on (cvar `g_fixes`).
    pub fixes: bool,
    /// Switch wall → the door region it is a face of.
    door_faces: std::collections::HashMap<usize, usize>,
    /// Door regions open (or opening) at the last tick.
    open_doors: std::collections::HashSet<usize>,
}

/// A switch wall: touching it fires `tags` (while `usable`).
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchInfo {
    pub wall: usize,
    pub tags: std::ops::RangeInclusive<i32>,
    pub usable: bool,
}

impl Effects {
    /// Every switch wall of the level.
    pub fn switches(&self) -> Vec<SwitchInfo> {
        self.walls
            .iter()
            .filter_map(|w| match &w.kind {
                WallKind::Switch { tag, tag_end, uses, .. } => Some(SwitchInfo { wall: w.wall, tags: *tag..=(*tag_end).max(*tag), usable: uses.is_none_or(|n| n > 0) }),
                _ => None,
            })
            .collect()
    }

    /// Doors (and rotating doors): (region, tag that opens it).
    pub fn doors(&self) -> Vec<(usize, i32)> {
        self.regions.iter().filter(|r| matches!(r.kind, RegionKind::Door { .. } | RegionKind::Rotate(_))).map(|r| (r.region, r.tag)).collect()
    }

    /// Auto-close delay (seconds) of each RTDOOR that has one.
    pub fn door_waits(&self) -> Vec<(usize, f64)> {
        self.regions.iter().filter_map(|r| match r.kind {
            RegionKind::Door { wait: Some(w), .. } => Some((r.region, w)),
            _ => None,
        }).collect()
    }

    /// Whether any door (RTDOOR) is opening or closing now.
    /// The effect a region carries, by name (debug and tools): "door", "lift", "bounce",
    /// "animated", "trigger" or "rotate".
    pub fn region_kind(&self, region: usize) -> Option<&'static str> {
        let i = self.by_region.get(region).copied().flatten()?;
        Some(match self.regions[i].kind {
            RegionKind::Door { .. } => "door",
            RegionKind::Lift { .. } => "lift",
            RegionKind::Bounce { .. } => "bounce",
            RegionKind::Animated { .. } => "animated",
            RegionKind::Trigger { .. } => "trigger",
            RegionKind::Rotate(_) => "rotate",
        })
    }

    pub fn doors_moving(&self) -> bool {
        self.regions.iter().any(|r| matches!(r.kind, RegionKind::Door { moving: true, .. }))
    }

    /// Perpetual lifts (RFUP1): (region, bottom floor).
    pub fn lifts(&self) -> Vec<(usize, f32)> {
        self.regions.iter().filter_map(|r| match r.kind {
            RegionKind::Lift { bottom, .. } => Some((r.region, bottom)),
            _ => None,
        }).collect()
    }

    /// Regions that fire tags when entered, left, landed on or bumped (ceiling): (region,
    /// the tags, 0 = none).
    /// Activator codes (aux +0x98) of trigger regions.
    pub fn trigger_codes(&self) -> Vec<(usize, i32)> {
        self.regions
            .iter()
            .filter_map(|r| match &r.kind {
                RegionKind::Trigger { aux: Some(a), .. } => Some((r.region, a.p98())),
                _ => None,
            })
            .collect()
    }

    pub fn triggers(&self) -> Vec<(usize, [i32; 4])> {
        self.regions
            .iter()
            .filter_map(|r| match &r.kind {
                RegionKind::Trigger { aux, .. } | RegionKind::Animated { aux } => {
                    let a = aux.as_ref()?;
                    Some((r.region, [a.p88(), a.p8c(), a.p90(), a.p94()]))
                }
                _ => None,
            })
            .collect()
    }

    pub fn new(level: &Level) -> Self {
        let mut fx = Effects {
            by_region: vec![None; level.world.regions.len()],
            by_wall: vec![None; level.world.walls.len()],
            ..Default::default()
        };
        let animated = |tex: i32| -> Option<usize> {
            let t = usize::try_from(tex).ok()?;
            let seq = level.sequences.get(t)?.as_ref()?;
            (level.orb.texs[t].kind & 1 != 0 && seq.entries.len() > 1).then_some(t)
        };
        let slot = |target: SlotTarget, tex: i32| -> Option<AnimSlot> {
            let t = animated(tex)?;
            let seq = level.sequences[t].as_ref()?;
            Some(AnimSlot { target, tex: t, player: anim::Player::start(seq, 0) })
        };

        for (wi, w) in level.orb.records(Section::Walls).iter().enumerate() {
            let effect = w.le_i32(0xd0);
            let kind = match effect {
                index::WTAGSW => {
                    let aux = Aux::load(level, w.le_i32(0xdc));
                    let tag = w.le_u16(0xa6) as i16 as i32;
                    let flags = w.le_i32(0) as u32;
                    WallKind::Switch {
                        tag,
                        tag_end: aux.as_ref().map_or(tag, |a| a.p88().max(tag)),
                        uses: (flags & WALL_LIMITED_USES != 0).then(|| w.le_i32(0xac)),
                        state: w.le_i32(0xbc) as u8 & 1,
                        lockout: 0.0,
                    }
                }
                index::WANIM => WallKind::Animated,
                index::WTEXHOR => {
                    let Some(aux) = Aux::load(level, w.le_i32(0xdc)) else { continue };
                    WallKind::Scroll { speed: (aux.p8c() & 0xff) as f32, negative: aux.p88() != 0 }
                }
                _ => continue,
            };
            let slots = [(SlotTarget::WallUpper(wi), 0x2c), (SlotTarget::WallMiddle(wi), 0x44), (SlotTarget::WallLower(wi), 0x5c)]
                .into_iter()
                .filter_map(|(t, off)| slot(t, w.le_i32(off)))
                .collect();
            fx.by_wall[wi] = Some(fx.walls.len());
            fx.walls.push(WallEffect { wall: wi, kind, slots });
        }

        for (ri, r) in level.orb.records(Section::Regions).iter().enumerate() {
            let effect = r.le_i32(0xb0);
            let aux = || Aux::load(level, r.le_i32(0x10c));
            let kind = match effect {
                index::RTDOOR => {
                    let aux = aux();
                    let wait = aux.as_ref().and_then(|a| (a.p98() > 0).then(|| a.p98() as f64 * 0.1));
                    RegionKind::Door { open: r.fixed(0x74), speed: r.fixed(0xa0), moving: false, close_at: None, wait, aux }
                }
                index::RFUP1 => RegionKind::Lift { bottom: r.fixed(0x74), speed: r.fixed(0xa0) },
                index::RFBOUNCE => RegionKind::Bounce { speed: r.fixed(0x6c), sound: aux().and_then(|a| a.sound(4)) },
                index::RANIM => RegionKind::Animated { aux: aux() },
                index::RTAGSW => {
                    let uses = r.le_i32(0xa0);
                    RegionKind::Trigger { aux: aux(), uses_left: (uses > 0).then(|| r.le_i32(0x6c)) }
                }
                index::RROTATE => {
                    let aux = aux();
                    let pivot = usize::try_from(r.le_i32(0xf8))
                        .ok()
                        .and_then(|p| level.world.vertices.get(p).copied())
                        .unwrap_or_else(|| {
                            let ((x0, y0), (x1, y1)) = level.world.regions[ri].bounds;
                            ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
                        });
                    let mut saved: Vec<(usize, (f32, f32))> = Vec::new();
                    for &w in &level.world.regions[ri].walls {
                        for v in [level.world.walls[w].a, level.world.walls[w].b] {
                            if !saved.iter().any(|(k, _)| *k == v) {
                                saved.push((v, level.world.vertices[v]));
                            }
                        }
                    }
                    RegionKind::Rotate(Box::new(Rotation {
                        mode: r.le_i32(0x70),
                        step: r.le_u16(0x7c) as i16 as i32,
                        turned: 0,
                        segments_done: 0,
                        pivot,
                        segment: aux.as_ref().map_or(0, |a| a.params[2].abs()),
                        segments: aux.as_ref().map_or(0, |a| a.params[3]),
                        ping_pong: aux.as_ref().is_some_and(|a| a.params[4] == 1),
                        aux,
                        saved,
                    }))
                }
                _ => continue,
            };
            let slots = [(SlotTarget::Floor(ri), 0x1c), (SlotTarget::Ceiling(ri), 0x34)]
                .into_iter()
                .filter_map(|(t, off)| slot(t, r.le_i32(off)))
                .collect();
            fx.by_region[ri] = Some(fx.regions.len());
            fx.regions.push(RegionEffect { region: ri, tag: r.le_u16(0x86) as i16 as i32, kind, slots });
        }
        fx.ramps = build_ramps(level);
        // Switch walls that are faces of the door they open (g_fixes: not shut by a touch).
        let doors = fx.doors();
        for sw in fx.switches() {
            let wall = &level.world.walls[sw.wall];
            if let Some(&(door, _)) = doors.iter().find(|&&(r, t)| sw.tags.contains(&t) && (wall.front == r || wall.back == Some(r))) {
                fx.door_faces.insert(sw.wall, door);
            }
        }
        fx
    }

    /// RSLANT: (minimum z, maximum z) for an object standing at (x, y) in `region`.
    pub fn ramp_limits(&self, region: usize, x: f32, y: f32) -> Option<(f32, f32)> {
        let ramp = self.ramps.get(region)?.as_ref()?;
        let d = if ramp.along_y { (y - ramp.base.1).abs() } else { (x - ramp.base.0).abs() };
        Some(((ramp.low + ramp.slope * d).min(ramp.high), ramp.high))
    }

    /// OWTOW: wall `target` takes over the effect of wall `source` (or loses its own).
    pub fn copy_wall_effect(&mut self, target: usize, source: usize) {
        if let Some(i) = self.by_wall.get(target).copied().flatten() {
            self.walls[i].kind = WallKind::Animated;
            self.walls[i].slots.clear();
            self.by_wall[target] = None;
        }
        if let Some(i) = self.by_wall.get(source).copied().flatten() {
            let mut copy = self.walls[i].clone();
            copy.wall = target;
            for slot in &mut copy.slots {
                slot.target = match slot.target {
                    SlotTarget::WallUpper(_) => SlotTarget::WallUpper(target),
                    SlotTarget::WallMiddle(_) => SlotTarget::WallMiddle(target),
                    SlotTarget::WallLower(_) => SlotTarget::WallLower(target),
                    other => other,
                };
            }
            self.by_wall[target] = Some(self.walls.len());
            self.walls.push(copy);
        }
    }

    pub fn take_requests(&mut self) -> Vec<EffectRequest> {
        std::mem::take(&mut self.requests)
    }

    fn sound(&mut self, name: Option<String>) {
        if let Some(n) = name {
            self.requests.push(EffectRequest::Sound(n, None));
        }
    }

    /// A sound from effect entry `i`'s world region.
    fn sound_in(&mut self, i: usize, name: Option<String>) {
        if let Some(n) = name {
            let region = self.regions[i].region;
            self.requests.push(EffectRequest::Sound(n, Some(region)));
        }
    }

    fn tag(&mut self, tag: i32) {
        if tag > 0 {
            self.requests.push(EffectRequest::Tag(tag as u32));
        }
    }

    /// Event 0x40: regions listening on `tag` (doors).
    pub fn on_tag(&mut self, tag: u32) {
        for i in 0..self.regions.len() {
            if self.regions[i].tag == tag as i32 {
                self.start_door(i);
            }
        }
    }

    /// Start a door's motion (a tag while it is already moving is ignored); RROTATE starts
    /// or is asked to stop at the next segment end.
    fn start_door(&mut self, i: usize) {
        if let RegionKind::Rotate(rot) = &mut self.regions[i].kind {
            if rot.mode == 0 {
                rot.mode = 1;
                rot.turned = 0;
                rot.segments_done = 0;
                let (start, run) = (rot.aux.as_ref().and_then(|a| a.sound(0)), rot.aux.as_ref().and_then(|a| a.sound(1)));
                self.sound_in(i, start);
                self.sound_in(i, run);
            } else {
                rot.mode = 2;
            }
            return;
        }
        if let RegionKind::Door { moving, aux, .. } = &mut self.regions[i].kind
            && !*moving {
                *moving = true;
                let (start, run) = (aux.as_ref().and_then(|a| a.sound(0)), aux.as_ref().and_then(|a| a.sound(1)));
                self.sound_in(i, start);
                self.sound_in(i, run);
            }
    }

    /// Event 0x80 to a region (named trigger): doors start like on a tag.
    pub fn trigger_region(&mut self, region: usize) {
        if let Some(i) = self.by_region.get(region).copied().flatten() {
            self.start_door(i);
        }
    }

    /// Contacts reported by the player's movement.
    pub fn on_player_events(&mut self, events: &[PlayerEvent]) {
        for &event in events {
            match event {
                PlayerEvent::TouchedWall(w) => self.touch_wall(w),
                PlayerEvent::EnteredRegion(r) => self.region_event(r, 0x1000),
                PlayerEvent::LeftRegion(r) => self.region_event(r, 0x2000),
                PlayerEvent::Landed(r) => self.region_event(r, 0x400),
                PlayerEvent::HitCeiling(r) => self.region_event(r, 0x800),
            }
        }
    }

    fn touch_wall(&mut self, wall: usize) {
        let Some(i) = self.by_wall.get(wall).copied().flatten() else { return };
        // g_fixes: a door's own face does not shut it while it is open. In the original,
        // walking through a door with a switch face on both sides touches the far face and
        // closes the door in front of (or on) you; it still closes on its own timer.
        if self.fixes && self.door_faces.get(&wall).is_some_and(|d| self.open_doors.contains(d)) {
            return;
        }
        let WallKind::Switch { tag, tag_end, uses, state, lockout } = &mut self.walls[i].kind else { return };
        if *lockout > 0.0 {
            return;
        }
        if let Some(n) = uses {
            if *n < 1 {
                return;
            }
            *n -= 1;
        }
        *state ^= 1;
        *lockout = SWITCH_LOCKOUT;
        let (first, last) = (*tag, *tag_end);
        for t in first..=last.max(first) {
            self.tag(t);
        }
    }

    fn region_event(&mut self, region: usize, event: u32) {
        let Some(i) = self.by_region.get(region).copied().flatten() else { return };
        let fire = |aux: &Aux| match event {
            0x400 => aux.p90(),
            0x800 => aux.p94(),
            0x1000 => aux.p88(),
            0x2000 => aux.p8c(),
            _ => 0,
        };
        let mut tags = Vec::new();
        match &mut self.regions[i].kind {
            RegionKind::Animated { aux: Some(aux) } => tags.push(fire(aux)),
            RegionKind::Trigger { aux: Some(aux), uses_left } => {
                if let Some(n) = uses_left {
                    if *n < 1 {
                        return;
                    }
                    *n -= 1;
                }
                tags.push(fire(aux));
            }
            _ => {}
        }
        for t in tags {
            self.tag(t);
        }
    }

    /// Pre-tick work for one simulation tick of `dt` seconds at time `now` (seconds).
    /// `occupancy(region)` is the tallest object height in the region (0 if empty).
    pub fn tick(&mut self, level: &mut Level, now: f64, dt: f32, occupancy: &dyn Fn(usize) -> f32) {
        self.open_doors = self.door_faces.values().copied().filter(|&d| level.world.regions.get(d).is_some_and(|r| r.ceiling > r.floor + 0.5)).collect();
        let world = &mut level.world;
        let sequences = &level.sequences;
        for w in &mut self.walls {
            if let WallKind::Switch { lockout, .. } = &mut w.kind {
                *lockout = (*lockout - dt).max(0.0);
            }
            if let WallKind::Scroll { speed, negative } = w.kind {
                let delta = speed / SPEED_SECONDS * dt * if negative { -1.0 } else { 1.0 };
                let wall = &mut world.walls[w.wall];
                for surface in [&mut wall.upper, &mut wall.middle, &mut wall.lower].into_iter().flatten() {
                    surface.offset_u = (surface.offset_u + delta).rem_euclid(256.0);
                }
            }
            step_slots(&mut w.slots, world, sequences, dt);
        }
        let mut sounds = Vec::new();
        for r in &mut self.regions {
            step_slots(&mut r.slots, world, sequences, dt);
            let region = &mut world.regions[r.region];
            match &mut r.kind {
                RegionKind::Door { open, speed, moving, close_at, wait, aux } => {
                    if !*moving {
                        if close_at.is_some_and(|t| now > t) {
                            *close_at = None;
                            *moving = true;
                            sounds.push(aux.as_ref().and_then(|a| a.sound(0)));
                        }
                        continue;
                    }
                    let h = occupancy(r.region);
                    let step = *speed / SPEED_SECONDS * dt;
                    if *speed < 0.0 && h > 0.0 {
                        if region.floor + h + 3.0 < region.ceiling {
                            region.ceiling += step;
                        }
                    } else {
                        region.ceiling += step;
                    }
                    if region.ceiling >= *open {
                        region.ceiling = *open;
                        *speed = -speed.abs();
                        *moving = false;
                        *close_at = wait.map(|w| now + w);
                        sounds.push(aux.as_ref().and_then(|a| a.sound(2)));
                    } else if region.ceiling <= region.floor + h {
                        if h == 0.0 {
                            region.ceiling = region.floor;
                            *speed = speed.abs();
                            *moving = false;
                            *close_at = None;
                            sounds.push(aux.as_ref().and_then(|a| a.sound(0)));
                        } else {
                            region.ceiling = region.floor + h + 3.0;
                        }
                    }
                }
                RegionKind::Lift { bottom, speed } => {
                    let h = occupancy(r.region);
                    region.floor += *speed / SPEED_SECONDS * dt;
                    let top = region.ceiling - h - 2.0;
                    if region.floor >= top {
                        region.floor = top;
                        *speed = -speed.abs();
                    } else if region.floor <= *bottom {
                        region.floor = *bottom;
                        *speed = speed.abs();
                    }
                }
                _ => {}
            }
        }
        self.frame_accum += dt * FRAME_RATE;
        while self.frame_accum >= 1.0 {
            self.frame_accum -= 1.0;
            for r in &mut self.regions {
                let RegionKind::Rotate(rot) = &mut r.kind else { continue };
                if rot.mode == 0 || rot.step == 0 || rot.aux.is_none() {
                    continue;
                }
                let angle = rot.step as f32 / ANGLE_UNITS * std::f32::consts::TAU;
                rotate_vertices(world, &rot.saved, rot.pivot, angle);
                self.requests.push(EffectRequest::Rotated { region: r.region, pivot: rot.pivot, angle });
                rot.turned += rot.step;
                if rot.segment != 0 && rot.turned.abs() >= rot.segment {
                    rot.segments_done += 1;
                    if rot.ping_pong {
                        rot.step = -rot.step;
                        if rot.segments_done % 2 == 0 {
                            for &(v, p) in &rot.saved {
                                world.vertices[v] = p; // snap back: removes overshoot drift
                            }
                        }
                    }
                    rot.turned = 0;
                    let slot = if rot.segments_done % 2 == 0 { 0 } else { 2 };
                    sounds.push(rot.aux.as_ref().and_then(|a| a.sound(slot)));
                    if rot.mode == 2 {
                        rot.mode = 0;
                        rot.segments_done = 0;
                        sounds.push(rot.aux.as_ref().and_then(|a| a.sound(2)));
                    }
                }
                if rot.segments > 0 && rot.segments_done >= rot.segments {
                    rot.mode = 0;
                    rot.segments_done = 0;
                    sounds.push(rot.aux.as_ref().and_then(|a| a.sound(2)));
                }
            }
        }
        for s in sounds {
            self.sound(s);
        }
    }

    /// RFBOUNCE (world-effects.md 3.3, mode 0, every pad in the data): a body on the pad's
    /// floor that is not rising is launched at the pad's speed, on every contact, so one
    /// standing on it (or walking onto it) bounces too, not only one landing on it. Returns the
    /// speed and plays the pad's sound.
    pub fn bounce(&mut self, region: usize) -> Option<f32> {
        let i = self.by_region.get(region).copied().flatten()?;
        let RegionKind::Bounce { speed, sound } = &self.regions[i].kind else { return None };
        let (speed, sound) = (*speed, sound.clone());
        (speed > 0.0).then(|| {
            self.sound_in(i, sound);
            speed
        })
    }

    /// Whether a region is a jump pad (physics adds its speed to jumps started there).
    pub fn bounce_speed(&self, region: usize) -> Option<f32> {
        let i = self.by_region.get(region).copied().flatten()?;
        match self.regions[i].kind {
            RegionKind::Bounce { speed, .. } => Some(speed),
            _ => None,
        }
    }
}

/// Rotate the given vertices about `pivot` from their current positions by `angle`.
fn rotate_vertices(world: &mut World, vertices: &[(usize, (f32, f32))], pivot: (f32, f32), angle: f32) {
    let (sin, cos) = angle.sin_cos();
    for &(v, _) in vertices {
        let (x, y) = world.vertices[v];
        let (dx, dy) = (x - pivot.0, y - pivot.1);
        world.vertices[v] = (pivot.0 + dx * cos - dy * sin, pivot.1 + dy * cos + dx * sin);
    }
}

/// Move a region's wall endpoints by (dx, dy) (RMOVE rafts) and update its bounding box.
pub fn translate_region(world: &mut World, region: usize, dx: f32, dy: f32) {
    let mut moved: Vec<usize> = Vec::new();
    for &w in &world.regions[region].walls {
        for v in [world.walls[w].a, world.walls[w].b] {
            if !moved.contains(&v) {
                moved.push(v);
                world.vertices[v].0 += dx;
                world.vertices[v].1 += dy;
            }
        }
    }
    let r = &mut world.regions[region];
    r.bounds = ((r.bounds.0.0 + dx, r.bounds.0.1 + dy), (r.bounds.1.0 + dx, r.bounds.1.1 + dy));
}

fn build_ramps(level: &Level) -> Vec<Option<Ramp>> {
    let regions = level.orb.records(Section::Regions);
    let walls = level.orb.records(Section::Walls);
    let wall_by_name = |name: &str| walls.iter().position(|w| c_string(&w[0xb0..0xb8]).eq_ignore_ascii_case(name));
    let mut ramps = vec![None; regions.len()];
    for (si, s) in regions.iter().enumerate() {
        if s.le_i32(0xb0) != index::RSLANT {
            continue;
        }
        let Some(aux) = usize::try_from(s.le_i32(0x10c)).ok().and_then(|a| level.orb.records(Section::Auxs).get(a)) else { continue };
        let base_name = c_string(&aux[0x0e + 12 * 8..0x0e + 12 * 9]);
        let Some(base) = wall_by_name(&base_name) else { continue };
        let p1 = level.world.vertices[level.world.walls[base].a];
        let ramp = Ramp {
            along_y: s.fixed(0x6c) == 0.0,
            slope: s.fixed(0x74),
            base: p1,
            low: aux.fixed(0x90),
            high: aux.fixed(0x94),
        };
        for (ri, r) in regions.iter().enumerate() {
            if r.le_i32(0x04) as u32 & 0x20000 != 0 && r.le_u16(0x114) as usize == si {
                ramps[ri] = Some(ramp.clone());
            }
        }
    }
    ramps
}

/// Step animated texture slots and write the shown frame (rotation 0) into the world.
fn step_slots(slots: &mut [AnimSlot], world: &mut World, sequences: &[Option<Sequence>], dt: f32) {
    for slot in slots {
        let Some(seq) = sequences[slot.tex].as_ref() else { continue };
        if !slot.player.advance(seq, dt) {
            continue;
        }
        let Some(&(pic, _)) = seq.rotations_of(slot.player.entry).first() else { continue };
        if pic >= world.images.len() {
            continue;
        }
        let surface = match slot.target {
            SlotTarget::WallUpper(w) => &mut world.walls[w].upper,
            SlotTarget::WallMiddle(w) => &mut world.walls[w].middle,
            SlotTarget::WallLower(w) => &mut world.walls[w].lower,
            SlotTarget::Floor(r) => &mut world.regions[r].floor_surface,
            SlotTarget::Ceiling(r) => &mut world.regions[r].ceiling_surface,
        };
        if let Some(s) = surface {
            s.image = pic;
        }
    }
}
