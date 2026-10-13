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
