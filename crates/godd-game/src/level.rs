//! Build the engine [`World`] from a compiled orb (`.XRB` + `.XPG`), following
//! re/formats/XRB-records.md.

use crate::anim::{self, Sequence};
use crate::world::{Image, Region, Sprite, Surface, Wall, World};
use godd_formats::xrb::{Orb, RecordExt, Section};
use std::f32::consts::TAU;

const ANGLE_UNITS: f32 = 2048.0;

/// Wall flag bit: blocks movement.
const WALL_SOLID: u32 = 0x2;
/// Region flag bit: no distance shading.
const REGION_FULLBRIGHT: u32 = 0x0400_0000;

/// A loaded level: the world plus metadata the game needs.
pub struct Level {
    pub world: World,
    pub title: String,
    /// Player start: x, y, angle (radians), region.
    pub start: (f32, f32, f32, usize),
    /// Level music sound index, if any.
    pub music: Option<usize>,
    /// Object index → sprite index for objects that became sprites.
    pub object_sprites: Vec<Option<usize>>,
    /// Decoded sequence per Tex index.
    pub sequences: Vec<Option<Sequence>>,
    /// Per sprite: the Tex driving it and its playback state (animated sprites only).
    pub sprite_anims: Vec<Option<(usize, anim::Player)>>,
    pub orb: Orb,
}

fn index(v: i32) -> Option<usize> {
    usize::try_from(v).ok()
}

fn name_at(record: &[u8], offset: usize, len: usize) -> String {
    let bytes = &record[offset..offset + len];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(len);
    bytes[..end].iter().map(|&b| b as char).collect::<String>().trim().to_string()
}

impl Level {
    pub fn load(xrb: &[u8], xpg: &[u8]) -> Result<Self, godd_formats::Error> {
        let orb = Orb::parse(xrb)?;
        // Images, in pic order (pic index == image index).
        let images: Vec<Image> = orb
            .pics
            .iter()
            .map(|pic| match pic.pixels_row_major(xpg) {
                Some(pixels) if pic.width > 0 && pic.height > 0 => {
                    Image { width: pic.width as usize, height: pic.height as usize, pixels }
                }
                _ => Image::solid(1, 1, 0),
            })
            .collect();
        let mut world = World { images, ..World::default() };
        let pic_count = world.images.len();
        let pic = |v: i32| index(v).filter(|&i| i < pic_count);

        let points: Vec<(f32, f32)> = orb.records(Section::Points).iter().map(|p| (p.fixed(0x14), p.fixed(0x18))).collect();
        world.vertices = points.clone();

        let region_count = orb.records(Section::Regions).len();
        for w in orb.records(Section::Walls) {
            let flags = w.le_i32(0x00) as u32;
            let back = index(w.le_i32(0x84)).filter(|&b| b < region_count);
            let surface = |pic_offset: usize| {
                pic(w.le_i32(pic_offset)).map(|image| Surface {
                    image,
                    // The stored x offset is TexX + 1 (renderer.md).
                    offset_u: w.fixed(0x9c) - 1.0,
                    offset_v: w.fixed(0xa0),
                    scale: 1.0,
                })
            };
            let middle = surface(0x4c);
            world.walls.push(Wall {
                a: index(w.le_i32(0x0c)).unwrap_or(0),
                b: index(w.le_i32(0x10)).unwrap_or(0),
                front: index(w.le_i32(0x80)).unwrap_or(0),
                back,
                upper: surface(0x34),
                middle,
                lower: surface(0x64),
                masked: back.is_some() && middle.is_some(),
                blocking: flags & WALL_SOLID != 0,
                light: w.le_u16(0xa4) as i16 as i32,
                colormap: w.le_u16(0xaa) as i16 as i32,
            });
        }

        for r in orb.records(Section::Regions) {
            let flags = r.le_i32(0x00) as u32;
            let start = r.le_i32(0x110).max(0) as usize;
            let count = r.le_i32(0xbc).max(0) as usize;
            let walls = orb
                .wall_list
                .get(start..start + count)
                .unwrap_or_default()
                .iter()
                .filter_map(|&w| index(w).filter(|&w| w < world.walls.len()))
                .collect();
            let flat = |pic_offset: usize, off_u: usize, off_v: usize| {
                pic(r.le_i32(pic_offset)).map(|image| Surface { image, offset_u: r.fixed(off_u), offset_v: r.fixed(off_v), scale: 1.0 })
            };
            world.regions.push(Region {
                floor: r.fixed(0x0c),
                ceiling: r.fixed(0x10),
                floor_surface: flat(0x24, 0x4c, 0x50),
                ceiling_surface: flat(0x3c, 0x54, 0x58),
                floor_light: r.le_u16(0x82) as i16 as i32,
                ceiling_light: r.le_u16(0x84) as i16 as i32,
                floor_colormap: r.le_u16(0x8c) as i16 as i32,
                ceiling_colormap: r.le_u16(0x8e) as i16 as i32,
                fullbright: flags & REGION_FULLBRIGHT != 0,
                walls,
                bounds: ((r.fixed(0xc4), r.fixed(0xcc)), (r.fixed(0xc8), r.fixed(0xd0))),
                below: index(r.le_i32(0x5c)).filter(|&i| i < region_count),
                above: index(r.le_i32(0x60)).filter(|&i| i < region_count),
            });
        }

        // Objects: 0 is the player; header +0x4c is an inert start copy.
        let objects = orb.records(Section::Objects);
        let start_copy = xrb.get(0x4c..0x50).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).and_then(index);
        let position = |o: &[u8]| -> Option<(f32, f32)> { index(o.le_i32(0x14)).and_then(|p| points.get(p).copied()) };
        let angle_of = |o: &[u8]| o.le_u16(0x24) as f32 / ANGLE_UNITS * TAU;
        let start = objects
            .first()
            .and_then(|o| position(o).map(|(x, y)| (x, y, angle_of(o), index(o.le_i32(0xfc)).unwrap_or(0))))
            .unwrap_or((0.0, 0.0, 0.0, 0));

        let sequences: Vec<Option<Sequence>> = orb.texs.iter().map(Sequence::from_tex).collect();
        let mut sprite_anims = Vec::new();
        let mut object_sprites = vec![None; objects.len()];
        for (i, o) in objects.iter().enumerate() {
            if i == 0 || Some(i) == start_copy || name_at(o, 0x58, 8) == "NETPLAYR" {
                continue;
            }
            let Some((x, y)) = position(o) else { continue };
            // Objects outside every region are templates (tools, projectiles, explosions).
            let Some(region) = index(o.le_i32(0xfc)).filter(|&r| r < region_count) else { continue };
            let Some(tex_index) = index(o.le_i32(0x64)).filter(|&t| t < orb.texs.len()) else { continue };
            let Some(seq) = sequences[tex_index].as_ref() else { continue };
            let player = anim::Player::start(seq, 0);
            let frames: Vec<(usize, bool)> = seq.rotations_of(player.entry).into_iter().filter(|&(p, _)| p < pic_count).collect();
            if frames.is_empty() {
                continue;
            }
            sprite_anims.push(seq.is_animated().then_some((tex_index, player)));
            object_sprites[i] = Some(world.sprites.len());
            world.sprites.push(Sprite {
                x,
                y,
                z: o.fixed(0x2c),
                angle: angle_of(o),
                region,
                frames,
                scale: 1.0,
                visible: true,
                height: o.fixed(0x34),
            });
        }

        // Tail: title @0x08, sky texture slot @0x48 (pic @0x50), music sound @0x78.
        let tail = &orb.tail;
        let title = name_at(tail, 0x08, 16);
        world.sky = pic(tail.le_i32(0x50));
        let music = index(tail.le_i32(0x78));

        Ok(Self { world, title, start, music, object_sprites, sequences, sprite_anims, orb })
    }
}

impl Level {
    /// Add a runtime sprite driven by `tex` (projectiles, explosions, spawned objects).
    pub fn add_sprite(&mut self, mut sprite: Sprite, tex: usize) -> Option<usize> {
        let seq = self.sequences.get(tex)?.as_ref()?;
        let player = anim::Player::start(seq, 0);
        sprite.frames = seq.rotations_of(player.entry).into_iter().filter(|&(p, _)| p < self.world.images.len()).collect();
        if sprite.frames.is_empty() {
            return None;
        }
        self.world.sprites.push(sprite);
        self.sprite_anims.push(seq.is_animated().then_some((tex, player)));
        Some(self.world.sprites.len() - 1)
    }

    /// Switch a sprite to another Tex (an object changed class).
    pub fn set_sprite_tex(&mut self, sprite: usize, tex: Option<usize>) {
        let Some(seq) = tex.and_then(|t| self.sequences.get(t)?.as_ref().map(|s| (t, s))) else {
            if let Some(s) = self.world.sprites.get_mut(sprite) {
                s.frames.clear();
            }
            return;
        };
        let (t, seq) = seq;
        let player = anim::Player::start(seq, 0);
        let frames: Vec<(usize, bool)> = seq.rotations_of(player.entry).into_iter().filter(|&(p, _)| p < self.world.images.len()).collect();
        if let Some(s) = self.world.sprites.get_mut(sprite) {
            s.frames = frames;
        }
        if let Some(slot) = self.sprite_anims.get_mut(sprite) {
            *slot = seq.is_animated().then_some((t, player));
        }
    }

    /// Advance sprite animations by `dt` seconds.
    pub fn animate(&mut self, dt: f32) {
        let pic_count = self.world.images.len();
        for (sprite, slot) in self.world.sprites.iter_mut().zip(self.sprite_anims.iter_mut()) {
            let Some((tex, player)) = slot else { continue };
            let Some(seq) = self.sequences[*tex].as_ref() else { continue };
            if player.advance(seq, dt) {
                let frames: Vec<(usize, bool)> = seq.rotations_of(player.entry).into_iter().filter(|&(p, _)| p < pic_count).collect();
                if !frames.is_empty() {
                    sprite.frames = frames;
                }
            }
        }
    }

    /// Name of the level's music track (Sounds record +0x0c), if any.
    pub fn music_name(&self) -> Option<String> {
        let record = self.orb.records(Section::Sounds).get(self.music?)?;
        let name = name_at(record, 0x0c, 12);
        (!name.is_empty()).then_some(name)
    }
}
