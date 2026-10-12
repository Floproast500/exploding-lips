//! Platform-independent core of the Exploding Lips reimplementation.
//!
//! Front ends (desktop, web) own the window, audio device and input devices; they feed
//! [`Input`] into [`Game::tick`] at [`TICK_RATE`] and present the [`Frame`] that
//! [`Game::render`] fills, using [`Game::palette`] to turn indices into colours.

pub mod anim;
mod audio;
pub use engine_core::clock;
pub mod effects;
pub use engine_core::frame;
pub mod hud;
mod input;
pub mod level;
pub mod menus;
pub mod mover;
pub mod objects;
pub mod options;
pub mod player;
pub mod render;
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
    ui_colors: (u8, u8, u8, u8),
    fps: FpsCounter,
    /// Ticks since the game was created (the original HUD clock is time since launch).
    uptime_ticks: u64,
    /// Character save slots, carried across levels.
    save: godd_formats::gam::SaveFile,
    current_slot: Option<usize>,
    save_dirty: bool,
    console: Console,
    /// Quicksave contents and whether they changed since the front end last stored them.
    quicksave: Option<String>,
    quicksave_dirty: bool,
    /// F2 automap overlay.
    show_map: bool,
}

/// The engine console (~): runs INI commands like the original, plus `warp LEVEL`.
#[derive(Default)]
struct Console {
    open: bool,
    line: String,
    output: Vec<String>,
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
            ui_colors: (
                text::nearest_color(&palette.rgb, [10, 10, 40]),
                text::nearest_color(&palette.rgb, [24, 24, 252]),
                text::nearest_color(&palette.rgb, [200, 200, 200]),
                text::nearest_color(&palette.rgb, [255, 40, 40]),
            ),
            fps: FpsCounter::default(),
            uptime_ticks: 0,
            save: options
                .save
                .as_deref()
                .or_else(|| vfs.read("LIPSTART.GAM").ok())
                .and_then(|b| godd_formats::gam::SaveFile::parse(b).ok())
                .unwrap_or_default(),
            current_slot: None,
            save_dirty: false,
            console: Console::default(),
            quicksave: None,
            quicksave_dirty: false,
            show_map: false,
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
        if let State::Playing(old) = &mut self.state {
            self.save = std::mem::take(&mut old.save);
            self.current_slot = old.menus.current_slot;
            self.save_dirty |= old.save_dirty;
        }
        let save = std::mem::take(&mut self.save);
        let session = Session::start(vfs, name, entry, carried, save, self.current_slot)?;
        self.state = State::Playing(Box::new(session));
        Ok(())
    }

    fn console_input(&mut self, vfs: &Vfs, input: &Input) {
        for c in input.text.chars() {
            match c {
                '\u{8}' => {
                    self.console.line.pop();
                }
                '`' | '~' => {}
                c if !c.is_control() => self.console.line.push(c),
                _ => {}
            }
        }
        if input.pressed(Action::Menu) {
            self.console.open = false;
        }
        if !input.pressed(Action::MenuSelect) || self.console.line.trim().is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.console.line);
        self.console.output.push(format!("> {line}"));
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
                self.console.output.extend(reply.lines().map(str::to_string));
                self.options = options::GameOptions::from_cvars(&self.cvars);
                let excess = self.console.output.len().saturating_sub(6);
                self.console.output.drain(..excess);
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
            Some("help") => "warp LEVEL | set/get/toggle/reset/cvarlist | bind/unbind | INI commands, e.g. TAG=500".into(),
            _ => match self.session_mut() {
                Some(s) => {
                    s.run_command(&line);
                    "done".into()
                }
                None => "no level loaded".into(),
            },
        };
        self.console.output.push(reply);
        let excess = self.console.output.len().saturating_sub(6);
        self.console.output.drain(..excess);
    }

    /// F6: remember the current level, position, states and inventory.
    fn quick_save(&mut self) {
        let Some(s) = self.session() else { return };
        let p = &s.player;
        let states: Vec<String> = s.script.states.iter().map(i32::to_string).collect();
        let inventory: Vec<String> = s.objects.inventory.iter().map(|&i| s.objects.list[i].name.clone()).collect();
        self.quicksave = Some(format!("{} {} {} {} {} {} | {}", s.name, p.x, p.y, p.z, p.angle, states.join(" "), inventory.join(" ")));
        self.quicksave_dirty = true;
        if let Some(s) = self.session_mut() {
            s.show_message(script::DEFAULT_MESSAGE_COLOR, "Quick saved");
        }
    }

    /// F7: restore the quicksave (the level restarts, then position/states/inventory return).
    fn quick_load(&mut self, vfs: &Vfs) {
        let Some(text) = self.quicksave.clone() else { return };
        let (head, inventory) = text.split_once('|').unwrap_or((&text, ""));
        let parts: Vec<&str> = head.split_whitespace().collect();
        if parts.len() < 5 + script::STATE_COUNT {
            return;
        }
        let f = |i: usize| parts[i].parse::<f32>().unwrap_or(0.0);
        if self.start_level(vfs, parts[0], Entry::Fresh).is_err() {
            return;
        }
        let Some(s) = self.session_mut() else { return };
        for (k, v) in parts[5..5 + script::STATE_COUNT].iter().enumerate() {
            s.script.states[k] = v.parse().unwrap_or(0);
        }
        s.restore_position(f(1), f(2), f(3), f(4));
        let items: Vec<String> = inventory.split_whitespace().map(str::to_string).collect();
        s.restore_inventory(&items);
        s.show_message(script::DEFAULT_MESSAGE_COLOR, "Quick loaded");
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
        let Some(s) = self.session_mut() else { return };
        s.menus.current_slot = goto.slot;
        let Some(slot) = goto.slot.map(|i| s.save.slots[i].clone()) else { return };
        if slot.is_active() {
            s.script.states = slot.states();
        }
        if goto.restore {
            let (x, y, z) = slot.position();
            let angle = slot.angle() as f32 / 2048.0 * std::f32::consts::TAU;
            s.restore_position(x, y, z, angle);
            s.restore_inventory(&slot.inventory());
        }
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

    /// Audio requests produced since the last call.
    pub fn drain_audio(&mut self) -> Vec<AudioEvent> {
        if let State::Playing(session) = &mut self.state {
            self.audio.extend(session.drain_audio());
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
        let paused = self.options_menu.update(input, &mut self.cvars, &mut self.binds);
        self.options = options::GameOptions::from_cvars(&self.cvars);
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
        if input.pressed(Action::QuickSave) {
            self.quick_save();
        }
        if input.pressed(Action::QuickLoad) {
            self.quick_load(vfs);
            return;
        }
        if let State::Playing(s) = &mut self.state {
            s.message_scale = if self.options.long_messages { 2.0 } else { 1.0 };
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
                if session.quit_requested {
                    self.quit = true;
                }
                if let Some(goto) = session.pending_goto.take() {
                    self.goto(vfs, goto);
                    return;
                }
                if let Some((level, start)) = session.pending_level.take() {
                    match self.start_level(vfs, &level, Entry::Continue) {
                        Ok(()) => {
                            if let (Some(name), Some(s)) = (start, self.session_mut()) {
                                s.place_player_at(&name);
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

    /// Everything drawn over the view: automap, HUD, menus, options, console and FPS counter.
    pub fn render_ui(&mut self, frame: &mut Frame) {
        if let State::Playing(session) = &self.state {
            if self.show_map {
                draw_automap(frame, session, self.ui_colors);
            }
            let uptime = self.uptime_ticks / TICK_RATE as u64;
            self.hud.draw(frame, session, session.tool, uptime, self.options.high_contrast, self.options.hud_scale);
            draw_crosshair(frame, self.options.crosshair, self.ui_colors.3);
            session.menus.draw(frame, &session.save);
        }
        self.options_menu.draw(frame, &self.cvars, &self.binds, self.ui_colors);
        if self.console.open {
            let s = (frame.height / CLASSIC_HEIGHT).max(1);
            let line_h = (text::GLYPH + 2) * s;
            let rows = self.console.output.len() + 1;
            text::fill_rect(frame, 0, 0, frame.width, line_h * rows + 2 * s, self.ui_colors.0);
            for (i, l) in self.console.output.iter().enumerate() {
                text::draw_text(frame, 2 * s as i32, (i * line_h + s) as i32, s, self.ui_colors.2, l);
            }
            let prompt = format!("] {}_", self.console.line);
            text::draw_text(frame, 2 * s as i32, (self.console.output.len() * line_h + s) as i32, s, self.ui_colors.3, &prompt);
        }
        if self.options.show_fps {
            let s = (frame.height / CLASSIC_HEIGHT).max(1);
            text::draw_text(frame, 2 * s as i32, 2 * s as i32, s, self.ui_colors.3, &format!("{} fps", self.fps.shown));
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
    Camera {
        x: lerp(p.x, q.x),
        y: lerp(p.y, q.y),
        z: lerp(p.eye_z(physics), q.eye_z(physics)),
        angle: (p.angle + angle_delta * alpha + q.direct_turn + pending.0).rem_euclid(std::f32::consts::TAU),
        pitch: PhysicsParams::look_to_pitch(look),
        region: q.region,
    }
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
fn draw_automap(frame: &mut Frame, session: &Session, colors: (u8, u8, u8, u8)) {
    let world = &session.level.world;
    let p = &session.player;
    let scale = frame.height as f32 / 1600.0; // ~1600 map units fill the screen height
    let (cx, cy) = (frame.width as f32 / 2.0, frame.height as f32 / 2.0);
    let (sin, cos) = (p.angle - std::f32::consts::FRAC_PI_2).sin_cos();
    let to_screen = |x: f32, y: f32| {
        let (dx, dy) = (x - p.x, y - p.y);
        let (rx, ry) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        (cx + rx * scale, cy - ry * scale)
    };
    let (_, _, portal, solid) = colors;
    for wall in &world.walls {
        // Only walls of regions at roughly the player's level.
        let near_level = |r: usize| world.regions.get(r).is_some_and(|reg| reg.floor <= p.z + 64.0 && reg.ceiling >= p.z);
        if !near_level(wall.front) && !wall.back.is_some_and(near_level) {
            continue;
        }
        let (ax, ay) = world.vertices[wall.a];
        let (bx, by) = world.vertices[wall.b];
        let (x0, y0) = to_screen(ax, ay);
        let (x1, y1) = to_screen(bx, by);
        let color = if wall.back.is_some_and(|b| b != wall.front) { portal } else { solid };
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as usize;
        if steps > 4000 {
            continue;
        }
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            if x >= 0.0 && y >= 0.0 && (x as usize) < frame.width && (y as usize) < frame.height {
                frame.put(y as usize * frame.width + x as usize, color);
            }
        }
    }
    // The player: a small arrow pointing up (forward).
    let s = (frame.height / CLASSIC_HEIGHT).max(1) as i32;
    for k in 0..4 * s {
        for w in -k / 2..=k / 2 {
            let (x, y) = (cx as i32 + w, cy as i32 - 2 * s + k);
            if x >= 0 && y >= 0 && (x as usize) < frame.width && (y as usize) < frame.height {
                frame.put(y as usize * frame.width + x as usize, solid);
            }
        }
    }
}

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
