//! Models made from maths rather than files: parametric surfaces (torus knots, a Klein bottle,
//! a seashell, ...), the Platonic solids and two fractals. They cost nothing in the binary.

use std::collections::HashSet;
use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use super::{Builder, Corner, Model};

/// Ids the library's `procedural` entries can use, in no particular order.
pub const IDS: &[&str] = &[
    "sphere",
    "torus",
    "trefoil",
    "torus-knot",
    "klein-bottle",
    "mobius-strip",
    "seashell",
    "supershape",
    "tetrahedron",
    "cube",
    "octahedron",
    "dodecahedron",
    "icosahedron",
    "geodesic-sphere",
    "menger-sponge",
    "sierpinski-pyramid",
];

pub fn build(id: &str, name: &str) -> Result<Model, String> {
    let b = match id {
        "sphere" => parametric(96, 48, |u, v| {
            let (t, p) = (u * TAU, v * PI);
            Vec3::new(p.sin() * t.cos(), p.cos(), p.sin() * t.sin())
        }),
        "torus" => parametric(96, 48, |u, v| stand(torus_point(u * TAU, v * TAU, 1.0, 0.42))),
        "trefoil" => knot(2, 3, 0.33),
        "torus-knot" => knot(3, 7, 0.2),
        "klein-bottle" => parametric(128, 48, |u, v| {
            // The "bottle" immersion (Wikipedia), u along the body, v around it.
            let (u, v) = (u * PI, v * TAU);
            let (cu, su, cv, sv) = (u.cos(), u.sin(), v.cos(), v.sin());
            let x = -2.0 / 15.0 * cu * (3.0 * cv - 30.0 * su + 90.0 * cu.powi(4) * su - 60.0 * cu.powi(6) * su + 5.0 * cu * cv * su);
            let y = -1.0 / 15.0
                * su
                * (3.0 * cv - 3.0 * cu.powi(2) * cv - 48.0 * cu.powi(4) * cv + 48.0 * cu.powi(6) * cv - 60.0 * su + 5.0 * cu * cv * su
                    - 5.0 * cu.powi(3) * cv * su
                    - 80.0 * cu.powi(5) * cv * su
                    + 80.0 * cu.powi(7) * cv * su);
            let z = 2.0 / 15.0 * (3.0 + 5.0 * cu * su) * sv;
            // Stand it up: the neck along +y.
            Vec3::new(y, -x, z)
        }),
        "mobius-strip" => parametric(160, 12, |u, v| {
            let (t, w) = (u * TAU, (v - 0.5) * 0.9);
            let r = 1.0 + w * (t / 2.0).cos();
            stand(Vec3::new(r * t.cos(), w * (t / 2.0).sin(), r * t.sin()))
        }),
        "seashell" => parametric(160, 48, |u, v| {
            // Paul Bourke's seashell: a logarithmic spiral of growing circles.
            let (u, v) = (u * 6.0 * PI, v * TAU);
            let e = (u / (6.0 * PI)).exp();
            let c = (v / 2.0).cos().powi(2);
            Vec3::new(2.0 * (1.0 - e) * u.cos() * c, 1.0 - (u / (3.0 * PI)).exp() - v.sin() + e * v.sin(), 2.0 * (-1.0 + e) * u.sin() * c)
        }),
        "supershape" => parametric(128, 64, |u, v| {
            // Gielis' superformula, crossed with itself: a seven-armed star-blob.
            let (t, p) = (u * TAU - PI, v * PI - PI / 2.0);
            let r1 = superformula(t, 7.0, 0.2, 1.7, 1.7);
            let r2 = superformula(p, 7.0, 0.2, 1.7, 1.7);
            Vec3::new(r1 * t.cos() * r2 * p.cos(), r2 * p.sin(), r1 * t.sin() * r2 * p.cos())
        }),
        "tetrahedron" => solid(&tetrahedron()),
        "cube" => solid(&cube()),
        "octahedron" => solid(&octahedron()),
        "dodecahedron" => solid(&dual(&icosahedron())),
        "icosahedron" => solid(&icosahedron()),
        "geodesic-sphere" => geodesic(3),
        "menger-sponge" => menger(3),
        "sierpinski-pyramid" => sierpinski(5),
        _ => return Err(format!("no built-in shape {id:?}")),
    };
    b.build(name, "generated")
}

/// Turn a shape built around the y axis to face the camera (around z).
fn stand(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, -p.y)
}

fn torus_point(t: f32, p: f32, big: f32, small: f32) -> Vec3 {
    Vec3::new((big + small * p.cos()) * t.cos(), small * p.sin(), (big + small * p.cos()) * t.sin())
}

fn superformula(a: f32, m: f32, n1: f32, n2: f32, n3: f32) -> f32 {
    let t1 = (m * a / 4.0).cos().abs().powf(n2);
    let t2 = (m * a / 4.0).sin().abs().powf(n3);
    (t1 + t2).powf(-1.0 / n1)
}

/// A grid of `nu` × `nv` quads over `f(u, v)` (u, v in 0..1), with texture coordinates.
fn parametric(nu: usize, nv: usize, f: impl Fn(f32, f32) -> Vec3) -> Builder {
    let mut b = Builder::new();
    let at = |i: usize, j: usize| {
        let (u, v) = (i as f32 / nu as f32, j as f32 / nv as f32);
        Corner { uv: Some(Vec2::new(u, v)), ..Corner::at(f(u, v)) }
    };
    for i in 0..nu {
        for j in 0..nv {
            let _ = b.polygon(&[at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    b
}

/// A tube around the (p, q) torus knot.
fn knot(p: i32, q: i32, tube: f32) -> Builder {
    let curve = |t: f32| {
        let r = 2.0 + (q as f32 * t).cos();
        Vec3::new(r * (p as f32 * t).cos(), -(q as f32 * t).sin(), r * (p as f32 * t).sin())
    };
    parametric(64 * (p + q) as usize, 24, |u, v| {
        let t = u * TAU;
        let c = curve(t);
        let tangent = (curve(t + 1e-3) - curve(t - 1e-3)).normalize();
        // A frame that turns with the curve: towards the knot's axis, and the binormal.
        let side = tangent.cross(Vec3::Y).normalize();
        let up = side.cross(tangent);
        let a = v * TAU;
        stand(c + (side * a.cos() + up * a.sin()) * tube * 2.0)
    })
}

/// A polyhedron: vertices and faces (each a list of vertex indices).
struct Poly {
    v: Vec<Vec3>,
    f: Vec<Vec<usize>>,
}

/// Flat faces, each with the whole image on it (like the Shape projector's solids).
fn solid(p: &Poly) -> Builder {
    let mut b = Builder::flat();
    for face in &p.f {
        let pts: Vec<Vec3> = face.iter().map(|&i| p.v[i]).collect();
        let center = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
        let n = (pts[1] - pts[0]).cross(pts[2] - pts[0]).normalize();
        // Make every face point outwards.
        let (pts, n) = if n.dot(center) < 0.0 { (pts.into_iter().rev().collect::<Vec<_>>(), -n) } else { (pts, n) };
        let ax = (pts[0] - center).normalize();
        let ay = n.cross(ax);
        let r = pts.iter().map(|q| (*q - center).length()).fold(0.0, f32::max);
        let corners: Vec<Corner> = pts
            .iter()
            .map(|q| {
                let d = *q - center;
                Corner { uv: Some(Vec2::new(0.5 + d.dot(ax) / (2.0 * r), 0.5 - d.dot(ay) / (2.0 * r))), ..Corner::at(*q) }
            })
            .collect();
        let _ = b.polygon(&corners);
    }
    b
}

fn tetrahedron() -> Poly {
    let v = vec![Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, -1.0), Vec3::new(-1.0, 1.0, -1.0), Vec3::new(-1.0, -1.0, 1.0)];
    Poly { v, f: vec![vec![0, 1, 2], vec![0, 3, 1], vec![0, 2, 3], vec![1, 3, 2]] }
}

fn cube() -> Poly {
    let v = (0..8).map(|i| Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 })).collect();
    Poly { v, f: vec![vec![0, 1, 3, 2], vec![4, 6, 7, 5], vec![0, 4, 5, 1], vec![2, 3, 7, 6], vec![0, 2, 6, 4], vec![1, 5, 7, 3]] }
}

fn octahedron() -> Poly {
    let v = vec![Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
    let f = vec![vec![0, 2, 4], vec![4, 2, 1], vec![1, 2, 5], vec![5, 2, 0], vec![0, 4, 3], vec![4, 1, 3], vec![1, 5, 3], vec![5, 0, 3]];
    Poly { v, f }
}

fn icosahedron() -> Poly {
    let g = (1.0 + 5f32.sqrt()) / 2.0;
    let v = vec![
        Vec3::new(-1.0, g, 0.0),
        Vec3::new(1.0, g, 0.0),
        Vec3::new(-1.0, -g, 0.0),
        Vec3::new(1.0, -g, 0.0),
        Vec3::new(0.0, -1.0, g),
        Vec3::new(0.0, 1.0, g),
        Vec3::new(0.0, -1.0, -g),
        Vec3::new(0.0, 1.0, -g),
        Vec3::new(g, 0.0, -1.0),
        Vec3::new(g, 0.0, 1.0),
        Vec3::new(-g, 0.0, -1.0),
        Vec3::new(-g, 0.0, 1.0),
    ];
    let f = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    Poly { v, f: f.iter().map(|t| t.to_vec()).collect() }
}

/// The dual polyhedron: a vertex per face (its centroid), a face per vertex (the faces around
/// it, in order). The dodecahedron is the icosahedron's dual.
fn dual(p: &Poly) -> Poly {
    let v: Vec<Vec3> = p.f.iter().map(|f| f.iter().map(|&i| p.v[i]).sum::<Vec3>() / f.len() as f32).collect();
    let f = (0..p.v.len())
        .map(|vi| {
            let n = p.v[vi].normalize();
            let mut around: Vec<usize> = (0..p.f.len()).filter(|&fi| p.f[fi].contains(&vi)).collect();
            // Order by angle around the vertex.
            let ax = (v[around[0]] - p.v[vi]).normalize();
            let ay = n.cross(ax);
            around.sort_by(|&a, &b| {
                let ang = |i: usize| (v[i] - p.v[vi]).dot(ay).atan2((v[i] - p.v[vi]).dot(ax));
                ang(a).total_cmp(&ang(b))
            });
            around
        })
        .collect();
    Poly { v, f }
}

/// An icosahedron with each face split `levels` times, pushed onto the sphere; flat-shaded so
/// the facets show.
fn geodesic(levels: u32) -> Builder {
    let ico = icosahedron();
    let mut tris: Vec<[Vec3; 3]> = ico.f.iter().map(|f| [ico.v[f[0]], ico.v[f[1]], ico.v[f[2]]].map(Vec3::normalize)).collect();
    for _ in 0..levels {
        tris = tris
            .into_iter()
            .flat_map(|[a, b, c]| {
                let (ab, bc, ca) = ((a + b).normalize(), (b + c).normalize(), (c + a).normalize());
                [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]
            })
            .collect();
    }
    let mut b = Builder::flat();
    for t in tris {
        let _ = b.tri(outwards(t, t[0] + t[1] + t[2]).map(Corner::at));
    }
    b
}

/// The triangle wound so its normal points along `dir`.
fn outwards(t: [Vec3; 3], dir: Vec3) -> [Vec3; 3] {
    if (t[1] - t[0]).cross(t[2] - t[0]).dot(dir) < 0.0 { [t[0], t[2], t[1]] } else { t }
}

/// Menger sponge: cubes with the middles cut out, `levels` deep. Faces between two filled
/// cubes are left out.
fn menger(levels: u32) -> Builder {
    let n = 3i32.pow(levels);
    let filled = |x: i32, y: i32, z: i32| -> bool {
        if x < 0 || y < 0 || z < 0 || x >= n || y >= n || z >= n {
            return false;
        }
        let (mut x, mut y, mut z) = (x, y, z);
        for _ in 0..levels {
            if [x % 3 == 1, y % 3 == 1, z % 3 == 1].iter().filter(|m| **m).count() >= 2 {
                return false;
            }
            x /= 3;
            y /= 3;
            z /= 3;
        }
        true
    };
    let mut cells = HashSet::new();
    for x in 0..n {
        for y in 0..n {
            for z in 0..n {
                if filled(x, y, z) {
                    cells.insert((x, y, z));
                }
            }
        }
    }
    let mut b = Builder::flat();
    let s = 2.0 / n as f32;
    let dirs = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];
    for &(x, y, z) in &cells {
        let lo = Vec3::new(x as f32, y as f32, z as f32) * s - Vec3::ONE;
        for (dx, dy, dz) in dirs {
            if cells.contains(&(x + dx, y + dy, z + dz)) {
                continue;
            }
            let normal = Vec3::new(dx as f32, dy as f32, dz as f32);
            // Two axes across the face, and the face's corner offset.
            let (a, c) = match (dx, dy) {
                (0, 0) => (Vec3::X, Vec3::Y),
                (0, _) => (Vec3::Z, Vec3::X),
                _ => (Vec3::Y, Vec3::Z),
            };
            let base = lo + (normal.max(Vec3::ZERO)) * s;
            let q = [base, base + a * s, base + (a + c) * s, base + c * s];
            let q = if (a.cross(c)).dot(normal) < 0.0 { [q[0], q[3], q[2], q[1]] } else { q };
            // Texture: the whole sponge face is one image, like a Rubik's cube.
            let uv = |p: Vec3| Vec2::new((p.dot(a) + 1.0) / 2.0, 1.0 - (p.dot(c) + 1.0) / 2.0);
            let corners = q.map(|p| Corner { uv: Some(uv(p)), ..Corner::at(p) });
            let _ = b.polygon(&corners);
        }
    }
    b
}

/// Sierpinski tetrahedron, `levels` deep.
fn sierpinski(levels: u32) -> Builder {
    let t = tetrahedron();
    let mut tets: Vec<[Vec3; 4]> = vec![[t.v[0], t.v[1], t.v[2], t.v[3]]];
    for _ in 0..levels {
        tets = tets.into_iter().flat_map(|q| (0..4).map(move |i| q.map(|p| (p + q[i]) * 0.5))).collect();
    }
    let mut b = Builder::flat();
    for q in tets {
        for f in &t.f {
            let _ = b.tri([q[f[0]], q[f[1]], q[f[2]]].map(Corner::at));
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_builds() {
        for id in IDS {
            let m = build(id, id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(m.triangles() >= 4, "{id}");
            assert!(m.vertices.iter().all(|v| v.pos.iter().chain(&v.normal).all(|x| x.is_finite())), "{id} has NaNs");
        }
        assert!(build("nope", "x").is_err());
    }

    #[test]
    fn polyhedra_have_the_right_faces() {
        let tris = |id: &str| build(id, id).unwrap().triangles();
        assert_eq!(tris("tetrahedron"), 4);
        assert_eq!(tris("cube"), 12);
        assert_eq!(tris("octahedron"), 8);
        assert_eq!(tris("dodecahedron"), 12 * 3);
        assert_eq!(tris("icosahedron"), 20);
        assert_eq!(tris("geodesic-sphere"), 20 * 64);
        assert_eq!(tris("sierpinski-pyramid"), 4 * 4usize.pow(5));
    }

    #[test]
    fn solid_faces_point_outwards() {
        for id in ["tetrahedron", "cube", "octahedron", "dodecahedron", "icosahedron"] {
            let m = build(id, id).unwrap();
            for t in m.vertices.chunks(3) {
                let c = t.iter().fold(Vec3::ZERO, |s, v| s + Vec3::from(v.pos));
                assert!(Vec3::from(t[0].face).dot(c) > 0.0, "{id}: inward face");
            }
        }
    }
}
