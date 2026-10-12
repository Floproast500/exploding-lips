//! Native front end: window, input and presentation for the Exploding Lips reimplementation.
//!
//!     lips-desktop --data <GAME_DIR> [--size WxH] [--classic] [--fullscreen] [--no-vsync] [--software] [--soundfont FILE.sf2] [--mute] [--level LIPSnn] [--true-color] [--fov DEG] [--host [--port N] [--ws-port N]] [--join HOST[:PORT]] [--name NAME] [--mod DIR]...\nsettings file: {}

mod audio;
mod config;
mod gamepad;
mod present;

use godd_net::session as net;
use godd_game::{Action, CLASSIC_HEIGHT, CLASSIC_PIXEL_ASPECT, CLASSIC_WIDTH, FixedClock, Frame, Game, Input, TICK_RATE, Vfs};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

struct Options {
    settings: config::Settings,
    data: PathBuf,
    size: (u32, u32),
    /// Render at the original 320x200 (shown 4:3) instead of the window's native resolution.
    classic: bool,
    fullscreen: bool,
    vsync: bool,
    /// Present on the CPU even when a GPU is available.
    software: bool,
    soundfont: Option<PathBuf>,
    mute: bool,
    level: Option<String>,
    true_color: bool,
    host: bool,
    join: Option<String>,
    port: u16,
    ws_port: Option<u16>,
    name: String,
    mods: Vec<PathBuf>,
}

fn parse_args() -> Result<Options, String> {
    let mut settings = config::Settings::load();
    // Steam Deck (Steam sets SteamDeck=1): native 1280x800 fullscreen unless configured.
    let steam_deck = std::env::var("SteamDeck").is_ok_and(|v| v == "1");
    if steam_deck && config::path().is_none_or(|p| !p.exists()) {
        settings.size = (1280, 800);
        settings.fullscreen = true;
    }
    let mut opts = Options {
        data: settings.data.clone().unwrap_or_else(|| PathBuf::from("original")),
        size: settings.size,
        classic: settings.classic,
        fullscreen: settings.fullscreen,
        vsync: settings.vsync,
        software: false,
        soundfont: settings.soundfont.clone(),
        mute: false,
        level: None,
        true_color: settings.true_color,
        host: false,
        join: None,
        port: godd_net::DEFAULT_PORT,
        ws_port: None,
        mods: Vec::new(),
        name: std::env::var("USER").unwrap_or_else(|_| "player".into()),
        settings,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => opts.data = args.next().ok_or("--data needs a directory")?.into(),
            "--size" => {
                let v = args.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                opts.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--classic" => opts.classic = true,
            "--fullscreen" => opts.fullscreen = true,
            "--no-vsync" => opts.vsync = false,
            "--software" => opts.software = true,
            "--soundfont" => opts.soundfont = Some(args.next().ok_or("--soundfont needs a .sf2 file")?.into()),
            "--mute" => opts.mute = true,
            "--true-color" => opts.true_color = true,
            "--level" => opts.level = Some(args.next().ok_or("--level needs a name like LIPS00")?),
            "--host" => opts.host = true,
            "--join" => opts.join = Some(args.next().ok_or("--join needs host[:port]")?),
            "--port" => opts.port = args.next().and_then(|v| v.parse().ok()).ok_or("--port needs a number")?,
            "--mod" => opts.mods.push(args.next().ok_or("--mod needs a directory")?.into()),
            "--ws-port" => opts.ws_port = Some(args.next().and_then(|v| v.parse().ok()).ok_or("--ws-port needs a number")?),
            "--name" => opts.name = args.next().ok_or("--name needs a name")?,
            "--fov" => opts.settings.fov = args.next().and_then(|v| v.parse().ok()).ok_or("--fov needs degrees")?,
            "--help" | "-h" => {
                let path = config::path().map(|p| p.display().to_string()).unwrap_or_default();
                return Err(format!("usage: lips-desktop --data <GAME_DIR> [--size WxH] [--classic] [--fullscreen] [--no-vsync] [--software] [--soundfont FILE.sf2] [--mute] [--level LIPSnn] [--true-color] [--fov DEG] [--host [--port N] [--ws-port N]] [--join HOST[:PORT]] [--name NAME] [--mod DIR]...\nsettings file: {path}"));
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(opts)
}

struct App {
    opts: Options,
    game: Game,
    vfs: Vfs,
    audio: Option<audio::Audio>,
    gamepads: Option<gamepad::Gamepads>,
    input: Input,
    /// Where character saves are written (the game folder may be read-only).
    save_path: Option<PathBuf>,
    net: Option<net::NetSession>,
    bindings: Bindings,
    clock: FixedClock,
    frame: Frame,
    last: Instant,
    window: Option<Arc<Window>>,
    mouse_captured: bool,
    presenter: Option<present::Presenter>,
}

impl App {
    fn frame_size(&self) -> (usize, usize) {
        match (&self.presenter, self.opts.classic) {
            (Some(p), false) => {
                let (w, h) = p.surface_size();
                (w as usize, h as usize)
            }
            _ => (CLASSIC_WIDTH, CLASSIC_HEIGHT),
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        if let Some(pads) = &mut self.gamepads {
            pads.update(&mut self.input);
        }
        let elapsed = (now - self.last).as_secs_f64();
        if let Some(net) = &mut self.net {
            net.update(&mut self.game, &self.vfs, elapsed);
        }
        let ticks = self.clock.advance(elapsed);
        self.last = now;
        for _ in 0..ticks {
            self.game.tick(&self.vfs, &self.input);
            self.input.end_tick();
        }
        if let (Some(bytes), Some(path)) = (self.game.take_save(), &self.save_path) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(path, bytes) {
                log::warn!("cannot write save {}: {e}", path.display());
            }
        }
        if let (Some(text), Some(path)) = (self.game.take_quicksave(), config::path().map(|p| p.with_file_name("quick.sav")))
            && let Err(e) = std::fs::write(&path, text) {
                log::warn!("cannot write quicksave {}: {e}", path.display());
            }
        if self.game.quit_requested() {
            self.save_settings();
            std::process::exit(0);
        }
        let events = self.game.drain_audio();
        if let Some(audio) = &mut self.audio {
            for event in &events {
                audio.handle(&self.vfs, event);
            }
        }
        let (w, h) = self.frame_size();
        self.frame.resize(w, h);
        if let Some((action, key)) = self.game.options_menu.take_rebind()
            && let Some(code) = key_from_name(&key) {
                self.bindings.custom.insert(action, code);
                self.opts.settings.bindings.insert(format!("{action:?}"), key);
            }
        self.game.options_menu.binding_names = self.bindings.names();
        // The in-game options overlay is the source of truth while running.
        let o = self.game.options;
        self.opts.classic = o.classic;
        self.opts.true_color = o.true_color;
        self.opts.settings.fov = o.fov;
        self.opts.settings.mouse_sensitivity = o.mouse_sensitivity;
        self.opts.settings.invert_mouse = o.invert_mouse;
        self.frame.pixel_aspect = if self.opts.classic { CLASSIC_PIXEL_ASPECT } else { 1.0 };
        self.frame.set_true_color(self.opts.true_color);
        self.game.render(&mut self.frame, self.clock.alpha());
        if let Some(p) = &mut self.presenter {
            p.present(&self.frame, self.game.palette());
        }
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else { return };
        let down = event.state == ElementState::Pressed;
        if code == KeyCode::F11 && down && !event.repeat
            && let Some(w) = &self.window {
                w.set_fullscreen(w.fullscreen().is_none().then_some(Fullscreen::Borderless(None)));
            }
        let _ = event_loop;
        for action in self.bindings.actions(code) {
            self.input.set(action, down);
        }
        if down && !event.repeat {
            self.input.last_key = Some(key_name(code));
        }
        if down {
            if code == KeyCode::Backspace {
                self.input.text.push('\u{8}');
            } else if let Some(text) = &event.text {
                self.input.text.extend(text.chars().filter(|c| !c.is_control()));
            }
        }
    }
}

/// Keys that can be bound, for converting between key names and codes.
const BINDABLE_KEYS: &[KeyCode] = &[
    KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::ArrowLeft, KeyCode::ArrowRight,
    KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG,
    KeyCode::KeyH, KeyCode::KeyI, KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN,
    KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT, KeyCode::KeyU,
    KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ,
    KeyCode::Space, KeyCode::Tab, KeyCode::CapsLock, KeyCode::ShiftLeft, KeyCode::ShiftRight,
    KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft, KeyCode::AltRight, KeyCode::Enter,
    KeyCode::Insert, KeyCode::Delete, KeyCode::Home, KeyCode::End, KeyCode::PageUp, KeyCode::PageDown,
    KeyCode::Numpad0, KeyCode::Numpad1, KeyCode::Numpad2, KeyCode::Numpad3, KeyCode::Numpad4,
    KeyCode::Numpad5, KeyCode::Numpad6, KeyCode::Numpad7, KeyCode::Numpad8, KeyCode::Numpad9,
    KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4, KeyCode::F5, KeyCode::F8, KeyCode::F12,
    KeyCode::Comma, KeyCode::Period, KeyCode::Slash, KeyCode::Semicolon, KeyCode::Quote,
    KeyCode::BracketLeft, KeyCode::BracketRight, KeyCode::Backslash, KeyCode::Minus, KeyCode::Equal,
];

fn key_name(code: KeyCode) -> String {
    format!("{code:?}")
}

fn key_from_name(name: &str) -> Option<KeyCode> {
    BINDABLE_KEYS.iter().copied().find(|&c| key_name(c) == name)
}

/// Player-configured bindings for the rebindable actions (overrides the defaults).
#[derive(Default)]
struct Bindings {
    custom: std::collections::HashMap<Action, KeyCode>,
}

impl Bindings {
    fn actions(&self, code: KeyCode) -> Vec<Action> {
        let mut actions: Vec<Action> = default_bindings(code)
            .iter()
            .copied()
            .filter(|a| !self.custom.contains_key(a))
            .collect();
        actions.extend(self.custom.iter().filter(|(_, c)| **c == code).map(|(a, _)| *a));
        actions
    }

    fn names(&self) -> Vec<(Action, String)> {
        godd_game::options::REBINDABLE
            .iter()
            .map(|&(action, _)| {
                let key = self.custom.get(&action).copied().or_else(|| {
                    BINDABLE_KEYS.iter().copied().chain([KeyCode::Enter, KeyCode::Escape]).find(|&c| default_bindings(c).contains(&action))
                });
                (action, key.map(key_name).unwrap_or_else(|| "-".into()))
            })
            .collect()
    }
}

/// Default bindings, following the original keyboard.txt plus WASD-style extras.
fn default_bindings(code: KeyCode) -> &'static [Action] {
    use Action::*;
    match code {
        KeyCode::ArrowUp => &[Forward, MenuUp],
        KeyCode::ArrowDown => &[Back, MenuDown],
        KeyCode::ArrowLeft => &[TurnLeft],
        KeyCode::ArrowRight => &[TurnRight],
        KeyCode::AltLeft | KeyCode::AltRight => &[StrafeModifier],
        KeyCode::KeyA => &[StrafeLeft],
        KeyCode::KeyD => &[StrafeRight],
        KeyCode::ShiftLeft | KeyCode::ShiftRight => &[Run],
        KeyCode::Tab | KeyCode::Insert | KeyCode::Space => &[Jump],
        KeyCode::CapsLock | KeyCode::Delete | KeyCode::KeyC => &[Crouch],
        KeyCode::KeyW | KeyCode::PageUp => &[LookUp],
        KeyCode::KeyX | KeyCode::PageDown => &[LookDown],
        KeyCode::KeyS | KeyCode::End => &[CenterView],
        KeyCode::ControlLeft | KeyCode::ControlRight => &[Use],
        KeyCode::Enter | KeyCode::NumpadEnter => &[MenuSelect, Use],
        KeyCode::Escape => &[Menu],
        KeyCode::F2 => &[Map],
        KeyCode::NumpadMultiply => &[Brighten],
        KeyCode::NumpadDivide => &[Darken],
        KeyCode::F10 => &[Options],
        KeyCode::Backquote => &[Console],
        KeyCode::F6 => &[QuickSave],
        KeyCode::F7 => &[QuickLoad],
        KeyCode::Digit0 => &[Tool0],
        KeyCode::Digit1 => &[Tool1],
        KeyCode::Digit2 => &[Tool2],
        KeyCode::Digit3 => &[Tool3],
        KeyCode::Digit4 => &[Tool4],
        KeyCode::Digit5 => &[Tool5],
        KeyCode::Digit6 => &[Tool6],
        KeyCode::Digit7 => &[Tool7],
        KeyCode::Digit8 => &[Tool8],
        KeyCode::Digit9 => &[Tool9],
        _ => &[],
    }
}

impl App {
    fn save_settings(&mut self) {
        let s = &mut self.opts.settings;
        s.data = Some(std::fs::canonicalize(&self.opts.data).unwrap_or_else(|_| self.opts.data.clone()));
        s.true_color = self.opts.true_color;
        s.classic = self.opts.classic;
        if let Some(w) = &self.window {
            s.fullscreen = w.fullscreen().is_some();
            if !s.fullscreen {
                let size = w.inner_size();
                s.size = (size.width, size.height);
            }
        }
        s.save();
    }

    fn capture_mouse(&mut self, capture: bool) {
        let Some(window) = &self.window else { return };
        if capture {
            let grabbed = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            if grabbed.is_err() {
                return;
            }
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
        }
        window.set_cursor_visible(!capture);
        self.mouse_captured = capture;
    }
}

impl ApplicationHandler for App {
    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event
            && self.mouse_captured {
                let s = self.opts.settings.mouse_sensitivity;
                let invert = if self.opts.settings.invert_mouse { -1.0 } else { 1.0 };
                self.input.look_delta.0 += dx as f32 * s;
                self.input.look_delta.1 += dy as f32 * s * invert;
            }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("Exploding Lips")
            .with_inner_size(PhysicalSize::new(self.opts.size.0, self.opts.size.1));
        if self.opts.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.presenter = Some(present::Presenter::new(window.clone(), self.opts.vsync, self.opts.software));
        self.window = Some(window);
        self.last = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.save_settings();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(p) = &mut self.presenter {
                    p.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape) && self.mouse_captured {
                    self.capture_mouse(false);
                }
                self.key(event_loop, &event);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                if down && !self.mouse_captured {
                    self.capture_mouse(true);
                }
                match button {
                    // Original: LMB = use tool, RMB = run forward.
                    MouseButton::Left => self.input.set(Action::Use, down),
                    MouseButton::Right => self.input.set(Action::Forward, down),
                    _ => {}
                }
            }
            WindowEvent::Focused(false) => {
                // Losing focus also precedes suspend on handhelds: persist settings now.
                self.capture_mouse(false);
                self.save_settings();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let opts = parse_args().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let mut vfs = Vfs::from_dir(&opts.data).unwrap_or_else(|e| {
        eprintln!("cannot read game data from {}: {e}", opts.data.display());
        std::process::exit(1);
    });
    for dir in &opts.mods {
        match vfs.overlay_dir(dir) {
            Ok(n) => log::info!("mod {}: {n} files override the game data", dir.display()),
            Err(e) => log::warn!("mod {}: {e}", dir.display()),
        }
    }
    let save_path = config::path().map(|p| p.with_file_name("LIPSTART.GAM"));
    let save = save_path.as_ref().and_then(|p| std::fs::read(p).ok());
    let game = Game::new(&vfs, &godd_game::Options { start_level: opts.level.clone(), save }).unwrap_or_else(|e| {
        eprintln!("cannot start game: {e}");
        std::process::exit(1);
    });
    let audio = if opts.mute { None } else { audio::Audio::new(opts.soundfont.as_deref()) };
    let mut game = game;
    let quick_path = config::path().map(|p| p.with_file_name("quick.sav"));
    if let Some(text) = quick_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
        game.set_quicksave(text);
    }
    game.options = godd_game::options::GameOptions {
        fov: opts.settings.fov,
        true_color: opts.true_color,
        classic: opts.classic,
        mouse_sensitivity: opts.settings.mouse_sensitivity,
        invert_mouse: opts.settings.invert_mouse,
        ..Default::default()
    };
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        opts,
        game,
        vfs,
        audio,
        gamepads: gamepad::Gamepads::new(),
        input: Input::default(),
        save_path,
        net: None,
        bindings: Bindings::default(),
        clock: FixedClock::new(TICK_RATE),
        frame: Frame::new(640, 480),
        last: Instant::now(),
        window: None,
        mouse_captured: false,
        presenter: None,
    };
    for (action_name, key) in &app.opts.settings.bindings {
        let action = godd_game::options::REBINDABLE.iter().find(|(a, _)| format!("{a:?}") == *action_name).map(|(a, _)| *a);
        if let (Some(action), Some(code)) = (action, key_from_name(key)) {
            app.bindings.custom.insert(action, code);
        }
    }
    if app.opts.host || app.opts.join.is_some() {
        let level = app.opts.level.clone().unwrap_or_else(|| "LIPS00".into());
        let result = match &app.opts.join {
            Some(address) => net::NetSession::join(address, &app.opts.name),
            None => {
                if app.game.session().is_none()
                    && let Err(e) = app.game.start_level(&app.vfs, &level, godd_game::session::Entry::Fresh) {
                        eprintln!("cannot load {level}: {e}");
                        std::process::exit(1);
                    }
                net::NetSession::host(app.opts.port, &level, &app.opts.name).and_then(|mut n| {
                    if let Some(ws) = app.opts.ws_port {
                        n.enable_websocket(ws)?;
                    }
                    Ok(n)
                })
            }
        };
        match result {
            Ok(n) => app.net = Some(n),
            Err(e) => {
                eprintln!("netplay: {e}");
                std::process::exit(1);
            }
        }
    }
    event_loop.run_app(&mut app).expect("event loop");
}
