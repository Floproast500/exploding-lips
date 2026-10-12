// UI layer over the 3D view: an indexed frame drawn on the CPU (HUD, menus, weapon), shown
// where its coverage mask is set.

@group(0) @binding(0) var ui_tex: texture_2d<u32>;      // index in r, coverage in g
@group(0) @binding(1) var palette: texture_2d<f32>;

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
    let size = vec2<f32>(textureDimensions(ui_tex));
    let texel = textureLoad(ui_tex, vec2<i32>(clamp(in.uv * size, vec2<f32>(0.0), size - 1.0)), 0);
    if (texel.g == 0u) {
        discard;
    }
    return textureLoad(palette, vec2<i32>(i32(texel.r), 0), 0);
}
