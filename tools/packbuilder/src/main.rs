//! `packbuilder`: build an HD texture pack from the user's own game files.
//!
//!     packbuilder <GAME_DIR> <OUT_DIR> [--upscaler scale2x|esrgan] [--levels LIPS00,LIPS01] [--normal-strength 2.0]
//!
//! Every image of every level is exported once (deduplicated by name and content), classified
//! by use (wall, flat, masked, sprite, sky, other), upscaled 4x, given a normal map, and listed
//! in OUT_DIR/pack.txt (see engine_core::pack). Tiling classes are upscaled with wrap-around
//! padding so their edges stay seamless; transparent texels (index 0 on sprites and masked walls)
//! keep a clean alpha. Packs are built locally and must not be redistributed.

mod upscale;

use engine_core::pack::{PackEntry, PackManifest, Rgba, encode_png, image_hash};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type AnyResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const SCALE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Wall,
    Flat,
    Masked,
    Sprite,
    Sky,
    Other,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Wall => "wall",
            Class::Flat => "flat",
            Class::Masked => "masked",
            Class::Sprite => "sprite",
            Class::Sky => "sky",
            Class::Other => "other",
        }
    }

    /// Repeats across surfaces: upscale with wrap-around so the edges match.
    pub fn tiles(self) -> bool {
        matches!(self, Class::Wall | Class::Flat | Class::Masked | Class::Sky)
    }

    /// Texel 0 is see-through.
    pub fn transparent(self) -> bool {
        matches!(self, Class::Masked | Class::Sprite)
    }
}

struct Source {
    name: String,
    hash: u64,
    class: Class,
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

fn main() -> AnyResult {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(game), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("usage: packbuilder <GAME_DIR> <OUT_DIR> [--upscaler scale2x|esrgan] [--levels LIPS00,...] [--normal-strength N]");
        std::process::exit(2);
    };
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let upscaler = opt("--upscaler").unwrap_or_else(|| if upscale::esrgan_available() { "esrgan".into() } else { "scale2x".into() });
    let strength: f32 = opt("--normal-strength").and_then(|v| v.parse().ok()).unwrap_or(2.0);
    let vfs = godd_game::Vfs::from_dir(Path::new(game))?;
    let palette = godd_formats::pal::Palette::parse(vfs.read("PALETTE.PAL")?)?.rgb;
    let levels: Vec<String> = match opt("--levels") {
        Some(list) => list.split(',').map(str::to_ascii_uppercase).collect(),
        None => std::iter::once("LIPSTART".to_string()).chain((0..18).map(|i| format!("LIPS{i:02}"))).collect(),
    };
    let sources = collect(&vfs, &levels)?;
    let out = PathBuf::from(out);
    std::fs::create_dir_all(out.join("t"))?;
    println!("{} unique images from {} levels; upscaler {upscaler}", sources.len(), levels.len());
    let mut manifest = PackManifest { name: out.file_name().map_or("pack".into(), |n| n.to_string_lossy().into_owned()), scale: SCALE as u32, game: "godd".into(), spec: true, entries: Vec::new() };
    let rgba: Vec<Rgba> = sources.iter().map(|s| to_rgba(s, &palette)).collect();
    let classes: Vec<Class> = sources.iter().map(|s| s.class).collect();
    let big = upscale::upscale_all(&upscaler, &rgba, &classes, SCALE)?;
    for ((s, img), up) in sources.iter().zip(&rgba).zip(big) {
        let _ = img;
        let stem = format!("{}_{:016x}", sanitize(&s.name), s.hash);
        let albedo = format!("t/{stem}.png");
        std::fs::write(out.join(&albedo), encode_png(&up)?)?;
        let normal = (s.class != Class::Sky && s.class != Class::Other).then(|| {
            let n = normal_map(&up, strength, s.class.tiles());
            let path = format!("t/{stem}_n.png");
            std::fs::write(out.join(&path), encode_png(&n).unwrap_or_default()).map(|_| path)
        });
        manifest.entries.push(PackEntry {
            name: s.name.clone(),
            hash: Some(s.hash),
            class: s.class.name().into(),
            logical: (s.width, s.height),
            albedo,
            normal: normal.transpose()?,
        });
    }
    std::fs::write(out.join("pack.txt"), manifest.write())?;
    println!("wrote {} ({} textures)", out.join("pack.txt").display(), manifest.entries.len());
    Ok(())
}

/// Every image of the given levels, once per (name, content), with the class of its first
/// use that matters most (sky > masked > wall > flat > sprite > other).
fn collect(vfs: &godd_game::Vfs, levels: &[String]) -> AnyResult<Vec<Source>> {
    let mut found: BTreeMap<(String, u64), Source> = BTreeMap::new();
    for level in levels {
        let (Ok(xrb), Ok(xpg)) = (vfs.read(&format!("{level}.XRB")), vfs.read(&format!("{level}.XPG"))) else { continue };
        let lvl = godd_game::level::Level::load(xrb, xpg)?;
        let w = &lvl.world;
        let mut class = vec![Class::Other; w.images.len()];
        let mut mark = |i: usize, c: Class| {
            if let Some(slot) = class.get_mut(i) {
                let rank = |c: Class| match c {
                    Class::Sky => 5,
                    Class::Masked => 4,
                    Class::Wall => 3,
                    Class::Flat => 2,
                    Class::Sprite => 1,
                    Class::Other => 0,
                };
                if rank(c) > rank(*slot) {
                    *slot = c;
                }
            }
        };
        for wall in &w.walls {
            let masked = wall.masked || wall.back == Some(wall.front);
            for s in [wall.upper, wall.lower].into_iter().flatten() {
                mark(s.image, Class::Wall);
            }
            if let Some(s) = wall.middle {
                mark(s.image, if masked { Class::Masked } else { Class::Wall });
            }
        }
        for r in &w.regions {
            for s in [r.floor_surface, r.ceiling_surface].into_iter().flatten() {
                mark(s.image, Class::Flat);
            }
        }
        for seq in lvl.sequences.iter().flatten() {
            for &(p, _) in &seq.pics {
                if let Some(p) = p {
                    mark(p, Class::Sprite);
                }
            }
        }
        if let Some(s) = w.sky {
            mark(s, Class::Sky);
        }
        for (i, img) in w.images.iter().enumerate() {
            if img.width < 2 || img.height < 2 {
                continue;
            }
            let name = lvl.orb.pics.get(i).map_or_else(|| format!("PIC{i}"), |p| p.name.trim().to_string());
            let hash = image_hash(img.width, img.height, &img.pixels);
            let entry = found.entry((name.to_ascii_uppercase(), hash)).or_insert_with(|| Source {
                name,
                hash,
                class: class[i],
                width: img.width,
                height: img.height,
                pixels: img.pixels.clone(),
            });
            // Keep the most demanding class seen across levels.
            if class[i] > entry.class && class[i] != Class::Other {
                entry.class = class[i];
            }
        }
    }
    Ok(found.into_values().collect())
}

fn to_rgba(s: &Source, palette: &[[u8; 3]; 256]) -> Rgba {
    let pixels = s
        .pixels
        .iter()
        .flat_map(|&i| {
            let [r, g, b] = palette[i as usize];
            let a = if i == 0 && s.class.transparent() { 0 } else { 255 };
            [r, g, b, a]
        })
        .collect();
    Rgba { width: s.width, height: s.height, pixels }
}

/// Specular amount (alpha of the normal map): bright texels and fine raised detail (metal
/// edges, wet stone) shine; dark, flat areas stay matte.
pub fn specular(lum: f32, blurred: f32) -> u8 {
    let detail = (lum - blurred).max(0.0) * 4.0;
    ((lum * lum * 0.6 + detail).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Tangent-space normal map from luminance as height (blurred, Sobel), wrapping for tiles;
/// specular in alpha.
pub fn normal_map(img: &Rgba, strength: f32, wrap: bool) -> Rgba {
    let (w, h) = (img.width, img.height);
    let lum: Vec<f32> = img.pixels.as_chunks::<4>().0.iter().map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0).collect();
    let at = |v: &[f32], x: isize, y: isize| {
        let (x, y) = if wrap {
            (x.rem_euclid(w as isize) as usize, y.rem_euclid(h as isize) as usize)
        } else {
            (x.clamp(0, w as isize - 1) as usize, y.clamp(0, h as isize - 1) as usize)
        };
        v[y * w + x]
    };
    // 3x3 blur so the upscaler's fine noise doesn't become bumps.
    let blur: Vec<f32> = (0..h as isize)
        .flat_map(|y| (0..w as isize).map(move |x| (x, y)))
        .map(|(x, y)| (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))).map(|(dx, dy)| at(&lum, x + dx, y + dy)).sum::<f32>() / 9.0)
        .collect();
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h as isize {
        for x in 0..w as isize {
            let s = |dx, dy| at(&blur, x + dx, y + dy);
            let gx = (s(1, -1) + 2.0 * s(1, 0) + s(1, 1)) - (s(-1, -1) + 2.0 * s(-1, 0) + s(-1, 1));
            let gy = (s(-1, 1) + 2.0 * s(0, 1) + s(1, 1)) - (s(-1, -1) + 2.0 * s(0, -1) + s(1, -1));
            let (nx, ny, nz) = (-gx * strength, -gy * strength, 1.0f32);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            let enc = |v: f32| ((v / len * 0.5 + 0.5) * 255.0).round() as u8;
            pixels.extend_from_slice(&[enc(nx), enc(ny), enc(nz), specular(at(&lum, x, y), s(0, 0))]);
        }
    }
    Rgba { width: w, height: h, pixels }
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect()
}
