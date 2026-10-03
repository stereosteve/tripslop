// RGB time split: green and blue come from older frames.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = input(in.uv);
    let g = hist(in.uv, 1);
    let b = hist(in.uv, 2);
    let amt = P(1);
    return finish(vec4<f32>(base.r, mix(base.g, g.g, amt), mix(base.b, b.b, amt), max(base.a, max(g.a, b.a) * amt)));
}
