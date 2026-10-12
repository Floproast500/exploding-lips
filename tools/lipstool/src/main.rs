//! `lipstool`: inspect and convert Exploding Lips data files.
//!
//! Usage:
//!   lipstool info  <LEVEL.XRB>
//!   lipstool pics  <LEVEL.XRB> <PALETTE.PAL> <OUT_DIR>
//!   lipstool pic   <FILE.PIC> <PALETTE.PAL> <OUT.png>
//!   lipstool map   <LEVEL.XRB> <OUT.png>          top-down wall plot
//!   lipstool biosfont <OUT.bin>                    IBM 8x8 glyphs 0-127 in BIOS ROM layout
//!   lipstool render <GAME_DIR> <LEVEL> <OUT.png> [WxH] [turn_degrees] [forward_units] [truecolor]
//!                                                  render the player's start view headlessly

use godd_formats::xrb::{Orb, RecordExt, Section};
use godd_formats::{pal::Palette, pic::Pic};
use std::error::Error;
use std::path::Path;

type AnyResult<T = ()> = Result<T, Box<dyn Error>>;

fn main() -> AnyResult {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["info", xrb] => info(Path::new(xrb)),
        ["pics", xrb, pal, out] => export_pics(Path::new(xrb), Path::new(pal), Path::new(out)),
        ["pic", pic, pal, out] => export_pic(Path::new(pic), Path::new(pal), Path::new(out)),
        ["map", xrb, out] => export_map(Path::new(xrb), Path::new(out)),
        ["biosfont", out] => export_bios_font(Path::new(out)),
        ["render", dir, level, out, rest @ ..] => render_view(Path::new(dir), level, Path::new(out), rest),
        _ => {
            eprintln!(
                "usage:\n  lipstool info <LEVEL.XRB>\n  lipstool pics <LEVEL.XRB> <PALETTE.PAL> <OUT_DIR>\n  lipstool pic <FILE.PIC> <PALETTE.PAL> <OUT.png>\n  lipstool map <LEVEL.XRB> <OUT.png>\n  lipstool biosfont <OUT.bin>\n  lipstool render <GAME_DIR> <LEVEL> <OUT.png> [WxH] [turn_degrees] [forward_units]"
            );
            std::process::exit(2);
        }
    }
}

fn info(xrb: &Path) -> AnyResult {
    let orb = Orb::parse(&std::fs::read(xrb)?)?;
    println!("source: {}", orb.header.source_path);
    println!("build:  {:#010x}", orb.header.build);
    println!("xpg:    {} bytes", orb.header.xpg_size);
    for (section, d) in Section::ALL.iter().zip(&orb.sections) {
        println!("  {:<13} used {:>5} / allocated {:>5}  ({} B records)", format!("{section:?}"), d.used, d.allocated, d.record_len);
    }
    Ok(())
}

fn export_pics(xrb: &Path, pal: &Path, out: &Path) -> AnyResult {
    let orb = Orb::parse(&std::fs::read(xrb)?)?;
    let xpg = std::fs::read(xrb.with_extension("XPG"))?;
    let pal = Palette::parse(&std::fs::read(pal)?)?;
    std::fs::create_dir_all(out)?;
    let mut written = 0;
    for (i, pic) in orb.pics.iter().enumerate() {
        let Some(pixels) = pic.pixels_row_major(&xpg) else { continue };
        if pic.width == 0 || pic.height == 0 {
            continue;
        }
        let file = out.join(format!("{i:04}_{}.png", sanitize(&pic.name)));
        write_png(&file, pic.width.into(), pic.height.into(), &pixels, &pal)?;
        written += 1;
    }
    println!("wrote {written} pics to {}", out.display());
    Ok(())
}

fn export_pic(pic: &Path, pal: &Path, out: &Path) -> AnyResult {
    let pic = Pic::parse(&std::fs::read(pic)?)?;
    let pal = Palette::parse(&std::fs::read(pal)?)?;
    write_png(out, pic.width.into(), pic.height.into(), &pic.pixels, &pal)
}

/// Plot every wall as a line between its two points (Points +0x14/+0x18, Walls +0x0c/+0x10).
fn export_map(xrb: &Path, out: &Path) -> AnyResult {
    const SIZE: usize = 2048;
    let orb = Orb::parse(&std::fs::read(xrb)?)?;
    let points: Vec<(f32, f32)> = orb.records(Section::Points).iter().map(|p| (p.fixed(0x14), p.fixed(0x18))).collect();
    let walls: Vec<(usize, usize)> = orb
        .records(Section::Walls)
        .iter()
        .map(|w| (w.le_i32(0x0c) as usize, w.le_i32(0x10) as usize))
        .filter(|&(a, b)| a < points.len() && b < points.len())
        .collect();
    let (mut min, mut max) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
    for &(a, b) in &walls {
        for &(x, y) in [&points[a], &points[b]] {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
    }
    let scale = (SIZE as f32 - 16.0) / (max.0 - min.0).max(max.1 - min.1).max(1.0);
    let to_px = |(x, y): (f32, f32)| (8.0 + (x - min.0) * scale, SIZE as f32 - 8.0 - (y - min.1) * scale);
    let mut rgb = vec![16u8; SIZE * SIZE * 3];
    for &(a, b) in &walls {
        let ((x0, y0), (x1, y1)) = (to_px(points[a]), to_px(points[b]));
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = ((x0 + (x1 - x0) * t) as usize, (y0 + (y1 - y0) * t) as usize);
            if x < SIZE && y < SIZE {
                rgb[(y * SIZE + x) * 3..][..3].copy_from_slice(&[230, 220, 200]);
            }
        }
    }
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(out)?), SIZE as u32, SIZE as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;
    println!("{} walls, {} points, extent {:?}..{:?}", walls.len(), points.len(), min, max);
    Ok(())
}

/// The original reads text glyphs from the PC BIOS 8x8 font (F000:FA6E), which Wine leaves
/// empty. font8x8's basic set is the same IBM glyph set stored LSB-left; flip it to the ROM's
/// MSB-left rows so tools/pilot can inject it.
fn export_bios_font(out: &Path) -> AnyResult {
    let mut rom = Vec::with_capacity(128 * 8);
    for glyph in font8x8::legacy::BASIC_LEGACY {
        rom.extend(glyph.iter().map(|row| row.reverse_bits()));
    }
    // CP437 HUD icons in the control range (heart, diamond, spade).
    for (code, glyph) in [
        (0x03, [0x6C, 0xFE, 0xFE, 0xFE, 0x7C, 0x38, 0x10, 0x00]),
        (0x04, [0x10, 0x38, 0x7C, 0xFE, 0x7C, 0x38, 0x10, 0x00]),
        (0x06, [0x10, 0x38, 0x7C, 0xFE, 0xFE, 0x7C, 0x10, 0x38u8]),
    ] {
        rom[code * 8..code * 8 + 8].copy_from_slice(&glyph);
    }
    std::fs::write(out, &rom)?;
    println!("wrote {} bytes to {}", rom.len(), out.display());
    Ok(())
}

/// Render a level from the player start (optionally turned and moved) through the game's own
/// renderer, without a window. Useful for quick visual checks and golden images.
fn render_view(dir: &Path, level: &str, out: &Path, rest: &[&str]) -> AnyResult {
    let (w, h) = match rest.first() {
        Some(size) => {
            let (w, h) = size.split_once('x').ok_or("size must be WxH")?;
            (w.parse()?, h.parse()?)
        }
        None => (godd_game::CLASSIC_WIDTH, godd_game::CLASSIC_HEIGHT),
    };
    let turn: f32 = rest.get(1).map_or(Ok(0.0), |v| v.parse())?;
    let forward: f32 = rest.get(2).map_or(Ok(0.0), |v| v.parse())?;
    let vfs = godd_game::Vfs::from_dir(dir)?;
    let mut game = godd_game::Game::new(&vfs, &godd_game::Options { start_level: Some(level.to_string()), save: None })?;
    // Let objects settle for half a second (start-pad tool pickups etc.) before moving.
    let idle = godd_game::Input::default();
    for _ in 0..30 {
        game.tick(&vfs, &idle);
    }
    game.debug_move_player(turn.to_radians(), forward);
    let true_color = rest.get(3) == Some(&"truecolor");
    game.settings.true_color = true_color;
    let mut frame = godd_game::Frame::new(w, h);
    frame.set_true_color(true_color);
    // Render repeatedly so the HUD's rolling counters reach their values.
    for _ in 0..150 {
        game.render(&mut frame, 1.0);
    }
    let mut rgba = Vec::new();
    frame.to_rgba(game.palette(), &mut rgba);
    let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(out)?), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;
    Ok(())
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect()
}

fn write_png(path: &Path, width: u32, height: u32, indices: &[u8], pal: &Palette) -> AnyResult {
    let rgb: Vec<u8> = indices.iter().flat_map(|&i| pal.rgb[i as usize]).collect();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;
    Ok(())
}
