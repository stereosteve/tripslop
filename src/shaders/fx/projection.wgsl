// Projection mapping: a projector throws the input at a spinning 3D object, seen from a
// different angle. For each pixel, a ray from the viewer finds the surface it hits; that
// point is then projected into the projector's image to find which input pixel lands there.
// Faces turned away from the projector stay dark; an optional back wall catches the rest of
// the image, with the object's shadow.
//
// Params: 0 shape, 1 sides, 2 size, 3 height, 4-6 rotation x/y/z (deg), 7-9 spin x/y/z
// (turns per bar), 10 projector angle (deg), 11 projector elevation (deg), 12 projector zoom,
// 13 wall, 14 wall distance, 15 ambient, 16 shading, 17 background, 18 x, 19 y.

/// Where a world point lands in the projector's image: uv in .xy, .z > 0 if inside the beam.
fn to_projector(x: vec3<f32>, pos: vec3<f32>, fwd: vec3<f32>, right: vec3<f32>, up: vec3<f32>, focal: f32) -> vec3<f32> {
    let v = x - pos;
    let z = dot(v, fwd);
    if (z <= 0.0) {
        return vec3<f32>(0.0);
    }
    let px = dot(v, right) / z * focal;
    let py = dot(v, up) / z * focal;
    let uv = vec2<f32>(0.5 + px / (2.0 * aspect()), 0.5 - py * 0.5);
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    return vec3<f32>(uv, select(0.0, 1.0, inside));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let shape = i32(P(0));
    let n = clamp(i32(P(1)), 3, 12);
    let size = max(P(2), 0.01);
    let h = max(P(3), 0.01) * size * 1.4;
    let r = size;
    let wall_on = P(13) > 0.5;
    let ambient = P(15);
    let background = i32(P(17));

    // Viewer (same camera as the Shape projector).
    let ar = vec2<f32>(aspect(), 1.0);
    let sp = (in.uv - 0.5) * ar - vec2<f32>(P(18) * aspect(), -P(19));
    let focal = 1.0 / tan(radians(22.5));
    let cam = max(0.9 * focal, 2.2 * max(r, h * 0.5) + 0.3);
    let ro = vec3<f32>(0.0, 0.0, -cam);
    let rd = normalize(vec3<f32>(sp.x * 2.0, -sp.y * 2.0, focal));

    // Object rotation: fixed angles plus beat-synced spin (turns per 4-beat bar).
    let bar_turns = u.b.x / 4.0 * TAU;
    let m = rot_z(radians(P(6)) + P(9) * bar_turns) * rot_y(radians(P(5)) + P(8) * bar_turns) * rot_x(radians(P(4)) + P(7) * bar_turns);
    let mt = transpose(m);

    // Projector: same distance as the viewer, swung around by angle / elevation, aimed at
    // the object. Zoom narrows its beam (a bigger image on the object).
    let yaw = radians(P(10));
    let pitch = radians(P(11));
    let ppos = cam * vec3<f32>(sin(yaw) * cos(pitch), sin(pitch), -cos(yaw) * cos(pitch));
    let pfwd = normalize(-ppos);
    let pright = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), pfwd));
    let pup = cross(pfwd, pright);
    let pfocal = focal * max(P(12), 0.05);

    // What does the viewer's ray hit first: the object or the wall?
    let obj = hit_solid(mt * ro, mt * rd, shape, n, r, h);
    let wall_z = max(r, h * 0.5) + P(14);
    let t_wall = (wall_z - ro.z) / rd.z;

    if (obj.ok) {
        let x = ro + rd * obj.t;
        let normal = m * obj.normal;
        let to_proj = normalize(ppos - x);
        let facing = dot(normal, to_proj);
        var light = vec3<f32>(0.0);
        let pj = to_projector(x, ppos, pfwd, pright, pup, pfocal);
        // A convex solid only shadows itself on faces turned away from the projector.
        if (facing > 0.0 && pj.z > 0.0) {
            let shade = mix(1.0, facing, P(16));
            light = input(pj.xy).rgb * shade;
        }
        // Unlit surfaces keep a hint of form so the object reads in the dark.
        let base = vec3<f32>(ambient) * (0.6 + 0.4 * max(dot(normal, normalize(vec3<f32>(-0.4, 0.6, -1.0))), 0.0));
        return vec4<f32>(min(base + light, vec3<f32>(1.0)), 1.0);
    }

    if (wall_on && t_wall > 0.0) {
        let x = ro + rd * t_wall;
        let pj = to_projector(x, ppos, pfwd, pright, pup, pfocal);
        var light = vec3<f32>(0.0);
        if (pj.z > 0.0) {
            // Shadow: does the object block the projector's beam on its way to this point?
            let dir = x - ppos;
            let dist = length(dir);
            let blocker = hit_solid(mt * ppos, mt * (dir / dist), shape, n, r, h);
            if (!(blocker.ok && blocker.t < dist)) {
                light = input(pj.xy).rgb * 0.85;
            }
        }
        return vec4<f32>(min(vec3<f32>(ambient * 0.5) + light, vec3<f32>(1.0)), 1.0);
    }

    switch background {
        case 1: { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
        default: { return vec4<f32>(0.0); }
    }
}
