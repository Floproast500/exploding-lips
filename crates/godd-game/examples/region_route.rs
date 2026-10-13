//! Region-level route between two regions at the start of a level (debug aid):
//! `cargo run --example region_route LIPS05 278 416`
use godd_game::{Game, Options, Vfs};
use std::collections::VecDeque;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (from, to): (usize, usize) = (args[2].parse().unwrap(), args[3].parse().unwrap());
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(args[1].clone()), save: None }).unwrap();
    let w = &game.session().unwrap().level.world;
    let doors: Vec<usize> = game.session().unwrap().effects.doors().iter().map(|d| d.0).collect();
    let mut prev = vec![usize::MAX; w.regions.len()];
    prev[from] = from;
    let mut queue = VecDeque::from([from]);
    while let Some(r) = queue.pop_front() {
        for &wi in &w.regions[r].walls {
            let x = &w.walls[wi];
            let Some(b) = x.back.filter(|&b| b != x.front && (std::env::var("ALL").is_ok() || !x.blocking || doors.contains(&b) || doors.contains(&x.front))) else { continue };
            let o = if x.front == r { b } else { x.front };
            // (Not through a stray portal line: its sides must lie in the regions it joins.)
            let ((ax, ay), (bx, by)) = (w.vertices[x.a], w.vertices[x.b]);
            let len = (bx - ax).hypot(by - ay).max(1e-3);
            let (mx, my, nx, ny) = ((ax + bx) / 2.0, (ay + by) / 2.0, -(by - ay) / len * 2.0, (bx - ax) / len * 2.0);
            let fits = |p: (f32, f32), q: (f32, f32)| (w.point_in_region(p.0, p.1, x.front) && w.point_in_region(q.0, q.1, b)) || (w.point_in_region(p.0, p.1, b) && w.point_in_region(q.0, q.1, x.front));
            if std::env::var("ALL").is_err() && !doors.contains(&b) && !doors.contains(&x.front) && !fits((mx + nx, my + ny), (mx - nx, my - ny)) {
                continue;
            }
            if prev[o] == usize::MAX {
                prev[o] = r;
                queue.push_back(o);
            }
        }
    }
    if prev[to] == usize::MAX {
        println!("no route");
        return;
    }
    let mut path = vec![to];
    while let Some(&r) = path.last() {
        if prev[r] == r {
            break;
        }
        path.push(prev[r]);
    }
    for r in path.into_iter().rev() {
        let reg = &w.regions[r];
        println!("r{r}: floor {} ceil {} bounds {:?}", reg.floor, reg.ceiling, reg.bounds);
    }
}
