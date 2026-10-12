//! Shadows from the strongest dynamic light: a depth cube map of the opaque world geometry
//! around it, compared with hardware filtering (PCF) in the world shader.

/// Cube face resolution.
pub const SIZE: u32 = 1024;
/// Near plane of the light's views (world units): geometry closer than this doesn't cast, so
/// an explosion spawned slightly inside the wall it hit still lights the room.
pub const NEAR: f32 = 6.0;
/// Uniform block per face (dynamic offsets need 256-byte alignment).
const FACE_BLOCK: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct FaceUniform {
    light: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    depth: [f32; 4],
}

/// (right, up, forward) per cube face +X, -X, +Y, -Y, +Z, -Z, so that a lookup with direction
/// v selects the face and texel this view wrote (s from right, t downwards from up).
pub const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]),
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
    ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
];

/// Stored depth = a + b / z for distance z along the face's forward axis (z in near..far
/// maps to 0..1).
pub fn depth_params(far: f32) -> (f32, f32) {
    let far = far.max(NEAR * 2.0);
    (far / (far - NEAR), -far * NEAR / (far - NEAR))
}

pub(crate) struct ShadowMap {
    pipeline: wgpu::RenderPipeline,
    faces: wgpu::Buffer,
    bind: wgpu::BindGroup,
    face_views: Vec<wgpu::TextureView>,
    pub cube: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl ShadowMap {
    pub(crate) fn new(device: &wgpu::Device, vertex_layout: wgpu::VertexBufferLayout) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow cube"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let face_views = (0..6)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let cube = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let faces = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow faces"),
            size: FACE_BLOCK * 6,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<FaceUniform>() as u64),
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &faces, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<FaceUniform>() as u64) }),
            }],
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shadow.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow"), bind_group_layouts: &[Some(&layout)], ..Default::default() });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[Some(vertex_layout)], compilation_options: Default::default() },
            fragment: None,
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                // Slope-scaled bias against acne on surfaces at grazing angles to the light.
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, faces, bind, face_views, cube, sampler }
    }

    /// Render the six faces around `light` (reach `far`) from `vertices` (`count` of them).
    pub(crate) fn render(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, vertices: wgpu::BufferSlice, count: u32, light: [f32; 3], far: f32) {
        let (a, b) = depth_params(far);
        for (i, &(right, up, forward)) in FACES.iter().enumerate() {
            let v = |x: [f32; 3]| [x[0], x[1], x[2], 0.0];
            let face = FaceUniform { light: [light[0], light[1], light[2], 0.0], right: v(right), up: v(up), forward: v(forward), depth: [a, b, 0.0, 0.0] };
            queue.write_buffer(&self.faces, i as u64 * FACE_BLOCK, bytemuck::bytes_of(&face));
        }
        for (i, view) in self.face_views.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[(i as u64 * FACE_BLOCK) as u32]);
            pass.set_vertex_buffer(0, vertices);
            pass.draw(0..count, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A direction selects the face whose forward axis is its major axis, and lands on the
    /// texel that face's projection puts it at (Vulkan cube selection rules).
    #[test]
    fn faces_match_cube_lookup() {
        let dirs = [[1.0, 0.3, -0.2], [-1.0, -0.4, 0.5], [0.2, 1.0, 0.6], [-0.3, -1.0, 0.1], [0.5, -0.2, 1.0], [-0.1, 0.7, -1.0f32]];
        for (k, d) in dirs.iter().enumerate() {
            let (right, up, forward) = FACES[k];
            let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
            let ma = dot(forward);
            assert!(ma > 0.0 && d.iter().all(|c| c.abs() <= ma + 1e-6), "face {k} is the major axis");
            // Rendered texel: s from right, t downward from up.
            let (s, t) = ((dot(right) / ma + 1.0) / 2.0, (1.0 - dot(up) / ma) / 2.0);
            // Vulkan table: (sc, tc) per face.
            let (sc, tc) = match k {
                0 => (-d[2], -d[1]),
                1 => (d[2], -d[1]),
                2 => (d[0], d[2]),
                3 => (d[0], -d[2]),
                4 => (d[0], -d[1]),
                _ => (-d[0], -d[1]),
            };
            let (es, et) = ((sc / ma + 1.0) / 2.0, (tc / ma + 1.0) / 2.0);
            assert!((s - es).abs() < 1e-5 && (t - et).abs() < 1e-5, "face {k}: ({s}, {t}) vs ({es}, {et})");
        }
    }

    #[test]
    fn depth_spans_near_to_far() {
        let (a, b) = depth_params(300.0);
        assert!((a + b / NEAR).abs() < 1e-5);
        assert!((a + b / 300.0 - 1.0).abs() < 1e-5);
    }
}
