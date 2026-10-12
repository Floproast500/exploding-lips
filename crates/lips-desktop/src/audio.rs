//! Desktop audio: engine-audio playback with the game's files, music through a SoundFont from
//! the settings, a system location, or the built-in fallback.

use godd_game::{AudioEvent, Vfs, music_path, sound_path};
use std::path::{Path, PathBuf};

pub struct Audio {
    inner: engine_audio::Audio,
}

impl Audio {
    pub fn new(soundfont: Option<&Path>) -> Option<Self> {
        let path = soundfont.map(Path::to_path_buf).or_else(find_soundfont);
        let bytes = path.as_ref().and_then(|p| std::fs::read(p).map_err(|e| log::warn!("soundfont {}: {e}", p.display())).ok());
        if let Some(p) = path.as_ref().filter(|_| bytes.is_some()) {
            log::info!("music: SoundFont {}", p.display());
        }
        Some(Self { inner: engine_audio::Audio::new(bytes)? })
    }

    pub fn set_volumes(&mut self, volumes: (f32, f32, f32)) {
        self.inner.set_volumes(volumes);
    }

    pub fn set_spatial(&mut self, on: bool) {
        self.inner.spatial = on;
    }

    pub fn handle(&mut self, vfs: &Vfs, event: &AudioEvent) {
        let load = |asset: engine_audio::Asset| {
            let path = match asset {
                engine_audio::Asset::Sound(n) => sound_path(n),
                engine_audio::Asset::Music(n) => music_path(n),
            };
            vfs.read(&path).map(<[u8]>::to_vec).map_err(|e| log::warn!("{e}")).ok()
        };
        self.inner.handle(&load, event);
    }
}

/// A General MIDI SoundFont in the usual system locations.
fn find_soundfont() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/soundfonts/default.sf2",
        "/usr/share/soundfonts/FluidR3_GM.sf2",
        "/usr/share/sounds/sf2/FluidR3_GM.sf2",
        "/usr/share/sounds/sf2/default-GM.sf2",
        "/usr/share/soundfonts/GeneralUser-GS.sf2",
        "/usr/share/sounds/sf2/TimGM6mb.sf2",
        "/opt/homebrew/share/soundfonts/default.sf2",
    ];
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file()).or_else(|| {
        let dir = std::fs::read_dir("/usr/share/soundfonts").ok()?;
        dir.filter_map(Result::ok).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sf2")))
    })
}
