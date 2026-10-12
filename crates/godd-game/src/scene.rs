//! The G.O.D.D. world as GPU scene data: each region's floor/ceiling outline triangulated
//! (holes and non-convex shapes included). Triangles index world points, so rigid motion of
//! points (RROTATE, translating regions) needs no re-triangulation.

use crate::world::World;
use engine_scene::polygon::{self, Outline};

#[derive(Debug, Clone, Default)]
pub struct RegionMesh {
    pub outline: Outline,
    /// Point-id triangles, counter-clockwise seen from above.
    pub triangles: Vec<[u32; 3]>,
}

/// Directed boundary edges of a region with the region on the right (the front side of a
/// wall is to the right of a→b). Internal walls (both sides the same region) are skipped.
pub fn region_edges(world: &World, region: usize, weld: &[u32]) -> Vec<(u32, u32)> {
    world.regions[region]
        .walls
        .iter()
        .filter_map(|&w| world.walls.get(w))
        .filter(|w| w.back != Some(w.front))
        .filter_map(|w| {
            let (a, b) = (weld[w.a] as u32, weld[w.b] as u32);
            if w.front == region {
                Some((a, b))
            } else if w.back == Some(region) {
                Some((b, a))
            } else {
                None
            }
        })
        .collect()
}

pub fn points(world: &World) -> Vec<[f64; 2]> {
    world.vertices.iter().map(|&(x, y)| [x as f64, y as f64]).collect()
}

/// Triangulate every region.
pub fn build(world: &World) -> Vec<RegionMesh> {
    let pts = points(world);
    let weld = polygon::weld(&pts);
    let mut earcut = earcut_state();
    (0..world.regions.len())
        .map(|r| {
            let (outline, triangles) = polygon::region_mesh(&pts, &region_edges(world, r, &weld), &mut earcut);
            RegionMesh { outline, triangles }
        })
        .collect()
}

fn earcut_state() -> engine_scene::Earcut {
    engine_scene::Earcut::new()
}
