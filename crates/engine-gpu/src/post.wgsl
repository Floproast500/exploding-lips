// Post-processing for modern lighting: bloom (bright-pass downsample chain, additive tent
// upsample) and tonemapping of the HDR image to the display, encoding sRGB in the shader
// because presentation surfaces here are not sRGB formats.

struct Post {
    params: vec4<f32>,   // threshold, bloom strength, exposure, tonemap (0 clamp, 1 soft, 2 ACES)
    texel: vec4<f32>,    // 1 / source size (xy)
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> post: Post;
@group(0) @binding(3) var bloom: texture_2d<f32>;

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

fn box4(uv: vec2<f32>) -> vec3<f32> {
    let d = post.texel.xy;
    return 0.25 * (textureSample(src, smp, uv + vec2<f32>(-d.x, -d.y)).rgb + textureSample(src, smp, uv + vec2<f32>(d.x, -d.y)).rgb
        + textureSample(src, smp, uv + vec2<f32>(-d.x, d.y)).rgb + textureSample(src, smp, uv + vec2<f32>(d.x, d.y)).rgb);
}

// First downsample: keep only what is brighter than the threshold (soft knee).
@fragment
fn fs_bright(in: VsOut) -> @location(0) vec4<f32> {
    let c = box4(in.uv);
    let peak = max(c.r, max(c.g, c.b));
    let k = clamp((peak - post.params.x) / max(peak, 1e-4), 0.0, 1.0);
    return vec4<f32>(c * k, 1.0);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(box4(in.uv), 1.0);
}

// Upsample (drawn with additive blending onto the next larger level).
@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(box4(in.uv), 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Identity up to a knee, then an exponential shoulder towards 1: keeps the original's
// colours in the normal range and rolls bright light off smoothly.
fn soft(x: vec3<f32>) -> vec3<f32> {
    let k = 0.8;
    let over = max(x - vec3<f32>(k), vec3<f32>(0.0));
    return min(x, vec3<f32>(k)) + (1.0 - k) * (1.0 - exp(-over / (1.0 - k)));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_tonemap(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSample(src, smp, in.uv).rgb + post.params.y * textureSample(bloom, smp, in.uv).rgb;
    c = c * post.params.z;
    if (post.params.w > 1.5) {
        c = aces(c);
    } else if (post.params.w > 0.5) {
        c = soft(c);
    } else {
        c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(to_srgb(c), 1.0);
}
