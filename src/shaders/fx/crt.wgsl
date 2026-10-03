@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var p = in.uv * 2.0 - 1.0;
    p = p + p * dot(p, p) * P(4);
    let uv = p * 0.5 + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let shift = vec2<f32>(P(3) * u.a.z, 0.0);
    let cr = input(uv + shift);
    let cg = input(uv);
    let cb = input(uv - shift);
    var rgb = vec3<f32>(cr.r, cg.g, cb.b);
    let a = max(cg.a, max(cr.a, cb.a));
    let line = 0.5 + 0.5 * sin(uv.y / u.a.w * PI);
    rgb = rgb * (1.0 - P(0) * (1.0 - line));
    let v = (uv - 0.5) * vec2<f32>(aspect(), 1.0);
    rgb = rgb * (1.0 - P(1) * smoothstep(0.3, 1.0, length(v)));
    let nz = hash12(in.pos.xy + vec2<f32>(u.a.x * 91.0, u.a.x * 57.0)) - 0.5;
    rgb = rgb + nz * P(2) * 0.3;
    return finish(vec4<f32>(rgb, a));
}
