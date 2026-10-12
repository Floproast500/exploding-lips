//! GPU renderer for engine scenes (wgpu). The world pass draws [`SceneFrame`] triangles with
//! palette-indexed textures packed into an atlas array, reproducing the classic light rules
//! (shade tables) or continuous true-colour light; the UI pass composites a CPU-drawn indexed
//! layer by its coverage mask. Works with WebGL2-level limits (no storage buffers).

use engine_scene::scene::{IndexedImage, SceneCamera, SceneFrame, Shading};

/// Atlas layer size (the WebGL2 minimum for 2D textures).
const LAYER: u32 = 2048;
const NEAR: f32 = 4.0;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// One clip window: edge count and up to 15 edge equations (matches `Clip` in world.wgsl).
const CLIP_BLOCK: u64 = 256;
const MAX_CLIP_EDGES: usize = 15;

/// Edge equations (a, b, c: inside when a·x + b·y + c >= 0) of a counter-clockwise polygon,
/// normalized and loosened by `slack` (NDC units) so neighbouring windows share their edges.
fn clip_block(poly: Option<&Vec<[f32; 2]>>, slack: f32) -> [u8; CLIP_BLOCK as usize] {
    let mut block = [0u8; CLIP_BLOCK as usize];
    let Some(poly) = poly.filter(|p| p.len() >= 3 && p.len() <= MAX_CLIP_EDGES) else { return block };
    let mut words = [0f32; 64];
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (a, b) = (-(q[1] - p[1]), q[0] - p[0]);
        let len = (a * a + b * b).sqrt().max(1e-12);
        let (a, b) = (a / len, b / len);
        words[4 + i * 4..4 + i * 4 + 3].copy_from_slice(&[a, b, -(a * p[0] + b * p[1]) + slack]);
    }
    block.copy_from_slice(bytemuck::cast_slice(&words));
    block[..4].copy_from_slice(&(poly.len() as u32).to_le_bytes());
    block
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    rect: [u16; 4],
    layer_flags: [u32; 2],
    light: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    cam: [f32; 4],
    basis: [f32; 4],
    proj: [f32; 4],
    viewport: [f32; 4],
    shading: [f32; 4],
    shading2: [f32; 4],
    sky_rect: [u32; 4],
    sky_layer: [u32; 4],
    look: [f32; 4],
}

/// Where an image lives in the atlas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Slot {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    layer: u32,
}

/// Shelf-pack images into `LAYER`-sized layers, tallest first.
fn pack(sizes: &[(usize, usize)]) -> (Vec<Slot>, u32) {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes[i].1));
    let mut slots = vec![Slot::default(); sizes.len()];
    let (mut layer, mut x, mut y, mut row_h) = (0u32, 0u32, 0u32, 0u32);
    for i in order {
        let (w, h) = (sizes[i].0.clamp(1, LAYER as usize) as u32, sizes[i].1.clamp(1, LAYER as usize) as u32);
        if x + w > LAYER {
            x = 0;
            y += row_h;
            row_h = 0;
        }
        if y + h > LAYER {
            layer += 1;
            x = 0;
            y = 0;
            row_h = 0;
        }
        slots[i] = Slot { x: x as u16, y: y as u16, w: w as u16, h: h as u16, layer };
        x += w;
        row_h = row_h.max(h);
    }
    (slots, layer + 1)
}

pub struct WorldRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    clip_layout: wgpu::BindGroupLayout,
    /// Per-batch clip windows (256-byte blocks bound with a dynamic offset).
    clips: Option<(wgpu::Buffer, wgpu::BindGroup, u64)>,
    uniforms: wgpu::Buffer,
    palette: wgpu::Texture,
    colormap: Option<(wgpu::Texture, u32)>,
    atlas: Option<wgpu::Texture>,
    slots: Vec<Slot>,
    bind_group: Option<wgpu::BindGroup>,
    depth: Option<(wgpu::TextureView, u32, u32)>,
    vertices: Option<(wgpu::Buffer, u64)>,
    staging: Vec<GpuVertex>,
    ui: UiOverlay,
}

impl WorldRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("world.wgsl"));
        let tex = |binding, sample_type, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex(1, wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2Array),
                tex(2, wgpu::TextureSampleType::Float { filterable: false }, wgpu::TextureViewDimension::D2),
                tex(3, wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2),
            ],
        });
        let clip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world-clip"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(CLIP_BLOCK),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world"),
            bind_group_layouts: &[Some(&layout), Some(&clip_layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Uint16x4, 3 => Uint32x2, 4 => Float32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette = texture(device, "palette", 256, 1, 1, wgpu::TextureFormat::Rgba8Unorm);
        Self {
            pipeline,
            layout,
            clip_layout,
            clips: None,
            uniforms,
            ui: UiOverlay::new(device, format, &palette),
            palette,
            colormap: None,
            atlas: None,
            slots: Vec::new(),
            bind_group: None,
            depth: None,
            vertices: None,
            staging: Vec::new(),
        }
    }

    /// Palette (RGB) and shade tables (`rows[level][index]`, level 0 brightest).
    pub fn set_palette(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, palette: &[[u8; 3]; 256], rows: &[[u8; 256]]) {
        let rgba: Vec<u8> = palette.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect();
        write(queue, &self.palette, &rgba, (0, 0, 0), (256, 1), 4);
        let levels = rows.len().max(1) as u32;
        if self.colormap.as_ref().is_none_or(|(_, n)| *n != levels) {
            self.colormap = Some((texture(device, "colormap", 256, levels, 1, wgpu::TextureFormat::R8Uint), levels));
            self.bind_group = None;
        }
        let data: Vec<u8> = if rows.is_empty() { (0..=255).collect() } else { rows.iter().flatten().copied().collect() };
        write(queue, &self.colormap.as_ref().expect("colormap").0, &data, (0, 0, 0), (256, levels), 1);
    }

    /// Upload all images (a new level). Vertices refer to them by index.
    pub fn set_images(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, images: &[IndexedImage]) {
        let (slots, layers) = pack(&images.iter().map(|i| (i.width, i.height)).collect::<Vec<_>>());
        let atlas = texture(device, "atlas", LAYER, LAYER, layers, wgpu::TextureFormat::R8Uint);
        self.atlas = Some(atlas);
        self.slots = slots;
        self.bind_group = None;
        for (i, &image) in images.iter().enumerate() {
            self.update_image(queue, i as u32, image);
        }
    }

    /// Re-upload one image whose pixels changed (same size as before).
    pub fn update_image(&mut self, queue: &wgpu::Queue, index: u32, image: IndexedImage) {
        let (Some(atlas), Some(slot)) = (&self.atlas, self.slots.get(index as usize)) else { return };
        let (w, h) = (slot.w as usize, slot.h as usize);
        if image.width == 0 || image.height == 0 || image.pixels.len() < image.width * image.height {
            return;
        }
        let mut data = vec![0u8; w * h];
        for y in 0..h.min(image.height) {
            let n = w.min(image.width);
            data[y * w..y * w + n].copy_from_slice(&image.pixels[y * image.width..y * image.width + n]);
        }
        write(queue, atlas, &data, (slot.x as u32, slot.y as u32, slot.layer), (w as u32, h as u32), 1);
    }

    /// Draw the scene into `target` (size `target_size`) within `viewport` (x, y, w, h).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_size: (u32, u32),
        viewport: [f32; 4],
        camera: &SceneCamera,
        shading: &Shading,
        scene: &SceneFrame,
    ) {
        let (Some(atlas), Some((colormap, levels))) = (&self.atlas, &self.colormap) else { return };
        if self.bind_group.is_none() {
            let view = |t: &wgpu::Texture| t.create_view(&Default::default());
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("world"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&atlas.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            ..Default::default()
                        })),
                    },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view(&self.palette)) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&view(colormap)) },
                ],
            }));
        }
        if self.depth.as_ref().is_none_or(|(_, w, h)| (*w, *h) != target_size) {
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width: target_size.0, height: target_size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth = Some((depth.create_view(&Default::default()), target_size.0, target_size.1));
        }
        // Uniforms: the software renderer's projection (focal from the vertical FOV, Hor+).
        let [_, _, vw, vh] = viewport;
        let focal = vh / 2.0 / (camera.vfov_degrees.to_radians() / 2.0).tan();
        let (sin, cos) = camera.angle.sin_cos();
        let sky = scene.sky.and_then(|i| self.slots.get(i as usize)).copied();
        let uniforms = Uniforms {
            cam: [camera.pos[0], camera.pos[1], camera.pos[2], NEAR],
            basis: [cos, sin, sin, -cos],
            proj: [focal / (vw / 2.0), focal / (vh / 2.0), camera.pitch, focal],
            viewport,
            shading: [shading.visibility, shading.fade_bias, shading.true_color as u32 as f32, shading.fullbright as u32 as f32],
            shading2: [shading.max_object_shade, shading.zero_substitute as f32, *levels as f32, sky.is_some() as u32 as f32],
            sky_rect: sky.map_or([0, 0, 1, 1], |s| [s.x as u32, s.y as u32, s.w as u32, s.h as u32]),
            sky_layer: [sky.map_or(0, |s| s.layer), 0, 0, 0],
            look: {
                let a = camera.pitch.atan();
                [camera.freelook as u32 as f32, a.sin(), a.cos(), 0.0]
            },
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        // Vertices with their images' atlas slots.
        self.staging.clear();
        self.staging.extend(scene.vertices.iter().map(|v| {
            let s = self.slots.get(v.image as usize).copied().unwrap_or(Slot { w: 1, h: 1, ..Default::default() });
            GpuVertex { pos: v.pos, uv: v.uv, rect: [s.x, s.y, s.w, s.h], layer_flags: [s.layer, v.flags], light: v.light }
        }));
        let bytes = bytemuck::cast_slice::<GpuVertex, u8>(&self.staging);
        if self.vertices.as_ref().is_none_or(|(_, cap)| *cap < bytes.len() as u64) {
            let cap = (bytes.len() as u64).next_power_of_two().max(1 << 16);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("world-vertices"),
                size: cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vertices = Some((buffer, cap));
        }
        let (buffer, _) = self.vertices.as_ref().expect("vertex buffer");
        if !bytes.is_empty() {
            queue.write_buffer(buffer, 0, bytes);
        }
        // Clip windows, one block per batch (block 0 doubles as "no clip" when there are none).
        let blocks = scene.batches.len().max(1) as u64;
        if self.clips.as_ref().is_none_or(|(_, _, cap)| *cap < blocks) {
            let cap = blocks.next_power_of_two().max(64);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("world-clips"),
                size: cap * CLIP_BLOCK,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("world-clips"),
                layout: &self.clip_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset: 0, size: wgpu::BufferSize::new(CLIP_BLOCK) }),
                }],
            });
            self.clips = Some((buffer, bind, cap));
        }
        let slack = 1.0 / viewport[3].max(1.0); // half a pixel in NDC
        let data: Vec<u8> = if scene.batches.is_empty() {
            clip_block(None, slack).to_vec()
        } else {
            scene.batches.iter().flat_map(|b| clip_block(b.clip.as_ref(), slack)).collect()
        };
        queue.write_buffer(&self.clips.as_ref().expect("clips").0, 0, &data);
        let (depth, ..) = self.depth.as_ref().expect("depth");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let [x, y, w, h] = viewport;
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, self.bind_group.as_ref(), &[]);
        pass.set_vertex_buffer(0, buffer.slice(..bytes.len().max(1) as u64));
        let clip_bind = self.clips.as_ref().map(|(_, b, _)| b).expect("clip buffer");
        if scene.batches.is_empty() {
            pass.set_bind_group(1, clip_bind, &[0]);
            pass.draw(0..self.staging.len() as u32, 0..1);
        }
        let (tw, th) = target_size;
        for (i, b) in scene.batches.iter().enumerate() {
            // Scissor to the window's bounding box; the shader clips to the polygon.
            let (mut sx0, mut sy0, mut sx1, mut sy1) = (x, y, x + w, y + h);
            if let Some(poly) = &b.clip {
                let px = |p: &[f32; 2]| (x + (p[0] + 1.0) / 2.0 * w, y + (1.0 - p[1]) / 2.0 * h);
                let pts: Vec<(f32, f32)> = poly.iter().map(px).collect();
                sx0 = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).max(x);
                sx1 = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).min(x + w);
                sy0 = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).max(y);
                sy1 = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).min(y + h);
            }
            let (ax, ay) = ((sx0.floor().max(0.0) as u32).min(tw), (sy0.floor().max(0.0) as u32).min(th));
            let (zx, zy) = ((sx1.ceil().max(0.0) as u32).min(tw), (sy1.ceil().max(0.0) as u32).min(th));
            if zx <= ax || zy <= ay {
                continue;
            }
            pass.set_scissor_rect(ax, ay, zx - ax, zy - ay);
            pass.set_bind_group(1, clip_bind, &[(i as u64 * CLIP_BLOCK) as u32]);
            pass.draw(b.start..b.end.min(self.staging.len() as u32), 0..1);
        }
    }

    /// Composite a CPU-drawn indexed UI layer (`indices`, `coverage`) over `target`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_ui(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: [f32; 4],
        size: (usize, usize),
        indices: &[u8],
        coverage: &[u8],
    ) {
        self.ui.draw(device, queue, encoder, target, viewport, size, indices, coverage, &self.palette);
    }
}

struct UiOverlay {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    tex: Option<(wgpu::Texture, wgpu::BindGroup, usize, usize)>,
    staging: Vec<u8>,
}

impl UiOverlay {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, _palette: &wgpu::Texture) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ui.wgsl"));
        let tex = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui"),
            entries: &[tex(0, wgpu::TextureSampleType::Uint), tex(1, wgpu::TextureSampleType::Float { filterable: false })],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
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
        Self { pipeline, layout, tex: None, staging: Vec::new() }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: [f32; 4],
        (w, h): (usize, usize),
        indices: &[u8],
        coverage: &[u8],
        palette: &wgpu::Texture,
    ) {
        if w == 0 || h == 0 || indices.len() < w * h || coverage.len() < w * h {
            return;
        }
        if self.tex.as_ref().is_none_or(|(_, _, tw, th)| (*tw, *th) != (w, h)) {
            let t = texture(device, "ui", w as u32, h as u32, 1, wgpu::TextureFormat::Rg8Uint);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ui"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.create_view(&Default::default())) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&palette.create_view(&Default::default())) },
                ],
            });
            self.tex = Some((t, bind, w, h));
        }
        self.staging.clear();
        self.staging.extend(indices[..w * h].iter().zip(&coverage[..w * h]).flat_map(|(&i, &c)| [i, c]));
        let (t, bind, ..) = self.tex.as_ref().expect("ui texture");
        write(queue, t, &self.staging, (0, 0, 0), (w as u32, h as u32), 2);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        let [x, y, vw, vh] = viewport;
        pass.set_viewport(x, y, vw, vh, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn texture(device: &wgpu::Device, label: &str, width: u32, height: u32, layers: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &[u8], (x, y, z): (u32, u32, u32), (w, h): (u32, u32), bpp: u32) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z }, aspect: wgpu::TextureAspect::All },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

/// A device without a window, for tools and tests. `None` when no adapter is available.
pub fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("headless"),
        required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .ok()
}

/// Render a scene off screen and read it back as RGBA8 (`width * height * 4` bytes).
#[allow(clippy::too_many_arguments)]
pub fn render_offscreen(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut WorldRenderer,
    (width, height): (u32, u32),
    camera: &SceneCamera,
    shading: &Shading,
    scene: &SceneFrame,
    ui: Option<(&[u8], &[u8])>,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    let vp = [0.0, 0.0, width as f32, height as f32];
    renderer.render(device, queue, &mut encoder, &view, (width, height), vp, camera, shading, scene);
    if let Some((indices, coverage)) = ui {
        renderer.draw_ui(device, queue, &mut encoder, &view, vp, (width as usize, height as usize), indices, coverage);
    }
    let row = (width * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) } },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let Ok(data) = slice.get_mapped_range() else { return vec![0; (width * height * 4) as usize] };
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&data[y * row as usize..][..width as usize * 4]);
    }
    drop(data);
    readback.unmap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_fills_layers_without_overlap() {
        let sizes: Vec<(usize, usize)> = (0..900).map(|i| (32 + (i * 37) % 300, 16 + (i * 53) % 260)).collect();
        let (slots, layers) = pack(&sizes);
        assert!(layers >= 2);
        for (i, a) in slots.iter().enumerate() {
            assert!(a.x as u32 + a.w as u32 <= LAYER && a.y as u32 + a.h as u32 <= LAYER);
            assert_eq!((a.w as usize, a.h as usize), sizes[i]);
            for b in &slots[i + 1..] {
                let apart = a.layer != b.layer || a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }
}
