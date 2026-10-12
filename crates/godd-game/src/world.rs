//! Engine-side level model, decoupled from the on-disk XRB layout.
//!
//! Units: map units (the original's 16.16 integer part). X grows east, Y grows north,
//! Z grows up; angles are radians, 0 = +X, counter-clockwise.

/// An 8-bit image, stored row-major (`pixels[y * width + x]`).
#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn solid(width: usize, height: usize, index: u8) -> Self {
        Self { width, height, pixels: vec![index; width * height] }
    }

    #[inline]
    pub fn sample_wrapped(&self, u: i32, v: i32) -> u8 {
        let x = u.rem_euclid(self.width as i32) as usize;
        let y = v.rem_euclid(self.height as i32) as usize;
        self.pixels[y * self.width + x]
    }
}

/// Palette index treated as transparent in sprites and masked walls.
pub const TRANSPARENT: u8 = 0;

/// A surface texture: which image, plus offsets and scale in texels per map unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    pub image: usize,
    pub offset_u: f32,
    pub offset_v: f32,
    pub scale: f32,
}

impl Surface {
    pub fn new(image: usize) -> Self {
        Self { image, offset_u: 0.0, offset_v: 0.0, scale: 1.0 }
    }
}

#[derive(Debug, Clone)]
pub struct Wall {
    pub a: usize,
    pub b: usize,
    /// Region on the wall's front side (the region whose outline contains it).
    pub front: usize,
    /// Region behind the wall when the wall is a portal.
    pub back: Option<usize>,
    pub upper: Option<Surface>,
    pub middle: Option<Surface>,
    pub lower: Option<Surface>,
    /// Draw the middle texture on a portal with transparency (fences, windows).
    pub masked: bool,
    /// Blocks movement even when it is a portal.
    pub blocking: bool,
    /// Shade row (0 = brightest) before distance shading.
    pub light: i32,
    /// Colormap block (0 = normal shading).
    pub colormap: i32,
}

#[derive(Debug, Clone)]
pub struct Region {
    pub floor: f32,
    pub ceiling: f32,
    pub floor_surface: Option<Surface>,
    /// `None` = sky / open ceiling.
    pub ceiling_surface: Option<Surface>,
    /// Shade rows (0 = brightest) of the floor and ceiling before distance shading.
    pub floor_light: i32,
    pub ceiling_light: i32,
    pub floor_colormap: i32,
    pub ceiling_colormap: i32,
    /// Distance shading disabled (region flag 0x04000000).
    pub fullbright: bool,
    pub walls: Vec<usize>,
    /// Bounding box ((min x, min y), (max x, max y)) of the region's walls.
    pub bounds: ((f32, f32), (f32, f32)),
    /// Stacked regions: the floor is a see-through portal into `below`, the ceiling into `above`.
    pub below: Option<usize>,
    pub above: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Sprite {
    pub x: f32,
    pub y: f32,
    /// Bottom of the sprite.
    pub z: f32,
    pub angle: f32,
    pub region: usize,
    /// Rotation frames, evenly spaced counter-clockwise starting facing +X; one entry means
    /// the sprite looks the same from all directions. The flag mirrors the image horizontally.
    pub frames: Vec<(usize, bool)>,
    /// World units per texel.
    pub scale: f32,
    pub visible: bool,
    /// Collision height of the object (doors and lifts stop above the tallest occupant).
    pub height: f32,
}

#[derive(Debug, Clone, Default)]
pub struct World {
    pub vertices: Vec<(f32, f32)>,
    pub walls: Vec<Wall>,
    pub regions: Vec<Region>,
    pub sprites: Vec<Sprite>,
    pub images: Vec<Image>,
    /// Background image for open ceilings.
    pub sky: Option<usize>,
}

impl World {
    /// Region containing the point, using the original's rule (XRB-records.md §4): cast a
    /// ray toward +X and walk the wall crossings nearest first; a wall running toward −Y means
    /// the point is in its front region, otherwise in its back region (none for one-sided
    /// walls). The first candidate whose bounding box holds the point — and, when `z` is given,
    /// whose floor ≤ z < ceiling — wins, which resolves stacked regions.
    pub fn region_at_z(&self, x: f32, y: f32, z: Option<f32>) -> Option<usize> {
        let mut hits: Vec<(f32, Option<usize>)> = self
            .walls
            .iter()
            .filter(|w| w.back != Some(w.front))
            .filter_map(|wall| {
                let d = self.crossing(wall, x, y)?;
                let (_, ay) = self.vertices[wall.a];
                let (_, by) = self.vertices[wall.b];
                Some((d, if ay > by { Some(wall.front) } else { wall.back }))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let fits = |r: usize| {
            let reg = &self.regions[r];
            let (min, max) = reg.bounds;
            x >= min.0 && x <= max.0 && y >= min.1 && y <= max.1 && z.is_none_or(|z| z >= reg.floor && z < reg.ceiling)
        };
        let candidates = || hits.iter().filter_map(|h| h.1).filter(|&r| r < self.regions.len());
        candidates().find(|&r| fits(r)).or_else(|| hits.first().and_then(|h| h.1).filter(|&r| r < self.regions.len()))
    }

    pub fn region_at(&self, x: f32, y: f32) -> Option<usize> {
        self.region_at_z(x, y, None)
    }

    /// X distance from (x, y) to where `wall` crosses the horizontal line through y, if the
    /// crossing lies to the right.
    fn crossing(&self, wall: &Wall, x: f32, y: f32) -> Option<f32> {
        let (ax, ay) = self.vertices[wall.a];
        let (bx, by) = self.vertices[wall.b];
        // The original counts a crossing when max(y1, y2) >= y > min(y1, y2).
        if !(ay.max(by) >= y && y > ay.min(by)) {
            return None;
        }
        let cx = ax + (y - ay) / (by - ay) * (bx - ax);
        (cx > x).then_some(cx - x)
    }

    /// Like [`World::region_at`] but tries `hint` first (the common case while moving).
    pub fn region_at_hint(&self, x: f32, y: f32, hint: usize) -> Option<usize> {
        if self.point_in_region(x, y, hint) { Some(hint) } else { self.region_at(x, y) }
    }

    /// Even-odd test against one region's walls (internal walls ignored).
    pub fn point_in_region(&self, x: f32, y: f32, region: usize) -> bool {
        let Some(r) = self.regions.get(region) else { return false };
        let mut inside = false;
        for &w in &r.walls {
            let wall = &self.walls[w];
            if wall.back == Some(wall.front) {
                continue;
            }
            if self.crossing(wall, x, y).is_some() {
                inside = !inside;
            }
        }
        inside
    }

    /// The solid floor under `region`: see-through floors of stacked regions are followed
    /// down their `below` links. Returns (floor height, region owning it).
    pub fn solid_floor(&self, region: usize) -> (f32, usize) {
        let mut r = region;
        for _ in 0..8 {
            match self.regions[r].below.filter(|&b| b < self.regions.len()) {
                Some(b) => r = b,
                None => break,
            }
        }
        (self.regions[r].floor, r)
    }

    /// The solid ceiling over `region`, following `above` links.
    pub fn solid_ceiling(&self, region: usize) -> (f32, usize) {
        let mut r = region;
        for _ in 0..8 {
            match self.regions[r].above.filter(|&a| a < self.regions.len()) {
                Some(a) => r = a,
                None => break,
            }
        }
        (self.regions[r].ceiling, r)
    }

    /// A rectangular room for tests: `w` x `h` units, floor 0, ceiling `height`.
    pub fn test_room(w: f32, h: f32, height: f32) -> Self {
        let mut world = World {
            vertices: vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)],
            images: vec![Image::solid(64, 64, 10), Image::solid(64, 64, 20), Image::solid(64, 64, 30)],
            ..Default::default()
        };
        for i in 0..4 {
            world.walls.push(Wall {
                a: i,
                b: (i + 1) % 4,
                front: 0,
                back: None,
                upper: None,
                middle: Some(Surface::new(0)),
                lower: None,
                masked: false,
                blocking: true,
                light: 0,
                colormap: 0,
            });
        }
        world.regions.push(Region {
            floor: 0.0,
            ceiling: height,
            floor_surface: Some(Surface::new(1)),
            ceiling_surface: Some(Surface::new(2)),
            floor_light: 0,
            ceiling_light: 0,
            floor_colormap: 0,
            ceiling_colormap: 0,
            fullbright: false,
            walls: vec![0, 1, 2, 3],
            bounds: ((0.0, 0.0), (w, h)),
            below: None,
            above: None,
        });
        world
    }
}
