// Shared interface for every effect: one input (premultiplied), an optional history ring
// of past frames, 24 parameters.

struct FxU {
    a: vec4<f32>, // time, aspect, texel x, texel y
    b: vec4<f32>, // beat, bpm, unused, unused
    r: vec4<f32>, // ring layer for history taps 0..3
    p: array<vec4<f32>, 6>,
};

@group(0) @binding(0) var<uniform> u: FxU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var ring: texture_2d_array<f32>;

fn P(i: i32) -> f32 {
    return u.p[i / 4][i % 4];
}

fn input(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src, samp, uv, 0.0);
}

fn hist(uv: vec2<f32>, tap: i32) -> vec4<f32> {
    return textureSampleLevel(ring, samp, uv, i32(u.r[tap]), 0.0);
}

fn aspect() -> f32 {
    return u.a.y;
}

// Keep premultiplied output valid (rgb <= alpha).
fn finish(c: vec4<f32>) -> vec4<f32> {
    let a = clamp(max(c.a, max3(c.rgb)), 0.0, 1.0);
    return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(a)), a);
}
