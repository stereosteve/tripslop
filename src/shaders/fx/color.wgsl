@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = unpremul(input(in.uv));
    var rgb = hue_rotate(c.rgb, P(0));
    rgb = mix(vec3<f32>(luma(rgb)), rgb, P(1));
    rgb = (rgb - 0.5) * P(2) + 0.5;
    rgb = rgb * P(3);
    rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / max(P(5), 0.01)));
    if (P(4) > 0.5) {
        rgb = 1.0 - rgb;
    }
    return premul(vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), c.a));
}
