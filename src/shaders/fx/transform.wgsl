@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ar = vec2<f32>(aspect(), 1.0);
    var p = (in.uv - 0.5) * ar - vec2<f32>(P(2) * aspect(), -P(3));
    p = rot2(-radians(P(1))) * p / max(P(0), 0.01);
    let uv = p / ar + 0.5;
    switch i32(P(4)) {
        case 1: { return input(fract(uv)); }
        case 2: { return input(mirror_uv(uv)); }
        default: {
            if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
                return vec4<f32>(0.0);
            }
            return input(uv);
        }
    }
}
