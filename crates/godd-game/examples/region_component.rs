//! The regions reachable from one through non-solid portal lines (door lines count), and the
//! lines that leave that set (debug aid): `cargo run --example region_component LIPS07 97`
use godd_game::{Game, Options, Vfs};
use std::collections::VecDeque;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let from: usize = args[2].parse().unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let game = Game::new(&vfs, &Options { start_level: Some(args[1].clone()), save: None }).unwrap();
    let s = game.session().unwrap();
    let w = &s.level.world;
    let doors: Vec<usize> = s.effects.doors().iter().map(|d| d.0).collect();
    let mut seen = vec![false; w.regions.len()];
    seen[from] = true;
    let mut queue = VecDeque::from([from]);
    let mut exits = Vec::new();
    while let Some(r) = queue.pop_front() {
        for &wi in &w.regions[r].walls {
            let x = &w.walls[wi];
            let Some(b) = x.back.filter(|&b| b != x.front) else { continue };
            let o = if x.front == r { b } else { x.front };
            let door = doors.contains(&b) || doors.contains(&x.front);
            let ((ax, ay), (bx, by)) = (w.vertices[x.a], w.vertices[x.b]);
            let len = (bx - ax).hypot(by - ay).max(1e-3);
            let (mx, my, nx, ny) = ((ax + bx) / 2.0, (ay + by) / 2.0, -(by - ay) / len * 2.0, (bx - ax) / len * 2.0);
            let fits = (w.point_in_region(mx + nx, my + ny, x.front) && w.point_in_region(mx - nx, my - ny, b)) || (w.point_in_region(mx + nx, my + ny, b) && w.point_in_region(mx - nx, my - ny, x.front));
            let stray = !door && !fits && std::env::var("STRAY").is_err();
            if stray && !seen[o] {
                exits.push(format!("wall {wi} r{r}->r{o} (stray)"));
            }
            let open = (!x.blocking || door) && !stray;
            if !open {
                if !seen[o] {
                    exits.push(format!("wall {wi} r{r}->r{o} (solid)"));
                }
                continue;
            }
            if !seen[o] {
                seen[o] = true;
                queue.push_back(o);
            }
        }
    }
    let set: Vec<usize> = (0..w.regions.len()).filter(|&r| seen[r]).collect();
    println!("{} regions: {:?}", set.len(), set);
    for e in exits {
        println!("{e}");
    }
}
