// Shared solid geometry for the Shape projector and Projection mapping effects.
// A convex solid (N-sided prism, pyramid or bipyramid) is the intersection of half-spaces,
// one per face; a ray hits it where it has entered every half-space before leaving any.

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

struct SolidHit {
    ok: bool,
    t: f32,
    face: i32,
    /// Object-space normal.
    normal: vec3<f32>,
}

/// Ray vs. solid in object space. `shape` 0 prism, 1 pyramid, 2 bipyramid, 3 sphere (radius r).
fn hit_solid(ro: vec3<f32>, rd: vec3<f32>, shape: i32, n: i32, r: f32, h: f32) -> SolidHit {
    var hit: SolidHit;
    hit.ok = false;
    hit.t = 0.0;
    hit.face = -1;
    hit.normal = vec3<f32>(0.0);
    if (shape == 3) {
        let b = dot(ro, rd);
        let c = dot(ro, ro) - r * r;
        let disc = b * b - c;
        if (disc < 0.0) {
            return hit;
        }
        var t = -b - sqrt(disc);
        if (t < 0.0) {
            t = -b + sqrt(disc);
        }
        if (t < 0.0) {
            return hit;
        }
        hit.ok = true;
        hit.t = t;
        hit.normal = normalize(ro + rd * t);
        return hit;
    }
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
    if (face >= 0 && t_near <= t_far && t_near > 0.0) {
        hit.ok = true;
        hit.t = t_near;
        hit.face = face;
        hit.normal = plane(face, n, shape, r, h).xyz;
    }
    return hit;
}
