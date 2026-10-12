//! Console variables: typed, ranged settings that games and front ends register, the options
//! menu edits, the console changes (`set name value`) and the config file persists.

use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Video,
    Audio,
    Controls,
    Hud,
    Accessibility,
    Game,
}

impl Category {
    pub const ALL: [Category; 6] = [Self::Video, Self::Audio, Self::Controls, Self::Hud, Self::Accessibility, Self::Game];

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Controls => "Controls",
            Self::Hud => "HUD",
            Self::Accessibility => "Accessibility",
            Self::Game => "Game",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Bool,
    Int { min: i64, max: i64, step: i64 },
    Float { min: f64, max: f64, step: f64 },
    /// One of the listed choices; the value is the index.
    Enum(&'static [&'static str]),
    Str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

/// Saved to the config file.
pub const ARCHIVE: u32 = 1;
/// Not shown in the options menu (console and config only).
pub const HIDDEN: u32 = 2;

#[derive(Debug, Clone)]
pub struct Cvar {
    pub name: &'static str,
    pub label: &'static str,
    pub category: Category,
    pub kind: Kind,
    pub flags: u32,
    pub default: Value,
    pub value: Value,
}

impl Cvar {
    /// Value as text, as written to the config file and shown by `get`.
    pub fn text(&self) -> String {
        match &self.value {
            Value::Bool(b) => (if *b { "1" } else { "0" }).into(),
            Value::Int(i) => match self.kind {
                Kind::Enum(choices) => choices.get(*i as usize).copied().unwrap_or("?").into(),
                _ => i.to_string(),
            },
            Value::Float(f) => {
                let s = format!("{f:.4}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            Value::Str(s) => s.clone(),
        }
    }

    /// Value for the options menu.
    pub fn display(&self) -> String {
        match &self.value {
            Value::Bool(b) => (if *b { "on" } else { "off" }).into(),
            _ => self.text(),
        }
    }

    fn parse(&self, text: &str) -> Option<Value> {
        let t = text.trim();
        Some(match &self.kind {
            Kind::Bool => Value::Bool(match t.to_ascii_lowercase().as_str() {
                "1" | "on" | "true" | "yes" => true,
                "0" | "off" | "false" | "no" => false,
                _ => return None,
            }),
            Kind::Int { min, max, .. } => Value::Int(t.parse::<i64>().ok()?.clamp(*min, *max)),
            Kind::Float { min, max, .. } => Value::Float(t.parse::<f64>().ok().filter(|v| v.is_finite())?.clamp(*min, *max)),
            Kind::Enum(choices) => Value::Int(
                choices.iter().position(|c| c.eq_ignore_ascii_case(t)).or_else(|| t.parse().ok().filter(|&i: &usize| i < choices.len()))? as i64,
            ),
            Kind::Str => Value::Str(t.to_string()),
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct CvarRegistry {
    vars: Vec<Cvar>,
    /// Bumped on every change, so consumers can cheaply notice edits.
    generation: u64,
}

impl CvarRegistry {
    fn define(&mut self, name: &'static str, label: &'static str, category: Category, kind: Kind, flags: u32, default: Value) {
        if let Some(v) = self.vars.iter_mut().find(|v| v.name == name) {
            // Re-registration (e.g. a front end refining a game's definition) keeps the value.
            v.label = label;
            v.category = category;
            v.kind = kind;
            v.flags = flags;
            v.default = default;
            return;
        }
        self.vars.push(Cvar { name, label, category, kind, flags, value: default.clone(), default });
    }

    pub fn bool(&mut self, name: &'static str, label: &'static str, category: Category, default: bool) {
        self.define(name, label, category, Kind::Bool, ARCHIVE, Value::Bool(default));
    }

    pub fn int(&mut self, name: &'static str, label: &'static str, category: Category, (min, max, step): (i64, i64, i64), default: i64) {
        self.define(name, label, category, Kind::Int { min, max, step }, ARCHIVE, Value::Int(default));
    }

    pub fn float(&mut self, name: &'static str, label: &'static str, category: Category, (min, max, step): (f64, f64, f64), default: f64) {
        self.define(name, label, category, Kind::Float { min, max, step }, ARCHIVE, Value::Float(default));
    }

    pub fn choice(&mut self, name: &'static str, label: &'static str, category: Category, choices: &'static [&'static str], default: usize) {
        self.define(name, label, category, Kind::Enum(choices), ARCHIVE, Value::Int(default as i64));
    }

    pub fn string(&mut self, name: &'static str, label: &'static str, category: Category, default: &str) {
        self.define(name, label, category, Kind::Str, ARCHIVE | HIDDEN, Value::Str(default.into()));
    }

    pub fn set_flags(&mut self, name: &str, flags: u32) {
        if let Some(v) = self.find_mut(name) {
            v.flags = flags;
        }
    }

    pub fn find(&self, name: &str) -> Option<&Cvar> {
        self.vars.iter().find(|v| v.name.eq_ignore_ascii_case(name))
    }

    fn find_mut(&mut self, name: &str) -> Option<&mut Cvar> {
        self.vars.iter_mut().find(|v| v.name.eq_ignore_ascii_case(name))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Cvar> {
        self.vars.iter()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get_bool(&self, name: &str) -> bool {
        matches!(self.find(name).map(|v| &v.value), Some(Value::Bool(true)))
    }

    pub fn get_int(&self, name: &str) -> i64 {
        match self.find(name).map(|v| &v.value) {
            Some(Value::Int(i)) => *i,
            Some(Value::Float(f)) => *f as i64,
            Some(Value::Bool(b)) => *b as i64,
            _ => 0,
        }
    }

    pub fn get_float(&self, name: &str) -> f64 {
        match self.find(name).map(|v| &v.value) {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(i)) => *i as f64,
            _ => 0.0,
        }
    }

    pub fn get_str(&self, name: &str) -> String {
        self.find(name).map(|v| v.text()).unwrap_or_default()
    }

    /// Set from text (console, config file). Values are clamped to the range.
    pub fn set(&mut self, name: &str, text: &str) -> Result<(), String> {
        let v = self.find_mut(name).ok_or_else(|| format!("unknown variable {name}"))?;
        let value = v.parse(text).ok_or_else(|| format!("bad value {text:?} for {}", v.name))?;
        if v.value != value {
            v.value = value;
            self.generation += 1;
        }
        Ok(())
    }

    pub fn set_bool(&mut self, name: &str, value: bool) {
        let _ = self.set(name, if value { "1" } else { "0" });
    }

    pub fn set_float(&mut self, name: &str, value: f64) {
        let _ = self.set(name, &value.to_string());
    }

    pub fn reset(&mut self, name: &str) -> Result<(), String> {
        let v = self.find_mut(name).ok_or_else(|| format!("unknown variable {name}"))?;
        if v.value != v.default {
            v.value = v.default.clone();
            self.generation += 1;
        }
        Ok(())
    }

    /// One step left (`dir < 0`) or right: toggles booleans, cycles choices, moves numbers.
    pub fn step(&mut self, name: &str, dir: i32) {
        let Some(v) = self.find_mut(name) else { return };
        let d = dir.signum() as i64;
        let value = match (&v.kind, &v.value) {
            (Kind::Bool, Value::Bool(b)) => Value::Bool(!b),
            (Kind::Int { min, max, step }, Value::Int(i)) => Value::Int((i + d * step).clamp(*min, *max)),
            (Kind::Float { min, max, step }, Value::Float(f)) => {
                // Snap to the step grid so repeated steps don't accumulate float noise.
                let n = ((f - min) / step).round() + d as f64;
                Value::Float((min + n * step).clamp(*min, *max))
            }
            (Kind::Enum(choices), Value::Int(i)) => Value::Int((i + d).rem_euclid(choices.len() as i64)),
            _ => return,
        };
        if v.value != value {
            v.value = value;
            self.generation += 1;
        }
    }

    /// `set` lines for every archived variable that differs from its default.
    pub fn to_config(&self) -> String {
        let mut out = String::new();
        for v in self.vars.iter().filter(|v| v.flags & ARCHIVE != 0 && v.value != v.default) {
            let text = v.text();
            if text.contains(char::is_whitespace) || text.is_empty() {
                let _ = writeln!(out, "set {} \"{}\"", v.name, text);
            } else {
                let _ = writeln!(out, "set {} {}", v.name, text);
            }
        }
        out
    }
}

/// Split a config/console line into words; double quotes group words.
pub fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(cur);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> CvarRegistry {
        let mut r = CvarRegistry::default();
        r.bool("vid_vsync", "VSync", Category::Video, true);
        r.float("fov", "Field of view", Category::Video, (30.0, 110.0, 2.0), 48.9);
        r.choice("crosshair", "Crosshair", Category::Hud, &["off", "dot", "cross"], 0);
        r.int("hud_scale", "HUD scale", Category::Hud, (0, 8, 1), 0);
        r.string("snd_soundfont", "SoundFont", Category::Audio, "");
        r
    }

    #[test]
    fn set_get_clamp_and_round_trip() {
        let mut r = reg();
        r.set("fov", "200").unwrap();
        assert_eq!(r.get_float("fov"), 110.0, "clamped");
        r.set("CROSSHAIR", "cross").unwrap();
        assert_eq!(r.get_int("crosshair"), 2);
        r.set("vid_vsync", "off").unwrap();
        r.set("snd_soundfont", "/usr/share/soundfonts/My Font.sf2").unwrap();
        assert!(r.set("nope", "1").is_err());
        assert!(r.set("hud_scale", "big").is_err());
        let cfg = r.to_config();
        assert!(cfg.contains("set snd_soundfont \"/usr/share/soundfonts/My Font.sf2\""), "{cfg}");
        let mut again = reg();
        for line in cfg.lines() {
            let w = split_words(line);
            again.set(&w[1], &w[2]).unwrap();
        }
        assert_eq!(again.to_config(), cfg);
        assert!(!again.get_bool("vid_vsync"));
    }

    #[test]
    fn step_cycles_and_snaps() {
        let mut r = reg();
        let g = r.generation();
        r.step("crosshair", -1);
        assert_eq!(r.get_str("crosshair"), "cross", "wraps");
        r.step("fov", 1);
        assert!((r.get_float("fov") - 50.0).abs() < 1e-9, "snapped to the 2-degree grid from 30");
        r.step("vid_vsync", 1);
        assert!(!r.get_bool("vid_vsync"));
        assert!(r.generation() > g);
        r.reset("fov").unwrap();
        assert_eq!(r.get_float("fov"), 48.9);
        assert_eq!(r.to_config().lines().count(), 2, "only changed values are written");
    }

    #[test]
    fn words_respect_quotes() {
        assert_eq!(split_words(r#"bind key:KeyW "Forward"  "#), ["bind", "key:KeyW", "Forward"]);
        assert_eq!(split_words(r#"set x "" "#), ["set", "x", ""]);
    }
}
