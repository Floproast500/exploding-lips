//! Bring your own data: Big Lips ships without Exploding Lips' files. On start it finds the
//! player's copy of the game (the configured folder, common places, a download from the Internet
//! Archive, see `install`, or a folder picker) and,
//! on first run, builds the HD texture pack from it in the background with `packbuilder`.

use std::path::{Path, PathBuf};

/// A folder holding the original game (its first level and palette).
pub fn is_game_dir(dir: &Path) -> bool {
    let has = |names: &[&str]| names.iter().any(|n| dir.join(n).is_file());
    has(&["LIPS00.XRB", "lips00.xrb", "Lips00.xrb"]) && has(&["PALETTE.PAL", "palette.pal", "Palette.pal"])
}

/// The configured game folder if it is valid, else the first of the usual places that is.
pub fn find_game_dir(configured: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !configured.is_empty() {
        candidates.push(configured.into());
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.extend([exe_dir.join("original"), exe_dir.join("data"), exe_dir.clone()]);
    }
    candidates.extend([PathBuf::from("original"), PathBuf::from(".")]);
    // Where `--install` and the first-run download put it.
    candidates.extend(crate::install::default_dir());
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) {
        for rel in ["Games/Exploding Lips", "Games/ExplodingLips", "Exploding Lips", ".local/share/biglips/game"] {
            candidates.push(home.join(rel));
        }
    }
    candidates.into_iter().find(|d| is_game_dir(d))
}

/// Ask the player for their Exploding Lips folder (system folder picker), until they pick a
/// valid one or cancel.
pub fn ask_game_dir() -> Option<PathBuf> {
    loop {
        let dir = rfd::FileDialog::new().set_title("Big Lips: choose your Exploding Lips folder (the one with LIPS00.XRB)").pick_folder()?;
        if is_game_dir(&dir) {
            return Some(dir);
        }
        let again = rfd::MessageDialog::new()
            .set_title("Big Lips")
            .set_description(format!("{} does not look like Exploding Lips (no LIPS00.XRB and PALETTE.PAL). Choose another folder?", dir.display()))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if again != rfd::MessageDialogResult::Yes {
            return None;
        }
    }
}

/// Where the first-run HD pack is built.
pub fn auto_pack_dir() -> Option<PathBuf> {
    crate::config::dir().map(|d| d.join("packs").join("lips-hd"))
}

/// A background `packbuilder` run.
pub struct PackJob {
    child: std::process::Child,
    out: PathBuf,
}

impl PackJob {
    /// Start building the HD pack from `data` with the `packbuilder` that ships next to the
    /// game (Real-ESRGAN when it is installed, else the built-in upscaler). `None` when there
    /// is no packbuilder.
    pub fn start(data: &Path, out: PathBuf) -> Option<Self> {
        let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let tool = ["packbuilder", "packbuilder.exe"].iter().map(|n| exe_dir.join(n)).find(|p| p.is_file())?;
        let child = std::process::Command::new(tool)
            .arg(data)
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| log::warn!("cannot start packbuilder: {e}"))
            .ok()?;
        log::info!("building the HD texture pack in {} (first run)", out.display());
        Some(Self { child, out })
    }

    /// `Some` once finished: the pack folder, or why it failed.
    pub fn poll(&mut self) -> Option<Result<PathBuf, String>> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) if status.success() && self.out.join("pack.txt").is_file() => Some(Ok(self.out.clone())),
            Ok(Some(status)) => Some(Err(format!("packbuilder failed ({status})"))),
            Err(e) => Some(Err(e.to_string())),
        }
    }
}
