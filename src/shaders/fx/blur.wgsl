// 24-tap golden-angle spiral blur.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let radius = P(0);
    if (radius <= 0.0) {
        return input(in.uv);
    }
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < 24; i = i + 1) {
        let fi = f32(i) + 0.5;
        let r = sqrt(fi / 24.0) * radius;
        let a = fi * 2.39996323;
        let w = 1.0 - fi / 26.0;
        sum = sum + input(in.uv + vec2<f32>(cos(a), sin(a)) * r * u.a.zw) * w;
        total = total + w;
    }
    return sum / total;
}
