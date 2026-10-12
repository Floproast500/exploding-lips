//! Presenters: a wgpu path that does the palette lookup on the GPU, and a CPU fallback
//! (softbuffer) for systems without a usable GPU surface, e.g. headless test displays.

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
            Self::Gpu(p) => p.surface_size(),
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
    pub fn present_world(&mut self, game: &godd_game::Game, view: &godd_game::GpuView, ui: &godd_game::Frame) -> bool {
        match self {
            Self::Gpu(p) => {
                p.present_world(game, view, ui);
                true
            }
            Self::Cpu(_) => false,
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
        let (x0, y0, vw, vh) = letterbox(sw, sh, frame.display_aspect());
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
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    palette_tex: wgpu::Texture,
    frame_tex: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
    rgb_pipeline: wgpu::RenderPipeline,
    rgb_layout: wgpu::BindGroupLayout,
    rgb_tex: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
    /// GPU world renderer and the level epoch whose images it holds.
    world: Option<(engine_gpu::WorldRenderer, u64)>,
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
            required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let present_mode = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("present.wgsl"));
        let texture_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Uint),
                texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let rgb_shader = device.create_shader_module(wgpu::include_wgsl!("present_rgb.wgsl"));
        let rgb_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present-rgb"),
            entries: &[texture_entry(0, wgpu::TextureSampleType::Float { filterable: false })],
        });
        let rgb_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present-rgb"),
            bind_group_layouts: &[Some(&rgb_layout)],
            ..Default::default()
        });
        let rgb_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present-rgb"),
            layout: Some(&rgb_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rgb_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &rgb_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let palette_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("palette"),
            size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            layout,
            palette_tex,
            frame_tex: None,
            rgb_pipeline,
            rgb_layout,
            rgb_tex: None,
            world: None,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn present(&mut self, frame: &godd_game::Frame, palette: &[[u8; 3]; 256]) {
        let (w, h) = (frame.width as u32, frame.height as u32);
        let true_color = frame.rgb.is_some();
        if let Some(rgb) = &frame.rgb {
            if !matches!(&self.rgb_tex, Some((_, _, fw, fh)) if (*fw, *fh) == (w, h)) {
                self.rgb_tex = Some(self.create_rgb_texture(w, h));
            }
            let (tex, ..) = self.rgb_tex.as_ref().expect("rgb texture");
            // 0x00RRGGBB little-endian is B,G,R,0 in memory: upload as Bgra8Unorm.
            write_texture(&self.queue, tex, bytemuck::cast_slice(rgb), w, h, 4);
        } else {
            if !matches!(&self.frame_tex, Some((_, _, fw, fh)) if (*fw, *fh) == (w, h)) {
                self.frame_tex = Some(self.create_frame_texture(w, h));
            }
            let rgba: Vec<u8> = palette.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect();
            write_texture(&self.queue, &self.palette_tex, &rgba, 256, 1, 4);
            let (frame_tex, ..) = self.frame_tex.as_ref().expect("frame texture");
            write_texture(&self.queue, frame_tex, &frame.pixels, w, h, 1);
        }
        let (pipeline, bind_group) = if true_color {
            (&self.rgb_pipeline, &self.rgb_tex.as_ref().expect("rgb texture").1)
        } else {
            (&self.pipeline, &self.frame_tex.as_ref().expect("frame texture").1)
        };

        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            other => {
                log::warn!("surface unavailable: {other:?}");
                return;
            }
        };
        let view = output.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            let (x, y, vw, vh) = letterbox(self.config.width, self.config.height, frame.display_aspect());
            pass.set_viewport(x, y, vw, vh, 0.0, 1.0);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
    }

    fn present_world(&mut self, game: &godd_game::Game, view: &godd_game::GpuView, ui: &godd_game::Frame) {
        let (device, queue) = (&self.device, &self.queue);
        let (world, epoch) = self.world.get_or_insert_with(|| (engine_gpu::WorldRenderer::new(device, self.config.format), u64::MAX));
        if *epoch != view.epoch {
            world.set_palette(device, queue, game.palette(), game.shade_rows());
            world.set_images(device, queue, &game.world_images());
            *epoch = view.epoch;
        }
        for &i in &view.scene.dirty_images {
            if let Some(image) = game.world_image(i) {
                world.update_image(queue, i, image);
            }
        }
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            other => {
                log::warn!("surface unavailable: {other:?}");
                return;
            }
        };
        let target = output.texture.create_view(&Default::default());
        let size = (self.config.width, self.config.height);
        let (x, y, w, h) = letterbox(size.0, size.1, ui.display_aspect());
        let vp = [x, y, w, h];
        let mut encoder = self.device.create_command_encoder(&Default::default());
        world.render(&self.device, &self.queue, &mut encoder, &target, size, vp, &view.camera, &view.shading, &view.scene);
        if let Some(coverage) = &ui.coverage {
            world.draw_ui(&self.device, &self.queue, &mut encoder, &target, vp, (ui.width, ui.height), &ui.pixels, coverage);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
    }

    fn create_rgb_texture(&self, width: u32, height: u32) -> (wgpu::Texture, wgpu::BindGroup, u32, u32) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame-rgb"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present-rgb"),
            layout: &self.rgb_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())),
            }],
        });
        (texture, bind_group, width, height)
    }

    fn create_frame_texture(&self, width: u32, height: u32) -> (wgpu::Texture, wgpu::BindGroup, u32, u32) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.palette_tex.create_view(&Default::default())),
                },
            ],
        });
        (texture, bind_group, width, height)
    }
}

fn write_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &[u8], width: u32, height: u32, bpp: u32) {
    queue.write_texture(
        texture.as_image_copy(),
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * bpp), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
}

/// Largest rectangle with the given display aspect ratio centred in the surface.
fn letterbox(sw: u32, sh: u32, aspect: f32) -> (f32, f32, f32, f32) {
    let (w, h) = if sw as f32 / sh as f32 > aspect { (sh as f32 * aspect, sh as f32) } else { (sw as f32, sw as f32 / aspect) };
    ((sw as f32 - w) / 2.0, (sh as f32 - h) / 2.0, w, h)
}
