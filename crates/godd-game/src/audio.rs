//! Audio requests produced by the simulation. Front ends own the audio device and decode the
//! files; the game only names sounds (`snd/<name>.wav`) and music (`snd/<name>.mid`).

#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// Play a one-shot sound effect.
    Sound {
        name: String,
        /// 0..=1
        volume: f32,
        /// -1 (left) ..= 1 (right)
        pan: f32,
    },
    /// Start a looping sound on a channel (replacing whatever plays there).
    Loop { channel: u32, name: String, volume: f32, pan: f32 },
    /// Change a looping channel's volume/pan (positional sounds).
    ChannelVolume { channel: u32, volume: f32, pan: f32 },
    /// Stop a looping channel.
    Stop { channel: u32 },
    /// Start (or switch to) background music; `None` stops it.
    Music { name: Option<String>, looped: bool },
}

impl AudioEvent {
    pub fn sound(name: impl Into<String>) -> Self {
        Self::Sound { name: name.into(), volume: 1.0, pan: 0.0 }
    }
}

/// Path of a sound effect inside the game folder.
pub fn sound_path(name: &str) -> String {
    format!("snd/{name}.wav")
}

/// Path of a music track inside the game folder.
pub fn music_path(name: &str) -> String {
    format!("snd/{name}.mid")
}

/// Playing time of a PCM WAV file in seconds (from its header), if it can be read.
pub fn wav_seconds(data: &[u8]) -> Option<f32> {
    if data.get(0..4)? != b"RIFF" || data.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut byte_rate = None;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        match id {
            b"fmt " => byte_rate = Some(u32::from_le_bytes(data.get(pos + 16..pos + 20)?.try_into().ok()?)),
            b"data" => return byte_rate.filter(|&r| r > 0).map(|r| len as f32 / r as f32),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_duration_from_header() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend(16u32.to_le_bytes());
        wav.extend([1, 0, 1, 0]); // PCM, mono
        wav.extend(22050u32.to_le_bytes());
        wav.extend(44100u32.to_le_bytes()); // byte rate
        wav.extend([2, 0, 16, 0]);
        wav.extend(b"data");
        wav.extend(22050u32.to_le_bytes());
        wav.extend(vec![0u8; 22050]);
        assert_eq!(super::wav_seconds(&wav), Some(0.5));
    }
}
