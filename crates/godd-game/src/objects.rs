//! Runtime objects: bots driven by mover scripts, pickups, switches, timers, teleports, level
//! exits, floor/ceiling movers, sound and text objects, plus the player's tools, projectiles
//! and explosions (re/engine/object-effects.md, movers.md, combat.md).
//!
//! Object logic runs at the original's 30 Hz frame rate (mover waits count frames); the
//! session calls [`Objects::tick`] every simulation tick and frames are accumulated here.

use crate::anim::{self, Sequence};
use crate::level::Level;
use crate::mover::{self, Mover, MoverRequest, action};
use crate::world::{Sprite, World};
use godd_formats::xrb::{RecordExt, Section};
use std::collections::HashMap;
use std::f32::consts::TAU;

/// Original logic frames per second.
const FRAME_RATE: f32 = 30.0;
const ANGLE_UNITS: f32 = 2048.0;
/// How long a shot lights the scene (modern lighting only).
const MUZZLE_FLASH_SECONDS: f32 = 0.07;

/// Bump cooldown for bots/things: 20 animation units of 12 ms.
const BUMP_COOLDOWN: f32 = 0.24;
/// Movers farther than this from the player (and not in its region) sleep.
const ACTIVATE_DISTANCE: f32 = 1024.0;
/// Bot physics (movers.md §5), per 1.2 s units applied per frame as v/40.
const BOT_ACCEL: f32 = 400.0;
const BOT_FRICTION: f32 = 200.0;
const BOT_MAX_SPEED: f32 = 500.0;
const BOT_GRAVITY: f32 = 400.0;
const BOT_HOP: f32 = 200.0;
const PROJECTILE_RADIUS: f32 = 8.0;
const PROJECTILE_HEIGHT: f32 = 16.0;

/// Effect indices used here (row order of re/engine/effects.md).
mod fx {
    pub const OWALK: i32 = 5;
    pub const OFOLLOW: i32 = 10;
    pub const OFLYON: i32 = 26;
    pub const OFLYOFF: i32 = 27;
    pub const OSNDSEQ: i32 = 54;
    pub const OOTOO: i32 = 66;
    pub const OWTOW: i32 = 67;
    pub const OORIENT: i32 = 69;
    pub const OCOUNT: i32 = 72;
    pub const OEFFSW: i32 = 73;
    pub const OTEXOVER: i32 = 86;
    pub const ODROP: i32 = 131;
    pub const OFLASH: i32 = 141;
    pub const OPHYSON: i32 = 172;
    pub const OVIEWON: i32 = 181;
    pub const OVIEWOFF: i32 = 182;
    pub const OTELE: i32 = 15;
    pub const OTAGSW: i32 = 20;
    pub const OBUMPSW: i32 = 22;
    pub const OSTRAP: i32 = 29;
    pub const OSETF: i32 = 41;
    pub const OSETC: i32 = 49;
    pub const OSOUND: i32 = 53;
    pub const ODESTROY: i32 = 78;
    pub const OTIMER: i32 = 80;
    pub const ORANDOM: i32 = 81;
    pub const OINCOS: i32 = 112;
    pub const OSELTOOL: i32 = 127;
    pub const OPICKUP: i32 = 130;
    pub const OTEXT: i32 = 132;
}

/// The aux-record fields object effects use.
#[derive(Debug, Clone, Default)]
pub struct ObjAux {
    pub ints: [i32; 5],
    /// Name slots 0–9 (12 chars each from +0x0e).
    pub slots: [String; 10],
    /// Resolved sample names for sound slots 0–4.
    pub sounds: [Option<String>; 5],
}

impl ObjAux {
    fn load(level: &Level, index: i32) -> Option<Self> {
        let record = level.orb.records(Section::Auxs).get(usize::try_from(index).ok()?)?;
        let sound_records = level.orb.records(Section::Sounds);
        let slots = std::array::from_fn(|k| c_string(&record[0x0e + 12 * k..0x0e + 12 * (k + 1)]));
        let sounds = std::array::from_fn(|k| {
            let s = usize::try_from(record.le_i32(0x9c + 4 * k)).ok()?;
            let name = c_string(&sound_records.get(s)?[0x0c..0x18]);
            (!name.is_empty()).then_some(name)
        });
        Some(Self { ints: std::array::from_fn(|k| record.le_i32(0x88 + 4 * k)), slots, sounds })
    }

    fn slot(&self, k: usize) -> Option<&str> {
        let s = self.slots[k].as_str();
        (!s.is_empty() && !s.eq_ignore_ascii_case("NO_NAME")).then_some(s)
    }
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| b as char).collect::<String>().trim().to_string()
}

#[derive(Debug, Clone)]
pub enum Kind {
    Normal,
    Projectile { from_player: bool, tag: Option<u32>, flying: bool, life: f32, explosion: Option<usize> },
    /// `weapon`: counts as a player weapon against bots (AMF bit 0x80).
    Explosion { life: f32, weapon: bool },
}

#[derive(Debug, Clone)]
pub struct Obj {
    pub name: String,
    pub effect: i32,
    pub flags: u32,
    pub tag: i32,
    pub aux: Option<ObjAux>,
    /// Per-effect counter (object +0x54: uses, timer period, kind …).
    pub counter: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub angle: f32,
    pub radius: f32,
    pub height: f32,
    pub region: Option<usize>,
    pub tex: Option<usize>,
    pub sprite: Option<usize>,
    pub hidden: bool,
    pub destroyed: bool,
    /// In contact with the player this frame / last frame (first-contact edge).
    contact: bool,
    cooldown: f32,
    pub mover: Option<Mover>,
    vx: f32,
    vz: f32,
    pub kind: Kind,
    /// OTIMER/ORANDOM deadline, OTEXT/OSETF state (seconds).
    deadline: Option<f64>,
    /// OWALK wander: frames left in the current turn burst and its direction.
    wander: (u32, f32),
    home: (f32, f32, f32),
    /// Max forward speed (+0x7c, units per 1.2 s).
    pub speed: f32,
    /// Leader object (OORIENT) / path holder (OFOLLOW), from +0x198.
    link: Option<usize>,
    /// Path (OFOLLOW), from +0x110, and the current node.
    path: Option<usize>,
    path_node: usize,
    /// Region carried along when this object moves (RMOVE rafts).
    pub carries: Option<usize>,
    /// Sound playback state (OSOUND / OSNDSEQ).
    sound: SoundState,
    /// OFLASH: flashes left and the end of the current on/off phase (seconds).
    flash: (i32, Option<f64>, bool),
}

#[derive(Debug, Clone, Default)]
struct SoundState {
    active: bool,
    /// Plays left (OSOUND) or sequence loops left (OSNDSEQ).
    left: i32,
    /// When the current play ends or the next one starts (seconds).
    next: Option<f64>,
    /// OSNDSEQ: current slot and remaining repeats of it.
    slot: usize,
    reps: i32,
    looping: bool,
}

impl Obj {
    fn blank() -> Self {
        Obj {
            name: String::new(),
            effect: -1,
            flags: 0,
            tag: -1,
            aux: None,
            counter: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            angle: 0.0,
            radius: 16.0,
            height: 48.0,
            region: None,
            tex: None,
            sprite: None,
            hidden: false,
            destroyed: false,
            contact: false,
            cooldown: 0.0,
            mover: None,
            vx: 0.0,
            vz: 0.0,
            kind: Kind::Normal,
            deadline: None,
            wander: (0, 0.0),
            home: (0.0, 0.0, 0.0),
            speed: 0.0,
            link: None,
            path: None,
            path_node: 0,
            carries: None,
            sound: SoundState::default(),
            flash: (0, None, false),
        }
    }

    fn in_world(&self) -> bool {
        !self.destroyed && !self.hidden && self.region.is_some()
    }
}

/// Tool, projectile or explosion definition from OPTOOL/OPPROJ/OPXPLD (and OO* for bots).
#[derive(Debug, Clone, Default)]
pub struct ToolDef {
    pub tex: Option<usize>,
    pub ty: u32,
    pub radius: f32,
    pub height: f32,
    pub max_speed: [f32; 4],
    pub sounds: [Option<String>; 5],
    pub nums: [i32; 5],
    /// AMF flags; 0x80 = the explosion counts as a weapon.
    pub amf: u32,
}

impl ToolDef {
    fn parse(args: &str, texs: &HashMap<String, usize>) -> Self {
        let mut def = ToolDef { radius: 16.0, height: 32.0, ..Default::default() };
        let text = args.trim().trim_start_matches('(').trim_end_matches(')');
        let mut i = 0;
        let chars: Vec<char> = text.chars().collect();
        while i < chars.len() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let start = i;
            while i < chars.len() && chars[i] != '=' && !chars[i].is_whitespace() {
                i += 1;
            }
            let key: String = chars[start..i].iter().collect::<String>().to_ascii_uppercase();
            if i >= chars.len() || chars[i] != '=' {
                i += 1;
                continue;
            }
            i += 1;
            let value: String = if chars.get(i) == Some(&'(') {
                let end = chars[i..].iter().position(|&c| c == ')').map_or(chars.len(), |p| p + i);
                let v = chars[i + 1..end].iter().collect();
                i = end + 1;
                v
            } else {
                let s = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[s..i].iter().collect()
            };
            let list: Vec<&str> = value.split_whitespace().collect();
            match key.as_str() {
                "TEX" => def.tex = texs.get(&value.to_ascii_uppercase()).copied(),
                "TY" => def.ty = u32::from_str_radix(&value, 16).unwrap_or(0),
                "AMF" => def.amf = u32::from_str_radix(&value, 16).unwrap_or(0),
                "R" => def.radius = value.parse().unwrap_or(16.0),
                "H" => def.height = value.parse().unwrap_or(32.0),
                "MX" => {
                    for (d, v) in def.max_speed.iter_mut().zip(&list) {
                        *d = v.parse().unwrap_or(0.0);
                    }
                }
                "SNDS" => {
                    for (d, v) in def.sounds.iter_mut().zip(&list) {
                        *d = (*v != "~").then(|| v.to_string());
                    }
                }
                "NUMS" => {
                    for (d, v) in def.nums.iter_mut().zip(&list) {
                        *d = v.parse().unwrap_or(0);
                    }
                }
                _ => {}
            }
        }
        def
    }

    fn flying(&self) -> bool {
        self.ty & 0x2 != 0
    }
}

/// Things the session must carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjRequest {
    Tag(u32),
    /// A tag from a world switch (OTAGSW/OBUMPSW): fired locally and shared in netplay.
    SharedTag(u32),
    TagRange(u32, u32),
    Ini(String),
    Sound(String),
    Message { color: u8, text: String },
    /// OTEXT display: text, 320x200 position, seconds.
    Text { text: String, x: i32, y: i32, seconds: f32 },
    /// Raw player state change (SV(IS…)).
    PlayerState { index: usize, op: char, value: i32 },
    /// OINCOS: add through the threshold logic.
    IncState { index: usize, delta: i32 },
    Teleport { x: f32, y: f32, z: Option<f32>, angle: Option<f32>, region: Option<usize> },
    LevelExit { level: String, start: Option<String> },
    TurnPlayer { units_per_frame: i32, frames: u32 },
    /// Trigger (event 0x80) sent to a wall/region by name.
    TriggerNamed(String),
    /// A projectile hit the player and carried this tag.
    PlayerHit(u32),
    /// Looping/channel audio (positional or chained sounds).
    Audio(crate::audio::AudioEvent),
    /// OWTOW: give the wall named `target` the look and behaviour of wall `source`.
    CopyWall { target: String, source: String },
    /// OOTOO on the player: new height and speed (units per 1.2 s).
    MorphPlayer { height: f32, speed: f32 },
    /// OFLYON / OFLYOFF (optional height while flying).
    Fly { on: bool, height: Option<f32> },
    /// OVIEWON / OVIEWOFF: switch the named secondary view (TV screens).
    View { name: String, on: bool },
    /// OTEXOVER: show/hide a screen overlay picture (Tex index) at 320x200 (x, y).
    Overlay { tex: Option<usize>, x: i32, y: i32, on: bool },
    /// OFLASH: lightning (everything full bright while on).
    Flash(bool),
    /// RMOVE: a carrier moved its region by (dx, dy).
    CarryRegion { region: usize, dx: f32, dy: f32 },
}

/// Player tool state.
#[derive(Debug, Clone)]
pub struct Tools {
    pub current: usize,
    pub available: u32,
    pub defs: [Option<ToolDef>; 16],
    pub projectiles: [Option<ToolDef>; 16],
    pub explosions: [Option<ToolDef>; 16],
    pub bot_projectiles: [Option<ToolDef>; 16],
    pub bot_explosions: [Option<ToolDef>; 16],
    /// OTOOLCON: (state index, needed, delta, fail tag).
    pub costs: [Option<(usize, i32, i32, u32)>; 16],
    cooldown: f32,
    /// Weapon view animation (tool Tex, playback) and whether a shot is armed.
    pub anim: Option<(usize, anim::Player)>,
    shoot: bool,
}

impl Default for Tools {
    fn default() -> Self {
        Tools {
            current: 0,
            available: 1,
            defs: Default::default(),
            projectiles: Default::default(),
            explosions: Default::default(),
            bot_projectiles: Default::default(),
            bot_explosions: Default::default(),
            costs: Default::default(),
            cooldown: 0.0,
            anim: None,
            shoot: false,
        }
    }
}

/// The player as seen by object logic.
#[derive(Debug, Clone, Copy)]
pub struct PlayerView {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub eye: f32,
    pub angle: f32,
    pub pitch: f32,
    pub radius: f32,
    pub height: f32,
    pub region: usize,
}

pub struct Objects {
    pub list: Vec<Obj>,
    scripts: HashMap<String, mover::Script>,
    texs_by_name: HashMap<String, usize>,
    pub tools: Tools,
    pub inventory: Vec<usize>,
    requests: Vec<ObjRequest>,
    frame_accum: f32,
    frame: u64,
    /// OSETF/OSETC motions: (object, region, target, speed per second, is ceiling).
    motions: Vec<(usize, usize, f32, f32, bool)>,
    /// Sprites whose Tex changed (XN), applied at the next sync.
    retex: Vec<(usize, Option<usize>)>,
    rng: u32,
    /// Named points (ODROP destinations) and every point's position.
    point_names: HashMap<String, usize>,
    points: Vec<(f32, f32, f32)>,
    /// Paths: point indices of their nodes.
    paths: Vec<Vec<usize>>,
    /// Playing time per sample name (from the WAV headers), for chained/repeated sounds.
    pub sound_lengths: HashMap<String, f32>,
    /// Shown OTEXOVER overlays (object indices).
    overlays_on: Vec<usize>,
    /// Seconds left of the player's muzzle flash (a light for modern renderers).
    pub muzzle_flash: f32,
}

impl Objects {
    pub fn new(level: &Level, ini_text: &str) -> Self {
        let texs_by_name = level.orb.texs.iter().enumerate().map(|(i, t)| (t.name.to_ascii_uppercase(), i)).collect();
        let points = level.orb.records(Section::Points);
        let region_count = level.world.regions.len();
        let mut list = Vec::new();
        for (i, o) in level.orb.records(Section::Objects).iter().enumerate() {
            let mut obj = Obj::blank();
            obj.name = c_string(&o[0x58..0x60]).to_ascii_uppercase();
            obj.effect = o.le_i32(0x140);
            obj.flags = o.le_i32(0) as u32;
            obj.tag = o.le_u16(0x46) as i16 as i32;
            obj.aux = ObjAux::load(level, o.le_i32(0x1b0));
            obj.counter = o.le_i32(0x54);
            if let Some(p) = usize::try_from(o.le_i32(0x14)).ok().and_then(|p| points.get(p)) {
                obj.x = p.fixed(0x14);
                obj.y = p.fixed(0x18);
            }
            obj.z = o.fixed(0x2c);
            obj.angle = o.le_u16(0x24) as f32 / ANGLE_UNITS * TAU;
            obj.radius = o.fixed(0x3c).max(1.0);
            obj.height = o.fixed(0x34).max(1.0);
            obj.region = usize::try_from(o.le_i32(0xfc)).ok().filter(|&r| r < region_count);
            obj.tex = usize::try_from(o.le_i32(0x64)).ok().filter(|&t| t < level.orb.texs.len());
            obj.sprite = level.object_sprites.get(i).copied().flatten();
            obj.home = (obj.x, obj.y, obj.angle);
            // Hidden/inactive markers (OSPOT/OSPAWN) and templates outside every region.
            if matches!(obj.effect, 61 | 62) {
                obj.hidden = true;
            }
            list.push(obj);
        }
        let mut objects = Objects {
            list,
            scripts: mover::compile_section(ini_text),
            texs_by_name,
            tools: Tools::default(),
            inventory: Vec::new(),
            requests: Vec::new(),
            frame_accum: 0.0,
            frame: 0,
            motions: Vec::new(),
            retex: Vec::new(),
            rng: 0x1234_5678,
            point_names: points
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    let n = c_string(&p[0x44..0x50]).to_ascii_uppercase();
                    (!n.is_empty()).then_some((n, i))
                })
                .collect(),
            points: points.iter().map(|p| (p.fixed(0x14), p.fixed(0x18), p.fixed(0x1c))).collect(),
            paths: level
                .orb
                .records(Section::Paths)
                .iter()
                .map(|p| {
                    let n = (p.le_i32(0x18).max(0) as usize).min(64);
                    (0..n).map(|k| p.le_u16(0x44 + 2 * k) as usize).collect()
                })
                .collect(),
            sound_lengths: HashMap::new(),
            overlays_on: Vec::new(),
            muzzle_flash: 0.0,
        };
        // Links stored by the editor: leaders/path holders (+0x198), paths (+0x110), and the
        // regions carried by RMOVE carriers (region +0xfc).
        for (i, o) in level.orb.records(Section::Objects).iter().enumerate() {
            objects.list[i].speed = o.fixed(0x7c);
            objects.list[i].link = usize::try_from(o.le_i32(0x198)).ok().filter(|&l| l < objects.list.len());
            objects.list[i].path = usize::try_from(o.le_i32(0x110)).ok().filter(|&p| p < objects.paths.len());
        }
        for (r, rec) in level.orb.records(Section::Regions).iter().enumerate() {
            if rec.le_i32(0xb0) == 213
                && let Some(c) = usize::try_from(rec.le_i32(0xfc)).ok().filter(|&c| c < objects.list.len()) {
                    objects.list[c].carries = Some(r);
                }
        }
        // The player object, its inert start copy and the network template are never drawn.
        let start_copy = level.orb.header_word(0x4c);
        for (i, o) in objects.list.iter_mut().enumerate() {
            if i == 0 || i as i32 == start_copy || o.name == "NETPLAYR" {
                o.hidden = true;
            }
        }
        objects.tools.available = level.orb.header_word(0x50).max(1) as u32;
        objects
    }

    /// Place an explosion (tests and the console): a weapon blast hits bots inside it
    /// repeatedly, limited by their 240 ms contact debounce.
    pub fn spawn_explosion(&mut self, (x, y, z): (f32, f32, f32), radius: f32, height: f32, life: f32, region: Option<usize>) {
        let mut e = Obj::blank();
        e.name = "_XPLD_".into();
        (e.x, e.y, e.z, e.region, e.radius, e.height) = (x, y, z, region, radius, height);
        e.kind = Kind::Explosion { life, weapon: true };
        self.list.push(e);
    }

    /// Whether an OTIMER/ORANDOM object is counting down.
    pub fn timer_running(&self, i: usize) -> bool {
        self.list.get(i).is_some_and(|o| o.deadline.is_some())
    }

    pub fn take_requests(&mut self) -> Vec<ObjRequest> {
        std::mem::take(&mut self.requests)
    }

    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    fn find(&self, name: &str) -> Vec<usize> {
        let upper = name.to_ascii_uppercase();
        (0..self.list.len()).filter(|&i| self.list[i].name == upper && !self.list[i].destroyed).collect()
    }

    /// INITMOVES: attach the same-named script to every object with that name.
    pub fn init_mover(&mut self, object: &str, script: &str) {
        let Some(compiled) = self.scripts.get(&script.to_ascii_uppercase()).cloned() else {
            log::debug!("no mover script {script}");
            return;
        };
        for i in self.find(object) {
            let seed = self.random();
            let (mover, reqs) = Mover::attach(compiled.clone(), seed);
            self.list[i].mover = Some(mover);
            self.apply_mover_requests(i, reqs, None);
        }
    }

    /// Register a tool/projectile/explosion definition (OPTOOL…/OOPROJ…).
    pub fn define_tool(&mut self, kind: &str, index: usize, args: &str) {
        if index >= 16 {
            return;
        }
        let def = ToolDef::parse(args, &self.texs_by_name);
        match kind {
            "OPTOOL" => self.tools.defs[index] = Some(def),
            "OPPROJ" => self.tools.projectiles[index] = Some(def),
            "OPXPLD" => self.tools.explosions[index] = Some(def),
            "OOPROJ" => self.tools.bot_projectiles[index] = Some(def),
            "OOXPLD" => self.tools.bot_explosions[index] = Some(def),
            _ => {}
        }
    }

    /// OTOOLCON[n]=(STATE, need, delta, failtag).
    pub fn define_tool_cost(&mut self, index: usize, args: &str) {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if index >= 16 || parts.len() < 4 {
            return;
        }
        let state = match parts[0].to_ascii_uppercase().as_str() {
            s if s.starts_with('L') => 1,
            s if s.starts_with("MA") => 2,
            s if s.starts_with('A') => 3,
            s if s.starts_with("MO") => 4,
            _ => 5,
        };
        let n = |s: &str| s.parse::<i32>().unwrap_or(0);
        self.tools.costs[index] = Some((state, n(parts[1]), n(parts[2]), n(parts[3]).max(0) as u32));
    }

    pub fn select_tool(&mut self, index: usize, force: bool, sequences: &[Option<Sequence>]) {
        if index >= 16 || (!force && self.tools.available & (1 << index) == 0) || self.tools.defs[index].is_none() {
            return;
        }
        self.tools.current = index;
        self.tools.anim = self.tools.defs[index].as_ref().and_then(|d| d.tex).and_then(|t| {
            let seq = sequences.get(t)?.as_ref()?;
            Some((t, anim::Player::start(seq, 0)))
        });
    }

    /// Step to the previous (`-1`) or next (`+1`) selectable tool among 0–9, wrapping around.
    pub fn cycle_tool(&mut self, delta: i32, sequences: &[Option<Sequence>]) {
        for step in 1..10 {
            let index = (self.tools.current as i32 + delta * step).rem_euclid(10) as usize;
            if self.tools.available & (1 << index) != 0 && self.tools.defs[index].is_some() {
                self.select_tool(index, false, sequences);
                return;
            }
        }
    }

    /// Event 0x40: a tag reached every object listening on it.
    pub fn on_tag(&mut self, tag: u32, now: f64) {
        for i in 0..self.list.len() {
            if self.list[i].destroyed {
                continue;
            }
            let own_tag = self.list[i].tag;
            if let Some(m) = &mut self.list[i].mover
                && (m.waiting_tag == Some(tag) || own_tag == tag as i32) {
                    m.tag(tag);
                }
            // Switches (OTAGSW/OBUMPSW, mask 0x280) do not listen to tags (no 0x40 in the mask),
            // so firing their own tag cannot loop.
            let listens = !matches!(self.list[i].effect, fx::OTAGSW | fx::OBUMPSW);
            if self.list[i].tag == tag as i32 && listens {
                self.fire(i, Trigger::Tag, now);
            }
        }
    }

    /// Event 0x80 by name (switches, timers): objects first, then walls/regions.
    fn trigger_named(&mut self, name: &str, now: f64) {
        let targets = self.find(name);
        if targets.is_empty() {
            self.requests.push(ObjRequest::TriggerNamed(name.to_string()));
        }
        for t in targets {
            self.fire(t, Trigger::Named, now);
        }
    }

    fn sound(&mut self, name: Option<String>) {
        if let Some(n) = name {
            self.requests.push(ObjRequest::Sound(n));
        }
    }

    /// A sound coming from object `i` (positional playback where the front end supports it).
    fn sound_from(&mut self, i: usize, name: Option<String>) {
        let o = &self.list[i];
        let pos = [o.x, o.y, o.z + o.height / 2.0];
        self.sound_at(name, pos);
    }

    fn sound_at(&mut self, name: Option<String>, pos: [f32; 3]) {
        if let Some(n) = name {
            self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::sound_at(n, pos)));
        }
    }

    /// Run an object's effect for a tag (0x40), a named trigger (0x80) or a player bump (0x200).
    fn fire(&mut self, i: usize, how: Trigger, now: f64) {
        let o = &self.list[i];
        let aux = o.aux.clone();
        let int = |k: usize| aux.as_ref().map_or(0, |a| a.ints[k]);
        let slot = |k: usize| aux.as_ref().and_then(|a| a.slot(k)).map(str::to_string);
        let sound = |k: usize| aux.as_ref().and_then(|a| a.sounds[k].clone());
        match o.effect {
            fx::OTAGSW | fx::OBUMPSW => {
                let limited = o.flags & 0x1000 != 0;
                if limited && o.counter <= 0 {
                    return;
                }
                if limited {
                    self.list[i].counter -= 1;
                }
                if let Some(target) = slot(9) {
                    self.trigger_named(&target, now);
                }
                let tag = self.list[i].tag;
                if tag > 0 {
                    self.requests.push(ObjRequest::SharedTag(tag as u32));
                }
            }
            fx::OTEXT => {
                if aux.is_none() || self.list[i].deadline.is_some_and(|d| now < d) {
                    return;
                }
                let limit = int(4) % 1000;
                if limit != 0 {
                    if self.list[i].counter <= 0 && self.list[i].counter != i32::MIN {
                        return;
                    }
                    self.list[i].counter = if self.list[i].counter == 0 { limit - 1 } else { self.list[i].counter - 1 };
                }
                let seconds = if int(3) < 1 { 5.0 } else { int(3) as f32 };
                let text: String = (5..10).filter_map(&slot).collect::<Vec<_>>().join("").replace('_', " ");
                self.list[i].deadline = Some(now + seconds as f64);
                self.requests.push(ObjRequest::Text { text, x: int(0), y: int(1), seconds });
                self.sound_from(i, sound(0));
            }
            fx::OPICKUP => {
                if self.list[i].hidden {
                    return;
                }
                if matches!(how, Trigger::Bump) {
                    self.list[i].hidden = true;
                    self.inventory.push(i);
                }
                let tool = int(1);
                if (1..16).contains(&tool) {
                    self.tools.available |= 1 << tool;
                }
                if int(0) > 0 {
                    self.requests.push(ObjRequest::Tag(int(0) as u32));
                }
                self.sound_from(i, sound(4));
            }
            fx::OTIMER | fx::ORANDOM => {
                let period = if o.effect == fx::ORANDOM {
                    let span = (o.counter.max(1)) as u32;
                    (self.random() % span) as f64 / 10.0
                } else {
                    o.counter.max(0) as f64 / 10.0
                };
                if self.list[i].deadline.is_none() || int(0) != 1 {
                    self.list[i].deadline = Some(now + period);
                }
            }
            fx::OTELE => {
                let Some(dest) = slot(9) else { return };
                if let Some(&d) = self.find(&dest).first() {
                    let t = &self.list[d];
                    self.requests.push(ObjRequest::Teleport { x: t.x, y: t.y, z: Some(t.z), angle: Some(t.angle), region: t.region });
                } else {
                    self.requests.push(ObjRequest::TriggerNamed(format!("@teleport:{dest}")));
                }
                self.sound_from(i, sound(4));
            }
            fx::OSTRAP => {
                if let Some(level) = slot(9) {
                    self.requests.push(ObjRequest::LevelExit { level: level.to_ascii_uppercase(), start: slot(8) });
                }
                if int(0) != 1 {
                    self.sound_from(i, sound(4));
                }
            }
            fx::OSELTOOL => {
                let idx = self.list[i].counter.max(0) as usize;
                if idx < 16 {
                    let delta = int(1);
                    if delta < 0 {
                        self.tools.available &= !(1 << idx);
                    } else if delta > 0 {
                        self.tools.available |= 1 << idx;
                    }
                    self.requests.push(ObjRequest::Ini(format!("@seltool:{idx}:{}", delta != 0)));
                }
                if int(0) > 0 {
                    self.requests.push(ObjRequest::Tag(int(0) as u32));
                }
            }
            fx::OINCOS => {
                for (k, index) in [1usize, 2, 3, 4, 5].into_iter().enumerate() {
                    if int(k) != 0 {
                        self.requests.push(ObjRequest::IncState { index, delta: int(k) });
                    }
                }
            }
            fx::ODESTROY => {
                if let Some(target) = slot(9) {
                    for t in self.find(&target) {
                        self.list[t].destroyed = true;
                    }
                }
            }
            fx::OSETF | fx::OSETC => {
                if let Some(region) = slot(9) {
                    self.requests.push(ObjRequest::TriggerNamed(format!("@setheight:{i}:{region}")));
                }
            }
            fx::OSOUND => self.start_sound(i, now),
            fx::OSNDSEQ => {
                if self.list[i].sound.active {
                    self.stop_sound(i);
                } else {
                    self.start_sequence(i, now);
                }
            }
            fx::OWTOW => {
                if let (Some(target), Some(source)) = (slot(9), slot(8)) {
                    self.requests.push(ObjRequest::CopyWall { target, source });
                    self.sound_from(i, sound(4));
                }
            }
            fx::OOTOO => {
                let (Some(target), Some(source)) = (slot(9), slot(8)) else { return };
                let Some(&src) = self.find(&source).first() else { return };
                if target.eq_ignore_ascii_case("PLAYER") {
                    let s = &self.list[src];
                    self.requests.push(ObjRequest::MorphPlayer { height: s.height, speed: s.speed });
                } else {
                    let all = int(0) % 10 != 1;
                    let targets = self.find(&target);
                    for t in targets.into_iter().take(if all { usize::MAX } else { 1 }) {
                        self.morph(t, src);
                    }
                }
                self.sound_from(i, sound(4));
            }
            fx::OFLYON => {
                let height = (int(0) > 0).then(|| int(0) as f32);
                self.requests.push(ObjRequest::Fly { on: true, height });
                self.sound_from(i, sound(4));
            }
            fx::OFLYOFF => {
                self.requests.push(ObjRequest::Fly { on: false, height: None });
                self.sound_from(i, sound(4));
            }
            fx::ODROP => self.drop_item(i, slot(9), slot(8), int(0), sound(4)),
            fx::OTEXOVER => {
                let tex = slot(9).and_then(|n| self.texs_by_name.get(&n.to_ascii_uppercase()).copied());
                let on = !self.overlays_on.contains(&i);
                if on {
                    self.overlays_on.push(i);
                    self.sound_from(i, sound(0));
                } else {
                    self.overlays_on.retain(|&o| o != i);
                    self.sound_from(i, sound(2));
                }
                self.requests.push(ObjRequest::Overlay { tex, x: int(1), y: int(0), on });
            }
            fx::OFLASH => {
                let extra = self.list[i].counter;
                let flashes = if extra > 0 { extra } else { (self.random() % extra.unsigned_abs().max(1)) as i32 };
                let on_time = self.flash_time(int(0));
                self.list[i].flash = (flashes, Some(now + on_time), true);
                self.requests.push(ObjRequest::Flash(true));
                self.sound_from(i, sound(4));
            }
            fx::OVIEWON | fx::OVIEWOFF => {
                if let Some(view) = slot(9) {
                    self.requests.push(ObjRequest::View { name: view, on: o.effect == fx::OVIEWON });
                }
                self.sound_from(i, sound(4));
            }
            fx::OCOUNT => {
                self.list[i].deadline = None;
                let left = self.list[i].wander.0 as i32;
                let left = if left == 0 { self.list[i].counter } else { left } - 1;
                if left < 1 {
                    self.list[i].wander.0 = self.list[i].counter.max(0) as u32;
                    if let Some(target) = slot(9) {
                        self.trigger_named(&target, now);
                    }
                    self.sound_from(i, sound(4));
                    if int(3) > 0 {
                        self.requests.push(ObjRequest::Tag(int(3) as u32));
                    }
                } else {
                    self.list[i].wander.0 = left as u32;
                }
            }
            fx::OEFFSW => {
                let limited = o.flags & 0x1000 != 0;
                if limited && o.counter <= 0 {
                    return;
                }
                if limited {
                    self.list[i].counter -= 1;
                }
                if let Some(target) = slot(9) {
                    self.trigger_named(&target, now);
                }
                self.sound_from(i, sound(4));
            }
            fx::OPHYSON => {} // movement model 0 is the only one used (set at every start)
            _ => {}
        }
    }

    /// OFLASH on/off durations: 100 ms units, negative = random 0..|v|-1.
    fn flash_time(&mut self, v: i32) -> f64 {
        let units = if v > 0 { v as u32 } else { self.random() % v.unsigned_abs().max(1) };
        units as f64 / 10.0
    }

    fn sound_seconds(&self, name: &str) -> f64 {
        self.sound_lengths.get(&name.to_ascii_uppercase()).copied().unwrap_or(1.0).max(0.05) as f64
    }

    /// OSOUND start (object-effects.md): aux int0 plays (0 once, n, -1 loop, 9999 endless),
    /// int1 volume 1..1000, int2 digits (units = positional), int3 falloff, int4 delay (0.1 s).
    fn start_sound(&mut self, i: usize, now: f64) {
        let Some(aux) = self.list[i].aux.clone() else { return };
        let Some(name) = aux.sounds[0].clone() else { return };
        let count = aux.ints[0];
        if self.list[i].sound.active && aux.ints[2] / 10 % 10 == 0 && (self.list[i].sound.looping || self.list[i].sound.next.is_some_and(|t| now < t)) {
            return; // still playing and no restart digit
        }
        self.stop_sound(i);
        let delay = aux.ints[4];
        let st = &mut self.list[i].sound;
        st.active = true;
        st.left = count;
        if count == -1 {
            st.looping = true;
            self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::Loop { channel: channel(i), name, volume: volume(aux.ints[1]), pan: 0.0, pos: None }));
            return;
        }
        if delay != 0 {
            let wait = if delay > 0 { delay as f64 / 10.0 } else { (self.random() % delay.unsigned_abs()) as f64 / 10.0 };
            self.list[i].sound.next = Some(now + wait);
            self.list[i].sound.reps = -1; // waiting to start
            return;
        }
        let len = self.sound_seconds(&name);
        self.list[i].sound.next = Some(now + len);
        if count > 0 && count != 9999 {
            self.list[i].sound.left -= 1;
        }
        self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::Sound { name, volume: volume(aux.ints[1]), pan: 0.0, pos: None, muffle: 0.0 }));
    }

    /// OSNDSEQ start: slots 0..4 one after another, each `int k + 1` times; the whole chain
    /// `counter` times (-1/9999 endless).
    fn start_sequence(&mut self, i: usize, now: f64) {
        let Some(aux) = self.list[i].aux.clone() else { return };
        let Some(first) = aux.sounds[0].clone() else { return };
        let len = self.sound_seconds(&first);
        let counter = self.list[i].counter;
        let st = &mut self.list[i].sound;
        st.active = true;
        st.left = counter;
        st.slot = 0;
        st.reps = aux.ints[0];
        st.next = Some(now + len);
        self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::sound(first)));
    }

    fn stop_sound(&mut self, i: usize) {
        if self.list[i].sound.looping {
            self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::Stop { channel: channel(i) }));
        }
        self.list[i].sound = SoundState::default();
    }

    /// Per logic frame: replay/advance sounds, positional loop volume.
    fn step_sounds(&mut self, player: &PlayerView, now: f64) {
        for i in 0..self.list.len() {
            if !self.list[i].sound.active {
                continue;
            }
            let Some(aux) = self.list[i].aux.clone() else { continue };
            let o = &self.list[i];
            if o.sound.looping {
                if aux.ints[2] % 10 > 0 {
                    let d = ((o.x - player.x).powi(2) + (o.y - player.y).powi(2)).sqrt();
                    let falloff = aux.ints[3].max(1) as f32;
                    let vol = (aux.ints[1].clamp(1, 1000) as f32 - d / falloff).max(0.0) / 1000.0;
                    let pan = ((o.y - player.y).atan2(o.x - player.x) - player.angle).sin() * -0.8;
                    self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::ChannelVolume { channel: channel(i), volume: vol, pan }));
                }
                continue;
            }
            if !o.sound.next.is_some_and(|t| now >= t) {
                continue;
            }
            if o.effect == fx::OSNDSEQ {
                let endless = matches!(o.counter, -1 | 9999);
                let st = &mut self.list[i].sound;
                st.reps -= 1;
                if st.reps < 0 {
                    st.slot += 1;
                    if st.slot >= 5 || aux.sounds[st.slot].is_none() {
                        st.slot = 0;
                        st.left -= 1;
                        if st.left < 1 && !endless {
                            self.list[i].sound = SoundState::default();
                            continue;
                        }
                    }
                    let slot = self.list[i].sound.slot;
                    self.list[i].sound.reps = aux.ints[slot];
                }
                let slot = self.list[i].sound.slot;
                let Some(name) = aux.sounds[slot].clone() else {
                    self.list[i].sound = SoundState::default();
                    continue;
                };
                self.list[i].sound.next = Some(now + self.sound_seconds(&name));
                self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::sound(name)));
                continue;
            }
            // OSOUND: delayed first play, then repeats while plays are left.
            let first = self.list[i].sound.reps == -1;
            let left = self.list[i].sound.left;
            if !first && left == 0 {
                self.list[i].sound = SoundState::default();
                continue;
            }
            let Some(name) = aux.sounds[0].clone() else { continue };
            if !first && left > 0 && left != 9999 {
                self.list[i].sound.left -= 1;
            }
            self.list[i].sound.reps = 0;
            let len = self.sound_seconds(&name);
            self.list[i].sound.next = Some(now + len);
            self.requests.push(ObjRequest::Audio(crate::audio::AudioEvent::Sound { name, volume: volume(aux.ints[1]), pan: 0.0, pos: None, muffle: 0.0 }));
        }
    }

    /// OOTOO: give object `t` the look and behaviour of template `src`, keeping its position,
    /// region, angle and name.
    fn morph(&mut self, t: usize, src: usize) {
        let s = self.list[src].clone();
        let o = &mut self.list[t];
        o.effect = s.effect;
        o.flags = s.flags;
        o.tex = s.tex;
        o.aux = s.aux.clone().or(o.aux.take());
        o.radius = s.radius;
        o.height = s.height;
        if s.tag != -1 {
            o.tag = s.tag;
        }
        if s.counter != -1 {
            o.counter = s.counter;
        }
        o.hidden = false;
        o.contact = true; // a morph does not count as a new touch
        if let Some(sp) = o.sprite {
            self.retex.push((sp, o.tex));
        }
    }

    /// ODROP: take the named item out of the inventory and place it at the named point (or
    /// at the dropper), visible again; then fire int0.
    fn drop_item(&mut self, i: usize, item: Option<String>, point: Option<String>, tag: i32, sound: Option<String>) {
        let Some(item) = item else { return };
        let upper = item.to_ascii_uppercase();
        let Some(pos) = self.inventory.iter().position(|&k| self.list[k].name == upper) else { return };
        let k = self.inventory.remove(pos);
        let dest = point
            .and_then(|p| self.point_names.get(&p.to_ascii_uppercase()).copied())
            .and_then(|p| self.points.get(p).copied())
            .unwrap_or((self.list[i].x, self.list[i].y, self.list[i].z));
        let region = self.list[i].region;
        let o = &mut self.list[k];
        (o.x, o.y, o.z) = dest;
        o.region = region.or(o.region);
        o.hidden = false;
        o.contact = true;
        if tag > 0 {
            self.requests.push(ObjRequest::Tag(tag as u32));
        }
        self.sound(sound);
    }

    /// Per logic frame: OFLASH phases, OORIENT followers, OFOLLOW path movers.
    fn step_misc(&mut self, now: f64, world: &World) {
        let frame_dt = 1.0 / FRAME_RATE;
        for i in 0..self.list.len() {
            match self.list[i].effect {
                fx::OFLASH => {
                    let (left, until, on) = self.list[i].flash;
                    let Some(t) = until else { continue };
                    if now < t {
                        continue;
                    }
                    let aux = self.list[i].aux.clone().unwrap_or_default();
                    if on {
                        self.requests.push(ObjRequest::Flash(false));
                        if left < 1 {
                            self.list[i].flash = (0, None, false);
                        } else {
                            let gap = self.flash_time(aux.ints[1]);
                            self.list[i].flash = (left - 1, Some(now + gap), false);
                        }
                    } else {
                        let on_time = self.flash_time(aux.ints[0]);
                        self.list[i].flash = (left, Some(now + on_time), true);
                        self.requests.push(ObjRequest::Flash(true));
                    }
                }
                fx::OORIENT => {
                    if let Some(l) = self.list[i].link {
                        let offset = self.list[i].aux.as_ref().map_or(0, |a| a.ints[0]) as f32 / ANGLE_UNITS * TAU;
                        let (angle, z, x, y, destroyed) = {
                            let leader = &self.list[l];
                            (leader.angle, leader.z, leader.x, leader.y, leader.destroyed)
                        };
                        let o = &mut self.list[i];
                        if destroyed {
                            o.destroyed = true;
                        } else {
                            (o.angle, o.z, o.x, o.y) = (angle + offset, z, x, y);
                        }
                    }
                }
                fx::OFOLLOW => {
                    // Follow the path of the OPATH holder (or the object's own path) forever.
                    let path = self.list[i].path.or_else(|| self.list[i].link.and_then(|h| self.list[h].path));
                    let Some(nodes) = path.and_then(|p| self.paths.get(p)).filter(|n| !n.is_empty()) else { continue };
                    let node = self.list[i].path_node % nodes.len();
                    let Some(&(tx, ty, _)) = self.points.get(nodes[node]) else { continue };
                    let o = &self.list[i];
                    let (dx, dy) = (tx - o.x, ty - o.y);
                    let dist = (dx * dx + dy * dy).sqrt();
                    let speed = if o.speed > 0.0 { o.speed } else { BOT_MAX_SPEED } / 1.2 * frame_dt;
                    if dist <= o.radius.max(speed) {
                        self.list[i].path_node = (node + 1) % nodes.len();
                        continue;
                    }
                    let step = speed.min(dist);
                    let (mx, my) = (dx / dist * step, dy / dist * step);
                    let o = &mut self.list[i];
                    o.angle = dy.atan2(dx);
                    o.x += mx;
                    o.y += my;
                    if let Some(r) = world.region_at_z(o.x, o.y, Some(o.z + 1.0)) {
                        o.region = Some(r);
                    }
                    if let Some(region) = o.carries {
                        self.requests.push(ObjRequest::CarryRegion { region, dx: mx, dy: my });
                    }
                }
                _ => {}
            }
        }
    }

    /// Start an OSETF/OSETC motion once the session resolved the region by name.
    pub fn start_height_motion(&mut self, object: usize, region: usize, current: f32) {
        let o = &self.list[object];
        let Some(aux) = &o.aux else { return };
        let target = aux.ints[0] as f32;
        let speed = aux.ints[1] as f32;
        let ceiling = o.effect == fx::OSETC;
        if speed <= 0.0 || (target - current).abs() < 0.01 || self.motions.iter().any(|m| m.1 == region && m.4 == ceiling) {
            return;
        }
        let sounds = aux.sounds.clone();
        self.motions.push((object, region, target, speed / 1.2, ceiling));
        self.sound_from(object, sounds[0].clone());
    }

    /// Advance one simulation tick of `dt` seconds.
    pub fn tick(&mut self, level: &mut Level, player: &PlayerView, now: f64, dt: f32) {
        // Floor/ceiling motions run every tick for smoothness.
        let mut finished = Vec::new();
        for (k, &(obj, region, target, speed, ceiling)) in self.motions.iter().enumerate() {
            let r = &mut level.world.regions[region];
            let h = if ceiling { &mut r.ceiling } else { &mut r.floor };
            let step = speed * dt * (target - *h).signum();
            if (target - *h).abs() <= step.abs() {
                *h = target;
                finished.push((k, obj));
            } else {
                *h += step;
            }
        }
        for (k, obj) in finished.into_iter().rev() {
            self.motions.remove(k);
            let stop = self.list[obj].aux.as_ref().and_then(|a| a.sounds[2].clone());
            self.sound_from(obj, stop);
        }

        self.frame_accum += dt * FRAME_RATE;
        while self.frame_accum >= 1.0 {
            self.frame_accum -= 1.0;
            self.frame += 1;
            self.logic_frame(level, player, now);
        }
        self.sync_sprites(level);
    }

    fn logic_frame(&mut self, level: &mut Level, player: &PlayerView, now: f64) {
        let frame_dt = 1.0 / FRAME_RATE;
        // Timers.
        for i in 0..self.list.len() {
            let o = &self.list[i];
            if matches!(o.effect, fx::OTIMER | fx::ORANDOM) && o.deadline.is_some_and(|d| now > d) {
                self.list[i].deadline = None;
                let aux = self.list[i].aux.clone();
                if let Some(target) = aux.as_ref().and_then(|a| a.slot(9)).map(str::to_string) {
                    self.trigger_named(&target, now);
                }
                self.sound_from(i, aux.and_then(|a| a.sounds[4].clone()));
            }
        }

        // Contacts with the player (bump, first frame only) and with player projectiles.
        for i in 0..self.list.len() {
            let o = &self.list[i];
            if !o.in_world() || matches!(o.kind, Kind::Projectile { .. } | Kind::Explosion { .. }) {
                continue;
            }
            let touching = (o.x - player.x).abs() < o.radius + player.radius
                && (o.y - player.y).abs() < o.radius + player.radius
                && o.z <= player.z + player.height
                && player.z <= o.z + o.height;
            let was = o.contact;
            self.list[i].contact = touching;
            self.list[i].cooldown = (self.list[i].cooldown - frame_dt).max(0.0);
            if touching && !was {
                self.bump(i, false, now);
            }
        }

        // Movers: activation by distance, then one logic frame each.
        for i in 0..self.list.len() {
            let o = &self.list[i];
            // Scripts on objects outside every region (PLAYTRN1) run only when explicitly
            // activated (SV(AD=Y)) and are exempt from distance activation.
            let unplaced = o.region.is_none() && o.mover.as_ref().is_some_and(|m| m.active);
            if o.mover.is_none() || o.destroyed || (!o.in_world() && !unplaced) {
                continue;
            }
            let (ox, oy, region) = (o.x, o.y, o.region);
            let near = unplaced || ((ox - player.x).powi(2) + (oy - player.y).powi(2)).sqrt() <= ACTIVATE_DISTANCE || region == Some(player.region);
            let mut out = Vec::new();
            let mask = {
                let m = self.list[i].mover.as_mut().expect("mover");
                if self.frame.is_multiple_of(16) && !m.active && near && !m.finished {
                    m.active = true;
                } else if self.frame % 16 == 8 && m.active && !near {
                    m.active = false;
                }
                m.think(&mut out)
            };
            self.apply_mover_requests(i, out, Some(player));
            let mask = if mask & action::OWN != 0 { self.own_behaviour(i) } else { mask };
            self.move_bot(i, mask, &level.world);
        }

        self.step_sounds(player, now);
        self.step_misc(now, &level.world);

        // Projectiles and explosions.
        self.step_projectiles(level, player, now);
        self.explosion_hits(now);
        self.list.iter_mut().filter(|o| o.destroyed).for_each(|o| o.hidden = true);

        // Player tools.
        self.tool_frame(level, player, now);
    }

    /// Weapon explosions (AMF 0x80) from the player keep hitting bots inside them; the bots'
    /// 240 ms contact debounce limits the rate (a bomb blast lasts ~2.5 s → several hits).
    fn explosion_hits(&mut self, now: f64) {
        let blasts: Vec<(f32, f32, f32, f32, f32)> = self
            .list
            .iter()
            .filter(|o| !o.destroyed && matches!(o.kind, Kind::Explosion { weapon: true, .. }))
            .map(|o| (o.x, o.y, o.z, o.radius, o.height))
            .collect();
        for (x, y, z, r, h) in blasts {
            for t in 0..self.list.len() {
                let o = &self.list[t];
                if o.mover.is_none() || !o.in_world() || !matches!(o.kind, Kind::Normal) {
                    continue;
                }
                if (o.x - x).abs() < o.radius + r && (o.y - y).abs() < o.radius + r && o.z <= z + h && z <= o.z + o.height {
                    self.bump(t, true, now);
                }
            }
        }
    }

    fn bump(&mut self, i: usize, by_projectile: bool, now: f64) {
        let o = &mut self.list[i];
        if let Some(mover) = &mut o.mover {
            if o.cooldown > 0.0 {
                return;
            }
            o.cooldown = BUMP_COOLDOWN;
            mover.bumped(by_projectile);
            return;
        }
        let effect = o.effect;
        let accepts = match effect {
            fx::OBUMPSW | fx::OTELE | fx::OSELTOOL => !by_projectile,
            fx::OTAGSW | fx::OPICKUP | fx::OSOUND | fx::OTEXT | fx::OTIMER | fx::OSTRAP | fx::OSETF | fx::OSETC | fx::OSNDSEQ
            | fx::OWTOW | fx::OOTOO | fx::OFLYON | fx::OFLYOFF | fx::ODROP | fx::OTEXOVER | fx::OFLASH | fx::OVIEWON | fx::OVIEWOFF
            | fx::OCOUNT | fx::OEFFSW | fx::OPHYSON => true,
            _ => false,
        };
        if accepts {
            self.fire(i, Trigger::Bump, now);
        }
    }

    fn apply_mover_requests(&mut self, i: usize, reqs: Vec<MoverRequest>, player: Option<&PlayerView>) {
        for r in reqs {
            match r {
                MoverRequest::Ini(line) => self.requests.push(ObjRequest::Ini(line)),
                MoverRequest::Tag(t) => self.requests.push(ObjRequest::Tag(t)),
                MoverRequest::TagRange(a, b) => self.requests.push(ObjRequest::TagRange(a, b)),
                MoverRequest::Message { color, text } => self.requests.push(ObjRequest::Message { color, text }),
                MoverRequest::Sound(s) => self.sound_from(i, Some(s)),
                MoverRequest::PlayerState { index, op, value } => self.requests.push(ObjRequest::PlayerState { index, op, value }),
                MoverRequest::TurnPlayer { units_per_frame, frames } => self.requests.push(ObjRequest::TurnPlayer { units_per_frame, frames }),
                MoverRequest::FacePlayer { away } => {
                    if let Some(p) = player {
                        let o = &mut self.list[i];
                        o.angle = (p.y - o.y).atan2(p.x - o.x) + if away { std::f32::consts::PI } else { 0.0 };
                    }
                }
                MoverRequest::SetAngle(deg) => self.list[i].angle = deg.to_radians(),
                MoverRequest::Attack { tool, tag } => self.bot_attack(i, tool, tag, player),
                MoverRequest::Spawn(name) => self.spawn_clone(i, &name),
                MoverRequest::ChangeClass(name) => self.change_class(i, &name),
                MoverRequest::Destroy => {
                    self.list[i].destroyed = true;
                }
                MoverRequest::Activate(_) => {}
            }
        }
    }

    /// XL: clone the named object at this object's position.
    fn spawn_clone(&mut self, at: usize, name: &str) {
        let Some(&template) = self.find(name).first() else { return };
        let mut clone = self.list[template].clone();
        let src = &self.list[at];
        (clone.x, clone.y, clone.z, clone.angle, clone.region) = (src.x, src.y, src.z, src.angle, src.region);
        clone.hidden = false;
        clone.destroyed = false;
        clone.sprite = None;
        clone.mover = None;
        clone.name = format!("{}#", clone.name);
        self.list.push(clone);
    }

    /// XN: become a copy of the named template (look, effect, aux and script state).
    fn change_class(&mut self, at: usize, name: &str) {
        let Some(&template) = self.find(name).first() else { return };
        let t = self.list[template].clone();
        let o = &mut self.list[at];
        o.effect = t.effect;
        o.flags = t.flags;
        o.aux = t.aux;
        o.tex = t.tex;
        o.radius = t.radius;
        o.height = t.height;
        o.counter = t.counter;
        o.vx = 0.0;
        o.cooldown = BUMP_COOLDOWN;
        o.mover = t.mover.map(|mut m| {
            m.active = true;
            m
        });
        // The sprite keeps its slot; its frames follow the new Tex in sync_sprites.
        o.kind = Kind::Normal;
        if let Some(s) = o.sprite {
            self.retex.push((s, o.tex));
        }
    }

    /// `MD`: OWALK wanders (forward with random turn bursts); OTHING stands still.
    fn own_behaviour(&mut self, i: usize) -> u32 {
        if self.list[i].effect != fx::OWALK {
            return 0;
        }
        if self.list[i].wander.0 == 0 {
            let r = self.random();
            let turn = if r.is_multiple_of(3) { 0.0 } else if r.is_multiple_of(2) { 0.08 } else { -0.08 };
            self.list[i].wander = (8 + r % 24, turn);
        }
        let o = &mut self.list[i];
        o.wander.0 -= 1;
        o.angle += o.wander.1;
        action::FORWARD
    }

    fn move_bot(&mut self, i: usize, mask: u32, world: &World) {
        let o = &mut self.list[i];
        let scale = 1.0 / 40.0; // original frame step: v·3/120
        if mask & action::FORWARD != 0 {
            o.vx = (o.vx + BOT_ACCEL * scale).min(BOT_MAX_SPEED);
        } else if mask & action::BACKWARD != 0 {
            o.vx = (o.vx - BOT_ACCEL * scale).max(-BOT_MAX_SPEED);
        }
        let floor = o.region.map_or(o.z, |r| world.solid_floor(r).0);
        let airborne = o.z > floor + 0.5;
        let friction = BOT_FRICTION * scale * if airborne { 0.5 } else { 1.0 };
        o.vx -= o.vx.signum() * friction.min(o.vx.abs());
        if mask & action::UP != 0 && !airborne {
            o.vz = BOT_HOP;
        }
        if o.vx.abs() > 0.01 {
            let (s, c) = o.angle.sin_cos();
            let step = o.vx * scale;
            let (nx, ny) = (o.x + c * step, o.y + s * step);
            if let Some(region) = world.region_at_z(nx, ny, Some(o.z + 1.0)) {
                let blocked = world.solid_floor(region).0 > o.z + 32.0 || world.regions[region].ceiling - world.regions[region].floor < o.height;
                if !blocked && !crosses_solid_wall(world, o.x, o.y, nx, ny, o.radius) {
                    o.x = nx;
                    o.y = ny;
                    o.region = Some(region);
                } else {
                    o.vx = 0.0;
                }
            }
        }
        if o.vz != 0.0 || airborne {
            o.z += o.vz * scale;
            o.vz -= BOT_GRAVITY * scale;
            let floor = o.region.map_or(o.z, |r| world.solid_floor(r).0);
            if o.z <= floor {
                o.z = floor;
                o.vz = 0.0;
            }
        }
    }

    fn bot_attack(&mut self, i: usize, tool: usize, tag: Option<u32>, player: Option<&PlayerView>) {
        let Some(def) = self.tools.bot_projectiles.get(tool).cloned().flatten() else { return };
        let o = &self.list[i];
        let mut p = Obj::blank();
        p.name = "_PROJ_".into();
        p.x = o.x + o.angle.cos() * (o.radius + 4.0);
        p.y = o.y + o.angle.sin() * (o.radius + 4.0);
        p.z = o.z + o.height / 2.0;
        p.angle = o.angle;
        p.region = o.region;
        p.tex = def.tex;
        p.radius = PROJECTILE_RADIUS;
        p.height = PROJECTILE_HEIGHT;
        p.vx = def.max_speed[0];
        p.vz = def.max_speed[2] / 2.0;
        if let Some(pl) = player
            && (pl.z - o.z).abs() <= 16.0 {
                p.vz += 0.0;
            }
        let explosion = self.tools.bot_explosions.get(tool).cloned().flatten().map(|_| tool);
        p.kind = Kind::Projectile { from_player: false, tag, flying: def.flying(), life: 6.0, explosion };
        self.sound_at(def.sounds[0].clone(), [p.x, p.y, p.z]);
        self.list.push(p);
    }

    fn fire_player_projectile(&mut self, player: &PlayerView) {
        let t = self.tools.current;
        let Some(def) = self.tools.projectiles[t].clone() else { return };
        self.muzzle_flash = MUZZLE_FLASH_SECONDS;
        let mut p = Obj::blank();
        p.name = "_PPROJ_".into();
        let dist = player.radius + 4.0;
        p.x = player.x + player.angle.cos() * dist;
        p.y = player.y + player.angle.sin() * dist;
        p.z = (player.eye - 16.0).clamp(player.z, player.z + player.height);
        p.angle = player.angle;
        p.region = Some(player.region);
        p.tex = def.tex;
        p.radius = PROJECTILE_RADIUS;
        p.height = PROJECTILE_HEIGHT;
        p.vx = def.max_speed[0];
        p.vz = if def.flying() { p.vx * player.pitch } else { def.max_speed[2] / 2.0 };
        let explosion = self.tools.explosions[t].as_ref().map(|_| t);
        p.kind = Kind::Projectile { from_player: true, tag: None, flying: def.flying(), life: 4.0, explosion };
        self.sound(def.sounds[0].clone());
        self.requests.push(ObjRequest::Tag(910 + t as u32));
        self.list.push(p);
    }

    fn step_projectiles(&mut self, level: &Level, player: &PlayerView, now: f64) {
        let world = &level.world;
        let scale = 1.0 / 40.0;
        let frame_dt = 1.0 / FRAME_RATE;
        for i in 0..self.list.len() {
            let (from_player, tag, flying, explosion) = match &mut self.list[i].kind {
                Kind::Explosion { life, .. } => {
                    *life -= frame_dt;
                    if *life <= 0.0 {
                        self.list[i].destroyed = true;
                    }
                    continue;
                }
                Kind::Projectile { from_player, tag, flying, life, explosion } => {
                    *life -= frame_dt;
                    if *life <= 0.0 {
                        self.list[i].destroyed = true;
                        continue;
                    }
                    (*from_player, *tag, *flying, *explosion)
                }
                Kind::Normal => continue,
            };
            if self.list[i].destroyed {
                continue;
            }
            let o = &self.list[i];
            let (s, c) = o.angle.sin_cos();
            let step = o.vx * scale;
            // Sub-steps of at most 8 units so fast shots do not tunnel.
            let n = (step.abs() / 8.0).ceil().max(1.0) as usize;
            let (mut x, mut y, mut z, mut vz) = (o.x, o.y, o.z, o.vz);
            let mut impact: Option<Option<usize>> = None;
            let mut region = o.region;
            for _ in 0..n {
                let (nx, ny) = (x + c * step / n as f32, y + s * step / n as f32);
                z += vz * scale / n as f32;
                if !flying {
                    vz -= BOT_GRAVITY * scale / n as f32;
                }
                let r = world.region_at_z(nx, ny, Some(z));
                let hit_wall = match r {
                    None => true,
                    Some(r) => {
                        let reg = &world.regions[r];
                        z < world.solid_floor(r).0 || z + PROJECTILE_HEIGHT > world.solid_ceiling(r).0.max(reg.floor + PROJECTILE_HEIGHT)
                            || crosses_solid_wall(world, x, y, nx, ny, 1.0)
                    }
                };
                if hit_wall {
                    impact = Some(None);
                    break;
                }
                region = r;
                (x, y) = (nx, ny);
                // Object hits.
                if from_player {
                    let target = self.list.iter().enumerate().position(|(j, t)| {
                        j != i && t.in_world() && matches!(t.kind, Kind::Normal) && t.mover.is_some() && (t.x - x).abs() < t.radius + PROJECTILE_RADIUS
                            && (t.y - y).abs() < t.radius + PROJECTILE_RADIUS && z <= t.z + t.height && t.z <= z + PROJECTILE_HEIGHT
                    });
                    if let Some(t) = target {
                        impact = Some(Some(t));
                        break;
                    }
                    let switch = self.list.iter().enumerate().position(|(j, t)| {
                        j != i && t.in_world() && t.effect == fx::OTAGSW && (t.x - x).abs() < t.radius + PROJECTILE_RADIUS && (t.y - y).abs() < t.radius + PROJECTILE_RADIUS
                    });
                    if let Some(t) = switch {
                        impact = Some(Some(t));
                        break;
                    }
                } else if (player.x - x).abs() < player.radius + PROJECTILE_RADIUS
                    && (player.y - y).abs() < player.radius + PROJECTILE_RADIUS
                    && z <= player.z + player.height
                    && player.z <= z + PROJECTILE_HEIGHT
                {
                    if let Some(t) = tag {
                        self.requests.push(ObjRequest::PlayerHit(t));
                    }
                    impact = Some(None);
                    break;
                }
            }
            let o = &mut self.list[i];
            (o.x, o.y, o.z, o.vz, o.region) = (x, y, z, vz, region);
            if let Some(target) = impact {
                o.destroyed = true;
                let (ex, ey, ez, er) = (x, y, z, region);
                if let Some(t) = target {
                    self.bump(t, true, now);
                }
                if from_player {
                    self.requests.push(ObjRequest::Tag(930 + self.tools.current as u32));
                }
                let def = explosion.and_then(|e| if from_player { self.tools.explosions[e].clone() } else { self.tools.bot_explosions[e].clone() });
                if let Some(def) = def {
                    let mut e = Obj::blank();
                    e.name = "_XPLD_".into();
                    (e.x, e.y, e.z, e.region) = (ex, ey, ez - 8.0, er);
                    e.tex = def.tex;
                    e.radius = def.radius;
                    e.height = def.height;
                    // Lives until its animation ends (falls back to 0.6 s).
                    let life = def.tex.and_then(|t| level.sequences.get(t)?.as_ref()).map(|s| sequence_seconds(s).max(1.0 / FRAME_RATE)).unwrap_or(0.6);
                    e.kind = Kind::Explosion { life, weapon: from_player && def.amf & 0x80 != 0 };
                    let snd = def.sounds[if target.is_some() { 0 } else { 2 }].clone().or(def.sounds[0].clone());
                    self.sound_at(snd, [e.x, e.y, e.z]);
                    self.list.push(e);
                }
            }
        }
    }

    /// Player tool activation (Use) is requested by the session; this advances cooldown and
    /// the weapon animation, releasing the shot at the sequence entry marked 99.
    fn tool_frame(&mut self, level: &Level, player: &PlayerView, _now: f64) {
        let frame_dt = 1.0 / FRAME_RATE;
        self.muzzle_flash = (self.muzzle_flash - frame_dt).max(0.0);
        self.tools.cooldown = (self.tools.cooldown - frame_dt).max(0.0);
        let mut release = false;
        if let Some((tex, anim)) = &mut self.tools.anim
            && let Some(seq) = level.sequences.get(*tex).and_then(Option::as_ref) {
                let before = anim.finished;
                anim.advance(seq, frame_dt);
                if anim.finished && !before {
                    anim.finished = false;
                    release = true;
                }
            }
        if release || self.tools.shoot {
            self.tools.shoot = false;
            self.fire_player_projectile(player);
        }
    }

    /// Use key: start the current tool (cost check, use animation, sound).
    /// `states` are the player's OS values (OTOOLCON deducts from them).
    pub fn use_tool(&mut self, states: &mut [i32; 16], level: &Level) {
        if self.tools.cooldown > 0.0 {
            return;
        }
        let t = self.tools.current;
        let Some(def) = self.tools.defs[t].clone() else { return };
        // NUMS[3] >= 1: the press only counts once the use animation is back at rest
        // (the shotgun's 120 cu cycle allows one shot per 1.44 s).
        if def.nums[3] >= 1
            && let Some((tex, anim)) = &self.tools.anim
            && level.sequences.get(*tex).and_then(Option::as_ref).is_some_and(|seq| !anim.is_idle(seq))
        {
            return;
        }
        if let Some((state, need, delta, fail_tag)) = self.tools.costs[t] {
            if states[state] < need {
                self.tools.cooldown = 0.36;
                if fail_tag > 0 {
                    self.requests.push(ObjRequest::Tag(fail_tag));
                }
                return;
            }
            states[state] += delta;
        }
        self.tools.cooldown = 0.36;
        self.sound(def.sounds[0].clone().or(def.sounds[4].clone()));
        let animated = self.tools.anim.as_mut().and_then(|(tex, anim)| {
            let seq = level.sequences.get(*tex)?.as_ref()?;
            let has_release = seq.entries.iter().any(|e| e.next == 99);
            if has_release {
                *anim = anim::Player::start(seq, 1);
            }
            Some(has_release)
        });
        if animated != Some(true) {
            self.tools.shoot = true;
        }
    }

    /// Pic currently shown for the weapon view, if any.
    pub fn weapon_pic(&self, level: &Level) -> Option<usize> {
        let (tex, anim) = self.tools.anim.as_ref()?;
        let seq = level.sequences.get(*tex)?.as_ref()?;
        seq.rotations_of(anim.entry).first().map(|&(p, _)| p)
    }

    /// Keep world sprites in step with objects (position, visibility, new objects, Tex changes).
    fn sync_sprites(&mut self, level: &mut Level) {
        for (sprite, tex) in std::mem::take(&mut self.retex) {
            level.set_sprite_tex(sprite, tex);
        }
        for o in &mut self.list {
            if o.sprite.is_none() && !o.destroyed && !o.hidden && o.region.is_some()
                && let Some(tex) = o.tex {
                    o.sprite = level.add_sprite(Sprite {
                        x: o.x,
                        y: o.y,
                        z: o.z,
                        angle: o.angle,
                        region: o.region.unwrap_or(0),
                        frames: Vec::new(),
                        scale: 1.0,
                        visible: true,
                        height: o.height,
                    }, tex);
                }
            let Some(s) = o.sprite else { continue };
            if let Some(sp) = level.world.sprites.get_mut(s) {
                sp.x = o.x;
                sp.y = o.y;
                sp.z = o.z;
                sp.angle = o.angle;
                sp.region = o.region.unwrap_or(sp.region);
                sp.visible = o.in_world() && !sp.frames.is_empty();
                sp.height = o.height;
            }
        }
    }
}

/// Audio channel used by an object's looping sound.
fn channel(object: usize) -> u32 {
    1000 + object as u32
}

/// OSOUND volume 1..1000 (0 = full) as amplitude.
fn volume(v: i32) -> f32 {
    if v <= 0 { 1.0 } else { v.min(1000) as f32 / 1000.0 }
}

/// Play time of an animation from entry 0 until it holds or ends (explosion lifetime).
fn sequence_seconds(seq: &Sequence) -> f32 {
    // Finished as soon as the current entry loops on itself, returns to entry 0 or holds
    // (combat.md §4.1): BLACK (entry 0 → 0) lasts one frame.
    let mut total = 0.0;
    let mut entry = 0;
    for _ in 0..seq.entries.len() {
        let e = seq.entries[entry];
        let next = match e.next {
            98 => break,
            99 => entry + 1,
            n => n as usize,
        };
        if next == entry || next == 0 || next >= seq.entries.len() {
            break;
        }
        total += e.delay;
        entry = next;
    }
    total
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Trigger {
    Tag,
    Named,
    Bump,
}

/// Whether moving from (x0,y0) to (x1,y1) crosses a blocking (solid one-sided) wall.
fn crosses_solid_wall(world: &World, x0: f32, y0: f32, x1: f32, y1: f32, _radius: f32) -> bool {
    world.walls.iter().any(|w| {
        if !(w.blocking || w.back.is_none()) {
            return false;
        }
        let (ax, ay) = world.vertices[w.a];
        let (bx, by) = world.vertices[w.b];
        segments_cross((x0, y0), (x1, y1), (ax, ay), (bx, by))
    })
}

fn segments_cross(p: (f32, f32), q: (f32, f32), a: (f32, f32), b: (f32, f32)) -> bool {
    let cross = |o: (f32, f32), u: (f32, f32), v: (f32, f32)| (u.0 - o.0) * (v.1 - o.1) - (u.1 - o.1) * (v.0 - o.0);
    let d1 = cross(a, b, p);
    let d2 = cross(a, b, q);
    let d3 = cross(p, q, a);
    let d4 = cross(p, q, b);
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
}
