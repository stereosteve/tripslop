// Output stage: only affects what you see, never what is fed back.

struct PostU {
    a: vec4<f32>, // time, hue, invert, posterize levels
    b: vec4<f32>, // scanlines, vignette, brightness, height in px
    c: vec4<f32>, // aspect, rgb split amount, ring layer for green, ring layer for blue
};

@group(0) @binding(0) var<uniform> u: PostU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var ring: texture_2d_array<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(src, samp, in.uv, 0.0).rgb;
    // RGB time split: green and blue come from older frames of the delay line.
    if (u.c.y > 0.0) {
        let g = textureSampleLevel(ring, samp, in.uv, i32(u.c.z), 0.0).g;
        let b = textureSampleLevel(ring, samp, in.uv, i32(u.c.w), 0.0).b;
        c = vec3<f32>(c.r, mix(c.g, g, u.c.y), mix(c.b, b, u.c.y));
    }
    c = hue_rotate(c, u.a.y);
    if (u.a.z > 0.5) {
        c = 1.0 - c;
    }
    if (u.a.w >= 2.0) {
        c = floor(c * u.a.w) / (u.a.w - 1.0);
    }
    if (u.b.x > 0.0) {
        let line = 0.5 + 0.5 * sin(in.uv.y * u.b.w * PI);
        c = c * (1.0 - u.b.x * (1.0 - line));
    }
    if (u.b.y > 0.0) {
        let p = (in.uv - 0.5) * vec2<f32>(u.c.x, 1.0);
        c = c * (1.0 - u.b.y * smoothstep(0.3, 1.0, length(p)));
    }
    c = c * u.b.z;
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
