//! Data fixes for levels of the original game that cannot be finished as shipped (cvar
//! `g_fixes`, on by default; off = the data exactly as lips.exe has it).
//!
//! Each fix names the level and the walls it changes, and checks they still join the regions
//! it expects, so a modded level with other geometry is left alone.

use crate::world::World;

/// Walls made passable: (level, wall, front region, back region).
const OPEN_WALLS: &[(&str, usize, usize, usize)] = &[
    // LIPS02: the block r24 in the wall between the east corridor and the sealed west room
    // (both region 25) is the only way to the pickup that opens door r13 (tag 29), whose other
    // switch is behind that door; its four walls are flagged solid, so the level cannot be
    // finished. They become passable, a secret passage into the room.
    ("LIPS02", 1539, 25, 24),
    ("LIPS02", 1540, 25, 24),
    ("LIPS02", 1541, 25, 24),
    ("LIPS02", 1542, 25, 24),
];

/// Door faces that become the door's switch: (level, face wall, switch wall whose effect it
/// takes with fixes on, wall whose effect it has without).
const SWITCH_FACES: &[(&str, usize, usize, usize)] = &[
    // LIPS01: doors r8 (tag 104) and r39 (tag 28) are on the only way to the pickup that
    // starts the exit chain. Their faces on the way in only knock (tag 7, a "locked" sound);
    // the switches meant to open them, walls 302 and 606, are stray lines off the doors that
    // are only touched from inside r49's and r51's open outlines, where no walking route goes
    // (the original confirms: walking into these faces does nothing, and wall 302 opens the
    // door only when the player is poked into r49 beside it). The faces open their doors, as
    // the far face of r39 (wall 230) does.
    ("LIPS01", 112, 302, 192),
    ("LIPS01", 134, 302, 192),
    ("LIPS01", 231, 230, 192),
];

/// Give (`on`) or take back the door faces' switches of `level` (see `SWITCH_FACES`).
pub fn apply_effects(level: &str, effects: &mut crate::effects::Effects, on: bool) {
    for &(name, face, fixed, classic) in SWITCH_FACES {
        if name.eq_ignore_ascii_case(level) {
            effects.copy_wall_effect(face, if on { fixed } else { classic });
        }
    }
}

/// Apply (`on`) or undo the fixes for `level`. Returns how many walls changed.
pub fn apply(level: &str, world: &mut World, on: bool) -> usize {
    let mut changed = 0;
    for &(name, wall, front, back) in OPEN_WALLS {
        if !name.eq_ignore_ascii_case(level) {
            continue;
        }
        let Some(w) = world.walls.get_mut(wall) else { continue };
        if w.front != front || w.back != Some(back) {
            continue;
        }
        if w.blocking == on {
            w.blocking = !on;
            changed += 1;
        }
    }
    changed
}
