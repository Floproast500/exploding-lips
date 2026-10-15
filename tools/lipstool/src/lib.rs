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
    // "at:X,Y,Z,ANGLE": put the player there (ANGLE in radians, as a quicksave has it).
    if let Some(at) = rest.iter().find_map(|a| a.strip_prefix("at:")) {
        let v: Vec<f32> = at.split(',').map(str::parse).collect::<Result<_, _>>()?;
        let [x, y, z, angle] = v[..] else { return Err("at:X,Y,Z,ANGLE".into()) };
        let s = game.session_mut().ok_or("no level")?;
        let region = s.level.world.region_at_z(x, y, Some(z + 1.0)).ok_or("no region there")?;
        (s.player.x, s.player.y, s.player.z, s.player.region, s.player.angle) = (x, y, z, region, angle);
        s.previous = s.player;
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
    // "boom": an explosion 160 units ahead (to see dynamic lights).
    if rest.contains(&"boom")
        && let Some(s) = game.session_mut() {
            let p = s.player;
            let (x, y) = (p.x + p.angle.cos() * 160.0, p.y + p.angle.sin() * 160.0);
            let region = s.level.world.region_at(x, y);
            s.objects.spawn_explosion((x, y, p.z + 8.0), 32.0, 48.0, 0.5, region);
        }
    // "map": walk around a little (to reveal), then show the automap.
    if rest.contains(&"map") {
        let mut walk = godd_game::Input::default();
        walk.set(godd_game::Action::Forward, true);
        for _ in 0..60 {
            game.tick(&vfs, &walk);
        }
        let mut open = godd_game::Input::default();
        open.set(godd_game::Action::Map, true);
        game.tick(&vfs, &open);
    }
    // GPU renders use the original lighting (parity with `render`) unless asked for "modern".
    let _ = game.cvars.set("r_lighting", if rest.contains(&"modern") { "modern" } else { "classic" });
    // Comparisons with the original use its HUD; "modern" renders show the modern one.
    let _ = game.cvars.set("hud_style", if rest.contains(&"modern") { "modern" } else { "classic" });
    // "tag:N": fire tag N, then let a second pass (tag:132 in LIPS00 enters its BSP world).
    for tag in rest.iter().filter_map(|a| a.strip_prefix("tag:")?.parse::<u32>().ok()) {
        if let Some(s) = game.session_mut() {
            s.fire_tag(tag);
        }
        for _ in 0..60 {
            game.tick(&vfs, &idle);
        }
    }
    // "set:NAME=VALUE": any cvar (r_msaa=8x, r_scale=1.5, r_fxaa=1 ...).
    for (name, value) in rest.iter().filter_map(|a| a.strip_prefix("set:")?.split_once('=')) {
        game.cvars.set(name, value)?;
    }
    Ok((vfs, game, w, h))
}

/// Render the view of `lipstool render` through the GPU renderer; returns RGB.
pub fn render_gpu(dir: &Path, level: &str, rest: &[&str]) -> AnyResult<(usize, usize, Vec<u8>)> {
    render_gpu_with(dir, level, rest, |_| None)
}

/// As [`render_gpu`], with HD textures supplied by `hd` (overrides a `pack=` argument).
pub fn render_gpu_with(
    dir: &Path,
    level: &str,
    rest: &[&str],
    hd: impl Fn(&godd_game::Game) -> Option<Vec<Option<engine_scene::scene::HdTexture>>>,
) -> AnyResult<(usize, usize, Vec<u8>)> {
    let (_vfs, mut game, w, h) = view_game(dir, level, rest)?;
    let (adapter, device, queue) = engine_gpu::headless_adapter().ok_or("no GPU adapter")?;
    let mut renderer = engine_gpu::WorldRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.use_adapter(&device, &queue, &adapter);
    let (uw, uh) = game.ui_layer_size(w, h);
    let mut ui = godd_game::Frame::new(uw, uh);
    // Repeat so the HUD's rolling counters reach their values, as `render` does.
    let mut view = None;
    for _ in 0..150 {
        view = game.gpu_frame(&mut ui, 1.0);
    }
    let view = view.ok_or("no level running")?;
    renderer.set_palette(&device, &queue, game.palette(), game.shade_rows());
    renderer.set_images(&device, &queue, &game.world_images());
    if let Some(textures) = hd(&game) {
        renderer.set_hd(&device, &queue, &textures);
    } else if let Some(dir) = rest.iter().find_map(|a| a.strip_prefix("pack=")) {
        renderer.set_hd(&device, &queue, &hd_textures(&game, Path::new(dir))?);
    }
    let coverage = ui.coverage.clone().unwrap_or_default();
    let rgba = engine_gpu::render_offscreen(&device, &queue, &mut renderer, (w as u32, h as u32), &view.camera, &view.shading, &view.scene, Some(((ui.width, ui.height), &ui.pixels, &coverage, ui.ui_list.as_deref().unwrap_or_default())));
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

/// Average CPU time (milliseconds) of building one GPU frame: portal walk, scene vertices, the
/// UI layer (weapon, HUD) and live camera views.
pub fn bench_scene(dir: &Path, level: &str, rest: &[&str], frames: u32) -> AnyResult<f64> {
    let (_vfs, mut game, w, h) = view_game(dir, level, rest)?;
    let (uw, uh) = game.ui_layer_size(w, h);
    let mut ui = godd_game::Frame::new(uw, uh);
    for _ in 0..5 {
        game.gpu_frame(&mut ui, 1.0);
    }
    let start = std::time::Instant::now();
    for _ in 0..frames {
        game.gpu_frame(&mut ui, 1.0).ok_or("no level running")?;
    }
    Ok(start.elapsed().as_secs_f64() * 1000.0 / frames as f64)
}

/// HD textures for the current level from a pack directory.
pub fn hd_textures(game: &godd_game::Game, dir: &Path) -> AnyResult<Vec<Option<engine_scene::scene::HdTexture>>> {
    Ok(game.hd_textures(&engine_core::pack::LoadedPack::open(dir)?))
}

/// Time `frames` GPU frames of the `render-gpu` view (HUD included) with timestamp queries
/// (plus wall time of submitting and waiting); returns a one-line report.
pub fn bench_gpu(dir: &Path, level: &str, rest: &[&str], frames: usize) -> AnyResult<String> {
    let (_vfs, mut game, w, h) = view_game(dir, level, rest)?;
    let (adapter, device, queue) = engine_gpu::headless_adapter().ok_or("no GPU adapter")?;
    let mut renderer = engine_gpu::WorldRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.use_adapter(&device, &queue, &adapter);
    let (uw, uh) = game.ui_layer_size(w, h);
    let mut ui = godd_game::Frame::new(uw, uh);
    let view = game.gpu_frame(&mut ui, 1.0).ok_or("no level running")?;
    renderer.set_palette(&device, &queue, game.palette(), game.shade_rows());
    renderer.set_images(&device, &queue, &game.world_images());
    if let Some(dir) = rest.iter().find_map(|a| a.strip_prefix("pack=")) {
        renderer.set_hd(&device, &queue, &hd_textures(&game, Path::new(dir))?);
    }
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench"),
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = target.create_view(&Default::default());
    let coverage = ui.coverage.clone().unwrap_or_default();
    let list = ui.ui_list.clone().unwrap_or_default();
    let vp = [0.0, 0.0, w as f32, h as f32];
    let (mut gpu, mut wall) = (Vec::new(), Vec::new());
    for i in 0..frames + 10 {
        let started = std::time::Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&device, &queue, &mut encoder, &target, (w as u32, h as u32), vp, &view.camera, &view.shading, &view.scene);
        renderer.draw_ui(&device, &queue, &mut encoder, &target, vp, (ui.width, ui.height), &ui.pixels, &coverage, &list, true);
        queue.submit([encoder.finish()]);
        renderer.submitted();
        device.poll(wgpu::PollType::wait_indefinitely())?;
        let g = renderer.gpu_ms(&device);
        if i >= 10 {
            wall.push(started.elapsed().as_secs_f32() * 1000.0);
            gpu.extend(g);
        }
    }
    let stats = |v: &[f32]| {
        let avg = v.iter().sum::<f32>() / v.len().max(1) as f32;
        let max = v.iter().copied().fold(0.0, f32::max);
        format!("avg {avg:.2} ms, max {max:.2} ms")
    };
    let m = view.shading.modern;
    Ok(format!(
        "{w}x{h} {} msaa {} scale {:.2}{}: gpu {} | submit+wait {}",
        adapter.get_info().name,
        m.map_or(1, |m| m.msaa),
        m.map_or(1.0, |m| m.render_scale),
        if m.is_some_and(|m| m.fxaa) { " fxaa" } else { "" },
        if gpu.is_empty() { "n/a (no timestamp queries)".to_string() } else { stats(&gpu) },
        stats(&wall)
    ))
}

/// Render a game module's current view off screen (UI layer included); returns RGB.
pub fn render_module(module: &mut dyn engine_scene::module::GameModule, (w, h): (usize, usize)) -> AnyResult<Vec<u8>> {
    let (device, queue) = engine_gpu::headless().ok_or("no GPU adapter")?;
    let mut renderer = engine_gpu::WorldRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let mut ui = godd_game::Frame::new(w, h);
    let view = module.view(&mut ui, 1.0).ok_or("nothing to show")?;
    renderer.set_palette(&device, &queue, module.palette(), module.shade_rows());
    renderer.set_images(&device, &queue, &module.images());
    let coverage = ui.coverage.clone().unwrap_or_default();
    let rgba = engine_gpu::render_offscreen(&device, &queue, &mut renderer, (w as u32, h as u32), &view.camera, &view.shading, &view.scene, Some(((ui.width, ui.height), &ui.pixels, &coverage, ui.ui_list.as_deref().unwrap_or_default())));
    Ok(rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect())
}

/// A BSP viewer for `source` (see [`quake_bsp::viewer::open`]).
pub fn bsp_viewer(source: &Path, map: &str, palette: Option<&Path>) -> AnyResult<quake_bsp::viewer::BspViewer> {
    Ok(quake_bsp::viewer::open(source, map, palette)?)
}
