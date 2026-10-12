// Recorded UI drawing at display resolution: boxes, 8x8 glyphs (smooth or crisp), images.

struct Params {
    ui_size: vec2<f32>,
    smooth_text: u32,
    _pad: u32,
};

@group(0) @binding(0) var atlas: texture_2d<f32>;   // glyph coverage / image index / 255
@group(0) @binding(1) var palette: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: u32,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: u32, @location(3) kind: u32) -> VsOut {
    var out: VsOut;
    let p = pos / params.ui_size;
    out.pos = vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    out.kind = kind;
    return out;
}

fn texel(p: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(atlas));
    return textureLoad(atlas, clamp(vec2<i32>(floor(p)), vec2<i32>(0), size - 1), 0).r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Glyph coverage, bilinear between texel centres: thresholding it at 0.5 rounds the
    // bitmap's corners and diagonals, and the screen-space width anti-aliases the edge.
    // Computed for every fragment (derivatives need uniform control flow).
    let q = in.uv - 0.5;
    let f = fract(q);
    let i = floor(q) + 0.5;
    let top = mix(texel(i), texel(i + vec2<f32>(1.0, 0.0)), f.x);
    let bottom = mix(texel(i + vec2<f32>(0.0, 1.0)), texel(i + vec2<f32>(1.0, 1.0)), f.x);
    let coverage = mix(top, bottom, f.y);
    let edge = max(fwidth(coverage), 1e-3);
    // Lines: pixels from the quad's long edge, from the across coordinate's rate of change.
    let edge_px = (1.0 - abs(in.uv.y)) / max(fwidth(in.uv.y), 1e-4);
    var alpha = 1.0;
    var index = in.color;
    if (in.kind == 1u) {
        if (params.smooth_text != 0u) {
            alpha = smoothstep(0.5 - edge, 0.5 + edge, coverage);
        } else {
            alpha = step(0.5, texel(in.uv));
        }
    } else if (in.kind == 3u) {
        // Opaque core, fading over the last pixel.
        alpha = clamp(edge_px, 0.0, 1.0);
    } else if (in.kind == 2u) {
        index = u32(round(texel(in.uv) * 255.0));
        alpha = select(1.0, 0.0, index == 0u);
    }
    if (alpha <= 0.0) {
        discard;
    }
    let c = textureLoad(palette, vec2<i32>(i32(index), 0), 0);
    return vec4<f32>(c.rgb, alpha);
}
