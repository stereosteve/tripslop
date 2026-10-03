@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let amp = P(0);
    let f = P(1) * TAU;
    let t = u.a.x * P(2);
    var uv = in.uv;
    switch i32(P(3)) {
        case 1: { uv.y = uv.y + amp * sin(uv.x * f + t); }
        case 2: {
            let p = (uv - 0.5) * vec2<f32>(aspect(), 1.0);
            let r = length(p);
            let dir = select(vec2<f32>(0.0), p / r, r > 0.0001);
            uv = uv + dir / vec2<f32>(aspect(), 1.0) * amp * sin(r * f - t);
        }
        default: { uv.x = uv.x + amp * sin(uv.y * f + t); }
    }
    return input(mirror_uv(uv));
}
