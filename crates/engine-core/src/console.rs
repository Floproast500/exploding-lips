//! The drop-down console's state: line editing, history, Tab completion and scrollback. Games
//! draw it and run the submitted lines.

/// Lines kept for scrollback.
const KEEP: usize = 500;

#[derive(Debug, Default)]
pub struct Console {
    pub open: bool,
    pub line: String,
    pub output: Vec<String>,
    history: Vec<String>,
    /// Position while browsing history (`None`: editing a new line).
    browsing: Option<usize>,
    /// Lines scrolled up from the bottom.
    pub scroll: usize,
}

impl Console {
    /// Typed text: printable characters, backspace (`\u{8}`) and Tab (`\t`, completes the first
    /// word against `words`).
    pub fn edit(&mut self, text: &str, words: &[&str]) {
        for c in text.chars() {
            match c {
                '\u{8}' => {
                    self.line.pop();
                }
                '\t' => self.complete(words),
                '`' | '~' => {}
                c if !c.is_control() => self.line.push(c),
                _ => {}
            }
        }
    }

    fn complete(&mut self, words: &[&str]) {
        // Complete the word under the cursor (the last one), commands and variables alike.
        let start = self.line.rfind(' ').map_or(0, |i| i + 1);
        let prefix = self.line[start..].to_ascii_lowercase();
        let mut matches: Vec<&str> = words.iter().copied().filter(|w| w.to_ascii_lowercase().starts_with(&prefix)).collect();
        matches.sort_unstable();
        matches.dedup();
        match matches.as_slice() {
            [] => {}
            [one] => {
                self.line.truncate(start);
                self.line.push_str(one);
                self.line.push(' ');
            }
            many => {
                let common = many.iter().skip(1).fold(many[0].to_string(), |acc, w| {
                    acc.chars().zip(w.chars()).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).map(|(a, _)| a).collect()
                });
                if common.len() > prefix.len() {
                    self.line.truncate(start);
                    self.line.push_str(&common);
                } else {
                    self.print(&many.join("  "));
                }
            }
        }
    }

    /// Step through history: `older` true goes back.
    pub fn history(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() - 1;
        self.browsing = match (self.browsing, older) {
            (None, true) => Some(last),
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i < last => Some(i + 1),
            (_, false) => None,
        };
        self.line = self.browsing.map_or_else(String::new, |i| self.history[i].clone());
    }

    /// Take the line for running (echoed and remembered). `None` if it is blank.
    pub fn submit(&mut self) -> Option<String> {
        let line = std::mem::take(&mut self.line);
        self.browsing = None;
        self.scroll = 0;
        if line.trim().is_empty() {
            return None;
        }
        self.print(&format!("> {line}"));
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        Some(line)
    }

    pub fn print(&mut self, text: &str) {
        self.output.extend(text.lines().map(str::to_string));
        let excess = self.output.len().saturating_sub(KEEP);
        self.output.drain(..excess);
    }

    pub fn scroll_by(&mut self, lines: isize) {
        self.scroll = (self.scroll as isize + lines).clamp(0, self.output.len() as isize) as usize;
    }

    /// The `rows` lines to show, scrollback applied.
    pub fn visible(&self, rows: usize) -> &[String] {
        let end = self.output.len().saturating_sub(self.scroll);
        &self.output[end.saturating_sub(rows)..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_history_and_scrollback() {
        let mut c = Console::default();
        let words = ["set", "r_fov", "r_freelook", "warp"];
        c.edit("se\t", &words);
        assert_eq!(c.line, "set ");
        c.edit("r_f\t", &words);
        assert_eq!(c.line, "set r_f", "ambiguous: nothing longer in common");
        assert_eq!(c.output.last().unwrap(), "r_fov  r_freelook");
        c.edit("o\t90", &words);
        assert_eq!(c.line, "set r_fov 90");
        assert_eq!(c.submit().as_deref(), Some("set r_fov 90"));
        c.edit("warp LIPS01", &words);
        c.submit();
        c.history(true);
        assert_eq!(c.line, "warp LIPS01");
        c.history(true);
        assert_eq!(c.line, "set r_fov 90");
        c.history(false);
        c.history(false);
        assert_eq!(c.line, "", "back to a new line");
        for i in 0..20 {
            c.print(&format!("line {i}"));
        }
        assert_eq!(c.visible(2), ["line 18", "line 19"]);
        c.scroll_by(5);
        assert_eq!(c.visible(2), ["line 13", "line 14"]);
        assert!(c.submit().is_none() && c.scroll == 0, "blank lines don't run but reset scroll");
    }
}
