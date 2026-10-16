//! Presenters: a wgpu path (software frames through the palette shader, or the GPU world
//! renderer with a UI layer), and a CPU fallback (softbuffer) for systems without a usable GPU
//! surface, e.g. headless test displays.

use std::num::NonZeroU32;
use std::sync::Arc;
use winit::window::Window;

pub enum Presenter {
    Gpu(Box<GpuPresenter>),
    Cpu(CpuPresenter),
}

impl Presenter {
    pub fn new(window: Arc<Window>, vsync: bool, force_cpu: bool) -> Self {
        if !force_cpu {
            match GpuPresenter::new(window.clone(), vsync) {
                Ok(gpu) => return Self::Gpu(Box::new(gpu)),
                Err(e) => log::warn!("GPU presenter unavailable ({e}); using CPU presenter"),
            }
        }
        Self::Cpu(CpuPresenter::new(window))
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        match self {
            Self::Gpu(p) => p.resize(width, height),
            Self::Cpu(p) => p.resize(width, height),
        }
    }

    pub fn surface_size(&self) -> (u32, u32) {
        match self {
            Self::Gpu(p) => (p.config.width, p.config.height),
            Self::Cpu(p) => p.size,
        }
    }

    pub fn present(&mut self, frame: &godd_game::Frame, palette: &[[u8; 3]; 256]) {
        match self {
            Self::Gpu(p) => p.present(frame, palette),
            Self::Cpu(p) => p.present(frame, palette),
        }
    }

    /// Draw a GPU world view with the UI layer on top. False when this presenter cannot (CPU).
    pub fn present_world(&mut self, game: &godd_game::Game, view: &godd_game::GpuView, ui: &godd_game::Frame, pack: Option<&engine_core::pack::LoadedPack>) -> bool {
        match self {
            Self::Gpu(p) => {
                p.present_world(game, view, ui, pack);
                true
            }
            Self::Cpu(_) => false,
        }
    }

    /// GPU time of a recent world frame in milliseconds (devices with timestamp queries).
    pub fn gpu_ms(&mut self) -> Option<f32> {
        match self {
            Self::Gpu(p) => p.world.as_mut().and_then(|(w, _)| w.gpu_ms(&p.device)),
            Self::Cpu(_) => None,
        }
    }

    /// Save the next presented frame to `path` (GPU presentation only).
    pub fn request_screenshot(&mut self, path: std::path::PathBuf) {
        match self {
            Self::Gpu(p) => p.screenshot = Some(path),
            Self::Cpu(_) => log::warn!("screenshots need the GPU presenter"),
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(self, Self::Gpu(_))
    }
}

pub struct CpuPresenter {
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    size: (u32, u32),
}

impl CpuPresenter {
    fn new(window: Arc<Window>) -> Self {
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
        let size = window.inner_size();
        let mut p = Self { surface, size: (0, 0) };
        p.resize(size.width, size.height);
        p
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        let (w, h) = (NonZeroU32::new(self.size.0).unwrap(), NonZeroU32::new(self.size.1).unwrap());
        self.surface.resize(w, h).expect("resize softbuffer");
    }

    fn present(&mut self, frame: &godd_game::Frame, palette: &[[u8; 3]; 256]) {
        let lut: Vec<u32> = palette.iter().map(|&[r, g, b]| (r as u32) << 16 | (g as u32) << 8 | b as u32).collect();
        let (sw, sh) = self.size;
        let [x0, y0, vw, vh] = engine_gpu::letterbox(sw, sh, frame.display_aspect());
        let (x0, y0, vw, vh) = (x0 as usize, y0 as usize, (vw as usize).max(1), (vh as usize).max(1));
        let Ok(mut buffer) = self.surface.buffer_mut() else { return };
        buffer.fill(0);
        let src_x: Vec<usize> = (0..vw).map(|x| x * frame.width / vw).collect();
        for y in 0..vh.min(sh as usize - y0) {
            let row = (y * frame.height / vh) * frame.width;
            let dst = &mut buffer[(y0 + y) * sw as usize + x0..][..vw.min(sw as usize - x0)];
            match &frame.rgb {
                Some(rgb) => {
                    let src = &rgb[row..][..frame.width];
                    for (d, &sx) in dst.iter_mut().zip(&src_x) {
                        *d = src[sx];
                    }
                }
                None => {
                    let src = &frame.pixels[row..][..frame.width];
                    for (d, &sx) in dst.iter_mut().zip(&src_x) {
                        *d = lut[src[sx] as usize];
                    }
                }
            }
        }
        let _ = buffer.present();
    }
}

pub struct GpuPresenter {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    frames: engine_gpu::FramePresenter,
    /// GPU world renderer, plus the level epoch and pack folder whose images it holds.
    world: Option<(engine_gpu::WorldRenderer, (u64, Option<std::path::PathBuf>))>,
    adapter: wgpu::Adapter,
    /// Save the next presented frame here (PNG).
    screenshot: Option<std::path::PathBuf>,
}

impl GpuPresenter {
    fn new(window: Arc<Window>, vsync: bool) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| e.to_string())?;
        log::info!("adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lips"),
            required_features: engine_gpu::features_for(&adapter),
            required_limits: engine_gpu::limits_for(&adapter),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let present_mode = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        let config = wgpu::SurfaceConfiguration {
            // Copyable when the surface allows it, for screenshots of exactly what is shown.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC),
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        log::info!("surface {:?} {}x{}", config.format, config.width, config.height);
        Ok(Self { frames: engine_gpu::FramePresenter::new(&device, format), surface, device, queue, config, world: None, adapter, screenshot: None })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                None
            }
            other => {
                log::warn!("surface unavailable: {other:?}");
                None
            }
        }
    }

    fn present(&mut self, frame: &godd_game::Frame, palette: &[[u8; 3]; 256]) {
        let Some(output) = self.acquire() else { return };
        let target = output.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let cpu = engine_gpu::CpuFrame { width: frame.width as u32, height: frame.height as u32, indices: &frame.pixels, rgb: frame.rgb.as_deref(), palette };
        let vp = engine_gpu::letterbox(self.config.width, self.config.height, frame.display_aspect());
        self.frames.present(&self.device, &self.queue, &mut encoder, &target, vp, &cpu);
        let grab = self.grab(&mut encoder, &output.texture);
        self.queue.submit([encoder.finish()]);
        self.save_grab(grab);
        self.queue.present(output);
    }

    fn present_world(&mut self, game: &godd_game::Game, view: &godd_game::GpuView, ui: &godd_game::Frame, pack: Option<&engine_core::pack::LoadedPack>) {
        let (device, queue) = (&self.device, &self.queue);
        let adapter = &self.adapter;
        let (world, loaded) = self.world.get_or_insert_with(|| {
            let mut w = engine_gpu::WorldRenderer::new(device, self.config.format);
            w.use_adapter(device, queue, adapter);
            (w, (u64::MAX, None))
        });
        let wanted = (view.epoch, pack.map(|p| p.dir.clone()));
        if *loaded != wanted {
            world.set_palette(device, queue, game.palette(), game.shade_rows());
            world.set_images(device, queue, &game.world_images());
            if let Some(pack) = pack {
                let started = std::time::Instant::now();
                let textures = game.hd_textures(pack);
                let found = textures.iter().filter(|t| t.is_some()).count();
                world.set_hd(device, queue, &textures);
                log::info!("texture pack {}: {found} of {} images in {:.1} s", pack.dir.display(), textures.len(), started.elapsed().as_secs_f32());
            }
            *loaded = wanted;
        }
        for &i in &view.scene.dirty_images {
            if let Some(image) = game.world_image(i) {
                world.update_image(queue, i, image);
            }
        }
        let Some(output) = self.acquire() else { return };
        let target = output.texture.create_view(&Default::default());
        let size = (self.config.width, self.config.height);
        let vp = [0.0, 0.0, size.0 as f32, size.1 as f32];
        let (device, queue) = (&self.device, &self.queue);
        let (world, _) = self.world.as_mut().expect("world renderer");
        let mut encoder = device.create_command_encoder(&Default::default());
        world.render(device, queue, &mut encoder, &target, size, vp, &view.camera, &view.shading, &view.scene);
        if let Some(coverage) = &ui.coverage {
            let list = ui.ui_list.as_deref().unwrap_or_default();
            world.draw_ui(device, queue, &mut encoder, &target, vp, (ui.width, ui.height), &ui.pixels, coverage, list, game.cvars.get_bool("hud_smooth"));
        }
        if let Some(path) = &self.screenshot {
            let c = &view.camera;
            log::info!("screenshot {} camera {} {} {} angle {} pitch {}", path.display(), c.pos[0], c.pos[1], c.pos[2], c.angle, c.pitch);
        }
        let grab = self.grab(&mut encoder, &output.texture);
        self.queue.submit([encoder.finish()]);
        if let Some((world, _)) = &mut self.world {
            world.submitted();
        }
        self.save_grab(grab);
        self.queue.present(output);
    }

    /// When a screenshot is requested: copy the frame into a readback buffer.
    fn grab(&mut self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> Option<(std::path::PathBuf, wgpu::Buffer, u32)> {
        self.screenshot.as_ref()?;
        if !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
            log::warn!("screenshots are not supported by this display surface");
            self.screenshot = None;
            return None;
        }
        let path = self.screenshot.take()?;
        let (w, h) = (self.config.width, self.config.height);
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        Some((path, buffer, row))
    }

    /// Wait for a grabbed frame and write it as a PNG.
    fn save_grab(&self, grab: Option<(std::path::PathBuf, wgpu::Buffer, u32)>) {
        let Some((path, buffer, row)) = grab else { return };
        let (w, h) = (self.config.width as usize, self.config.height as usize);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let Ok(data) = buffer.slice(..).get_mapped_range() else { return };
        let format = self.config.format;
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for px in data[y * row as usize..][..w * 4].as_chunks::<4>().0 {
                pixels.extend_from_slice(&match format {
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => [px[2], px[1], px[0], 255],
                    // 10 bits per channel (HDR-capable compositors): keep the top 8.
                    wgpu::TextureFormat::Rgb10a2Unorm => {
                        let v = u32::from_le_bytes(*px);
                        [(v >> 2) as u8, (v >> 12) as u8, (v >> 22) as u8, 255]
                    }
                    _ => [px[0], px[1], px[2], 255],
                });
            }
        }
        let image = engine_core::pack::Rgba { width: w, height: h, pixels };
        let written = engine_core::pack::encode_png(&image).map_err(|e| e.to_string()).and_then(|png| {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, png).map_err(|e| e.to_string())
        });
        match written {
            Ok(()) => log::info!("screenshot {} ({w}x{h})", path.display()),
            Err(e) => log::warn!("screenshot {}: {e}", path.display()),
        }
    }
}
