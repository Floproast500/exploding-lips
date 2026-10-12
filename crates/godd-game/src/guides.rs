//! Progression guides (a remaster aid, `hud_guides`): where a level's way forward is. These are
//! walk-in teleporters, the level exit and switches, for glowing markers in the 3D view and
//! symbols on the automap. The original shows none of them: its teleporters are invisible and
//! its switches look like any other wall.

use crate::session::Session;

/// Object effects (re/engine/object-effects.md).
const OTELE: i32 = 15;
const OSTRAP: i32 = 29;
const OTAGSW: i32 = 20;
const OBUMPSW: i32 = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideKind {
    /// Walk into it to be teleported.
    Teleport,
    /// The level exit.
    Exit,
    /// Walk into (or shoot) it to open or start something.
    Switch,
}

impl GuideKind {
    /// Linear light colour of the marker.
    pub fn light(self) -> [f32; 3] {
        match self {
            Self::Teleport => [0.45, 1.1, 2.4],
            Self::Exit => [0.4, 2.4, 0.6],
            Self::Switch => [2.2, 1.3, 0.25],
        }
    }

    /// Display colour (automap symbols).
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::Teleport => [80, 170, 255],
            Self::Exit => [80, 255, 100],
            Self::Switch => [255, 180, 40],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub kind: GuideKind,
    pub pos: [f32; 3],
    pub region: Option<usize>,
}

/// Every guide of the running level. Named teleporters are left out: those are moved to by
/// scripts (respawn, traps), not walked into. The level-exit object (OSTRAP) sits outside the
/// map and fires on a tag; the switch wall that fires that tag is the exit.
pub fn find(session: &Session) -> Vec<Guide> {
    let mut guides = Vec::new();
    let mut exit_tags = Vec::new();
    for o in session.objects.list.iter().filter(|o| !o.destroyed) {
        let kind = match o.effect {
            OSTRAP => {
                exit_tags.push(o.tag);
                continue;
            }
            _ if o.region.is_none() => continue,
            // Without an aux record a teleporter does nothing (decoration in LIPS00's start room).
            OTELE if o.name.is_empty() && o.aux.is_some() => GuideKind::Teleport,
            OTAGSW | OBUMPSW if o.name.is_empty() => GuideKind::Switch,
            _ => continue,
        };
        guides.push(Guide { kind, pos: [o.x, o.y, o.z + o.height.max(32.0) / 2.0], region: o.region });
    }
    let world = &session.level.world;
    for sw in session.effects.switches().into_iter().filter(|s| s.usable && *s.tags.start() > 0) {
        let kind = if exit_tags.iter().any(|t| *t > 0 && sw.tags.contains(t)) { GuideKind::Exit } else { GuideKind::Switch };
        let wall = &world.walls[sw.wall];
        let (a, b) = (world.vertices[wall.a], world.vertices[wall.b]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        // Just in front of the wall, on the side of the region it bounds.
        let normal = (-dy / len * 12.0, dx / len * 12.0);
        let ahead = (mid.0 + normal.0, mid.1 + normal.1);
        let side = if world.region_at(ahead.0, ahead.1) == Some(wall.front) { 1.0 } else { -1.0 };
        let Some(region) = world.regions.get(wall.front) else { continue };
        let z = region.floor + ((region.ceiling - region.floor) / 2.0).min(48.0);
        guides.push(Guide { kind, pos: [mid.0 + normal.0 * side, mid.1 + normal.1 * side, z], region: Some(wall.front) });
    }
    guides
}

/// Up to `max` guides nearest to `at` within `range`, nearest first.
pub fn nearest(guides: &[Guide], at: [f32; 3], range: f32, max: usize) -> Vec<Guide> {
    let dist = |g: &Guide| (g.pos[0] - at[0]).hypot(g.pos[1] - at[1]);
    let mut near: Vec<Guide> = guides.iter().filter(|g| dist(g) < range).copied().collect();
    near.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
    near.truncate(max);
    near
}
