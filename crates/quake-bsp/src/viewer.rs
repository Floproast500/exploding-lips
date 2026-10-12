//! A fly-through viewer for Quake-family maps as a [`GameModule`]: the world model's faces with
//! their textures, baked lightmap light per vertex, sky faces drawn as sky, and modern lighting
//! (fullbright palette colours glow). Free flight: WASD/arrows, mouse look, jump/crouch up/down.

use crate::{Bsp, Error, Pak};
use engine_core::Frame;
use engine_scene::module::{GameModule, ModuleInput, ModuleView, TICK_RATE, fit_light};
use engine_scene::scene::{IndexedImage, Modern, NO_DISTANCE, SKY, SceneCamera, SceneFrame, SceneVertex, Shading};

/// Units per second, like a running Quake player.
const SPEED: f32 = 320.0;
/// Eye height above the player start's origin.
const EYE: f32 = 22.0;

pub struct BspViewer {
    name: String,
    bsp: Bsp,
    palette: [[u8; 3]; 256],
    rows: Vec<[u8; 256]>,
    light_fit: ([f32; 16], [u32; 8]),
    /// Static scene (the world doesn't move); copied into each view.
    scene: SceneFrame,
    pos: [f32; 3],
    prev: [f32; 3],
    yaw: f32,
    pitch: f32,
}

/// 768-byte palette (Quake/Hexen II `gfx/palette.lmp`, 8-bit RGB).
pub fn palette_from_lmp(data: &[u8]) -> Option<[[u8; 3]; 256]> {
    (data.len() >= 768).then(|| std::array::from_fn(|i| [data[i * 3], data[i * 3 + 1], data[i * 3 + 2]]))
}

/// 16 shade rows from a colormap (`gfx/colormap.lmp`: 64 rows of 256, brightest first; every
/// fourth row), or computed by nearest colour when there is none.
pub fn shade_rows(palette: &[[u8; 3]; 256], colormap: Option<&[u8]>) -> Vec<[u8; 256]> {
    if let Some(cm) = colormap.filter(|c| c.len() >= 64 * 256) {
        return (0..16).map(|k| std::array::from_fn(|i| cm[k * 4 * 256 + i])).collect();
    }
    let nearest = |c: [f32; 3]| {
        (0..256usize)
            .min_by(|&a, &b| {
                let d = |i: usize| palette[i].iter().zip(&c).map(|(&p, &q)| (p as f32 - q).powi(2)).sum::<f32>();
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0) as u8
    };
    (0..16)
        .map(|k| {
            let f = 1.0 - k as f32 / 15.0;
            std::array::from_fn(|i| nearest(palette[i].map(|v| v as f32 * f)))
        })
        .collect()
}

impl BspViewer {
    pub fn new(name: &str, bsp: Bsp, palette: [[u8; 3]; 256], colormap: Option<&[u8]>) -> Self {
        let rows = shade_rows(&palette, colormap);
        let light_fit = fit_light(&palette, &rows);
        let (start, yaw) = bsp.player_start();
        let pos = [start[0], start[1], start[2] + EYE];
        let scene = build_scene(&bsp);
        Self { name: name.to_string(), bsp, palette, rows, light_fit, scene, pos, prev: pos, yaw: yaw.to_radians(), pitch: 0.0 }
    }

    /// Open `maps/<map>.bsp` from PAK archives (later ones override earlier ones), with the
    /// palette and colormap from the same archives.
    pub fn from_paks(paks: &[Pak], map: &str) -> Result<Self, Error> {
        let get = |name: &str| paks.iter().rev().find_map(|p| p.get(name));
        let path = if map.contains('/') { map.to_string() } else { format!("maps/{map}.bsp") };
        let bsp = Bsp::parse(get(&path).ok_or_else(|| Error(format!("{path} not found")))?)?;
        let palette = get("gfx/palette.lmp").and_then(palette_from_lmp).ok_or_else(|| Error("gfx/palette.lmp not found".into()))?;
        Ok(Self::new(map, bsp, palette, get("gfx/colormap.lmp")))
    }

    pub fn bsp(&self) -> &Bsp {
        &self.bsp
    }
}

/// Triangles of the world model: fans over each face, texel UVs, lightmap light per vertex.
fn build_scene(bsp: &Bsp) -> SceneFrame {
    let mut scene = SceneFrame::default();
    for face in &bsp.faces[bsp.world_faces.clone()] {
        if face.points.len() < 3 {
            continue;
        }
        let Some(ti) = bsp.texinfo.get(face.texinfo) else { continue };
        let tex = bsp.textures.get(ti.miptex);
        let sky = tex.is_some_and(|t| t.name.to_ascii_lowercase().starts_with("sky"));
        if sky && scene.sky.is_none() {
            scene.sky = Some(ti.miptex as u32);
        }
        let [a, b, c] = [face.points[0], face.points[1], face.points[2]];
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        let normal = [n[0] / len, n[1] / len, n[2] / len];
        let vertex = |p: [f32; 3]| {
            let s = p[0] * ti.s[0] + p[1] * ti.s[1] + p[2] * ti.s[2] + ti.s[3];
            let t = p[0] * ti.t[0] + p[1] * ti.t[1] + p[2] * ti.t[2] + ti.t[3];
            // Lightmap value (0 dark .. 255 bright) to the engine's darkness levels.
            let light = (255.0 - bsp.light_at(face, p).min(255.0)) / 255.0 * 15.0;
            SceneVertex { pos: p, uv: [s, t], image: ti.miptex as u32, light, flags: if sky { SKY } else { NO_DISTANCE }, normal }
        };
        for k in 1..face.points.len() - 1 {
            // Quake faces wind clockwise seen from the front; the renderer doesn't cull.
            scene.vertices.extend_from_slice(&[vertex(face.points[0]), vertex(face.points[k]), vertex(face.points[k + 1])]);
        }
    }
    scene
}

impl GameModule for BspViewer {
    fn title(&self) -> String {
        format!("{} — BSP viewer", self.name)
    }

    fn tick(&mut self, input: &ModuleInput) {
        self.prev = self.pos;
        self.yaw += input.look.0;
        self.pitch = (self.pitch + input.look.1).clamp(-1.48, 1.48);
        let speed = SPEED / TICK_RATE as f32 * if input.run { 2.0 } else { 1.0 };
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        // Fly where you look; strafe stays level.
        let f = [cy * cp, sy * cp, sp];
        let r = [sy, -cy, 0.0];
        for k in 0..3 {
            self.pos[k] += (f[k] * input.forward + r[k] * input.strafe) * speed + if k == 2 { input.up * speed } else { 0.0 };
        }
    }

    fn view(&mut self, ui: &mut Frame, alpha: f32) -> Option<ModuleView> {
        if ui.coverage.is_none() {
            ui.track_coverage(true);
        }
        ui.clear(0);
        let lerp = |k: usize| self.prev[k] + (self.pos[k] - self.prev[k]) * alpha;
        let (brightness, emissive) = self.light_fit;
        Some(ModuleView {
            camera: SceneCamera { pos: [lerp(0), lerp(1), lerp(2)], angle: self.yaw, pitch: self.pitch.tan(), vfov_degrees: 74.0, freelook: true },
            shading: Shading {
                visibility: 1.0e9,
                fade_bias: 0.0,
                true_color: true,
                fullbright: false,
                max_object_shade: 15.0,
                zero_substitute: 0,
                modern: Some(Modern { brightness, emissive, dynamic_lights: false, bloom: true, tonemap: 1, exposure: 1.4, msaa: 4 }),
            },
            scene: self.scene.clone(),
            epoch: 1,
        })
    }

    fn images(&self) -> Vec<IndexedImage<'_>> {
        self.bsp.textures.iter().map(|t| IndexedImage { width: t.width, height: t.height, pixels: &t.pixels }).collect()
    }

    fn palette(&self) -> &[[u8; 3]; 256] {
        &self.palette
    }

    fn shade_rows(&self) -> &[[u8; 256]] {
        &self.rows
    }
}

/// Open a map for viewing: `source` is a .pak (its sibling pak0..pak9.pak are added, later ones
/// override) with a map name, or a loose .bsp with a palette file (768-byte `palette.lmp`, or a
/// G.O.D.D. `PALETTE.PAL` read by `godd_palette`).
pub fn open(source: &std::path::Path, map: &str, palette: Option<&std::path::Path>) -> Result<BspViewer, Error> {
    let io = |e: std::io::Error| Error(e.to_string());
    if source.to_string_lossy().to_ascii_lowercase().ends_with(".pak") {
        let dir = source.parent().unwrap_or(std::path::Path::new("."));
        let mut paks = Vec::new();
        for i in 0..10 {
            if let Ok(data) = std::fs::read(dir.join(format!("pak{i}.pak"))) {
                paks.push(Pak::parse(data)?);
            }
        }
        if paks.is_empty() {
            paks.push(Pak::parse(std::fs::read(source).map_err(io)?)?);
        }
        return BspViewer::from_paks(&paks, map);
    }
    let bsp = Bsp::parse(&std::fs::read(source).map_err(io)?)?;
    let data = std::fs::read(palette.ok_or_else(|| Error("a loose .bsp needs palette=FILE".into()))?).map_err(io)?;
    let pal = if data.len() == 768 { palette_from_lmp(&data) } else { godd_palette(&data) }.ok_or_else(|| Error("unreadable palette".into()))?;
    let name = source.file_stem().map_or("map".into(), |s| s.to_string_lossy().into_owned());
    Ok(BspViewer::new(&name, bsp, pal, None))
}

/// The 256 colours of a G.O.D.D. PALETTE.PAL (6-bit RGB at offset 8).
pub fn godd_palette(data: &[u8]) -> Option<[[u8; 3]; 256]> {
    let rgb = data.get(8..8 + 768)?;
    let x = |v: u8| ((v.min(63) as u16 * 255) / 63) as u8;
    Some(std::array::from_fn(|i| [x(rgb[i * 3]), x(rgb[i * 3 + 1]), x(rgb[i * 3 + 2])]))
}
