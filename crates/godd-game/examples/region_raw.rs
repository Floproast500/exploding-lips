//! A region's raw XRB record bytes and the per-region physics block (debug aid):
//! `cargo run --example region_raw LIPS00 303`
use godd_formats::xrb::{RecordExt, Section};
use godd_game::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r: usize = args[2].parse().unwrap();
    let vfs = Vfs::from_dir(std::path::Path::new("original")).unwrap();
    let lvl = godd_game::level::Level::load(vfs.read(&format!("{}.XRB", args[1])).unwrap(), vfs.read(&format!("{}.XPG", args[1])).unwrap()).unwrap();
    let rec = &lvl.orb.records(Section::Regions)[r];
    println!("len {} byte0 {:#04x} byte1 {:#04x} byte2 {:#04x} byte3 {:#04x}", rec.len(), rec[0], rec[1], rec[2], rec[3]);
    println!("+0x90..: fx {} fy {} g {}", rec.le_i32(0x90), rec.le_i32(0x94), rec.le_i32(0x98));
}
