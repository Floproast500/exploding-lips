//! Clustered dynamic lights: the view is split into screen tiles and depth slices, and each
//! cluster gets a bit mask of the lights whose sphere can reach it, so a fragment only shades
//! with the lights near it. Bounds are conservative (a light may be listed in a cluster it
//! doesn't quite reach, never left out of one it does).

/// Screen tiles across and down, and depth slices.
pub const TILES_X: usize = 16;
pub const TILES_Y: usize = 9;
pub const SLICES: usize = 4;
pub const CLUSTERS: usize = TILES_X * TILES_Y * SLICES;
/// Far edges of the depth slices (horizontal distance ahead of the eye, world units); the last
/// slice reaches to infinity.
pub const SLICE_FAR: [f32; SLICES - 1] = [256.0, 640.0, 1536.0];

/// The camera as the world shader projects: eye, horizontal view direction, NDC scale and
/// pitch (tangent; a real rotation with `freelook`, a y-shear otherwise).
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub eye: [f32; 3],
    pub dir: [f32; 2],
    pub right: [f32; 2],
    pub fx: f32,
    pub fy: f32,
    pub pitch: f32,
    pub freelook: bool,
    pub near: f32,
}

fn slice_of(depth: f32) -> usize {
    SLICE_FAR.iter().position(|&f| depth < f).unwrap_or(SLICES - 1)
}

fn tile(ndc: f32, tiles: usize, flip: bool) -> f32 {
    let t = if flip { (1.0 - ndc) / 2.0 } else { (ndc + 1.0) / 2.0 };
    t * tiles as f32
}

/// Bit masks (two words: lights 0-31 and 32-63) per cluster, cluster index
/// `(slice * TILES_Y + ty) * TILES_X + tx`.
pub fn assign(view: &View, lights: &[([f32; 3], f32)]) -> Vec<[u32; 2]> {
    let mut masks = vec![[0u32; 2]; CLUSTERS];
    for (i, &(pos, radius)) in lights.iter().enumerate().take(64) {
        let rel = [pos[0] - view.eye[0], pos[1] - view.eye[1], pos[2] - view.eye[2]];
        let forward = rel[0] * view.dir[0] + rel[1] * view.dir[1];
        let right = rel[0] * view.right[0] + rel[1] * view.right[1];
        // Depth slices use the horizontal distance ahead (the shader's `depth`).
        if forward + radius < 0.0 {
            continue; // wholly behind the eye
        }
        let (s0, s1) = (slice_of(forward - radius), slice_of(forward + radius));
        // Clip-space w and the vertical numerator, with the sphere's extent in each.
        let (w, up, r_up) = if view.freelook {
            let a = view.pitch.atan();
            let (sin, cos) = a.sin_cos();
            (forward * cos + rel[2] * sin, rel[2] * cos - forward * sin, radius)
        } else {
            (forward, rel[2] - view.pitch * forward, radius * (1.0 + view.pitch.abs()))
        };
        let (mut x0, mut x1, mut y0, mut y1) = (0.0, TILES_X as f32, 0.0, TILES_Y as f32);
        let r_w = r_up.max(radius);
        if w - r_w > view.near {
            let ratio = |num: f32, sign: f32| {
                let a = (num + sign * radius) / (w - r_w);
                let b = (num + sign * radius) / (w + r_w);
                if sign < 0.0 { a.min(b) } else { a.max(b) }
            };
            let (nx0, nx1) = (ratio(right, -1.0) * view.fx, ratio(right, 1.0) * view.fx);
            let ratio_y = |sign: f32| {
                let a = (up + sign * r_up) / (w - r_w);
                let b = (up + sign * r_up) / (w + r_w);
                if sign < 0.0 { a.min(b) } else { a.max(b) }
            };
            let (ny0, ny1) = (ratio_y(-1.0) * view.fy, ratio_y(1.0) * view.fy);
            if nx1 < -1.0 || nx0 > 1.0 || ny1 < -1.0 || ny0 > 1.0 {
                continue; // off screen
            }
            (x0, x1) = (tile(nx0, TILES_X, false), tile(nx1, TILES_X, false));
            (y0, y1) = (tile(ny1, TILES_Y, true), tile(ny0, TILES_Y, true));
        }
        let span = |a: f32, b: f32, n: usize| (a.floor().max(0.0) as usize).min(n - 1)..=(b.floor().max(0.0) as usize).min(n - 1);
        for s in s0..=s1 {
            for ty in span(y0, y1, TILES_Y) {
                for tx in span(x0, x1, TILES_X) {
                    masks[(s * TILES_Y + ty) * TILES_X + tx][i / 32] |= 1 << (i % 32);
                }
            }
        }
    }
    masks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(freelook: bool, pitch: f32) -> View {
        View { eye: [0.0; 3], dir: [1.0, 0.0], right: [0.0, -1.0], fx: 1.0, fy: 16.0 / 9.0, pitch, freelook, near: 4.0 }
    }

    /// Project a world point like the shader and find its cluster.
    fn cluster_of(v: &View, p: [f32; 3]) -> Option<usize> {
        let (forward, right) = (p[0] * v.dir[0] + p[1] * v.dir[1], p[0] * v.right[0] + p[1] * v.right[1]);
        let (w, up) = if v.freelook {
            let (s, c) = v.pitch.atan().sin_cos();
            (forward * c + p[2] * s, p[2] * c - forward * s)
        } else {
            (forward, p[2] - v.pitch * forward)
        };
        if w <= v.near {
            return None;
        }
        let (nx, ny) = (right * v.fx / w, up * v.fy / w);
        if nx.abs() > 1.0 || ny.abs() > 1.0 {
            return None;
        }
        let tx = (tile(nx, TILES_X, false) as usize).min(TILES_X - 1);
        let ty = (tile(ny, TILES_Y, true) as usize).min(TILES_Y - 1);
        Some((slice_of(forward) * TILES_Y + ty) * TILES_X + tx)
    }

    #[test]
    fn every_lit_point_is_in_a_cluster_listing_its_light() {
        for (freelook, pitch) in [(false, 0.0), (false, 0.4), (true, 0.0), (true, -0.6), (true, 0.9)] {
            let v = view(freelook, pitch);
            let lights: Vec<([f32; 3], f32)> = (0..40).map(|i| {
                let a = i as f32 * 0.7;
                ([200.0 + 60.0 * i as f32, a.sin() * 300.0, (a * 1.3).cos() * 120.0], 80.0 + (i % 5) as f32 * 60.0)
            }).collect();
            let masks = assign(&v, &lights);
            for (i, &(c, r)) in lights.iter().enumerate() {
                // Sample points inside the sphere; each visible one must see the light.
                for k in 0..200 {
                    let t = k as f32 * 2.399;
                    let f = (k as f32 / 200.0).sqrt() * 0.99;
                    let p = [c[0] + r * f * t.cos(), c[1] + r * f * t.sin() * (t * 0.37).cos(), c[2] + r * f * (t * 0.37).sin() * t.sin()];
                    if let Some(cl) = cluster_of(&v, p) {
                        assert!(masks[cl][i / 32] & (1 << (i % 32)) != 0, "light {i} missing from cluster {cl} ({freelook}, {pitch})");
                    }
                }
            }
        }
    }

    #[test]
    fn lights_behind_or_off_screen_are_culled() {
        let v = view(true, 0.0);
        let masks = assign(&v, &[([-500.0, 0.0, 0.0], 100.0), ([300.0, 5000.0, 0.0], 100.0), ([300.0, 0.0, 0.0], 50.0)]);
        let has = |i: usize| masks.iter().any(|m| m[0] & (1 << i) != 0);
        assert!(!has(0) && !has(1) && has(2));
        // A small light far ahead touches few clusters.
        assert!(masks.iter().filter(|m| m[0] & 4 != 0).count() < CLUSTERS / 8);
    }
}
