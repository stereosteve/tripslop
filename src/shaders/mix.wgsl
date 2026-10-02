// Deck A/B mixer with built-in "video synth" oscillators.

struct MixU {
    // time, crossfade, blend_mode, aspect
    g: vec4<f32>,
    // per deck: (use_pattern, pattern, freq, speed), (gain, hue, invert, has_tex)
    a0: vec4<f32>,
    a1: vec4<f32>,
    b0: vec4<f32>,
    b1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: MixU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex_a: texture_2d<f32>;
@group(0) @binding(3) var tex_b: texture_2d<f32>;

fn pattern(kind: i32, uv: vec2<f32>, freq: f32, speed: f32, t: f32) -> vec3<f32> {
    let aspect = u.g.w;
    let p = (uv - 0.5) * vec2<f32>(aspect, 1.0);
    let ts = t * speed;
    switch kind {
        case 0: { // bars
            let q = rot2(ts * 0.2) * p;
            let v = 0.5 + 0.5 * sin(q.x * freq * TAU * 0.5 + ts * 3.0);
            return palette(floor(q.x * freq * 0.5 + ts) * 0.17) * smoothstep(0.3, 0.7, v);
        }
        case 1: { // rings
            let r = length(p);
            let v = 0.5 + 0.5 * sin(r * freq * TAU - ts * 6.0);
            return palette(r * 0.8 - ts * 0.3) * v;
        }
        case 2: { // plasma
            let f = freq * 0.6;
            let v = sin(p.x * f + ts) + sin(p.y * f * 1.3 - ts * 1.1)
                + sin(length(p) * f * 1.7 + ts * 0.7) + sin((p.x + p.y) * f * 0.7 - ts * 0.5);
            return palette(v * 0.25 + ts * 0.05);
        }
        case 3: { // checker
            let q = rot2(ts * 0.3) * p * freq;
            let c = (i32(floor(q.x)) + i32(floor(q.y))) & 1;
            return palette(ts * 0.1) * f32(c);
        }
        default: { // orbiting dot: a perfect seed for feedback fractals
            let c = vec2<f32>(cos(ts * 1.3), sin(ts * 1.7)) * 0.25;
            let d = length(p - c);
            let rad = 0.02 + 0.06 / max(freq * 0.25, 0.1);
            return palette(ts * 0.15) * (1.0 - smoothstep(rad * 0.7, rad, d));
        }
    }
}

// Scale-to-cover the output, preserving the texture's aspect ratio.
fn cover_uv(uv: vec2<f32>, dims: vec2<u32>) -> vec2<f32> {
    let ta = f32(dims.x) / f32(dims.y);
    let oa = u.g.w;
    if (ta > oa) {
        return vec2<f32>(0.5 + (uv.x - 0.5) * oa / ta, uv.y);
    }
    return vec2<f32>(uv.x, 0.5 + (uv.y - 0.5) * ta / oa);
}

fn deck(which: i32, uv: vec2<f32>) -> vec3<f32> {
    var d0 = u.a0;
    var d1 = u.a1;
    if (which == 1) {
        d0 = u.b0;
        d1 = u.b1;
    }
    var c = vec3<f32>(0.0);
    if (d0.x > 0.5) {
        c = pattern(i32(d0.y), uv, d0.z, d0.w, u.g.x);
    } else if (d1.w > 0.5) {
        if (which == 0) {
            c = textureSampleLevel(tex_a, samp, cover_uv(uv, textureDimensions(tex_a)), 0.0).rgb;
        } else {
            c = textureSampleLevel(tex_b, samp, cover_uv(uv, textureDimensions(tex_b)), 0.0).rgb;
        }
    }
    c = hue_rotate(c, d1.y);
    if (d1.z > 0.5) {
        c = 1.0 - c;
    }
    return c * d1.x;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let a = deck(0, in.uv);
    let b = deck(1, in.uv);
    let x = u.g.y;
    var c: vec3<f32>;
    switch i32(u.g.z) {
        case 1: { c = a * min(1.0, 2.0 - 2.0 * x) + b * min(1.0, 2.0 * x); }
        case 2: { c = mix(a, a * b, x); }
        case 3: { c = mix(a, abs(a - b), x); }
        case 4: {
            // B keyed over A where B is bright; crossfader sets the key threshold.
            let k = smoothstep(1.0 - x, 1.0 - x + 0.1, luma(b));
            c = mix(a, b, k);
        }
        case 5: { c = max(a * min(1.0, 2.0 - 2.0 * x), b * min(1.0, 2.0 * x)); }
        default: { c = mix(a, b, x); }
    }
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
