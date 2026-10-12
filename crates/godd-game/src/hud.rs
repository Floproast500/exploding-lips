//! Heads-up display, laid out exactly as the original HNH status HUD (re/engine/hud.md) in
//! 320x200 coordinates and palette colours. On wider frames the right column stays anchored
//! to the right edge and the rest to the left; everything scales by whole pixels.

use crate::script::{ARMOR, DEATHS, EXPERIENCE, KILLS, LIFE, MAGIC, MONEY, STATE_COUNT, TREASURES};
use crate::session::Session;
use crate::text::{DIAMOND, GLYPH, HEART, SPADE, draw_text, fill_rect};
use crate::{CLASSIC_HEIGHT, CLASSIC_WIDTH, Frame};

/// Palette indices used by the original HUD.
mod color {
    pub const LABEL: u8 = 120;
    pub const TIME: u8 = 195;
    pub const VALUE: u8 = 92;
    pub const STATE_LABEL: u8 = 232;
    pub const HEART: u8 = 176;
    pub const SPADE: u8 = 251;
    pub const DIAMOND: u8 = 199;
    pub const DOLLAR: u8 = 123;
    pub const STAR: u8 = 230;
    pub const OTEXT: u8 = 176;
    pub const OTEXT_BOX: u8 = 0;
}

/// Messages drawn at once, characters per line, continuation indent.
const MESSAGE_LINES: usize = 5;
const LINE_CHARS: usize = 38;
const INDENT: usize = 4;
/// OTEXT position in 320x200 coordinates.
const OTEXT_POS: (i32, i32) = (0, 126);

#[derive(Default)]
pub struct Hud {
    /// Rolling display values: move one unit per drawn frame toward the real state value.
    shown: [i32; STATE_COUNT],
    pub visible: bool,
}

impl Hud {
    pub fn new() -> Self {
        Self { shown: [0; STATE_COUNT], visible: true }
    }

    /// The modern HUD: status messages top-left; one compact cluster bottom-left with Life,
    /// Armor and Money, and bars for Magic and Experience. Scales with the frame (or `scale`),
    /// anchored to the corner at any aspect ratio. Score, time, tool, treasures, deaths and
    /// kills are on the automap's panel instead ([`Hud::draw_stats`]).
    pub fn draw_modern(&mut self, frame: &mut Frame, session: &Session, high_contrast: bool, scale: usize) {
        let fit = (frame.height / CLASSIC_HEIGHT).max(1);
        let s = if scale == 0 { fit } else { scale.min(fit) };
        let si = s as i32;
        self.draw_messages(frame, session, s);
        if !self.visible {
            return;
        }
        let pal = frame.palette;
        let col = |rgb: [u8; 3]| crate::text::nearest_color(&pal, rgb);
        let (shadow, white, back) = (col([0, 0, 0]), col([235, 235, 235]), col([24, 22, 30]));
        let (red, blue, green, pink, yellow) = (col([235, 50, 50]), col([70, 130, 255]), col([90, 220, 90]), col([240, 80, 210]), col([250, 215, 50]));
        let value = if high_contrast { col([255, 255, 80]) } else { white };
        let states = session.script.states;
        let mut roll = |i: usize| {
            let old = self.shown[i];
            self.shown[i] += (states[i] - old).signum();
            if old == states[i] { states[i] } else { old }
        };
        let (life, magic, armor, money, exp) = (roll(LIFE), roll(MAGIC), roll(ARMOR), roll(MONEY), roll(EXPERIENCE));
        let text = |frame: &mut Frame, x: i32, y: i32, color: u8, t: &str| {
            draw_text(frame, x + si, y + si, s, shadow, t);
            draw_text(frame, x, y, s, color, t)
        };
        // Bottom-left corner, 6 units in.
        let (x0, bottom) = (6 * si, frame.height as i32 - 6 * si);
        let line = 10 * si;
        // Row 1: Life, Armor, Money.
        let y1 = bottom - 3 * line;
        let mut x = x0;
        for (icon, color, number) in [(HEART.to_string(), red, format!("{life}")), (DIAMOND.to_string(), blue, format!("{armor}")), ("$".to_string(), green, format!("{money}"))] {
            x += text(frame, x, y1, color, &icon) as i32 + 2 * si;
            x += text(frame, x, y1, value, &number) as i32 + 10 * si;
        }
        // Rows 2 and 3: Magic and Experience bars.
        let thresholds = &session.script.thresholds;
        let magic_max = thresholds.max[MAGIC].max(1);
        let next_exp = ((exp / 1000) + 1) * 1000;
        let bar_w = 96 * si;
        for (row, icon, color, fill, label) in [
            (2, SPADE.to_string(), pink, magic as f32 / magic_max as f32, format!("{magic}")),
            (1, "*".to_string(), yellow, (exp % 1000) as f32 / 1000.0, format!("{exp} / {next_exp}")),
        ] {
            let y = bottom - row * line;
            let bx = x0 + 10 * si;
            text(frame, x0, y, color, &icon);
            fill_rect(frame, bx - si, y - si, (bar_w + 2 * si) as usize, (8 * si) as usize, shadow);
            fill_rect(frame, bx, y, bar_w as usize, (6 * si) as usize, back);
            let w = (bar_w as f32 * fill.clamp(0.0, 1.0)).round() as i32;
            if w > 0 {
                fill_rect(frame, bx, y, w as usize, (6 * si) as usize, color);
            }
            text(frame, bx + bar_w + 4 * si, y - si, value, &label);
        }
    }

    /// The automap's panel (modern HUD): score heading, time, tool, treasures, deaths, kills.
    pub fn draw_stats(&self, frame: &mut Frame, session: &Session, tool: usize, uptime_seconds: u64, scale: usize) {
        let fit = (frame.height / CLASSIC_HEIGHT).max(1);
        let s = if scale == 0 { fit } else { scale.min(fit) };
        let si = s as i32;
        let pal = frame.palette;
        let col = |rgb: [u8; 3]| crate::text::nearest_color(&pal, rgb);
        let (shadow, label, value, back, edge) = (col([0, 0, 0]), col([120, 200, 120]), col([235, 235, 235]), col([16, 16, 22]), col([70, 70, 90]));
        let states = session.script.states;
        let t = uptime_seconds % 3600;
        let rows = [
            ("Time", format!("{:02}:{:02}", t / 60, t % 60)),
            ("Tool", tool.to_string()),
            ("Treasures found", format!("{}", states[TREASURES])),
            ("Deaths", format!("{}", states[DEATHS])),
            ("Enemies killed", format!("{}", states[KILLS])),
        ];
        let (w, h) = (150 * si, (16 + rows.len() as i32 * 11) * si);
        let (x0, y0) = (frame.width as i32 - w - 8 * si, 8 * si);
        fill_rect(frame, x0 - si, y0 - si, (w + 2 * si) as usize, (h + 2 * si) as usize, edge);
        fill_rect(frame, x0, y0, w as usize, h as usize, back);
        draw_text(frame, x0 + 6 * si + si, y0 + 4 * si + si, s, shadow, "Score");
        draw_text(frame, x0 + 6 * si, y0 + 4 * si, s, value, "Score");
        for (k, (name, v)) in rows.iter().enumerate() {
            let y = y0 + (16 + k as i32 * 11) * si;
            draw_text(frame, x0 + 6 * si, y, s, label, name);
            let vw = (v.chars().count() * GLYPH) as i32 * si;
            draw_text(frame, x0 + w - 6 * si - vw, y, s, value, v);
        }
    }

    /// Status messages (the oldest five, top-left, wrapped) and the OTEXT box.
    fn draw_messages(&self, frame: &mut Frame, session: &Session, s: usize) {
        let si = s as i32;
        let bottom0 = frame.height as i32 - (CLASSIC_HEIGHT * s) as i32;
        let mut row = 0;
        for m in session.messages.iter().take(MESSAGE_LINES) {
            for (i, line) in wrap(&m.text).iter().enumerate() {
                let x = if i == 0 { 0 } else { (INDENT * GLYPH) as i32 };
                draw_text(frame, x * si, row * 8 * si, s, m.color, line);
                row += 1;
            }
        }
        if let Some(t) = &session.overlay_text {
            let (x, y) = OTEXT_POS;
            let w = (t.text.chars().count() * GLYPH + 6) * s;
            fill_rect(frame, x * si, bottom0 + y * si - 2 * si, w, 12 * s, color::OTEXT_BOX);
            draw_text(frame, (x + 3) * si, bottom0 + y * si, s, color::OTEXT, &t.text);
        }
    }

    /// `uptime_seconds` is time since the game was launched (the original's HUD clock).
    /// `scale` 0 picks the largest whole scale that fits; other values shrink the HUD.
    pub fn draw(&mut self, frame: &mut Frame, session: &Session, tool: usize, uptime_seconds: u64, high_contrast: bool, scale: usize) {
        // High contrast: white labels, yellow values, each with a black outline.
        let (label, value, state_label) = if high_contrast { (255u8, 230u8, 255u8) } else { (color::LABEL, color::VALUE, color::STATE_LABEL) };
        let fit = (frame.height / CLASSIC_HEIGHT).max(1);
        let s = if scale == 0 { fit } else { scale.min(fit) };
        let si = s as i32;
        // Right column origin so that x = 320 lands on the frame's right edge.
        let right0 = frame.width as i32 - (CLASSIC_WIDTH * s) as i32;
        let bottom0 = frame.height as i32 - (CLASSIC_HEIGHT * s) as i32;
        let at = |frame: &mut Frame, x: i32, y: i32, color: u8, text: &str| {
            draw_text(frame, x * si, y * si, s, color, text);
        };
        let at_right = |frame: &mut Frame, x: i32, y: i32, color: u8, text: &str| {
            if high_contrast {
                draw_text(frame, right0 + x * si + si, y * si + si, s, 0, text);
            }
            draw_text(frame, right0 + x * si, y * si, s, color, text);
        };
        let at_bottom = |frame: &mut Frame, x: i32, y: i32, color: u8, text: &str| {
            if high_contrast {
                draw_text(frame, x * si + si, bottom0 + y * si + si, s, 0, text);
            }
            draw_text(frame, x * si, bottom0 + y * si, s, color, text);
        };

        // Status messages (XM): the oldest five, top-left, wrapped.
        let mut row = 0;
        for m in session.messages.iter().take(MESSAGE_LINES) {
            for (i, line) in wrap(&m.text).iter().enumerate() {
                let x = if i == 0 { 0 } else { (INDENT * GLYPH) as i32 };
                at(frame, x, row * 8, m.color, line);
                row += 1;
            }
        }
        // OTEXT: red on a black box.
        if let Some(t) = &session.overlay_text {
            let (x, y) = OTEXT_POS;
            let w = (t.text.chars().count() * GLYPH + 6) * s;
            fill_rect(frame, x * si, bottom0 + y * si - 2 * si, w, 12 * s, color::OTEXT_BOX);
            at_bottom(frame, x + 3, y, color::OTEXT, &t.text);
        }
        if !self.visible {
            return;
        }

        let states = session.script.states;
        // Right column.
        at_right(frame, 280, 0, label, "Score");
        at_right(frame, 288, 40, label, "Time");
        let t = uptime_seconds % 3600;
        let time = format!("{:02}:{:02}", t / 60, t % 60);
        at_right(frame, 320 - 8 * time.len() as i32, 48, color::TIME, &time);
        at_right(frame, 288, 68, label, "Tool");
        at_right(frame, 304, 76, value, &tool.to_string());
        at_right(frame, 248, 88, label, "Treasures");
        at_right(frame, 264, 96, label, "  found");
        at_right(frame, 296, 104, value, &format!("{:03}", states[TREASURES]));
        at_right(frame, 272, 118, label, "Deaths");
        at_right(frame, 296, 126, value, &format!("{:03}", states[DEATHS]));
        at_right(frame, 264, 138, label, "Enemies");
        at_right(frame, 264, 146, label, " killed");
        at_right(frame, 296, 154, value, &format!("{:03}", states[KILLS]));

        // Bottom rows with rolling values.
        let mut roll = |i: usize| {
            let old = self.shown[i];
            self.shown[i] += (states[i] - old).signum();
            if old == states[i] { states[i] } else { old }
        };
        let (life, magic, armor, money, exp) = (roll(LIFE), roll(MAGIC), roll(ARMOR), roll(MONEY), roll(EXPERIENCE));
        // On wide frames the right-hand items of each row follow the right edge.
        let shift = (right0 / si).max(0);
        at_bottom(frame, 1, 181, color::HEART, &HEART.to_string());
        at_bottom(frame, 8, 181, state_label, "Life:");
        at_bottom(frame, 64, 181, value, &format!("{life:03}"));
        at_bottom(frame, 120 + shift / 2, 181, color::SPADE, &SPADE.to_string());
        at_bottom(frame, 128 + shift / 2, 181, state_label, "Magic:");
        at_bottom(frame, 176 + shift / 2, 181, value, &format!("{magic:03}"));
        at_bottom(frame, 240 + shift, 181, color::DIAMOND, &DIAMOND.to_string());
        at_bottom(frame, 248 + shift, 181, state_label, "Armor:");
        at_bottom(frame, 296 + shift, 181, value, &format!("{armor:03}"));
        at_bottom(frame, 1, 190, color::DOLLAR, "$");
        at_bottom(frame, 8, 190, state_label, "Money:");
        at_bottom(frame, 64, 190, value, &format!("{money:06}"));
        at_bottom(frame, 176 + shift, 190, color::STAR, "*");
        at_bottom(frame, 184 + shift, 190, state_label, "Experience:");
        at_bottom(frame, 272 + shift, 190, value, &format!("{exp:06}"));
    }
}

/// Wrap a message like the original: 38 bytes per line, break after the last blank (unless
/// that leaves fewer than 2 chars), continuation lines indented 4 and holding 34.
fn wrap(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest: Vec<char> = text.chars().collect();
    let mut width = LINE_CHARS;
    while rest.len() > width {
        let mut cut = width;
        if let Some(blank) = rest[..width].iter().rposition(|&c| c == ' ')
            && blank + 1 >= 2 {
                cut = blank + 1;
            }
        lines.push(rest[..cut].iter().collect());
        rest.drain(..cut);
        width = LINE_CHARS - INDENT;
    }
    lines.push(rest.into_iter().collect());
    lines
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn wraps_like_the_original() {
        let lines = wrap("Careful  your LIFE is getting low! Find a green bottle soon.");
        assert_eq!(lines[0], "Careful  your LIFE is getting low! ");
        assert!(lines.iter().skip(1).all(|l| l.chars().count() <= 34));
        assert_eq!(wrap("short"), vec!["short"]);
    }
}
