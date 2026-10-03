@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ar = vec2<f32>(aspect(), 1.0);
    let c = vec2<f32>(P(2), -P(3));
    let p = (in.uv - 0.5) * ar - c;
    let seg = TAU / max(P(0), 1.0);
    let r = length(p) / max(P(4), 0.01);
    var a = atan2(p.y, p.x) - radians(P(1));
    a = a - seg * floor(a / seg);
    a = abs(a - seg * 0.5);
    let q = vec2<f32>(cos(a), sin(a)) * r + c;
    return input(mirror_uv(q / ar + 0.5));
}
