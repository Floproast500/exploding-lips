//! The GPU renderer reproduces the software renderer: the same views (HUD and weapon included)
//! rendered both ways differ by little more than polygon-edge rasterization. Skips without the
//! game data or a GPU adapter.

use std::path::Path;

fn rmse(a: &[u8], b: &[u8]) -> f64 {
    let sum: f64 = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum();
    (sum / a.len() as f64).sqrt() / 255.0
}

#[test]
fn gpu_matches_software() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    if !dir.join("LIPS00.XRB").exists() || engine_gpu::headless().is_none() {
        eprintln!("skipped: needs original/ and a GPU adapter");
        return;
    }
    // Stand in the middle of large rooms spread over each level and look three ways: rooms,
    // corridors, stairs, masked walls and objects at many distances.
    let mut views: Vec<(&str, Vec<String>)> = Vec::new();
    for level in ["LIPS00", "LIPS01", "LIPS03", "LIPS05", "LIPS07", "LIPS09", "LIPS11", "LIPS13", "LIPS16"] {
        for r in lipstool::viewpoints(&dir, level, 4).unwrap() {
            for turn in ["15", "135", "255"] {
                views.push((level, vec!["480x300".into(), turn.into(), format!("r{r}")]));
            }
        }
    }
    if cfg!(debug_assertions) {
        // Unoptimized builds render slowly: check every fourth view.
        views = views.into_iter().step_by(4).collect();
    }
    views.push(("LIPS00", ["640x400", "45", "100", "truecolor"].map(String::from).to_vec()));
    views.push(("LIPS09", ["1280x540", "200", "0"].map(String::from).to_vec()));
    let mut worst = (0.0, String::new());
    let mut total = 0.0;
    for (level, args) in &views {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (Ok((_, _, sw)), Ok((_, _, gpu))) = (lipstool::render_software(&dir, level, &args), lipstool::render_gpu(&dir, level, &args)) else {
            panic!("{level} {args:?} failed to render");
        };
        let e = rmse(&sw, &gpu);
        total += e;
        println!("{level} {args:?}: RMSE {e:.4}");
        if e > worst.0 {
            worst = (e, format!("{level} {args:?}"));
        }
    }
    println!("{} views, mean {:.4}, worst {:.4} at {}", views.len(), total / views.len() as f64, worst.0, worst.1);
    assert!(views.len() > if cfg!(debug_assertions) { 20 } else { 60 }, "enough viewpoints");
    assert!(worst.0 < 0.05, "worst view {} RMSE {:.4}", worst.1, worst.0);
}

/// HD textures keep the original texture coordinates: replacing every image by a 4x
/// nearest-neighbour copy (as a pack) changes the modern render only by filtering.
#[test]
fn hd_textures_keep_texture_coordinates() {
    use engine_scene::scene::{HdTexture, RgbaImage};
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    if !dir.join("LIPS00.XRB").exists() || engine_gpu::headless().is_none() {
        eprintln!("skipped: needs original/ and a GPU adapter");
        return;
    }
    let upscaled = |game: &godd_game::Game| {
        let pal = *game.palette();
        Some(
            game.world_images()
                .iter()
                .map(|img| {
                    let (w, h) = (img.width * 4, img.height * 4);
                    let pixels = (0..h)
                        .flat_map(|y| (0..w).map(move |x| (x, y)))
                        .flat_map(|(x, y)| {
                            let i = img.pixels[(y / 4) * img.width + x / 4];
                            let [r, g, b] = pal[i as usize];
                            [r, g, b, if i == 0 { 0 } else { 255 }]
                        })
                        .collect();
                    Some(HdTexture { albedo: RgbaImage { width: w, height: h, pixels }, normal: None })
                })
                .collect(),
        )
    };
    for (level, region) in [("LIPS00", "r126"), ("LIPS09", "r93")] {
        let args = ["640x400", "135", region, "-", "0", "modern"];
        let (_, _, plain) = lipstool::render_gpu(&dir, level, &args).unwrap();
        let (_, _, hd) = lipstool::render_gpu_with(&dir, level, &args, upscaled).unwrap();
        let e = rmse(&plain, &hd);
        println!("{level}: HD vs indexed RMSE {e:.4}");
        assert!(e < 0.04, "{level}: HD textures moved or stretched (RMSE {e:.4})");
    }
}

/// Render scale, FXAA and every MSAA level change only edges and filtering: the view stays
/// the same picture as the default modern settings.
#[test]
fn quality_settings_keep_the_picture() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    if !dir.join("LIPS00.XRB").exists() || engine_gpu::headless().is_none() {
        eprintln!("skipped: needs original/ and a GPU adapter");
        return;
    }
    let base = ["640x400", "135", "r126", "-", "0", "modern"];
    let (_, _, reference) = lipstool::render_gpu(&dir, "LIPS00", &base).unwrap();
    for extra in [&["set:r_fxaa=1"][..], &["set:r_scale=0.5"], &["set:r_scale=2"], &["set:r_msaa=off"], &["set:r_msaa=8x"], &["set:r_msaa=2x", "set:r_fxaa=1"]] {
        let args: Vec<&str> = base.iter().chain(extra).copied().collect();
        let (_, _, img) = lipstool::render_gpu(&dir, "LIPS00", &args).unwrap();
        let e = rmse(&reference, &img);
        println!("{extra:?}: RMSE {e:.4}");
        assert!(e < 0.05, "{extra:?} changed the picture (RMSE {e:.4})");
        assert!(img.iter().any(|&v| v > 40), "{extra:?}: blank image");
    }
}
