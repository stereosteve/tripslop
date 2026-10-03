// Composition -> output: master fader, opaque.

struct FinalU {
    a: vec4<f32>, // master, unused...
};

@group(0) @binding(0) var<uniform> u: FinalU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src, samp, in.uv, 0.0);
    return vec4<f32>(clamp(c.rgb * u.a.x, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
