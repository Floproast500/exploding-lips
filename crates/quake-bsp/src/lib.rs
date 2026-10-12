//! Quake-family BSP (version 29) as an engine scene: Quake, Hexen II (its 80-byte model
//! records are detected) and the BSP in Exploding Lips' data. Reads PAK archives, the palette and
//! colormap, level-0 mip textures, faces with lightmaps, and the player start. Written from the
//! publicly documented file formats.

pub mod viewer;

use std::collections::HashMap;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

fn u32_at(d: &[u8], o: usize) -> Result<u32, Error> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| Error(format!("truncated at {o}")))
}

fn i32_at(d: &[u8], o: usize) -> Result<i32, Error> {
    u32_at(d, o).map(|v| v as i32)
}

fn f32_at(d: &[u8], o: usize) -> Result<f32, Error> {
    u32_at(d, o).map(f32::from_bits)
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, Error> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| Error(format!("truncated at {o}")))
}

fn cstr(b: &[u8]) -> String {
    String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or(b)).into_owned()
}

/// A PAK archive: name → file bytes (names lower-case).
pub struct Pak {
    files: HashMap<String, (usize, usize)>,
    data: Vec<u8>,
}

impl Pak {
    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        if data.get(0..4) != Some(b"PACK") {
            return err("not a PAK file");
        }
        let (dir, len) = (u32_at(&data, 4)? as usize, u32_at(&data, 8)? as usize);
        let mut files = HashMap::new();
        for i in 0..len / 64 {
            let e = dir + i * 64;
            let name = cstr(data.get(e..e + 56).ok_or_else(|| Error("bad directory".into()))?).to_ascii_lowercase();
            let (pos, size) = (u32_at(&data, e + 56)? as usize, u32_at(&data, e + 60)? as usize);
            if pos + size <= data.len() {
                files.insert(name, (pos, size));
            }
        }
        Ok(Self { files, data })
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files.get(&name.to_ascii_lowercase()).map(|&(p, s)| &self.data[p..p + s])
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

#[derive(Debug, Clone)]
pub struct MipTex {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Level-0 palette indices (empty when the texture is external / missing).
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct TexInfo {
    pub s: [f32; 4],
    pub t: [f32; 4],
    pub miptex: usize,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct Face {
    /// Vertex positions in order (from the edge loop).
    pub points: Vec<[f32; 3]>,
    pub texinfo: usize,
    pub light_offset: Option<usize>,
    pub styles: [u8; 4],
}

#[derive(Debug)]
pub struct Bsp {
    pub textures: Vec<MipTex>,
    pub texinfo: Vec<TexInfo>,
    pub faces: Vec<Face>,
    /// Faces of the world model (model 0); the rest are doors, lifts etc.
    pub world_faces: std::ops::Range<usize>,
    pub lighting: Vec<u8>,
    pub entities: String,
}

const LUMP_ENTITIES: usize = 0;
const LUMP_TEXTURES: usize = 2;
const LUMP_VERTICES: usize = 3;
const LUMP_TEXINFO: usize = 6;
const LUMP_FACES: usize = 7;
const LUMP_LIGHTING: usize = 8;
const LUMP_EDGES: usize = 12;
const LUMP_SURFEDGES: usize = 13;
const LUMP_MODELS: usize = 14;

impl Bsp {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        if i32_at(d, 0)? != 29 {
            return err("not a version 29 BSP");
        }
        let lump = |i: usize| -> Result<&[u8], Error> {
            let (o, l) = (i32_at(d, 4 + i * 8)? as usize, i32_at(d, 8 + i * 8)? as usize);
            d.get(o..o + l).ok_or_else(|| Error(format!("lump {i} out of range")))
        };
        let verts: Vec<[f32; 3]> = lump(LUMP_VERTICES)?.as_chunks::<12>().0.iter().map(|c| [f32_at(c, 0).unwrap_or(0.0), f32_at(c, 4).unwrap_or(0.0), f32_at(c, 8).unwrap_or(0.0)]).collect();
        let edges: Vec<(u16, u16)> = lump(LUMP_EDGES)?.as_chunks::<4>().0.iter().map(|c| (u16_at(c, 0).unwrap_or(0), u16_at(c, 2).unwrap_or(0))).collect();
        let surfedges: Vec<i32> = lump(LUMP_SURFEDGES)?.as_chunks::<4>().0.iter().map(|c| i32_at(c, 0).unwrap_or(0)).collect();
        let texinfo: Vec<TexInfo> = lump(LUMP_TEXINFO)?
            .as_chunks::<40>()
            .0
            .iter()
            .map(|c| {
                let f = |o| f32_at(c, o).unwrap_or(0.0);
                TexInfo { s: [f(0), f(4), f(8), f(12)], t: [f(16), f(20), f(24), f(28)], miptex: u32_at(c, 32).unwrap_or(0) as usize, flags: u32_at(c, 36).unwrap_or(0) }
            })
            .collect();
        let mut faces = Vec::new();
        for c in lump(LUMP_FACES)?.as_chunks::<20>().0 {
            let (first, count) = (i32_at(c, 4)? as usize, u16_at(c, 8)? as usize);
            let points = (first..first + count)
                .filter_map(|i| {
                    let se = *surfedges.get(i)?;
                    let (a, b) = *edges.get(se.unsigned_abs() as usize)?;
                    verts.get(if se >= 0 { a } else { b } as usize).copied()
                })
                .collect();
            let light = i32_at(c, 16)?;
            faces.push(Face { points, texinfo: u16_at(c, 10)? as usize, light_offset: (light >= 0).then_some(light as usize), styles: [c[12], c[13], c[14], c[15]] });
        }
        // Model 0 is the world. Quake model records are 64 bytes, Hexen II's 80 (more hulls):
        // take the size whose models' face ranges tile the face list exactly.
        let models = lump(LUMP_MODELS)?;
        let ranges = |record: usize| -> Option<Vec<(usize, usize)>> {
            if models.len() % record != 0 || models.is_empty() {
                return None;
            }
            let r: Vec<(usize, usize)> = models
                .chunks_exact(record)
                .map(|m| (i32_at(m, record - 8).unwrap_or(-1), i32_at(m, record - 4).unwrap_or(-1)))
                .map(|(f, n)| (f.max(0) as usize, n.max(0) as usize))
                .collect();
            let mut next = 0;
            for &(f, n) in &r {
                if f != next {
                    return None;
                }
                next = f + n;
            }
            (next == faces.len()).then_some(r)
        };
        let (first, count) = ranges(64).or_else(|| ranges(80)).and_then(|r| r.first().copied()).unwrap_or((0, faces.len()));
        let world_faces = first.min(faces.len())..(first + count).min(faces.len());
        let tex = lump(LUMP_TEXTURES)?;
        let mut textures = Vec::new();
        if tex.len() >= 4 {
            let n = i32_at(tex, 0)?.max(0) as usize;
            for k in 0..n {
                let off = i32_at(tex, 4 + 4 * k)?;
                let t = (off >= 0).then(|| tex.get(off as usize..)).flatten();
                let Some(t) = t.filter(|t| t.len() >= 40) else {
                    textures.push(MipTex { name: String::new(), width: 1, height: 1, pixels: vec![0] });
                    continue;
                };
                let (w, h) = (u32_at(t, 16)? as usize, u32_at(t, 20)? as usize);
                let pix_off = u32_at(t, 24)? as usize;
                let pixels = t.get(pix_off..pix_off + w * h).map(<[u8]>::to_vec).unwrap_or_else(|| vec![0; w * h]);
                textures.push(MipTex { name: cstr(&t[0..16]), width: w.max(1), height: h.max(1), pixels });
            }
        }
        Ok(Self { textures, texinfo, faces, world_faces, lighting: lump(LUMP_LIGHTING)?.to_vec(), entities: cstr(lump(LUMP_ENTITIES)?) })
    }

    /// Key/value blocks of the entity lump.
    pub fn entity_list(&self) -> Vec<HashMap<String, String>> {
        let mut out = Vec::new();
        let mut cur: Option<HashMap<String, String>> = None;
        for line in self.entities.lines().map(str::trim) {
            match line {
                "{" => cur = Some(HashMap::new()),
                "}" => out.extend(cur.take()),
                _ => {
                    let parts: Vec<&str> = line.split('"').collect();
                    if let (Some(map), [_, k, _, v, ..]) = (cur.as_mut(), parts.as_slice()) {
                        map.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
        out
    }

    /// Player start (origin, yaw degrees), or the first entity with an origin.
    pub fn player_start(&self) -> ([f32; 3], f32) {
        let ents = self.entity_list();
        let parse = |e: &HashMap<String, String>| -> Option<([f32; 3], f32)> {
            let o: Vec<f32> = e.get("origin")?.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let yaw = e.get("angle").and_then(|a| a.parse().ok()).unwrap_or(0.0);
            (o.len() == 3).then(|| ([o[0], o[1], o[2]], yaw))
        };
        ents.iter()
            .filter(|e| e.get("classname").is_some_and(|c| c == "info_player_start" || c == "info_player_deathmatch" || c == "info_player_coop"))
            .find_map(parse)
            .or_else(|| ents.iter().find_map(parse))
            .unwrap_or(([0.0; 3], 0.0))
    }

    /// Light (0..255, 128 = normal) at a face point, from the face's lightmap (style 0).
    pub fn light_at(&self, face: &Face, p: [f32; 3]) -> f32 {
        let Some(base) = face.light_offset else { return 200.0 };
        let ti = &self.texinfo[face.texinfo.min(self.texinfo.len() - 1)];
        let st = |q: [f32; 3]| (q[0] * ti.s[0] + q[1] * ti.s[1] + q[2] * ti.s[2] + ti.s[3], q[0] * ti.t[0] + q[1] * ti.t[1] + q[2] * ti.t[2] + ti.t[3]);
        let (mut smin, mut tmin, mut smax, mut tmax) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &q in &face.points {
            let (s, t) = st(q);
            (smin, tmin, smax, tmax) = (smin.min(s), tmin.min(t), smax.max(s), tmax.max(t));
        }
        let (s0, t0) = ((smin / 16.0).floor(), (tmin / 16.0).floor());
        let w = ((smax / 16.0).ceil() - s0) as usize + 1;
        let h = ((tmax / 16.0).ceil() - t0) as usize + 1;
        let (s, t) = st(p);
        let x = ((s / 16.0 - s0).round().max(0.0) as usize).min(w - 1);
        let y = ((t / 16.0 - t0).round().max(0.0) as usize).min(h - 1);
        self.lighting.get(base + y * w + x).map_or(200.0, |&l| l as f32)
    }
}
