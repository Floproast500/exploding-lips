//! Texture sequence animation (XRB-records.md §5): a Tex holds entries
//! `{frame_base, next, delay}`; `next` 98 holds the current entry, 99 ends the sequence (the
//! owner's "finished" flag is set and stepping continues with the following entry). Delays are
//! 16.16 units of 12 ms (timers count down by ticks·100/120; re/engine/world-effects.md).

use godd_formats::xrb::{RecordExt, Tex};

const HOLD: u16 = 98;
const END: u16 = 99;
/// Seconds per delay unit.
const ENGINE_TICK: f32 = 0.012;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    pub frame_base: u16,
    pub next: u16,
    /// Seconds to show this entry.
    pub delay: f32,
}

/// Decoded sequence data of one Tex.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    pub entries: Vec<Entry>,
    /// Start entry per action (0 = idle, others walk/attack/die...).
    pub action_starts: [u16; 16],
    pub frames: usize,
    pub rotations: usize,
    /// Pic indices, frame-major (`frame * rotations + rotation`), with mirror flags.
    pub pics: Vec<(Option<usize>, bool)>,
}

impl Sequence {
    pub fn from_tex(tex: &Tex) -> Option<Self> {
        let r = &tex.record;
        let frames = r.le_u16(0x10) as usize;
        let rotations = (r.le_u16(0x12) as usize).max(1);
        let n = r.le_u16(0x1a) as usize;
        let data = &tex.data;
        if n == 0 || frames == 0 || data.len() < n * 12 + frames * rotations * 5 {
            return None;
        }
        let entries = (0..n)
            .map(|i| Entry {
                frame_base: data.le_u16(i * 12),
                next: data.le_u16(i * 12 + 2),
                delay: data.le_i32(i * 12 + 4).max(0) as f32 / 65536.0 * ENGINE_TICK,
            })
            .collect();
        let pics_at = n * 12;
        let mirrors_at = pics_at + frames * rotations * 4;
        let pics = (0..frames * rotations)
            .map(|k| (usize::try_from(data.le_i32(pics_at + k * 4)).ok(), data[mirrors_at + k] != 0))
            .collect();
        Some(Self { entries, action_starts: std::array::from_fn(|k| r.le_u16(0x24 + 2 * k)), frames, rotations, pics })
    }

    /// True when the sequence has more than one entry to step through.
    pub fn is_animated(&self) -> bool {
        self.entries.len() > 1 && self.entries.iter().any(|e| e.delay > 0.0)
    }

    /// Pics for every rotation of the entry's frame.
    pub fn rotations_of(&self, entry: usize) -> Vec<(usize, bool)> {
        let Some(e) = self.entries.get(entry) else { return Vec::new() };
        (0..self.rotations)
            .filter_map(|rot| {
                let slot = e.frame_base as usize + rot;
                let (pic, mirrored) = *self.pics.get(slot)?;
                Some((pic?, mirrored))
            })
            .collect()
    }
}

/// Playback state of one animated slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Player {
    pub entry: usize,
    timer: f32,
    /// Set when an entry with `next == 99` was passed.
    pub finished: bool,
}

impl Player {
    pub fn start(seq: &Sequence, action: usize) -> Self {
        let entry = seq.action_starts.get(action).copied().unwrap_or(0) as usize;
        let entry = entry.min(seq.entries.len().saturating_sub(1));
        Self { entry, timer: seq.entries[entry].delay, finished: false }
    }

    /// Resting: the current entry loops on itself or holds (combat.md `toolAnimIsIdle`).
    pub fn is_idle(&self, seq: &Sequence) -> bool {
        seq.entries.get(self.entry).is_none_or(|e| e.next as usize == self.entry || e.next == HOLD)
    }

    /// Advance by `dt` seconds; returns true when the shown entry changed.
    pub fn advance(&mut self, seq: &Sequence, dt: f32) -> bool {
        let start = self.entry;
        self.timer -= dt;
        let mut guard = 0;
        while self.timer <= 0.0 && guard < seq.entries.len() * 2 {
            guard += 1;
            let current = seq.entries[self.entry];
            let next = match current.next {
                HOLD => {
                    self.timer = f32::INFINITY;
                    break;
                }
                END => {
                    self.finished = true;
                    self.entry + 1
                }
                n => n as usize,
            };
            self.entry = if next < seq.entries.len() { next } else { 0 };
            let delay = seq.entries[self.entry].delay;
            if delay <= 0.0 {
                self.timer = f32::INFINITY;
                break;
            }
            self.timer += delay;
        }
        self.entry != start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(entries: &[(u16, u16, f32)]) -> Sequence {
        Sequence {
            entries: entries.iter().map(|&(f, n, d)| Entry { frame_base: f, next: n, delay: d }).collect(),
            action_starts: [0; 16],
            frames: entries.len(),
            rotations: 1,
            pics: (0..entries.len()).map(|i| (Some(i), false)).collect(),
        }
    }

    #[test]
    fn loops_and_holds() {
        let s = seq(&[(0, 1, 0.1), (1, 0, 0.1)]);
        let mut p = Player::start(&s, 0);
        assert!(!p.advance(&s, 0.05));
        assert!(p.advance(&s, 0.06));
        assert_eq!(p.entry, 1);
        p.advance(&s, 0.1);
        assert_eq!(p.entry, 0);

        let h = seq(&[(0, 1, 0.1), (1, HOLD, 0.1)]);
        let mut p = Player::start(&h, 0);
        p.advance(&h, 0.15);
        p.advance(&h, 10.0);
        assert_eq!(p.entry, 1, "hold keeps the last entry");
    }

    #[test]
    fn end_marks_finished() {
        let s = seq(&[(0, END, 0.1), (1, HOLD, 0.1)]);
        let mut p = Player::start(&s, 0);
        p.advance(&s, 0.2);
        assert!(p.finished);
        assert_eq!(p.entry, 1);
    }
}
