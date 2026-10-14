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

/// Doors that stay open once opened: (level, door region, its tag).
const DOORS_STAY_OPEN: &[(&str, usize, i32)] = &[
    // LIPS01: door r8 (tag 104, closes after 5 s) is the only way to the pickup that starts the
    // exit chain. Its switch, wall 302, is a stray line 450 units off the door that is only
    // touched from inside r49's open outline, which no walking route reaches; its other opener,
    // pickup #104, is on the ground floor far below. Once opened it stays open.
    ("LIPS01", 8, 104),
];

/// The doors of `level` that stay open once opened (with fixes on).
pub fn doors_staying_open(level: &str, doors: &[(usize, i32)]) -> Vec<usize> {
    DOORS_STAY_OPEN
        .iter()
        .filter(|(name, d, tag)| name.eq_ignore_ascii_case(level) && doors.contains(&(*d, *tag)))
        .map(|&(_, d, _)| d)
        .collect()
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
