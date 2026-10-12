//! Real maps: the BSP shipped with Exploding Lips (in original/), and Quake's and Hexen II's
//! maps when they are installed (Steam library). Each test skips when its data is missing.

use engine_scene::module::{GameModule, ModuleInput};
use quake_bsp::viewer::{BspViewer, open};
use std::path::{Path, PathBuf};

fn steam(rel: &str) -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("HOME")?).join(".local/share/Steam/steamapps/common").join(rel);
    p.exists().then_some(p)
}

fn check(viewer: &mut BspViewer, min_faces: usize) {
    let b = viewer.bsp();
    assert!(b.world_faces.len() >= min_faces, "{} world faces", b.world_faces.len());
    assert!(b.textures.iter().all(|t| t.pixels.len() == t.width * t.height));
    assert!(b.faces[b.world_faces.clone()].iter().all(|f| f.points.len() >= 3), "every face is a polygon");
    let mut ui = engine_core::Frame::new(64, 40);
    let v = viewer.view(&mut ui, 1.0).expect("a view");
    assert!(v.scene.vertices.len() >= min_faces * 3);
    assert!(v.scene.vertices.iter().all(|p| p.pos.iter().all(|c| c.is_finite()) && (0.0..=15.0).contains(&p.light)));
    // Flying forward moves the camera.
    let before = v.camera.pos;
    for _ in 0..30 {
        viewer.tick(&ModuleInput { forward: 1.0, ..Default::default() });
    }
    let after = viewer.view(&mut ui, 1.0).unwrap().camera.pos;
    let moved = ((after[0] - before[0]).powi(2) + (after[1] - before[1]).powi(2) + (after[2] - before[2]).powi(2)).sqrt();
    assert!((moved - 160.0).abs() < 1.0, "half a second at 320 units/s: {moved}");
}

#[test]
fn exploding_lips_bsp() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    if !dir.join("LIPS00C.BSP").exists() {
        return;
    }
    let mut v = open(&dir.join("LIPS00C.BSP"), "lips00c", Some(&dir.join("PALETTE.PAL"))).unwrap();
    assert!(v.bsp().entities.contains("worldspawn"));
    check(&mut v, 1000);
}

#[test]
fn hexen2_demo_maps() {
    let Some(pak) = steam("Hexen 2/data1/pak0.pak") else { return };
    for map in ["demo1", "demo2", "demo3"] {
        let mut v = open(&pak, map, None).unwrap();
        check(&mut v, 1000);
    }
}

#[test]
fn quake_e1m1() {
    let Some(pak) = steam("Quake/rerelease/id1/pak0.pak") else { return };
    let mut v = open(&pak, "e1m1", None).unwrap();
    check(&mut v, 3000);
    let (_, yaw) = v.bsp().player_start();
    assert!((0.0..360.0).contains(&yaw));
}
