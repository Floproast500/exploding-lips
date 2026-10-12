//! Engine audio on kira: sound effects on an effects track with a room-reverb send, positional
//! (3D) playback relative to the listener, looping channels, volume buses, and MIDI music
//! streamed through rustysynth — with a SoundFont from the system or the built-in fallback
//! ([`sf2`]). Front ends resolve sound and music names to file bytes. Audio is optional: errors
//! only log.

pub mod sf2;

use engine_core::AudioEvent;
use kira::effect::filter::FilterBuilder;
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::info::Info;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::{Sound, SoundData};
use kira::track::{SendTrackBuilder, SendTrackHandle, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Mix, Panning, Tween};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What a front end is asked to load.
pub enum Asset<'a> {
    Sound(&'a str),
    Music(&'a str),
}

/// Listener for positional sound.
#[derive(Debug, Clone, Copy, Default)]
pub struct Listener {
    pub pos: [f32; 3],
    pub angle: f32,
}

/// Distance (world units) within which a sound plays at full volume.
const NEAR: f32 = 160.0;
/// Half-volume distance beyond `NEAR`.
const ROLLOFF: f32 = 420.0;

/// Gain and pan of a sound at `pos` for `listener`: full volume up close, inverse-distance
/// falloff beyond, panned by bearing (right is clockwise from the facing).
pub fn spatialize(listener: &Listener, pos: [f32; 3]) -> (f32, f32) {
    let (dx, dy, dz) = (pos[0] - listener.pos[0], pos[1] - listener.pos[1], pos[2] - listener.pos[2]);
    let d = (dx * dx + dy * dy + dz * dz).sqrt();
    let gain = 1.0 / (1.0 + (d - NEAR).max(0.0) / ROLLOFF);
    let flat = (dx * dx + dy * dy).sqrt();
    // Directly above/below or very close: centred.
    let pan = if flat < 8.0 { 0.0 } else { -(dy.atan2(dx) - listener.angle).sin() * 0.8 * (flat / 64.0).min(1.0) };
    (gain, pan)
}

struct Channel {
    handle: StaticSoundHandle,
    volume: f32,
    pan: f32,
    pos: Option<[f32; 3]>,
}

pub struct Audio {
    _manager: AudioManager,
    effects: TrackHandle,
    /// Effects heard through walls: low-passed, same reverb send.
    muffled: TrackHandle,
    music_track: TrackHandle,
    reverb_send: SendTrackHandle,
    _reverb: ReverbHandle,
    cache: HashMap<String, Option<StaticSoundData>>,
    soundfont: Option<Arc<rustysynth::SoundFont>>,
    music: Option<MidiHandle>,
    channels: HashMap<u32, Channel>,
    listener: Listener,
    /// Master, effects and music volume (0..=1).
    volumes: (f32, f32, f32),
    /// Positional playback of sounds that carry a position.
    pub spatial: bool,
    reverb: f32,
}

impl Audio {
    /// `soundfont`: SoundFont 2 bytes for music; `None` uses the built-in fallback.
    pub fn new(soundfont: Option<Vec<u8>>) -> Option<Self> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).map_err(|e| log::warn!("audio disabled: {e}")).ok()?;
        let mut send = SendTrackBuilder::new();
        let reverb = send.add_effect(ReverbBuilder::new().feedback(0.82).damping(0.45).mix(Mix::WET));
        let reverb_send = manager.add_send_track(send).map_err(|e| log::warn!("audio: {e}")).ok()?;
        let effects = manager.add_sub_track(TrackBuilder::new().with_send(&reverb_send, Decibels::SILENCE)).map_err(|e| log::warn!("audio: {e}")).ok()?;
        let mut muffled_track = TrackBuilder::new().with_send(&reverb_send, Decibels::SILENCE);
        muffled_track.add_effect(FilterBuilder::new().cutoff(700.0));
        let muffled = manager.add_sub_track(muffled_track).map_err(|e| log::warn!("audio: {e}")).ok()?;
        let music_track = manager.add_sub_track(TrackBuilder::new()).map_err(|e| log::warn!("audio: {e}")).ok()?;
        let bytes = soundfont.unwrap_or_else(|| {
            log::info!("music: built-in fallback SoundFont");
            sf2::fallback_soundfont()
        });
        let soundfont = rustysynth::SoundFont::new(&mut Cursor::new(bytes)).map(Arc::new).map_err(|e| log::warn!("soundfont: {e}")).ok();
        Some(Self {
            _manager: manager,
            effects,
            muffled,
            music_track,
            reverb_send,
            _reverb: reverb,
            cache: HashMap::new(),
            soundfont,
            music: None,
            channels: HashMap::new(),
            listener: Listener::default(),
            volumes: (1.0, 1.0, 1.0),
            spatial: true,
            reverb: 0.0,
        })
    }

    /// Master, effects and music volume (0..=1); applied to everything playing at once.
    pub fn set_volumes(&mut self, volumes: (f32, f32, f32)) {
        if volumes == self.volumes {
            return;
        }
        self.volumes = volumes;
        self.effects.set_volume(amplitude_to_db(volumes.0 * volumes.1), Tween::default());
        self.muffled.set_volume(amplitude_to_db(volumes.0 * volumes.1), Tween::default());
        self.music_track.set_volume(amplitude_to_db(volumes.0 * volumes.2), Tween::default());
    }

    fn placed(&self, volume: f32, pan: f32, pos: Option<[f32; 3]>) -> (f32, f32) {
        match pos.filter(|_| self.spatial) {
            Some(p) => {
                let (g, pan) = spatialize(&self.listener, p);
                (volume * g, pan)
            }
            None => (volume, pan),
        }
    }

    pub fn handle(&mut self, load: &dyn Fn(Asset) -> Option<Vec<u8>>, event: &AudioEvent) {
        match event {
            AudioEvent::Sound { name, volume, pan, pos, muffle } => {
                let (volume, pan) = self.placed(*volume, *pan, *pos);
                let muffle = if self.spatial { muffle.clamp(0.0, 1.0) } else { 0.0 };
                let volume = volume * (1.0 - 0.5 * muffle);
                if volume < 0.01 {
                    return;
                }
                let Some(data) = self.load_sound(load, name) else { return };
                let data = data.volume(amplitude_to_db(volume)).panning(Panning(pan.clamp(-1.0, 1.0)));
                let track = if muffle >= 0.5 { &mut self.muffled } else { &mut self.effects };
                if let Err(e) = track.play(data) {
                    log::warn!("play {name}: {e}");
                }
            }
            AudioEvent::Loop { channel, name, volume, pan, pos } => {
                if let Some(mut old) = self.channels.remove(channel) {
                    old.handle.stop(Tween::default());
                }
                let Some(data) = self.load_sound(load, name) else { return };
                let (v, p) = self.placed(*volume, *pan, *pos);
                let data = data.loop_region(..).volume(amplitude_to_db(v)).panning(Panning(p.clamp(-1.0, 1.0)));
                match self.effects.play(data) {
                    Ok(handle) => {
                        self.channels.insert(*channel, Channel { handle, volume: *volume, pan: *pan, pos: *pos });
                    }
                    Err(e) => log::warn!("loop {name}: {e}"),
                }
            }
            AudioEvent::ChannelVolume { channel, volume, pan } => {
                if let Some(c) = self.channels.get_mut(channel) {
                    c.volume = *volume;
                    c.pan = *pan;
                    c.handle.set_volume(amplitude_to_db(*volume), Tween::default());
                    c.handle.set_panning(Panning(pan.clamp(-1.0, 1.0)), Tween::default());
                }
            }
            AudioEvent::Stop { channel } => {
                if let Some(mut c) = self.channels.remove(channel) {
                    c.handle.stop(Tween::default());
                }
            }
            AudioEvent::Listener { pos, angle, reverb } => {
                self.listener = Listener { pos: *pos, angle: *angle };
                let spatial = self.spatial;
                for c in self.channels.values_mut().filter(|c| c.pos.is_some() && spatial) {
                    let (g, pan) = spatialize(&self.listener, c.pos.unwrap_or_default());
                    c.handle.set_volume(amplitude_to_db(c.volume * g), Tween::default());
                    c.handle.set_panning(Panning(pan.clamp(-1.0, 1.0)), Tween::default());
                }
                if (reverb - self.reverb).abs() > 0.01 {
                    self.reverb = *reverb;
                    let tween = Tween { duration: std::time::Duration::from_millis(400), ..Default::default() };
                    let _ = self.effects.set_send(&self.reverb_send, amplitude_to_db(*reverb), tween);
                    let _ = self.muffled.set_send(&self.reverb_send, amplitude_to_db(*reverb), tween);
                }
            }
            AudioEvent::Music { name, looped } => {
                if let Some(old) = self.music.take() {
                    old.stop();
                }
                let (Some(name), Some(font)) = (name, &self.soundfont) else { return };
                let Some(bytes) = load(Asset::Music(name)) else { return };
                let midi = match rustysynth::MidiFile::new(&mut Cursor::new(bytes)) {
                    Ok(m) => Arc::new(m),
                    Err(e) => return log::warn!("midi {name}: {e}"),
                };
                let data = MidiSoundData { font: font.clone(), midi, looped: *looped };
                self.music = self.music_track.play(data).map_err(|e| log::warn!("music {name}: {e:?}")).ok();
            }
        }
    }

    fn load_sound(&mut self, load: &dyn Fn(Asset) -> Option<Vec<u8>>, name: &str) -> Option<StaticSoundData> {
        self.cache
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| {
                let bytes = load(Asset::Sound(name))?;
                StaticSoundData::from_cursor(Cursor::new(bytes)).map_err(|e| log::warn!("decode {name}: {e}")).ok()
            })
            .clone()
    }
}

fn amplitude_to_db(amplitude: f32) -> Decibels {
    if amplitude <= 0.0 { Decibels::SILENCE } else { Decibels(20.0 * amplitude.log10()) }
}

/// MIDI music rendered while it plays (no up-front render, starts instantly).
struct MidiSoundData {
    font: Arc<rustysynth::SoundFont>,
    midi: Arc<rustysynth::MidiFile>,
    looped: bool,
}

pub struct MidiHandle {
    stop: Arc<AtomicBool>,
}

impl MidiHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl SoundData for MidiSoundData {
    type Error = ();
    type Handle = MidiHandle;

    fn into_sound(self) -> Result<(Box<dyn Sound>, Self::Handle), Self::Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let sound = MidiSound { data: self, sequencer: None, left: Vec::new(), right: Vec::new(), stop: stop.clone(), done: false, fade: 1.0 };
        Ok((Box::new(sound), MidiHandle { stop }))
    }
}

struct MidiSound {
    data: MidiSoundData,
    sequencer: Option<rustysynth::MidiFileSequencer>,
    left: Vec<f32>,
    right: Vec<f32>,
    stop: Arc<AtomicBool>,
    done: bool,
    /// Short fade-out after a stop request (no click).
    fade: f32,
}

impl Sound for MidiSound {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        if self.sequencer.is_none() {
            // The synthesizer runs at the output rate, known only here.
            let rate = (1.0 / dt).round() as i32;
            let Ok(synth) = rustysynth::Synthesizer::new(&self.data.font, &rustysynth::SynthesizerSettings::new(rate)) else {
                self.done = true;
                return;
            };
            let mut seq = rustysynth::MidiFileSequencer::new(synth);
            seq.play(&self.data.midi, self.data.looped);
            self.sequencer = Some(seq);
        }
        let n = out.len();
        if self.left.len() < n {
            self.left.resize(n, 0.0);
            self.right.resize(n, 0.0);
        }
        let seq = self.sequencer.as_mut().expect("sequencer");
        seq.render(&mut self.left[..n], &mut self.right[..n]);
        let stopping = self.stop.load(Ordering::Relaxed);
        let step = (dt * 20.0) as f32; // 50 ms fade
        for (i, f) in out.iter_mut().enumerate() {
            if stopping {
                self.fade = (self.fade - step).max(0.0);
            }
            *f = Frame { left: self.left[i] * self.fade, right: self.right[i] * self.fade };
        }
        if (stopping && self.fade <= 0.0) || (!self.data.looped && seq.end_of_sequence()) {
            self.done = true;
        }
    }

    fn finished(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_music_plays_at_the_output_rate_and_stops() {
        let font = Arc::new(rustysynth::SoundFont::new(&mut Cursor::new(sf2::fallback_soundfont())).unwrap());
        // Middle C held for a while on program 0.
        let track: Vec<u8> = vec![0x00, 0xC0, 0, 0x00, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 0, 0x00, 0xFF, 0x2F, 0x00];
        let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk".to_vec();
        bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
        bytes.extend(track);
        let midi = Arc::new(rustysynth::MidiFile::new(&mut Cursor::new(bytes)).unwrap());
        let (mut sound, handle) = MidiSoundData { font, midi, looped: true }.into_sound().unwrap();
        let info = kira::info::MockInfoBuilder::new().build();
        let mut out = vec![Frame::ZERO; 4800];
        sound.process(&mut out, 1.0 / 48_000.0, &info);
        assert!(out.iter().any(|f| f.left.abs() > 0.01), "music is audible");
        assert!(!sound.finished());
        handle.stop();
        for _ in 0..10 {
            sound.process(&mut out, 1.0 / 48_000.0, &info);
        }
        assert!(sound.finished(), "fades out and finishes after stop");
    }

    #[test]
    fn positional_gain_and_pan() {
        let l = Listener { pos: [0.0, 0.0, 0.0], angle: 0.0 }; // facing +X
        assert_eq!(spatialize(&l, [100.0, 0.0, 0.0]).0, 1.0, "full volume up close");
        let (g, _) = spatialize(&l, [160.0 + 420.0, 0.0, 0.0]);
        assert!((g - 0.5).abs() < 1e-6, "half volume at NEAR + ROLLOFF");
        let (_, right) = spatialize(&l, [0.0, -500.0, 0.0]); // -Y is to the right when facing +X
        let (_, left) = spatialize(&l, [0.0, 500.0, 0.0]);
        assert!(right > 0.7 && left < -0.7, "pan {right} / {left}");
        assert_eq!(spatialize(&l, [0.0, 0.0, 300.0]).1, 0.0, "overhead is centred");
    }
}
