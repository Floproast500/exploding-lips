//! What a game hands the GPU renderer each frame: world-space triangles with texture
//! coordinates, the camera and the lighting rules. Images are palette-indexed; texture
//! coordinates are in texels (unwrapped), so one texel per world unit stays exact.

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
pub fn clip_convex(subject: &[[f32; 2]], window: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = subject.to_vec();
    for i in 0..window.len() {
        if out.is_empty() {
            break;
        }
        let (p, q) = (window[i], window[(i + 1) % window.len()]);
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
    out
}

/// Signed area (counter-clockwise positive).
pub fn polygon_area(p: &[[f32; 2]]) -> f32 {
    (0..p.len()).map(|i| {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f32>() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

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
