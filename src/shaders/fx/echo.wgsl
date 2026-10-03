// Video delay trails: mixes in the input from 1, 2 and 3 × spacing frames ago.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = input(in.uv);
    let amt = P(0);
    let d = P(2);
    let t1 = hist(in.uv, 1) * d;
    let t2 = hist(in.uv, 2) * d * d;
    let t3 = hist(in.uv, 3) * d * d * d;
    var out: vec4<f32>;
    switch i32(P(3)) {
        case 1: { out = base + (t1 + t2 + t3) * amt; }
        case 2: { out = mix(base, (base + t1 + t2 + t3) / (1.0 + d + d * d + d * d * d), amt); }
        default: { out = max(base, max(t1, max(t2, t3)) * amt); }
    }
    return finish(out);
}
