// Makes dark (or, inverted, bright) areas transparent.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = input(in.uv);
    let l = luma(unpremul(c).rgb);
    var k = smoothstep(P(0), P(0) + P(1) + 0.001, l);
    if (P(2) > 0.5) {
        k = 1.0 - k;
    }
    return c * k;
}
