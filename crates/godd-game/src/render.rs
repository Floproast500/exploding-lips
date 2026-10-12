//! Software renderer: per-column ray casting through region portals, perspective floors and
//! ceilings, a panoramic sky, depth-tested sprites and masked walls. Works at any resolution:
//! the vertical field of view is fixed and the horizontal one follows the aspect ratio (Hor+).
//!
//! Behaviour follows re/engine/renderer.md: a wall's front side is right of p1→p2; one texel
//! per world unit; shade = clamp(light + depth / visibility, 0, 15) with light stored as
//! darkness; the sky is unlit, linear in angle and centred on the horizon.

use crate::world::{Image, Surface, TRANSPARENT, World};
use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    /// Eye height.
    pub z: f32,
    pub angle: f32,
    /// Look up/down as a y-shear, in units of the focal length (the original shears; it has
    /// no true pitch).
    pub pitch: f32,
    pub region: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct RenderSettings {
    /// Vertical field of view in degrees. The original uses focal = (W/2)·cot 36° on a 320x200
    /// buffer: 72° horizontally, 2·atan(100/220.2) ≈ 48.9° vertically.
    pub vfov_degrees: f32,
    /// World units per shade level of distance darkening ("visibility", 768 in every level).
    pub visibility: f32,
    /// Added to every shade value.
    pub fade_bias: i32,
    /// Continuous RGB lighting instead of the 16 palette shade tables.
    pub true_color: bool,
    /// Lightning (OFLASH): every surface full bright.
    pub fullbright: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self { vfov_degrees: 48.9, visibility: 768.0, fade_bias: 0, true_color: false, fullbright: false }
    }
}

/// Shade tables: `rows[level][index]`, level 0 = brightest.
pub struct Shading<'a> {
    pub rows: &'a [[u8; 256]],
}

impl Shading<'_> {
    #[inline]
    fn row(&self, level: i32) -> &[u8; 256] {
        &self.rows[level.clamp(0, self.rows.len() as i32 - 1) as usize]
    }
}

/// Rectangle inside the frame that receives the 3D view.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

const MAX_PORTAL_DEPTH: usize = 128;
/// Nesting limit for floor/ceiling portals into stacked regions.
const MAX_STACK_DEPTH: u8 = 6;

/// State for tracing one column through a region and its portals.
#[derive(Clone, Copy)]
struct Trace {
    column: usize,
    ray: (f32, f32),
    region: usize,
    ytop: i32,
    ybot: i32,
    /// Ray distance at which the trace enters `region`.
    start_t: f32,
    /// Stacked region we came from (not traced back into).
    from: Option<usize>,
    stack_depth: u8,
}

#[derive(Clone, Copy)]
struct FlatContext {
    column: usize,
    ray: (f32, f32),
    region: usize,
    entry_t: f32,
    from: Option<usize>,
    stack_depth: u8,
}
/// Near clipping distance (original: 8).
const NEAR: f32 = 8.0;
/// A lit colour of 0 would read as "transparent/unwritten"; the original substitutes 8.
const ZERO_SUBSTITUTE: u8 = 8;
/// Objects never shade darker than this level.
const MAX_OBJECT_SHADE: i32 = 14;

#[inline]
fn lit(row: &[u8; 256], texel: u8) -> u8 {
    match row[texel as usize] {
        0 => ZERO_SUBSTITUTE,
        c => c,
    }
}

struct MaskedSpan {
    column: usize,
    dist: f32,
    y0: i32,
    y1: i32,
    surface: Surface,
    u: f32,
    top: f32,
    shade: f32,
}

#[derive(Default)]
pub struct Renderer {
    depth: Vec<f32>,
    masked: Vec<MaskedSpan>,
    sprite_order: Vec<(f32, usize)>,
}

struct View<'a> {
    cam: Camera,
    focal: f32,
    horizon: f32,
    dir: (f32, f32),
    plane: (f32, f32),
    width: usize,
    height: usize,
    shading: &'a Shading<'a>,
    settings: &'a RenderSettings,
    vp: Viewport,
}

impl View<'_> {
    /// Screen row of world height `z` at perpendicular distance `dist`.
    #[inline]
    fn row_of(&self, z: f32, dist: f32) -> f32 {
        self.horizon - (z - self.cam.z) * self.focal / dist
    }

    /// Shade level (0 = brightest) as a continuous value; classic mode truncates it.
    #[inline]
    fn shade(&self, light: i32, dist: f32, fullbright: bool) -> f32 {
        if self.settings.fullbright {
            return 0.0;
        }
        let distance = if fullbright { 0.0 } else { dist / self.settings.visibility };
        let distance = if self.settings.true_color { distance } else { distance.floor() };
        (light + self.settings.fade_bias) as f32 + distance
    }

    /// Write a lit texel: shade-table lookup in classic mode, scaled RGB in true colour.
    #[inline]
    fn paint(&self, frame: &mut crate::Frame, offset: usize, texel: u8, shade: f32) {
        if self.settings.true_color {
            let f = ((self.shading.rows.len() as f32 - shade) / self.shading.rows.len() as f32).clamp(0.0, 1.0);
            let [r, g, b] = frame.palette[texel as usize];
            frame.put_rgb(offset, [(r as f32 * f) as u8, (g as f32 * f) as u8, (b as f32 * f) as u8]);
            frame.pixels[offset] = texel;
        } else {
            frame.put(offset, lit(self.shading.row(shade as i32), texel));
        }
    }

    #[inline]
    fn out_index(&self, frame_width: usize, column: usize, y: usize) -> usize {
        (self.vp.y + y) * frame_width + self.vp.x + column
    }
}

impl Renderer {
    pub fn render(
        &mut self,
        world: &World,
        cam: &Camera,
        shading: &Shading,
        settings: &RenderSettings,
        frame: &mut crate::Frame,
        vp: Viewport,
    ) {
        let (width, height) = (vp.width, vp.height);
        if width == 0 || height == 0 || world.regions.is_empty() || shading.rows.is_empty() {
            return;
        }
        self.depth.clear();
        self.depth.resize(width * height, f32::INFINITY);
        self.masked.clear();

        let focal = height as f32 / 2.0 / (settings.vfov_degrees.to_radians() / 2.0).tan();
        let tan_half_h = width as f32 / 2.0 / focal;
        let (sin, cos) = cam.angle.sin_cos();
        let view = View {
            cam: *cam,
            focal,
            horizon: height as f32 / 2.0 + cam.pitch * focal,
            dir: (cos, sin),
            // Screen right is clockwise from the view direction.
            plane: (sin * tan_half_h, -cos * tan_half_h),
            width,
            height,
            shading,
            settings,
            vp,
        };

        for column in 0..width {
            self.render_column(world, &view, frame, column);
        }
        self.render_masked(world, &view, frame);
        self.render_sprites(world, &view, frame);
    }

    fn render_column(&mut self, world: &World, view: &View, frame: &mut crate::Frame, column: usize) {
        let k = 2.0 * (column as f32 + 0.5) / view.width as f32 - 1.0;
        let ray = (view.dir.0 + view.plane.0 * k, view.dir.1 + view.plane.1 * k);
        let region = view.cam.region.min(world.regions.len() - 1);
        let trace = Trace { column, ray, region, ytop: 0, ybot: view.height as i32, start_t: 0.0, from: None, stack_depth: 0 };
        self.trace(world, view, frame, trace);
    }

    /// Trace one column through `region` and its portals within rows `ytop..ybot`, starting
    /// where the ray is at distance `start_t`. Floors/ceilings of stacked regions recurse.
    fn trace(&mut self, world: &World, view: &View, frame: &mut crate::Frame, tr: Trace) {
        let Trace { column, ray, mut region, mut ytop, mut ybot, start_t, mut from, stack_depth } = tr;
        let (cx, cy) = (view.cam.x, view.cam.y);
        let mut last_t = start_t;

        for _ in 0..MAX_PORTAL_DEPTH {
            let reg = &world.regions[region];
            // Nearest wall of this region hit by the ray beyond the previous hit.
            let mut hit: Option<(f32, f32, usize)> = None;
            for &w in &reg.walls {
                let wall = &world.walls[w];
                let (ax, ay) = world.vertices[wall.a];
                let (bx, by) = world.vertices[wall.b];
                let (ex, ey) = (bx - ax, by - ay);
                let denom = ray.0 * ey - ray.1 * ex;
                if denom.abs() < 1e-9 {
                    continue;
                }
                let (qx, qy) = (ax - cx, ay - cy);
                let t = (qx * ey - qy * ex) / denom;
                let s = (qx * ray.1 - qy * ray.0) / denom;
                if t > last_t + 1e-3 && (0.0..=1.0).contains(&s) && hit.is_none_or(|h| t < h.0) {
                    hit = Some((t, s, w));
                }
            }
            let entry_t = last_t;
            let Some((t, s, w)) = hit else {
                let ctx = FlatContext { column, ray, region, entry_t, from, stack_depth };
                self.flats(world, view, frame, ctx, ytop, ybot, None);
                return;
            };
            last_t = t;
            let dist = t.max(NEAR);
            let wall = &world.walls[w];
            let y_ceil = view.row_of(reg.ceiling, dist);
            let y_floor = view.row_of(reg.floor, dist);
            let ctx = FlatContext { column, ray, region, entry_t, from, stack_depth };
            self.flats(world, view, frame, ctx, ytop, ybot, Some((y_ceil, y_floor)));

            let (ax, ay) = world.vertices[wall.a];
            let (bx, by) = world.vertices[wall.b];
            let along = s * ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            let shade = view.shade(wall.light, dist, reg.fullbright);

            // The region on the far side of this wall, if it is a portal.
            let other = match wall.back {
                Some(back) if back != wall.front => Some(if region == wall.front { back } else { wall.front }),
                _ => None,
            };
            match other.filter(|&o| o < world.regions.len()) {
                Some(next) => {
                    let bk = &world.regions[next];
                    let y_bceil = view.row_of(bk.ceiling, dist);
                    let y_bfloor = view.row_of(bk.floor, dist);
                    if bk.ceiling < reg.ceiling
                        && let Some(surface) = wall.upper
                    {
                        // Upper part: texture bottom at the lower (far) ceiling.
                        let anchor = bk.ceiling + world.images.get(surface.image).map_or(0.0, |i| i.height as f32);
                        self.wall_span(world, view, frame, column, dist, (y_ceil, y_bceil), (ytop, ybot), surface, along, anchor, shade);
                    }
                    if bk.floor > reg.floor
                        && let Some(surface) = wall.lower
                    {
                        // Lower part: texture top at the higher floor.
                        self.wall_span(world, view, frame, column, dist, (y_bfloor, y_floor), (ytop, ybot), surface, along, bk.floor, shade);
                    }
                    let new_top = ytop.max(y_ceil.max(y_bceil).ceil() as i32);
                    let new_bot = ybot.min(y_floor.min(y_bfloor).ceil() as i32);
                    if wall.masked
                        && let Some(surface) = wall.middle
                    {
                        let top = reg.ceiling.min(bk.ceiling);
                        self.masked.push(MaskedSpan { column, dist, y0: new_top, y1: new_bot, surface, u: along, top, shade });
                    }
                    ytop = new_top;
                    ybot = new_bot;
                    from = None;
                    region = next;
                    if ytop >= ybot {
                        return;
                    }
                }
                None if wall.back == Some(wall.front) => {
                    // Internal two-sided wall (grate/fence inside one region): draw it masked and
                    // keep tracing in the same region.
                    if let Some(surface) = wall.middle {
                        let (y0, y1) = (ytop.max(y_ceil.ceil() as i32), ybot.min(y_floor.ceil() as i32));
                        self.masked.push(MaskedSpan { column, dist, y0, y1, surface, u: along, top: reg.ceiling, shade });
                    }
                }
                None => {
                    match wall.middle {
                        // One-sided wall: texture top at the ceiling.
                        Some(surface) => {
                            self.wall_span(world, view, frame, column, dist, (y_ceil, y_floor), (ytop, ybot), surface, along, reg.ceiling, shade)
                        }
                        // No texture: the original shows the sky.
                        None => self.sky_span(world, view, frame, column, ray, ytop.max(y_ceil.ceil() as i32), ybot.min(y_floor.ceil() as i32)),
                    }
                    return;
                }
            }
        }
    }

    /// Draw a vertical wall slice between screen rows `rows` (clipped to `clip`). `anchor` is
    /// the world height where texture row `offset_v` sits.
    #[allow(clippy::too_many_arguments)]
    fn wall_span(
        &mut self,
        world: &World,
        view: &View,
        frame: &mut crate::Frame,
        column: usize,
        dist: f32,
        (y0, y1): (f32, f32),
        (ytop, ybot): (i32, i32),
        surface: Surface,
        along: f32,
        anchor: f32,
        shade: f32,
    ) {
        let Some(image) = world.images.get(surface.image) else { return };
        let (start, end) = ((y0.ceil() as i32).max(ytop), (y1.ceil() as i32).min(ybot));
        if start >= end {
            return;
        }
        let u = ((along + surface.offset_u) * surface.scale).floor() as i32;
        let units_per_row = dist / view.focal;
        for y in start..end {
            let z = view.cam.z + (view.horizon - y as f32 - 0.5) * units_per_row;
            let v = ((anchor - z + surface.offset_v) * surface.scale).floor() as i32;
            self.depth[y as usize * view.width + column] = dist;
            view.paint(frame, view.out_index(frame.width, column, y as usize), image.sample_wrapped(u, v), shade);
        }
    }

    /// Fill the ceiling above and floor below the wall hit in `region` (or the whole
    /// remaining window when nothing was hit). A floor or ceiling that is a portal into a
    /// stacked region is traced through instead of drawn.
    #[allow(clippy::too_many_arguments)]
    fn flats(&mut self, world: &World, view: &View, frame: &mut crate::Frame, ctx: FlatContext, ytop: i32, ybot: i32, wall_rows: Option<(f32, f32)>) {
        let FlatContext { column, ray, region, entry_t, from, stack_depth } = ctx;
        let reg = &world.regions[region];
        let (ceil_end, floor_start) = match wall_rows {
            Some((yc, yf)) => ((yc.ceil() as i32).clamp(ytop, ybot), (yf.ceil() as i32).clamp(ytop, ybot)),
            None => {
                let h = view.horizon.ceil() as i32;
                (h.clamp(ytop, ybot), h.clamp(ytop, ybot))
            }
        };
        let stacked = |link: Option<usize>| link.filter(|&r| Some(r) != from && r < world.regions.len() && stack_depth < MAX_STACK_DEPTH);
        let through = |target: usize, ytop: i32, ybot: i32| Trace {
            column,
            ray,
            region: target,
            ytop,
            ybot,
            start_t: entry_t,
            from: Some(region),
            stack_depth: stack_depth + 1,
        };
        match stacked(reg.above) {
            Some(above) if ytop < ceil_end => self.trace(world, view, frame, through(above, ytop, ceil_end)),
            _ => {
                let ceiling = (reg.ceiling, reg.ceiling_surface, reg.ceiling_light);
                self.flat_span(world, view, frame, column, ray, (ytop, ceil_end), ceiling, reg.fullbright);
            }
        }
        match stacked(reg.below) {
            Some(below) if floor_start < ybot => self.trace(world, view, frame, through(below, floor_start, ybot)),
            _ => {
                let floor = (reg.floor, reg.floor_surface, reg.floor_light);
                self.flat_span(world, view, frame, column, ray, (floor_start, ybot), floor, reg.fullbright);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn flat_span(
        &mut self,
        world: &World,
        view: &View,
        frame: &mut crate::Frame,
        column: usize,
        ray: (f32, f32),
        (start, end): (i32, i32),
        (height, surface, light): (f32, Option<Surface>, i32),
        fullbright: bool,
    ) {
        let (start, end) = (start.max(0), end.min(view.height as i32));
        if start >= end {
            return;
        }
        let image: Option<(Surface, &Image)> = surface.and_then(|s| world.images.get(s.image).map(|i| (s, i)));
        let Some((s, image)) = image else {
            self.sky_span(world, view, frame, column, ray, start, end);
            return;
        };
        let rel = height - view.cam.z;
        for y in start..end {
            let out = view.out_index(frame.width, column, y as usize);
            let dy = view.horizon - y as f32 - 0.5;
            let dist = rel * view.focal / dy;
            if !(dist > 0.0 && dist.is_finite()) {
                continue;
            }
            let wx = view.cam.x + ray.0 * dist;
            let wy = view.cam.y + ray.1 * dist;
            let u = ((wx + s.offset_u) * s.scale).floor() as i32;
            let v = ((-wy + s.offset_v) * s.scale).floor() as i32;
            self.depth[y as usize * view.width + column] = dist;
            view.paint(frame, out, image.sample_wrapped(u, v), view.shade(light, dist, fullbright));
        }
    }

    /// Sky: unlit, `sky.width` columns per full turn, centre row on the horizon, spanning
    /// twice the view height.
    #[allow(clippy::too_many_arguments)]
    fn sky_span(&mut self, world: &World, view: &View, frame: &mut crate::Frame, column: usize, ray: (f32, f32), start: i32, end: i32) {
        let (start, end) = (start.max(0), end.min(view.height as i32));
        let Some(sky) = world.sky.and_then(|i| world.images.get(i)) else {
            for y in start..end {
                frame.put(view.out_index(frame.width, column, y as usize), ZERO_SUBSTITUTE);
            }
            return;
        };
        let angle = ray.1.atan2(ray.0);
        let u = (-angle / TAU * sky.width as f32).floor() as i32;
        let span = 2.0 * view.height as f32;
        let top = view.horizon - span / 2.0;
        for y in start..end {
            let v = (((y as f32 + 0.5 - top) / span * sky.height as f32) as i32).clamp(0, sky.height as i32 - 1);
            frame.put(view.out_index(frame.width, column, y as usize), sky.sample_wrapped(u, v));
        }
    }

    fn render_masked(&mut self, world: &World, view: &View, frame: &mut crate::Frame) {
        let mut masked = std::mem::take(&mut self.masked);
        masked.sort_by(|a, b| b.dist.total_cmp(&a.dist));
        for span in &masked {
            let Some(image) = world.images.get(span.surface.image) else { continue };
            let u = ((span.u + span.surface.offset_u) * span.surface.scale).floor() as i32;
            let units_per_row = span.dist / view.focal;
            for y in span.y0.max(0)..span.y1.min(view.height as i32) {
                let o = y as usize * view.width + span.column;
                if self.depth[o] < span.dist {
                    continue;
                }
                let z = view.cam.z + (view.horizon - y as f32 - 0.5) * units_per_row;
                let v = ((span.top - z + span.surface.offset_v) * span.surface.scale).floor() as i32;
                if v < 0 || v >= image.height as i32 {
                    continue;
                }
                let texel = image.sample_wrapped(u, v);
                if texel != TRANSPARENT {
                    self.depth[o] = span.dist;
                    view.paint(frame, view.out_index(frame.width, span.column, y as usize), texel, span.shade);
                }
            }
        }
        self.masked = masked;
    }

    fn render_sprites(&mut self, world: &World, view: &View, frame: &mut crate::Frame) {
        self.sprite_order.clear();
        for (i, s) in world.sprites.iter().enumerate() {
            if !s.visible || s.frames.is_empty() {
                continue;
            }
            let (rx, ry) = (s.x - view.cam.x, s.y - view.cam.y);
            let depth = rx * view.dir.0 + ry * view.dir.1;
            if depth > NEAR {
                self.sprite_order.push((depth, i));
            }
        }
        self.sprite_order.sort_by(|a, b| b.0.total_cmp(&a.0));
        let plane_len = (view.plane.0.powi(2) + view.plane.1.powi(2)).sqrt();
        let right = (view.plane.0 / plane_len, view.plane.1 / plane_len);
        for &(depth, i) in &self.sprite_order {
            let s = &world.sprites[i];
            let (rx, ry) = (s.x - view.cam.x, s.y - view.cam.y);
            let lateral = rx * right.0 + ry * right.1;
            // Rotation: frame 0 faces the camera; index = round((objAngle − bearing(obj→cam)) / step).
            let n = s.frames.len();
            let bearing = (-ry).atan2(-rx);
            let rel = (s.angle - bearing).rem_euclid(TAU);
            let sector = ((rel / TAU * n as f32) + 0.5) as usize % n;
            let (image_index, mirrored) = s.frames[sector];
            let Some(image) = world.images.get(image_index) else { continue };
            let world_w = image.width as f32 * s.scale;
            let world_h = image.height as f32 * s.scale;
            let center_x = view.width as f32 / 2.0 + lateral * view.focal / depth;
            let half_w = world_w / 2.0 * view.focal / depth;
            let y_top = view.row_of(s.z + world_h, depth);
            let y_bot = view.row_of(s.z, depth);
            let (x0, x1) = ((center_x - half_w).ceil() as i32, (center_x + half_w).ceil() as i32);
            let (y0, y1) = (y_top.ceil() as i32, y_bot.ceil() as i32);
            if x1 <= 0 || x0 >= view.width as i32 || y1 <= 0 || y0 >= view.height as i32 {
                continue;
            }
            let (light, fullbright) = world.regions.get(s.region).map_or((0, false), |r| (r.floor_light, r.fullbright));
            let shade = view.shade(light, depth, fullbright).min(MAX_OBJECT_SHADE as f32);
            for x in x0.max(0)..x1.min(view.width as i32) {
                let mut u = (((x as f32 + 0.5 - (center_x - half_w)) / (2.0 * half_w)) * image.width as f32) as usize;
                u = u.min(image.width - 1);
                if mirrored {
                    u = image.width - 1 - u;
                }
                for y in y0.max(0)..y1.min(view.height as i32) {
                    let o = y as usize * view.width + x as usize;
                    if self.depth[o] < depth {
                        continue;
                    }
                    let v = (((y as f32 + 0.5 - y_top) / (y_bot - y_top)) * image.height as f32) as usize;
                    let texel = image.pixels[v.min(image.height - 1) * image.width + u];
                    if texel != TRANSPARENT {
                        self.depth[o] = depth;
                        view.paint(frame, view.out_index(frame.width, x as usize, y as usize), texel, shade);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Frame;

    fn identity() -> Vec<[u8; 256]> {
        let mut row = [0u8; 256];
        for (i, v) in row.iter_mut().enumerate() {
            *v = i as u8;
        }
        vec![row; 16]
    }

    fn render(world: &World, width: usize) -> Frame {
        let rows = identity();
        let shading = Shading { rows: &rows };
        let cam = Camera { x: 256.0, y: 256.0, z: 48.0, angle: 0.0, pitch: 0.0, region: 0 };
        let mut frame = Frame::new(width, 200);
        let vp = Viewport { x: 0, y: 0, width, height: 200 };
        Renderer::default().render(world, &cam, &shading, &RenderSettings::default(), &mut frame, vp);
        frame
    }

    #[test]
    fn room_has_wall_floor_and_ceiling() {
        let world = World::test_room(512.0, 512.0, 128.0);
        let frame = render(&world, 320);
        let center_column: Vec<u8> = (0..200).map(|y| frame.pixels[y * 320 + 160]).collect();
        assert_eq!(center_column[0], 30, "ceiling at top");
        assert_eq!(center_column[100], 10, "wall at horizon");
        assert_eq!(center_column[199], 20, "floor at bottom");
    }

    #[test]
    fn widescreen_sees_more_horizontally() {
        // Hor+: at the same height a wider frame keeps the vertical extent of the wall.
        let world = World::test_room(512.0, 512.0, 128.0);
        let wall_rows = |w: usize| {
            let frame = render(&world, w);
            (0..200).filter(|y| frame.pixels[y * w + w / 2] == 10).count()
        };
        assert_eq!(wall_rows(320), wall_rows(860));
    }

    #[test]
    fn original_projection_constants() {
        // 320 columns at 72° horizontal FOV: focal = 160·cot 36° ≈ 220.2 → vfov ≈ 48.9°.
        let focal = 100.0 / (RenderSettings::default().vfov_degrees.to_radians() / 2.0).tan();
        assert!((focal - 220.2).abs() < 0.5, "focal {focal}");
    }
}
