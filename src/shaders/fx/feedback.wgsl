// The analog rig: a camera pointed at N monitors that show its own (delayed) output.
// Each frame, N scaled/rotated copies of an older output are composited and fresh input is
// keyed on top. Repeated every frame, the copies form an iterated function system: fractals.

fn edge_uv(q: vec2<f32>) -> vec3<f32> {
    let mode = i32(P(9));
    if (mode == 1) {
        return vec3<f32>(mirror_uv(q), 1.0);
    }
    if (mode == 2) {
        return vec3<f32>(fract(q), 1.0);
    }
    // Black bezel, with a soft edge to avoid shimmer.
    let px = u.a.zw * 1.5;
    let wx = smoothstep(0.0, px.x, q.x) * smoothstep(0.0, px.x, 1.0 - q.x);
    let wy = smoothstep(0.0, px.y, q.y) * smoothstep(0.0, px.y, 1.0 - q.y);
    return vec3<f32>(clamp(q, vec2<f32>(0.0), vec2<f32>(1.0)), wx * wy);
}

fn sample_prev(q: vec2<f32>) -> vec4<f32> {
    let e = edge_uv(q);
    if (e.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    var col = hist(e.xy, 0);
    let blur = P(15);
    if (blur > 0.0) {
        let o = u.a.zw * (1.0 + blur * 4.0);
        col = col * 0.2
            + hist(e.xy + vec2<f32>(o.x, 0.0), 0) * 0.2
            + hist(e.xy - vec2<f32>(o.x, 0.0), 0) * 0.2
            + hist(e.xy + vec2<f32>(0.0, o.y), 0) * 0.2
            + hist(e.xy - vec2<f32>(0.0, o.y), 0) * 0.2;
    }
    return col * e.z;
}

fn fold_symmetry(p_in: vec2<f32>) -> vec2<f32> {
    var p = p_in;
    let mode = i32(P(10));
    if (mode == 1) {
        p.x = abs(p.x);
    } else if (mode == 2) {
        p = abs(p);
    } else if (mode == 3) {
        let seg = TAU / max(P(11), 1.0);
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
    let ar = vec2<f32>(aspect(), 1.0);
    let center = vec2<f32>(P(6), P(7));
    let p = fold_symmetry((in.uv - 0.5) * ar - center) + center;

    let n = i32(max(P(1), 1.0));
    let scale = max(P(2), 0.001);
    let rot = radians(P(3));
    let spread = P(4);
    let twist = radians(P(5));
    let combine = i32(P(8));
    var fb = vec4<f32>(0.0);
    for (var k = 0; k < n; k = k + 1) {
        let fk = f32(k);
        let place = fk * TAU / f32(n) - PI * 0.5 + rot;
        let offset = center + spread * vec2<f32>(cos(place), sin(place));
        let src_p = rot2(-(rot + fk * twist)) * (p - offset) / scale + center;
        let col = sample_prev(src_p / ar + 0.5);
        if (combine == 0) {
            fb = max(fb, col);
        } else {
            fb = fb + col;
        }
    }
    if (combine == 2) {
        fb = fb / f32(n);
    }

    // Proc amp in the loop (on straight colour).
    fb = min(fb * P(0), vec4<f32>(1.0));
    var s = unpremul(fb);
    var rgb = hue_rotate(s.rgb, P(12));
    rgb = mix(vec3<f32>(luma(rgb)), rgb, P(13));
    rgb = (rgb - 0.5) * P(14) + 0.5;
    if (P(16) > 0.0) {
        let nz = hash12(in.pos.xy + vec2<f32>(u.a.x * 113.0, u.a.x * 71.0)) - 0.5;
        rgb = rgb + nz * P(16) * 0.25;
    }
    fb = premul(vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), s.a));

    // Keyer: fresh input over the loop.
    let raw = input(in.uv);
    let inp = vec4<f32>(raw.rgb * P(18), raw.a);
    var out: vec4<f32>;
    switch i32(P(17)) {
        case 1: { out = fb + inp; }
        case 2: { out = max(fb, inp); }
        case 3: { out = vec4<f32>(abs(fb.rgb - inp.rgb), max(fb.a, inp.a)); }
        case 4: { out = inp + fb * (1.0 - raw.a); }
        default: {
            let th = P(19);
            let k = smoothstep(th, th + P(20) + 0.001, luma(raw.rgb));
            out = mix(fb, inp, k);
        }
    }
    return finish(out);
}
