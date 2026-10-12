//! Persistent desktop settings in a small `key = value` file:
//! `$XDG_CONFIG_HOME/exploding-lips/settings.ini` (or the platform's config directory).
//! Command-line options override it; F9/F11 toggles are written back on exit.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub data: Option<PathBuf>,
    pub size: (u32, u32),
    pub fullscreen: bool,
    pub classic: bool,
    pub true_color: bool,
    pub vsync: bool,
    /// Vertical field of view in degrees (original ≈ 48.9).
    pub fov: f32,
    /// Radians per mouse count.
    pub mouse_sensitivity: f32,
    pub invert_mouse: bool,
    pub soundfont: Option<PathBuf>,
    /// Rebound actions: action name → key name (`bind.Forward = KeyW`).
    pub bindings: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data: None,
            size: (1280, 960),
            fullscreen: false,
            classic: false,
            true_color: false,
            vsync: true,
            fov: 48.9,
            mouse_sensitivity: 0.0025,
            invert_mouse: false,
            soundfont: None,
            bindings: Default::default(),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let home = PathBuf::from(h);
                if cfg!(target_os = "macos") { home.join("Library/Application Support") } else { home.join(".config") }
            })
        })?;
    Some(base.join("exploding-lips").join("settings.ini"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return s };
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            let flag = || matches!(value, "1" | "true" | "yes" | "on");
            match key {
                "data" if !value.is_empty() => s.data = Some(value.into()),
                "size" => {
                    if let Some((w, h)) = value.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                        s.size = (w, h);
                    }
                }
                "fullscreen" => s.fullscreen = flag(),
                "classic" => s.classic = flag(),
                "true_color" => s.true_color = flag(),
                "vsync" => s.vsync = flag(),
                "fov" => s.fov = value.parse::<f32>().map_or(s.fov, |v| v.clamp(30.0, 120.0)),
                "mouse_sensitivity" => s.mouse_sensitivity = value.parse().unwrap_or(s.mouse_sensitivity),
                "invert_mouse" => s.invert_mouse = flag(),
                "soundfont" if !value.is_empty() => s.soundfont = Some(value.into()),
                k if k.starts_with("bind.") => {
                    s.bindings.insert(k["bind.".len()..].to_string(), value.to_string());
                }
                _ => log::warn!("settings: unknown key {key:?}"),
            }
        }
        s
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let opt = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        let text = format!(
            "# Exploding Lips settings\n\
             data = {}\nsize = {}x{}\nfullscreen = {}\nclassic = {}\ntrue_color = {}\nvsync = {}\n\
             fov = {}\nmouse_sensitivity = {}\ninvert_mouse = {}\nsoundfont = {}\n",
            opt(&self.data),
            self.size.0,
            self.size.1,
            self.fullscreen,
            self.classic,
            self.true_color,
            self.vsync,
            self.fov,
            self.mouse_sensitivity,
            self.invert_mouse,
            opt(&self.soundfont),
        );
        let mut text = text;
        for (action, key) in &self.bindings {
            text.push_str(&format!("bind.{action} = {key}\n"));
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, text) {
            log::warn!("cannot save settings to {}: {e}", path.display());
        }
    }
}
