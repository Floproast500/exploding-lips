//! Desktop settings: `config.cfg` (cvars and bindings as `set` / `bind` lines, the same commands
//! the console accepts) in `$XDG_CONFIG_HOME/exploding-lips/` or the platform's config
//! directory. An old `settings.ini` from earlier versions is imported once.

use engine_core::BindTable;
use engine_core::config::exec_config;
use engine_core::cvar::{Category, CvarRegistry, HIDDEN};
use std::path::PathBuf;

/// The per-user config directory (saves and the quicksave live here too).
pub fn dir() -> Option<PathBuf> {
    // (An empty or relative XDG_CONFIG_HOME counts as unset, as the XDG spec says: the game
    // was downloaded into a relative "exploding-lips/" under whatever folder it ran from.)
    let absolute = |v: std::ffi::OsString| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .and_then(absolute)
        .or_else(|| std::env::var_os("APPDATA").and_then(absolute))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let home = PathBuf::from(h);
                if cfg!(target_os = "macos") { home.join("Library/Application Support") } else { home.join(".config") }
            })
        })?;
    Some(base.join("exploding-lips"))
}

pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join("config.cfg"))
}

/// Cvars only the desktop front end uses.
pub fn register(c: &mut CvarRegistry) {
    use Category::*;
    c.bool("vid_fullscreen", "Fullscreen (F11)", Video, false);
    c.bool("vid_vsync", "VSync (restart)", Video, true);
    c.int("vid_width", "Window width", Video, (320, 16384, 1), 1280);
    c.int("vid_height", "Window height", Video, (200, 16384, 1), 960);
    c.set_flags("vid_width", engine_core::cvar::ARCHIVE | HIDDEN);
    c.set_flags("vid_height", engine_core::cvar::ARCHIVE | HIDDEN);
    c.string("fs_data", "Game folder", Game, "");
    c.bool("r_autopack", "Build the HD textures on first run", Video, true);
    c.string("snd_soundfont", "SoundFont", Audio, "");
}

/// Load config.cfg (or import settings.ini) into a registry that already has every cvar
/// registered. Returns false when no config.cfg existed yet.
pub fn load(cvars: &mut CvarRegistry, binds: &mut BindTable) -> bool {
    let Some(path) = path() else { return false };
    if let Ok(text) = std::fs::read_to_string(&path) {
        for e in exec_config(cvars, binds, &text) {
            log::warn!("{}: {e}", path.display());
        }
        return true;
    }
    let legacy = path.with_file_name("settings.ini");
    let Ok(text) = std::fs::read_to_string(&legacy) else { return false };
    log::info!("importing {}", legacy.display());
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        let set = |name: &str, v: &str| format!("set {name} \"{v}\"");
        match key {
            "data" => lines.push(set("fs_data", value)),
            "size" => {
                if let Some((w, h)) = value.split_once('x') {
                    lines.push(set("vid_width", w));
                    lines.push(set("vid_height", h));
                }
            }
            "fullscreen" => lines.push(set("vid_fullscreen", value)),
            "classic" => lines.push(set("r_classic", value)),
            "true_color" => lines.push(set("r_truecolor", value)),
            "vsync" => lines.push(set("vid_vsync", value)),
            "fov" => lines.push(set("r_fov", value)),
            // settings.ini stored radians per count; m_sensitivity is milliradians per count.
            "mouse_sensitivity" => {
                if let Ok(v) = value.parse::<f64>() {
                    lines.push(set("m_sensitivity", &(v * 1000.0).to_string()));
                }
            }
            "invert_mouse" => lines.push(set("m_invert", value)),
            "soundfont" => lines.push(set("snd_soundfont", value)),
            k if k.starts_with("bind.") => {
                // Old rebinds replaced the action's default keys.
                binds.rebind(&k["bind.".len()..], &format!("key:{value}"), |a| a.starts_with("Menu"));
            }
            _ => {}
        }
    }
    for e in exec_config(cvars, binds, &lines.join("\n")) {
        log::warn!("{}: {e}", legacy.display());
    }
    false
}

pub fn save(text: &str) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        log::warn!("cannot save settings to {}: {e}", path.display());
    }
}
