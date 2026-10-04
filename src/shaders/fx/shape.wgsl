// Projects the input onto a spinning 3D solid (N-sided prism, pyramid or bipyramid),
// ray-traced per pixel: each face is a plane, and a convex solid is the intersection of
// their half-spaces.
//
// Params: 0 shape, 1 sides, 2 size, 3 height, 4-6 rotation x/y/z (deg), 7-9 spin x/y/z
// (turns per bar), 10 mapping, 11 caps, 12 lighting, 13 background, 14 fov (deg), 15 x, 16 y.

fn rot_x(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, c, s), vec3<f32>(0.0, -s, c));
}

fn rot_y(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(c, 0.0, -s), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(s, 0.0, c));
}

fn rot_z(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(c, s, 0.0), vec3<f32>(-s, c, 0.0), vec3<f32>(0.0, 0.0, 1.0));
}

// Outward direction of side face i in the xz-plane.
fn side_dir(i: i32, n: i32) -> vec2<f32> {
    let a = (f32(i) + 0.5) * TAU / f32(n);
    return vec2<f32>(cos(a), sin(a));
}

// Plane i of the solid as (unit normal, offset): points p inside satisfy dot(n, p) <= d.
// Indices: sides first (n of them, or 2n for the bipyramid), then the caps.
fn plane(i: i32, n: i32, shape: i32, r: f32, h: f32) -> vec4<f32> {
    let apothem = r * cos(PI / f32(n));
    let hh = h * 0.5;
    if (shape == 0) {
        // Prism: n vertical sides, then top and bottom.
        if (i < n) {
            let d = side_dir(i, n);
            return vec4<f32>(d.x, 0.0, d.y, apothem);
        }
        if (i == n) {
            return vec4<f32>(0.0, 1.0, 0.0, hh);
        }
        return vec4<f32>(0.0, -1.0, 0.0, hh);
    }
    if (shape == 1) {
        // Pyramid: sides lean in to an apex at +hh; base at -hh.
        if (i < n) {
            let d = side_dir(i, n);
            let nn = vec3<f32>(h * d.x, apothem, h * d.y);
            let len = length(nn);
            return vec4<f32>(nn / len, apothem * hh / len);
        }
        return vec4<f32>(0.0, -1.0, 0.0, hh);
    }
    // Bipyramid: upper and lower halves meeting at y = 0.
    let up = i < n;
    let d = side_dir(i % n, n);
    let ny = select(-apothem, apothem, up);
    let nn = vec3<f32>(hh * d.x, ny, hh * d.y);
    let len = length(nn);
    return vec4<f32>(nn / len, apothem * hh / len);
}

fn plane_count(n: i32, shape: i32) -> i32 {
    if (shape == 0) {
        return n + 2;
    }
    if (shape == 1) {
        return n + 1;
    }
    return 2 * n;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let shape = i32(P(0));
    let n = clamp(i32(P(1)), 3, 12);
    let size = max(P(2), 0.01);
    let h = max(P(3), 0.01) * size * 1.4;
    let r = size;
    let wrap = P(10) > 0.5;
    let caps = P(11) > 0.5;
    let background = i32(P(13));

    // Camera: looking down +z from in front, perspective from the field of view.
    let ar = vec2<f32>(aspect(), 1.0);
    let sp = (in.uv - 0.5) * ar - vec2<f32>(P(15) * aspect(), -P(16));
    let half_fov = radians(clamp(P(14), 5.0, 150.0)) * 0.5;
    let focal = 1.0 / tan(half_fov);
    // Size 1 roughly fills the frame height; never let the camera end up inside the solid.
    let cam = max(0.9 * focal, 2.2 * max(r, h * 0.5) + 0.3);
    var ro = vec3<f32>(0.0, 0.0, -cam);
    var rd = normalize(vec3<f32>(sp.x * 2.0, -sp.y * 2.0, focal));

    // Object rotation: fixed angles plus beat-synced spin (turns per 4-beat bar).
    let bar_turns = u.b.x / 4.0 * TAU;
    let ax = radians(P(4)) + P(7) * bar_turns;
    let ay = radians(P(5)) + P(8) * bar_turns;
    let az = radians(P(6)) + P(9) * bar_turns;
    let m = rot_z(az) * rot_y(ay) * rot_x(ax);
    let mt = transpose(m);
    ro = mt * ro;
    rd = mt * rd;

    // Ray vs. intersection of half-spaces.
    var t_near = -1e9;
    var t_far = 1e9;
    var face = -1;
    let count = plane_count(n, shape);
    for (var i = 0; i < count; i = i + 1) {
        let pl = plane(i, n, shape, r, h);
        let denom = dot(pl.xyz, rd);
        let dist = dot(pl.xyz, ro) - pl.w;
        if (abs(denom) < 1e-6) {
            if (dist > 0.0) {
                t_near = 1e9;
            }
            continue;
        }
        let t = -dist / denom;
        if (denom < 0.0) {
            if (t > t_near) {
                t_near = t;
                face = i;
            }
        } else {
            t_far = min(t_far, t);
        }
    }

    let hit = face >= 0 && t_near <= t_far && t_far > 0.0;
    let is_cap = (shape == 0 && face >= n) || (shape == 1 && face == n);
    if (!hit || (is_cap && !caps)) {
        switch background {
            case 1: { return input(in.uv); }
            case 2: { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
            default: { return vec4<f32>(0.0); }
        }
    }

    let p = ro + rd * t_near;
    let pl = plane(face, n, shape, r, h);
    var uv: vec2<f32>;
    if (is_cap) {
        uv = vec2<f32>(p.x, -p.z) / (2.0 * r) + 0.5;
    } else {
        let side = face % n;
        let d = side_dir(side, n);
        let tangent = vec2<f32>(-d.y, d.x);
        let half_edge = r * sin(PI / f32(n));
        let local_u = dot(p.xz, tangent) / (2.0 * half_edge) + 0.5;
        var top = h * 0.5;
        var bottom = -h * 0.5;
        if (shape == 2) {
            if (face < n) {
                bottom = 0.0;
            } else {
                top = 0.0;
            }
        }
        let local_v = (top - p.y) / max(top - bottom, 1e-4);
        if (wrap) {
            uv = vec2<f32>((f32(side) + local_u) / f32(n), local_v);
        } else {
            uv = vec2<f32>(local_u, local_v);
        }
    }
    var c = input(clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));

    // Lighting: lambert from a light near the camera, blended by the lighting amount.
    let normal = m * pl.xyz;
    let light = normalize(vec3<f32>(-0.4, 0.5, -1.0));
    let lambert = 0.3 + 0.7 * max(dot(normal, light), 0.0);
    let shade = mix(1.0, lambert, P(12));
    c = vec4<f32>(c.rgb * shade, c.a);

    // Keep a solid face where the input is transparent, so the shape reads as a shape.
    let solid = vec4<f32>(c.rgb + vec3<f32>(0.04) * shade * (1.0 - c.a), 1.0);
    return select(c, solid, background == 2);
}
