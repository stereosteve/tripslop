// Optional vertical flip between tripslop's top-left textures and Shadertoy's bottom-left (GL)
// convention, plus an alpha mode for user shader output.

struct FlipU {
    a: vec4<f32>, // mode (0 opaque, 1 luminance, 2 shader alpha, 3 copy as-is), flip (1 = flip vertically)
};

@group(0) @binding(0) var<uniform> u: FlipU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let v = select(in.uv.y, 1.0 - in.uv.y, u.a.y > 0.5);
    let c = textureSampleLevel(src, samp, vec2<f32>(in.uv.x, v), 0.0);
    switch i32(u.a.x) {
        case 0: { return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0); }
        case 1: {
            let rgb = clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
            return vec4<f32>(rgb, max3(rgb));
        }
        case 2: {
            let a = clamp(c.a, 0.0, 1.0);
            return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(a)), a);
        }
        default: { return c; }
    }
}
