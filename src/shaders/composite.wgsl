// Composites one layer (premultiplied) onto the composition so far.

struct CompU {
    a: vec4<f32>, // blend mode (9 = crossfade), opacity (or crossfade amount), unused, unused
};

@group(0) @binding(0) var<uniform> u: CompU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var base_tex: texture_2d<f32>;
@group(0) @binding(3) var layer_tex: texture_2d<f32>;

fn overlay(b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    let lo = 2.0 * b * s;
    let hi = 1.0 - 2.0 * (1.0 - b) * (1.0 - s);
    return select(hi, lo, b < vec3<f32>(0.5));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = textureSampleLevel(base_tex, samp, in.uv, 0.0);
    if i32(u.a.x) == 9 {
        // Crossfade: dissolve from the base (bank A) to the layer (bank B) by the amount.
        return mix(d, textureSampleLevel(layer_tex, samp, in.uv, 0.0), u.a.y);
    }
    let s = textureSampleLevel(layer_tex, samp, in.uv, 0.0) * u.a.y;
    let a = s.a;
    let sc = select(vec3<f32>(0.0), s.rgb / max(a, 0.0001), a > 0.0001); // straight colour
    var rgb: vec3<f32>;
    switch i32(u.a.x) {
        case 1: { rgb = d.rgb + s.rgb; }                      // add
        case 2: { rgb = d.rgb + s.rgb - d.rgb * s.rgb; }      // screen
        case 3: { rgb = mix(d.rgb, d.rgb * sc, a); }          // multiply
        case 4: { rgb = mix(d.rgb, abs(d.rgb - sc), a); }     // difference
        case 5: { rgb = mix(d.rgb, max(d.rgb, sc), a); }      // lighten
        case 6: { rgb = mix(d.rgb, min(d.rgb, sc), a); }      // darken
        case 7: { rgb = mix(d.rgb, overlay(d.rgb, sc), a); }  // overlay
        case 8: { rgb = d.rgb - s.rgb; }                      // subtract
        default: { rgb = s.rgb + d.rgb * (1.0 - a); }         // normal
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), a + d.a * (1.0 - a));
}
