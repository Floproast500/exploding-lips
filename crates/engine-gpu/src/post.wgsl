// Post-processing for modern lighting: bloom (bright-pass downsample chain, additive tent
// upsample) and tonemapping of the HDR image to the display, encoding sRGB in the shader
// because presentation surfaces here are not sRGB formats.

struct Post {
    params: vec4<f32>,   // threshold, bloom strength, exposure, tonemap (0 clamp, 1 soft, 2 ACES)
    texel: vec4<f32>,    // 1 / source size (xy), FXAA on (z)
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

// The display colour (sRGB-encoded) of the HDR image plus bloom at `uv`.
fn graded(uv: vec2<f32>) -> vec3<f32> {
    var c = textureSampleLevel(src, smp, uv, 0.0).rgb + post.params.y * textureSampleLevel(bloom, smp, uv, 0.0).rgb;
    c = c * post.params.z;
    if (post.params.w > 1.5) {
        c = aces(c);
    } else if (post.params.w > 0.5) {
        c = soft(c);
    } else {
        c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return to_srgb(c);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

// FXAA (after Lottes): find the edge direction from the corners' luma and blend along it,
// keeping the wider blend only when it stays within the local luma range.
fn fxaa(uv: vec2<f32>) -> vec3<f32> {
    let t = post.texel.xy;
    let m = graded(uv);
    let nw = luma(graded(uv + vec2<f32>(-1.0, -1.0) * t));
    let ne = luma(graded(uv + vec2<f32>(1.0, -1.0) * t));
    let sw = luma(graded(uv + vec2<f32>(-1.0, 1.0) * t));
    let se = luma(graded(uv + vec2<f32>(1.0, 1.0) * t));
    let lm = luma(m);
    let lo = min(lm, min(min(nw, ne), min(sw, se)));
    let hi = max(lm, max(max(nw, ne), max(sw, se)));
    var dir = vec2<f32>(-((nw + ne) - (sw + se)), (nw + sw) - (ne + se));
    let reduce = max((nw + ne + sw + se) * 0.25 * (1.0 / 8.0), 1.0 / 128.0);
    let scale = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * scale, vec2<f32>(-8.0), vec2<f32>(8.0)) * t;
    let a = 0.5 * (graded(uv + dir * (1.0 / 3.0 - 0.5)) + graded(uv + dir * (2.0 / 3.0 - 0.5)));
    let b = a * 0.5 + 0.25 * (graded(uv - dir * 0.5) + graded(uv + dir * 0.5));
    let lb = luma(b);
    return select(b, a, lb < lo || lb > hi);
}

@fragment
fn fs_tonemap(in: VsOut) -> @location(0) vec4<f32> {
    if (post.texel.z > 0.5) {
        return vec4<f32>(fxaa(in.uv), 1.0);
    }
    return vec4<f32>(graded(in.uv), 1.0);
}
