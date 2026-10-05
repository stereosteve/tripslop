//! 3D models: triangle meshes loaded from files (`formats.rs`: OBJ, STL, PLY, glTF / GLB, OFF)
//! or generated (`shapes.rs`: the library's Math and Platonic shapes), ready for the GPU. They
//! play as clips, and the Shape projector and Projection mapping effects can use them as their
//! object. `meshes.rs` draws them.
//!
//! Every model is stored the same way, whatever it came from: a triangle soup (three vertices
//! per triangle, so each corner can carry its own normal and texture coordinate), centered and
//! scaled to fit inside the unit sphere, y up. The effects size it from there.

pub mod formats;
pub mod shapes;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use glam::{Mat3, Vec2, Vec3};

/// One triangle corner as the GPU sees it (see `shaders/mesh.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// The triangle's own normal (flat shading, explode).
    pub face: [f32; 3],
    /// Texture coordinates with the image's first row at v = 0.
    pub uv: [f32; 2],
    /// Vertex or material colour, straight alpha.
    pub color: [u8; 4],
    /// `TEXTURED`: sample the model's texture.
    pub flags: u32,
}

pub const TEXTURED: u32 = 1;
/// Bigger meshes are refused (each triangle costs ~160 bytes of GPU memory).
pub const MAX_TRIANGLES: usize = 1_500_000;
const MAX_TEXTURE: u32 = 4096;
/// Without normals in the file, faces meeting at a sharper angle than this get a hard edge.
const CREASE_DEGREES: f32 = 60.0;

pub struct Model {
    /// Identity for the GPU cache.
    pub id: u64,
    pub name: String,
    /// Three per triangle.
    pub vertices: Vec<Vertex>,
    /// Base colour texture (straight alpha), for the corners flagged `TEXTURED`.
    pub texture: Option<image::RgbaImage>,
    pub has_uvs: bool,
    pub has_colors: bool,
    /// What it was read from ("OBJ", "glTF", "generated", ...).
    pub format: &'static str,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Model({:?}, {})", self.name, self.summary())
    }
}

impl Model {
    pub fn triangles(&self) -> usize {
        self.vertices.len() / 3
    }

    /// "OBJ · 6,320 triangles · 1024×1024 texture"
    pub fn summary(&self) -> String {
        let mut s = format!("{} · {} triangles", self.format, thousands(self.triangles()));
        if let Some(t) = &self.texture {
            s += &format!(" · {}×{} texture", t.width(), t.height());
        } else if self.has_colors {
            s += " · vertex colours";
        }
        s
    }

    /// Turn the model (vertices and normals) by `degrees` about x, y and z (applied in that
    /// order). For bundled models that come lying down.
    pub fn rotate(&mut self, degrees: [f32; 3]) {
        let [x, y, z] = degrees.map(f32::to_radians);
        let m = Mat3::from_rotation_z(z) * Mat3::from_rotation_y(y) * Mat3::from_rotation_x(x);
        for v in &mut self.vertices {
            v.pos = (m * Vec3::from(v.pos)).into();
            v.normal = (m * Vec3::from(v.normal)).into();
            v.face = (m * Vec3::from(v.face)).into();
        }
    }
}

/// A model as a clip or an effect holds it: shared, with the library key (or file path) it
/// came from.
#[derive(Clone, Debug)]
pub struct ModelRef {
    pub key: String,
    pub model: Arc<Model>,
}

impl ModelRef {
    pub fn name(&self) -> &str {
        &self.model.name
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The model the Shape projector and Projection mapping show when *Model* is picked before
/// any model has been chosen: the Utah teapot.
pub fn fallback() -> Arc<Model> {
    static TEAPOT: OnceLock<Arc<Model>> = OnceLock::new();
    TEAPOT
        .get_or_init(|| Arc::new(formats::load_bundled("teapot.obj", "Utah Teapot").expect("the bundled teapot loads")))
        .clone()
}

/// A triangle corner while a model is being built.
#[derive(Clone, Copy, Debug)]
pub struct Corner {
    pub pos: Vec3,
    pub normal: Option<Vec3>,
    pub uv: Option<Vec2>,
    pub color: [u8; 4],
    /// Index into `Builder::textures`.
    pub texture: Option<usize>,
}

impl Corner {
    pub fn at(pos: Vec3) -> Self {
        Self { pos, normal: None, uv: None, color: [255; 4], texture: None }
    }
}

/// Collects triangles from a loader, then fills in what the file left out.
pub struct Builder {
    tris: Vec<[Corner; 3]>,
    pub textures: Vec<image::RgbaImage>,
    /// Every face gets its own normal (hard edges everywhere).
    flat: bool,
}

impl Builder {
    pub fn new() -> Self {
        Self { tris: Vec::new(), textures: Vec::new(), flat: false }
    }

    pub fn flat() -> Self {
        Self { flat: true, ..Self::new() }
    }

    pub fn tri(&mut self, c: [Corner; 3]) -> Result<(), String> {
        if self.tris.len() >= MAX_TRIANGLES {
            return Err(format!("more than {} triangles; simplify the model first", thousands(MAX_TRIANGLES)));
        }
        self.tris.push(c);
        Ok(())
    }

    /// A polygon as a fan of triangles (convex polygons, which is what files have).
    pub fn polygon(&mut self, corners: &[Corner]) -> Result<(), String> {
        for i in 1..corners.len().saturating_sub(1) {
            self.tri([corners[0], corners[i], corners[i + 1]])?;
        }
        Ok(())
    }

    pub fn add_texture(&mut self, img: image::RgbaImage) -> usize {
        self.textures.push(img);
        self.textures.len() - 1
    }

    pub fn build(self, name: &str, format: &'static str) -> Result<Model, String> {
        let Builder { tris, mut textures, flat } = self;
        // Face normals (area-weighted, unnormalized); degenerate triangles are dropped.
        let mut faces = Vec::with_capacity(tris.len());
        let mut kept = Vec::with_capacity(tris.len());
        for t in tris {
            let n = (t[1].pos - t[0].pos).cross(t[2].pos - t[0].pos);
            if n.length_squared() > 1e-24 && n.is_finite() {
                faces.push(n);
                kept.push(t);
            }
        }
        let tris = kept;
        if tris.is_empty() {
            return Err("no triangles in the file".into());
        }

        // Corners without a normal get the average of the faces around their position that
        // meet this face at less than the crease angle.
        let needs_normals = !flat && tris.iter().flatten().any(|c| c.normal.is_none());
        let mut around: HashMap<[u32; 3], Vec<u32>> = HashMap::new();
        if needs_normals {
            for (i, t) in tris.iter().enumerate() {
                for c in t {
                    around.entry(c.pos.to_array().map(f32::to_bits)).or_default().push(i as u32);
                }
            }
        }
        let crease = CREASE_DEGREES.to_radians().cos();

        // The texture most triangles use is the model's texture; triangles with any other
        // texture get that texture's average colour instead.
        let mut uses = vec![0usize; textures.len()];
        for t in &tris {
            if let Some(i) = t[0].texture {
                uses[i] += 1;
            }
        }
        let main = (0..textures.len()).max_by_key(|i| uses[*i]).filter(|i| uses[*i] > 0);
        let average: Vec<[u8; 4]> = textures.iter().map(average_color).collect();

        let mut vertices = Vec::with_capacity(tris.len() * 3);
        let (mut has_uvs, mut has_colors) = (false, false);
        for (i, t) in tris.iter().enumerate() {
            let face = faces[i].normalize();
            for c in t {
                let normal = match c.normal.filter(|n| n.length_squared() > 1e-12 && !flat) {
                    Some(n) => n.normalize(),
                    None if flat => face,
                    None => {
                        let mut sum = Vec3::ZERO;
                        for &j in &around[&c.pos.to_array().map(f32::to_bits)] {
                            let fj = faces[j as usize];
                            if fj.normalize().dot(face) >= crease {
                                sum += fj;
                            }
                        }
                        sum.try_normalize().unwrap_or(face)
                    }
                };
                let mut color = c.color;
                let mut flags = 0;
                match (c.texture, main) {
                    (Some(tx), Some(m)) if tx == m && c.uv.is_some() => flags |= TEXTURED,
                    (Some(tx), _) => color = mul_color(color, average[tx]),
                    _ => {}
                }
                has_uvs |= c.uv.is_some();
                has_colors |= color != [255; 4];
                vertices.push(Vertex {
                    pos: c.pos.into(),
                    normal: normal.into(),
                    face: face.into(),
                    uv: c.uv.unwrap_or_default().into(),
                    color,
                    flags,
                });
            }
        }

        // Center on the bounding box and fit in the unit sphere.
        let (lo, hi) = vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| {
            (lo.min(Vec3::from(v.pos)), hi.max(Vec3::from(v.pos)))
        });
        let center = (lo + hi) * 0.5;
        let radius = vertices.iter().map(|v| (Vec3::from(v.pos) - center).length()).fold(0.0f32, f32::max).max(1e-12);
        for v in &mut vertices {
            v.pos = ((Vec3::from(v.pos) - center) / radius).into();
        }

        let texture = main.map(|m| {
            let img = std::mem::take(&mut textures[m]);
            if img.width() > MAX_TEXTURE || img.height() > MAX_TEXTURE {
                image::DynamicImage::ImageRgba8(img).resize(MAX_TEXTURE, MAX_TEXTURE, image::imageops::FilterType::Triangle).to_rgba8()
            } else {
                img
            }
        });
        Ok(Model {
            id: crate::clip::next_id(),
            name: name.to_string(),
            vertices,
            texture,
            has_uvs,
            has_colors,
            format,
        })
    }
}

fn average_color(img: &image::RgbaImage) -> [u8; 4] {
    let n = (img.width() as u64 * img.height() as u64).max(1);
    let mut sum = [0u64; 4];
    for p in img.pixels() {
        for k in 0..4 {
            sum[k] += p.0[k] as u64;
        }
    }
    sum.map(|s| (s / n) as u8)
}

pub fn mul_color(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    std::array::from_fn(|k| ((a[k] as u16 * b[k] as u16 + 127) / 255) as u8)
}

/// Linear 0..1 colour → bytes.
pub fn color_bytes(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(b: &mut Builder, pts: [Vec3; 4]) {
        b.polygon(&pts.map(Corner::at)).unwrap();
    }

    #[test]
    fn normalizes_into_the_unit_sphere() {
        let mut b = Builder::new();
        quad(&mut b, [Vec3::new(10.0, 0.0, 0.0), Vec3::new(14.0, 0.0, 0.0), Vec3::new(14.0, 2.0, 0.0), Vec3::new(10.0, 2.0, 0.0)]);
        let m = b.build("q", "test").unwrap();
        assert_eq!(m.triangles(), 2);
        let r = m.vertices.iter().map(|v| Vec3::from(v.pos).length()).fold(0.0, f32::max);
        assert!((r - 1.0).abs() < 1e-5, "radius {r}");
        let c = m.vertices.iter().fold(Vec3::ZERO, |s, v| s + Vec3::from(v.pos)) / m.vertices.len() as f32;
        assert!(c.length() < 0.5);
    }

    #[test]
    fn creases_keep_box_edges_sharp() {
        // Two faces of a box meeting at 90°: each keeps its own normal at the shared edge.
        let mut b = Builder::new();
        let (o, x, y, z) = (Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        quad(&mut b, [o, x, x + y, y]);
        quad(&mut b, [o, z, z + y, y]);
        let m = b.build("box", "test").unwrap();
        for v in &m.vertices {
            assert!(Vec3::from(v.normal).dot(Vec3::from(v.face)) > 0.999);
        }
    }

    #[test]
    fn smooths_shallow_angles() {
        // Two faces bent by 20°: the shared edge gets the averaged normal.
        let mut b = Builder::new();
        let bend = Vec3::new(1.0, 0.0, 20f32.to_radians().tan());
        quad(&mut b, [Vec3::new(-1.0, 0.0, 0.0), Vec3::ZERO, Vec3::Y, Vec3::new(-1.0, 1.0, 0.0)]);
        quad(&mut b, [Vec3::ZERO, bend, bend + Vec3::Y, Vec3::Y]);
        let m = b.build("bend", "test").unwrap();
        let at_edge: Vec<Vec3> = m.vertices.iter().filter(|v| v.pos[0].abs() < 0.3 && (v.pos[2] - m.vertices[0].pos[2]).abs() < 0.3).map(|v| Vec3::from(v.normal)).collect();
        assert!(at_edge.iter().any(|n| (n.dot(Vec3::Z).abs() - 1.0).abs() > 1e-3), "edge normals weren't averaged");
    }

    #[test]
    fn main_texture_wins_others_become_colors() {
        let mut b = Builder::new();
        let red = b.add_texture(image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255])));
        let blue = b.add_texture(image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255])));
        let tri = |x: f32, tex| {
            [Vec3::new(x, 0.0, 0.0), Vec3::new(x + 1.0, 0.0, 0.0), Vec3::new(x, 1.0, 0.0)].map(|p| Corner { uv: Some(Vec2::ZERO), texture: Some(tex), ..Corner::at(p) })
        };
        b.tri(tri(0.0, red)).unwrap();
        b.tri(tri(2.0, red)).unwrap();
        b.tri(tri(4.0, blue)).unwrap();
        let m = b.build("t", "test").unwrap();
        assert_eq!(m.texture.as_ref().unwrap().get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(m.vertices[0].flags, TEXTURED);
        assert_eq!(m.vertices[6].flags, 0);
        assert_eq!(m.vertices[6].color, [0, 0, 255, 255]);
    }

    #[test]
    fn drops_degenerate_triangles_and_refuses_empty_models() {
        let mut b = Builder::new();
        b.tri([Vec3::ZERO, Vec3::X, Vec3::X * 2.0].map(Corner::at)).unwrap();
        assert!(b.build("line", "test").is_err());
    }

    #[test]
    fn the_fallback_is_the_teapot() {
        let m = fallback();
        assert!(m.triangles() > 6000, "{}", m.summary());
        assert!(Arc::ptr_eq(&m, &fallback()));
    }
}
