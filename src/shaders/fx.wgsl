// The feedback loop. Emulates an analog rig where a camera looks at one or more
// monitors showing its own (delayed) output: every frame, N scaled/rotated "copies"
// of an older output frame are composited, then fresh input is keyed on top.
// Repeated each frame, the copies become an iterated function system -> fractals.

struct FxU {
    a: vec4<f32>, // time, aspect, feedback, copies
    b: vec4<f32>, // scale, rotate(rad), spread, twist(rad)
    c: vec4<f32>, // center_x, center_y, combine, edge
    d: vec4<f32>, // symmetry, kaleido_segments, hue_shift, saturation
    e: vec4<f32>, // contrast, blur, noise, input_mode
    f: vec4<f32>, // input_level, key_threshold, key_softness, echo_amount
    g: vec4<f32>, // ring layers: loop, echo1, echo2, echo3
    h: vec4<f32>, // unused
    i: vec4<f32>, // texel size x, y, unused, unused
};

@group(0) @binding(0) var<uniform> u: FxU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var input_tex: texture_2d<f32>;
@group(0) @binding(3) var ring: texture_2d_array<f32>;

fn ring_at(uv: vec2<f32>, layer: f32) -> vec3<f32> {
    return textureSampleLevel(ring, samp, uv, i32(layer), 0.0).rgb;
}

// Address mode for copies that sample outside the "monitor".
// Returns uv in .xy and a visibility weight in .z.
fn edge_uv(q: vec2<f32>) -> vec3<f32> {
    let mode = i32(u.c.w);
    if (mode == 1) {
        let t = fract(q * 0.5) * 2.0;
        return vec3<f32>(1.0 - abs(t - 1.0), 1.0);
    }
    if (mode == 2) {
        return vec3<f32>(fract(q), 1.0);
    }
    // Black bezel: soft 1.5px edge to avoid shimmering.
    let px = u.i.xy * 1.5;
    let wx = smoothstep(0.0, px.x, q.x) * smoothstep(0.0, px.x, 1.0 - q.x);
    let wy = smoothstep(0.0, px.y, q.y) * smoothstep(0.0, px.y, 1.0 - q.y);
    return vec3<f32>(clamp(q, vec2<f32>(0.0), vec2<f32>(1.0)), wx * wy);
}

fn sample_prev(q: vec2<f32>) -> vec3<f32> {
    let e = edge_uv(q);
    if (e.z <= 0.0) {
        return vec3<f32>(0.0);
    }
    let layer = u.g.x;
    var col = ring_at(e.xy, layer);
    let blur = u.e.y;
    if (blur > 0.0) {
        let o = u.i.xy * (1.0 + blur * 4.0);
        col = col * 0.2
            + ring_at(e.xy + vec2<f32>(o.x, 0.0), layer) * 0.2
            + ring_at(e.xy - vec2<f32>(o.x, 0.0), layer) * 0.2
            + ring_at(e.xy + vec2<f32>(0.0, o.y), layer) * 0.2
            + ring_at(e.xy - vec2<f32>(0.0, o.y), layer) * 0.2;
    }
    return col * e.z;
}

fn fold_symmetry(p_in: vec2<f32>) -> vec2<f32> {
    var p = p_in;
    let mode = i32(u.d.x);
    if (mode == 1) {
        p.x = abs(p.x);
    } else if (mode == 2) {
        p = abs(p);
    } else if (mode == 3) {
        let n = max(u.d.y, 1.0);
        let seg = TAU / n;
        let r = length(p);
        var a = atan2(p.y, p.x);
        a = a - seg * floor(a / seg);
        a = abs(a - seg * 0.5);
        p = vec2<f32>(cos(a), sin(a)) * r;
    }
    return p;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = u.a.y;
    let ar = vec2<f32>(aspect, 1.0);
    let center = u.c.xy;
    let p = fold_symmetry((in.uv - 0.5) * ar - center) + center;

    // --- N copies of the delayed frame: the "camera sees several monitors" stage ---
    let n = i32(max(u.a.w, 1.0));
    let scale = max(u.b.x, 0.001);
    let combine = i32(u.c.z);
    var fb = vec3<f32>(0.0);
    for (var k = 0; k < n; k = k + 1) {
        let fk = f32(k);
        let place = fk * TAU / f32(n) - PI * 0.5 + u.b.y;
        let offset = center + u.b.z * vec2<f32>(cos(place), sin(place));
        let ang = u.b.y + fk * u.b.w;
        let src = rot2(-ang) * (p - offset) / scale + center;
        let col = sample_prev(src / ar + 0.5);
        if (combine == 0) {
            fb = max(fb, col);
        } else {
            fb = fb + col;
        }
    }
    if (combine == 2) {
        fb = fb / f32(n);
    }

    // --- signal path colour processing (like a proc amp in the loop) ---
    fb = fb * u.a.z;
    fb = hue_rotate(fb, u.d.z);
    let l = luma(fb);
    fb = mix(vec3<f32>(l), fb, u.d.w);
    fb = (fb - 0.5) * u.e.x + 0.5;
    if (u.e.z > 0.0) {
        let nz = hash12(in.pos.xy + vec2<f32>(u.a.x * 113.0, u.a.x * 71.0)) - 0.5;
        fb = fb + nz * u.e.z * 0.25;
    }
    fb = clamp(fb, vec3<f32>(0.0), vec3<f32>(1.0));

    // --- keyer: fresh input over the feedback ---
    let src_in = textureSampleLevel(input_tex, samp, in.uv, 0.0).rgb;
    let inp = src_in * u.f.x;
    var out: vec3<f32>;
    switch i32(u.e.w) {
        case 1: { out = fb + inp; }
        case 2: { out = max(fb, inp); }
        case 3: { out = abs(fb - inp); }
        default: {
            let th = u.f.y;
            let k = smoothstep(th, th + u.f.z + 0.001, luma(src_in));
            out = mix(fb, inp, k);
        }
    }

    // --- video delay line taps ---
    if (u.f.w > 0.0) {
        let echo = ring_at(in.uv, u.g.y) * 0.55 + ring_at(in.uv, u.g.z) * 0.3 + ring_at(in.uv, u.g.w) * 0.15;
        out = max(out, echo * u.f.w * 1.5) * 0.5 + (out + echo * u.f.w) * 0.5;
    }

    return vec4<f32>(clamp(out, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
