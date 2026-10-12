//! Bot ("mover") scripts from `[moveractioncodes]` (re/engine/movers.md).
//!
//! `NAME= [IN{…}] {ON cond{…}}×≤4 main`: the IN body runs once on attach, ON blocks preempt
//! the main body when their condition becomes true and then resume it, `*` restarts the main
//! body and a body that ends without `*` stops the script. At most one opcode runs per 30 Hz
//! logic frame; words of four or more letters are INI commands run in the same frame.

use crate::script::preprocess;
use std::collections::HashMap;

/// Movement action bits (physics.md §3.1).
pub mod action {
    pub const FORWARD: u32 = 0x10;
    pub const BACKWARD: u32 = 0x20;
    pub const LEFT: u32 = 0x40;
    pub const RIGHT: u32 = 0x80;
    pub const UP: u32 = 0x100;
    pub const DOWN: u32 = 0x200;
    /// `MD`: the object's own effect behaviour (wander for OWALK, stand for OTHING).
    pub const OWN: u32 = 0x400_0000;
}

/// INI commands that may appear inside scripts (four letters or more).
const INI_WORDS: &[&str] = &["OSOUND", "OTEXT", "XCHANGE", "INCOS", "SUMOS", "SETOS", "INCTOOS", "STATEVAL", "ONTAG", "OSETMOVE"];

#[derive(Debug, Clone, PartialEq)]
enum Op {
    /// Embedded INI command line (run immediately).
    Ini(String),
    /// Mover opcode: name (uppercase, optional leading `~`) and raw argument text.
    Code { name: String, args: String },
    /// `*`: restart the main body.
    Star,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Condition {
    bump: bool,
    hit: bool,
    kill: bool,
    tag: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
struct OnBlock {
    cond: Condition,
    body: Vec<Op>,
}

/// A compiled script.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Script {
    pub name: String,
    init: Vec<Op>,
    on: Vec<OnBlock>,
    main: Vec<Op>,
    has_kill: bool,
}

/// Compile every script of a `[moveractioncodes]` section.
pub fn compile_section(ini_text: &str) -> HashMap<String, Script> {
    crate::script::section_lines(ini_text, "moveractioncodes")
        .into_iter()
        .filter_map(|line| {
            let (name, body) = line.split_once('=')?;
            let name = name.trim().to_ascii_uppercase();
            Some((name.clone(), compile(&name, &preprocess(body))))
        })
        .collect()
}

fn compile(name: &str, text: &str) -> Script {
    let mut script = Script { name: name.to_string(), ..Default::default() };
    let chars: Vec<char> = text.chars().collect();
    let mut i = skip_ws(&chars, 0);
    if starts_with(&chars, i, "IN") && chars.get(skip_ws(&chars, i + 2)) == Some(&'{') {
        let open = skip_ws(&chars, i + 2);
        let close = matching(&chars, open);
        script.init = tokenize(&chars[open + 1..close]);
        i = skip_ws(&chars, close + 1);
    }
    while script.on.len() < 4 && starts_with(&chars, i, "ON") {
        let Some(open) = chars[i..].iter().position(|&c| c == '{').map(|p| p + i) else { break };
        let cond = parse_condition(&chars[i + 2..open].iter().collect::<String>());
        let close = matching(&chars, open);
        script.has_kill |= cond.kill;
        script.on.push(OnBlock { cond, body: tokenize(&chars[open + 1..close]) });
        i = skip_ws(&chars, close + 1);
    }
    script.main = tokenize(&chars[i.min(chars.len())..]);
    script
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

fn starts_with(chars: &[char], i: usize, word: &str) -> bool {
    word.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c))
}

/// Index of the bracket closing the one at `open`; only the same bracket kind counts, so
/// stray `)` inside `{ … }` bodies (shipped scripts have them) do not end a block.
fn matching(chars: &[char], open: usize) -> usize {
    let (opener, closer) = if chars.get(open) == Some(&'{') { ('{', '}') } else { ('(', ')') };
    let (mut depth, mut quote) = (0, false);
    for (j, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '"' => quote = !quote,
            c if c == opener && !quote => depth += 1,
            c if c == closer && !quote => {
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

fn parse_condition(text: &str) -> Condition {
    let t = text.trim().to_ascii_uppercase();
    let mut cond = Condition::default();
    if let Some(start) = t.find("T(") {
        cond.tag = t[start + 2..].split(')').next().and_then(|v| v.trim().parse().ok());
    }
    let letters: String = t.split('(').next().unwrap_or("").chars().filter(|c| c.is_ascii_alphabetic()).collect();
    // Event letter followed by an actor letter: B/H with P (player), K = kill.
    cond.bump = letters.contains("BP") || letters.starts_with('B');
    cond.hit = letters.contains("HP") || (letters.starts_with('H') && !letters.contains("HK"));
    cond.kill = letters.contains('K');
    if cond.kill {
        cond.hit = false;
        cond.bump = false;
    }
    cond
}

fn tokenize(chars: &[char]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == ',' || c == ';' {
            i += 1;
            continue;
        }
        if c == '*' {
            ops.push(Op::Star);
            i += 1;
            continue;
        }
        if c == ')' || c == '}' || c == '{' {
            // Stray brackets execute as a no-op opcode (one idle frame).
            ops.push(Op::Code { name: c.to_string(), args: String::new() });
            i += 1;
            continue;
        }
        let start = i;
        if c == '~' {
            i += 1;
        }
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        if i == start {
            i += 1;
            continue;
        }
        let word: String = chars[start..i].iter().collect::<String>().to_ascii_uppercase();
        let j = skip_ws(chars, i);
        let (args, next) = if chars.get(j) == Some(&'(') {
            let close = matching(chars, j);
            (chars[j + 1..close.min(chars.len())].iter().collect::<String>(), close + 1)
        } else {
            (String::new(), i)
        };
        if word.len() >= 4 && INI_WORDS.contains(&word.as_str()) {
            ops.push(Op::Ini(format!("{word}({args})")));
        } else {
            ops.push(Op::Code { name: word, args: args.trim().to_string() });
        }
        i = next;
    }
    ops
}

/// Side effects a mover step asks the game to perform.
#[derive(Debug, Clone, PartialEq)]
pub enum MoverRequest {
    /// Run an INI command line (OSOUND, OTEXT, XCHANGE…).
    Ini(String),
    Tag(u32),
    TagRange(u32, u32),
    Message { color: u8, text: String },
    /// Turn to face the player (TP; `away` for ~TP).
    FacePlayer { away: bool },
    SetAngle(f32),
    /// Fire other-tool `tool`; a hit on the player fires `tag`.
    Attack { tool: usize, tag: Option<u32> },
    /// Player state `index op value` (SV(IS…)), raw.
    PlayerState { index: usize, op: char, value: i32 },
    /// Spawn a clone of the named object here (XL).
    Spawn(String),
    /// Become a copy of the named template, script included (XN).
    ChangeClass(String),
    Destroy,
    Sound(String),
    /// Spin the player's view (PTL/PTR): angle units per frame for `frames` frames.
    TurnPlayer { units_per_frame: i32, frames: u32 },
    Activate(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Where {
    Main,
    On(usize),
}

#[derive(Debug, Clone, Copy)]
struct Context {
    body: Where,
    pc: usize,
    wait: u32,
    action: u32,
}

/// Per-object interpreter state.
#[derive(Debug, Clone)]
pub struct Mover {
    pub script: Script,
    ctx: Context,
    saved_main: Option<Context>,
    pub active: bool,
    pub finished: bool,
    /// Hits left before the K condition (state value 6, set by SH).
    pub hits_left: i32,
    killed: bool,
    bump_pending: bool,
    hit_pending: bool,
    tag_pending: Option<u32>,
    rescan: bool,
    /// Waiting for this tag (WT).
    pub waiting_tag: Option<u32>,
    rng: u32,
}

impl Mover {
    /// Attach a script; its IN body runs immediately. Returns requests produced by IN.
    pub fn attach(script: Script, seed: u32) -> (Self, Vec<MoverRequest>) {
        let mut m = Mover {
            script,
            ctx: Context { body: Where::Main, pc: 0, wait: 0, action: 0 },
            saved_main: None,
            active: false,
            finished: false,
            hits_left: 0,
            killed: false,
            bump_pending: false,
            hit_pending: false,
            tag_pending: None,
            rescan: false,
            waiting_tag: None,
            rng: seed | 1,
        };
        let mut out = Vec::new();
        let init = m.script.init.clone();
        for op in &init {
            m.execute(op, &mut out);
        }
        m.ctx = Context { body: Where::Main, pc: 0, wait: 0, action: 0 };
        (m, out)
    }

    /// Contact by the player's body (BP) or a player projectile (HP).
    pub fn bumped(&mut self, by_projectile: bool) {
        self.active = true;
        if by_projectile {
            self.hit_pending = true;
        } else {
            self.bump_pending = true;
        }
        self.rescan = true;
    }

    /// A tag reached this object. Only tags the script listens for (a `T` condition or a `WT`
    /// wait) are events; others must not trigger an ON scan (a pending HK would fire early).
    pub fn tag(&mut self, tag: u32) {
        let listens = self.script.on.iter().any(|b| b.cond.tag.is_some_and(|t| t as u32 == tag));
        if self.waiting_tag == Some(tag) {
            self.waiting_tag = None;
        } else if !listens {
            return;
        }
        self.tag_pending = Some(tag);
        self.rescan = true;
    }

    fn body(&self, w: Where) -> &[Op] {
        match w {
            Where::Main => &self.script.main,
            Where::On(i) => &self.script.on[i].body,
        }
    }

    fn condition_true(&mut self, i: usize) -> bool {
        let c = self.script.on[i].cond.clone();
        if c.hit && self.hit_pending && !self.bump_pending {
            if self.script.has_kill {
                self.hits_left -= 1;
                if self.hits_left < 1 {
                    self.killed = true;
                }
            }
            self.hit_pending = false;
            return true;
        }
        if c.bump && self.bump_pending && !self.hit_pending {
            self.bump_pending = false;
            return true;
        }
        if c.kill && self.killed && self.hits_left <= 0 {
            self.killed = false;
            return true;
        }
        if let (Some(t), Some(p)) = (c.tag, self.tag_pending)
            && t as u32 == p {
                self.tag_pending = None;
                return true;
            }
        false
    }

    /// One 30 Hz logic frame. Returns the movement action mask.
    pub fn think(&mut self, out: &mut Vec<MoverRequest>) -> u32 {
        if self.finished && !self.rescan {
            return 0;
        }
        if self.rescan {
            self.rescan = false;
            let current = match self.ctx.body {
                Where::On(i) => Some(i),
                Where::Main => None,
            };
            let hit = (0..self.script.on.len()).filter(|&i| Some(i) != current).find(|&i| self.condition_true(i));
            if let Some(i) = hit {
                if self.saved_main.is_none() {
                    self.saved_main = Some(self.ctx);
                }
                self.ctx = Context { body: Where::On(i), pc: 0, wait: 0, action: 0 };
                self.finished = false;
                return 0;
            }
            self.tag_pending = None;
            self.hit_pending = false;
            self.bump_pending = false;
        }
        if !self.active || self.finished {
            return 0;
        }
        if self.ctx.wait > 0 {
            self.ctx.wait -= 1;
            return self.ctx.action;
        }
        if self.waiting_tag.is_some() {
            return 0;
        }
        self.step(out);
        self.ctx.action
    }

    fn step(&mut self, out: &mut Vec<MoverRequest>) {
        for _ in 0..64 {
            let body_len = self.body(self.ctx.body).len();
            if self.ctx.pc >= body_len {
                // End of a body: an ON block resumes the main context; main without * ends.
                match self.ctx.body {
                    Where::On(_) => {
                        self.ctx = self.saved_main.take().unwrap_or(Context { body: Where::Main, pc: 0, wait: 0, action: 0 });
                        return;
                    }
                    Where::Main => {
                        self.finished = true;
                        self.ctx.action = 0;
                        return;
                    }
                }
            }
            let op = self.body(self.ctx.body)[self.ctx.pc].clone();
            self.ctx.pc += 1;
            match op {
                Op::Ini(_) => self.execute(&op, out),
                Op::Star => {
                    self.ctx.pc = 0;
                    if matches!(self.ctx.body, Where::On(_)) {
                        self.ctx = self.saved_main.take().unwrap_or(Context { body: Where::Main, pc: 0, wait: 0, action: 0 });
                        return;
                    }
                }
                Op::Code { .. } => {
                    self.execute(&op, out);
                    return;
                }
            }
        }
    }

    fn random(&mut self, lo: u32, hi: u32) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        if hi <= lo { lo } else { lo + self.rng % (hi - lo + 1) }
    }

    fn execute(&mut self, op: &Op, out: &mut Vec<MoverRequest>) {
        let (name, args) = match op {
            Op::Ini(line) => {
                out.push(MoverRequest::Ini(line.clone()));
                return;
            }
            Op::Star => return,
            Op::Code { name, args } => (name.as_str(), args.as_str()),
        };
        let n = |s: &str| s.trim().parse::<i64>().unwrap_or(0).max(0) as u32;
        let ints: Vec<i64> = args.split([',', ' ']).filter_map(|v| v.trim().parse().ok()).collect();
        let away = name.starts_with('~');
        let code = name.trim_start_matches('~');
        match code {
            "TP" => {
                out.push(MoverRequest::FacePlayer { away });
                self.ctx.wait = 0;
            }
            "W" => {
                self.ctx.action = 0;
                self.ctx.wait = n(args);
            }
            "WT" => {
                self.ctx.action = 0;
                self.waiting_tag = Some(n(args));
            }
            "MD" => {
                self.ctx.action = action::OWN;
                self.ctx.wait = n(args);
            }
            "SH" => self.hits_left = n(args) as i32,
            "SV" => {
                for setting in args.split(',') {
                    let s: String = setting.chars().filter(|c| !c.is_whitespace()).collect();
                    if let Some(rest) = s.strip_prefix("IS") {
                        if let Some(pos) = rest.find(['=', '+', '-', '*', '/'])
                            && let (Ok(index), Ok(value)) = (rest[..pos].parse(), rest[pos + 1..].parse()) {
                                let op = rest[pos..].chars().next().unwrap_or('=');
                                out.push(MoverRequest::PlayerState { index, op, value });
                            }
                    } else if let Some(v) = s.strip_prefix("AD=") {
                        self.active = v.starts_with('Y');
                        out.push(MoverRequest::Activate(self.active));
                    }
                }
            }
            "XT" => {
                let t = args.trim().trim_start_matches('*');
                match t.split_once('-') {
                    Some((a, b)) => {
                        let (a, b) = (n(a), n(b));
                        let tag = self.random(a, b);
                        out.push(MoverRequest::TagRange(a, b));
                        let _ = tag;
                    }
                    None => out.push(MoverRequest::Tag(n(t))),
                }
            }
            "XM" | "XS" => {
                let text = args.split('"').nth(1).unwrap_or("").to_string();
                let color = args.split('"').next().and_then(|c| c.trim().trim_end_matches(',').trim().parse::<u32>().ok()).filter(|&c| c > 0 && c < 256);
                out.push(MoverRequest::Message { color: color.unwrap_or(92) as u8, text });
            }
            "XL" => out.push(MoverRequest::Spawn(unquote(args))),
            "XN" => {
                out.push(MoverRequest::ChangeClass(unquote(args.split(',').next().unwrap_or(""))));
            }
            "XD" => {
                out.push(MoverRequest::Destroy);
                self.finished = true;
                self.active = false;
            }
            "P" | "PO" => {
                let tool = ints.first().copied().unwrap_or(0).max(0) as usize;
                let tag = ints.get(1).copied().filter(|&t| t > 0).map(|t| t as u32);
                out.push(MoverRequest::Attack { tool, tag });
            }
            "PS" => out.push(MoverRequest::Sound(unquote(args.split(',').next().unwrap_or("")))),
            "PTL" | "PTR" => {
                let units = ints.first().copied().unwrap_or(0) as i32;
                let frames = ints.get(1).copied().unwrap_or(0).max(0) as u32 + 1;
                out.push(MoverRequest::TurnPlayer { units_per_frame: if code == "PTL" { units } else { -units }, frames });
            }
            "A" | "AS" => out.push(MoverRequest::SetAngle(ints.first().copied().unwrap_or(0) as f32)),
            "H" => {
                self.ctx.action = 0;
                self.ctx.wait = n(args);
            }
            _ if !code.is_empty() && code.chars().all(|c| "UDFBRL".contains(c)) => {
                let mut mask = 0;
                for c in code.chars() {
                    mask |= match c {
                        'F' => action::FORWARD,
                        'B' => action::BACKWARD,
                        'L' => action::LEFT,
                        'R' => action::RIGHT,
                        'U' => action::UP,
                        'D' => action::DOWN,
                        _ => 0,
                    };
                }
                self.ctx.action = mask;
                self.ctx.wait = n(args);
            }
            _ => self.ctx.action = 0, // unknown / stray bracket: one idle frame
        }
    }
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INI: &str = "[moveractioncodes]\n\
DETHBOT1= IN { SetHitstokill(1) } \\\n\
          ON HK { XL(DROPBONZ) XT(761) SV(IS12 + 1) XN(\"DEADBOT1\")) } \\\n\
          ON HP { U(8) XT(761) XM(199,\"OUCH OUCH!!!\")} \\\n\
          ON BP { XM(199,\"GOTCHA!!!\") XT(661) ) } \\\n\
          TP OSOUND(GROWL14) F(16) P(1,411) W(16)*\n\
DEADBOT1= ON BP { XT(401) OTEXT(\"YOU FOUND SOME LIFE ELIXIR!\") SV(IS10 + 1) XD } \\\n\
          W(1) *\n";

    fn run(m: &mut Mover, frames: usize) -> (Vec<u32>, Vec<MoverRequest>) {
        let mut out = Vec::new();
        let masks = (0..frames).map(|_| m.think(&mut out)).collect();
        (masks, out)
    }

    #[test]
    fn compiles_shipped_shapes() {
        let scripts = compile_section(INI);
        let bot = &scripts["DETHBOT1"];
        assert_eq!(bot.on.len(), 3);
        assert!(bot.on[0].cond.kill && bot.on[1].cond.hit && bot.on[2].cond.bump);
        assert!(bot.main.contains(&Op::Ini("OSOUND(GROWL14)".into())));
        assert_eq!(bot.main.last(), Some(&Op::Star));
    }

    #[test]
    fn main_cycle_faces_moves_attacks_and_waits() {
        let scripts = compile_section(INI);
        let (mut m, init) = Mover::attach(scripts["DETHBOT1"].clone(), 1);
        assert!(init.is_empty());
        assert_eq!(m.hits_left, 1, "IN {{SH(1)}} ran at attach");
        m.active = true;
        let (masks, out) = run(&mut m, 40);
        assert!(out.contains(&MoverRequest::FacePlayer { away: false }));
        assert!(out.contains(&MoverRequest::Ini("OSOUND(GROWL14)".into())));
        assert!(out.contains(&MoverRequest::Attack { tool: 1, tag: Some(411) }));
        // F(16) = 17 frames, and P keeps the action for its own frame.
        let first_run = masks.iter().skip_while(|&&a| a != action::FORWARD).take_while(|&&a| a == action::FORWARD).count();
        assert_eq!(first_run, 18);
    }

    #[test]
    fn hit_then_kill_on_next_event() {
        let scripts = compile_section(INI);
        let (mut m, _) = Mover::attach(scripts["DETHBOT1"].clone(), 1);
        m.active = true;
        run(&mut m, 5);
        m.bumped(true);
        let (_, out) = run(&mut m, 14);
        assert!(out.iter().any(|r| matches!(r, MoverRequest::Message { text, .. } if text == "OUCH OUCH!!!")), "HP block ran");
        assert_eq!(m.hits_left, 0);
        m.bumped(true);
        let (_, out) = run(&mut m, 8);
        assert!(out.contains(&MoverRequest::ChangeClass("DEADBOT1".into())), "HK on the next event: {out:?}");
        assert!(out.contains(&MoverRequest::PlayerState { index: 12, op: '+', value: 1 }));
    }

    #[test]
    fn pickup_template_gives_and_destroys() {
        let scripts = compile_section(INI);
        let (mut m, _) = Mover::attach(scripts["DEADBOT1"].clone(), 1);
        m.active = true;
        m.bumped(false);
        let (_, out) = run(&mut m, 8);
        assert!(out.contains(&MoverRequest::Tag(401)));
        assert!(out.iter().any(|r| matches!(r, MoverRequest::Ini(l) if l.starts_with("OTEXT"))));
        assert!(out.contains(&MoverRequest::Destroy));
    }
}
