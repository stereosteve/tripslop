// Rasterized 3D models (see meshes.rs): a model clip with its own materials, the Shape
// projector's input wrapped onto a model, and Projection mapping's projector throwing the input
// at a model (with a shadow map). common.wgsl is prepended.

struct MeshU {
    view_proj: mat4x4<f32>,
    // Object -> world (rotation and scale); normals use `normal_mat`.
    model: mat4x4<f32>,
    normal_mat: mat4x4<f32>,
    // Projection mapping: world -> the projector's clip space (also renders the shadow map).
    proj_vp: mat4x4<f32>,
    a: vec4<f32>, // mode (0 clip, 1 shape projector, 2 projection mapping), material, hue, lighting
    b: vec4<f32>, // time, beat, wire amount, wire width (px)
    c: vec4<f32>, // explode, twist, wobble, flat shading
    d: vec4<f32>, // mapping, model flags (1 texture, 2 UVs, 4 colours), ambient, surface shading
    e: vec4<f32>, // camera position xyz, 1 = this draw is the projection wall
    f: vec4<f32>, // projector position xyz, input aspect
    g: vec4<f32>, // shape projector: 1 = opaque faces (black background), unused...
};

@group(0) @binding(0) var<uniform> u: MeshU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var input_tex: texture_2d<f32>;
@group(0) @binding(3) var model_tex: texture_2d<f32>;
@group(0) @binding(4) var shadow_map: texture_depth_2d;
@group(0) @binding(5) var shadow_samp: sampler_comparison;
// Repeats: model texture coordinates often run past 0..1.
@group(0) @binding(6) var model_samp: sampler;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) face: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) flags: u32,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) face: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    // Object space after deforming, before scaling: the mappings' coordinates.
    @location(5) obj: vec3<f32>,
    @location(6) obj_n: vec3<f32>,
    @location(7) bary: vec3<f32>,
    @location(8) @interpolate(flat) flags: u32,
};

fn turn_y(p: vec3<f32>, a: f32) -> vec3<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec3<f32>(c * p.x + s * p.z, p.y, -s * p.x + c * p.z);
}

struct Deformed {
    p: vec3<f32>,
    n: vec3<f32>,
    f: vec3<f32>,
};

// Explode (triangles fly out along their normals), twist (turned about y by height) and
// wobble (a breathing ripple along the normals), in object space.
fn deform(v: VIn) -> Deformed {
    var d: Deformed;
    d.p = v.pos + v.face * u.c.x * 0.5;
    let twist = u.c.y * PI * d.p.y;
    d.p = turn_y(d.p, twist);
    d.n = turn_y(v.normal, twist);
    d.f = turn_y(v.face, twist);
    let t = u.b.x;
    let w = sin(d.p.x * 5.0 + t * 2.0) * sin(d.p.y * 4.0 - t * 1.7) * sin(d.p.z * 6.0 + t * 1.3);
    d.p = d.p + d.n * w * u.c.z * 0.25;
    return d;
}

@vertex
fn vs_mesh(v: VIn, @builtin(vertex_index) vi: u32) -> VOut {
    var o: VOut;
    let wall = u.e.w > 0.5;
    var world: vec3<f32>;
    if (wall) {
        // A unit quad placed by `model`, never deformed.
        world = (u.model * vec4<f32>(v.pos, 1.0)).xyz;
        o.normal = v.normal;
        o.face = v.face;
        o.obj = v.pos;
        o.obj_n = v.normal;
    } else {
        let d = deform(v);
        world = (u.model * vec4<f32>(d.p, 1.0)).xyz;
        o.normal = (u.normal_mat * vec4<f32>(d.n, 0.0)).xyz;
        o.face = (u.normal_mat * vec4<f32>(d.f, 0.0)).xyz;
        o.obj = d.p;
        o.obj_n = d.n;
    }
    o.clip = u.view_proj * vec4<f32>(world, 1.0);
    o.world = world;
    o.uv = v.uv;
    o.color = v.color;
    o.flags = v.flags;
    let k = vi % 3u;
    o.bary = vec3<f32>(f32(k == 0u), f32(k == 1u), f32(k == 2u));
    return o;
}

// Depth only, from the projector (Projection mapping's shadow map).
@vertex
fn vs_shadow(v: VIn) -> @builtin(position) vec4<f32> {
    let d = deform(v);
    let world = (u.model * vec4<f32>(d.p, 1.0)).xyz;
    return u.proj_vp * vec4<f32>(world, 1.0);
}

fn input(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(input_tex, samp, uv, 0.0);
}

// The input wrapped onto the object (Shape projector), by the mapping parameter.
fn mapped(p: vec3<f32>, n: vec3<f32>, uv: vec2<f32>) -> vec4<f32> {
    var m = i32(u.d.x);
    let has_uvs = (i32(u.d.y) & 2) != 0;
    if (m == 0) {
        // Auto: the model's own UVs if it has them, else a box.
        m = select(4, 1, has_uvs);
    }
    let r = max(length(p), 1e-4);
    switch m {
        case 1: { return input(fract(uv)); }
        case 2: { return input(vec2<f32>(atan2(p.x, -p.z) / TAU + 0.5, 0.5 - p.y * 0.5)); }
        case 3: { return input(vec2<f32>(atan2(p.x, -p.z) / TAU + 0.5, acos(clamp(p.y / r, -1.0, 1.0)) / PI)); }
        case 5: {
            let a = u.f.w;
            return input(clamp(vec2<f32>(0.5 + p.x * 0.5 / a, 0.5 - p.y * 0.5), vec2<f32>(0.0), vec2<f32>(1.0)));
        }
        default: {
            // Box (triplanar): one planar projection per axis, blended by the normal.
            var w = pow(abs(n), vec3<f32>(4.0));
            w = w / max(w.x + w.y + w.z, 1e-4);
            let q = p * 0.5 + 0.5;
            let x = input(vec2<f32>(select(1.0 - q.z, q.z, n.x < 0.0), 1.0 - q.y));
            let y = input(vec2<f32>(q.x, select(1.0 - q.z, q.z, n.y < 0.0)));
            let z = input(vec2<f32>(select(q.x, 1.0 - q.x, n.z > 0.0), 1.0 - q.y));
            return x * w.x + y * w.y + z * w.z;
        }
    }
}

// A studio for the Chrome material: a bright sky with soft bands over a dark floor.
fn environment(r: vec3<f32>, hue: f32) -> vec3<f32> {
    let sky = mix(hsv2rgb(vec3<f32>(hue + 0.55, 0.45, 0.9)), vec3<f32>(1.0), smoothstep(0.0, 0.9, r.y));
    let floor = hsv2rgb(vec3<f32>(hue + 0.05, 0.6, 0.18)) * (1.0 + 0.5 * r.y);
    var c = mix(floor, sky, smoothstep(-0.05, 0.05, r.y));
    // Softbox stripes and a horizon glow.
    c = c + vec3<f32>(1.0) * smoothstep(0.93, 0.99, sin(atan2(r.x, r.z) * 3.0) * 0.5 + 0.5) * smoothstep(0.1, 0.5, r.y);
    c = c + hsv2rgb(vec3<f32>(hue, 0.8, 1.0)) * 0.8 * exp(-abs(r.y) * 14.0);
    return c;
}

@fragment
fn fs_mesh(in: VOut) -> @location(0) vec4<f32> {
    // Derivatives first, while control flow is uniform.
    let fw = fwidth(in.bary);
    let edge3 = smoothstep(vec3<f32>(0.0), fw * max(u.b.w, 0.3), in.bary);
    let edge = 1.0 - min(min(edge3.x, edge3.y), edge3.z);

    let view = normalize(u.e.xyz - in.world);
    var n = normalize(select(in.normal, in.face, u.c.w > 0.5));
    var on = normalize(in.obj_n);
    // Two-sided: the side the camera sees faces the camera.
    if (dot(n, view) < 0.0) {
        n = -n;
        on = -on;
    }
    let mode = i32(u.a.x);
    let hue = u.a.z;
    let lighting = u.a.w;

    if (mode == 1) {
        // Shape projector: the input on the surface, lit like the built-in solids.
        var c = mapped(in.obj, on, in.uv);
        let lambert = 0.3 + 0.7 * max(dot(n, normalize(vec3<f32>(-0.4, 0.5, -1.0))), 0.0);
        let shade = mix(1.0, lambert, lighting);
        c = vec4<f32>(c.rgb * shade, c.a);
        if (u.g.x > 0.5) {
            // Keep a solid face where the input is transparent, so the shape reads.
            return vec4<f32>(c.rgb + vec3<f32>(0.04) * shade * (1.0 - c.a), 1.0);
        }
        return c;
    }

    if (mode == 2) {
        // Projection mapping: where does this point land in the projector's image, and does
        // the beam reach it (shadow map)?
        let pj = u.proj_vp * vec4<f32>(in.world, 1.0);
        let ndc = pj.xyz / max(pj.w, 1e-5);
        let puv = vec2<f32>(0.5 + 0.5 * ndc.x, 0.5 - 0.5 * ndc.y);
        let inside = pj.w > 0.0 && all(puv >= vec2<f32>(0.0)) && all(puv <= vec2<f32>(1.0));
        let lit = textureSampleCompareLevel(shadow_map, shadow_samp, clamp(puv, vec2<f32>(0.0), vec2<f32>(1.0)), ndc.z - 0.002);
        let to_proj = normalize(u.f.xyz - in.world);
        let facing = dot(n, to_proj);
        let ambient = u.d.z;
        if (u.e.w > 0.5) {
            var light = vec3<f32>(0.0);
            if (inside) {
                light = input(puv).rgb * 0.85 * lit;
            }
            return vec4<f32>(min(vec3<f32>(ambient * 0.5) + light, vec3<f32>(1.0)), 1.0);
        }
        var light = vec3<f32>(0.0);
        if (inside && facing > 0.0) {
            light = input(puv).rgb * mix(1.0, facing, u.d.w) * lit;
        }
        // Unlit surfaces keep a hint of form so the object reads in the dark.
        let base = vec3<f32>(ambient) * (0.6 + 0.4 * max(dot(n, normalize(vec3<f32>(-0.4, 0.6, -1.0))), 0.0));
        return vec4<f32>(min(base + light, vec3<f32>(1.0)), 1.0);
    }

    // Model clip materials.
    let material = i32(u.a.y);
    let key = normalize(vec3<f32>(-0.5, 0.7, -0.6));
    let fill = normalize(vec3<f32>(0.7, 0.2, -0.4));
    let ndv = clamp(dot(n, view), 0.0, 1.0);
    let wire_color = hsv2rgb(vec3<f32>(hue + 0.5, 0.6, 1.0));
    var col: vec3<f32>;
    switch material {
        case 1: {
            // Normals: the surface direction as colour.
            col = hue_rotate(n * 0.5 + 0.5, hue);
        }
        case 2: {
            // Chrome: a mirror of a studio, tinted by the hue.
            let r = reflect(-view, n);
            col = environment(r, hue) * mix(vec3<f32>(1.0), hsv2rgb(vec3<f32>(hue, 0.25, 1.0)), 0.6);
            col = col + vec3<f32>(0.25) * pow(1.0 - ndv, 4.0);
        }
        case 3: {
            // Toon: three flat bands and a dark outline at grazing angles.
            let l = max(dot(n, key), 0.0);
            let band = select(select(0.35, 0.7, l > 0.15), 1.0, l > 0.6);
            col = hsv2rgb(vec3<f32>(hue, 0.65, 1.0)) * band;
            col = col * smoothstep(0.18, 0.26, ndv);
        }
        case 4: {
            // Hologram: glowing rim, scanlines, faint body (drawn additively, no depth).
            let rim = pow(1.0 - ndv, 2.0);
            let scan = 0.6 + 0.4 * sin(in.world.y * 140.0 - u.b.x * 6.0);
            col = hsv2rgb(vec3<f32>(hue + 0.5, 0.7, 1.0)) * (0.04 + 1.1 * rim) * scan + wire_color * edge * 0.35;
            return vec4<f32>(col, max3(col));
        }
        case 5: {
            // Wireframe: just the triangles' edges, glowing (additive, no depth).
            let depth = clamp(0.5 + 0.5 * dot(in.world, normalize(u.e.xyz)), 0.0, 1.0);
            col = hsv2rgb(vec3<f32>(hue + 0.6 + depth * 0.15, 0.7, 0.35 + 0.65 * depth)) * edge;
            return vec4<f32>(col, max3(col));
        }
        default: {
            // Surface: the model's texture or colours (clay when it has neither), lit by a
            // key, a fill, a specular highlight and a rim.
            let flags = i32(u.d.y);
            var base = in.color.rgb;
            if ((in.flags & 1u) != 0u && (flags & 1) != 0) {
                base = base * textureSampleLevel(model_tex, model_samp, in.uv, 0.0).rgb;
            }
            if ((flags & 5) == 0) {
                base = hsv2rgb(vec3<f32>(hue, 0.3, 0.92));
            } else {
                base = hue_rotate(base, hue);
            }
            let h = normalize(key + view);
            let lit = base * (0.22 + 0.78 * max(dot(n, key), 0.0) + 0.25 * max(dot(n, fill), 0.0))
                + vec3<f32>(0.35) * pow(max(dot(n, h), 0.0), 40.0)
                + vec3<f32>(0.18) * pow(1.0 - ndv, 3.0);
            col = mix(base, lit, lighting);
        }
    }
    col = mix(col, wire_color, edge * u.b.z);
    return vec4<f32>(min(col, vec3<f32>(1.0)), 1.0);
}
