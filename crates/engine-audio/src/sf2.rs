//! A small General MIDI SoundFont generated in code, so MIDI music plays without any
//! third-party sound data. Instruments are single-cycle waveforms (sine, triangle, band-limited
//! saw and square) with per-family envelopes, plus a noise kit for drums; every GM program maps
//! to one of them. It sounds like an early-90s sound card, which suits the era.

use std::f32::consts::TAU;

/// Samples per waveform cycle; the sample rate is chosen so the cycle plays at middle C.
const CYCLE: usize = 256;
const ROOT_KEY: u8 = 60;
const MIDDLE_C: f32 = 261.625_58;
/// SoundFont 2 requires 46 zero samples after each sample.
const GUARD: usize = 46;

struct Wave {
    name: &'static str,
    data: Vec<i16>,
    looped: bool,
    rate: u32,
}

/// (name, wave index, attack s, decay s, sustain attenuation cB, release s, attenuation cB)
type Inst = (&'static str, usize, f32, f32, i16, f32, i16);

const INSTRUMENTS: [Inst; 8] = [
    ("Keys", 0, 0.002, 1.2, 300, 0.25, 40),
    ("Organ", 3, 0.01, 0.1, 30, 0.12, 120),
    ("Strings", 2, 0.12, 0.6, 60, 0.45, 110),
    ("Bass", 1, 0.004, 0.6, 150, 0.12, 20),
    ("Lead", 2, 0.01, 0.3, 80, 0.2, 130),
    ("Pad", 1, 0.35, 1.0, 60, 0.9, 90),
    ("Pluck", 1, 0.001, 0.5, 600, 0.15, 30),
    ("Drums", 4, 0.001, 0.18, 960, 0.08, 0),
];

/// Instrument for each GM family (programs 8k..8k+7).
const FAMILY: [usize; 16] = [0, 6, 1, 6, 3, 2, 2, 4, 4, 5, 4, 5, 5, 6, 6, 7];

fn waves() -> Vec<Wave> {
    let cycle = |f: &dyn Fn(f32) -> f32| -> Vec<i16> { (0..CYCLE).map(|i| (f(i as f32 / CYCLE as f32) * 20_000.0) as i16).collect() };
    let harmonics = |odd_only: bool| {
        move |t: f32| {
            let mut v = 0.0;
            for k in 1..=12 {
                if odd_only && k % 2 == 0 {
                    continue;
                }
                v += (TAU * k as f32 * t).sin() / k as f32;
            }
            v * if odd_only { 0.9 } else { 0.6 }
        }
    };
    let rate = (CYCLE as f32 * MIDDLE_C).round() as u32;
    let mut noise = Vec::with_capacity(8192);
    let mut x: u32 = 0x1234_5678;
    for _ in 0..8192 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        noise.push(((x >> 16) as i16).wrapping_sub(0) / 2);
    }
    vec![
        Wave { name: "sine", data: cycle(&|t| (TAU * t).sin()), looped: true, rate },
        Wave { name: "triangle", data: cycle(&|t| 1.0 - 4.0 * (t - 0.5).abs()), looped: true, rate },
        Wave { name: "saw", data: cycle(&harmonics(false)), looped: true, rate },
        Wave { name: "square", data: cycle(&harmonics(true)), looped: true, rate },
        Wave { name: "noise", data: noise, looped: false, rate: 22_050 },
    ]
}

fn timecents(seconds: f32) -> i16 {
    (1200.0 * seconds.max(0.001).log2()).round() as i16
}

fn name20(s: &str) -> [u8; 20] {
    let mut out = [0u8; 20];
    for (o, b) in out.iter_mut().zip(s.bytes().take(19)) {
        *o = b;
    }
    out
}

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 9);
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = kind.to_vec();
    for p in parts {
        body.extend_from_slice(p);
    }
    chunk(b"LIST", &body)
}

// Generator operators (SoundFont 2.04 §8.1.2).
const GEN_INITIAL_ATTENUATION: u16 = 48;
const GEN_ATTACK: u16 = 34;
const GEN_DECAY: u16 = 36;
const GEN_SUSTAIN: u16 = 37;
const GEN_RELEASE: u16 = 38;
const GEN_INSTRUMENT: u16 = 41;
const GEN_SAMPLE_ID: u16 = 53;
const GEN_SAMPLE_MODES: u16 = 54;
const GEN_ROOT_KEY: u16 = 58;

/// The SoundFont file bytes.
pub fn fallback_soundfont() -> Vec<u8> {
    let waves = waves();
    // sdta: all samples back to back, each followed by the guard zeros.
    let mut smpl: Vec<u8> = Vec::new();
    let mut shdr: Vec<u8> = Vec::new();
    let mut at = 0u32;
    for w in &waves {
        let start = at;
        let end = start + w.data.len() as u32;
        for s in w.data.iter().chain(std::iter::repeat_n(&0i16, GUARD)) {
            smpl.extend_from_slice(&s.to_le_bytes());
        }
        at = end + GUARD as u32;
        shdr.extend_from_slice(&name20(w.name));
        for v in [start, end, start, end, w.rate] {
            shdr.extend_from_slice(&v.to_le_bytes());
        }
        shdr.extend_from_slice(&[ROOT_KEY, 0]);
        shdr.extend_from_slice(&0u16.to_le_bytes()); // sample link
        shdr.extend_from_slice(&1u16.to_le_bytes()); // mono
    }
    shdr.extend_from_slice(&name20("EOS")); // terminal record
    shdr.extend_from_slice(&[0u8; 26]);

    // Instruments: one zone each (envelope, loop mode, sample).
    let (mut inst, mut ibag, mut igen) = (Vec::new(), Vec::new(), Vec::new());
    let mut gen_count = 0u16;
    let push_gen = |g: &mut Vec<u8>, n: &mut u16, op: u16, amount: i16| {
        g.extend_from_slice(&op.to_le_bytes());
        g.extend_from_slice(&amount.to_le_bytes());
        *n += 1;
    };
    for (i, &(name, wave, attack, decay, sustain, release, atten)) in INSTRUMENTS.iter().enumerate() {
        inst.extend_from_slice(&name20(name));
        inst.extend_from_slice(&(i as u16).to_le_bytes());
        ibag.extend_from_slice(&gen_count.to_le_bytes());
        ibag.extend_from_slice(&0u16.to_le_bytes());
        push_gen(&mut igen, &mut gen_count, GEN_INITIAL_ATTENUATION, atten);
        push_gen(&mut igen, &mut gen_count, GEN_ATTACK, timecents(attack));
        push_gen(&mut igen, &mut gen_count, GEN_DECAY, timecents(decay));
        push_gen(&mut igen, &mut gen_count, GEN_SUSTAIN, sustain);
        push_gen(&mut igen, &mut gen_count, GEN_RELEASE, timecents(release));
        push_gen(&mut igen, &mut gen_count, GEN_ROOT_KEY, ROOT_KEY as i16);
        push_gen(&mut igen, &mut gen_count, GEN_SAMPLE_MODES, if waves[wave].looped { 1 } else { 0 });
        push_gen(&mut igen, &mut gen_count, GEN_SAMPLE_ID, wave as i16);
    }
    inst.extend_from_slice(&name20("EOI"));
    inst.extend_from_slice(&(INSTRUMENTS.len() as u16).to_le_bytes());
    ibag.extend_from_slice(&gen_count.to_le_bytes());
    ibag.extend_from_slice(&0u16.to_le_bytes());
    igen.extend_from_slice(&[0u8; 4]);

    // Presets: GM programs 0..127 in bank 0, the drum kit in bank 128.
    let (mut phdr, mut pbag, mut pgen) = (Vec::new(), Vec::new(), Vec::new());
    let mut pgen_count = 0u16;
    let presets: Vec<(u16, u16, usize)> = (0..128u16).map(|p| (p, 0u16, FAMILY[p as usize / 8])).chain(std::iter::once((0, 128, 7))).collect();
    for (k, &(program, bank, instrument)) in presets.iter().enumerate() {
        phdr.extend_from_slice(&name20(&format!("{} {program}", INSTRUMENTS[instrument].0)));
        phdr.extend_from_slice(&program.to_le_bytes());
        phdr.extend_from_slice(&bank.to_le_bytes());
        phdr.extend_from_slice(&(k as u16).to_le_bytes());
        phdr.extend_from_slice(&[0u8; 12]);
        pbag.extend_from_slice(&pgen_count.to_le_bytes());
        pbag.extend_from_slice(&0u16.to_le_bytes());
        pgen.extend_from_slice(&GEN_INSTRUMENT.to_le_bytes());
        pgen.extend_from_slice(&(instrument as u16).to_le_bytes());
        pgen_count += 1;
    }
    phdr.extend_from_slice(&name20("EOP"));
    phdr.extend_from_slice(&[0u8; 4]);
    phdr.extend_from_slice(&(presets.len() as u16).to_le_bytes());
    phdr.extend_from_slice(&[0u8; 12]);
    pbag.extend_from_slice(&pgen_count.to_le_bytes());
    pbag.extend_from_slice(&0u16.to_le_bytes());
    pgen.extend_from_slice(&[0u8; 4]);
    let terminal_mod = [0u8; 10];

    let info = list(b"INFO", &[chunk(b"ifil", &[2, 0, 1, 0]), chunk(b"isng", b"EMU8000\0"), chunk(b"INAM", b"Fallback GM\0")]);
    let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);
    let pdta = list(
        b"pdta",
        &[
            chunk(b"phdr", &phdr),
            chunk(b"pbag", &pbag),
            chunk(b"pmod", &terminal_mod),
            chunk(b"pgen", &pgen),
            chunk(b"inst", &inst),
            chunk(b"ibag", &ibag),
            chunk(b"imod", &terminal_mod),
            chunk(b"igen", &igen),
            chunk(b"shdr", &shdr),
        ],
    );
    let mut body = b"sfbk".to_vec();
    body.extend(info);
    body.extend(sdta);
    body.extend(pdta);
    chunk(b"RIFF", &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A one-track MIDI file: program change, then a few notes on channels 0 and 9 (drums).
    fn tiny_midi() -> Vec<u8> {
        let mut track: Vec<u8> = vec![0x00, 0xC0, 40]; // program 40 (strings) on channel 0
        for (k, note) in [60u8, 64, 67].iter().enumerate() {
            track.extend_from_slice(&[if k == 0 { 0x00 } else { 0x60 }, 0x90, *note, 100]);
            track.extend_from_slice(&[0x00, 0x99, 36, 110]);
            track.extend_from_slice(&[0x60, 0x80, *note, 0]);
        }
        track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let mut midi = b"MThd\0\0\0\x06\0\0\0\x01\0\x60".to_vec();
        midi.extend_from_slice(b"MTrk");
        midi.extend_from_slice(&(track.len() as u32).to_be_bytes());
        midi.extend(track);
        midi
    }

    #[test]
    fn soundfont_parses_and_plays_music() {
        let bytes = fallback_soundfont();
        let font = Arc::new(rustysynth::SoundFont::new(&mut std::io::Cursor::new(&bytes)).expect("valid SoundFont"));
        assert_eq!(font.get_presets().len(), 129);
        let midi = Arc::new(rustysynth::MidiFile::new(&mut std::io::Cursor::new(tiny_midi())).unwrap());
        let synth = rustysynth::Synthesizer::new(&font, &rustysynth::SynthesizerSettings::new(44_100)).unwrap();
        let mut seq = rustysynth::MidiFileSequencer::new(synth);
        seq.play(&midi, false);
        let (mut l, mut r) = (vec![0.0f32; 44_100], vec![0.0f32; 44_100]);
        seq.render(&mut l, &mut r);
        let peak = l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.05 && peak < 1.5, "audible and not clipping wildly: peak {peak}");
    }
}
