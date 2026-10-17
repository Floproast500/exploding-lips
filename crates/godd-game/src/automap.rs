//! The automap (F2 / Tab): explored regions only, with darkened floor textures, walls by kind,
//! the player, other players and the player's markers. Zoom with +/- or the wheel; north-up or
//! rotating with the view (`am_rotate`).

use crate::scene::RegionMesh;
use crate::session::Session;
use crate::world::World;
use crate::{CLASSIC_HEIGHT, Frame};

#[derive(Debug, Default)]
pub struct Automap {
    /// Regions the player has seen (through portals) on this level.
    pub seen: Vec<bool>,
    /// Markers placed with the Mark action, in world coordinates.
    pub marks: Vec<(f32, f32)>,
}

pub struct Style {
    pub textured: bool,
    pub rotate: bool,
    /// Map units per screen height at zoom 1 is 1600.
    pub zoom: f32,
    /// Shade row for floors (darker than the view so walls stand out).
    pub floor_row: usize,
    /// UI colours: background, border, text, highlight.
    pub colors: (u8, u8, u8, u8),
    /// Progression guides (x, y, region, colour): shown once their region has been seen.
    pub guides: Vec<(f32, f32, Option<usize>, u8)>,
}

impl Automap {
    pub fn reset(&mut self, regions: usize) {
        self.seen = vec![false; regions];
        self.marks.clear();
    }

    /// Mark what is visible from the player now (portal reachability, generous field of view).
    pub fn reveal(&mut self, world: &World, cam: &crate::render::Camera) {
        if self.seen.len() != world.regions.len() {
            self.seen = vec![false; world.regions.len()];
        }
        if let Some(s) = self.seen.get_mut(cam.region) {
            *s = true;
        }
        for (r, windows) in crate::scene::visible_regions(world, cam, (2.4, 1.2), false).iter().enumerate() {
            if !windows.is_empty() {
                self.seen[r] = true;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, frame: &mut Frame, session: &Session, meshes: &[RegionMesh], shade_rows: &[[u8; 256]], style: &Style, others: &[(f32, f32, f32)]) {
        let world = &session.level.world;
        let p = &session.player;
        let scale = frame.height as f32 / 1600.0 * style.zoom;
        let (cx, cy) = (frame.width as f32 / 2.0, frame.height as f32 / 2.0);
        let turn = if style.rotate { p.angle - std::f32::consts::FRAC_PI_2 } else { 0.0 };
        let (sin, cos) = turn.sin_cos();
        let to_screen = |x: f32, y: f32| {
            let (dx, dy) = (x - p.x, y - p.y);
            let (rx, ry) = (dx * cos + dy * sin, -dx * sin + dy * cos);
            (cx + rx * scale, cy - ry * scale)
        };
        let from_screen = |sx: f32, sy: f32| {
            let (rx, ry) = ((sx - cx) / scale, -(sy - cy) / scale);
            (p.x + rx * cos - ry * sin, p.y + rx * sin + ry * cos)
        };
        let seen = |r: usize| self.seen.get(r).copied().unwrap_or(false);
        let level_ok = |r: usize| world.regions.get(r).is_some_and(|reg| reg.floor <= p.z + 96.0 && reg.ceiling >= p.z - 32.0);
        let (bg, _, portal, solid) = style.colors;
        text_fill(frame, bg);
        // Floors: darkened texture, sampled per pixel inside each seen region's triangles.
        if style.textured {
            let row = &shade_rows[style.floor_row.min(shade_rows.len().saturating_sub(1))];
            for (r, mesh) in meshes.iter().enumerate().filter(|(r, _)| seen(*r) && level_ok(*r)) {
                let reg = &world.regions[r];
                let Some(surface) = reg.floor_surface else { continue };
                let Some(image) = world.images.get(surface.image) else { continue };
                for t in &mesh.triangles {
                    let pts = t.map(|i| {
                        let (x, y) = world.vertices[i as usize];
                        to_screen(x, y)
                    });
                    fill_triangle(frame, pts, |sx, sy| {
                        let (wx, wy) = from_screen(sx, sy);
                        let u = ((wx + surface.offset_u) * surface.scale).floor() as i32;
                        let v = ((-wy + surface.offset_v) * surface.scale).floor() as i32;
                        match row[image.sample_wrapped(u, v) as usize] {
                            0 => 8,
                            c => c,
                        }
                    });
                }
            }
        }
        // Walls: solid outlines bright, steps between regions dimmer.
        for wall in &world.walls {
            let front_seen = seen(wall.front) && level_ok(wall.front);
            let back_seen = wall.back.is_some_and(|b| seen(b) && level_ok(b));
            if !front_seen && !back_seen {
                continue;
            }
            let two_sided = wall.back.is_some_and(|b| b != wall.front);
            let step = two_sided
                && wall.back.and_then(|b| world.regions.get(b)).is_some_and(|b| {
                    let f = &world.regions[wall.front];
                    (b.floor - f.floor).abs() > 1.0 || (b.ceiling - f.ceiling).abs() > 1.0
                });
            if two_sided && !step && style.textured {
                continue; // flat joins between regions only clutter a textured map
            }
            let color = if two_sided { portal } else { solid };
            let (ax, ay) = world.vertices[wall.a];
            let (bx, by) = world.vertices[wall.b];
            line(frame, to_screen(ax, ay), to_screen(bx, by), color);
        }
        let s = (frame.height / CLASSIC_HEIGHT).max(1) as i32;
        // Markers (numbered) and other players.
        for (i, &(mx, my)) in self.marks.iter().enumerate() {
            let (x, y) = to_screen(mx, my);
            crate::text::draw_text(frame, x as i32 - 3 * s, y as i32 - 4 * s, s as usize, portal, &(i + 1).to_string());
        }
        // Guides: a diamond each.
        for &(gx, gy, region, color) in &style.guides {
            if region.is_some_and(|r| !seen(r)) {
                continue;
            }
            let (x, y) = to_screen(gx, gy);
            let r = 3.0 * s as f32;
            for pts in [[(x, y - r), (x + r, y), (x - r, y)], [(x - r, y), (x + r, y), (x, y + r)]] {
                if let Some(list) = &mut frame.ui_list {
                    list.push(engine_core::UiCmd::Triangle { points: pts, color });
                } else {
                    fill_triangle(frame, pts, |_, _| color);
                }
            }
        }
        for &(ox, oy, oa) in others {
            arrow(frame, to_screen(ox, oy), oa - turn, s, portal);
        }
        arrow(frame, (cx, cy), p.angle - turn, s, solid);
        compass(frame, turn, s, style);
    }
}

impl Automap {
    /// The map while the player is in a level's BSP part (LIPS00 starts in one): its walls
    /// near the player's height seen from above, the exits, and the player, who is there and
    /// not in the orb's regions.
    pub fn draw_bsp(&self, frame: &mut Frame, world: &crate::bsp_world::BspWorld, style: &Style) {
        let p = &world.player;
        let feet = p.pos[2] - 24.0;
        let angle = p.orb_angle();
        let scale = frame.height as f32 / 1600.0 * style.zoom;
        let (cx, cy) = (frame.width as f32 / 2.0, frame.height as f32 / 2.0);
        let turn = if style.rotate { angle - std::f32::consts::FRAC_PI_2 } else { 0.0 };
        let (sin, cos) = turn.sin_cos();
        let to_screen = |x: f32, y: f32| {
            let (dx, dy) = (x - p.pos[0], y - p.pos[1]);
            let (rx, ry) = (dx * cos + dy * sin, -dx * sin + dy * cos);
            (cx + rx * scale, cy - ry * scale)
        };
        let (bg, _, portal, solid) = style.colors;
        text_fill(frame, bg);
        // Walls (faces standing up) around the player's height; floors and ceilings are left out.
        for face in &world.bsp.faces[world.bsp.world_faces.clone()] {
            let (lo, hi) = face.points.iter().fold((f32::MAX, f32::MIN), |(l, h), q| (l.min(q[2]), h.max(q[2])));
            if hi - lo < 8.0 || hi < feet - 48.0 || lo > feet + 160.0 {
                continue;
            }
            let n = face.points.len();
            for i in 0..n {
                let (a, b) = (face.points[i], face.points[(i + 1) % n]);
                if (a[0] - b[0]).abs() + (a[1] - b[1]).abs() < 1.0 {
                    continue; // (a vertical edge: a point from above)
                }
                line(frame, to_screen(a[0], a[1]), to_screen(b[0], b[1]), solid);
            }
        }
        let s = (frame.height / CLASSIC_HEIGHT).max(1) as i32;
        // The exits: a diamond each, in the guides' exit colour when guides are on.
        let exit_color = style.guides.first().map_or(portal, |g| g.3);
        for c in world.exits() {
            let (x, y) = to_screen(c.centre[0], c.centre[1]);
            let r = 3.0 * s as f32;
            for pts in [[(x, y - r), (x + r, y), (x - r, y)], [(x - r, y), (x + r, y), (x, y + r)]] {
                if let Some(list) = &mut frame.ui_list {
                    list.push(engine_core::UiCmd::Triangle { points: pts, color: exit_color });
                } else {
                    fill_triangle(frame, pts, |_, _| exit_color);
                }
            }
        }
        arrow(frame, (cx, cy), angle - turn, s, solid);
        compass(frame, turn, s, style);
    }
}

/// A compass in the top left: N, E, S and W round a ring, an arrow to north (+Y). It turns
/// with the map when it rotates with the view.
fn compass(frame: &mut Frame, turn: f32, s: i32, style: &Style) {
    use std::f32::consts::{FRAC_PI_2, PI};
    let (_, border, text, highlight) = style.colors;
    let r = 14.0 * s as f32;
    let c = (r + 10.0 * s as f32, r + 10.0 * s as f32);
    // (Screen angles: 0 = right, counter-clockwise; world north is +Y, up when not turned.)
    let at = |a: f32, d: f32| (c.0 + a.cos() * d, c.1 - a.sin() * d);
    for k in 0..32 {
        let (a, b) = (k as f32 / 32.0 * 2.0 * PI, (k + 1) as f32 / 32.0 * 2.0 * PI);
        line(frame, at(a, r * 0.72), at(b, r * 0.72), border);
    }
    let north = FRAC_PI_2 - turn;
    arrow(frame, at(north, r * 0.32), north, s, highlight);
    for (k, letter) in ["N", "W", "S", "E"].iter().enumerate() {
        let a = north + k as f32 * FRAC_PI_2;
        let (x, y) = at(a, r * 1.05);
        let half = 4 * s;
        crate::text::draw_text(frame, x as i32 - half, y as i32 - half, s as usize, if k == 0 { highlight } else { text }, letter);
    }
}

fn text_fill(frame: &mut Frame, color: u8) {
    // A dark backdrop so the map reads over the view.
    for o in 0..frame.pixels.len() {
        frame.put(o, color);
    }
}

fn line(frame: &mut Frame, (x0, y0): (f32, f32), (x1, y1): (f32, f32), color: u8) {
    if let Some(list) = &mut frame.ui_list {
        list.push(engine_core::UiCmd::Line { from: (x0, y0), to: (x1, y1), width: 0.5, color });
        return;
    }
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as usize;
    if steps > 8000 {
        return;
    }
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
        if x >= 0.0 && y >= 0.0 && (x as usize) < frame.width && (y as usize) < frame.height {
            frame.put(y as usize * frame.width + x as usize, color);
        }
    }
}

/// Fill a screen-space triangle, colouring each covered pixel centre with `shade(x, y)`.
fn fill_triangle(frame: &mut Frame, [a, b, c]: [(f32, f32); 3], mut shade: impl FnMut(f32, f32) -> u8) {
    let area = (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
    if area.abs() < 1e-6 {
        return;
    }
    let x0 = a.0.min(b.0).min(c.0).floor().max(0.0) as usize;
    let x1 = (a.0.max(b.0).max(c.0).ceil() as usize).min(frame.width);
    let y0 = a.1.min(b.1).min(c.1).floor().max(0.0) as usize;
    let y1 = (a.1.max(b.1).max(c.1).ceil() as usize).min(frame.height);
    let edge = |p: (f32, f32), q: (f32, f32), x: f32, y: f32| ((q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0)) * area.signum();
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            if edge(a, b, px, py) >= 0.0 && edge(b, c, px, py) >= 0.0 && edge(c, a, px, py) >= 0.0 {
                let color = shade(px, py);
                frame.put(y * frame.width + x, color);
            }
        }
    }
}

/// A small arrow at `pos` pointing along `angle` (screen: 0 = right, counter-clockwise).
fn arrow(frame: &mut Frame, (cx, cy): (f32, f32), angle: f32, s: i32, color: u8) {
    let (dx, dy) = (angle.cos(), -angle.sin());
    let len = 4.0 * s as f32;
    let tip = (cx + dx * len, cy + dy * len);
    let back = (cx - dx * len * 0.6, cy - dy * len * 0.6);
    let side = (-dy * len * 0.5, dx * len * 0.5);
    let points = [tip, (back.0 + side.0, back.1 + side.1), (back.0 - side.0, back.1 - side.1)];
    if let Some(list) = &mut frame.ui_list {
        list.push(engine_core::UiCmd::Triangle { points, color });
        return;
    }
    fill_triangle(frame, points, |_, _| color);
}
