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

/// Collision from the player start: the player box (hull 1) is in open space, falls onto a
/// floor, and every horizontal sweep stops at a wall facing it, never inside solid.
fn check_collision(bsp: &quake_bsp::Bsp) {
    use quake_bsp::collision::SOLID;
    let h = &bsp.hulls;
    assert!(h.hull_count() >= 3, "{} hulls", h.hull_count());
    let (start, _) = bsp.player_start();
    // Quake starts stand on the floor with the origin 24 above it; nudge up in case.
    let origin = [start[0], start[1], start[2] + 1.0];
    assert_ne!(h.contents(1, origin), SOLID, "player box at the start is free");
    let down = h.trace(1, origin, [origin[0], origin[1], origin[2] - 4096.0]);
    assert!(down.fraction < 1.0 && down.normal[2] > 0.7, "lands on a floor: {down:?}");
    let mut walls = 0;
    for k in 0..16 {
        let a = k as f32 / 16.0 * std::f32::consts::TAU;
        let (dx, dy) = (a.cos(), a.sin());
        let end = [origin[0] + dx * 8192.0, origin[1] + dy * 8192.0, origin[2]];
        let t = h.trace(1, origin, end);
        assert!(!t.start_solid);
        assert_ne!(h.contents(1, t.end), SOLID, "stops outside solid: {t:?}");
        if t.fraction < 1.0 {
            walls += 1;
            assert!(t.normal[0] * dx + t.normal[1] * dy < 0.0, "the wall faces the mover: {t:?}");
            // Just past the stop is solid.
            let past = [t.end[0] + dx * 2.0, t.end[1] + dy * 2.0, t.end[2]];
            let beyond = h.trace(1, t.end, past);
            assert!(beyond.fraction < 1.0, "blocked right after the stop");
        }
    }
    assert!(walls >= 14, "{walls} of 16 directions hit walls");
}

#[test]
fn collision_on_real_maps() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
    if dir.join("LIPS00C.BSP").exists() {
        let v = open(&dir.join("LIPS00C.BSP"), "lips00c", Some(&dir.join("PALETTE.PAL"))).unwrap();
        check_collision(v.bsp());
    }
    if let Some(pak) = steam("Quake/rerelease/id1/pak0.pak") {
        check_collision(open(&pak, "e1m1", None).unwrap().bsp());
    }
    if let Some(pak) = steam("Hexen 2/data1/pak0.pak") {
        check_collision(open(&pak, "demo1", None).unwrap().bsp());
    }
}
