// Draws a clip (texture or generator) into a layer, applying the layer transform and the
// clip's fit mode. Output is premultiplied; blending "over" is done by the pipeline.

struct ClipU {
    a: vec4<f32>, // time, aspect, mode (0 texture, 1 generator), opacity
    b: vec4<f32>, // pos x, pos y, scale, rotation (rad)
    c: vec4<f32>, // fit (0 fill, 1 fit, 2 stretch), texture aspect, pattern, frequency
    d: vec4<f32>, // pattern speed, hue, texture is straight alpha (1) or premultiplied (0), 1 / target height
};

@group(0) @binding(0) var<uniform> u: ClipU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex: texture_2d<f32>;

fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash12(i);
    let b = hash12(i + vec2<f32>(1.0, 0.0));
    let c = hash12(i + vec2<f32>(0.0, 1.0));
    let d = hash12(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// Generators output premultiplied colour with alpha = brightness, so they layer nicely.
fn pattern(kind: i32, uv: vec2<f32>) -> vec4<f32> {
    let aspect = u.a.y;
    let p = (uv - 0.5) * vec2<f32>(aspect, 1.0);
    let freq = u.c.w;
    let ts = u.a.x * u.d.x;
    let hue = u.d.y;
    var col = vec3<f32>(0.0);
    switch kind {
        case 0: { // bars
            let q = rot2(ts * 0.2) * p;
            let v = 0.5 + 0.5 * sin(q.x * freq * TAU * 0.5 + ts * 3.0);
            col = palette(floor(q.x * freq * 0.5 + ts) * 0.17 + hue) * smoothstep(0.3, 0.7, v);
        }
        case 1: { // rings
            let r = length(p);
            let v = 0.5 + 0.5 * sin(r * freq * TAU - ts * 6.0);
            col = palette(r * 0.8 - ts * 0.3 + hue) * v;
        }
        case 2: { // plasma
            let f = freq * 0.6;
            let v = sin(p.x * f + ts) + sin(p.y * f * 1.3 - ts * 1.1)
                + sin(length(p) * f * 1.7 + ts * 0.7) + sin((p.x + p.y) * f * 0.7 - ts * 0.5);
            col = palette(v * 0.25 + ts * 0.05 + hue);
        }
        case 3: { // checker
            let q = rot2(ts * 0.3) * p * freq;
            let c = (i32(floor(q.x)) + i32(floor(q.y))) & 1;
            col = palette(ts * 0.1 + hue) * f32(c);
        }
        case 4: { // orbiting dot: a perfect seed for feedback fractals
            let c = vec2<f32>(cos(ts * 1.3), sin(ts * 1.7)) * 0.25;
            let d = length(p - c);
            let rad = 0.02 + 0.06 / max(freq * 0.25, 0.1);
            col = palette(ts * 0.15 + hue) * (1.0 - smoothstep(rad * 0.7, rad, d));
        }
        case 5: { // drifting value noise
            let n = noise2(p * freq + vec2<f32>(ts, ts * 0.7)) * 0.65
                + noise2(p * freq * 2.3 - vec2<f32>(ts * 0.5, 0.0)) * 0.35;
            col = palette(n + hue) * smoothstep(0.35, 0.75, n);
        }
        default: { // solid colour
            return vec4<f32>(palette(hue), 1.0);
        }
    }
    return vec4<f32>(col, max3(col));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = u.a.y;
    let ar = vec2<f32>(aspect, 1.0);
    // Screen position -> position inside the layer's (moved, rotated, scaled) box.
    var p = (in.uv - 0.5) * ar - vec2<f32>(u.b.x * aspect, -u.b.y);
    p = rot2(-u.b.w) * p / max(u.b.z, 0.001);
    let luv = p / ar + 0.5;

    // Soft 1px edge on the box.
    let px = vec2<f32>(1.5 * u.d.w) / max(u.b.z, 0.001);
    let edge = smoothstep(vec2<f32>(0.0), px, luv) * smoothstep(vec2<f32>(0.0), px, 1.0 - luv);
    let inside = edge.x * edge.y;
    if (inside <= 0.0) {
        return vec4<f32>(0.0);
    }

    var c: vec4<f32>;
    if (u.a.z > 0.5) {
        c = pattern(i32(u.c.z), luv);
    } else {
        var tuv = luv;
        let ta = u.c.y;
        let fit = i32(u.c.x);
        if (fit == 0) {
            // Fill: cover the box, crop overflow.
            if (ta > aspect) {
                tuv.x = 0.5 + (luv.x - 0.5) * aspect / ta;
            } else {
                tuv.y = 0.5 + (luv.y - 0.5) * ta / aspect;
            }
        } else if (fit == 1) {
            // Fit: show everything, transparent bars.
            if (ta > aspect) {
                tuv.y = 0.5 + (luv.y - 0.5) * ta / aspect;
            } else {
                tuv.x = 0.5 + (luv.x - 0.5) * aspect / ta;
            }
            if (any(tuv < vec2<f32>(0.0)) || any(tuv > vec2<f32>(1.0))) {
                return vec4<f32>(0.0);
            }
        }
        c = textureSampleLevel(tex, samp, tuv, 0.0);
        if (u.d.z > 0.5) {
            c = premul(c);
        }
    }
    return c * inside * u.a.w;
}
