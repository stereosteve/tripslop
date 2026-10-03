// Beat-synced strobe.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var divs = array<f32, 6>(0.0625, 0.125, 0.25, 0.5, 1.0, 2.0);
    let div = divs[clamp(i32(P(0)), 0, 5)];
    let on = fract(u.b.x / div) < P(1);
    let c = input(in.uv);
    switch i32(P(2)) {
        case 1: { return select(c, vec4<f32>(1.0), on); }
        case 2: { return select(c, vec4<f32>(c.a - c.rgb, c.a), on); }
        case 3: { return select(vec4<f32>(0.0), c, on); }
        default: { return select(vec4<f32>(0.0, 0.0, 0.0, 1.0), c, on); }
    }
}
