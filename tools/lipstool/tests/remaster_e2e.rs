//! The whole remaster at once, end to end: one LIPS00 session with every remaster feature on
//! (modern HDR lighting, dynamic lights, shadows, bloom, MSAA, FXAA, the HD texture pack when
//! one is built, the HD HUD, guides), played through walking, firing, the automap, the BSP
//! catacombs and back, and a save and load, rendered on the GPU along the way. Each step checks
//! the picture, the sound events and the game state. Skips without the game data or a GPU.

use godd_game::{Action, AudioEvent, Game, Input};
use std::path::Path;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: engine_gpu::WorldRenderer,
    epoch: Option<u64>,
    pack: Option<std::path::PathBuf>,
}

impl Gpu {
    /// Render the current view (3D + UI layer) at w×h; RGB bytes.
    fn shot(&mut self, game: &mut Game, (w, h): (usize, usize)) -> Vec<u8> {
        let (uw, uh) = game.ui_layer_size(w, h);
        let mut ui = godd_game::Frame::new(uw, uh);
        let view = game.gpu_frame(&mut ui, 1.0).expect("a GPU view");
        if self.epoch != Some(view.epoch) {
            // A level (or the BSP world) was loaded: its images, and the HD pack for them.
            self.epoch = Some(view.epoch);
            self.renderer.set_palette(&self.device, &self.queue, game.palette(), game.shade_rows());
            self.renderer.set_images(&self.device, &self.queue, &game.world_images());
            if let Some(pack) = &self.pack {
                let hd = lipstool::hd_textures(game, pack).expect("HD pack");
                self.renderer.set_hd(&self.device, &self.queue, &hd);
            }
        }
        let coverage = ui.coverage.clone().unwrap_or_default();
        let rgba = engine_gpu::render_offscreen(
            &self.device,
            &self.queue,
            &mut self.renderer,
            (w as u32, h as u32),
            &view.camera,
            &view.shading,
            &view.scene,
            Some(((ui.width, ui.height), &ui.pixels, &coverage, ui.ui_list.as_deref().unwrap_or_default())),
        );
        rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()
    }
}

/// A real picture: mostly not black, and many distinct colours (not one flat fill).
fn assert_picture(rgb: &[u8], what: &str) {
    if let Ok(dir) = std::env::var("E2E_SHOTS") {
        let w = if rgb.len() == 3440 * 1440 * 3 { 3440 } else { 1280 };
        let _ = lipstool::save_rgb(&Path::new(&dir).join(format!("{}.png", what.replace(' ', "_"))), w, rgb.len() / 3 / w, rgb);
    }
    let lit = rgb.chunks(3).filter(|p| p.iter().any(|&v| v > 24)).count();
    let pixels = rgb.len() / 3;
    let mut colours: Vec<u32> = rgb.chunks(3).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
    colours.sort_unstable();
    colours.dedup();
    // (Lips is dark: a corner of a dim corridor is a fair picture.)
    assert!(lit * 50 > pixels, "{what}: black ({lit} of {pixels} lit)");
    assert!(colours.len() > 500, "{what}: only {} colours", colours.len());
}

fn diff(a: &[u8], b: &[u8]) -> f64 {
    let sum: f64 = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum();
    (sum / a.len() as f64).sqrt() / 255.0
}

fn hold(game: &mut Game, vfs: &godd_game::Vfs, actions: &[Action], ticks: usize) {
    let mut input = Input::default();
    for &a in actions {
        input.set(a, true);
    }
    for _ in 0..ticks {
        game.tick(vfs, &input);
    }
}

#[test]
fn remaster_end_to_end() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    let Some((adapter, device, queue)) = engine_gpu::headless_adapter().filter(|_| dir.join("LIPS00.XRB").exists()) else {
        eprintln!("skipped: needs original/ and a GPU adapter");
        return;
    };
    let vfs = godd_game::Vfs::from_dir(&dir).unwrap();
    let mut game = Game::new(&vfs, &godd_game::Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
    for line in [
        "set r_lighting modern",
        "set r_dynlights 1",
        "set r_shadows 1",
        "set r_bloom 1",
        "set r_msaa 4x",
        "set r_fxaa 1",
        "set r_truecolor 1",
        "set hud_hd 1",
        "set hud_style modern",
        "set hud_guides 1",
        "set g_fixes 1",
    ] {
        assert!(game.exec_config(line).is_empty(), "{line}");
    }
    let mut renderer = engine_gpu::WorldRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.use_adapter(&device, &queue, &adapter);
    // The HD pack built from the player's own files, when there is one (never committed).
    let pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/lips-hd");
    let pack = pack.join("pack.txt").is_file().then_some(pack);
    println!("HD pack: {}", pack.as_ref().map_or("none (indexed textures)".into(), |p| p.display().to_string()));
    let mut gpu = Gpu { device, queue, renderer, epoch: None, pack };
    let size = (1280, 720);
    hold(&mut game, &vfs, &[], 30);
    let _ = game.drain_audio();

    // 1. The start: the remastered view.
    let start = gpu.shot(&mut game, size);
    assert_picture(&start, "start");

    // 2. Walking: the view changes, footsteps are positional, the listener follows.
    let before = game.session().unwrap().player;
    hold(&mut game, &vfs, &[Action::Forward], 90);
    let after = game.session().unwrap().player;
    assert!((after.x - before.x).hypot(after.y - before.y) > 50.0, "walked");
    let walked = gpu.shot(&mut game, size);
    assert_picture(&walked, "walking");
    assert!(diff(&start, &walked) > 0.02, "the view moved");
    let audio = game.drain_audio();
    assert!(audio.iter().any(|e| matches!(e, AudioEvent::Listener { .. })), "listener updates");

    // 3. Firing: the tool plays its use animation, and the picture shows it.
    let calm = gpu.shot(&mut game, size);
    let idle = {
        let s = game.session().unwrap();
        s.objects.weapon_pic(&s.level)
    };
    let mut animated = false;
    for k in 0..30 {
        hold(&mut game, &vfs, if k < 2 { &[Action::Use][..] } else { &[] }, 1);
        let s = game.session().unwrap();
        animated |= s.objects.weapon_pic(&s.level) != idle;
    }
    assert!(animated, "the tool animates when used");
    let firing = gpu.shot(&mut game, size);
    assert_picture(&firing, "firing");
    assert!(diff(&calm, &firing) > 0.001, "the shot shows");
    hold(&mut game, &vfs, &[], 120);

    // 4. The automap (HD UI lines over the view) and back.
    hold(&mut game, &vfs, &[Action::Map], 1);
    hold(&mut game, &vfs, &[], 1);
    let map = gpu.shot(&mut game, size);
    assert_picture(&map, "automap");
    assert!(diff(&walked, &map) > 0.05, "the automap covers the view");
    hold(&mut game, &vfs, &[Action::Map], 1);
    hold(&mut game, &vfs, &[], 1);

    // 5. Save, wander off, load: back where we were.
    let saved = game.session().unwrap().player;
    game.save_slot(1);
    let files = game.take_slots();
    assert_eq!(files.len(), 1, "one save slot written");
    hold(&mut game, &vfs, &[Action::Back], 60);
    game.set_slot(1, &files[0].1);
    game.load_slot(&vfs, 1);
    hold(&mut game, &vfs, &[], 2);
    let p = game.session().unwrap().player;
    assert!((p.x - saved.x).abs() < 1.0 && (p.y - saved.y).abs() < 1.0, "loaded back to ({}, {}): ({}, {})", saved.x, saved.y, p.x, p.y);
    assert_picture(&gpu.shot(&mut game, size), "after loading");

    // 6. The BSP catacombs (Quake-style world, its own palette and textures) and back.
    game.session_mut().unwrap().fire_tag(132);
    hold(&mut game, &vfs, &[], 30);
    assert!(game.session().unwrap().in_bsp, "in the catacombs");
    let catacombs = gpu.shot(&mut game, size);
    assert_picture(&catacombs, "catacombs");
    hold(&mut game, &vfs, &[Action::Forward], 60);
    assert_picture(&gpu.shot(&mut game, size), "walking the catacombs");
    game.session_mut().unwrap().bsp.as_mut().unwrap().player.pos = [-2384.0, -1120.0, 424.0];
    hold(&mut game, &vfs, &[], 120);
    assert!(!game.session().unwrap().in_bsp, "back from the catacombs");
    assert_picture(&gpu.shot(&mut game, size), "back on the coffin");

    // 7. Ultrawide at the end, as the player runs it.
    assert_picture(&gpu.shot(&mut game, (3440, 1440)), "3440x1440");
}
