@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    switch i32(P(0)) {
        case 1: { uv.x = select(uv.x, 1.0 - uv.x, uv.x < 0.5); }
        case 2: { uv.y = select(uv.y, 1.0 - uv.y, uv.y > 0.5); }
        case 3: { uv.y = select(uv.y, 1.0 - uv.y, uv.y < 0.5); }
        case 4: { uv = select(uv, 1.0 - uv, uv > vec2<f32>(0.5)); }
        default: { uv.x = select(uv.x, 1.0 - uv.x, uv.x > 0.5); }
    }
    return input(uv);
}
