//! Modern-lighting post-processing: the HDR world image (multisampled and resolved), a bloom
//! mip chain and the tonemap pass that writes the display image.

use super::{HDR_FORMAT, write_uniform};
use engine_scene::scene::Modern;

const BLOOM_LEVELS: usize = 5;
const BLOOM_THRESHOLD: f32 = 0.9;
const BLOOM_STRENGTH: f32 = 0.35;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniforms {
    params: [f32; 4],
    texel: [f32; 4],
}

struct Targets {
    size: (u32, u32),
    samples: u32,
    ms: Option<wgpu::TextureView>,
    hdr: wgpu::TextureView,
    /// Bloom levels at 1/2, 1/4, ... of the HDR size.
    mips: Vec<(wgpu::TextureView, (u32, u32))>,
    /// Uniform buffer and bind group per pass: bright, downs, ups, tonemap.
    passes: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
}

pub struct Post {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bright: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
    targets: Option<Targets>,
}

impl Post {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("post.wgsl"));
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                tex(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = |entry: &str, format: wgpu::TextureFormat, additive: bool| {
            let blend = additive.then_some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::OVER,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            bright: pipeline("fs_bright", HDR_FORMAT, false),
            down: pipeline("fs_down", HDR_FORMAT, false),
            up: pipeline("fs_up", HDR_FORMAT, true),
            tonemap: pipeline("fs_tonemap", format, false),
            layout,
            sampler,
            targets: None,
        }
    }

    /// (Re)create the HDR image and bloom chain for this size and sample count.
    pub fn prepare(&mut self, device: &wgpu::Device, size: (u32, u32), samples: u32) {
        if self.targets.as_ref().is_some_and(|t| t.size == size && t.samples == samples) {
            return;
        }
        let make = |label: &str, (w, h): (u32, u32), samples: u32| {
            let usage = if samples > 1 {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
            };
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let hdr = make("hdr", size, 1);
        let ms = (samples > 1).then(|| make("hdr-ms", size, samples));
        let mut mips = Vec::new();
        let mut s = size;
        for _ in 0..BLOOM_LEVELS {
            s = ((s.0 / 2).max(1), (s.1 / 2).max(1));
            mips.push((make("bloom", s, 1), s));
        }
        let bind = |src: &wgpu::TextureView, bloom: &wgpu::TextureView| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post-uniforms"),
                size: std::mem::size_of::<PostUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom) },
                ],
            });
            (buffer, group)
        };
        // Bloom passes don't read binding 3: give them a dummy so no texture is both read and written.
        let dummy = make("post-dummy", (1, 1), 1);
        let mut passes = vec![bind(&hdr, &dummy)];
        for i in 1..BLOOM_LEVELS {
            passes.push(bind(&mips[i - 1].0, &dummy));
        }
        for i in (1..BLOOM_LEVELS).rev() {
            passes.push(bind(&mips[i].0, &dummy));
        }
        passes.push(bind(&hdr, &mips[0].0));
        self.targets = Some(Targets { size, samples, ms, hdr, mips, passes });
    }

    /// Colour attachment for the world pass and its resolve target (when multisampled).
    pub fn world_targets(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        let t = self.targets.as_ref().expect("post targets");
        match &t.ms {
            Some(ms) => (ms, Some(&t.hdr)),
            None => (&t.hdr, None),
        }
    }

    /// Bloom and tonemap the HDR image into `target` within `viewport`.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        _target_size: (u32, u32),
        viewport: [f32; 4],
        m: &Modern,
    ) {
        let Some(t) = &self.targets else { return };
        let texel = |(w, h): (u32, u32)| [1.0 / w as f32, 1.0 / h as f32, 0.0, 0.0];
        let pass = |encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup, clear: bool, vp: Option<[f32; 4]>| {
            let load = if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load };
            let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            if let Some([x, y, w, h]) = vp {
                p.set_viewport(x, y, w, h, 0.0, 1.0);
            }
            p.set_pipeline(pipeline);
            p.set_bind_group(0, group, &[]);
            p.draw(0..3, 0..1);
        };
        let params = |p: [f32; 4], tex: [f32; 4]| PostUniforms { params: p, texel: tex };
        let last = t.passes.len() - 1;
        if m.bloom {
            write_uniform(queue, &t.passes[0].0, &params([BLOOM_THRESHOLD, 0.0, 0.0, 0.0], texel(t.size)));
            pass(encoder, &t.mips[0].0, &self.bright, &t.passes[0].1, true, None);
            for i in 1..BLOOM_LEVELS {
                write_uniform(queue, &t.passes[i].0, &params([0.0; 4], texel(t.mips[i - 1].1)));
                pass(encoder, &t.mips[i].0, &self.down, &t.passes[i].1, true, None);
            }
            for (k, i) in (1..BLOOM_LEVELS).rev().enumerate() {
                let p = BLOOM_LEVELS + k;
                write_uniform(queue, &t.passes[p].0, &params([0.0; 4], texel(t.mips[i].1)));
                pass(encoder, &t.mips[i - 1].0, &self.up, &t.passes[p].1, false, None);
            }
        }
        let strength = if m.bloom { BLOOM_STRENGTH } else { 0.0 };
        write_uniform(queue, &t.passes[last].0, &params([0.0, strength, m.exposure, m.tonemap as f32], texel(t.size)));
        pass(encoder, target, &self.tonemap, &t.passes[last].1, false, Some(viewport));
    }
}
