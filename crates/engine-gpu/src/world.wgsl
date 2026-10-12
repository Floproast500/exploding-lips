// World pass: palette-indexed textures from an atlas array, original light rules (shade =
// light + depth / visibility through the 16 shade tables), the panoramic sky, alpha-tested
// masked walls and sprites. Projection reproduces the software renderer: Hor+ field of view and
// the horizon moved by the pitch (y-shear).

struct Uniforms {
    cam: vec4<f32>,        // eye x, y, z, near
    basis: vec4<f32>,      // dir.xy, right.xy
    proj: vec4<f32>,       // fx, fy, pitch, focal (pixels)
    viewport: vec4<f32>,   // x, y, w, h (target pixels)
    shading: vec4<f32>,    // visibility, fade bias, true colour, full bright
    shading2: vec4<f32>,   // max object shade, zero substitute, shade levels, has sky
    sky_rect: vec4<u32>,   // atlas x, y, w, h
    sky_layer: vec4<u32>,  // layer
    look: vec4<f32>,       // freelook (0/1), sin(pitch), cos(pitch)
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas: texture_2d_array<u32>;
@group(0) @binding(2) var palette: texture_2d<f32>;      // 256 x 1
@group(0) @binding(3) var colormap: texture_2d<u32>;     // 256 x levels

// Portal window of the current batch: inside where every edge equation is >= 0 (NDC).
struct Clip {
    count: vec4<u32>,
    edges: array<vec4<f32>, 15>,
};
@group(1) @binding(0) var<uniform> clip: Clip;

const TRANSPARENT: u32 = 1u;
const SKY: u32 = 2u;
const NO_DISTANCE: u32 = 4u;
const DISCARD_OUTSIDE_V: u32 = 8u;
const CLAMP: u32 = 16u;
const OBJECT: u32 = 32u;
const VOID: u32 = 64u;
const TAU: f32 = 6.28318530718;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) rect: vec4<u32>,
    @location(3) layer_flags: vec2<u32>,
    @location(4) light: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) depth: f32,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) @interpolate(flat) layer_flags: vec2<u32>,
    @location(4) @interpolate(flat) light: f32,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let rel = v.pos - u.cam.xyz;
    let forward = dot(rel.xy, u.basis.xy);
    let right = dot(rel.xy, u.basis.zw);
    var out: VsOut;
    if (u.look.x > 0.5) {
        // True pitch: rotate (forward, up) about the right axis.
        let f = forward * u.look.z + rel.z * u.look.y;
        let up = rel.z * u.look.z - forward * u.look.y;
        out.clip = vec4<f32>(right * u.proj.x, up * u.proj.y, f - u.cam.w, f);
    } else {
        out.clip = vec4<f32>(right * u.proj.x, (rel.z - u.proj.z * forward) * u.proj.y, forward - u.cam.w, forward);
    }
    out.uv = v.uv;
    out.depth = forward;
    out.rect = v.rect;
    out.layer_flags = v.layer_flags;
    out.light = v.light;
    return out;
}

fn wrap(x: i32, n: i32) -> i32 {
    return ((x % n) + n) % n;
}

fn color_of(index: u32) -> vec4<f32> {
    return textureLoad(palette, vec2<i32>(i32(index), 0), 0);
}

fn sky(frag: vec2<f32>) -> vec4<f32> {
    if (u.shading2.w < 0.5) {
        return color_of(u32(u.shading2.y));
    }
    let px = frag - u.viewport.xy;
    let k = 2.0 * px.x / u.viewport.z - 1.0;
    let ray = u.basis.xy + u.basis.zw * (k / u.proj.x);
    let angle = atan2(ray.y, ray.x);
    let w = i32(u.sky_rect.z);
    let h = i32(u.sky_rect.w);
    let tu = wrap(i32(floor(-angle / TAU * f32(w))), w);
    let horizon = u.viewport.w / 2.0 + u.proj.z * u.proj.w;
    let top = horizon - u.viewport.w;
    let tv = clamp(i32((px.y - top) / (2.0 * u.viewport.w) * f32(h)), 0, h - 1);
    let index = textureLoad(atlas, vec2<i32>(i32(u.sky_rect.x) + tu, i32(u.sky_rect.y) + tv), i32(u.sky_layer.x), 0).r;
    return color_of(index);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = in.clip.xy - u.viewport.xy;
    let ndc = vec2<f32>(px.x / u.viewport.z * 2.0 - 1.0, 1.0 - px.y / u.viewport.w * 2.0);
    for (var i = 0u; i < clip.count.x; i = i + 1u) {
        let e = clip.edges[i];
        if (e.x * ndc.x + e.y * ndc.y + e.z < 0.0) {
            discard;
        }
    }
    let flags = in.layer_flags.y;
    if ((flags & SKY) != 0u) {
        return sky(in.clip.xy);
    }
    if ((flags & VOID) != 0u) {
        return color_of(0u);
    }
    let w = i32(in.rect.z);
    let h = i32(in.rect.w);
    var tx = i32(floor(in.uv.x));
    var ty = i32(floor(in.uv.y));
    if ((flags & CLAMP) != 0u) {
        tx = clamp(tx, 0, w - 1);
        ty = clamp(ty, 0, h - 1);
    } else {
        if ((flags & DISCARD_OUTSIDE_V) != 0u && (ty < 0 || ty >= h)) {
            discard;
        }
        tx = wrap(tx, w);
        ty = wrap(ty, h);
    }
    let index = textureLoad(atlas, vec2<i32>(i32(in.rect.x) + tx, i32(in.rect.y) + ty), i32(in.layer_flags.x), 0).r;
    if ((flags & TRANSPARENT) != 0u && index == 0u) {
        discard;
    }
    var shade = in.light + u.shading.y;
    if ((flags & NO_DISTANCE) == 0u) {
        var d = in.depth / u.shading.x;
        if (u.shading.z < 0.5) {
            d = floor(d);
        }
        shade = shade + d;
    }
    if ((flags & OBJECT) != 0u) {
        shade = min(shade, u.shading2.x);
    }
    if (u.shading.w > 0.5) {
        shade = 0.0;
    }
    let levels = u.shading2.z;
    if (u.shading.z > 0.5) {
        let f = clamp((levels - shade) / levels, 0.0, 1.0);
        let c = color_of(index);
        return vec4<f32>(c.rgb * f, 1.0);
    }
    let row = clamp(i32(shade), 0, i32(levels) - 1);
    var lit = textureLoad(colormap, vec2<i32>(i32(index), row), 0).r;
    if (lit == 0u) {
        lit = u32(u.shading2.y);
    }
    return color_of(lit);
}
