// Sobel outlines, tinted by the source colour.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = u.a.zw;
    var l: array<f32, 9>;
    var i = 0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            l[i] = luma(input(in.uv + vec2<f32>(f32(x), f32(y)) * t).rgb);
            i = i + 1;
        }
    }
    let gx = -l[0] - 2.0 * l[3] - l[6] + l[2] + 2.0 * l[5] + l[8];
    let gy = -l[0] - 2.0 * l[1] - l[2] + l[6] + 2.0 * l[7] + l[8];
    let e = clamp(length(vec2<f32>(gx, gy)) * P(0), 0.0, 1.0);
    let c = input(in.uv);
    let tint = 0.4 + 0.6 * unpremul(c).rgb;
    let edge = vec4<f32>(tint * e, e);
    return finish(mix(c, edge, P(1)));
}
