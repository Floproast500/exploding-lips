//! Desktop audio: plays the game's WAV effects with kira and renders MIDI music through a
//! user-supplied SoundFont (rustysynth). Audio is optional — failures only log.

use godd_game::{AudioEvent, Vfs, music_path, sound_path};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Decibels, Panning, Tween};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

const MUSIC_SAMPLE_RATE: u32 = 44_100;

pub struct Audio {
    manager: AudioManager,
    cache: HashMap<String, Option<StaticSoundData>>,
    soundfont: Option<Arc<rustysynth::SoundFont>>,
    music: Option<StaticSoundHandle>,
    channels: HashMap<u32, StaticSoundHandle>,
}

impl Audio {
    pub fn new(soundfont: Option<&Path>) -> Option<Self> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| log::warn!("audio disabled: {e}"))
            .ok()?;
        let found = soundfont.map(Path::to_path_buf).or_else(find_soundfont);
        if found.is_none() {
            log::warn!("music disabled: no SoundFont found (install one, e.g. `soundfont-fluid`, or pass --soundfont FILE.sf2)");
        }
        let soundfont = found.as_deref().and_then(|path| {
            log::info!("music: SoundFont {}", path.display());
            let mut file = std::fs::File::open(path).map_err(|e| log::warn!("soundfont {}: {e}", path.display())).ok()?;
            rustysynth::SoundFont::new(&mut file).map(Arc::new).map_err(|e| log::warn!("soundfont: {e}")).ok()
        });
        Some(Self { manager, cache: HashMap::new(), soundfont, music: None, channels: HashMap::new() })
    }

    pub fn handle(&mut self, vfs: &Vfs, event: &AudioEvent) {
        match event {
            AudioEvent::Sound { name, volume, pan } => {
                let Some(data) = self.load_sound(vfs, name) else { return };
                let data = data.volume(amplitude_to_db(*volume)).panning(Panning(pan.clamp(-1.0, 1.0)));
                if let Err(e) = self.manager.play(data) {
                    log::warn!("play {name}: {e}");
                }
            }
            AudioEvent::Loop { channel, name, volume, pan } => {
                if let Some(mut old) = self.channels.remove(channel) {
                    old.stop(Tween::default());
                }
                let Some(data) = self.load_sound(vfs, name) else { return };
                let data = data.loop_region(..).volume(amplitude_to_db(*volume)).panning(Panning(pan.clamp(-1.0, 1.0)));
                match self.manager.play(data) {
                    Ok(handle) => {
                        self.channels.insert(*channel, handle);
                    }
                    Err(e) => log::warn!("loop {name}: {e}"),
                }
            }
            AudioEvent::ChannelVolume { channel, volume, pan } => {
                if let Some(h) = self.channels.get_mut(channel) {
                    h.set_volume(amplitude_to_db(*volume), Tween::default());
                    h.set_panning(Panning(pan.clamp(-1.0, 1.0)), Tween::default());
                }
            }
            AudioEvent::Stop { channel } => {
                if let Some(mut h) = self.channels.remove(channel) {
                    h.stop(Tween::default());
                }
            }
            AudioEvent::Music { name, looped } => {
                if let Some(mut old) = self.music.take() {
                    old.stop(Tween::default());
                }
                let Some(name) = name else { return };
                let Some(mut data) = self.render_midi(vfs, name) else { return };
                if *looped {
                    data = data.loop_region(..);
                }
                self.music = self.manager.play(data).map_err(|e| log::warn!("music {name}: {e}")).ok();
            }
        }
    }

    fn load_sound(&mut self, vfs: &Vfs, name: &str) -> Option<StaticSoundData> {
        self.cache
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| {
                let bytes = vfs.read(&sound_path(name)).map_err(|e| log::warn!("{e}")).ok()?;
                StaticSoundData::from_cursor(Cursor::new(bytes.to_vec())).map_err(|e| log::warn!("decode {name}: {e}")).ok()
            })
            .clone()
    }

    fn render_midi(&self, vfs: &Vfs, name: &str) -> Option<StaticSoundData> {
        let soundfont = self.soundfont.as_ref()?;
        let bytes = vfs.read(&music_path(name)).map_err(|e| log::warn!("{e}")).ok()?;
        let midi = rustysynth::MidiFile::new(&mut Cursor::new(bytes)).map_err(|e| log::warn!("midi {name}: {e}")).ok()?;
        let settings = rustysynth::SynthesizerSettings::new(MUSIC_SAMPLE_RATE as i32);
        let synth = rustysynth::Synthesizer::new(soundfont, &settings).ok()?;
        let mut sequencer = rustysynth::MidiFileSequencer::new(synth);
        let midi = Arc::new(midi);
        sequencer.play(&midi, false);
        let frames = (midi.get_length() * MUSIC_SAMPLE_RATE as f64) as usize + MUSIC_SAMPLE_RATE as usize;
        let (mut left, mut right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        sequencer.render(&mut left, &mut right);
        let frames: Arc<[kira::Frame]> = left.iter().zip(&right).map(|(&l, &r)| kira::Frame { left: l, right: r }).collect();
        Some(StaticSoundData {
            sample_rate: MUSIC_SAMPLE_RATE,
            frames,
            settings: Default::default(),
            slice: None,
        })
    }
}

/// Look for a General MIDI SoundFont in the usual system locations.
fn find_soundfont() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/soundfonts/default.sf2",
        "/usr/share/soundfonts/FluidR3_GM.sf2",
        "/usr/share/sounds/sf2/FluidR3_GM.sf2",
        "/usr/share/sounds/sf2/default-GM.sf2",
        "/usr/share/soundfonts/GeneralUser-GS.sf2",
        "/usr/share/sounds/sf2/TimGM6mb.sf2",
        "/opt/homebrew/share/soundfonts/default.sf2",
        "C:\\soundfonts\\default.sf2",
    ];
    CANDIDATES.iter().map(std::path::PathBuf::from).find(|p| p.is_file()).or_else(|| {
        let dir = std::fs::read_dir("/usr/share/soundfonts").ok()?;
        dir.filter_map(Result::ok).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sf2")))
    })
}

fn amplitude_to_db(amplitude: f32) -> Decibels {
    if amplitude <= 0.0 { Decibels::SILENCE } else { Decibels(20.0 * amplitude.log10()) }
}
