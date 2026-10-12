// Present a true-colour frame (0x00RRGGBB per texel, uploaded as BGRA bytes).

@group(0) @binding(0) var frame_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(frame_tex));
    let texel = vec2<i32>(clamp(in.uv * size, vec2<f32>(0.0), size - 1.0));
    return vec4<f32>(textureLoad(frame_tex, texel, 0).rgb, 1.0);
}
