//! G.O.D.D. level scripts (`LIPSnn.INI`): INI-style sections whose values can hold command
//! blocks such as `ONTAG[401]={ OSOUND(DROPE01) INCOS(LIFE=5) }`.
//!
//! Syntax notes (from the shipped files):
//! - `;` starts a comment outside quotes; a trailing `\` continues the line.
//! - Keys may carry an index: `ONTAG[401]`, `OPTOOL[0]`.
//! - Quoted values are often left unclosed (`m1title="  -= Game Menu =-`).
//! - Command calls may have a space before the parenthesis (`XT (250-258)`).

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub index: Option<String>,
    pub value: String,
    /// 1-based line number of the entry's first line.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub name: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ini {
    pub sections: Vec<Section>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::default();
        let mut pending: Option<(String, usize)> = None;
        for (i, raw) in text.lines().enumerate() {
            let line = strip_comment(raw.trim_end_matches('\r'));
            let (joined, start) = match pending.take() {
                Some((mut acc, start)) => {
                    acc.push(' ');
                    acc.push_str(line.trim());
                    (acc, start)
                }
                None => (line.trim().to_string(), i + 1),
            };
            if let Some(stripped) = joined.strip_suffix('\\') {
                pending = Some((stripped.trim_end().to_string(), start));
                continue;
            }
            ini.add_line(&joined, start);
        }
        if let Some((acc, start)) = pending {
            ini.add_line(&acc, start);
        }
        ini
    }

    fn add_line(&mut self, line: &str, line_no: usize) {
        if line.is_empty() {
            return;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next())
            && !line.contains('=') {
                self.sections.push(Section { name: name.trim().to_string(), entries: Vec::new() });
                return;
            }
        let Some((lhs, rhs)) = line.split_once('=') else { return };
        let lhs = lhs.trim();
        let (key, index) = match lhs.split_once('[') {
            Some((k, rest)) => (k.trim(), Some(rest.trim_end_matches(']').trim().to_string())),
            None => (lhs, None),
        };
        if self.sections.is_empty() {
            self.sections.push(Section { name: String::new(), entries: Vec::new() });
        }
        let section = self.sections.last_mut().expect("section exists");
        section.entries.push(Entry { key: key.to_string(), index, value: unquote(rhs.trim()), line: line_no });
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Sections whose names start with `prefix` (e.g. every `commands*` section).
    pub fn sections_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a Section> + 'a {
        self.sections
            .iter()
            .filter(move |s| s.name.len() >= prefix.len() && s.name[..prefix.len()].eq_ignore_ascii_case(prefix))
    }
}

impl Section {
    /// First value for `key` (case-insensitive, unindexed).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|e| e.index.is_none() && e.key.eq_ignore_ascii_case(key)).map(|e| e.value.as_str())
    }

    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Entry> + 'a {
        self.entries.iter().filter(move |e| e.key.eq_ignore_ascii_case(key))
    }
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

fn unquote(value: &str) -> String {
    match value.strip_prefix('"') {
        Some(rest) => rest.strip_suffix('"').unwrap_or(rest).to_string(),
        None => value.to_string(),
    }
}

/// One command in a `{ ... }` block: `NAME(args)`; `args` is the raw text between the
/// parentheses with surrounding whitespace trimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    pub args: String,
}

/// Parse a command block such as `{ OSOUND(DROPE01) INCOS(LIFE=5) XT (250-258) }`.
/// Parentheses nest and quoted strings may contain any characters.
pub fn parse_calls(block: &str) -> Vec<Call> {
    let body = block.trim().trim_start_matches('{').trim_end_matches('}');
    let chars: Vec<char> = body.chars().collect();
    let mut calls = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        let mut j = i;
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j >= chars.len() || chars[j] != '(' {
            calls.push(Call { name, args: String::new() });
            continue;
        }
        let (mut depth, mut in_quote, args_start) = (0usize, false, j + 1);
        while j < chars.len() {
            match chars[j] {
                '"' => in_quote = !in_quote,
                '(' if !in_quote => depth += 1,
                ')' if !in_quote => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let args: String = chars[args_start..j.min(chars.len())].iter().collect();
        calls.push(Call { name, args: args.trim().to_string() });
        i = j + 1;
    }
    calls
}

/// Split `LIFE=100,MAGIC=0` (optionally wrapped in parentheses) into name/value pairs.
pub fn parse_assignments(args: &str) -> Vec<(String, String)> {
    args.trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            Some((k.trim().to_string(), v.trim().trim_matches(|c| c == '(' || c == ')').to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_entries_and_continuations() {
        let ini = Ini::parse(
            "[startup]\r\norbname=lips00 ; comment\r\nm1title=\"  -= Game Menu =-\r\n[commands9999]\r\nONTAG[401]={ OSOUND(DROPE01) INCOS(LIFE=5) }\r\nOPTOOL[0]=( TN=0 \\ \r\n  R=16 )\r\n",
        );
        let startup = ini.section("STARTUP").unwrap();
        assert_eq!(startup.get("orbname"), Some("lips00"));
        assert_eq!(startup.get("m1title"), Some("  -= Game Menu =-"));
        let cmds = ini.section("commands9999").unwrap();
        assert_eq!(cmds.entries[0].index.as_deref(), Some("401"));
        assert_eq!(cmds.entries[1].value, "( TN=0 R=16 )");
    }

    #[test]
    fn command_blocks() {
        let calls = parse_calls(r#"{ OSOUND( QUACK05 ) OTEXT( " You need (more) MONEY!") XT (250-258) }"#);
        assert_eq!(
            calls,
            vec![
                Call { name: "OSOUND".into(), args: "QUACK05".into() },
                Call { name: "OTEXT".into(), args: r#"" You need (more) MONEY!""#.into() },
                Call { name: "XT".into(), args: "250-258".into() },
            ]
        );
    }

    #[test]
    fn assignments() {
        assert_eq!(
            parse_assignments("(LIFE=100,MAGIC=(672))"),
            vec![("LIFE".to_string(), "100".to_string()), ("MAGIC".to_string(), "672".to_string())]
        );
    }
}
