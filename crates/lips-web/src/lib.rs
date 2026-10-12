//! WebAssembly front end. The page loads the user's game folder into a [`Loader`], starts a
//! [`WebGame`] and, when the browser offers WebGPU or WebGL2, attaches a [`GpuContext`] made by
//! [`create_gpu`] for the GPU renderer (modern lighting); then each animation frame calls
//! [`WebGame::frame_gpu`]. Without a GPU, [`WebGame::frame`] returns RGBA pixels for a 2D
//! canvas. Input arrives as `KeyboardEvent.code` strings; audio starts on the first gesture
//! ([`WebGame::start_audio`]).

#![cfg(target_arch = "wasm32")]

use godd_game::{FixedClock, Frame, Game, Input, TICK_RATE, Vfs};
use godd_net::{Message, PROTOCOL_VERSION, PlayerState};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Collects the user's game files before the game starts.
#[wasm_bindgen]
#[derive(Default)]
pub struct Loader {
    vfs: Vfs,
    save: Option<Vec<u8>>,
    config: Option<String>,
}

#[wasm_bindgen]
impl Loader {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one file; `path` is relative to the game folder (a leading folder name is stripped).
    pub fn add_file(&mut self, path: &str, data: Vec<u8>) {
        let rel = path.split_once('/').map_or(path, |(_, rest)| rest);
        self.vfs.insert(rel, data);
    }

    /// Character save file from browser storage (overrides the folder's LIPSTART.GAM).
    pub fn set_save(&mut self, data: Vec<u8>) {
        self.save = Some(data);
    }

    /// Options and bindings from browser storage (`set`/`bind` lines, see [`WebGame::take_config`]).
    pub fn set_config(&mut self, text: String) {
        self.config = Some(text);
    }

    /// True once the files needed to boot are present.
    pub fn ready(&self) -> bool {
        self.vfs.contains("lips.bmp") && self.vfs.contains("lipstart.xrb")
    }

    /// Start the game; `level` (e.g. "LIPS00") skips the title screen.
    pub fn start(self, level: Option<String>) -> Result<WebGame, JsError> {
        let mut game = Game::new(&self.vfs, &godd_game::Options { start_level: level, save: self.save }).map_err(|e| JsError::new(&e.to_string()))?;
        if let Some(text) = &self.config {
            for e in game.exec_config(text) {
                web_sys::console::warn_1(&format!("config: {e}").into());
            }
        }
        Ok(WebGame {
            game,
            vfs: self.vfs,
            input: Input::default(),
            clock: FixedClock::new(TICK_RATE),
            frame: Frame::new(640, 480),
            ui: Frame::new(640, 480),
            rgba: Vec::new(),
            net: None,
            gpu: None,
            audio: None,
        })
    }
}

#[wasm_bindgen]
pub struct WebGame {
    game: Game,
    vfs: Vfs,
    input: Input,
    clock: FixedClock,
    frame: Frame,
    /// UI layer over the GPU world view.
    ui: Frame,
    rgba: Vec<u8>,
    net: Option<WebNet>,
    gpu: Option<GpuContext>,
    audio: Option<engine_audio::Audio>,
}

/// The canvas's GPU device (WebGPU, or WebGL2 where WebGPU is missing).
#[wasm_bindgen]
pub struct GpuContext {
    /// Kept alive: on WebGPU the surface depends on it.
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    frames: engine_gpu::FramePresenter,
    world: engine_gpu::WorldRenderer,
    /// Level epoch whose images the world renderer holds.
    loaded: u64,
    backend: String,
}

#[wasm_bindgen]
impl GpuContext {
    /// "WebGpu" or "Gl" (WebGL2).
    pub fn backend(&self) -> String {
        self.backend.clone()
    }
}

/// Set up the GPU on `canvas`. Fails when the browser has neither WebGPU nor WebGL2 (the page
/// then uses a fresh canvas with a 2D context).
#[wasm_bindgen]
pub async fn create_gpu(canvas: web_sys::HtmlCanvasElement, webgl_only: bool) -> Result<GpuContext, JsError> {
    let (width, height) = (canvas.width().max(1), canvas.height().max(1));
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = if webgl_only { wgpu::Backends::GL } else { wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL };
    let instance = wgpu::util::new_instance_with_webgpu_detection(desc).await;
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| JsError::new(&e.to_string()))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
    let backend = format!("{:?}", adapter.get_info().backend);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor { label: Some("lips-web"), required_limits: engine_gpu::limits_for(&adapter), ..Default::default() })
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
    let caps = surface.get_capabilities(&adapter);
    let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width,
        height,
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&device, &config);
    Ok(GpuContext {
        frames: engine_gpu::FramePresenter::new(&device, format),
        world: engine_gpu::WorldRenderer::new(&device, format),
        _instance: instance,
        surface,
        device,
        queue,
        config,
        loaded: u64::MAX,
        backend,
    })
}

/// Netplay over WebSocket (the desktop host runs with `--host --ws-port N`).
struct WebNet {
    socket: web_sys::WebSocket,
    inbox: Rc<RefCell<VecDeque<Vec<u8>>>>,
    since_send: f64,
    tick: u32,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_open: Closure<dyn FnMut()>,
}

#[wasm_bindgen]
impl WebGame {
    /// Join a netplay host, e.g. `ws://192.168.1.10:27961`.
    pub fn connect(&mut self, url: &str, name: &str) -> Result<(), JsError> {
        let socket = web_sys::WebSocket::new(url).map_err(|e| JsError::new(&format!("{e:?}")))?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let inbox: Rc<RefCell<VecDeque<Vec<u8>>>> = Rc::default();
        let queue = inbox.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                queue.borrow_mut().push_back(js_sys::Uint8Array::new(&buf).to_vec());
            }
        });
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        let hello = Message::Hello { version: PROTOCOL_VERSION, name: name.to_string() }.encode();
        let opened = socket.clone();
        let on_open = Closure::<dyn FnMut()>::new(move || {
            let _ = opened.send_with_u8_array(&hello);
        });
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        self.net = Some(WebNet { socket, inbox, since_send: 0.0, tick: 0, _on_message: on_message, _on_open: on_open });
        Ok(())
    }

    fn update_net(&mut self, elapsed_ms: f64) {
        let Some(net) = &mut self.net else { return };
        let messages: Vec<Vec<u8>> = net.inbox.borrow_mut().drain(..).collect();
        for data in messages {
            match Message::decode(&data) {
                Some(Message::Welcome { level, .. }) => {
                    if self.game.session().map(|s| s.name.as_str()) != Some(level.as_str()) {
                        let _ = self.game.start_level(&self.vfs, &level, godd_game::session::Entry::Fresh);
                    }
                }
                Some(Message::State { id, state, .. }) => {
                    if let Some(s) = self.game.session_mut() {
                        s.set_remote_player(id, state.x, state.y, state.z, state.angle, state.region as usize);
                    }
                }
                Some(Message::Left { id }) => {
                    if let Some(s) = self.game.session_mut() {
                        s.remove_remote_player(id);
                    }
                }
                Some(Message::Tag { tag }) => {
                    if let Some(s) = self.game.session_mut() {
                        s.apply_shared_tag(tag);
                    }
                }
                Some(Message::Chat { text, .. }) | Some(Message::Joined { name: text, .. }) => {
                    if let Some(s) = self.game.session_mut() {
                        s.show_message(195, text);
                    }
                }
                _ => {}
            }
        }
        if net.socket.ready_state() != web_sys::WebSocket::OPEN {
            return;
        }
        let Some(session) = self.game.session_mut() else { return };
        for tag in std::mem::take(&mut session.outgoing_tags) {
            let _ = net.socket.send_with_u8_array(&Message::Tag { tag }.encode());
        }
        net.since_send += elapsed_ms;
        if net.since_send >= 50.0 {
            net.since_send = 0.0;
            net.tick = net.tick.wrapping_add(1);
            let p = session.player;
            let state = PlayerState { x: p.x, y: p.y, z: p.z, angle: p.angle, region: p.region as u16, action: 0 };
            let _ = net.socket.send_with_u8_array(&Message::State { id: 0, tick: net.tick, state }.encode());
        }
    }

    /// Use the GPU renderer from now on (see [`create_gpu`]).
    pub fn attach_gpu(&mut self, gpu: GpuContext) {
        self.gpu = Some(gpu);
    }

    /// Start audio. Browsers allow it only from a user gesture: call from a click or key handler.
    pub fn start_audio(&mut self) {
        if self.audio.is_some() {
            return;
        }
        // A SoundFont in the game folder is used for music; otherwise the built-in one.
        let font = self.vfs.names().find(|n| n.to_ascii_lowercase().ends_with(".sf2")).map(str::to_string).and_then(|n| self.vfs.read(&n).ok().map(<[u8]>::to_vec));
        self.audio = engine_audio::Audio::new(font);
    }

    fn advance(&mut self, elapsed_ms: f64) {
        self.update_net(elapsed_ms);
        for _ in 0..self.clock.advance(elapsed_ms / 1000.0) {
            self.game.tick(&self.vfs, &self.input);
            self.input.end_tick();
        }
        let events = self.game.drain_audio();
        if let Some(audio) = &mut self.audio {
            audio.set_volumes(self.game.options.volume);
            audio.spatial = self.game.cvars.get_bool("snd_spatial");
            let vfs = &self.vfs;
            let load = |asset: engine_audio::Asset| {
                let path = match asset {
                    engine_audio::Asset::Sound(n) => godd_game::sound_path(n),
                    engine_audio::Asset::Music(n) => godd_game::music_path(n),
                };
                vfs.read(&path).ok().map(<[u8]>::to_vec)
            };
            for e in &events {
                audio.handle(&load, e);
            }
        }
    }

    /// GPU path: advance by `elapsed_ms` and draw into the canvas (`width`x`height` pixels).
    pub fn frame_gpu(&mut self, elapsed_ms: f64, width: u32, height: u32) {
        self.advance(elapsed_ms);
        let Some(gpu) = &mut self.gpu else { return };
        let (w, h) = (width.max(1), height.max(1));
        if (gpu.config.width, gpu.config.height) != (w, h) {
            gpu.config.width = w;
            gpu.config.height = h;
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
        let output = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                web_sys::console::warn_1(&format!("surface: {other:?}").into());
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
        };
        let target = output.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        self.game.preview_look(&self.input);
        let (uw, uh) = self.game.ui_layer_size(w as usize, h as usize);
        self.ui.resize(uw, uh);
        match self.game.gpu_frame(&mut self.ui, self.clock.alpha()) {
            Some(view) => {
                if gpu.loaded != view.epoch {
                    gpu.world.set_palette(&gpu.device, &gpu.queue, self.game.palette(), self.game.shade_rows());
                    gpu.world.set_images(&gpu.device, &gpu.queue, &self.game.world_images());
                    gpu.loaded = view.epoch;
                }
                for &i in &view.scene.dirty_images {
                    if let Some(img) = self.game.world_image(i) {
                        gpu.world.update_image(&gpu.queue, i, img);
                    }
                }
                let vp = [0.0, 0.0, w as f32, h as f32];
                gpu.world.render(&gpu.device, &gpu.queue, &mut encoder, &target, (w, h), vp, &view.camera, &view.shading, &view.scene);
                if let Some(coverage) = &self.ui.coverage {
                    gpu.world.draw_ui(&gpu.device, &gpu.queue, &mut encoder, &target, vp, (self.ui.width, self.ui.height), &self.ui.pixels, coverage);
                }
            }
            None => {
                // Title screen and classic 320x200: the software renderer, shown on the GPU.
                let classic = self.game.options.classic;
                let (fw, fh) = if classic { (godd_game::CLASSIC_WIDTH, godd_game::CLASSIC_HEIGHT) } else { (w as usize, h as usize) };
                self.frame.resize(fw, fh);
                self.frame.pixel_aspect = if classic { godd_game::CLASSIC_PIXEL_ASPECT } else { 1.0 };
                self.frame.set_true_color(self.game.options.true_color);
                self.game.render(&mut self.frame, self.clock.alpha());
                let cpu = engine_gpu::CpuFrame {
                    width: self.frame.width as u32,
                    height: self.frame.height as u32,
                    indices: &self.frame.pixels,
                    rgb: self.frame.rgb.as_deref(),
                    palette: self.game.palette(),
                };
                let vp = engine_gpu::letterbox(w, h, self.frame.display_aspect());
                gpu.frames.present(&gpu.device, &gpu.queue, &mut encoder, &target, vp, &cpu);
            }
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(output);
    }

    /// Software path: advance by `elapsed_ms` and render at `width`x`height`. Returns RGBA bytes.
    pub fn frame(&mut self, elapsed_ms: f64, width: usize, height: usize) -> Vec<u8> {
        self.advance(elapsed_ms);
        self.frame.resize(width.max(1), height.max(1));
        self.frame.set_true_color(self.game.options.true_color);
        self.game.preview_look(&self.input);
        self.game.render(&mut self.frame, self.clock.alpha());
        self.frame.to_rgba(self.game.palette(), &mut self.rgba);
        std::mem::take(&mut self.rgba)
    }

    /// Feed a key event using `KeyboardEvent.code`. Returns true if the key is bound (so the
    /// page can call `preventDefault`).
    pub fn key(&mut self, code: &str, down: bool) -> bool {
        let source = format!("key:{code}");
        self.input.source(&self.game.binds, &source, down);
        self.game.binds.actions(&source).next().is_some() || self.game.options_menu.is_capturing()
    }

    /// Mouse button (`MouseEvent.button`: 0 left, 1 middle, 2 right, 3 back, 4 forward).
    pub fn mouse_button(&mut self, button: u32, down: bool) {
        let name = match button {
            0 => "Left",
            1 => "Middle",
            2 => "Right",
            3 => "Back",
            4 => "Forward",
            _ => return,
        };
        self.input.source(&self.game.binds, &format!("mouse:{name}"), down);
    }

    /// Mouse wheel (`WheelEvent.deltaY`: negative = up).
    pub fn wheel(&mut self, delta_y: f32) {
        if delta_y != 0.0 {
            self.input.pulse(&self.game.binds, if delta_y < 0.0 { "wheel:Up" } else { "wheel:Down" });
        }
    }

    /// Release every held key and button (the page lost focus).
    pub fn release_all(&mut self) {
        self.input.release_all();
    }

    /// Options and bindings as config text when they changed (the page stores it).
    pub fn take_config(&mut self) -> Option<String> {
        self.game.take_config()
    }

    /// Typed text for name prompts (a single character or "Backspace").
    pub fn type_text(&mut self, key: &str) {
        if key == "Backspace" {
            self.input.text.push('\u{8}');
        } else if key.chars().count() == 1 {
            self.input.text.push_str(key);
        }
    }

    /// Save file bytes when they changed (the page stores them in localStorage).
    pub fn take_save(&mut self) -> Option<Vec<u8>> {
        self.game.take_save()
    }

    /// Mouse-look motion in pixels (the game applies sensitivity and invert).
    pub fn mouse_move(&mut self, dx: f32, dy: f32) {
        self.input.mouse.0 += dx;
        self.input.mouse.1 += dy;
    }
}
