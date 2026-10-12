//! Native front end: window, input and presentation for the Exploding Lips reimplementation.
//!
//!     lips-desktop [--data <GAME_DIR>] [--size WxH] [--classic] [--fullscreen] [--no-vsync] [--software] [--soundfont FILE.sf2] [--mute] [--level LIPSnn] [--true-color] [--fov DEG] [--set NAME VALUE]... [--host [--port N] [--ws-port N]] [--join HOST[:PORT]] [--name NAME] [--mod DIR]...
//!
//! Options and bindings live in config.cfg (see [`config`]); command-line options override it.

mod audio;
mod config;
mod gamepad;
mod module_app;
mod present;
#[cfg(feature = "sdl")]
mod sdl_pad;

use engine_core::{BindTable, CvarRegistry};
use godd_game::{CLASSIC_HEIGHT, CLASSIC_PIXEL_ASPECT, CLASSIC_WIDTH, FixedClock, Frame, Game, Input, TICK_RATE, Vfs};
use godd_net::session as net;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

const USAGE: &str = "usage: lips-desktop [--data <GAME_DIR>] [--size WxH] [--classic] [--fullscreen] [--no-vsync] [--software] [--soundfont FILE.sf2] [--mute] [--level LIPSnn] [--true-color] [--fov DEG] [--set NAME VALUE]... [--host [--port N] [--ws-port N]] [--join HOST[:PORT]] [--name NAME] [--mod DIR]... [--screenshot-after SECONDS FILE.png] | --pad-test [SECONDS] | --bsp <PAK|BSP> <MAP>";

/// Command-line options that are not cvars (cvar options are applied to the registry directly).
struct Options {
    /// Present on the CPU even when a GPU is available.
    software: bool,
    mute: bool,
    level: Option<String>,
    host: bool,
    join: Option<String>,
    port: u16,
    ws_port: Option<u16>,
    name: String,
    mods: Vec<PathBuf>,
    /// `--screenshot-after SECONDS FILE`: save a screenshot after that long, then quit.
    screenshot_after: Option<(f64, PathBuf)>,
    /// Cvars set on the command line: (name, value before, value set). They last for this run
    /// only; config.cfg keeps the earlier value unless the player changes it in game.
    overrides: Vec<(String, String, String)>,
}

fn parse_args(cvars: &mut CvarRegistry) -> Result<Options, String> {
    let mut opts = Options {
        software: false,
        mute: false,
        level: None,
        host: false,
        join: None,
        port: godd_net::DEFAULT_PORT,
        ws_port: None,
        mods: Vec::new(),
        screenshot_after: None,
        overrides: Vec::new(),
        name: std::env::var("USER").unwrap_or_else(|_| "player".into()),
    };
    let mut args = std::env::args().skip(1);
    let mut overrides = Vec::new();
    let mut set = |name: &str, value: &str| {
        let before = cvars.get_str(name);
        cvars.set(name, value)?;
        // The game folder is remembered on purpose (so later runs need no --data).
        if name != "fs_data" {
            overrides.push((name.to_string(), before, cvars.get_str(name)));
        }
        Ok::<(), String>(())
    };
    while let Some(arg) = args.next() {
        let mut next = |what: &str| args.next().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--data" => set("fs_data", &next("a directory")?)?,
            "--size" => {
                let v = next("WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                set("vid_width", w)?;
                set("vid_height", h)?;
            }
            "--classic" => set("r_classic", "1")?,
            "--fullscreen" => set("vid_fullscreen", "1")?,
            "--no-vsync" => set("vid_vsync", "0")?,
            "--true-color" => set("r_truecolor", "1")?,
            "--fov" => set("r_fov", &next("degrees")?)?,
            "--soundfont" => set("snd_soundfont", &next("a .sf2 file")?)?,
            "--set" => {
                let name = next("a name")?;
                set(&name, &next("a value")?)?;
            }
            "--software" => opts.software = true,
            "--mute" => opts.mute = true,
            "--screenshot-after" => {
                let secs = next("seconds")?.parse().map_err(|_| "--screenshot-after needs seconds")?;
                opts.screenshot_after = Some((secs, PathBuf::from(next("a .png file")?)));
            }
            "--level" => opts.level = Some(next("a name like LIPS00")?),
            "--host" => opts.host = true,
            "--join" => opts.join = Some(next("host[:port]")?),
            "--port" => opts.port = next("a number")?.parse().map_err(|_| "--port needs a number")?,
            "--mod" => opts.mods.push(next("a directory")?.into()),
            "--ws-port" => opts.ws_port = Some(next("a number")?.parse().map_err(|_| "--ws-port needs a number")?),
            "--name" => opts.name = next("a name")?,
            "--help" | "-h" => {
                let path = config::path().map(|p| p.display().to_string()).unwrap_or_default();
                return Err(format!("{USAGE}\nsettings file: {path}"));
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    opts.overrides = overrides;
    Ok(opts)
}

struct App {
    opts: Options,
    game: Game,
    vfs: Vfs,
    audio: Option<audio::Audio>,
    gamepads: Option<gamepad::Gamepads>,
    #[cfg(feature = "sdl")]
    sdl_pads: Option<sdl_pad::SdlPads>,
    input: Input,
    /// Where character saves are written (the game folder may be read-only).
    save_path: Option<PathBuf>,
    net: Option<net::NetSession>,
    clock: FixedClock,
    frame: Frame,
    /// UI layer composited over the GPU world view.
    ui: Frame,
    /// HD texture pack (r_texpack).
    pack: Option<engine_core::pack::LoadedPack>,
    last: Instant,
    started: Instant,
    /// Screenshot requested by --screenshot-after (quit once it is saved).
    auto_shot: bool,
    window: Option<Arc<Window>>,
    mouse_captured: bool,
    presenter: Option<present::Presenter>,
}

impl App {
    fn frame_size(&self) -> (usize, usize) {
        match (&self.presenter, self.game.options.classic) {
            (Some(p), false) => {
                let (w, h) = p.surface_size();
                (w as usize, h as usize)
            }
            _ => (CLASSIC_WIDTH, CLASSIC_HEIGHT),
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        // Automated screenshot: request it once the time has passed; quit on the next frame
        // (saving settings as a normal quit does; command-line overrides are not kept).
        if let Some((secs, path)) = &self.opts.screenshot_after
            && (now - self.started).as_secs_f64() >= *secs
            && let Some(p) = &mut self.presenter
        {
            if self.auto_shot {
                self.save_settings();
                std::process::exit(0);
            }
            p.request_screenshot(path.clone());
            self.auto_shot = true;
        }
        let elapsed = (now - self.last).as_secs_f64();
        #[cfg(feature = "sdl")]
        if let Some(pads) = &mut self.sdl_pads {
            let gyro = self.game.cvars.get_bool("joy_gyro").then(|| self.game.cvars.get_float("joy_gyro_sensitivity") as f32);
            pads.update(&mut self.input, &self.game.binds, gyro, elapsed as f32);
        }
        if let Some(pads) = &mut self.gamepads {
            pads.update(&mut self.input, &self.game.binds);
        }
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
        if let (Some(text), Some(path)) = (self.game.take_quicksave(), config::dir().map(|d| d.join("quick.sav")))
            && let Err(e) = std::fs::write(&path, text) {
                log::warn!("cannot write quicksave {}: {e}", path.display());
            }
        if let Some(text) = self.game.take_config() {
            config::save(&text);
        }
        if let Some(dir) = config::dir().map(|d| d.join("saves")) {
            for (n, text) in self.game.take_slots() {
                let _ = std::fs::create_dir_all(&dir);
                if let Err(e) = std::fs::write(dir.join(format!("slot{}.sav", n + 1)), text) {
                    log::warn!("cannot write save slot {}: {e}", n + 1);
                }
            }
        }
        if self.game.quit_requested() {
            self.save_settings();
            std::process::exit(0);
        }
        let haptics = self.game.drain_haptics();
        if let Some(pads) = &mut self.gamepads {
            for h in &haptics {
                pads.rumble(h);
            }
        }
        #[cfg(feature = "sdl")]
        if let Some(pads) = &mut self.sdl_pads {
            for h in &haptics {
                pads.rumble(h);
            }
        }
        let events = self.game.drain_audio();
        if let Some(audio) = &mut self.audio {
            audio.set_volumes(self.game.options.volume);
            audio.set_spatial(self.game.cvars.get_bool("snd_spatial"));
            for event in &events {
                audio.handle(&self.vfs, event);
            }
        }
        let fullscreen = self.game.cvars.get_bool("vid_fullscreen");
        if let Some(w) = &self.window
            && fullscreen != w.fullscreen().is_some() {
                w.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
            }
        let (w, h) = self.frame_size();
        self.game.preview_look(&self.input);
        // GPU world renderer (r_renderer gpu) when the presenter has a GPU; the software
        // renderer for classic 320x200, the title screen and CPU presentation.
        let want_gpu = self.game.cvars.get_str("r_renderer") == "gpu" && self.presenter.as_ref().is_some_and(present::Presenter::is_gpu);
        self.update_pack();
        if want_gpu {
            let (uw, uh) = self.game.ui_layer_size(w, h);
            self.ui.resize(uw, uh);
            self.ui.pixel_aspect = 1.0;
            if let Some(p) = &mut self.presenter {
                self.game.gpu_ms = p.gpu_ms();
            }
            if let Some(view) = self.game.gpu_frame(&mut self.ui, self.clock.alpha())
                && let Some(p) = &mut self.presenter
                && p.present_world(&self.game, &view, &self.ui, self.pack.as_ref())
            {
                return;
            }
        }
        self.frame.resize(w, h);
        self.frame.pixel_aspect = if self.game.options.classic { CLASSIC_PIXEL_ASPECT } else { 1.0 };
        self.frame.set_true_color(self.game.options.true_color);
        self.game.render(&mut self.frame, self.clock.alpha());
        if let Some(p) = &mut self.presenter {
            p.present(&self.frame, self.game.palette());
        }
    }

    /// Follow r_texpack: open the pack folder when it changes (textures load with the level).
    fn update_pack(&mut self) {
        let want = self.game.cvars.get_str("r_texpack");
        if self.pack.as_ref().map(|p| p.dir.display().to_string()) == Some(want.clone()).filter(|w| !w.is_empty()) || (want.is_empty() && self.pack.is_none()) {
            return;
        }
        self.pack = (!want.is_empty())
            .then(|| engine_core::pack::LoadedPack::open(std::path::Path::new(&want)).map_err(|e| log::warn!("texture pack: {e}")).ok())
            .flatten();
    }

    fn key(&mut self, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else { return };
        let down = event.state == ElementState::Pressed;
        if code == KeyCode::F12 && down && !event.repeat {
            let dir = self.save_path.as_ref().and_then(|f| f.parent()).map_or_else(|| PathBuf::from("screenshots"), |d| d.join("screenshots"));
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if let Some(p) = &mut self.presenter {
                p.request_screenshot(dir.join(format!("lips-{stamp}.png")));
            }
            return;
        }
        if code == KeyCode::F11 && down && !event.repeat {
            let on = !self.game.cvars.get_bool("vid_fullscreen");
            self.game.cvars.set_bool("vid_fullscreen", on);
        }
        if !event.repeat {
            self.input.source(&self.game.binds, &format!("key:{code:?}"), down);
        }
        if down {
            if code == KeyCode::Backspace {
                self.input.text.push('\u{8}');
            } else if code == KeyCode::Tab {
                self.input.text.push('\t'); // console completion
            } else if let Some(text) = &event.text {
                self.input.text.extend(text.chars().filter(|c| !c.is_control()));
            }
        }
    }

    fn save_settings(&mut self) {
        // Command-line overrides still in effect are written with their earlier values.
        let mut restore = Vec::new();
        for (name, before, set) in &self.opts.overrides {
            if self.game.cvars.get_str(name) == *set {
                restore.push((name.clone(), set.clone()));
                let _ = self.game.cvars.set(name, before);
            }
        }
        self.write_settings();
        for (name, value) in restore {
            let _ = self.game.cvars.set(&name, &value);
        }
    }

    fn write_settings(&mut self) {
        let data = self.game.cvars.get_str("fs_data");
        if let Ok(abs) = std::fs::canonicalize(&data) {
            let _ = self.game.cvars.set("fs_data", &abs.display().to_string());
        }
        if let Some(w) = &self.window
            && w.fullscreen().is_none() {
                let size = w.inner_size();
                self.game.cvars.set("vid_width", &size.width.to_string()).ok();
                self.game.cvars.set("vid_height", &size.height.to_string()).ok();
            }
        config::save(&self.game.config_text());
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

fn mouse_source(button: MouseButton) -> String {
    match button {
        MouseButton::Other(n) => format!("mouse:Button{n}"),
        b => format!("mouse:{b:?}"),
    }
}

impl ApplicationHandler for App {
    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event
            && self.mouse_captured {
                self.input.mouse.0 += dx as f32;
                self.input.mouse.1 += dy as f32;
            }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let c = &self.game.cvars;
        let mut attrs = Window::default_attributes()
            .with_title("Exploding Lips")
            .with_inner_size(PhysicalSize::new(c.get_int("vid_width") as u32, c.get_int("vid_height") as u32));
        if c.get_bool("vid_fullscreen") {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.presenter = Some(present::Presenter::new(window.clone(), c.get_bool("vid_vsync"), self.opts.software));
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
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape) && self.mouse_captured && !self.game.options_menu.is_capturing() {
                    self.capture_mouse(false);
                }
                self.key(&event);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                if down && !self.mouse_captured {
                    // The first click only captures the mouse.
                    self.capture_mouse(true);
                    return;
                }
                self.input.source(&self.game.binds, &mouse_source(button), down);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if y != 0.0 {
                    self.input.pulse(&self.game.binds, if y > 0.0 { "wheel:Up" } else { "wheel:Down" });
                }
            }
            WindowEvent::Focused(false) => {
                // Losing focus also precedes suspend on handhelds: persist settings now.
                self.capture_mouse(false);
                self.input.release_all();
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

/// The window event loop. Under Steam on Linux the window goes through X11 (XWayland): Steam
/// cannot see the focus of native Wayland windows and would keep the controller on its desktop
/// (mouse and keyboard) layout instead of the game's gamepad layout.
pub fn new_event_loop() -> EventLoop<()> {
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        let under_steam = std::env::var_os("SteamGameId").is_some() || std::env::var_os("SteamAppId").is_some();
        if under_steam && std::env::var_os("DISPLAY").is_some() {
            log::info!("launched by Steam: using X11 so Steam Input sees the window's focus");
            builder.with_x11();
        }
    }
    builder.build().expect("event loop")
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // `--bsp <PAK|BSP> <MAP> [palette=FILE]`: fly through a Quake-family map (Quake, Hexen II)
    // with the engine's renderer instead of starting Exploding Lips.
    let args: Vec<String> = std::env::args().collect();
    // `--pad-test [SECONDS]`: rumble every controller, then report gyro motion (SDL builds).
    if let Some(i) = args.iter().position(|a| a == "--pad-test") {
        let seconds = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
        #[cfg(feature = "sdl")]
        match sdl_pad::SdlPads::new() {
            Some(mut pads) => {
                println!("rumbling, then reading the gyro for {seconds} s: move the controller");
                for line in pads.pad_test(seconds) {
                    println!("{line}");
                }
            }
            None => eprintln!("SDL gamepads unavailable"),
        }
        #[cfg(not(feature = "sdl"))]
        eprintln!("--pad-test needs the SDL build (--features sdl), not {seconds} s of gilrs");
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--bsp") {
        let (Some(source), Some(map)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("usage: lips-desktop --bsp <PAK|BSP> <MAP> [palette=FILE]");
            std::process::exit(2);
        };
        let palette = args.iter().find_map(|a| a.strip_prefix("palette=")).map(std::path::Path::new);
        match quake_bsp::viewer::open(std::path::Path::new(source), map, palette) {
            Ok(viewer) => module_app::run(Box::new(viewer)),
            Err(e) => {
                eprintln!("cannot open {map}: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let mut cvars = CvarRegistry::default();
    godd_game::options::register_cvars(&mut cvars);
    config::register(&mut cvars);
    let mut binds: BindTable = godd_game::options::default_binds(godd_game::options::Profile::Modern);
    let existed = config::load(&mut cvars, &mut binds);
    // Steam Deck (Steam sets SteamDeck=1): native 1280x800 fullscreen unless configured.
    if !existed && std::env::var("SteamDeck").is_ok_and(|v| v == "1") {
        let _ = cvars.set("vid_width", "1280");
        let _ = cvars.set("vid_height", "800");
        cvars.set_bool("vid_fullscreen", true);
    }
    let opts = parse_args(&mut cvars).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let data = match cvars.get_str("fs_data") {
        d if d.is_empty() => PathBuf::from("original"),
        d => PathBuf::from(d),
    };
    let mut vfs = Vfs::from_dir(&data).unwrap_or_else(|e| {
        eprintln!("cannot read game data from {}: {e}", data.display());
        std::process::exit(1);
    });
    let _ = cvars.set("fs_data", &data.display().to_string());
    // Mods: --mod folders plus those enabled in the options (fs_mods).
    let enabled: Vec<PathBuf> = godd_game::options::enabled_mods(&cvars).into_iter().map(PathBuf::from).collect();
    for dir in opts.mods.iter().chain(&enabled) {
        match vfs.overlay_dir(dir) {
            Ok(n) => log::info!("mod {}: {n} files override the game data", dir.display()),
            Err(e) => log::warn!("mod {}: {e}", dir.display()),
        }
    }
    let save_path = config::dir().map(|d| d.join("LIPSTART.GAM"));
    let save = save_path.as_ref().and_then(|p| std::fs::read(p).ok());
    let mut game = Game::new(&vfs, &godd_game::Options { start_level: opts.level.clone(), save }).unwrap_or_else(|e| {
        eprintln!("cannot start game: {e}");
        std::process::exit(1);
    });
    let soundfont = Some(cvars.get_str("snd_soundfont")).filter(|s| !s.is_empty()).map(PathBuf::from);
    game.adopt_config(cvars, binds);
    let (packs, mods) = find_packs_and_mods();
    game.options_menu.packs = packs;
    game.options_menu.mods = mods;
    let audio = if opts.mute { None } else { audio::Audio::new(soundfont.as_deref()) };
    if let Some(text) = config::dir().and_then(|d| std::fs::read_to_string(d.join("quick.sav")).ok()) {
        game.set_quicksave(text);
    }
    for n in 0..godd_game::saves::SLOTS {
        if let Some(text) = config::dir().and_then(|d| std::fs::read_to_string(d.join("saves").join(format!("slot{}.sav", n + 1))).ok()) {
            game.set_slot(n, &text);
        }
    }
    // With SDL (feature "sdl") pads are read through SDL (gyro); otherwise, or if SDL can't
    // start, through gilrs.
    #[cfg(feature = "sdl")]
    let sdl_pads = sdl_pad::SdlPads::new();
    #[cfg(feature = "sdl")]
    let gamepads = if sdl_pads.is_none() { gamepad::Gamepads::new() } else { None };
    #[cfg(not(feature = "sdl"))]
    let gamepads = gamepad::Gamepads::new();
    let event_loop = new_event_loop();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        opts,
        game,
        vfs,
        audio,
        #[cfg(feature = "sdl")]
        sdl_pads,
        gamepads,
        input: Input::default(),
        save_path,
        net: None,
        clock: FixedClock::new(TICK_RATE),
        frame: Frame::new(640, 480),
        ui: Frame::new(640, 480),
        pack: None,
        last: Instant::now(),
        started: Instant::now(),
        auto_shot: false,
        window: None,
        mouse_captured: false,
        presenter: None,
    };
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

/// Texture packs (folders with pack.txt) and mod folders in ./packs, ./mods and the config
/// directory's packs/ and mods/.
fn find_packs_and_mods() -> (Vec<String>, Vec<String>) {
    let roots: Vec<PathBuf> = [Some(PathBuf::from(".")), config::dir()].into_iter().flatten().collect();
    let list = |sub: &str, want_manifest: bool| -> Vec<String> {
        let mut out: Vec<String> = roots
            .iter()
            .filter_map(|r| std::fs::read_dir(r.join(sub)).ok())
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && (!want_manifest || p.join("pack.txt").is_file()))
            .map(|p| p.display().to_string())
            .collect();
        out.sort();
        out
    };
    (list("packs", true), list("mods", false))
}
