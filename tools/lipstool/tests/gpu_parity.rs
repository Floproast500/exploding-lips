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
    assert!(views.len() > 60, "enough viewpoints");
    assert!(worst.0 < 0.05, "worst view {} RMSE {:.4}", worst.1, worst.0);
}
