//! Texture packs: high-resolution replacements for a game's images, built locally from the
//! user's own files (never shipped). A pack is a folder with `pack.txt` and PNG files:
//!
//! ```text
//! pack lips-hd scale=4 game=godd
//! tex DEF_Flor 9f3c8a01e2d4b567 flat 64 64 t/DEF_Flor.png t/DEF_Flor_n.png
//! ```
//!
//! Each `tex` line: name, content hash of the original pixels (so same-named images that differ
//! between levels stay apart; `-` matches by name only), class, the original (logical) size,
//! the albedo PNG and an optional normal map (`-` for none). Logical size stays the original's,
//! so texture coordinates never change.

use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub struct PackEntry {
    pub name: String,
    pub hash: Option<u64>,
    pub class: String,
    pub logical: (usize, usize),
    pub albedo: String,
    pub normal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PackManifest {
    pub name: String,
    pub scale: u32,
    pub game: String,
    pub entries: Vec<PackEntry>,
}

/// FNV-1a over an image's size and pixels: identifies one original image.
pub fn image_hash(width: usize, height: usize, pixels: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in (width as u32).to_le_bytes().iter().chain(&(height as u32).to_le_bytes()).chain(pixels) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl PackManifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut m = PackManifest { scale: 1, ..Default::default() };
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                [] => {}
                ["pack", name, rest @ ..] => {
                    m.name = name.to_string();
                    for kv in rest {
                        match kv.split_once('=') {
                            Some(("scale", v)) => m.scale = v.parse().map_err(|_| format!("line {}: bad scale", n + 1))?,
                            Some(("game", v)) => m.game = v.to_string(),
                            _ => {}
                        }
                    }
                }
                ["tex", name, hash, class, w, h, albedo, rest @ ..] => {
                    let num = |v: &str| v.parse::<usize>().map_err(|_| format!("line {}: bad size", n + 1));
                    m.entries.push(PackEntry {
                        name: name.to_string(),
                        hash: (*hash != "-").then(|| u64::from_str_radix(hash, 16)).transpose().map_err(|_| format!("line {}: bad hash", n + 1))?,
                        class: class.to_string(),
                        logical: (num(w)?, num(h)?),
                        albedo: albedo.to_string(),
                        normal: rest.first().filter(|v| **v != "-").map(|v| v.to_string()),
                    });
                }
                _ => return Err(format!("line {}: cannot read {line:?}", n + 1)),
            }
        }
        Ok(m)
    }

    pub fn write(&self) -> String {
        let mut out = format!("pack {} scale={} game={}\n", self.name, self.scale, self.game);
        for e in &self.entries {
            let hash = e.hash.map_or("-".to_string(), |h| format!("{h:016x}"));
            let _ = writeln!(
                out,
                "tex {} {hash} {} {} {} {} {}",
                e.name,
                e.class,
                e.logical.0,
                e.logical.1,
                e.albedo,
                e.normal.as_deref().unwrap_or("-")
            );
        }
        out
    }

    /// The entry for an image: exact (name and hash) first, then name only.
    pub fn find(&self, name: &str, hash: u64) -> Option<&PackEntry> {
        let named = |e: &&PackEntry| e.name.eq_ignore_ascii_case(name);
        self.entries.iter().filter(named).find(|e| e.hash == Some(hash)).or_else(|| self.entries.iter().filter(named).find(|e| e.hash.is_none()))
    }
}

/// A pack read from a directory (native front ends).
#[derive(Debug, Clone)]
pub struct LoadedPack {
    pub dir: std::path::PathBuf,
    pub manifest: PackManifest,
}

impl LoadedPack {
    pub fn open(dir: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(dir.join("pack.txt")).map_err(|e| format!("{}: {e}", dir.join("pack.txt").display()))?;
        Ok(Self { dir: dir.to_path_buf(), manifest: PackManifest::parse(&text)? })
    }

    /// Decoded (albedo, normal) for each image key (name, content hash); `None` where the pack
    /// has no entry or a file is unreadable.
    pub fn textures(&self, keys: &[(String, u64)]) -> Vec<Option<(Rgba, Option<Rgba>)>> {
        let read = |rel: &str| std::fs::read(self.dir.join(rel)).ok().and_then(|d| decode_png(&d).ok());
        let one = |(name, hash): &(String, u64)| {
            let e = self.manifest.find(name, *hash)?;
            Some((read(&e.albedo)?, e.normal.as_deref().and_then(read)))
        };
        parallel_map(keys, one)
    }
}

/// `f` over `items` on all CPU cores, keeping the order.
pub fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(items.len().max(1));
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items.chunks(chunk).map(|part| scope.spawn(|| part.iter().map(&f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker panicked")).collect()
    })
}

/// A decoded RGBA8 image.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// Decode a PNG to RGBA8 (8-bit gray, gray-alpha, RGB, RGBA and palette images).
pub fn decode_png(data: &[u8]) -> Result<Rgba, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let src = &buf[..info.buffer_size()];
    let pixels = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => src.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => src.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => src.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("unexpanded palette image".into()),
    };
    Ok(Rgba { width: w, height: h, pixels })
}

pub fn encode_png(image: &Rgba) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width as u32, image.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&image.pixels)).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip_and_lookup() {
        let text = "pack lips-hd scale=4 game=godd # comment\n\ntex WALL01 00000000000000ff wall 64 64 t/WALL01.png t/WALL01_n.png\ntex WALL01 - wall 64 64 t/WALL01b.png -\n";
        let m = PackManifest::parse(text).unwrap();
        assert_eq!((m.name.as_str(), m.scale, m.game.as_str(), m.entries.len()), ("lips-hd", 4, "godd", 2));
        assert_eq!(m.find("wall01", 0xff).unwrap().albedo, "t/WALL01.png");
        assert_eq!(m.find("WALL01", 7).unwrap().albedo, "t/WALL01b.png", "falls back to the name-only entry");
        assert!(m.find("NOPE", 0).is_none());
        assert_eq!(PackManifest::parse(&m.write()).unwrap(), m);
        assert!(PackManifest::parse("tex broken").is_err());
    }

    #[test]
    fn png_round_trip_and_hash() {
        let img = Rgba { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 0, 255, 128] };
        assert_eq!(decode_png(&encode_png(&img).unwrap()).unwrap(), img);
        assert_ne!(image_hash(2, 1, &[1, 2]), image_hash(1, 2, &[1, 2]), "size is part of the hash");
    }
}
