// Present an 8-bit palettised frame: look each index up in a 256x1 palette texture.

@group(0) @binding(0) var frame_tex: texture_2d<u32>;
@group(0) @binding(1) var palette_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the viewport.
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
    let index = textureLoad(frame_tex, texel, 0).r;
    return textureLoad(palette_tex, vec2<i32>(i32(index), 0), 0);
}
