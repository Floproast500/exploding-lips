//! The INI command language: tags, ONTAG bodies, object-state ("OS") values with threshold
//! tags, messages and sounds. Behaviour follows re/engine/script.md, including the original's
//! quirks (lowercase letters outside quotes are deleted, commas inside messages become spaces,
//! an unknown command silently drops the rest of its line).

use std::collections::HashMap;

/// Number of object-state values; 1..=5 are Life, Magic, Armor, Money, Experience.
pub const STATE_COUNT: usize = 16;
pub const LIFE: usize = 1;
pub const MAGIC: usize = 2;
pub const ARMOR: usize = 3;
pub const MONEY: usize = 4;
pub const EXPERIENCE: usize = 5;
/// Lips re-uses these as treasures found / deaths / enemies killed.
pub const TREASURES: usize = 10;
pub const DEATHS: usize = 11;
pub const KILLS: usize = 12;

/// Default status-message colour (palette index 92).
pub const DEFAULT_MESSAGE_COLOR: u8 = 92;
/// Nested tag activations deeper than this are ignored.
const MAX_TAG_DEPTH: u32 = 12;
/// Lines are truncated to this many characters.
const MAX_LINE: usize = 198;
/// Largest tag that can carry an ONTAG body.
const MAX_ONTAG: u32 = 998;
pub const STARTUP_TAG: u32 = 9999;

/// Every command word the engine knows (table at 0x0050FA48). Words not listed abandon the
/// rest of the line; listed words without an implementation are accepted and ignored.
const KNOWN_COMMANDS: &[&str] = &[
    "FLYON", "FLYOFF", "GOTO", "PLAY", "STOP", "PAUSE", "SOUNDON", "SOUNDOFF", "PHYSICSON", "PHYSICSOFF", "SET", "TRACE",
    "TAG", "WATCH", "DOORBELL", "SHOW", "INVERTMOUSE", "SYSINFO", "CHECKWDIST", "NEW", "SPEAK", "JOIN", "NAME", "QUIT", "LOG",
    "MAIL", "MAILBUG", "MAILCOMMENT", "MAILTO", "MAILFROM", "MAILSUBJECT", "KICKBAN", "GAMEPASSWORD", "NETTAG", "KEYMAP",
    "KEYBIND", "EXECUTE", "OPEN", "OSETJUMP", "OSETCROU", "OSETZSPD", "OSETFLY", "OPRSTON", "ODRSTON", "OPHYSON", "OSETPOS",
    "OSETSPD", "OMAXOS", "OSETOS", "OINCOS", "OINCTOOS", "OENGINE", "OSETGEN", "OSETVIEW", "OSETVIS", "OSTEPON", "OSETKEYS",
    "RPORTAL", "LSETTINT", "LSETFADE", "WEBCHATNEW", "WEBCHATOFF", "SETMINOS", "SETMINOSTAGS", "SETMAXOS", "SETMAXOSTAGS",
    "SETLOWOS", "SETLOWOSTAGS", "SETHIGHOS", "SETHIGHOSTAGS", "ONTAG", "OTEXT", "OSOUND", "OSETMOVE", "INITMOVES",
    "INITPARTSETS", "INCOS", "SUMOS", "INCTOOS", "SETOS", "XCHANGE", "INITTOOL", "TOOLSOUT", "OPTOOL", "OTOOLCON", "OPPROJ",
    "OPXPLD", "OOTOOL", "OOPROJ", "OOXPLD", "AVAILTOOLS", "OPPONENT", "XM", "XT", "NOSEEUM", "STATEVAL", "WTX", "WALLTX",
    "WTY", "WALLTY", "WFADE", "WALLFADE", "WTEX", "WALLTEX", "FTEX", "FFADE", "CTEX", "CFADE",
];

/// Effects of running script commands that the game must act on.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptEvent {
    /// Broadcast the tag to world objects/walls/regions listening on it.
    WorldTag(u32),
    /// Status-line message in palette colour `color`.
    Message { color: u8, text: String },
    /// Text overlay (OTEXT).
    Text(String),
    /// Play a sound (or trigger the sound object of that name).
    Sound(String),
    /// Attach the mover script `script` to the object `object`.
    InitMover { object: String, script: String },
    /// Particle set for the named object.
    InitParticles(String),
    /// Tool / projectile / explosion definition: (command, index, raw args).
    ToolDef { kind: String, index: usize, args: String },
    /// Tool resource control (OTOOLCON[n]).
    ToolCost { index: usize, args: String },
    Quit,
}

/// A threshold tag: `lo`, or a random tag in `lo..=hi` when `hi > lo`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TagRange {
    pub lo: u32,
    pub hi: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Thresholds {
    pub max: [i32; STATE_COUNT],
    pub min: [i32; STATE_COUNT],
    pub low: [i32; STATE_COUNT],
    pub high: [i32; STATE_COUNT],
    pub max_tags: [TagRange; STATE_COUNT],
    pub min_tags: [TagRange; STATE_COUNT],
    pub low_tags: [TagRange; STATE_COUNT],
    pub high_tags: [TagRange; STATE_COUNT],
}

#[derive(Debug, Clone)]
pub struct Script {
    ontag: HashMap<u32, String>,
    pub states: [i32; STATE_COUNT],
    pub thresholds: Thresholds,
    /// Bitmask of selectable tools (AVAILTOOLS).
    pub available_tools: u32,
    /// OSETJUMP integers (launch etc.) and sound names.
    pub jump: Option<([i32; 5], [String; 4])>,
    /// Inventory item names (up to 64).
    pub inventory: Vec<String>,
    events: Vec<ScriptEvent>,
    depth: u32,
    rng: u64,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            ontag: HashMap::new(),
            states: [0; STATE_COUNT],
            thresholds: Thresholds::default(),
            available_tools: 0,
            jump: None,
            inventory: Vec::new(),
            events: Vec::new(),
            depth: 0,
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }
}

impl Script {
    pub fn new(seed: u64) -> Self {
        Self { rng: seed | 1, ..Self::default() }
    }

    /// Events produced since the last call.
    pub fn take_events(&mut self) -> Vec<ScriptEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn has_ontag(&self, tag: u32) -> bool {
        self.ontag.contains_key(&tag)
    }

    /// Run every command line of an INI section (raw file text, see [`section_lines`]).
    pub fn run_section(&mut self, ini_text: &str, section: &str) {
        for line in section_lines(ini_text, section) {
            self.run_line(&line);
        }
    }

    /// Fire a tag: run its ONTAG body, then broadcast it to the world.
    pub fn activate_tag(&mut self, tag: u32) {
        if tag == 0 || self.depth >= MAX_TAG_DEPTH {
            return;
        }
        self.depth += 1;
        if (1..=MAX_ONTAG).contains(&tag)
            && let Some(body) = self.ontag.get(&tag).cloned() {
                self.run_line(&body);
            }
        self.events.push(ScriptEvent::WorldTag(tag));
        self.depth -= 1;
    }

    fn fire_range(&mut self, range: TagRange) {
        if range.lo == 0 {
            return;
        }
        let tag = if range.hi > range.lo { range.lo + (self.random() % (range.hi - range.lo + 1) as u64) as u32 } else { range.lo };
        self.activate_tag(tag);
    }

    fn random(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Run one command line (already joined; pre-processing is applied here).
    pub fn run_line(&mut self, raw: &str) {
        let line = preprocess(raw);
        let line: String = line.chars().take(MAX_LINE).collect();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && is_delimiter(chars[i]) && chars[i] != '"' {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let start = i;
            while i < chars.len() && !is_delimiter(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect::<String>().to_ascii_uppercase();
            let (name, index) = match word.split_once('[') {
                Some((n, rest)) => (n.to_string(), rest.trim_end_matches(']').trim().parse::<usize>().ok()),
                None => (word.clone(), None),
            };
            if word.is_empty() || !KNOWN_COMMANDS.contains(&name.as_str()) {
                // Unknown command: the rest of the line is abandoned.
                return;
            }
            // Argument: skip blanks and one '='.
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if i < chars.len() && chars[i] == '=' {
                i += 1;
            }
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            let (arg, next) = if i < chars.len() && (chars[i] == '(' || chars[i] == '{') {
                let end = matching_bracket(&chars, i);
                (chars[i..end.min(chars.len())].iter().collect::<String>(), end + 1)
            } else {
                let mut rest: String = chars[i..].iter().collect();
                if rest.ends_with(')') || rest.ends_with('}') {
                    rest.pop();
                }
                (rest, chars.len())
            };
            self.execute(&name, index, &arg);
            i = next;
        }
    }

    fn execute(&mut self, name: &str, index: Option<usize>, raw_arg: &str) {
        // Most handlers normalise their argument (strip '=', '(', commas → spaces).
        let arg = normalize_argument(raw_arg);
        match name {
            "TAG" => {
                if let Some(t) = first_int(&arg) {
                    self.activate_tag(t.max(0) as u32);
                }
            }
            "XT" => {
                let range = parse_tag_range(&arg);
                if range.lo > 0 {
                    // b ≤ a in a range gives tag 1 in the original; keep a for single tags.
                    self.fire_range(range);
                }
            }
            "ONTAG" => {
                if let Some(tag) = index.filter(|&t| t >= 1 && t <= MAX_ONTAG as usize) {
                    self.ontag.insert(tag as u32, raw_arg.trim().to_string());
                }
            }
            "XM" => {
                let (color, text) = parse_message(&arg);
                self.events.push(ScriptEvent::Message { color, text });
            }
            "OTEXT" => {
                let text = quoted(&arg).unwrap_or_else(|| arg.trim().to_string());
                self.events.push(ScriptEvent::Text(text));
            }
            "OSOUND" => {
                if let Some(sound) = arg.split_whitespace().next() {
                    self.events.push(ScriptEvent::Sound(sound.to_string()));
                }
            }
            "SETOS" | "OSETOS" => {
                for (state, value) in self.parse_states(&arg) {
                    self.states[state] = value;
                }
            }
            "INCOS" | "OINCOS" => {
                for (state, value) in self.parse_states(&arg) {
                    self.change_state(state, value);
                }
            }
            "INCTOOS" | "OINCTOOS" => {
                for (state, value) in self.parse_states(&arg) {
                    if value > 0 && self.states[state] < value {
                        self.states[state] = value;
                    }
                }
            }
            "SUMOS" => self.sum_states(&arg),
            "SETMAXOS" | "OMAXOS" => self.set_table(&arg, |t| &mut t.max),
            "SETMINOS" => self.set_table(&arg, |t| &mut t.min),
            "SETLOWOS" => self.set_table(&arg, |t| &mut t.low),
            "SETHIGHOS" => self.set_table(&arg, |t| &mut t.high),
            "SETMAXOSTAGS" => self.set_tags(raw_arg, |t| &mut t.max_tags),
            "SETMINOSTAGS" => self.set_tags(raw_arg, |t| &mut t.min_tags),
            "SETLOWOSTAGS" => self.set_tags(raw_arg, |t| &mut t.low_tags),
            "SETHIGHOSTAGS" => self.set_tags(raw_arg, |t| &mut t.high_tags),
            "STATEVAL" => self.state_value(&arg),
            "XCHANGE" => self.exchange(raw_arg),
            "AVAILTOOLS" => {
                if let Ok(mask) = u32::from_str_radix(arg.trim(), 16) {
                    self.available_tools = mask;
                }
            }
            "OSETJUMP" => {
                let parts: Vec<&str> = arg.split_whitespace().collect();
                let mut ints = [0i32; 5];
                for (slot, p) in ints.iter_mut().zip(&parts) {
                    *slot = p.parse().unwrap_or(-1);
                }
                let sound = |k: usize| parts.get(5 + k).copied().filter(|s| *s != "-1").unwrap_or_default().to_string();
                self.jump = Some((ints, [sound(0), sound(1), sound(2), sound(3)]));
            }
            "INITMOVES" => {
                for object in arg.split_whitespace() {
                    self.events.push(ScriptEvent::InitMover { object: object.to_string(), script: object.to_string() });
                }
            }
            "INITPARTSETS" => {
                for object in arg.split_whitespace() {
                    self.events.push(ScriptEvent::InitParticles(object.to_string()));
                }
            }
            "OPTOOL" | "OPPROJ" | "OPXPLD" | "OOTOOL" | "OOPROJ" | "OOXPLD" => {
                self.events.push(ScriptEvent::ToolDef { kind: name.to_string(), index: index.unwrap_or(0), args: raw_arg.to_string() });
            }
            "OTOOLCON" => self.events.push(ScriptEvent::ToolCost { index: index.unwrap_or(0), args: arg }),
            "QUIT" => self.events.push(ScriptEvent::Quit),
            _ => {} // accepted, not used by Lips
        }
    }

    /// Parse an OS list (`L=100 MA=0 A=50 MO=100 E=0`, or positional `100 0 50 100 0`).
    /// Only the first letter (and second for M) of a name counts; another initial stops the
    /// list. A leading `P…`/`O…` target word (player/opponent) is skipped.
    fn parse_states(&self, arg: &str) -> Vec<(usize, i32)> {
        let text = arg.trim();
        let mut out = Vec::new();
        if !text.contains(|c: char| c.is_ascii_alphabetic()) {
            for (i, v) in text.split_whitespace().filter_map(|v| v.parse::<i32>().ok()).take(5).enumerate() {
                out.push((i + 1, v));
            }
            return out;
        }
        let mut tokens = text.split([' ', ',']).filter(|t| !t.is_empty()).peekable();
        if let Some(first) = tokens.peek()
            && !first.contains('=') && (first.starts_with('P') || first.starts_with('O')) && text.contains('=') {
                tokens.next();
            }
        for token in tokens {
            let (name, value) = match token.split_once('=') {
                Some((n, v)) => (n.trim(), v),
                None => (token.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-'), token.trim_start_matches(|c: char| c.is_ascii_alphabetic())),
            };
            let Some(state) = state_index(name) else { break };
            let digits: String = value.chars().skip_while(|c| !(c.is_ascii_digit() || *c == '-')).take_while(|c| c.is_ascii_digit() || *c == '-').collect();
            if let Ok(v) = digits.parse() {
                out.push((state, v));
            }
        }
        out
    }

    /// Threshold logic for a player state change (FUN_0044b9ee).
    pub fn change_state(&mut self, i: usize, delta: i32) {
        let t = &self.thresholds;
        let cur = self.states[i];
        let mut fire = None;
        if delta < 0 {
            if cur <= t.min[i] {
                return;
            }
            if cur + delta <= t.min[i] {
                self.states[i] = t.min[i];
                fire = Some(t.min_tags[i]);
            } else {
                if cur > t.low[i] && cur + delta <= t.low[i] {
                    fire = Some(t.low_tags[i]);
                }
                self.states[i] = cur + delta;
            }
        } else {
            if cur >= t.max[i] {
                return;
            }
            if cur + delta >= t.max[i] {
                self.states[i] = t.max[i];
                fire = Some(t.max_tags[i]);
            } else {
                if cur < t.high[i] && cur + delta >= t.high[i] {
                    fire = Some(t.high_tags[i]);
                }
                self.states[i] = cur + delta;
            }
        }
        if let Some(range) = fire {
            self.fire_range(range);
        }
    }

    /// SUMOS(amount, A, L): spread one amount over the listed states in order. Damage is taken
    /// from each state up to its current value; a positive amount lands on Life.
    fn sum_states(&mut self, arg: &str) {
        let mut parts = arg.split_whitespace();
        let Some(amount) = parts.next().and_then(|v| v.parse::<i32>().ok()) else { return };
        let order: Vec<usize> = parts.map_while(state_index).collect();
        let mut deltas = [0i32; STATE_COUNT];
        if amount >= 0 {
            deltas[LIFE] = amount;
        } else {
            let mut remaining = -amount;
            for &s in &order {
                if remaining == 0 {
                    break;
                }
                let take = remaining.min(self.states[s].max(0));
                let take = if s == *order.last().unwrap() { remaining } else { take };
                deltas[s] -= take;
                remaining -= take;
            }
        }
        for (s, d) in deltas.iter().enumerate() {
            if *d != 0 {
                self.change_state(s, *d);
            }
        }
    }

    fn set_table(&mut self, arg: &str, table: impl Fn(&mut Thresholds) -> &mut [i32; STATE_COUNT]) {
        for (state, value) in self.parse_states(arg) {
            table(&mut self.thresholds)[state] = value;
        }
    }

    fn set_tags(&mut self, raw_arg: &str, table: impl Fn(&mut Thresholds) -> &mut [TagRange; STATE_COUNT]) {
        let text = raw_arg.trim().trim_start_matches('(');
        let text = text.strip_suffix(')').unwrap_or(text);
        for part in split_top_level(text) {
            let Some((name, value)) = part.split_once('=') else { continue };
            let Some(state) = state_index(name.trim()) else { break };
            let range = parse_tag_range(value);
            if range.lo > 0 {
                table(&mut self.thresholds)[state] = range;
            }
        }
    }

    /// STATEVAL(index op value) on any of the 16 states, raw.
    fn state_value(&mut self, arg: &str) {
        let compact: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
        let Some(pos) = compact.find(['=', '+', '-', '*', '/']) else { return };
        let (Ok(index), Ok(value)) = (compact[..pos].parse::<usize>(), compact[pos + 1..].parse::<i32>()) else { return };
        let Some(state) = self.states.get_mut(index) else { return };
        match &compact[pos..pos + 1] {
            "=" => *state = value,
            "+" => *state += value,
            "-" => *state -= value,
            "*" => *state *= value,
            "/" if value != 0 => *state /= value,
            _ => {}
        }
    }

    /// XCHANGE(give=amount, failTag, get=amount, okTag[, fullTag]).
    fn exchange(&mut self, raw_arg: &str) {
        let text = raw_arg.trim().trim_start_matches('(').trim_end_matches(')');
        let parts: Vec<String> = split_top_level(text).iter().map(|s| s.trim().to_string()).collect();
        let pair = |i: usize| -> Option<(String, i32)> {
            let (k, v) = parts.get(i)?.split_once('=')?;
            Some((k.trim().to_string(), v.trim().parse().ok()?))
        };
        let tag = |i: usize| parts.get(i).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let (Some((give, give_amount)), Some((get, get_amount))) = (pair(0), pair(2)) else { return };
        let (fail_tag, ok_tag, full_tag) = (tag(1), tag(3), tag(4));
        let message = |s: &mut Self, tag: u32, text: String| {
            if tag > 0 {
                s.activate_tag(tag);
            } else {
                s.events.push(ScriptEvent::Message { color: DEFAULT_MESSAGE_COLOR, text });
            }
        };
        let get_state = full_state_name(&get);
        match full_state_name(&give) {
            Some(gs) => {
                if self.states[gs] < give_amount {
                    return message(self, fail_tag, format!("Sorry, you need more {give}"));
                }
                if let Some(rs) = get_state
                    && self.states[rs] >= self.thresholds.max[rs] {
                        return message(self, full_tag, format!("You can't carry any more {get}"));
                    }
                self.states[gs] -= give_amount;
            }
            None => {
                let have = self.inventory.iter().filter(|i| i.eq_ignore_ascii_case(&give)).count() as i32;
                if have < give_amount {
                    return message(self, fail_tag, format!("Sorry, you need more {give}"));
                }
                for _ in 0..give_amount {
                    if let Some(pos) = self.inventory.iter().position(|i| i.eq_ignore_ascii_case(&give)) {
                        self.inventory.remove(pos);
                    }
                }
            }
        }
        if let Some(rs) = get_state {
            self.change_state(rs, get_amount);
        }
        message(self, ok_tag, "Thank you, come again!".to_string());
    }
}

/// State index from an OS-list name: only the first letter (and second for M) counts.
fn state_index(name: &str) -> Option<usize> {
    let upper = name.trim().to_ascii_uppercase();
    let mut chars = upper.chars();
    match (chars.next()?, chars.next()) {
        ('L', _) => Some(LIFE),
        ('A', _) => Some(ARMOR),
        ('E', _) => Some(EXPERIENCE),
        ('M', Some('A')) => Some(MAGIC),
        ('M', Some('O')) => Some(MONEY),
        _ => None,
    }
}

/// XCHANGE only treats complete state names as states.
fn full_state_name(name: &str) -> Option<usize> {
    match name.to_ascii_uppercase().as_str() {
        "LIFE" => Some(LIFE),
        "MAGIC" => Some(MAGIC),
        "ARMOR" => Some(ARMOR),
        "MONEY" => Some(MONEY),
        "EXPERIENCE" => Some(EXPERIENCE),
        _ => None,
    }
}

fn is_delimiter(c: char) -> bool {
    matches!(c, ' ' | ',' | ';' | '?' | '(' | '{' | ')' | '}' | '=' | '"' | '\r' | '\n')
}

fn matching_bracket(chars: &[char], open: usize) -> usize {
    let (mut depth, mut in_quote) = (0usize, false);
    for (j, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '"' => in_quote = !in_quote,
            '(' | '{' if !in_quote => depth += 1,
            ')' | '}' if !in_quote => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    chars.len()
}

/// Engine pre-processing: delete every lowercase a–z outside double quotes.
pub fn preprocess(line: &str) -> String {
    let mut in_quote = false;
    line.chars()
        .filter(|&c| {
            if c == '"' {
                in_quote = !in_quote;
            }
            in_quote || !c.is_ascii_lowercase()
        })
        .collect()
}

/// FUN_0040b706: skip blanks, one '=', one '(' or '{', blanks; every ',' after the first
/// character becomes a space (quoted text included).
fn normalize_argument(arg: &str) -> String {
    let mut s = arg.trim_start();
    s = s.strip_prefix('=').unwrap_or(s).trim_start();
    s = s.strip_prefix('(').or_else(|| s.strip_prefix('{')).unwrap_or(s).trim_start();
    let s = s.strip_suffix(')').or_else(|| s.strip_suffix('}')).unwrap_or(s);
    s.char_indices().map(|(i, c)| if c == ',' && i > 0 { ' ' } else { c }).collect()
}

fn first_int(arg: &str) -> Option<i32> {
    arg.split(|c: char| !(c.is_ascii_digit() || c == '-')).find(|t| !t.is_empty()).and_then(|t| t.parse().ok())
}

/// `n`, `(n)` or `a-b`.
fn parse_tag_range(text: &str) -> TagRange {
    let t = text.trim().trim_start_matches('(').trim_end_matches(')').trim();
    match t.split_once('-') {
        Some((a, b)) if !a.trim().is_empty() => {
            let lo = a.trim().parse().unwrap_or(0);
            let hi = b.trim().parse().unwrap_or(0);
            TagRange { lo, hi: hi.max(lo) }
        }
        _ => {
            let v = t.parse::<i32>().unwrap_or(0).max(0) as u32;
            TagRange { lo: v, hi: v }
        }
    }
}

fn quoted(arg: &str) -> Option<String> {
    let start = arg.find('"')?;
    let rest = &arg[start + 1..];
    Some(rest[..rest.find('"').unwrap_or(rest.len())].to_string())
}

/// `XM(199, "TEXT")` → (199, TEXT); `XM("TEXT")` → (default colour, TEXT).
fn parse_message(arg: &str) -> (u8, String) {
    let text = quoted(arg).unwrap_or_default();
    let before_quote = arg.split('"').next().unwrap_or("");
    let color = first_int(before_quote).filter(|&c| c > 0 && c < 256).map_or(DEFAULT_MESSAGE_COLOR, |c| c as u8);
    (color, text)
}

/// Split on commas that are not inside parentheses or quotes.
fn split_top_level(text: &str) -> Vec<String> {
    let (mut parts, mut current, mut depth, mut in_quote) = (Vec::new(), String::new(), 0i32, false);
    for c in text.chars() {
        match c {
            '"' => in_quote = !in_quote,
            '(' if !in_quote => depth += 1,
            ')' if !in_quote => depth -= 1,
            ',' if !in_quote && depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Logical command lines of an INI section, as the engine reads them: `;` comments (outside
/// quotes) removed, `\` continues a line, and line breaks inside `{ }` are joined.
pub fn section_lines(text: &str, section: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_section = false;
    let mut pending = String::new();
    let mut depth = 0i32;
    for raw in text.lines() {
        let line = strip_comment(raw.trim_end_matches('\r')).trim();
        if pending.is_empty() && line.starts_with('[') {
            let name = line.trim_start_matches('[').split(']').next().unwrap_or("").trim();
            in_section = name.eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section {
            continue;
        }
        let (body, continued) = match line.strip_suffix('\\') {
            Some(b) => (b.trim_end(), true),
            None => (line, false),
        };
        if !pending.is_empty() && !body.is_empty() {
            pending.push(' ');
        }
        pending.push_str(body);
        let mut in_quote = false;
        for c in body.chars() {
            match c {
                '"' => in_quote = !in_quote,
                '{' if !in_quote => depth += 1,
                '}' if !in_quote => depth -= 1,
                _ => {}
            }
        }
        if continued || depth > 0 {
            continue;
        }
        depth = 0;
        if !pending.trim().is_empty() {
            lines.push(std::mem::take(&mut pending));
        }
        pending.clear();
    }
    if !pending.trim().is_empty() {
        lines.push(pending);
    }
    lines
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ';' if !in_quote => return &line[..i],
            _ => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVEL: &str = "[commands9999]\r\n\
SETMAXOS=(LIFE=100,MAGIC=100,ARMOR=100,MONEY=999999,Experience=999999)\r\n\
SETMINOS=(LIFE=0,MAGIC=0,ARMOR=0,MONEY=0,Experience=0)\r\n\
SETMINOSTAGS=(LIFE=661,MAGIC=662,ARMOR=663,MONEY=664,Experience=665)\r\n\
SETLOWOS=(LIFE=25,MAGIC=30,ARMOR=10,MONEY=0)\r\n\
SETLOWOSTAGS=(LIFE=671,MAGIC=(672),ARMOR=(673),MONEY=680)\r\n\
; comment\r\n\
ONTAG[401]={ OSOUND(DROPE01) INCOS(LIFE=5)   }\r\n\
ONTAG[411]={ OSOUND(GROWL02) SUMOS(-5,Armor,Life)  XM(199, \"GO HOME!!\") }\r\n\
ONTAG[261]= { OS0UND(GORE001) }\r\n\
ONTAG[671]={ XM(\"Careful, your LIFE is getting low!\") }\r\n\
ONTAG[661]={ STATEVAL(11 + 1 ) INCTOOS(LIFE=100,MAGIC=0,ARMOR=50,MONEY=0,Experience=0) }\r\n\
[nostrap9999]\r\n\
SETOS=(LIFE=100,MAGIC=0,ARMOR=50,MONEY=100,Experience=0)\r\n";

    fn level() -> Script {
        let mut s = Script::new(1);
        s.run_section(LEVEL, "nostrap9999");
        s.run_section(LEVEL, "commands9999");
        s.take_events();
        s
    }

    #[test]
    fn preprocessing_deletes_lowercase_outside_quotes() {
        assert_eq!(preprocess(r#"SUMOS(-5,Armor,Life) XM("Hi there")"#), r#"SUMOS(-5,A,L) XM("Hi there")"#);
    }

    #[test]
    fn startup_sets_states() {
        let s = level();
        assert_eq!(&s.states[1..=5], &[100, 0, 50, 100, 0]);
        assert_eq!(s.thresholds.max[MONEY], 999_999);
        assert_eq!(s.thresholds.low_tags[MAGIC], TagRange { lo: 672, hi: 672 });
    }

    #[test]
    fn pickup_clamps_at_max_and_plays_sound() {
        let mut s = level();
        s.activate_tag(401);
        assert_eq!(s.states[LIFE], 100, "already at max: no change");
        s.states[LIFE] = 50;
        s.activate_tag(401);
        assert_eq!(s.states[LIFE], 55);
        assert!(s.take_events().contains(&ScriptEvent::Sound("DROPE01".into())));
    }

    #[test]
    fn damage_hits_armor_first_and_shows_message() {
        let mut s = level();
        s.activate_tag(411);
        assert_eq!((s.states[ARMOR], s.states[LIFE]), (45, 100));
        let events = s.take_events();
        assert!(events.contains(&ScriptEvent::Message { color: 199, text: "GO HOME!!".into() }));
    }

    #[test]
    fn armor_exhausted_damage_reaches_life_and_low_warning_fires() {
        let mut s = level();
        s.states[ARMOR] = 2;
        s.states[LIFE] = 30;
        s.run_line("SUMOS(-10,A,L)");
        assert_eq!((s.states[ARMOR], s.states[LIFE]), (0, 22));
        let events = s.take_events();
        // Low LIFE warning (tag 671) — commas in messages become spaces, as in the original.
        assert!(events.contains(&ScriptEvent::Message { color: DEFAULT_MESSAGE_COLOR, text: "Careful  your LIFE is getting low!".into() }));
        assert!(events.contains(&ScriptEvent::WorldTag(671)));
    }

    #[test]
    fn death_tag_restores_and_counts() {
        let mut s = level();
        s.states[LIFE] = 3;
        s.run_line("INCOS(LIFE=-10)");
        assert_eq!(s.states[DEATHS], 1, "STATEVAL(11 + 1) from ONTAG[661]");
        assert_eq!(s.states[LIFE], 100, "INCTOOS raises Life back");
    }

    #[test]
    fn unknown_command_abandons_line() {
        let mut s = level();
        s.activate_tag(261);
        assert!(!s.take_events().iter().any(|e| matches!(e, ScriptEvent::Sound(_))), "OS0UND typo never plays");
    }

    #[test]
    fn exchange_trades_states() {
        let mut s = level();
        s.states[MONEY] = 30;
        s.states[LIFE] = 50;
        s.run_line("XCHANGE(MONEY=25,501,LIFE=20,502)");
        assert_eq!((s.states[MONEY], s.states[LIFE]), (5, 70));
        s.run_line("XCHANGE(MONEY=25,501,LIFE=20,502)");
        assert_eq!(s.states[MONEY], 5, "not enough money");
        assert!(s.take_events().contains(&ScriptEvent::WorldTag(501)));
    }

    #[test]
    fn random_tag_range_stays_in_bounds() {
        let mut s = level();
        for _ in 0..50 {
            s.run_line("XT(250-258)");
        }
        for e in s.take_events() {
            if let ScriptEvent::WorldTag(t) = e {
                assert!((250..=258).contains(&t));
            }
        }
    }

    #[test]
    fn multi_line_brace_blocks_join() {
        let text = "[c]\nONTAG[5]={ XM(\"a\")\n  XM(\"b\") }\nTAG=5\n";
        let mut s = Script::new(1);
        s.run_section(text, "c");
        let msgs: Vec<_> = s.take_events().into_iter().filter(|e| matches!(e, ScriptEvent::Message { .. })).collect();
        assert_eq!(msgs.len(), 2);
    }
}
