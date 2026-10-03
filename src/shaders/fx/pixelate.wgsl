@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cell = max(P(0), 1.0) * u.a.zw;
    let uv = (floor(in.uv / cell) + 0.5) * cell;
    var c = unpremul(input(uv));
    let levels = P(1);
    if (levels >= 2.0) {
        c = vec4<f32>(floor(c.rgb * levels) / (levels - 1.0), c.a);
    }
    return premul(vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), c.a));
}
