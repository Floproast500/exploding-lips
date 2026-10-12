//! Input bindings: physical sources (by name) mapped to game actions (by name).
//!
//! Source names are front-end independent so one table serves desktop and web:
//! `key:<code>` uses the W3C `KeyboardEvent.code` names (`KeyW`, `ArrowUp`, `ShiftLeft`, ...,
//! which winit's `KeyCode` debug names match), `mouse:Left|Right|Middle|Back|Forward`,
//! `wheel:Up|Down` and `pad:<button>` (gilrs button names: `South`, `RightTrigger2`, ...).

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindTable {
    binds: Vec<(String, String)>,
}

impl BindTable {
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self { binds: pairs.iter().map(|(s, a)| (s.to_string(), a.to_string())).collect() }
    }

    /// Add a binding (a source may drive several actions, an action may have several sources).
    pub fn bind(&mut self, source: &str, action: &str) {
        if !self.binds.iter().any(|(s, a)| s == source && a.eq_ignore_ascii_case(action)) {
            self.binds.push((source.to_string(), action.to_string()));
        }
    }

    pub fn unbind(&mut self, source: &str) {
        self.binds.retain(|(s, _)| s != source);
    }

    pub fn unbind_action(&mut self, action: &str) {
        self.binds.retain(|(_, a)| !a.eq_ignore_ascii_case(action));
    }

    pub fn clear(&mut self) {
        self.binds.clear();
    }

    /// Make `source` the primary binding of `action`: other keyboard/mouse sources of the action
    /// are dropped (pad bindings stay, so rebinding a key never breaks the controller), and
    /// the source stops driving other non-menu actions.
    pub fn rebind(&mut self, action: &str, source: &str, keep: impl Fn(&str) -> bool) {
        let pad = |s: &str| s.starts_with("pad:");
        self.binds.retain(|(s, a)| {
            let same_action = a.eq_ignore_ascii_case(action);
            !(same_action && pad(s) == pad(source)) && !(s == source && !keep(a))
        });
        self.bind(source, action);
    }

    pub fn actions<'a>(&'a self, source: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.binds.iter().filter(move |(s, _)| s == source).map(|(_, a)| a.as_str())
    }

    pub fn sources(&self, action: &str) -> Vec<&str> {
        self.binds.iter().filter(|(_, a)| a.eq_ignore_ascii_case(action)).map(|(s, _)| s.as_str()).collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.binds.iter().map(|(s, a)| (s.as_str(), a.as_str()))
    }

    /// The full table as config lines (`unbindall` first, so a saved config is complete).
    pub fn to_config(&self) -> String {
        let mut out = String::from("unbindall\n");
        for (s, a) in &self.binds {
            out.push_str(&format!("bind {s} {a}\n"));
        }
        out
    }
}

/// Short human name of a source for menus: `key:KeyW` → `W`, `mouse:Left` → `Mouse1`.
pub fn pretty(source: &str) -> String {
    let (kind, name) = source.split_once(':').unwrap_or(("key", source));
    match kind {
        "key" => {
            let n = name
                .strip_prefix("Key")
                .or_else(|| name.strip_prefix("Digit"))
                .or_else(|| name.strip_prefix("Arrow"))
                .unwrap_or(name);
            match n {
                "ShiftLeft" | "ShiftRight" => "Shift".into(),
                "ControlLeft" | "ControlRight" => "Ctrl".into(),
                "AltLeft" | "AltRight" => "Alt".into(),
                "Backquote" => "`".into(),
                "Escape" => "Esc".into(),
                _ if name.starts_with("Numpad") => format!("KP{}", &name[6..]),
                _ => n.into(),
            }
        }
        "mouse" => match name {
            "Left" => "Mouse1".into(),
            "Right" => "Mouse2".into(),
            "Middle" => "Mouse3".into(),
            other => format!("Mouse {other}"),
        },
        "wheel" => format!("Wheel {name}"),
        "pad" => format!("Pad {}", pad_label(name)),
        _ => source.into(),
    }
}

fn pad_label(button: &str) -> &str {
    match button {
        "South" => "A",
        "East" => "B",
        "West" => "X",
        "North" => "Y",
        "LeftTrigger" => "LB",
        "RightTrigger" => "RB",
        "LeftTrigger2" => "LT",
        "RightTrigger2" => "RT",
        "LeftThumb" => "L3",
        "RightThumb" => "R3",
        "DPadUp" => "Up",
        "DPadDown" => "Down",
        "DPadLeft" => "Left",
        "DPadRight" => "Right",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_rebind_and_config() {
        let mut t = BindTable::from_pairs(&[("key:KeyW", "Forward"), ("key:ArrowUp", "Forward"), ("key:ArrowUp", "MenuUp"), ("pad:South", "Jump")]);
        assert_eq!(t.actions("key:ArrowUp").collect::<Vec<_>>(), ["Forward", "MenuUp"]);
        // Rebinding Forward to E drops the other key bindings of Forward but keeps menu actions.
        t.rebind("Forward", "key:KeyE", |a| a.starts_with("Menu"));
        assert_eq!(t.sources("forward"), ["key:KeyE"]);
        assert_eq!(t.actions("key:ArrowUp").collect::<Vec<_>>(), ["MenuUp"]);
        // A pad source for Jump doesn't remove the keyboard one.
        t.bind("key:Space", "Jump");
        t.rebind("Jump", "pad:North", |_| false);
        assert_eq!(t.sources("Jump"), ["key:Space", "pad:North"]);
        let cfg = t.to_config();
        let mut u = BindTable::default();
        for line in cfg.lines().skip(1) {
            let w: Vec<&str> = line.split(' ').collect();
            u.bind(w[1], w[2]);
        }
        assert_eq!(u, t);
    }

    #[test]
    fn pretty_names() {
        assert_eq!(pretty("key:KeyW"), "W");
        assert_eq!(pretty("key:Digit5"), "5");
        assert_eq!(pretty("key:ArrowUp"), "Up");
        assert_eq!(pretty("key:ControlLeft"), "Ctrl");
        assert_eq!(pretty("key:NumpadMultiply"), "KPMultiply");
        assert_eq!(pretty("mouse:Left"), "Mouse1");
        assert_eq!(pretty("wheel:Up"), "Wheel Up");
        assert_eq!(pretty("pad:RightTrigger2"), "Pad RT");
    }
}
