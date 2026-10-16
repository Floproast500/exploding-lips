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
    /// Edges of the outline with no wall (a boundary with a wall missing, closed straight
    /// across): the original's trace sees through them into darkness.
    pub gaps: Vec<(u32, u32)>,
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
            let (a, b) = (weld[w.a], weld[w.b]);
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
            let edges = region_edges(world, r, &weld);
            let (outline, triangles) = polygon::region_mesh(&pts, &edges, &mut earcut);
            let mut gaps = Vec::new();
            if outline.open_edges > 0 {
                let rings = outline.parts.iter().flat_map(|p| std::iter::once(&p.outer).chain(&p.holes));
                for ring in rings {
                    for i in 0..ring.len() {
                        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                        if !edges.iter().any(|&e| e == (a, b) || e == (b, a)) {
                            gaps.push((a, b));
                        }
                    }
                }
            }
            RegionMesh { outline, triangles, gaps }
        })
        .collect()
}

fn earcut_state() -> engine_scene::Earcut {
    engine_scene::Earcut::new()
}

use crate::render::Camera;
use crate::world::Surface;
use engine_scene::scene::{CLAMP, DISCARD_OUTSIDE_V, NO_DISTANCE, OBJECT, SCREEN, SKY, SceneFrame, SceneVertex, TRANSPARENT, VOID};
use std::f32::consts::TAU;

/// Sprites nearer than this (view depth) are skipped, like the software renderer's NEAR.
const SPRITE_NEAR: f32 = 8.0;

/// Image index used for sky geometry (the shader ignores it).
const NO_IMAGE: u32 = u32::MAX;

/// Regions visible from the camera through portals, each with the convex screen windows
/// (normalized device coordinates, counter-clockwise) it is seen through. This is the software
/// renderer's column trace in polygon form: a portal's opening (the wall between the higher
/// floor and the lower ceiling) projects to a convex quad, and nested portals intersect. Regions
/// that overlap in plan but are not reached never show, and a neighbour's floor or ceiling is
/// limited to the opening it is seen through. `tan_half` = tangents of the half fields of view.
pub fn visible_regions(world: &World, cam: &Camera, (tan_half_h, tan_half_v): (f32, f32), freelook: bool) -> Vec<Vec<Vec<[f32; 2]>>> {
    use engine_scene::scene::{clip_convex, convex_hull, polygon_area};
    const NEAR: f32 = 1.0;
    const MAX_DEPTH: usize = 128;
    const MAX_WINDOWS: usize = 16;
    let mut windows: Vec<Vec<Vec<[f32; 2]>>> = vec![Vec::new(); world.regions.len()];
    if world.regions.is_empty() {
        return windows;
    }
    let (sin, cos) = cam.angle.sin_cos();
    let (psin, pcos) = cam.pitch.atan().sin_cos();
    // World point -> (forward, right, up) relative to the eye (rotated by the pitch in freelook).
    let view = |(x, y): (f32, f32), z: f32| {
        let (rx, ry) = (x - cam.x, y - cam.y);
        let (f, up) = (rx * cos + ry * sin, z - cam.z);
        if freelook { [f * pcos + up * psin, rx * sin - ry * cos, up * pcos - f * psin] } else { [f, rx * sin - ry * cos, up] }
    };
    let shear = if freelook { 0.0 } else { cam.pitch };
    let project = |p: [f32; 3]| [p[1] / p[0] / tan_half_h, (p[2] / p[0] - shear) / tan_half_v];
    // Screen polygon of a portal opening, clipped to the near plane.
    let opening = |a: (f32, f32), b: (f32, f32), z0: f32, z1: f32| -> Vec<[f32; 2]> {
        let quad = [view(a, z0), view(b, z0), view(b, z1), view(a, z1)];
        let mut pts: Vec<[f32; 3]> = Vec::with_capacity(6);
        for i in 0..4 {
            let (p, q) = (quad[i], quad[(i + 1) % 4]);
            if p[0] >= NEAR {
                pts.push(p);
            }
            if (p[0] >= NEAR) != (q[0] >= NEAR) {
                let t = (NEAR - p[0]) / (q[0] - p[0]);
                pts.push([NEAR, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t]);
            }
        }
        let mut poly: Vec<[f32; 2]> = pts.into_iter().map(project).collect();
        if polygon_area(&poly) < 0.0 {
            poly.reverse();
        }
        poly
    };
    let contains = |outer: &[[f32; 2]], inner: &[[f32; 2]]| {
        inner.iter().all(|&v| {
            (0..outer.len()).all(|i| {
                let (p, q) = (outer[i], outer[(i + 1) % outer.len()]);
                (q[0] - p[0]) * (v[1] - p[1]) - (q[1] - p[1]) * (v[0] - p[0]) >= -1e-4
            })
        })
    };
    let screen = vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
    let start = cam.region.min(world.regions.len() - 1);
    let mut stack = vec![(start, screen.clone(), 0usize)];
    // An eye on (or right by) a portal line sees that line's opening edge-on: clipped at the
    // near plane it collapses, and everything beyond would go missing (a void beside a wall at
    // some angles). The regions across such lines start with the whole screen too.
    for &w in &world.regions[start].walls {
        let wall = &world.walls[w];
        let Some(back) = wall.back.filter(|&b| b != wall.front) else { continue };
        let other = if wall.front == start { back } else { wall.front };
        let ((ax, ay), (bx, by)) = (world.vertices[wall.a], world.vertices[wall.b]);
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((cam.x - ax) * dx + (cam.y - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
        if (cam.x - ax - t * dx).hypot(cam.y - ay - t * dy) < 16.0 && other < world.regions.len() {
            stack.push((other, screen.clone(), 1));
        }
    }
    while let Some((r, window, depth)) = stack.pop() {
        if depth > MAX_DEPTH || windows[r].iter().any(|w| contains(w, &window)) {
            continue;
        }
        // Past the cap a window joins the last one's hull rather than being lost (LIPS00's coffin,
        // seen through the many edges of the room around it, lost its ceiling: a black hole).
        // The walk goes on with the whole hull: what lies beyond is seen through it too (the
        // street behind LIPS00's canteen sign), and later views inside it stop at the check above.
        let capped = windows[r].len() >= MAX_WINDOWS;
        let mut window = window;
        if capped {
            let last = windows[r].last_mut().expect("capped");
            *last = convex_hull(&[&last[..], &window[..]].concat());
            window = last.clone();
        }
        let reg = &world.regions[r];
        for link in [reg.above, reg.below].into_iter().flatten() {
            if link < world.regions.len() {
                stack.push((link, window.clone(), depth + 1));
            }
        }
        for &w in &reg.walls {
            let wall = &world.walls[w];
            let other = match wall.back {
                Some(b) if b != wall.front && wall.front == r => b,
                Some(b) if b != wall.front && b == r => wall.front,
                _ => continue,
            };
            let Some(far) = world.regions.get(other) else { continue };
            let (z0, z1) = (reg.floor.max(far.floor), reg.ceiling.min(far.ceiling));
            if z1 <= z0 {
                continue; // closed: a shut door or a step to the ceiling
            }
            let poly = opening(world.vertices[wall.a], world.vertices[wall.b], z0, z1);
            if poly.len() < 3 {
                continue;
            }
            let through = clip_convex(&poly, &window);
            if through.len() >= 3 && polygon_area(&through) > 1e-7 {
                stack.push((other, through, depth + 1));
            }
        }
        if capped {
            continue;
        }
        // A window that joins one already here into a convex shape (strips of one opening
        // split by the walls between) merges with it: fewer windows, the same area.
        let area = polygon_area(&window);
        let bounds = |p: &[[f32; 2]]| p.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, v| [b[0].min(v[0]), b[1].min(v[1]), b[2].max(v[0]), b[3].max(v[1])]);
        let nb = bounds(&window);
        let joined = windows[r].iter().enumerate().find_map(|(k, w)| {
            let wb = bounds(w);
            if wb[0] > nb[2] + 1e-4 || nb[0] > wb[2] + 1e-4 || wb[1] > nb[3] + 1e-4 || nb[1] > wb[3] + 1e-4 {
                return None; // apart: no shared edge
            }
            let hull = convex_hull(&[&w[..], &window[..]].concat());
            (polygon_area(&hull) <= (polygon_area(w) + area) * 1.0005 + 1e-6).then_some((k, hull))
        });
        match joined {
            Some((k, hull)) => windows[r][k] = hull,
            None => windows[r].push(window),
        }
    }
    windows
}

/// One frame's GPU scene, following the software renderer's rules (render.rs): texture
/// anchors, shading per wall/flat, sky where a surface or one-sided wall has no texture,
/// see-through stacked floors, masked middles, and sprites facing the camera plane. Each
/// region visible through portals is a batch clipped to its window; sprites come last,
/// unclipped and depth-tested, as in the software renderer.
pub fn frame(world: &World, meshes: &[RegionMesh], cam: &Camera, tan_half: (f32, f32), freelook: bool, out: &mut SceneFrame) {
    out.vertices.clear();
    out.batches.clear();
    out.sky = world.sky.filter(|&i| i < world.images.len()).map(|i| i as u32);
    let image = |s: &Surface| world.images.get(s.image).map(|i| (s.image as u32, i.width as f32, i.height as f32));
    // Sky behind everything: a view between portals that no region's geometry covers (looking
    // up through a gap at a sky ceiling over regions not walked to) shows sky, as in the
    // software renderer, not black.
    if out.sky.is_some() {
        let corner = |x: f32, y: f32| SceneVertex { pos: [x, y, 0.0], image: NO_IMAGE, flags: SKY | SCREEN, ..Default::default() };
        out.quad([corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)]);
        out.batch(0, None);
    }
    let windows = visible_regions(world, cam, tan_half, freelook);
    for (r, region_windows) in windows.iter().enumerate() {
        if region_windows.is_empty() {
            continue;
        }
        let first = out.vertices.len();
        let reg = &world.regions[r];
        // Floor and ceiling.
        let base = if reg.fullbright { NO_DISTANCE } else { 0 };
        for (z, surface, light, link, up) in [(reg.floor, reg.floor_surface, reg.floor_light, reg.below, 1.0), (reg.ceiling, reg.ceiling_surface, reg.ceiling_light, reg.above, -1.0)] {
            if link.is_some() {
                continue; // see-through into the stacked region
            }
            let tex = surface.and_then(|s| image(&s).map(|t| (s, t)));
            for t in meshes.get(r).map_or(&[][..], |m| &m.triangles) {
                for &p in t {
                    let (x, y) = world.vertices[p as usize];
                    out.vertices.push(match tex {
                        Some((s, (img, ..))) => SceneVertex {
                            pos: [x, y, z],
                            uv: [(x + s.offset_u) * s.scale, (-y + s.offset_v) * s.scale],
                            image: img,
                            light: light as f32,
                            flags: base,
                            normal: [0.0, 0.0, up],
                        },
                        None => SceneVertex { pos: [x, y, z], image: NO_IMAGE, flags: SKY, ..Default::default() },
                    });
                }
            }
        }
        // Walls as seen from this region.
        for &w in &reg.walls {
            let wall = &world.walls[w];
            let ((ax, ay), (bx, by)) = (world.vertices[wall.a], world.vertices[wall.b]);
            let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            // Facing into this region (the front region lies right of a→b).
            let side = if wall.front == r { 1.0 } else { -1.0 } / len.max(1e-6);
            let normal = [(by - ay) * side, -(bx - ax) * side, 0.0];
            let section = |out: &mut SceneFrame, surface: Option<Surface>, (z0, z1): (f32, f32), anchor: f32, flags: u32| {
                if z1 <= z0 {
                    return;
                }
                let distance = if reg.fullbright { NO_DISTANCE } else { 0 };
                let corners = [(ax, ay, 0.0, z0), (bx, by, len, z0), (bx, by, len, z1), (ax, ay, 0.0, z1)];
                match surface {
                    Some(s) => {
                        let Some((img, ..)) = image(&s) else { return }; // missing texture: not drawn
                        out.quad(corners.map(|(x, y, along, z)| SceneVertex {
                            pos: [x, y, z],
                            uv: [(along + s.offset_u) * s.scale, (anchor - z + s.offset_v) * s.scale],
                            image: img,
                            light: wall.light as f32,
                            flags: flags | distance,
                            normal,
                        }));
                    }
                    None => out.quad(corners.map(|(x, y, _, z)| SceneVertex {
                        pos: [x, y, z],
                        image: NO_IMAGE,
                        flags: if flags == 0 { SKY } else { flags },
                        ..Default::default()
                    })),
                }
            };
            let height = |s: Option<Surface>| s.and_then(|s| image(&s)).map_or(0.0, |(_, _, h)| h);
            let far = match wall.back {
                None => {
                    if wall.front == r {
                        section(out, wall.middle, (reg.floor, reg.ceiling), reg.ceiling, 0);
                    }
                    continue;
                }
                Some(b) if b == wall.front => {
                    if wall.middle.is_some() {
                        section(out, wall.middle, (reg.floor, reg.ceiling), reg.ceiling, TRANSPARENT | DISCARD_OUTSIDE_V);
                    }
                    continue;
                }
                Some(b) => {
                    let other = if wall.front == r { b } else if b == r { wall.front } else { continue };
                    let Some(far) = world.regions.get(other) else { continue };
                    far
                }
            };
            // A step without a texture: sky under a sky ceiling, otherwise the gap the software
            // renderer never draws (VOID).
            if far.ceiling < reg.ceiling {
                match wall.upper {
                    Some(_) => section(out, wall.upper, (far.ceiling, reg.ceiling), far.ceiling + height(wall.upper), 0),
                    None if reg.ceiling_surface.is_none() => section(out, None, (far.ceiling, reg.ceiling), 0.0, SKY),
                    None => section(out, None, (far.ceiling, reg.ceiling), 0.0, VOID),
                }
            }
            if far.floor > reg.floor {
                match wall.lower {
                    Some(_) => section(out, wall.lower, (reg.floor, far.floor), far.floor, 0),
                    None => section(out, None, (reg.floor, far.floor), 0.0, VOID),
                }
            }
            if wall.masked && wall.middle.is_some() {
                let top = reg.ceiling.min(far.ceiling);
                section(out, wall.middle, (reg.floor.max(far.floor), top), top, TRANSPARENT | DISCARD_OUTSIDE_V);
            }
        }
        // A missing wall: dark, as the original's flats fading out past it.
        for &(a, b) in meshes.get(r).map_or(&[][..], |m| &m.gaps) {
            let ((ax, ay), (bx, by)) = (world.vertices[a as usize], world.vertices[b as usize]);
            let corner = |x: f32, y: f32, z: f32| SceneVertex { pos: [x, y, z], image: NO_IMAGE, flags: VOID, ..Default::default() };
            out.quad([corner(ax, ay, reg.floor), corner(bx, by, reg.floor), corner(bx, by, reg.ceiling), corner(ax, ay, reg.ceiling)]);
        }
        for window in region_windows {
            out.batch(first, Some(window.clone()));
        }
    }
    // Sprites: camera-facing quads, rotation frame chosen from the bearing to the camera.
    let first = out.vertices.len();
    let (sin, cos) = cam.angle.sin_cos();
    let right = (sin, -cos);
    for s in world.sprites.iter().filter(|s| s.visible && !s.frames.is_empty()) {
        let (rx, ry) = (s.x - cam.x, s.y - cam.y);
        if rx * cos + ry * sin <= SPRITE_NEAR {
            continue; // as the software renderer: sprites this close are not drawn
        }
        let n = s.frames.len();
        let rel = (s.angle - (-ry).atan2(-rx)).rem_euclid(TAU);
        let (img, mirrored) = s.frames[((rel / TAU * n as f32) + 0.5) as usize % n];
        let Some(image) = world.images.get(img) else { continue };
        let (w, h) = (image.width as f32 * s.scale, image.height as f32 * s.scale);
        let (light, fullbright) = world.regions.get(s.region).map_or((0, false), |r| (r.floor_light, r.fullbright));
        let flags = TRANSPARENT | CLAMP | OBJECT | if fullbright { NO_DISTANCE } else { 0 };
        let (u0, u1) = if mirrored { (image.width as f32, 0.0) } else { (0.0, image.width as f32) };
        let corner = |side: f32, top: bool, u: f32| SceneVertex {
            pos: [s.x + right.0 * side * w / 2.0, s.y + right.1 * side * w / 2.0, if top { s.z + h } else { s.z }],
            uv: [u, if top { 0.0 } else { image.height as f32 }],
            image: img as u32,
            light: light as f32,
            flags,
            normal: [-cos, -sin, 0.0],
        };
        out.quad([corner(-1.0, false, u0), corner(1.0, false, u1), corner(1.0, true, u1), corner(-1.0, true, u0)]);
    }
    out.batch(first, None);
}
