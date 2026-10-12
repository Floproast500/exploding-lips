//! A generic runner for any [`GameModule`] (the Quake-family BSP viewer today): window, GPU
//! presentation through the shared world renderer, keyboard/mouse/pad input mapped to
//! [`ModuleInput`], fixed-rate ticks.

use engine_scene::module::{GameModule, ModuleInput, TICK_RATE};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// Radians per mouse count.
const MOUSE: f32 = 0.0025;

struct ModuleApp {
    module: Box<dyn GameModule>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    keys: HashSet<KeyCode>,
    look: (f32, f32),
    captured: bool,
    clock: godd_game::FixedClock,
    last: Instant,
    ui: godd_game::Frame,
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    world: engine_gpu::WorldRenderer,
    loaded: u64,
}

impl Gpu {
    fn new(window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| e.to_string())?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("module"),
            required_limits: engine_gpu::limits_for(&adapter),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        Ok(Self { world: engine_gpu::WorldRenderer::new(&device, format), surface, device, queue, config, loaded: u64::MAX })
    }
}

impl ModuleApp {
    fn input(&mut self) -> ModuleInput {
        let k = |c: KeyCode| self.keys.contains(&c) as i32 as f32;
        let input = ModuleInput {
            forward: k(KeyCode::KeyW).max(k(KeyCode::ArrowUp)) - k(KeyCode::KeyS).max(k(KeyCode::ArrowDown)),
            strafe: k(KeyCode::KeyD) - k(KeyCode::KeyA),
            up: k(KeyCode::Space) - k(KeyCode::ControlLeft).max(k(KeyCode::KeyC)),
            look: (-self.look.0 * MOUSE + (k(KeyCode::ArrowLeft) - k(KeyCode::ArrowRight)) * 0.05, -self.look.1 * MOUSE),
            run: self.keys.contains(&KeyCode::ShiftLeft),
            use_pressed: false,
        };
        self.look = (0.0, 0.0);
        input
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        for _ in 0..self.clock.advance((now - self.last).as_secs_f64()) {
            let input = self.input();
            self.module.tick(&input);
        }
        self.last = now;
        let Some(gpu) = &mut self.gpu else { return };
        let (w, h) = (gpu.config.width, gpu.config.height);
        self.ui.resize(w as usize, h as usize);
        let Some(view) = self.module.view(&mut self.ui, self.clock.alpha()) else { return };
        if gpu.loaded != view.epoch {
            gpu.world.set_palette(&gpu.device, &gpu.queue, self.module.palette(), self.module.shade_rows());
            gpu.world.set_images(&gpu.device, &gpu.queue, &self.module.images());
            gpu.loaded = view.epoch;
        }
        let output = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
        };
        let target = output.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let vp = [0.0, 0.0, w as f32, h as f32];
        gpu.world.render(&gpu.device, &gpu.queue, &mut encoder, &target, (w, h), vp, &view.camera, &view.shading, &view.scene);
        if let Some(coverage) = &self.ui.coverage {
            gpu.world.draw_ui(&gpu.device, &gpu.queue, &mut encoder, &target, vp, (self.ui.width, self.ui.height), &self.ui.pixels, coverage);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(output);
    }

    fn capture(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        let ok = !on || w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
        if !on {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
        }
        if ok {
            w.set_cursor_visible(!on);
            self.captured = on;
        }
    }
}

impl ApplicationHandler for ModuleApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title(self.module.title()).with_inner_size(PhysicalSize::new(1600, 900));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        match Gpu::new(window.clone()) {
            Ok(g) => self.gpu = Some(g),
            Err(e) => {
                eprintln!("the module viewer needs a GPU: {e}");
                event_loop.exit();
            }
        }
        self.window = Some(window);
        self.last = Instant::now();
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event
            && self.captured {
                self.look.0 += dx as f32;
                self.look.1 += dy as f32;
            }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gpu {
                    g.config.width = size.width.max(1);
                    g.config.height = size.height.max(1);
                    g.surface.configure(&g.device, &g.config);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        if code == KeyCode::Escape {
                            if self.captured { self.capture(false) } else { event_loop.exit() }
                        }
                        self.keys.insert(code);
                    } else {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if !self.captured => self.capture(true),
            WindowEvent::Focused(false) => {
                self.keys.clear();
                self.capture(false);
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

/// Run `module` in its own window until it is closed.
pub fn run(module: Box<dyn GameModule>) {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = ModuleApp {
        module,
        window: None,
        gpu: None,
        keys: HashSet::new(),
        look: (0.0, 0.0),
        captured: false,
        clock: godd_game::FixedClock::new(TICK_RATE),
        last: Instant::now(),
        ui: godd_game::Frame::new(64, 40),
    };
    event_loop.run_app(&mut app).expect("event loop");
}
