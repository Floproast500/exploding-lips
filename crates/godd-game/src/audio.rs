//! Where G.O.D.D. games keep their audio: sounds as `snd/<name>.wav`, music as `snd/<name>.mid`.
//! The requests themselves are [`engine_core::AudioEvent`].

pub use engine_core::audio::*;

/// Path of a sound effect inside the game folder.
pub fn sound_path(name: &str) -> String {
    format!("snd/{name}.wav")
}

/// Path of a music track inside the game folder.
pub fn music_path(name: &str) -> String {
    format!("snd/{name}.mid")
}
