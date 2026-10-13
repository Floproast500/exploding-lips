//! Getting the game: the full version of Exploding Lips is on the Internet Archive (public
//! domain mark), so Big Lips can fetch it itself, as OldUnreal's installers fetch Unreal: download
//! the zip, check it is exactly the known file, and unpack it into the game folder. Big Lips
//! ships none of it; this is the player's own download from the archive.

#[cfg(feature = "download")]
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The archive.org item and the one file Big Lips takes from it.
pub const ITEM_URL: &str = "https://archive.org/details/exploding-lips-full";
#[cfg_attr(not(feature = "download"), allow(dead_code))]
const ZIP_URL: &str = "https://archive.org/download/exploding-lips-full/elips_Exploding-Lips-Folder-circa-01-02-2000.zip";
#[cfg_attr(not(feature = "download"), allow(dead_code))]
const ZIP_SIZE: u64 = 42_703_994;
const ZIP_SHA256: &str = "6e9dac418fbdae91a445439c834230234d69bba4b0fc6c99b6436eec0c5c3c6a";

/// Where `--install` (and the first-run download) puts the game: `game` in the config folder.
pub fn default_dir() -> Option<PathBuf> {
    crate::config::dir().map(|d| d.join("game"))
}

/// Download, verify and unpack the full game into `dir`. `progress(done, total)` is called as
/// bytes arrive. Returns `dir` once it holds a playable game.
pub fn install(dir: &Path, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let part = dir.join(".download.zip.part");
    download(&part, &mut progress)?;
    let unpacked = unpack(&part, dir);
    let _ = std::fs::remove_file(&part);
    let files = unpacked?;
    if !crate::setup::is_game_dir(dir) {
        return Err(format!("the download unpacked {files} files but {} has no LIPS00.XRB", dir.display()));
    }
    let note = format!("Exploding Lips (full version), from {ITEM_URL}\n{ZIP_URL}\nsha256 {ZIP_SHA256}\n");
    let _ = std::fs::write(dir.join("SOURCE.txt"), note);
    Ok(dir.to_path_buf())
}

/// (Built without the `download` feature, as for a cross-check without the target's C SDK.)
#[cfg(not(feature = "download"))]
fn download(_to: &Path, _progress: &mut impl FnMut(u64, u64)) -> Result<(), String> {
    Err(format!("this build of Big Lips cannot download; get the game from {ITEM_URL} and start with --data <folder>"))
}

#[cfg(not(feature = "download"))]
fn unpack(_zip_path: &Path, _dir: &Path) -> Result<usize, String> {
    Err("this build of Big Lips cannot unpack zips".into())
}

#[cfg(feature = "download")]
fn download(to: &Path, progress: &mut impl FnMut(u64, u64)) -> Result<(), String> {
    use std::io::Read;
    let response = ureq::get(ZIP_URL).call().map_err(|e| format!("download failed: {e}"))?;
    let total = response.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(ZIP_SIZE);
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(to).map_err(|e| format!("cannot write {}: {e}", to.display()))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| format!("cannot write {}: {e}", to.display()))?;
        done += n as u64;
        progress(done, total);
    }
    let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if done != ZIP_SIZE || digest != ZIP_SHA256 {
        return Err(format!("the downloaded file is not the expected one ({done} bytes, sha256 {digest}); the archive.org item may have changed"));
    }
    Ok(())
}

/// Unpack every file of the zip under `dir` (no absolute paths or `..`). Returns the count.
#[cfg(feature = "download")]
fn unpack(zip_path: &Path, dir: &Path) -> Result<usize, String> {
    use std::path::Component;
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))?;
    let mut files = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name().filter(|p| p.components().all(|c| matches!(c, Component::Normal(_)))) else { continue };
        let out = dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| format!("cannot unpack {}: {e}", out.display()))?;
        files += 1;
    }
    Ok(files)
}

/// `biglips --install [DIR]`: download into DIR (or the default folder, which the game looks in
/// by itself) with a progress line, and exit.
pub fn run_cli(dir: Option<PathBuf>) -> ! {
    let Some(dir) = dir.or_else(default_dir) else {
        eprintln!("no folder to install into: pass one, --install <DIR>");
        std::process::exit(2);
    };
    if crate::setup::is_game_dir(&dir) {
        println!("Exploding Lips is already installed in {}", dir.display());
        std::process::exit(0);
    }
    println!("Downloading Exploding Lips (full version) from {ITEM_URL}");
    println!("into {}", dir.display());
    let mut last = u64::MAX;
    let result = install(&dir, |done, total| {
        let pct = done * 100 / total.max(1);
        if pct != last {
            last = pct;
            print!("\r  {pct:3}%  {:.1} / {:.1} MB", done as f64 / 1e6, total as f64 / 1e6);
            let _ = std::io::stdout().flush();
        }
    });
    println!();
    match result {
        Ok(dir) => {
            println!("Done: Exploding Lips is installed in {}. Start Big Lips to play.", dir.display());
            if Some(&dir) != default_dir().as_ref() {
                println!("(Not the default folder: start it with --data \"{}\" once.)", dir.display());
            }
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// First run without the game: offer the download (yes) or a folder of one's own (no).
/// `None`: cancelled, or the download failed (already reported).
pub fn offer_download() -> Option<PathBuf> {
    let dir = default_dir()?;
    let answer = rfd::MessageDialog::new()
        .set_title("Big Lips")
        .set_description(format!(
            "Big Lips needs the game data of Exploding Lips.\n\nDownload the full version from the Internet Archive now (43 MB, public domain)?\n{ITEM_URL}\n\nChoose No to pick a folder with your own copy instead."
        ))
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();
    match answer {
        rfd::MessageDialogResult::Yes => {}
        rfd::MessageDialogResult::No => return crate::setup::ask_game_dir(),
        _ => return None,
    }
    log::info!("downloading Exploding Lips into {}", dir.display());
    let mut last = u64::MAX;
    match install(&dir, |done, total| {
        let tenth = done * 10 / total.max(1);
        if tenth != last {
            last = tenth;
            log::info!("download {}%", tenth * 10);
        }
    }) {
        Ok(dir) => Some(dir),
        Err(e) => {
            rfd::MessageDialog::new().set_title("Big Lips").set_description(format!("The download did not work:\n{e}")).show();
            crate::setup::ask_game_dir()
        }
    }
}
