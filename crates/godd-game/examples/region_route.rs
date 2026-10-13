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
    let mut prev = vec![usize::MAX; w.regions.len()];
    prev[from] = from;
    let mut queue = VecDeque::from([from]);
    while let Some(r) = queue.pop_front() {
        for &wi in &w.regions[r].walls {
            let x = &w.walls[wi];
            let Some(b) = x.back.filter(|&b| b != x.front) else { continue };
            let o = if x.front == r { b } else { x.front };
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
