//! An object's raw XRB movement fields (debug aid): `cargo run --example obj_raw LIPS00 670`
use godd_formats::xrb::{RecordExt, Section};
use godd_game::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let i: usize = args[2].parse().unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{}.XRB", args[1])).unwrap(), vfs.read(&format!("{}.XPG", args[1])).unwrap()).unwrap();
    let rec = &lvl.orb.records(Section::Objects)[i];
    let f = |o: usize| rec.le_i32(o) as f64 / 65536.0;
    println!("height {} {} radius {} speeds msx {} msy {} msz {} mst {} accel {} {} {} {}", f(0x34), f(0x38), f(0x3c), f(0x7c), f(0x80), f(0x84), f(0x88), f(0x8c), f(0x90), f(0x94), f(0x98));
    println!("flags {:#010x} {:#010x}", rec.le_i32(0), rec.le_i32(4));
}
