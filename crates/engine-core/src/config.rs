//! Config files and console commands shared by every game: `set`, `get`, `toggle`, `reset`,
//! `cvarlist`, `bind`, `unbind`, `unbindall`. A config file is a list of such lines.

use crate::bind::{BindTable, pretty};
use crate::cvar::{CvarRegistry, Kind, split_words};

/// Outcome of a command line.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Handled; optional text for the console.
    Done(Option<String>),
    /// Not a config command (the game may handle it).
    Unknown,
    Error(String),
}

pub fn exec_line(cvars: &mut CvarRegistry, binds: &mut BindTable, line: &str) -> Outcome {
    let words = split_words(line.split("//").next().unwrap_or(""));
    let Some(cmd) = words.first() else { return Outcome::Done(None) };
    let arg = |i: usize| words.get(i).map(String::as_str);
    let result = match (cmd.to_ascii_lowercase().as_str(), arg(1), arg(2)) {
        ("set" | "seta", Some(name), Some(value)) => cvars.set(name, value).map(|_| None),
        ("get", Some(name), _) => cvars.find(name).map(|v| Some(format!("{} = {}", v.name, v.text()))).ok_or(format!("unknown variable {name}")),
        ("toggle", Some(name), _) => match cvars.find(name).map(|v| &v.kind) {
            Some(Kind::Bool) => {
                let on = !cvars.get_bool(name);
                cvars.set_bool(name, on);
                Ok(Some(format!("{name} = {}", if on { 1 } else { 0 })))
            }
            Some(_) => Err(format!("{name} is not a toggle")),
            None => Err(format!("unknown variable {name}")),
        },
        ("reset", Some(name), _) => cvars.reset(name).map(|_| None),
        ("cvarlist", filter, _) => Ok(Some(
            cvars
                .iter()
                .filter(|v| filter.is_none_or(|f| v.name.contains(f)))
                .map(|v| format!("{} {}", v.name, v.text()))
                .collect::<Vec<_>>()
                .join("\n"),
        )),
        ("bind", Some(source), Some(action)) => {
            binds.bind(source, action);
            Ok(None)
        }
        ("bind", Some(source), None) => Ok(Some(format!("{} = {}", pretty(source), binds.actions(source).collect::<Vec<_>>().join(", ")))),
        ("unbind", Some(source), _) => {
            binds.unbind(source);
            Ok(None)
        }
        ("unbindall", _, _) => {
            binds.clear();
            Ok(None)
        }
        ("set" | "seta" | "get" | "toggle" | "reset" | "bind" | "unbind", _, _) => Err(format!("usage: {cmd} ...")),
        _ => {
            // `name value` / `name` work as shorthands for set/get, as in Quake-style consoles.
            return match (cvars.find(cmd).is_some(), arg(1)) {
                (true, Some(value)) => cvars.set(cmd, value).map_or_else(Outcome::Error, |_| Outcome::Done(None)),
                (true, None) => Outcome::Done(cvars.find(cmd).map(|v| format!("{} = {}", v.name, v.text()))),
                (false, _) => Outcome::Unknown,
            };
        }
    };
    match result {
        Ok(text) => Outcome::Done(text),
        Err(e) => Outcome::Error(e),
    }
}

/// Run a whole config file; returns the errors (with line numbers) it produced.
pub fn exec_config(cvars: &mut CvarRegistry, binds: &mut BindTable, text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| match exec_line(cvars, binds, line) {
            Outcome::Error(e) => Some(format!("line {}: {e}", i + 1)),
            Outcome::Unknown => Some(format!("line {}: unknown command {:?}", i + 1, line.trim())),
            Outcome::Done(_) => None,
        })
        .collect()
}

/// A complete config file for the current state.
pub fn write_config(cvars: &CvarRegistry, binds: &BindTable, header: &str) -> String {
    format!("// {header}\n{}{}", cvars.to_config(), binds.to_config())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cvar::Category;

    #[test]
    fn commands_and_round_trip() {
        let mut c = CvarRegistry::default();
        c.bool("cl_autorun", "Always run", Category::Controls, false);
        c.float("m_sensitivity", "Mouse speed", Category::Controls, (0.1, 20.0, 0.1), 2.5);
        let mut b = BindTable::from_pairs(&[("key:KeyW", "Forward")]);
        assert_eq!(exec_line(&mut c, &mut b, "toggle cl_autorun"), Outcome::Done(Some("cl_autorun = 1".into())));
        assert_eq!(exec_line(&mut c, &mut b, "m_sensitivity 4"), Outcome::Done(None));
        assert_eq!(exec_line(&mut c, &mut b, "m_sensitivity"), Outcome::Done(Some("m_sensitivity = 4".into())));
        assert_eq!(exec_line(&mut c, &mut b, "bind mouse:Left Use // fire"), Outcome::Done(None));
        assert_eq!(exec_line(&mut c, &mut b, "warp LIPS01"), Outcome::Unknown);
        assert!(matches!(exec_line(&mut c, &mut b, "set nope 1"), Outcome::Error(_)));
        let text = write_config(&c, &b, "test");
        let (mut c2, mut b2) = (c.clone(), BindTable::default());
        c2.reset("cl_autorun").unwrap();
        c2.reset("m_sensitivity").unwrap();
        assert!(exec_config(&mut c2, &mut b2, &text).is_empty());
        assert_eq!(write_config(&c2, &b2, "test"), text);
    }
}
