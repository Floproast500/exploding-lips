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
