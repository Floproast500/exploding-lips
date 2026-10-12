//! Present a CPU-rendered frame (the software renderer, title screens): palette indices looked
//! up on the GPU, or a true-colour frame uploaded as BGRA.

pub struct FramePresenter {
    indexed: wgpu::RenderPipeline,
    indexed_layout: wgpu::BindGroupLayout,
    rgb: wgpu::RenderPipeline,
    rgb_layout: wgpu::BindGroupLayout,
    palette: wgpu::Texture,
    index_tex: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
    rgb_tex: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
}

/// A frame to show: palette indices, or 0x00RRGGBB colours when `rgb` is given.
pub struct CpuFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub indices: &'a [u8],
    pub rgb: Option<&'a [u32]>,
    pub palette: &'a [[u8; 3]; 256],
}

impl FramePresenter {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let tex = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let pipeline = |label, source: wgpu::ShaderModuleDescriptor, layout: &wgpu::BindGroupLayout| {
            let shader = device.create_shader_module(source);
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(layout)], ..Default::default() });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_main"), targets: &[Some(format.into())], compilation_options: Default::default() }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let indexed_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[tex(0, wgpu::TextureSampleType::Uint), tex(1, wgpu::TextureSampleType::Float { filterable: false })],
        });
        let rgb_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame-rgb"),
            entries: &[tex(0, wgpu::TextureSampleType::Float { filterable: false })],
        });
        Self {
            indexed: pipeline("frame", wgpu::include_wgsl!("frame.wgsl"), &indexed_layout),
            rgb: pipeline("frame-rgb", wgpu::include_wgsl!("frame_rgb.wgsl"), &rgb_layout),
            indexed_layout,
            rgb_layout,
            palette: super::texture(device, "frame-palette", 256, 1, 1, wgpu::TextureFormat::Rgba8Unorm),
            index_tex: None,
            rgb_tex: None,
        }
    }

    /// Draw `frame` into `target` within `viewport` (x, y, w, h), clearing the rest to black.
    pub fn present(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, viewport: [f32; 4], frame: &CpuFrame) {
        let (w, h) = (frame.width.max(1), frame.height.max(1));
        let bind = match frame.rgb {
            Some(rgb) => {
                if !matches!(&self.rgb_tex, Some((_, _, tw, th)) if (*tw, *th) == (w, h)) {
                    let t = super::texture(device, "frame-rgb", w, h, 1, wgpu::TextureFormat::Bgra8Unorm);
                    let b = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("frame-rgb"),
                        layout: &self.rgb_layout,
                        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.create_view(&Default::default())) }],
                    });
                    self.rgb_tex = Some((t, b, w, h));
                }
                let (t, b, ..) = self.rgb_tex.as_ref().expect("rgb texture");
                // 0x00RRGGBB little-endian is B, G, R, 0 in memory.
                let bytes: &[u8] = bytemuck::cast_slice(&rgb[..(w * h) as usize]);
                super::write(queue, t, bytes, (0, 0, 0), (w, h), 4);
                (&self.rgb, b)
            }
            None => {
                if !matches!(&self.index_tex, Some((_, _, tw, th)) if (*tw, *th) == (w, h)) {
                    let t = super::texture(device, "frame", w, h, 1, wgpu::TextureFormat::R8Uint);
                    let b = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("frame"),
                        layout: &self.indexed_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.create_view(&Default::default())) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.palette.create_view(&Default::default())) },
                        ],
                    });
                    self.index_tex = Some((t, b, w, h));
                }
                let rgba: Vec<u8> = frame.palette.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect();
                super::write(queue, &self.palette, &rgba, (0, 0, 0), (256, 1), 4);
                let (t, b, ..) = self.index_tex.as_ref().expect("frame texture");
                super::write(queue, t, &frame.indices[..(w * h) as usize], (0, 0, 0), (w, h), 1);
                (&self.indexed, b)
            }
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        let [x, y, vw, vh] = viewport;
        pass.set_viewport(x, y, vw, vh, 0.0, 1.0);
        pass.set_pipeline(bind.0);
        pass.set_bind_group(0, bind.1, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Largest rectangle with the given display aspect ratio centred in a `sw`x`sh` surface.
pub fn letterbox(sw: u32, sh: u32, aspect: f32) -> [f32; 4] {
    let (w, h) = if sw as f32 / sh.max(1) as f32 > aspect { (sh as f32 * aspect, sh as f32) } else { (sw as f32, sw as f32 / aspect) };
    [(sw as f32 - w) / 2.0, (sh as f32 - h) / 2.0, w, h]
}
