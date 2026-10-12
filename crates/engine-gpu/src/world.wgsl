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
    modern: vec4<f32>,     // modern lighting (0/1), dynamic lights (0/1), light count
    brightness: array<vec4<f32>, 4>,
    emissive: array<vec4<u32>, 2>,
};

struct Lights {
    data: array<vec4<f32>, 128>, // per light: pos.xyz + radius, colour.rgb
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas: texture_2d_array<u32>;
@group(0) @binding(2) var palette: texture_2d<f32>;      // 256 x 1
@group(0) @binding(3) var colormap: texture_2d<u32>;     // 256 x levels
@group(0) @binding(4) var<uniform> lights: Lights;

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
    @location(5) normal: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) depth: f32,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) @interpolate(flat) layer_flags: vec2<u32>,
    @location(4) @interpolate(flat) light: f32,
    @location(5) world: vec3<f32>,
    @location(6) normal: vec3<f32>,
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
    out.world = v.pos;
    out.normal = v.normal;
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// Brightness of a continuous shade value from the fitted 16-level table.
fn brightness(shade: f32) -> f32 {
    let s = clamp(shade, 0.0, 15.0);
    let i = u32(floor(s));
    let j = min(i + 1u, 15u);
    let a = u.brightness[i / 4u][i % 4u];
    let b = u.brightness[j / 4u][j % 4u];
    return mix(a, b, s - floor(s));
}

fn modern_color(index: u32, shade: f32, world: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let albedo = to_linear(color_of(index).rgb);
    var b = brightness(shade);
    if (((u.emissive[index / 128u][(index / 32u) % 4u] >> (index % 32u)) & 1u) != 0u) {
        b = max(b, 1.5);
    }
    var light = vec3<f32>(b);
    if (u.modern.y > 0.5) {
        let n = normalize(normal);
        for (var i = 0u; i < u32(u.modern.z); i = i + 1u) {
            let p = lights.data[i * 2u];
            let c = lights.data[i * 2u + 1u].rgb;
            let to = p.xyz - world;
            let d = length(to);
            let att = pow(clamp(1.0 - d / p.w, 0.0, 1.0), 2.0);
            let facing = max(dot(n, to / max(d, 1e-3)), 0.0);
            light = light + c * att * mix(facing, 1.0, 0.25);
        }
    }
    return vec4<f32>(albedo * light, 1.0);
}

fn wrap(x: i32, n: i32) -> i32 {
    return ((x % n) + n) % n;
}

fn color_of(index: u32) -> vec4<f32> {
    return textureLoad(palette, vec2<i32>(i32(index), 0), 0);
}

fn sky(frag: vec2<f32>) -> vec4<f32> {
    let c = sky_index(frag);
    if (u.modern.x > 0.5) {
        return vec4<f32>(to_linear(color_of(c).rgb), 1.0);
    }
    return color_of(c);
}

fn sky_index(frag: vec2<f32>) -> u32 {
    if (u.shading2.w < 0.5) {
        return u32(u.shading2.y);
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
    return textureLoad(atlas, vec2<i32>(i32(u.sky_rect.x) + tu, i32(u.sky_rect.y) + tv), i32(u.sky_layer.x), 0).r;
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
        if (u.modern.x > 0.5) {
            return vec4<f32>(to_linear(color_of(0u).rgb), 1.0);
        }
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
        if (u.shading.z < 0.5 && u.modern.x < 0.5) {
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
    if (u.modern.x > 0.5) {
        return modern_color(index, shade, in.world, in.normal);
    }
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
