//! What a game hands the GPU renderer each frame: world-space triangles with texture
//! coordinates, the camera and the lighting rules. Images are palette-indexed; texture
//! coordinates are in texels (unwrapped), so one texel per world unit stays exact.

/// Recorded UI drawing, drawn by the GPU over the view (`WorldRenderer::draw_ui`).
pub use engine_core::frame::UiCmd;

/// Texel index 0 is transparent (masked walls, sprites).
pub const TRANSPARENT: u32 = 1;
/// Draw the panoramic sky here instead of the texture.
pub const SKY: u32 = 2;
/// No distance darkening (full-bright regions).
pub const NO_DISTANCE: u32 = 4;
/// Rows outside the image are empty instead of wrapping (masked walls).
pub const DISCARD_OUTSIDE_V: u32 = 8;
/// Clamp instead of wrap on both axes (sprites).
pub const CLAMP: u32 = 16;
/// Object shading: never darker than [`Shading::max_object_shade`].
pub const OBJECT: u32 = 32;
/// Palette colour 0, unlit: what classic portal renderers leave in gaps they never draw
/// (a step between two regions with no texture for it).
pub const VOID: u32 = 64;
/// Screen-space quad (first-person weapon): `pos.xy` are normalized device coordinates, drawn
/// in front of everything, unaffected by the camera and dynamic lights.
pub const SCREEN: u32 = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SceneVertex {
    /// World position (X east, Y north, Z up).
    pub pos: [f32; 3],
    /// Texel coordinates (u right, v down), unwrapped.
    pub uv: [f32; 2],
    /// Index into the images given to the renderer.
    pub image: u32,
    /// Base shade level (darkness, 0 = brightest) before distance.
    pub light: f32,
    pub flags: u32,
    /// Surface normal (unit, world space) for dynamic lights.
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneCamera {
    pub pos: [f32; 3],
    /// Radians, counter-clockwise from +X.
    pub angle: f32,
    /// Look up/down as a y-shear in units of the focal length (classic) — the horizon moves.
    /// With `freelook` the same value is the tangent of a real pitch rotation.
    pub pitch: f32,
    pub vfov_degrees: f32,
    /// Rotate the view for pitch (true 3D) instead of shearing it.
    pub freelook: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shading {
    /// World units of depth per shade level.
    pub visibility: f32,
    /// Added to every shade value.
    pub fade_bias: f32,
    /// Continuous RGB lighting instead of shade-table lookups.
    pub true_color: bool,
    /// Everything full bright (lightning).
    pub fullbright: bool,
    pub max_object_shade: f32,
    /// Shade-table entry substituted when a lookup yields colour 0 (the original uses 8).
    pub zero_substitute: u32,
    /// Modern lighting: linear HDR light from [`Shading::brightness`], emissive colours,
    /// dynamic lights, bloom and tonemapping. Off: the original shade tables (or true colour).
    pub modern: Option<Modern>,
}

/// Settings of the modern lighting model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modern {
    /// Brightness (linear, 1 = full) of each original shade level, fitted from the shade tables;
    /// interpolated for continuous shade values.
    pub brightness: [f32; 16],
    /// Palette indices that never darken (lamps, lava): bit i of word i/32.
    pub emissive: [u32; 8],
    pub dynamic_lights: bool,
    pub bloom: bool,
    /// 0 clamp, 1 soft shoulder (keeps the original's colours), 2 ACES filmic.
    pub tonemap: u32,
    pub exposure: f32,
    /// Multisampling (1, 2, 4 or 8; limited to what the GPU supports).
    pub msaa: u32,
    /// The 3D view is drawn at this fraction of the display resolution (0.25 .. 2; above 1
    /// supersamples) and filtered to the display.
    pub render_scale: f32,
    /// Fast approximate anti-aliasing on the tonemapped image.
    pub fxaa: bool,
    /// The strongest dynamic light casts shadows from the visible world geometry.
    pub shadows: bool,
}

/// A coloured point light (projectiles, explosions, muzzle flashes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLight {
    pub pos: [f32; 3],
    pub radius: f32,
    /// Linear colour times intensity.
    pub color: [f32; 3],
}

#[derive(Debug, Clone, Copy)]
pub struct IndexedImage<'a> {
    pub width: usize,
    pub height: usize,
    /// Row-major palette indices.
    pub pixels: &'a [u8],
}

/// A run of vertices drawn inside an optional convex clip window (portal visibility): the
/// window's corners in normalized device coordinates (x right, y up, -1..1), counter-clockwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub start: u32,
    pub end: u32,
    pub clip: Option<Vec<[f32; 2]>>,
}

/// An RGBA8 image (sRGB colour, straight alpha).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// A high-resolution replacement for one image (texture packs): colour with alpha and an
/// optional tangent-space normal map. Texture coordinates keep the original's texel units.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HdTexture {
    pub albedo: RgbaImage,
    pub normal: Option<RgbaImage>,
}

/// One frame's scene.
#[derive(Debug, Clone, Default)]
pub struct SceneFrame {
    pub vertices: Vec<SceneVertex>,
    /// Draw order and clipping; empty means one unclipped batch of all vertices.
    pub batches: Vec<Batch>,
    /// Image drawn where [`SKY`] is set (`None`: colour [`Shading::zero_substitute`]).
    pub sky: Option<u32>,
    /// Images whose pixels changed since the last frame (live camera views).
    pub dirty_images: Vec<u32>,
    pub lights: Vec<SceneLight>,
}

impl SceneFrame {
    /// Close the batch of vertices added since `start`.
    pub fn batch(&mut self, start: usize, clip: Option<Vec<[f32; 2]>>) {
        if self.vertices.len() > start {
            self.batches.push(Batch { start: start as u32, end: self.vertices.len() as u32, clip });
        }
    }

    /// Two triangles for a quad given counter-clockwise corners.
    pub fn quad(&mut self, corners: [SceneVertex; 4]) {
        let [a, b, c, d] = corners;
        self.vertices.extend_from_slice(&[a, b, c, a, c, d]);
    }
}

/// Clip a convex polygon (counter-clockwise) to another convex polygon (Sutherland-Hodgman).
/// Window edges shorter than [`CLIP_EPSILON`] are skipped: their direction is rounding noise
/// (clipping leaves corners a hair apart), and a half-plane along one cut an arbitrary wedge out
/// of the result (LIPS01's streets lost whole buildings to sky at a slight look down). The
/// result has no corners closer than that either.
pub fn clip_convex(subject: &[[f32; 2]], window: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = subject.to_vec();
    for i in 0..window.len() {
        if out.is_empty() {
            break;
        }
        let (p, q) = (window[i], window[(i + 1) % window.len()]);
        if (q[0] - p[0]).hypot(q[1] - p[1]) < CLIP_EPSILON {
            continue;
        }
        let side = |v: [f32; 2]| (q[0] - p[0]) * (v[1] - p[1]) - (q[1] - p[1]) * (v[0] - p[0]);
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (a, b) = (input[j], input[(j + 1) % input.len()]);
            let (sa, sb) = (side(a), side(b));
            if sa >= 0.0 {
                out.push(a);
            }
            if (sa >= 0.0) != (sb >= 0.0) {
                let t = sa / (sa - sb);
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
    }
    let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < CLIP_EPSILON;
    out.dedup_by(|b, a| near(*a, *b));
    while out.len() > 1 && near(out[0], out[out.len() - 1]) {
        out.pop();
    }
    out
}

/// Corners of clip polygons closer than this (in normalized device coordinates, about a
/// thousandth of a pixel at 4K) are one corner.
pub const CLIP_EPSILON: f32 = 1e-5;

/// Signed area (counter-clockwise positive).
pub fn polygon_area(p: &[[f32; 2]]) -> f32 {
    (0..p.len()).map(|i| {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f32>() / 2.0
}

/// Convex hull of a point set, counter-clockwise (monotone chain).
pub fn convex_hull(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<[f32; 2]> = Vec::with_capacity(pts.len() * 2);
    for pass in [&pts[..], &pts.iter().rev().copied().collect::<Vec<_>>()[..]] {
        let floor = hull.len();
        for &p in pass {
            while hull.len() >= floor + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_ignores_a_window_edge_of_rounding_length() {
        // Two corners a hair apart: the edge between them points anywhere, and must not cut.
        let window = [[-0.25, -0.24717832], [-0.25, -0.24717827], [0.0, -0.16], [0.0, 1.0], [-0.27, 1.0]];
        let subject = [[-0.2, -0.1], [-0.01, -0.1], [-0.01, 0.9], [-0.2, 0.9]];
        let out = clip_convex(&subject, &window);
        assert!((polygon_area(&out) - polygon_area(&subject)).abs() < 1e-6, "{out:?}");
    }

    #[test]
    fn hull_of_two_squares() {
        let pts = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [2.0, 0.5], [0.5, 0.5]];
        let h = convex_hull(&pts);
        assert_eq!(h.len(), 5, "the inner point is dropped: {h:?}");
        assert!((polygon_area(&h) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn convex_clipping() {
        let square = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let tri = [[0.0, -2.0], [2.0, 2.0], [-2.0, 2.0]];
        let c = clip_convex(&square, &tri);
        assert!(polygon_area(&c) > 0.0 && polygon_area(&c) < 4.0);
        assert!(clip_convex(&square, &[[3.0, 3.0], [4.0, 3.0], [4.0, 4.0]]).is_empty());
        assert_eq!(polygon_area(&clip_convex(&square, &square)), 4.0);
    }
}
