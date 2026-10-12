//! Recorded UI drawing ([`UiCmd`]) at full display resolution: boxes as solid quads, 8x8 bitmap
//! glyphs enlarged with smooth edges (bilinear coverage, anti-aliased threshold) or as crisp
//! pixels, and palette images. Glyphs and images are packed into a per-frame atlas.

use engine_scene::scene::UiCmd;
use std::collections::HashMap;

const ATLAS_WIDTH: u32 = 1024;
/// Glyph cell: the 8x8 bitmap with a one-texel empty border, so filtering fades to nothing.
const CELL: u32 = 10;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    /// UI frame pixels.
    pos: [f32; 2],
    /// Atlas texels.
    uv: [f32; 2],
    color: u32,
    /// 0 box, 1 glyph, 2 image.
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    ui_size: [f32; 2],
    smooth: u32,
    _pad: u32,
}

/// Texels to copy into the atlas once its size is known: position, width, height, texels.
type Blit = ((u32, u32), u32, u32, Vec<u8>);

/// Shelf packing of glyphs and images into the atlas.
struct Packer {
    x: u32,
    y: u32,
    row: u32,
}

impl Packer {
    fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > ATLAS_WIDTH {
            return None;
        }
        if self.x + w > ATLAS_WIDTH {
            (self.x, self.y, self.row) = (0, self.y + self.row, 0);
        }
        let at = (self.x, self.y);
        self.x += w;
        self.row = self.row.max(h);
        Some(at)
    }

    fn height(&self) -> u32 {
        self.y + self.row
    }
}

pub(crate) struct UiList {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    vertices: Option<wgpu::Buffer>,
    atlas: Option<(wgpu::Texture, wgpu::BindGroup, u32)>,
    staging: Vec<u8>,
    verts: Vec<Vertex>,
}

impl UiList {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ui_list.wgsl"));
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui list"),
            entries: &[
                tex(0),
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui list"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui list"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32, 3 => Uint32],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui list params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { pipeline, layout, params, vertices: None, atlas: None, staging: Vec::new(), verts: Vec::new() }
    }

    /// Build quads and the atlas for `cmds` (in a `ui_size` frame); `None` when nothing to draw.
    fn build(&mut self, cmds: &[UiCmd]) -> Option<u32> {
        self.verts.clear();
        let mut packer = Packer { x: 0, y: 0, row: 0 };
        let mut glyphs: HashMap<[u8; 8], (u32, u32)> = HashMap::new();
        let mut blits: Vec<Blit> = Vec::new();
        for cmd in cmds {
            let (x, y, w, h, uv, color, kind) = match cmd {
                UiCmd::Rect { x, y, w, h, color } => (*x, *y, *w, *h, [0.0; 4], *color, 0),
                UiCmd::Glyph { x, y, scale, color, rows } => {
                    let at = match glyphs.get(rows) {
                        Some(&at) => at,
                        None => {
                            let Some(at) = packer.place(CELL, CELL) else { continue };
                            let mut texels = vec![0u8; (CELL * CELL) as usize];
                            for (r, bits) in rows.iter().enumerate() {
                                for c in 0..8 {
                                    if bits & (1 << c) != 0 {
                                        texels[(r + 1) * CELL as usize + c + 1] = 255;
                                    }
                                }
                            }
                            blits.push((at, CELL, CELL, texels));
                            glyphs.insert(*rows, at);
                            at
                        }
                    };
                    let (u, v) = (at.0 as f32 + 1.0, at.1 as f32 + 1.0);
                    (*x, *y, 8 * scale, 8 * scale, [u, v, u + 8.0, v + 8.0], *color, 1)
                }
                UiCmd::Image { x, y, scale, width, height, pixels } => {
                    if pixels.len() < (*width * *height) as usize {
                        continue;
                    }
                    let Some(at) = packer.place(*width, *height) else { continue };
                    blits.push((at, *width, *height, pixels[..(*width * *height) as usize].to_vec()));
                    let (u, v) = (at.0 as f32, at.1 as f32);
                    (*x, *y, width * scale, height * scale, [u, v, u + *width as f32, v + *height as f32], 0, 2)
                }
            };
            let (x0, y0, x1, y1) = (x as f32, y as f32, x as f32 + w as f32, y as f32 + h as f32);
            let [u0, v0, u1, v1] = uv;
            let color = color as u32;
            let corner = |pos: [f32; 2], uv: [f32; 2]| Vertex { pos, uv, color, kind };
            let (a, b, c, d) = (corner([x0, y0], [u0, v0]), corner([x1, y0], [u1, v0]), corner([x1, y1], [u1, v1]), corner([x0, y1], [u0, v1]));
            self.verts.extend_from_slice(&[a, b, c, a, c, d]);
        }
        if self.verts.is_empty() {
            return None;
        }
        let height = packer.height().max(1);
        self.staging.clear();
        self.staging.resize((ATLAS_WIDTH * height) as usize, 0);
        for ((ax, ay), w, h, texels) in blits {
            for row in 0..h {
                let dst = ((ay + row) * ATLAS_WIDTH + ax) as usize;
                self.staging[dst..dst + w as usize].copy_from_slice(&texels[(row * w) as usize..((row + 1) * w) as usize]);
            }
        }
        Some(height)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: [f32; 4],
        ui_size: (usize, usize),
        cmds: &[UiCmd],
        smooth: bool,
        palette: &wgpu::Texture,
    ) {
        let Some(height) = self.build(cmds) else { return };
        if self.atlas.as_ref().is_none_or(|(_, _, h)| *h < height) {
            let h = height.next_power_of_two().max(64);
            let t = crate::texture(device, "ui atlas", ATLAS_WIDTH, h, 1, wgpu::TextureFormat::R8Unorm);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ui list"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.create_view(&Default::default())) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&palette.create_view(&Default::default())) },
                    wgpu::BindGroupEntry { binding: 2, resource: self.params.as_entire_binding() },
                ],
            });
            self.atlas = Some((t, bind, h));
        }
        let (atlas, bind, _) = self.atlas.as_ref().expect("ui atlas");
        crate::write(queue, atlas, &self.staging, (0, 0, 0), (ATLAS_WIDTH, height), 1);
        let params = Params { ui_size: [ui_size.0.max(1) as f32, ui_size.1.max(1) as f32], smooth: smooth as u32, _pad: 0 };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let bytes: &[u8] = bytemuck::cast_slice(&self.verts);
        if self.vertices.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
            self.vertices = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui list vertices"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let vertices = self.vertices.as_ref().expect("ui vertices");
        queue.write_buffer(vertices, 0, bytes);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui list"),
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
        pass.set_vertex_buffer(0, vertices.slice(..bytes.len() as u64));
        pass.draw(0..self.verts.len() as u32, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::Packer;

    #[test]
    fn packer_wraps_rows() {
        let mut p = Packer { x: 0, y: 0, row: 0 };
        assert_eq!(p.place(1000, 10), Some((0, 0)));
        assert_eq!(p.place(30, 5), Some((0, 10)));
        assert_eq!(p.place(2000, 5), None);
        assert_eq!(p.height(), 15);
    }
}
