// Depth of opaque world geometry from a point light, one cube face per pass. Each face is a
// 90-degree view along `forward` with screen axes `right` and `up` chosen so that sampling the
// cube with (point - light) lands on the texel this pass wrote (Vulkan/D3D face layout).

struct Face {
    light: vec4<f32>,    // position, unused
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    depth: vec4<f32>,    // a, b: stored depth = a + b / distance along forward
};

@group(0) @binding(0) var<uniform> face: Face;

const TRANSPARENT: u32 = 1u;
const SKY: u32 = 2u;
const OBJECT: u32 = 32u;
const VOID: u32 = 64u;
const SCREEN: u32 = 128u;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(3) layer_flags: vec2<u32>,
};

@vertex
fn vs_main(v: VsIn) -> @builtin(position) vec4<f32> {
    if ((v.layer_flags.y & (TRANSPARENT | SKY | OBJECT | VOID | SCREEN)) != 0u) {
        return vec4<f32>(2.0, 2.0, 2.0, 1.0); // not a caster: outside the view
    }
    let rel = v.pos - face.light.xyz;
    let z = dot(rel, face.forward.xyz);
    return vec4<f32>(dot(rel, face.right.xyz), dot(rel, face.up.xyz), z * face.depth.x + face.depth.y, z);
}
