//! Platform-independent core of the Exploding Lips reimplementation.
//!
//! Front ends (desktop, web) own the window, audio device and input devices; they feed
//! [`Input`] into [`Game::tick`] at [`TICK_RATE`] and present the [`Frame`] that
//! [`Game::render`] fills, using [`Game::palette`] to turn indices into colours.

pub mod anim;
pub mod automap;
pub mod bsp_world;
mod audio;
pub use engine_core::clock;
pub mod effects;
pub mod fixes;
pub mod guides;
pub mod route;
pub use engine_core::frame;
pub mod hud;
mod input;
pub mod level;
pub mod menus;
pub mod mover;
pub mod module;
pub mod objects;
pub mod options;
pub mod player;
pub mod render;
pub mod saves;
pub mod scene;
pub mod script;
pub mod session;
pub mod text;
pub use engine_core::vfs;
pub mod world;

pub use audio::{AudioEvent, music_path, sound_path, wav_seconds};
pub use engine_core::{FixedClock, Frame};
pub use input::{Action, Input};
pub use engine_core::Vfs;

use godd_formats::bmp::Bmp;
use godd_formats::pal::Palette;
use player::PhysicsParams;
use render::{Camera, RenderSettings, Renderer, Shading, Viewport};
use session::{Entry, Session};

/// Simulation rate in ticks per second.
pub const TICK_RATE: f64 = 60.0;
/// The original's logical screen (3D view and HUD).
pub const CLASSIC_WIDTH: usize = 320;
pub const CLASSIC_HEIGHT: usize = 200;
/// 320x200 was shown on 4:3 displays: each pixel is 1.2 times taller than wide.
pub const CLASSIC_PIXEL_ASPECT: f32 = 1.2;
/// Shade levels in PALETTE.PAL (level 0 brightest, 15 black); a blend table follows them.
const SHADE_LEVELS: usize = 16;

pub type BoxError = Box<dyn std::error::Error>;

/// Radians per tick at full right-stick deflection (yaw, pitch) before `joy_sensitivity`.
const STICK_LOOK: (f32, f32) = (0.06, 0.035);

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Skip the title and load this level (e.g. "LIPS00").
    pub start_level: Option<String>,
    /// Character save file contents (front ends keep it outside the game folder). When
    /// absent, LIPSTART.GAM from the game folder is used.
    pub save: Option<Vec<u8>>,
}

enum State {
    Title,
    Playing(Box<Session>),
}

pub struct Game {
    title: Bmp,
    game_palette: [[u8; 3]; 256],
    shade_rows: Vec<[u8; 256]>,
    state: State,
    renderer: Renderer,
    pub settings: RenderSettings,
    physics: PhysicsParams,
    audio: Vec<AudioEvent>,
    hud: hud::Hud,
    quit: bool,
    /// Snapshot of the option cvars (refreshed every tick).
    pub options: options::GameOptions,
    pub options_menu: options::OptionsMenu,
    /// Console variables (the game's options plus whatever the front end registers).
    pub cvars: engine_core::CvarRegistry,
    pub binds: engine_core::BindTable,
    /// Config state last handed to the front end by [`Game::take_config`].
    saved_config: (u64, engine_core::BindTable),
    /// Toggle-crouch state.
    crouch_latched: bool,
    /// Mouse look not yet consumed by a tick (yaw radians, look units), shown by the camera.
    pending_look: (f32, f32),
    /// Width / height of the GPU view, recorded by [`Game::ui_layer_size`]: the UI layer is
    /// rounded up to whole pixels, so its own aspect is a little off (LIPS01's facades split by
    /// sky seams near the screen edges, where portal windows came out a few pixels short).
    view_aspect: std::cell::Cell<Option<f32>>,
    /// Bumped whenever a level is loaded (GPU renderers re-upload images and geometry).
    epoch: u64,
    /// Triangulated regions of the current level, for the GPU scene.
    scene_cache: Option<(u64, Vec<scene::RegionMesh>)>,
    /// Rumble requests for the front end, and what they react to.
    haptics: Vec<engine_core::HapticEvent>,
    last_vitals: (i32, i32, f32),
    last_objects: usize,
    /// Portal hops from the listener's region to every region, cached per (level, region).
    hops_cache: ((u64, usize), Vec<u8>),
    /// Reverb of the listener's surroundings, cached per (level, region).
    reverb_cache: ((u64, usize), f32),
    /// Modern lighting fitted from the palette: brightness per shade level, emissive colours.
    light_fit: ([f32; 16], [u32; 8]),
    ui_colors: (u8, u8, u8, u8),
    fps: FpsCounter,
    /// GPU time of a recent frame (milliseconds), reported by the front end for `r_stats`.
    pub gpu_ms: Option<f32>,
    /// The last GPU frame showed the BSP world (its images are uploaded).
    bsp_shown: bool,
    /// Ticks since the game was created (the original HUD clock is time since launch).
    uptime_ticks: u64,
    /// Character save slots, carried across levels.
    save: godd_formats::gam::SaveFile,
    current_slot: Option<usize>,
    save_dirty: bool,
    /// The engine console (~): config commands, `warp LEVEL`, and INI commands like the original.
    console: engine_core::console::Console,
    /// Save slots and which changed since the front end last stored them.
    slots: [Option<saves::SaveSlot>; saves::SLOTS],
    slots_dirty: [bool; saves::SLOTS],
    /// Quicksave contents and whether they changed since the front end last stored them.
    quicksave: Option<String>,
    /// The input since the current level was entered: quicksaves replay it to rebuild the level
    /// exactly (doors, triggers, pickups, enemies), not only position and states.
    recording: Option<Recording>,
    /// Recordings of levels the player finished (level, replay text), for front ends to keep:
    /// route guides can be made from a player's own run.
    finished_replays: Vec<(String, String)>,
    quicksave_dirty: bool,
    /// F2 automap overlay.
    show_map: bool,
    automap: automap::Automap,
    /// Recorded routes by level name (upper case), for the route guide (`hud_route`); the front
    /// end loads them from `routes/` folders.
    pub routes: std::collections::HashMap<String, route::Route>,
    /// The level the route progress is for, and how far along it the player is.
    route_progress: (String, route::RouteProgress),
}

/// One frame for a GPU renderer (see [`Game::gpu_frame`]).
pub struct GpuView {
    pub camera: engine_scene::scene::SceneCamera,
    pub shading: engine_scene::scene::Shading,
    pub scene: engine_scene::scene::SceneFrame,
    /// Changes when a new level is loaded: re-upload [`Game::world_images`].
    pub epoch: u64,
}


/// How the current level was entered and every tick's input since (replay quicksaves).
#[derive(Clone)]
struct Recording {
    level: String,
    entry: Entry,
    /// The script carried in from the previous level (`Entry::Continue`).
    carried: Option<script::Script>,
    /// The start object a level change placed the player at.
    place: Option<String>,
    /// A LIPSTART "go to game": slot, states copied in, and the slot's position and inventory.
    goto: Option<GotoSetup>,
    inputs: Vec<Input>,
}

#[derive(Clone)]
struct GotoSetup {
    slot: Option<usize>,
    states: Option<[i32; script::STATE_COUNT]>,
    restore: Option<(f32, f32, f32, f32, Vec<String>)>,
}

const REPLAY_HEADER: &str = "REPLAY 1";

impl Recording {
    fn to_text(&self) -> String {
        let mut out = format!("{REPLAY_HEADER}\nlevel {}\nentry {}\n", self.level, if self.entry == Entry::Continue { "continue" } else { "fresh" });
        if let Some(name) = &self.place {
            out += &format!("place {name}\n");
        }
        if let Some(g) = &self.goto {
            out += &format!("goto {}\n", g.slot.map_or("-".to_string(), |n| n.to_string()));
            if let Some(st) = g.states {
                out += &format!("gotostates {}\n", st.iter().map(i32::to_string).collect::<Vec<_>>().join(" "));
            }
            if let Some((x, y, z, a, inv)) = &g.restore {
                out += &format!("gotorestore {x} {y} {z} {a} {}\n", inv.join(" "));
            }
        }
        if let Some(c) = &self.carried {
            out += "script\n";
            out += &c.to_text();
            out += "endscript\n";
        }
        // Runs of identical ticks as one line: "inputs N" then "COUNT LINE".
        let lines: Vec<String> = self.inputs.iter().map(Input::to_line).collect();
        out += &format!("inputs {}\n", lines.len());
        let mut i = 0;
        while i < lines.len() {
            let run = lines[i..].iter().take_while(|l| **l == lines[i]).count();
            out += &format!("{run} {}\n", lines[i]);
            i += run;
        }
        out
    }

    fn parse(text: &str) -> Option<Recording> {
        let mut lines = text.lines().skip_while(|l| *l != REPLAY_HEADER).skip(1);
        let mut rec = Recording { level: String::new(), entry: Entry::Fresh, carried: None, place: None, goto: None, inputs: Vec::new() };
        while let Some(line) = lines.next() {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "level" => rec.level = rest.to_string(),
                "entry" => rec.entry = if rest == "continue" { Entry::Continue } else { Entry::Fresh },
                "place" => rec.place = Some(rest.to_string()),
                "goto" => rec.goto = Some(GotoSetup { slot: rest.parse().ok(), states: None, restore: None }),
                "gotostates" => {
                    let v: Vec<i32> = rest.split_whitespace().map(str::parse).collect::<Result<_, _>>().ok()?;
                    rec.goto.as_mut()?.states = Some(v.try_into().ok()?);
                }
                "gotorestore" => {
                    let w: Vec<&str> = rest.split_whitespace().collect();
                    let f = |i: usize| w.get(i).and_then(|v| v.parse::<f32>().ok());
                    let inv = w.iter().skip(4).map(|s| s.to_string()).collect();
                    rec.goto.as_mut()?.restore = Some((f(0)?, f(1)?, f(2)?, f(3)?, inv));
                }
                "script" => {
                    let body: Vec<&str> = lines.by_ref().take_while(|l| *l != "endscript").collect();
                    rec.carried = Some(script::Script::from_text(&body.join("\n"))?);
                }
                "inputs" => {
                    let total: usize = rest.parse().ok()?;
                    while rec.inputs.len() < total {
                        let (count, input) = lines.next()?.split_once(' ')?;
                        let input = Input::from_line(input)?;
                        for _ in 0..count.parse::<usize>().ok()? {
                            rec.inputs.push(input.clone());
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
        (!rec.level.is_empty()).then_some(rec)
    }
}

/// Collects a route guide while a recording replays (`Game::route_from_replay`).
#[derive(Default)]
struct RouteRecorder {
    route: route::Route,
    trail: Vec<[f32; 3]>,
}

impl RouteRecorder {
    fn note(&mut self, s: &Session, walls: &[usize], bumped: &[usize], entered: &[usize]) {
        use route::{RouteStep, RouteTarget};
        let p = &s.player;
        if !s.in_bsp && self.trail.last().is_none_or(|q| (q[0] - p.x).hypot(q[1] - p.y) >= 48.0) {
            self.trail.push([p.x, p.y, p.z]);
        }
        let world = &s.level.world;
        let triggers: Vec<usize> = s.effects.triggers().iter().map(|t| t.0).collect();
        let mut steps: Vec<(RouteTarget, [f32; 3])> = Vec::new();
        for &w in walls {
            let wall = &world.walls[w];
            let ((ax, ay), (bx, by)) = (world.vertices[wall.a], world.vertices[wall.b]);
            steps.push((RouteTarget::Wall(w), [(ax + bx) / 2.0, (ay + by) / 2.0, p.z]));
        }
        for &i in bumped {
            let Some(o) = s.objects.list.get(i) else { continue };
            // (Things used, not bots met: pickups, teleporters, bump and tag switches.)
            if o.mover.is_none() && matches!(o.effect, 130 | 15 | 20 | 22) {
                steps.push((RouteTarget::Object(i), [o.x, o.y, o.z]));
            }
        }
        for &r in entered.iter().filter(|r| triggers.contains(r)) {
            let ((x0, y0), (x1, y1)) = world.regions[r].bounds;
            steps.push((RouteTarget::Region(r), [(x0 + x1) / 2.0, (y0 + y1) / 2.0, world.regions[r].floor]));
        }
        for (target, pos) in steps {
            if self.route.steps.last().is_some_and(|last| last.target == target) {
                continue;
            }
            let trail = route::untangle(&std::mem::take(&mut self.trail), 40.0);
            self.route.steps.push(RouteStep { target, pos, trail });
        }
    }
}

#[derive(Default)]
struct FpsCounter {
    frames: u32,
    ticks: u32,
    shown: u32,
}

impl Game {
    pub fn new(vfs: &Vfs, options: &Options) -> Result<Self, BoxError> {
        let title = Bmp::parse(vfs.read("lips.bmp")?)?;
        let palette = Palette::parse(vfs.read("PALETTE.PAL")?)?;
        let shade_rows = palette.colormaps.iter().take(SHADE_LEVELS).copied().collect();
        let mut game = Self {
            title,
            hud: hud::Hud::new(),
            game_palette: palette.rgb,
            shade_rows,
            state: State::Title,
            renderer: Renderer::default(),
            settings: RenderSettings::default(),
            physics: PhysicsParams::default(),
            audio: Vec::new(),
            quit: false,
            options: options::GameOptions::default(),
            options_menu: options::OptionsMenu::default(),
            cvars: {
                let mut c = engine_core::CvarRegistry::default();
                options::register_cvars(&mut c);
                c
            },
            binds: options::default_binds(options::Profile::Modern),
            saved_config: (0, engine_core::BindTable::default()),
            crouch_latched: false,
            pending_look: (0.0, 0.0),
            view_aspect: std::cell::Cell::new(None),
            epoch: 0,
            scene_cache: None,
            reverb_cache: ((u64::MAX, 0), 0.0),
            hops_cache: ((u64::MAX, 0), Vec::new()),
            haptics: Vec::new(),
            last_vitals: (0, 0, 0.0),
            last_objects: usize::MAX,
            light_fit: engine_scene::module::fit_light(&palette.rgb, &palette.colormaps[..SHADE_LEVELS.min(palette.colormaps.len())]),
            ui_colors: (
                text::nearest_color(&palette.rgb, [10, 10, 40]),
                text::nearest_color(&palette.rgb, [24, 24, 252]),
                text::nearest_color(&palette.rgb, [200, 200, 200]),
                text::nearest_color(&palette.rgb, [255, 40, 40]),
            ),
            fps: FpsCounter::default(),
            gpu_ms: None,
            bsp_shown: false,
            uptime_ticks: 0,
            routes: std::collections::HashMap::new(),
            route_progress: (String::new(), route::RouteProgress::default()),
            save: options
                .save
                .as_deref()
                .or_else(|| vfs.read("LIPSTART.GAM").ok())
                .and_then(|b| godd_formats::gam::SaveFile::parse(b).ok())
                .unwrap_or_default(),
            current_slot: None,
            save_dirty: false,
            console: engine_core::console::Console::default(),
            slots: Default::default(),
            slots_dirty: [false; saves::SLOTS],
            quicksave: None,
            quicksave_dirty: false,
            recording: None,
            finished_replays: Vec::new(),
            show_map: false,
            automap: automap::Automap::default(),
        };
        game.saved_config = (game.cvars.generation(), game.binds.clone());
        if let Some(name) = &options.start_level {
            game.start_level(vfs, name, Entry::Fresh)?;
        }
        Ok(game)
    }

    /// Take over a registry and bind table a front end prepared (its own cvars registered and
    /// its config file loaded) before the game existed.
    pub fn adopt_config(&mut self, cvars: engine_core::CvarRegistry, binds: engine_core::BindTable) {
        self.cvars = cvars;
        options::register_cvars(&mut self.cvars); // no-op for known names; adds any missing
        self.binds = binds;
        self.options = options::GameOptions::from_cvars(&self.cvars);
        self.saved_config = (self.cvars.generation(), self.binds.clone());
    }

    /// Apply a config file (`set`/`bind` lines). Returns its errors for logging.
    pub fn exec_config(&mut self, text: &str) -> Vec<String> {
        let errors = engine_core::config::exec_config(&mut self.cvars, &mut self.binds, text);
        self.options = options::GameOptions::from_cvars(&self.cvars);
        self.saved_config = (self.cvars.generation(), self.binds.clone());
        errors
    }

    /// The config file text when options or bindings changed since the last call.
    pub fn take_config(&mut self) -> Option<String> {
        if self.saved_config.0 == self.cvars.generation() && self.saved_config.1 == self.binds {
            return None;
        }
        self.saved_config = (self.cvars.generation(), self.binds.clone());
        Some(self.config_text())
    }

    pub fn config_text(&self) -> String {
        engine_core::config::write_config(&self.cvars, &self.binds, "Exploding Lips config (written by the game; `set` and `bind` lines)")
    }

    /// The camera the view would be rendered from at `alpha` (between the last two ticks).
    pub fn view_camera(&self, alpha: f32) -> Option<render::Camera> {
        self.session().map(|s| camera(s, &self.physics, alpha, self.pending_look))
    }

    /// Show mouse motion that the next tick will consume, so looking is 1:1 at any frame rate.
    pub fn preview_look(&mut self, input: &Input) {
        let (yaw, pitch) = self.mouse_radians(input.mouse);
        self.pending_look = (-yaw, -PhysicsParams::pitch_to_look(pitch));
    }

    /// Mouse counts to (yaw, pitch) radians with the sensitivity, vertical scale and invert.
    fn mouse_radians(&self, (dx, dy): (f32, f32)) -> (f32, f32) {
        let o = &self.options;
        let invert = if o.invert_mouse { -1.0 } else { 1.0 };
        (dx * o.mouse_sensitivity, dy * o.mouse_sensitivity * o.mouse_yscale * invert)
    }

    /// The input the simulation sees: options applied (autorun, toggle crouch, look scaling).
    fn effective_input(&mut self, input: &Input) -> Input {
        let mut eff = input.clone();
        let o = self.options;
        if o.autorun {
            eff.set(Action::Run, !input.held(Action::Run));
        }
        if o.toggle_crouch {
            if input.pressed(Action::Crouch) {
                self.crouch_latched = !self.crouch_latched;
            }
            eff.set(Action::Crouch, self.crouch_latched);
        } else {
            self.crouch_latched = false;
        }
        let (yaw, pitch) = self.mouse_radians(input.mouse);
        let stick_invert = if o.invert_stick { -1.0 } else { 1.0 };
        eff.look_delta.0 += yaw + input.look_axis.0 * STICK_LOOK.0 * o.stick_speed;
        eff.look_delta.1 += pitch - input.look_axis.1 * STICK_LOOK.1 * o.stick_speed * stick_invert;
        eff.mouse = (0.0, 0.0);
        eff
    }

    /// Load a level. `Entry::Continue` carries the player's states over from the current one.
    pub fn start_level(&mut self, vfs: &Vfs, name: &str, entry: Entry) -> Result<(), BoxError> {
        let carried = match (&self.state, entry) {
            (State::Playing(s), Entry::Continue) => Some(s.script.clone()),
            _ => None,
        };
        self.start_level_with(vfs, name, entry, carried)
    }

    /// Load a level with `carried` as the script brought in from the previous one.
    fn start_level_with(&mut self, vfs: &Vfs, name: &str, entry: Entry, carried: Option<script::Script>) -> Result<(), BoxError> {
        if let State::Playing(old) = &mut self.state {
            self.save = std::mem::take(&mut old.save);
            self.current_slot = old.menus.current_slot;
            self.save_dirty |= old.save_dirty;
        }
        let save = std::mem::take(&mut self.save);
        let session = Session::start(vfs, name, entry, carried.clone(), save, self.current_slot)?;
        self.recording = Some(Recording { level: name.to_string(), entry, carried, place: None, goto: None, inputs: Vec::new() });
        self.state = State::Playing(Box::new(session));
        self.epoch += 1;
        let regions = self.session().map_or(0, |s| s.level.world.regions.len());
        self.automap.reset(regions);
        Ok(())
    }

    fn console_input(&mut self, vfs: &Vfs, input: &Input) {
        const COMMANDS: [&str; 12] = ["set", "get", "toggle", "reset", "cvarlist", "bind", "unbind", "unbindall", "warp", "god", "noclip", "help"];
        let words: Vec<&str> = COMMANDS.iter().copied().chain(self.cvars.iter().map(|v| v.name)).collect();
        self.console.edit(&input.text, &words);
        if input.pressed(Action::Menu) {
            self.console.open = false;
        }
        if input.pressed(Action::MenuUp) {
            self.console.history(true);
        }
        if input.pressed(Action::MenuDown) {
            self.console.history(false);
        }
        if input.pressed(Action::LookUp) {
            self.console.scroll_by(5);
        }
        if input.pressed(Action::LookDown) {
            self.console.scroll_by(-5);
        }
        if !input.pressed(Action::MenuSelect) {
            return;
        }
        let Some(line) = self.console.submit() else { return };
        use engine_core::config::{Outcome, exec_line};
        match exec_line(&mut self.cvars, &mut self.binds, &line) {
            Outcome::Unknown => {}
            outcome => {
                let reply = match outcome {
                    Outcome::Done(Some(text)) => text,
                    Outcome::Done(None) => "ok".into(),
                    Outcome::Error(e) => e,
                    Outcome::Unknown => unreachable!(),
                };
                self.console.print(&reply);
                self.options = options::GameOptions::from_cvars(&self.cvars);
                return;
            }
        }
        let mut words = line.split_whitespace();
        let reply = match words.next().map(str::to_ascii_lowercase).as_deref() {
            Some("warp") => match words.next() {
                Some(level) => match self.start_level(vfs, &level.to_ascii_uppercase(), Entry::Continue) {
                    Ok(()) => format!("warped to {}", level.to_ascii_uppercase()),
                    Err(e) => format!("cannot load {level}: {e}"),
                },
                None => "usage: warp LIPSnn".into(),
            },
            // (Console changes to the world are not input: later quicksaves use the snapshot.)
            Some("god") => match self.session_mut() {
                Some(s) => {
                    s.god = !s.god;
                    format!("god mode {}", if s.god { "on" } else { "off" })
                }
                None => "no level loaded".into(),
            },
            Some("noclip") => match self.session_mut() {
                Some(s) => {
                    s.noclip = !s.noclip;
                    if !s.noclip {
                        // (Back to walking: falling from where we flew to, or out of a wall.)
                        s.player.flying = false;
                    }
                    format!("noclip {}", if s.noclip { "on (Jump/Crouch: up/down)" } else { "off" })
                }
                None => "no level loaded".into(),
            },
            Some("help") => "warp LEVEL | god | noclip | set/get/toggle/reset/cvarlist | bind/unbind | INI commands, e.g. TAG=500 | Tab completes, Up/Down history, PgUp/PgDn scroll".into(),
            _ => match self.session_mut() {
                Some(s) => {
                    s.run_command(&line);
                    "done".into()
                }
                None => "no level loaded".into(),
            },
        };
        if !line.trim_start().to_ascii_lowercase().starts_with("warp") {
            self.recording = None;
        }
        self.console.print(&reply);
    }

    /// The current level, position, states and inventory as text (quicksave and save slots).
    fn snapshot(&self) -> Option<String> {
        let s = self.session()?;
        let p = &s.player;
        let states: Vec<String> = s.script.states.iter().map(i32::to_string).collect();
        let inventory: Vec<String> = s.objects.inventory.iter().map(|&i| s.objects.list[i].name.clone()).collect();
        // View state last (`| view LOOK CROUCHING`), so older saves without it still load.
        Some(format!("{} {} {} {} {} {} | {} | view {} {}", s.name, p.x, p.y, p.z, p.angle, states.join(" "), inventory.join(" "), p.look, p.crouching as u8))
    }

    /// Restart the snapshot's level, then put back position, states and inventory.
    fn restore(&mut self, vfs: &Vfs, text: &str) -> bool {
        if let Some(rec) = Recording::parse(text) {
            if self.replay(vfs, rec) {
                // The snapshot line still wins where the world was changed other than by input
                // (tools, tests): its states, and its position if the replay ended elsewhere.
                let head = text.lines().next().unwrap_or("").split('|').next().unwrap_or("");
                let parts: Vec<&str> = head.split_whitespace().collect();
                if parts.len() >= 5 + script::STATE_COUNT
                    && let Some(s) = self.session_mut()
                {
                    for (k, v) in parts[5..5 + script::STATE_COUNT].iter().enumerate() {
                        s.script.states[k] = v.parse().unwrap_or(s.script.states[k]);
                    }
                    let f = |i: usize| parts[i].parse::<f32>().unwrap_or(0.0);
                    if (s.player.x - f(1)).hypot(s.player.y - f(2)) > 1.0 || (s.player.z - f(3)).abs() > 1.0 {
                        log::warn!("quicksave replay ended away from the save; moving to it");
                        s.restore_position(f(1), f(2), f(3), f(4));
                    }
                }
                return true;
            }
            log::warn!("quicksave replay failed; restoring the snapshot");
        }
        let text = text.lines().next().unwrap_or("");
        let (head, rest) = text.split_once('|').unwrap_or((text, ""));
        let (inventory, view) = rest.split_once('|').unwrap_or((rest, ""));
        let parts: Vec<&str> = head.split_whitespace().collect();
        if parts.len() < 5 + script::STATE_COUNT {
            return false;
        }
        let f = |i: usize| parts[i].parse::<f32>().unwrap_or(0.0);
        if self.start_level(vfs, parts[0], Entry::Fresh).is_err() {
            return false;
        }
        let Some(s) = self.session_mut() else { return false };
        for (k, v) in parts[5..5 + script::STATE_COUNT].iter().enumerate() {
            s.script.states[k] = v.parse().unwrap_or(0);
        }
        s.restore_position(f(1), f(2), f(3), f(4));
        let items: Vec<String> = inventory.split_whitespace().map(str::to_string).collect();
        s.restore_inventory(&items);
        let view: Vec<&str> = view.split_whitespace().collect();
        if let ["view", look, crouching, ..] = view[..] {
            s.player.look = look.parse().unwrap_or(0.0);
            s.player.crouching |= crouching == "1";
            s.previous = s.player;
        }
        true
    }

    /// F6: remember the current level, position, states and inventory.
    fn quick_save(&mut self) {
        let Some(mut text) = self.snapshot() else { return };
        if let Some(rec) = &self.recording {
            text = format!("{text}\n{}", rec.to_text());
        }
        self.quicksave = Some(text);
        self.quicksave_dirty = true;
        if let Some(s) = self.session_mut() {
            s.show_message(script::DEFAULT_MESSAGE_COLOR, "Quick saved");
        }
    }

    /// F7: restore the quicksave (the level restarts, then position/states/inventory return).
    /// Load the quicksave now (front ends: `--quickload`, to start where a player saved).
    pub fn load_quicksave(&mut self, vfs: &Vfs) {
        self.quick_load(vfs);
    }

    fn quick_load(&mut self, vfs: &Vfs) {
        let Some(text) = self.quicksave.clone() else { return };
        if self.restore(vfs, &text)
            && let Some(s) = self.session_mut() {
                s.show_message(script::DEFAULT_MESSAGE_COLOR, "Quick loaded");
            }
    }

    /// Save into slot `n` (0-based) with a thumbnail of the current view.
    pub fn save_slot(&mut self, n: usize) {
        if n >= saves::SLOTS {
            return;
        }
        let Some(mut snapshot) = self.snapshot() else { return };
        if let Some(rec) = &self.recording {
            snapshot = format!("{snapshot}\n{}", rec.to_text());
        }
        let State::Playing(s) = &self.state else { return };
        let label = format!("{}  {}", s.name, s.level.title.trim());
        let mut thumb = Frame::new(saves::THUMB_W, saves::THUMB_H);
        thumb.palette = self.game_palette;
        let cam = camera(s, &self.physics, 1.0, (0.0, 0.0));
        let settings = RenderSettings { true_color: false, ..self.settings };
        let vp = Viewport { x: 0, y: 0, width: saves::THUMB_W, height: saves::THUMB_H };
        self.renderer.render(&s.level.world, &cam, &Shading { rows: &self.shade_rows }, &settings, &mut thumb, vp);
        self.slots[n] = Some(saves::SaveSlot { label, snapshot, thumb: thumb.pixels });
        self.slots_dirty[n] = true;
        if let Some(s) = self.session_mut() {
            s.show_message(script::DEFAULT_MESSAGE_COLOR, format!("Saved to slot {}", n + 1));
        }
    }

    /// Load slot `n` (0-based).
    pub fn load_slot(&mut self, vfs: &Vfs, n: usize) {
        let Some(text) = self.slots.get(n).cloned().flatten().map(|s| s.snapshot) else { return };
        if self.restore(vfs, &text)
            && let Some(s) = self.session_mut() {
                s.show_message(script::DEFAULT_MESSAGE_COLOR, format!("Loaded slot {}", n + 1));
            }
    }

    /// Slots whose contents changed: (index, file text) for front ends to persist.
    pub fn take_slots(&mut self) -> Vec<(usize, String)> {
        (0..saves::SLOTS)
            .filter(|&i| std::mem::take(&mut self.slots_dirty[i]))
            .filter_map(|i| self.slots[i].as_ref().map(|s| (i, s.to_text())))
            .collect()
    }

    /// A stored slot (front ends call this at start-up).
    pub fn set_slot(&mut self, n: usize, text: &str) {
        if n < saves::SLOTS {
            self.slots[n] = saves::SaveSlot::parse(text);
        }
    }

    /// Quicksave text when it changed (front ends persist it), and restoring a stored one.
    pub fn take_quicksave(&mut self) -> Option<String> {
        std::mem::take(&mut self.quicksave_dirty).then(|| self.quicksave.clone()).flatten()
    }

    pub fn set_quicksave(&mut self, text: String) {
        self.quicksave = Some(text);
    }

    /// GAMEGOTO: start a level for the selected character, restoring a saved game.
    fn goto(&mut self, vfs: &Vfs, goto: session::Goto) {
        if let Err(e) = self.start_level(vfs, &goto.level, Entry::Fresh) {
            log::error!("cannot load {}: {e}", goto.level);
            return;
        }
        let mut setup = GotoSetup { slot: goto.slot, states: None, restore: None };
        if let Some(s) = self.session_mut() {
            s.menus.current_slot = goto.slot;
            if let Some(slot) = goto.slot.map(|i| s.save.slots[i].clone()) {
                if slot.is_active() {
                    s.script.states = slot.states();
                    setup.states = Some(s.script.states);
                }
                if goto.restore {
                    let (x, y, z) = slot.position();
                    let angle = slot.angle() as f32 / 2048.0 * std::f32::consts::TAU;
                    s.restore_position(x, y, z, angle);
                    s.restore_inventory(&slot.inventory());
                    setup.restore = Some((x, y, z, angle, slot.inventory()));
                }
            }
        }
        if let Some(rec) = &mut self.recording {
            rec.goto = Some(setup);
        }
    }

    /// Recordings of levels finished since the last call: (level, replay text).
    pub fn take_finished_replays(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.finished_replays)
    }

    /// A route guide from a recorded run (`take_finished_replays`, a quicksave): replay it and
    /// keep the switches, pickups, teleporters and trigger regions it used, with the way walked
    /// between them. None if the text holds no replay.
    pub fn route_from_replay(&mut self, vfs: &Vfs, text: &str) -> Option<route::Route> {
        let rec = Recording::parse(text)?;
        let mut recorder = RouteRecorder::default();
        self.replay_with(vfs, rec, Some(&mut recorder)).then_some(recorder.route)
    }

    /// Netplay changed the world from outside: the current level can no longer be rebuilt by
    /// replaying local input (quicksaves fall back to the plain snapshot).
    pub fn mark_unreplayable(&mut self) {
        self.recording = None;
    }

    /// Rebuild a level by replaying its recording: enter it as it was entered, then run every
    /// recorded tick through the simulation (the route guide follows along).
    fn replay(&mut self, vfs: &Vfs, rec: Recording) -> bool {
        self.replay_with(vfs, rec, None)
    }

    fn replay_with(&mut self, vfs: &Vfs, rec: Recording, mut recorder: Option<&mut RouteRecorder>) -> bool {
        if self.start_level_with(vfs, &rec.level, rec.entry, rec.carried.clone()).is_err() {
            return false;
        }
        if let Some(s) = self.session_mut() {
            if let Some(name) = &rec.place {
                s.place_player_at(name);
            }
            if let Some(g) = &rec.goto {
                s.menus.current_slot = g.slot;
                if let Some(states) = g.states {
                    s.script.states = states;
                }
                if let Some((x, y, z, angle, inventory)) = &g.restore {
                    s.restore_position(*x, *y, *z, *angle);
                    s.restore_inventory(inventory);
                }
            }
        }
        let physics = self.physics;
        let (message_scale, fixes) = (if self.options.long_messages { 2.0 } else { 1.0 }, self.options.fixes);
        let route = self.routes.get(&rec.level.to_ascii_uppercase()).cloned();
        self.route_progress = (rec.level.clone(), route::RouteProgress::default());
        for (n, input) in rec.inputs.iter().enumerate() {
            let State::Playing(s) = &mut self.state else { return false };
            s.message_scale = message_scale;
            s.fixes = fixes;
            s.tick(input, &physics);
            let walls = std::mem::take(&mut s.effects.touch_log);
            let entered = std::mem::take(&mut s.effects.enter_log);
            let bumped = std::mem::take(&mut s.objects.bump_log);
            if let Some(route) = &route {
                self.route_progress.1.advance(route, &walls, &bumped, &entered);
            }
            if let Some(r) = recorder.as_deref_mut() {
                r.note(s, &walls, &bumped, &entered);
            }
            if n % 8 == 0 {
                let cam = camera(s, &physics, 1.0, (0.0, 0.0));
                self.automap.reveal(&s.level.world, &cam);
            }
            // (A level change ends a recording, so none happens inside one.)
            s.pending_goto = None;
            s.pending_level = None;
        }
        if let Some(s) = self.session_mut() {
            s.wants_options = false;
            s.messages.clear();
        }
        self.recording = Some(rec);
        true
    }

    /// The save file's bytes when it changed since the last call (front ends persist it).
    pub fn take_save(&mut self) -> Option<Vec<u8>> {
        let dirty = match &mut self.state {
            State::Playing(s) => std::mem::take(&mut s.save_dirty) | std::mem::take(&mut self.save_dirty),
            State::Title => std::mem::take(&mut self.save_dirty),
        };
        if !dirty {
            return None;
        }
        Some(match &self.state {
            State::Playing(s) => s.save.to_bytes(),
            State::Title => self.save.to_bytes(),
        })
    }

    /// True when the game asked to quit (menu Quit / QUIT command).
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Current 256-colour palette for presenting frames.
    pub fn palette(&self) -> &[[u8; 3]; 256] {
        match self.state {
            State::Title => &self.title.palette,
            State::Playing(_) => &self.game_palette,
        }
    }

    /// Rumble requests since the last call.
    pub fn drain_haptics(&mut self) -> Vec<engine_core::HapticEvent> {
        std::mem::take(&mut self.haptics)
    }

    /// Audio requests produced since the last call.
    pub fn drain_audio(&mut self) -> Vec<AudioEvent> {
        if let State::Playing(session) = &mut self.state {
            self.audio.extend(session.drain_audio());
            let p = session.player;
            if self.reverb_cache.0 != (self.epoch, p.region) {
                self.reverb_cache = ((self.epoch, p.region), session.reverb_at(p.region));
            }
            let reverb = if self.cvars.get_bool("snd_reverb") { self.reverb_cache.1 } else { 0.0 };
            // Occlusion: open portals between the listener and each positional sound.
            if self.hops_cache.0 != (self.epoch, p.region) {
                self.hops_cache = ((self.epoch, p.region), session.portal_hops(p.region));
            }
            let world = &session.level.world;
            for e in &mut self.audio {
                if let AudioEvent::Sound { pos: Some(at), muffle, .. } = e
                    && let Some(r) = world.region_at_z(at[0], at[1], Some(at[2]))
                {
                    let hops = self.hops_cache.1.get(r).copied().unwrap_or(u8::MAX);
                    *muffle = ((hops.saturating_sub(1)) as f32 / 3.0).min(1.0);
                }
            }
            self.audio.push(AudioEvent::Listener { pos: [p.x, p.y, p.eye_z(&self.physics)], angle: p.angle, reverb });
        }
        std::mem::take(&mut self.audio)
    }

    /// Turn the player by `turn` radians and move `forward` units (with collision). For tools
    /// and tests that need a specific viewpoint.
    pub fn debug_move_player(&mut self, turn: f32, forward: f32) {
        if let State::Playing(session) = &mut self.state {
            let player = &mut session.player;
            player.angle = (player.angle + turn).rem_euclid(std::f32::consts::TAU);
            let mut input = Input::default();
            input.set(Action::Forward, forward != 0.0);
            let per_tick = self.physics.walk_speed / 1.2 / TICK_RATE as f32;
            for _ in 0..(forward / per_tick).round() as usize {
                session.player.tick(&session.level.world, &input, &self.physics);
            }
            session.previous = session.player;
        }
    }

    /// The running session, if a level is loaded.
    pub fn session(&self) -> Option<&Session> {
        match &self.state {
            State::Playing(s) => Some(s),
            State::Title => None,
        }
    }

    pub fn session_mut(&mut self) -> Option<&mut Session> {
        match &mut self.state {
            State::Playing(s) => Some(s),
            State::Title => None,
        }
    }

    /// Advance one simulation tick. `vfs` is needed when the game loads a level.
    pub fn tick(&mut self, vfs: &Vfs, input: &Input) {
        self.uptime_ticks += 1;
        self.fps.ticks += 1;
        if self.fps.ticks >= TICK_RATE as u32 {
            self.fps.shown = self.fps.frames;
            self.fps.frames = 0;
            self.fps.ticks = 0;
        }
        self.options_menu.slot_labels = self.slots.iter().map(|s| s.as_ref().map_or_else(String::new, |s| s.label.clone())).collect();
        self.options_menu.can_save = self.session().is_some();
        if let Some(s) = self.session_mut()
            && std::mem::take(&mut s.wants_options)
        {
            self.options_menu.open_main();
        }
        let paused = self.options_menu.update(input, &mut self.cvars, &mut self.binds);
        self.options = options::GameOptions::from_cvars(&self.cvars);
        if let Some(req) = self.options_menu.request.take() {
            if req.load {
                self.load_slot(vfs, req.slot);
            } else {
                self.save_slot(req.slot);
            }
        }
        if paused {
            return; // the overlay pauses the game
        }
        let effective = self.effective_input(input);
        let input = &effective;
        if input.pressed(Action::Console) {
            self.console.open = !self.console.open;
            self.console.line.clear();
            return;
        }
        if self.console.open {
            self.console_input(vfs, input);
            return; // the console pauses the game
        }
        if input.pressed(Action::Map) {
            self.show_map = !self.show_map;
        }
        let input = &{
            let mut i = input.clone();
            if self.show_map {
                // The wheel and +/- zoom the map instead of switching tools.
                let zoom = self.cvars.get_float("am_zoom");
                let step = |z: f64, up: bool| if up { (z * 1.25).min(8.0) } else { (z / 1.25).max(0.25) };
                if i.pressed(Action::MapZoomIn) || i.pressed(Action::ToolNext) {
                    self.cvars.set_float("am_zoom", step(zoom, true));
                }
                if i.pressed(Action::MapZoomOut) || i.pressed(Action::ToolPrev) {
                    self.cvars.set_float("am_zoom", step(zoom, false));
                }
                i.clear(Action::ToolNext);
                i.clear(Action::ToolPrev);
            }
            if i.pressed(Action::MapMark)
                && let Some(s) = self.session() {
                    self.automap.marks.push((s.player.x, s.player.y));
                }
            i
        };
        if self.uptime_ticks.is_multiple_of(4)
            && let State::Playing(s) = &self.state {
                let cam = camera(s, &self.physics, 1.0, (0.0, 0.0));
                self.automap.reveal(&s.level.world, &cam);
            }
        // (Recorded before a quicksave: the tick F6 is pressed on still runs, and is in the save.)
        if let (State::Playing(_), Some(rec)) = (&self.state, &mut self.recording) {
            rec.inputs.push(input.clone());
        }
        if input.pressed(Action::QuickSave) {
            self.quick_save();
        }
        if input.pressed(Action::QuickLoad) {
            self.quick_load(vfs);
            return;
        }
        if let State::Playing(s) = &mut self.state {
            s.message_scale = if self.options.long_messages { 2.0 } else { 1.0 };
            s.fixes = self.options.fixes;
        }
        self.settings.vfov_degrees = if self.options.classic { 48.9 } else { self.options.fov };
        self.settings.true_color = self.options.true_color;
        match &mut self.state {
            State::Title => {
                if (input.pressed(Action::MenuSelect) || input.pressed(Action::Jump) || input.pressed(Action::Use))
                    && let Err(e) = self.start_level(vfs, "LIPSTART", Entry::Fresh) {
                        log::error!("cannot load LIPSTART: {e}");
                    }
            }
            State::Playing(session) => {
                session.tick(input, &self.physics);
                // The route guide follows what was used this tick (the logs are drained either way).
                let walls = std::mem::take(&mut session.effects.touch_log);
                let entered = std::mem::take(&mut session.effects.enter_log);
                let bumped = std::mem::take(&mut session.objects.bump_log);
                if self.route_progress.0 != session.name {
                    self.route_progress = (session.name.clone(), route::RouteProgress::default());
                }
                if let Some(route) = self.routes.get(&session.name.to_ascii_uppercase()) {
                    let progress = &mut self.route_progress.1;
                    progress.advance(route, &walls, &bumped, &entered);
                    // (Picked up already, as after loading a save.)
                    progress.skip_done(route, |o| session.objects.inventory.contains(&o));
                }
                // Rumble: a light kick per shot, a heavy one when hurt, a thud for nearby blasts.
                let (life, armor) = (session.script.states[script::LIFE], session.script.states[script::ARMOR]);
                let flash = session.objects.muzzle_flash;
                let (pl, pa, pf) = self.last_vitals;
                if self.cvars.get_bool("joy_rumble") {
                    use engine_core::HapticEvent;
                    if flash > pf {
                        self.haptics.push(HapticEvent { low: 0.2, high: 0.6, seconds: 0.08 });
                    }
                    let hurt = (pl - life).max(0) + (pa - armor).max(0);
                    if hurt > 0 && pl > 0 {
                        self.haptics.push(HapticEvent { low: (hurt as f32 / 40.0).clamp(0.3, 1.0), high: 0.3, seconds: 0.25 });
                    }
                    let p = session.player;
                    // Objects are appended, so explosions past last tick's list length are new.
                    let near_blast = session.objects.list.iter().skip(self.last_objects).any(|o| {
                        matches!(o.kind, objects::Kind::Explosion { .. }) && !o.destroyed && (o.x - p.x).hypot(o.y - p.y) < 320.0
                    });
                    if near_blast {
                        self.haptics.push(HapticEvent { low: 0.7, high: 0.2, seconds: 0.2 });
                    }
                }
                self.last_vitals = (life, armor, flash);
                self.last_objects = session.objects.list.len();
                if session.quit_requested {
                    self.quit = true;
                }
                if let Some(goto) = session.pending_goto.take() {
                    if let Some(rec) = self.recording.take() {
                        self.finished_replays.push((rec.level.clone(), rec.to_text()));
                    }
                    self.goto(vfs, goto);
                    return;
                }
                if let Some((level, start)) = session.pending_level.take() {
                    if let Some(rec) = self.recording.take() {
                        self.finished_replays.push((rec.level.clone(), rec.to_text()));
                    }
                    match self.start_level(vfs, &level, Entry::Continue) {
                        Ok(()) => {
                            if let (Some(name), Some(s)) = (start, self.session_mut()) {
                                s.place_player_at(&name);
                                if let Some(rec) = &mut self.recording {
                                    rec.place = Some(name);
                                }
                            }
                        }
                        Err(e) => log::error!("cannot load {level}: {e}"),
                    }
                    return;
                }
                // Numpad * / change the "fade factor" (visibility) by 16 within 32..1024.
                if input.pressed(Action::Brighten) {
                    self.settings.visibility = (self.settings.visibility + 16.0).min(1024.0);
                }
                if input.pressed(Action::Darken) {
                    self.settings.visibility = (self.settings.visibility - 16.0).max(32.0);
                }
            }
        }
    }

    /// The route guide's next step for the running level (None: no route, or done).
    pub fn route_next(&self) -> Option<usize> {
        let s = self.session()?;
        let route = self.routes.get(&s.name.to_ascii_uppercase())?;
        (self.route_progress.0 == s.name && self.route_progress.1.next < route.steps.len()).then_some(self.route_progress.1.next)
    }

    /// Render into `frame`. `alpha` (0..1) interpolates between the last two ticks.
    pub fn render(&mut self, frame: &mut Frame, alpha: f32) {
        self.render_view(frame, alpha);
        self.render_ui(frame);
    }

    /// The world: title picture or 3D view with the first-person weapon and overlays. A GPU
    /// renderer replaces this part and keeps [`Game::render_ui`] on top.
    pub fn render_view(&mut self, frame: &mut Frame, alpha: f32) {
        self.fps.frames += 1;
        frame.palette = *self.palette();
        frame.clear(0);
        if let State::Playing(session) = &mut self.state {
            self.settings.fullbright = session.flash;
            render_tvs(&mut self.renderer, session, &self.shade_rows, &self.settings, &frame.palette);
        }
        match &self.state {
            State::Title => blit_fit(frame, self.title.width, self.title.height, &self.title.pixels),
            State::Playing(session) if session.in_bsp => {
                // The BSP world is drawn by the GPU renderer only.
                let s = (frame.height / CLASSIC_HEIGHT).max(1);
                let note = "This 3D area needs the GPU renderer";
                let x = (frame.width.saturating_sub(text::text_width(note, s))) / 2;
                text::draw_text(frame, x as i32, (frame.height / 2) as i32, s, self.ui_colors.3, note);
                draw_weapon(frame, session, &self.shade_rows);
            }
            State::Playing(session) => {
                let cam = camera(session, &self.physics, alpha, self.pending_look);
                let shading = Shading { rows: &self.shade_rows };
                let vp = Viewport { x: 0, y: 0, width: frame.width, height: frame.height };
                self.renderer.render(&session.level.world, &cam, &shading, &self.settings, frame, vp);
                draw_weapon(frame, session, &self.shade_rows);
                draw_overlays(frame, session);
            }
        }
    }

    /// The GPU path: returns the 3D scene for this frame and draws everything else (weapon,
    /// overlays, HUD, menus, console) into `ui`, with coverage, to be composited on top.
    /// `None` on the title screen and in classic 320x200 mode (render with [`Game::render`]).
    pub fn gpu_frame(&mut self, ui: &mut Frame, alpha: f32) -> Option<GpuView> {
        let in_bsp = matches!(&self.state, State::Playing(s) if s.in_bsp);
        // The BSP world has no software view: it always goes through the GPU.
        if (self.options.classic && !in_bsp) || !matches!(self.state, State::Playing(_)) {
            return None;
        }
        if in_bsp != self.bsp_shown {
            self.bsp_shown = in_bsp;
            self.epoch += 1; // other images: re-upload
        }
        self.fps.frames += 1;
        ui.palette = *self.palette();
        if ui.coverage.is_none() {
            ui.track_coverage(true);
        }
        ui.record_ui(self.cvars.get_bool("hud_hd"));
        ui.clear(0);
        let epoch = self.epoch;
        let State::Playing(session) = &mut self.state else { return None };
        self.settings.fullbright = session.flash;
        let modern = modern_settings(&self.cvars, self.light_fit);
        if in_bsp {
            let aspect = self.view_aspect.get().unwrap_or(ui.width as f32 / ui.height.max(1) as f32);
            let guides_on = self.cvars.get_bool("hud_guides") && modern.is_some_and(|m| m.dynamic_lights);
            let t = self.uptime_ticks as f32 / TICK_RATE as f32;
            let world = session.bsp.as_mut()?;
            world.images(&ui.palette, &session.level.world.images);
            let (god_pal, god_rows) = (world.palette, world.colormap.clone());
            let map = bsp_world::relight_levels(&god_pal, &god_rows, &ui.palette, &self.shade_rows);
            world.relight(map);
            let offset = world.texture_count() as u32;
            let mut frame = world.scene.clone();
            let p = world.player;
            if guides_on {
                // The way to the exit: green lights along the planned route and at the exit.
                let pulse = 1.0 + 0.35 * (t * std::f32::consts::TAU * 0.8).sin();
                let green = guides::GuideKind::Exit.light();
                for spot in world.route_ahead(6) {
                    frame.lights.push(engine_scene::scene::SceneLight { pos: spot, radius: 64.0, color: green.map(|v| v * 0.8 * pulse) });
                }
                for c in world.exits() {
                    frame.lights.push(engine_scene::scene::SceneLight { pos: c.centre, radius: 192.0, color: green.map(|v| v * pulse) });
                }
            }
            let first_sprite = frame.vertices.len();
            // The world is one unclipped batch (once any batch exists only batches are drawn).
            frame.batch(0, None);
            weapon_quad(&mut frame, session, aspect);
            for v in &mut frame.vertices[first_sprite..] {
                v.image += offset; // level images follow the BSP textures
            }
            let s = &self.settings;
            let view = GpuView {
                camera: engine_scene::scene::SceneCamera { pos: p.pos, angle: p.orb_angle(), pitch: p.pitch.tan(), vfov_degrees: s.vfov_degrees, freelook: true },
                shading: engine_scene::scene::Shading {
                    visibility: 1.0e9,
                    fade_bias: 0.0,
                    true_color: true,
                    fullbright: false,
                    max_object_shade: 14.0,
                    zero_substitute: 8,
                    modern,
                },
                scene: frame,
                epoch,
            };
            self.render_ui(ui);
            return Some(view);
        }
        render_tvs(&mut self.renderer, session, &self.shade_rows, &self.settings, &ui.palette);
        if self.scene_cache.as_ref().is_none_or(|(e, _)| *e != epoch) {
            self.scene_cache = Some((epoch, scene::build(&session.level.world)));
        }
        let cam = camera(session, &self.physics, alpha, self.pending_look);
        let mut frame = engine_scene::scene::SceneFrame::default();
        let tan_half_v = (self.settings.vfov_degrees.to_radians() / 2.0).tan();
        let tan_half_h = tan_half_v * self.view_aspect.get().unwrap_or(ui.width as f32 / ui.height.max(1) as f32);
        let freelook = self.cvars.get_bool("r_freelook");
        scene::frame(&session.level.world, &self.scene_cache.as_ref().expect("scene cache").1, &cam, (tan_half_h, tan_half_v), freelook, &mut frame);
        frame.dirty_images = session.tvs.iter().map(|t| t.image as u32).collect();
        weapon_quad(&mut frame, session, self.view_aspect.get().unwrap_or(ui.width as f32 / ui.height.max(1) as f32));
        draw_overlays(ui, session);
        let c = &self.cvars;
        if modern.is_some_and(|m| m.dynamic_lights) {
            frame.lights = scene_lights(session, &self.physics);
            let route = self.routes.get(&session.name.to_ascii_uppercase()).filter(|_| c.get_bool("hud_route") && self.route_progress.0 == session.name);
            if let Some(step) = route.and_then(|r| self.route_progress.1.current(r)) {
                // The route guide: a beacon at the next thing to do and a trail of lights to it.
                let p = &session.player;
                let t = self.uptime_ticks as f32 / TICK_RATE as f32;
                let pulse = 1.0 + 0.35 * (t * std::f32::consts::TAU * 0.8).sin();
                let green = guides::GuideKind::Exit.light();
                frame.lights.push(engine_scene::scene::SceneLight { pos: [step.pos[0], step.pos[1], step.pos[2] + 24.0], radius: 160.0, color: green.map(|v| v * 1.3 * pulse) });
                let room = 64usize.saturating_sub(frame.lights.len()).min(12);
                let route = route.expect("route");
                for (k, spot) in self.route_progress.1.trail_ahead(route, [p.x, p.y, p.z], 96.0, room).into_iter().enumerate() {
                    // (A wave running along the trail towards the beacon.)
                    let wave = 0.6 + 0.4 * ((t * 3.0 - k as f32 * 0.6).sin() * 0.5 + 0.5);
                    frame.lights.push(engine_scene::scene::SceneLight { pos: [spot[0], spot[1], spot[2] + 16.0], radius: 56.0, color: green.map(|v| v * 0.55 * wave) });
                }
            } else if c.get_bool("hud_guides") {
                // Pulsing markers on the way forward, nearest first, after the combat lights.
                let p = &session.player;
                let t = self.uptime_ticks as f32 / TICK_RATE as f32;
                let pulse = 1.0 + 0.35 * (t * std::f32::consts::TAU * 0.8).sin();
                let room = 64usize.saturating_sub(frame.lights.len()).min(12);
                for g in guides::nearest(&guides::find(session), [p.x, p.y, p.z], 2048.0, room) {
                    let radius = if g.kind == guides::GuideKind::Switch { 96.0 } else { 128.0 };
                    frame.lights.push(engine_scene::scene::SceneLight { pos: g.pos, radius, color: g.kind.light().map(|v| v * pulse) });
                }
            }
        }
        // The route guide's beam over the next thing to do and dots along the way: soft glows
        // that show on any surface (its lights alone vanish on dark floors), in either lighting.
        let route = self.routes.get(&session.name.to_ascii_uppercase()).filter(|_| c.get_bool("hud_route") && self.route_progress.0 == session.name);
        if let Some(route) = route
            && let Some(step) = self.route_progress.1.current(route)
        {
            use engine_scene::scene::{GLOW, GLOW_DOT, NO_DISTANCE, SceneVertex};
            const GREEN: [f32; 3] = [0.35, 1.0, 0.45];
            let t = self.uptime_ticks as f32 / TICK_RATE as f32;
            let pulse = 0.85 + 0.15 * (t * std::f32::consts::TAU * 0.8).sin();
            let (sin, cos) = cam.angle.sin_cos();
            let w = &session.level.world;
            let [x, y, z] = step.pos;
            let room = w.region_at_z(x, y, Some(z + 1.0)).map_or(320.0, |r| w.regions[r].ceiling - z);
            let first = frame.vertices.len();
            let corner = |cx: f32, cy: f32, cz: f32, uv: [f32; 2], strength: f32, flags: u32| SceneVertex { pos: [cx, cy, cz], uv, image: u32::MAX, light: strength, flags: GLOW | NO_DISTANCE | flags, normal: GREEN };
            // (Facing the camera, turning about the upright only.)
            let (half, height, strength) = (9.0, room.clamp(48.0, 320.0), 0.55 * pulse);
            let (rx, ry) = (sin * half, -cos * half);
            frame.quad([
                corner(x - rx, y - ry, z, [-1.0, 0.0], strength, 0),
                corner(x + rx, y + ry, z, [1.0, 0.0], strength, 0),
                corner(x + rx, y + ry, z + height, [1.0, 1.0], strength, 0),
                corner(x - rx, y - ry, z + height, [-1.0, 1.0], strength, 0),
            ]);
            let p = &session.player;
            for (k, spot) in self.route_progress.1.trail_ahead(route, [p.x, p.y, p.z], 96.0, 16).into_iter().enumerate() {
                // (Brightening in a wave that runs on towards the beam.)
                let wave = 0.7 + 0.3 * ((t * 3.0 - k as f32 * 0.6).sin() * 0.5 + 0.5);
                let (r, [sx, sy, sz]) = (8.0, spot);
                let (dx, dy, cz) = (sin * r, -cos * r, sz + 10.0);
                frame.quad([
                    corner(sx - dx, sy - dy, cz - r, [-1.0, -1.0], wave, GLOW_DOT),
                    corner(sx + dx, sy + dy, cz - r, [1.0, -1.0], wave, GLOW_DOT),
                    corner(sx + dx, sy + dy, cz + r, [1.0, 1.0], wave, GLOW_DOT),
                    corner(sx - dx, sy - dy, cz + r, [-1.0, 1.0], wave, GLOW_DOT),
                ]);
            }
            frame.batch(first, None);
        }
        let s = &self.settings;
        let view = GpuView {
            camera: engine_scene::scene::SceneCamera { pos: [cam.x, cam.y, cam.z], angle: cam.angle, pitch: cam.pitch, vfov_degrees: s.vfov_degrees, freelook },
            shading: engine_scene::scene::Shading {
                visibility: s.visibility,
                fade_bias: s.fade_bias as f32,
                true_color: s.true_color,
                fullbright: s.fullbright,
                max_object_shade: 14.0,
                zero_substitute: 8,
                modern,
            },
            scene: frame,
            epoch,
        };
        self.render_ui(ui);
        Some(view)
    }

    /// Size of the UI layer for a `width`x`height` GPU view: the UI is pixel art drawn at a whole
    /// scale (HUD scale option, or the largest that fits), so the layer is drawn at 1/scale and
    /// the GPU enlarges it.
    /// It also records the view's aspect for the next [`Game::gpu_frame`].
    pub fn ui_layer_size(&self, width: usize, height: usize) -> (usize, usize) {
        self.view_aspect.set(Some(width as f32 / height.max(1) as f32));
        let fit = (height / CLASSIC_HEIGHT).max(1);
        let s = if self.options.hud_scale > 0 { self.options.hud_scale.min(fit) } else { fit };
        (width.div_ceil(s), height.div_ceil(s))
    }

    /// The current level's images (for GPU upload when [`GpuView::epoch`] changes).
    pub fn world_images(&self) -> Vec<engine_scene::scene::IndexedImage<'_>> {
        self.session().map_or_else(Vec::new, |s| {
            Self::shown_images(s).iter().map(|i| engine_scene::scene::IndexedImage { width: i.width, height: i.height, pixels: &i.pixels }).collect()
        })
    }

    /// The images of the world being shown: the BSP's textures while in it, else the level's.
    fn shown_images(s: &Session) -> &[world::Image] {
        match &s.bsp {
            Some(b) if s.in_bsp => b.cached_images(),
            _ => &s.level.world.images,
        }
    }

    /// Pack keys (name, content hash) of the current level's images, by image index.
    pub fn world_image_keys(&self) -> Vec<(String, u64)> {
        let Some(s) = self.session() else { return Vec::new() };
        // In the BSP world: its textures first, then the level's images (as `world_images`).
        let bsp: Vec<(String, u64)> = match s.bsp.as_ref().filter(|_| s.in_bsp) {
            Some(b) => b.bsp.textures.iter().zip(b.cached_images()).map(|(t, i)| (t.name.clone(), engine_core::pack::image_hash(i.width, i.height, &i.pixels))).collect(),
            None => Vec::new(),
        };
        bsp.into_iter().chain(s.level
            .world
            .images
            .iter()
            .enumerate()
            .map(|(i, img)| {
                let name = s.level.orb.pics.get(i).map_or_else(|| format!("PIC{i}"), |p| p.name.trim().to_string());
                (name, engine_core::pack::image_hash(img.width, img.height, &img.pixels))
            })
            )
            .collect()
    }

    /// HD textures for the current level's images from a texture pack (by name and content).
    pub fn hd_textures(&self, pack: &engine_core::pack::LoadedPack) -> Vec<Option<engine_scene::scene::HdTexture>> {
        use engine_scene::scene::{HdTexture, RgbaImage};
        let conv = |r: engine_core::pack::Rgba| RgbaImage { width: r.width, height: r.height, pixels: r.pixels };
        pack.textures(&self.world_image_keys())
            .into_iter()
            .map(|t| t.map(|(a, n)| HdTexture { albedo: conv(a), normal: n.map(conv) }))
            .collect()
    }

    /// One world image (live camera views change every frame).
    pub fn world_image(&self, index: u32) -> Option<engine_scene::scene::IndexedImage<'_>> {
        let i = Self::shown_images(self.session()?).get(index as usize)?;
        Some(engine_scene::scene::IndexedImage { width: i.width, height: i.height, pixels: &i.pixels })
    }

    /// Shade tables (`rows[level][index]`).
    pub fn shade_rows(&self) -> &[[u8; 256]] {
        &self.shade_rows
    }

    /// Everything drawn over the view: automap, HUD, menus, options, console and FPS counter.
    pub fn render_ui(&mut self, frame: &mut Frame) {
        if let State::Playing(session) = &self.state {
            if self.show_map {
                // The backdrop and floors are pixels; walls, markers and arrows are recorded.
                if self.scene_cache.as_ref().is_none_or(|(e, _)| *e != self.epoch) {
                    self.scene_cache = Some((self.epoch, scene::build(&session.level.world)));
                }
                let style = automap::Style {
                    textured: self.cvars.get_bool("am_textured"),
                    rotate: self.cvars.get_bool("am_rotate"),
                    zoom: self.cvars.get_float("am_zoom") as f32,
                    floor_row: 7,
                    colors: self.ui_colors,
                    guides: if let Some(route) = self.routes.get(&session.name.to_ascii_uppercase()).filter(|_| self.cvars.get_bool("hud_route") && self.route_progress.0 == session.name)
                        && let Some(step) = self.route_progress.1.current(route)
                    {
                        // The route guide: the trail to the next step and the step itself.
                        let pal = self.palette();
                        let green = text::nearest_color(pal, guides::GuideKind::Exit.rgb());
                        let world = &session.level.world;
                        let mut marks: Vec<(f32, f32, Option<usize>, u8)> = step.trail.iter().step_by(2).map(|q| (q[0], q[1], world.region_at(q[0], q[1]), green)).collect();
                        marks.push((step.pos[0], step.pos[1], world.region_at(step.pos[0], step.pos[1]), green));
                        marks
                    } else if self.cvars.get_bool("hud_guides") {
                        let pal = self.palette();
                        guides::find(session).iter().map(|g| (g.pos[0], g.pos[1], g.region, text::nearest_color(pal, g.kind.rgb()))).collect()
                    } else {
                        Vec::new()
                    },
                };
                let others: Vec<(f32, f32, f32)> = session.remote_positions();
                // (In the level's BSP part the player is there, not in the orb: its own map.)
                match session.bsp.as_ref().filter(|_| session.in_bsp) {
                    Some(world) => self.automap.draw_bsp(frame, world, &style),
                    None => self.automap.draw(frame, session, &self.scene_cache.as_ref().expect("scene cache").1, &self.shade_rows, &style, &others),
                }
            }
            let uptime = self.uptime_ticks / TICK_RATE as u64;
            if self.cvars.get_str("hud_style") == "modern" && !self.options.classic {
                self.hud.draw_modern(frame, session, self.options.high_contrast, self.options.hud_scale);
                if self.show_map {
                    self.hud.draw_stats(frame, session, session.tool, uptime, self.options.hud_scale);
                }
            } else {
                self.hud.draw(frame, session, session.tool, uptime, self.options.high_contrast, self.options.hud_scale);
            }
            draw_crosshair(frame, self.options.crosshair, self.ui_colors.3);
            session.menus.draw(frame, &session.save);
        }
        self.options_menu.draw(frame, &self.cvars, &self.binds, self.ui_colors);
        if let Some(thumb) = self.options_menu.selected_slot().and_then(|i| self.slots[i].as_ref()) {
            // Thumbnail of the slot under the cursor, framed, at the right of the screen.
            let s = (frame.height / CLASSIC_HEIGHT).max(1);
            let (w, h) = (saves::THUMB_W * s, saves::THUMB_H * s);
            let (x0, y0) = (frame.width.saturating_sub(w + 8 * s), (frame.height.saturating_sub(h)) / 2);
            text::fill_rect(frame, x0 as i32 - s as i32, y0 as i32 - s as i32, w + 2 * s, h + 2 * s, self.ui_colors.1);
            // Index 0 is black in the thumbnail, not transparent: draw it on a black box.
            text::fill_rect(frame, x0 as i32, y0 as i32, w, h, 0);
            text::blit(frame, saves::THUMB_W, saves::THUMB_H, &thumb.thumb, x0 as i32, y0 as i32, s);
        }
        if self.console.open {
            // Top half of the screen: scrollback, then the prompt.
            let s = (frame.height / CLASSIC_HEIGHT).max(1);
            let line_h = (text::GLYPH + 2) * s;
            let rows = (frame.height / 2 / line_h).saturating_sub(1).max(1);
            let lines = self.console.visible(rows);
            text::fill_rect(frame, 0, 0, frame.width, line_h * (rows + 1) + 2 * s, self.ui_colors.0);
            let top = rows - lines.len();
            for (i, l) in lines.iter().enumerate() {
                text::draw_text(frame, 2 * s as i32, ((top + i) * line_h + s) as i32, s, self.ui_colors.2, l);
            }
            let marker = if self.console.scroll > 0 { "^ " } else { "" };
            let prompt = format!("{marker}] {}_", self.console.line);
            text::draw_text(frame, 2 * s as i32, (rows * line_h + s) as i32, s, self.ui_colors.3, &prompt);
        }
        let s = (frame.height / CLASSIC_HEIGHT).max(1);
        let mut line = 0;
        if self.options.show_fps {
            text::draw_text(frame, 2 * s as i32, 2 * s as i32, s, self.ui_colors.3, &format!("{} fps", self.fps.shown));
            line += 1;
        }
        if self.cvars.get_bool("r_stats") {
            let c = &self.cvars;
            let frame_ms = if self.fps.shown > 0 { format!("{:.2} ms", 1000.0 / self.fps.shown as f32) } else { "-".into() };
            let gpu = self.gpu_ms.map_or_else(|| "n/a".into(), |g| format!("{g:.2} ms"));
            let msaa = 1 << c.get_int("r_msaa").clamp(0, 3);
            let lines = [format!("frame {frame_ms}  gpu {gpu}"), format!("scale {:.2}  msaa {msaa}x{}", c.get_float("r_scale"), if c.get_bool("r_fxaa") { "  fxaa" } else { "" })];
            for (k, text) in lines.iter().enumerate() {
                text::draw_text(frame, 2 * s as i32, ((2 + (line + k) * 10) * s) as i32, s, self.ui_colors.3, text);
            }
        }
    }
}

/// Interpolated view between the last two ticks. Mouse/stick look (`direct_*`) is not
/// interpolated: the last tick's part is shown in full and `pending` (motion the next tick will
/// consume) is added, so the view follows the mouse 1:1 at any frame rate.
fn camera(session: &Session, physics: &PhysicsParams, alpha: f32, pending: (f32, f32)) -> Camera {
    let (p, q) = (&session.previous, &session.player);
    let lerp = |a: f32, b: f32| a + (b - a) * alpha;
    let mut angle_delta = (q.angle - q.direct_turn - p.angle).rem_euclid(std::f32::consts::TAU);
    if angle_delta > std::f32::consts::PI {
        angle_delta -= std::f32::consts::TAU;
    }
    let look = (lerp(p.look, q.look - q.direct_look) + q.direct_look + pending.1).clamp(-physics.look_limit, physics.look_limit);
    let (x, y) = (lerp(p.x, q.x), lerp(p.y, q.y));
    // The region the drawn position is in: between two ticks across a portal it is still the
    // one before (seen from the tick's region, behind the portal it came through, the view
    // lost and regained geometry every time a line was crossed while moving). Not across a
    // teleport.
    let w = &session.level.world;
    let region = if p.region != q.region && (q.x - p.x).hypot(q.y - p.y) < 64.0 && !w.point_in_region(x, y, q.region) && w.point_in_region(x, y, p.region) {
        p.region
    } else {
        q.region
    };
    Camera {
        x,
        y,
        z: lerp(p.eye_z(physics), q.eye_z(physics)),
        angle: (p.angle + angle_delta * alpha + q.direct_turn + pending.0).rem_euclid(std::f32::consts::TAU),
        pitch: PhysicsParams::look_to_pitch(look),
        region,
    }
}

/// Modern-lighting settings from the cvars (`None`: original shade tables).
fn modern_settings(c: &engine_core::CvarRegistry, light_fit: ([f32; 16], [u32; 8])) -> Option<engine_scene::scene::Modern> {
    (c.get_str("r_lighting") == "modern").then(|| engine_scene::scene::Modern {
        brightness: light_fit.0,
        emissive: light_fit.1,
        dynamic_lights: c.get_bool("r_dynlights"),
        bloom: c.get_bool("r_bloom"),
        tonemap: c.get_int("r_tonemap") as u32,
        exposure: c.get_float("r_exposure") as f32,
        msaa: 1 << c.get_int("r_msaa").clamp(0, 3),
        render_scale: c.get_float("r_scale") as f32,
        fxaa: c.get_bool("r_fxaa"),
        shadows: c.get_bool("r_shadows"),
    })
}

/// Dynamic lights for the modern renderer: flying projectiles, explosions (fading out) and the
/// player's muzzle flash.
fn scene_lights(session: &Session, physics: &PhysicsParams) -> Vec<engine_scene::scene::SceneLight> {
    use engine_scene::scene::SceneLight;
    use objects::Kind;
    let mut lights = Vec::new();
    for o in session.objects.list.iter().filter(|o| !o.destroyed && o.region.is_some()) {
        let mid = [o.x, o.y, o.z + o.height / 2.0];
        match o.kind {
            Kind::Projectile { flying: true, .. } => lights.push(SceneLight { pos: mid, radius: 160.0, color: [1.6, 1.1, 0.6] }),
            Kind::Explosion { life, .. } => {
                let k = (life / 0.25).clamp(0.35, 1.0);
                lights.push(SceneLight { pos: mid, radius: 320.0, color: [4.0 * k, 2.2 * k, 0.8 * k] });
            }
            _ => {}
        }
    }
    let flash = session.objects.muzzle_flash;
    if flash > 0.0 {
        let p = &session.player;
        let k = flash / 0.07;
        let (sin, cos) = p.angle.sin_cos();
        lights.push(SceneLight { pos: [p.x + cos * 24.0, p.y + sin * 24.0, p.eye_z(physics) - 8.0], radius: 300.0, color: [3.0 * k, 2.4 * k, 1.4 * k] });
    }
    lights.truncate(64);
    lights
}

/// Crosshair styles: 1 dot, 2 cross, 3 circle.
fn draw_crosshair(frame: &mut Frame, style: u8, color: u8) {
    if style == 0 {
        return;
    }
    let s = (frame.height / CLASSIC_HEIGHT).max(1) as i32;
    let (cx, cy) = (frame.width as i32 / 2, frame.height as i32 / 2);
    let dots: Vec<(i32, i32)> = match style {
        1 => vec![(0, 0)],
        2 => [-3, -2, 2, 3].iter().flat_map(|&d| [(d, 0), (0, d)]).collect(),
        _ => (0..16).map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            ((a.cos() * 3.0).round() as i32, (a.sin() * 3.0).round() as i32)
        }).collect(),
    };
    for (dx, dy) in dots {
        text::fill_rect(frame, cx + dx * s - s / 2, cy + dy * s - s / 2, s as usize, s as usize, color);
    }
}

/// Render every switched-on TV view from its camera object into its wall image.
fn render_tvs(renderer: &mut Renderer, session: &mut Session, shade_rows: &[[u8; 256]], settings: &RenderSettings, palette: &[[u8; 3]; 256]) {
    for t in 0..session.tvs.len() {
        let tv = &session.tvs[t];
        if !tv.on {
            continue;
        }
        let Some(cam_obj) = tv.camera.and_then(|c| session.objects.list.get(c)) else { continue };
        let Some(region) = cam_obj.region.or_else(|| session.level.world.region_at(cam_obj.x, cam_obj.y)) else { continue };
        let cam = Camera { x: cam_obj.x, y: cam_obj.y, z: cam_obj.z + cam_obj.height * 0.75, angle: cam_obj.angle, pitch: 0.0, region };
        let (w, h) = tv.size;
        let image = tv.image;
        let mut small = Frame::new(w, h);
        small.palette = *palette;
        let shading = Shading { rows: shade_rows };
        let tv_settings = RenderSettings { true_color: false, ..*settings };
        renderer.render(&session.level.world, &cam, &shading, &tv_settings, &mut small, Viewport { x: 0, y: 0, width: w, height: h });
        if let Some(img) = session.level.world.images.get_mut(image) {
            img.pixels = small.pixels;
            img.width = w;
            img.height = h;
        }
    }
}

/// Screen overlays (OTEXOVER, e.g. rain): transparent picture at its 320x200 position,
/// scaled with the frame and tiled across wide screens.
fn draw_overlays(frame: &mut Frame, session: &Session) {
    let scale = frame.height as f32 / CLASSIC_HEIGHT as f32;
    for (tex, x, y, anim) in &session.overlays {
        let Some(seq) = session.level.sequences.get(*tex).and_then(Option::as_ref) else { continue };
        let entry = anim.map_or(0, |a| a.entry);
        let Some(&(pic, _)) = seq.rotations_of(entry).first() else { continue };
        let Some(img) = session.level.world.images.get(pic) else { continue };
        let (w, h) = ((img.width as f32 * scale).max(1.0) as i32, (img.height as f32 * scale).max(1.0) as i32);
        let mut left = (*x as f32 * scale) as i32;
        while left < frame.width as i32 {
            let top = (*y as f32 * scale) as i32;
            for py in top.max(0)..(top + h).min(frame.height as i32) {
                let v = (((py - top) as f32 / scale) as usize).min(img.height - 1);
                for px in left.max(0)..(left + w).min(frame.width as i32) {
                    let u = (((px - left) as f32 / scale) as usize).min(img.width - 1);
                    let texel = img.pixels[v * img.width + u];
                    if texel != 0 {
                        frame.put(py as usize * frame.width + px as usize, texel);
                    }
                }
            }
            left += w;
        }
    }
}

/// F2 automap: walls within view, centred on the player and rotated so "up" is forward.
/// First-person tool picture: full bright, centred, bottom row at y = 208 of the 320x200 screen
/// (so the last 8 rows are off-screen), scaled with the frame height (combat.md §5.2).
fn draw_weapon(frame: &mut Frame, session: &Session, shade_rows: &[[u8; 256]]) {
    let Some(pic) = session.objects.weapon_pic(&session.level) else { return };
    let Some(image) = session.level.world.images.get(pic) else { return };
    let scale = frame.height as f32 / CLASSIC_HEIGHT as f32;
    let (w, h) = ((image.width as f32 * scale) as i32, (image.height as f32 * scale) as i32);
    let left = frame.width as i32 / 2 - w / 2;
    let top = (208.0 * scale) as i32 - h;
    let row = &shade_rows[0];
    for y in top.max(0)..(top + h).min(frame.height as i32) {
        let v = (((y - top) as f32 / scale) as usize).min(image.height - 1);
        for x in left.max(0)..(left + w).min(frame.width as i32) {
            let u = (((x - left) as f32 / scale) as usize).min(image.width - 1);
            let texel = image.pixels[v * image.width + u];
            if texel != 0 {
                frame.put(y as usize * frame.width + x as usize, match row[texel as usize] {
                    0 => 8,
                    c => c,
                });
            }
        }
    }
}

/// The first-person weapon as a screen-space quad for the GPU path (same placement as
/// `draw_weapon`: centred, scaled with the screen height, bottom row 8 lines below the screen),
/// so HD packs and modern lighting's colour handling apply to it.
fn weapon_quad(out: &mut engine_scene::scene::SceneFrame, session: &Session, aspect: f32) {
    use engine_scene::scene::{CLAMP, NO_DISTANCE, SCREEN, SceneVertex, TRANSPARENT};
    let Some(pic) = session.objects.weapon_pic(&session.level) else { return };
    let Some(image) = session.level.world.images.get(pic) else { return };
    // In units of the 320x200 screen height: 2 NDC units = 200 rows.
    let (w, h) = (image.width as f32 / CLASSIC_HEIGHT as f32 * 2.0 / aspect, image.height as f32 / CLASSIC_HEIGHT as f32 * 2.0);
    let bottom = 1.0 - 208.0 / CLASSIC_HEIGHT as f32 * 2.0;
    let corner = |x: f32, y: f32, u: f32, v: f32| SceneVertex {
        pos: [x, y, 0.0],
        uv: [u, v],
        image: pic as u32,
        light: 0.0,
        flags: SCREEN | TRANSPARENT | CLAMP | NO_DISTANCE,
        normal: [0.0, 0.0, 1.0],
    };
    let (iw, ih) = (image.width as f32, image.height as f32);
    let start = out.vertices.len();
    out.quad([corner(-w / 2.0, bottom, 0.0, ih), corner(w / 2.0, bottom, iw, ih), corner(w / 2.0, bottom + h, iw, 0.0), corner(-w / 2.0, bottom + h, 0.0, 0.0)]);
    out.batch(start, None);
}

/// Scale an image to fit the frame (nearest neighbour), centred, honouring pixel aspect.
fn blit_fit(frame: &mut Frame, width: usize, height: usize, pixels: &[u8]) {
    let scale = (frame.width as f32 / width as f32).min(frame.height as f32 * frame.pixel_aspect / height as f32);
    let w = ((width as f32 * scale) as usize).clamp(1, frame.width);
    let h = ((height as f32 * scale / frame.pixel_aspect) as usize).clamp(1, frame.height);
    let x0 = (frame.width - w) / 2;
    let y0 = (frame.height - h) / 2;
    for y in 0..h {
        let src = &pixels[(y * height / h).min(height - 1) * width..][..width];
        for x in 0..w {
            frame.put((y0 + y) * frame.width + x0 + x, src[(x * width / w).min(width - 1)]);
        }
    }
}
