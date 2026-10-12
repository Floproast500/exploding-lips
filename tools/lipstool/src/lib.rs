//! Library side of `lipstool`: headless software and GPU renders of a level view, shared by
//! the CLI and the GPU parity test.

use std::error::Error;
use std::path::Path;

pub type AnyResult<T = ()> = Result<T, Box<dyn Error>>;

/// Game started on `level`, settled for half a second and moved like `render`.
fn view_game(dir: &Path, level: &str, rest: &[&str]) -> AnyResult<(godd_game::Vfs, godd_game::Game, usize, usize)> {
    let (w, h) = match rest.first() {
        Some(size) => {
            let (w, h) = size.split_once('x').ok_or("size must be WxH")?;
            (w.parse()?, h.parse()?)
        }
        None => (godd_game::CLASSIC_WIDTH, godd_game::CLASSIC_HEIGHT),
    };
    let turn: f32 = rest.get(1).map_or(Ok(0.0), |v| v.parse())?;
    // `forward` may instead be `sN` (stand at sprite N) or `rN` (stand at region N's centre).
    let at_sprite: Option<usize> = rest.get(2).and_then(|v| v.strip_prefix('s')).map(str::parse).transpose()?;
    let at_region: Option<usize> = rest.get(2).and_then(|v| v.strip_prefix('r')).map(str::parse).transpose()?;
    let forward: f32 = if at_sprite.is_some() || at_region.is_some() { 0.0 } else { rest.get(2).map_or(Ok(0.0), |v| v.parse())? };
    let vfs = godd_game::Vfs::from_dir(dir)?;
    let mut game = godd_game::Game::new(&vfs, &godd_game::Options { start_level: Some(level.to_string()), save: None })?;
    let idle = godd_game::Input::default();
    for _ in 0..30 {
        game.tick(&vfs, &idle);
    }
    match at_sprite {
        Some(i) => {
            let s = game.session_mut().ok_or("no level")?;
            let sp = s.level.world.sprites.get(i).ok_or("no such sprite")?.clone();
            let floor = s.level.world.regions.get(sp.region).map_or(sp.z, |r| r.floor);
            (s.player.x, s.player.y, s.player.z, s.player.region) = (sp.x, sp.y, floor, sp.region);
            s.player.angle = turn.to_radians();
            s.previous = s.player;
        }
        None => match at_region {
            Some(r) => {
                let s = game.session_mut().ok_or("no level")?;
                let (x, y) = region_centre(&s.level.world, r).ok_or("region has no area")?;
                let floor = s.level.world.regions[r].floor;
                (s.player.x, s.player.y, s.player.z, s.player.region) = (x, y, floor, r);
                s.player.angle = turn.to_radians();
                s.previous = s.player;
            }
            None => game.debug_move_player(turn.to_radians(), forward),
        },
    }
    game.settings.true_color = rest.get(3) == Some(&"truecolor");
    // Optional 5th argument: look up/down (original look units, ±512), and "shear" to turn
    // off true-3D look in the GPU renderer.
    if let Some(look) = rest.get(4).and_then(|v| v.parse::<f32>().ok())
        && let Some(s) = game.session_mut() {
            s.player.look = look;
            s.previous = s.player;
        }
    if rest.contains(&"shear") {
        game.cvars.set_bool("r_freelook", false);
    }
    Ok((vfs, game, w, h))
}

/// Render the view of `lipstool render` through the GPU renderer; returns RGB.
pub fn render_gpu(dir: &Path, level: &str, rest: &[&str]) -> AnyResult<(usize, usize, Vec<u8>)> {
    let (_vfs, mut game, w, h) = view_game(dir, level, rest)?;
    let (device, queue) = engine_gpu::headless().ok_or("no GPU adapter")?;
    let mut renderer = engine_gpu::WorldRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let mut ui = godd_game::Frame::new(w, h);
    // Repeat so the HUD's rolling counters reach their values, as `render` does.
    let mut view = None;
    for _ in 0..150 {
        view = game.gpu_frame(&mut ui, 1.0);
    }
    let view = view.ok_or("no level running")?;
    renderer.set_palette(&device, &queue, game.palette(), game.shade_rows());
    renderer.set_images(&device, &queue, &game.world_images());
    let coverage = ui.coverage.clone().unwrap_or_default();
    let rgba = engine_gpu::render_offscreen(&device, &queue, &mut renderer, (w as u32, h as u32), &view.camera, &view.shading, &view.scene, Some((&ui.pixels, &coverage)));
    Ok((w, h, rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()))
}

/// Software render of the same view; returns RGB.
pub fn render_software(dir: &Path, level: &str, rest: &[&str]) -> AnyResult<(usize, usize, Vec<u8>)> {
    let (_vfs, mut game, w, h) = view_game(dir, level, rest)?;
    let mut frame = godd_game::Frame::new(w, h);
    frame.set_true_color(game.settings.true_color);
    for _ in 0..150 {
        game.render(&mut frame, 1.0);
    }
    let mut rgba = Vec::new();
    frame.to_rgba(game.palette(), &mut rgba);
    Ok((w, h, rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()))
}

pub fn save_rgb(out: &Path, w: usize, h: usize, rgb: &[u8]) -> AnyResult {
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(out)?), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgb)?;
    Ok(())
}


/// Area-weighted centre of a region's triangles.
fn region_centre(world: &godd_game::world::World, region: usize) -> Option<(f32, f32)> {
    let mesh = godd_game::scene::build(world).into_iter().nth(region)?;
    let (mut ax, mut ay, mut area) = (0.0f64, 0.0f64, 0.0f64);
    for t in &mesh.triangles {
        let p = t.map(|i| world.vertices[i as usize]);
        let a = ((p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1)) as f64 / 2.0;
        ax += a * (p[0].0 + p[1].0 + p[2].0) as f64 / 3.0;
        ay += a * (p[0].1 + p[1].1 + p[2].1) as f64 / 3.0;
        area += a;
    }
    (area > 0.0).then(|| ((ax / area) as f32, (ay / area) as f32))
}

/// Up to `n` good test viewpoints in a level: large regions whose centre lies inside them, at
/// least 64 units from every wall, with room between floor and ceiling.
pub fn viewpoints(dir: &Path, level: &str, n: usize) -> AnyResult<Vec<usize>> {
    let vfs = godd_game::Vfs::from_dir(dir)?;
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{level}.XRB"))?, vfs.read(&format!("{level}.XPG"))?)?;
    let w = &lvl.world;
    let mut good: Vec<usize> = (0..w.regions.len())
        .filter(|&r| {
            let reg = &w.regions[r];
            let ((x0, y0), (x1, y1)) = reg.bounds;
            if (x1 - x0) * (y1 - y0) < 60_000.0 || reg.ceiling - reg.floor < 80.0 {
                return false;
            }
            let Some((cx, cy)) = region_centre(w, r) else { return false };
            w.point_in_region(cx, cy, r)
                && reg.walls.iter().all(|&i| {
                    let wall = &w.walls[i];
                    let ((ax, ay), (bx, by)) = (w.vertices[wall.a], w.vertices[wall.b]);
                    let (dx, dy) = (bx - ax, by - ay);
                    let t = (((cx - ax) * dx + (cy - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
                    ((ax + t * dx - cx).powi(2) + (ay + t * dy - cy).powi(2)).sqrt() > 64.0
                })
        })
        .collect();
    let step = (good.len() / n.max(1)).max(1);
    good = good.into_iter().step_by(step).take(n).collect();
    Ok(good)
}
