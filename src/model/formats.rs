//! Reading model files: OBJ (with MTL materials and textures), STL (binary and ASCII), PLY
//! (ASCII and binary, with vertex colours and texture coordinates), glTF 2.0 (`.gltf` and
//! `.glb`, with node transforms, vertex colours and base colour textures) and OFF.
//!
//! Every loader reads through a `read` callback for the files a model refers to (an OBJ's
//! `.mtl`, its textures), so bundled models load from the binary the same way files load from
//! disk.

use std::path::Path;

use glam::{Mat3, Mat4, Vec2, Vec3};

use super::{Builder, Corner, Model, color_bytes};

mod bundle {
    include!(concat!(env!("OUT_DIR"), "/model_bundle.rs"));
}

/// File extensions the loaders know (lowercase).
pub const EXTENSIONS: &[&str] = &["obj", "stl", "ply", "gltf", "glb", "off"];

pub fn is_model(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// A file inside `assets/models`, as baked into the binary.
pub fn bundled(path: &str) -> Option<&'static [u8]> {
    bundle::FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

/// Load a bundled model by its path in `assets/models`; files it refers to are looked up next
/// to it in the bundle.
pub fn load_bundled(path: &str, name: &str) -> Result<Model, String> {
    let bytes = bundled(path).ok_or_else(|| format!("{path} isn't bundled"))?;
    let ext = path.rsplit('.').next().unwrap_or_default();
    let dir = path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
    from_bytes(name, ext, bytes, &|rel| bundled(&format!("{dir}{rel}")).map(<[u8]>::to_vec))
}

/// The bundled model files (not their textures and materials).
pub fn bundled_models() -> impl Iterator<Item = &'static str> {
    bundle::FILES.iter().map(|(p, _)| *p).filter(|p| p.rsplit('.').next().is_some_and(|e| EXTENSIONS.contains(&e)))
}

/// Load a model file from disk. Files it refers to are looked up next to it.
pub fn load(path: &Path) -> Result<Model, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let fail = |e: String| format!("{}: {e}", path.display());
    if ext == "gltf" {
        // External buffers and images are resolved by the gltf crate itself.
        let (doc, buffers, images) = gltf::import(path).map_err(|e| fail(e.to_string()))?;
        return gltf_model(&name, doc, buffers, images).map_err(fail);
    }
    let bytes = std::fs::read(path).map_err(|e| fail(e.to_string()))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    from_bytes(&name, &ext, &bytes, &|rel| std::fs::read(dir.join(rel)).ok()).map_err(fail)
}

/// Load a model from memory. `read` fetches the files it refers to, by relative path.
pub fn from_bytes(name: &str, ext: &str, bytes: &[u8], read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Model, String> {
    match ext {
        "obj" => obj(name, bytes, read),
        "stl" => stl(name, bytes),
        "ply" => ply(name, bytes, read),
        "off" => off(name, bytes),
        "glb" | "gltf" => {
            let (doc, buffers, images) = gltf::import_slice(bytes).map_err(|e| e.to_string())?;
            gltf_model(name, doc, buffers, images)
        }
        _ => Err(format!("can't read .{ext} models (try {})", EXTENSIONS.join(", "))),
    }
}

fn decode_image(bytes: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory(bytes).ok().map(|i| i.to_rgba8())
}

/// Texture paths in MTL files are often absolute or use backslashes; try the path as given,
/// then just the file name.
fn read_texture(read: &dyn Fn(&str) -> Option<Vec<u8>>, path: &str) -> Option<image::RgbaImage> {
    let path = path.trim().replace('\\', "/");
    let file = path.rsplit('/').next().unwrap_or(&path).to_string();
    read(&path).or_else(|| read(&file)).and_then(|b| decode_image(&b))
}

// ---------------------------------------------------------------------------------- OBJ

fn obj(name: &str, bytes: &[u8], read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Model, String> {
    let opts = tobj::LoadOptions { triangulate: true, ignore_points: true, ignore_lines: true, ..Default::default() };
    let (models, materials) = tobj::load_obj_buf(&mut std::io::Cursor::new(bytes), &opts, |p| {
        let mtl = read(&p.to_string_lossy()).ok_or(tobj::LoadError::OpenFileFailed)?;
        tobj::load_mtl_buf(&mut std::io::Cursor::new(mtl))
    })
    .map_err(|e| format!("not a readable OBJ file ({e})"))?;
    // A missing or broken .mtl only costs the colours.
    let materials = materials.unwrap_or_default();
    let mut b = Builder::new();
    let textures: Vec<Option<usize>> =
        materials.iter().map(|m| m.diffuse_texture.as_deref().and_then(|t| read_texture(read, t)).map(|img| b.add_texture(img))).collect();
    for m in &models {
        let mesh = &m.mesh;
        let mat = mesh.material_id.and_then(|i| materials.get(i));
        let diffuse = mat.and_then(|m| m.diffuse).unwrap_or([1.0; 3]);
        let alpha = mat.and_then(|m| m.dissolve).unwrap_or(1.0);
        let texture = mesh.material_id.and_then(|i| textures.get(i).copied().flatten());
        let corner = |k: usize| -> Corner {
            let vi = mesh.indices[k] as usize;
            let pos = Vec3::from_slice(&mesh.positions[3 * vi..3 * vi + 3]);
            // Attributes come with their own index per corner, or (when tobj reordered them)
            // one per position.
            let index = |own: &[u32], per_position: bool| match own.get(k) {
                Some(&i) => Some(i as usize),
                None => (own.is_empty() && per_position).then_some(vi),
            };
            let normal = index(&mesh.normal_indices, mesh.normals.len() == mesh.positions.len());
            let normal = normal.filter(|n| 3 * n + 3 <= mesh.normals.len()).map(|n| Vec3::from_slice(&mesh.normals[3 * n..3 * n + 3]));
            let uv = index(&mesh.texcoord_indices, mesh.texcoords.len() / 2 == mesh.positions.len() / 3);
            // OBJ's v points up; images (and tripslop) count rows down.
            let uv = uv.filter(|t| 2 * t + 2 <= mesh.texcoords.len()).map(|t| Vec2::new(mesh.texcoords[2 * t], 1.0 - mesh.texcoords[2 * t + 1]));
            let mut color = [diffuse[0], diffuse[1], diffuse[2], alpha];
            if mesh.vertex_color.len() == mesh.positions.len() {
                for (k, c) in color.iter_mut().take(3).enumerate() {
                    *c *= mesh.vertex_color[3 * vi + k];
                }
            }
            Corner { pos, normal, uv, color: color_bytes(color), texture }
        };
        for t in (0..mesh.indices.len() / 3).map(|t| 3 * t) {
            b.tri([corner(t), corner(t + 1), corner(t + 2)])?;
        }
    }
    b.build(name, "OBJ")
}

// ---------------------------------------------------------------------------------- STL

fn stl(name: &str, bytes: &[u8]) -> Result<Model, String> {
    // STL is usually z-up (3D printing): turn it y-up.
    let up = |p: Vec3| Vec3::new(p.x, p.z, -p.y);
    let mut b = Builder::new();
    let binary_len = bytes.get(80..84).map(|n| 84 + 50 * u32::from_le_bytes(n.try_into().unwrap()) as usize);
    // Some binary files start with "solid" too; the length decides.
    if binary_len == Some(bytes.len()) {
        for rec in bytes[84..].chunks_exact(50) {
            let f = |i: usize| f32::from_le_bytes(rec[4 * i..4 * i + 4].try_into().unwrap());
            let v = |k: usize| up(Vec3::new(f(3 + 3 * k), f(4 + 3 * k), f(5 + 3 * k)));
            b.tri([v(0), v(1), v(2)].map(Corner::at))?;
        }
        return b.build(name, "STL");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not an STL file".to_string())?;
    if !text.trim_start().starts_with("solid") {
        return Err("not an STL file".into());
    }
    let mut pts = Vec::with_capacity(3);
    for line in text.lines() {
        let mut w = line.split_whitespace();
        if w.next() == Some("vertex") {
            let n: Vec<f32> = w.filter_map(|x| x.parse().ok()).collect();
            if n.len() != 3 {
                return Err(format!("bad vertex line {line:?}"));
            }
            pts.push(Corner::at(up(Vec3::new(n[0], n[1], n[2]))));
            if pts.len() == 3 {
                b.tri([pts[0], pts[1], pts[2]])?;
                pts.clear();
            }
        } else if line.trim_start().starts_with("endloop") {
            pts.clear();
        }
    }
    b.build(name, "STL")
}

// ---------------------------------------------------------------------------------- PLY

#[derive(Clone, Copy, PartialEq, Debug)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "char" | "int8" => Self::I8,
            "uchar" | "uint8" => Self::U8,
            "short" | "int16" => Self::I16,
            "ushort" | "uint16" => Self::U16,
            "int" | "int32" => Self::I32,
            "uint" | "uint32" => Self::U32,
            "float" | "float32" => Self::F32,
            "double" | "float64" => Self::F64,
            _ => return None,
        })
    }
    fn size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
    /// Full scale for colours stored as integers.
    fn unit(self) -> f64 {
        match self {
            Self::U8 | Self::I8 => 255.0,
            Self::U16 | Self::I16 => 65535.0,
            _ => 1.0,
        }
    }
}

#[derive(Debug)]
struct Prop {
    name: String,
    ty: Scalar,
    /// List properties: the count's type.
    list: Option<Scalar>,
}

#[derive(Debug)]
struct Element {
    name: String,
    count: usize,
    props: Vec<Prop>,
}

/// Reads PLY values in whichever encoding the header says.
struct PlyReader<'a> {
    data: &'a [u8],
    pos: usize,
    /// None = ASCII, Some(true) = little endian.
    little: Option<bool>,
}

impl PlyReader<'_> {
    fn scalar(&mut self, ty: Scalar) -> Result<f64, String> {
        let Some(little) = self.little else {
            // ASCII: next whitespace-separated token.
            while self.pos < self.data.len() && self.data[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            let start = self.pos;
            while self.pos < self.data.len() && !self.data[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            let tok = std::str::from_utf8(&self.data[start..self.pos]).unwrap_or("");
            return tok.parse::<f64>().map_err(|_| format!("bad PLY value {tok:?}"));
        };
        let n = ty.size();
        let b = self.data.get(self.pos..self.pos + n).ok_or("the PLY file ends early")?;
        self.pos += n;
        let mut a = [0u8; 8];
        a[..n].copy_from_slice(b);
        if !little {
            a[..n].reverse();
        }
        Ok(match ty {
            Scalar::I8 => a[0] as i8 as f64,
            Scalar::U8 => a[0] as f64,
            Scalar::I16 => i16::from_le_bytes([a[0], a[1]]) as f64,
            Scalar::U16 => u16::from_le_bytes([a[0], a[1]]) as f64,
            Scalar::I32 => i32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Scalar::U32 => u32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Scalar::F32 => f32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Scalar::F64 => f64::from_le_bytes(a),
        })
    }
}

fn ply(name: &str, bytes: &[u8], read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Model, String> {
    let end = bytes.windows(10).position(|w| w == b"end_header").ok_or("not a PLY file")?;
    let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "not a PLY file")?;
    if !header.starts_with("ply") {
        return Err("not a PLY file".into());
    }
    let mut body = end + 10;
    while body < bytes.len() && bytes[body] != b'\n' {
        body += 1;
    }
    body += 1;
    let mut little = None;
    let mut elements: Vec<Element> = Vec::new();
    let mut texture_file = None;
    for line in header.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["format", "ascii", ..] => little = None,
            ["format", "binary_little_endian", ..] => little = Some(true),
            ["format", "binary_big_endian", ..] => little = Some(false),
            ["comment", "TextureFile", file, ..] => texture_file = Some(file.to_string()),
            ["element", name, count] => elements.push(Element { name: name.to_string(), count: count.parse().map_err(|_| "bad PLY element count")?, props: Vec::new() }),
            ["property", "list", count, ty, name] => {
                let e = elements.last_mut().ok_or("PLY property before any element")?;
                let (c, t) = (Scalar::parse(count), Scalar::parse(ty));
                e.props.push(Prop { name: name.to_string(), ty: t.ok_or("bad PLY type")?, list: Some(c.ok_or("bad PLY type")?) });
            }
            ["property", ty, name] => {
                let e = elements.last_mut().ok_or("PLY property before any element")?;
                e.props.push(Prop { name: name.to_string(), ty: Scalar::parse(ty).ok_or("bad PLY type")?, list: None });
            }
            _ => {}
        }
    }
    let mut r = PlyReader { data: bytes, pos: body, little };
    let mut verts: Vec<Corner> = Vec::new();
    let mut faces: Vec<(Vec<usize>, Vec<f64>)> = Vec::new();
    let mut b = Builder::new();
    let texture = texture_file.and_then(|f| read_texture(read, &f)).map(|img| b.add_texture(img));
    for e in &elements {
        for _ in 0..e.count {
            let mut scalars: Vec<(&str, f64, Scalar)> = Vec::new();
            let mut lists: Vec<(&str, Vec<f64>)> = Vec::new();
            for p in &e.props {
                match p.list {
                    Some(ct) => {
                        let n = r.scalar(ct)? as usize;
                        let vals = (0..n).map(|_| r.scalar(p.ty)).collect::<Result<Vec<_>, _>>()?;
                        lists.push((&p.name, vals));
                    }
                    None => scalars.push((&p.name, r.scalar(p.ty)?, p.ty)),
                }
            }
            let get = |names: &[&str]| scalars.iter().find(|(n, ..)| names.contains(n)).map(|(_, v, t)| (*v, *t));
            match e.name.as_str() {
                "vertex" => {
                    let f = |n: &str| get(&[n]).map_or(0.0, |(v, _)| v as f32);
                    let mut c = Corner::at(Vec3::new(f("x"), f("y"), f("z")));
                    if get(&["nx"]).is_some() {
                        c.normal = Some(Vec3::new(f("nx"), f("ny"), f("nz")));
                    }
                    if let (Some((u, _)), Some((v, _))) = (get(&["s", "u", "texture_u"]), get(&["t", "v", "texture_v"])) {
                        c.uv = Some(Vec2::new(u as f32, 1.0 - v as f32));
                        c.texture = texture;
                    }
                    let channel = |names: &[&str]| get(names).map(|(v, t)| (v / t.unit()) as f32);
                    if let (Some(red), Some(green), Some(blue)) = (channel(&["red", "r", "diffuse_red"]), channel(&["green", "g", "diffuse_green"]), channel(&["blue", "b", "diffuse_blue"])) {
                        c.color = color_bytes([red, green, blue, channel(&["alpha", "a"]).unwrap_or(1.0)]);
                    }
                    verts.push(c);
                }
                "face" => {
                    let idx = lists.iter().find(|(n, _)| *n == "vertex_indices" || *n == "vertex_index").map(|(_, v)| v.iter().map(|x| *x as usize).collect());
                    let uvs = lists.iter().find(|(n, _)| *n == "texcoord").map(|(_, v)| v.clone()).unwrap_or_default();
                    if let Some(idx) = idx {
                        faces.push((idx, uvs));
                    }
                }
                _ => {}
            }
        }
    }
    for (idx, uvs) in faces {
        let mut corners = Vec::with_capacity(idx.len());
        for (k, &i) in idx.iter().enumerate() {
            let mut c = *verts.get(i).ok_or_else(|| format!("PLY face uses vertex {i} of {}", verts.len()))?;
            if uvs.len() == 2 * idx.len() {
                c.uv = Some(Vec2::new(uvs[2 * k] as f32, 1.0 - uvs[2 * k + 1] as f32));
                c.texture = texture;
            }
            corners.push(c);
        }
        b.polygon(&corners)?;
    }
    b.build(name, "PLY")
}

// ---------------------------------------------------------------------------------- OFF

fn off(name: &str, bytes: &[u8]) -> Result<Model, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not an OFF file".to_string())?;
    let mut tokens = text.lines().map(|l| l.split('#').next().unwrap_or("")).flat_map(str::split_whitespace);
    let magic = tokens.next().ok_or("empty file")?;
    if !magic.ends_with("OFF") {
        return Err("not an OFF file".into());
    }
    let colored = magic.starts_with('C');
    let mut num = |what: &str| -> Result<f32, String> { tokens.next().and_then(|t| t.parse().ok()).ok_or_else(|| format!("OFF file: missing {what}")) };
    let (nv, nf, _) = (num("vertex count")? as usize, num("face count")? as usize, num("edge count")?);
    let mut verts = Vec::with_capacity(nv);
    for _ in 0..nv {
        let mut c = Corner::at(Vec3::new(num("x")?, num("y")?, num("z")?));
        if colored {
            let rgba = [num("red")?, num("green")?, num("blue")?, num("alpha")?];
            // Colours are 0..1 or 0..255.
            let scale = if rgba.iter().any(|v| *v > 1.0) { 255.0 } else { 1.0 };
            c.color = color_bytes(rgba.map(|v| v / scale));
        }
        verts.push(c);
    }
    let mut b = Builder::new();
    for _ in 0..nf {
        let n = num("face size")? as usize;
        let corners = (0..n)
            .map(|_| num("face index").and_then(|i| verts.get(i as usize).copied().ok_or_else(|| format!("OFF face uses vertex {i} of {nv}"))))
            .collect::<Result<Vec<_>, _>>()?;
        b.polygon(&corners)?;
    }
    b.build(name, "OFF")
}

// ---------------------------------------------------------------------------------- glTF

fn gltf_model(name: &str, doc: gltf::Document, buffers: Vec<gltf::buffer::Data>, images: Vec<gltf::image::Data>) -> Result<Model, String> {
    let mut b = Builder::new();
    // Textures are decoded on demand, once.
    let mut textures: Vec<Option<Option<usize>>> = vec![None; images.len()];
    let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or("the glTF file has no scene")?;
    // glTF models face +z; turn them round to face the camera (which looks down +z).
    let facing = Mat4::from_rotation_y(std::f32::consts::PI);
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, facing)).collect();
    while let Some((node, parent)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        stack.extend(node.children().map(|c| (c, world)));
        let Some(mesh) = node.mesh() else { continue };
        let normal_m = Mat3::from_mat4(world).inverse().transpose();
        for prim in mesh.primitives() {
            use gltf::mesh::Mode;
            if !matches!(prim.mode(), Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan) {
                continue;
            }
            let reader = prim.reader(|buf| buffers.get(buf.index()).map(|d| &d.0[..]));
            let Some(positions) = reader.read_positions() else { continue };
            let positions: Vec<Vec3> = positions.map(|p| world.transform_point3(Vec3::from(p))).collect();
            let normals: Option<Vec<Vec3>> = reader.read_normals().map(|n| n.map(|n| normal_m * Vec3::from(n)).collect());
            let uvs: Option<Vec<Vec2>> = reader.read_tex_coords(0).map(|t| t.into_f32().map(Vec2::from).collect());
            let colors: Option<Vec<[u8; 4]>> = reader.read_colors(0).map(|c| c.into_rgba_u8().collect());
            let pbr = prim.material().pbr_metallic_roughness();
            let factor = color_bytes(pbr.base_color_factor());
            let texture = match pbr.base_color_texture() {
                Some(info) => {
                    let i = info.texture().source().index();
                    *textures.get_mut(i).ok_or("glTF texture without an image")?.get_or_insert_with(|| gltf_image(&images[i]).map(|img| b.add_texture(img)))
                }
                None => None,
            };
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            let corner = |i: u32| -> Option<Corner> {
                let i = i as usize;
                let mut color = factor;
                if let Some(c) = colors.as_ref().and_then(|c| c.get(i)) {
                    color = super::mul_color(color, *c);
                }
                Some(Corner {
                    pos: *positions.get(i)?,
                    normal: normals.as_ref().and_then(|n| n.get(i).copied()),
                    uv: uvs.as_ref().and_then(|u| u.get(i).copied()),
                    color,
                    texture,
                })
            };
            let tris: Vec<[u32; 3]> = match prim.mode() {
                Mode::TriangleStrip => (2..indices.len()).map(|k| if k % 2 == 0 { [indices[k - 2], indices[k - 1], indices[k]] } else { [indices[k - 1], indices[k - 2], indices[k]] }).collect(),
                Mode::TriangleFan => (2..indices.len()).map(|k| [indices[0], indices[k - 1], indices[k]]).collect(),
                _ => indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect(),
            };
            for [i, j, k] in tris {
                if let (Some(a), Some(c1), Some(c2)) = (corner(i), corner(j), corner(k)) {
                    b.tri([a, c1, c2])?;
                }
            }
        }
    }
    b.build(name, "glTF")
}

fn gltf_image(img: &gltf::image::Data) -> Option<image::RgbaImage> {
    use gltf::image::Format;
    let px = &img.pixels;
    let rgba: Vec<u8> = match img.format {
        Format::R8G8B8A8 => px.clone(),
        Format::R8G8B8 => px.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        Format::R8G8 => px.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        Format::R8 => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        // 16-bit and float images: keep the high byte / clamp.
        Format::R16G16B16A16 => px.chunks_exact(2).map(|c| c[1]).collect(),
        Format::R16G16B16 => px.chunks_exact(6).flat_map(|c| [c[1], c[3], c[5], 255]).collect(),
        _ => return None,
    };
    image::RgbaImage::from_raw(img.width, img.height, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_files(_: &str) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn reads_every_bundled_model() {
        for path in bundled_models() {
            let m = load_bundled(path, path).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert!(m.triangles() > 400, "{path}: {}", m.summary());
            assert!(m.vertices.iter().all(|v| v.pos.iter().chain(&v.normal).all(|x| x.is_finite())), "{path} has NaNs");
            if path.contains('/') {
                // The textured CC0 characters: OBJ + MTL + PNG.
                assert!(m.texture.is_some(), "{path} lost its texture");
                assert!(m.vertices.iter().all(|v| v.flags == super::super::TEXTURED), "{path}");
            }
        }
    }

    #[test]
    fn obj_quads_materials_and_texture_coordinates() {
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl red\nf 1/1 2/2 3/3 4/4\n";
        let mtl = b"newmtl red\nKd 1 0 0\n";
        let m = from_bytes("q", "obj", obj, &|p| (p == "m.mtl").then(|| mtl.to_vec())).unwrap();
        assert_eq!(m.triangles(), 2);
        assert_eq!(m.vertices[0].color, [255, 0, 0, 255]);
        // v flipped to image rows: the corner at vt (0, 0) is at the bottom of the image.
        assert_eq!(m.vertices[0].uv, [0.0, 1.0]);
        assert!(m.has_uvs && m.texture.is_none());
        // Without its .mtl it still loads, white.
        assert_eq!(from_bytes("q", "obj", obj, &no_files).unwrap().vertices[0].color, [255; 4]);
    }

    #[test]
    fn stl_ascii_and_binary() {
        let ascii = b"solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        assert_eq!(from_bytes("t", "stl", ascii, &no_files).unwrap().triangles(), 1);
        let mut bin = vec![0u8; 80];
        bin.extend(2u32.to_le_bytes());
        for tri in [[0.0f32, 0., 0., 1., 0., 0., 0., 1., 0.], [0., 0., 0., 0., 1., 0., 0., 0., 1.]] {
            bin.extend([0u8; 12]);
            bin.extend(tri.iter().flat_map(|f| f.to_le_bytes()));
            bin.extend([0u8; 2]);
        }
        let m = from_bytes("t", "stl", &bin, &no_files).unwrap();
        assert_eq!(m.triangles(), 2);
        assert_eq!(m.format, "STL");
    }

    #[test]
    fn ply_ascii_with_colors_and_binary_big_endian() {
        let ascii = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255 0 0\n1 0 0 255 0 0\n0 1 0 255 0 0\n3 0 1 2\n";
        let m = from_bytes("p", "ply", ascii, &no_files).unwrap();
        assert_eq!(m.triangles(), 1);
        assert_eq!(m.vertices[0].color, [255, 0, 0, 255]);
        assert!(m.has_colors);

        let mut bin = b"ply\nformat binary_big_endian 1.0\nelement vertex 4\nproperty double x\nproperty double y\nproperty double z\nelement face 1\nproperty list uchar uint vertex_index\nend_header\n".to_vec();
        for p in [[0.0f64, 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]] {
            bin.extend(p.iter().flat_map(|f| f.to_be_bytes()));
        }
        bin.push(4);
        bin.extend([0u32, 1, 2, 3].iter().flat_map(|i| i.to_be_bytes()));
        assert_eq!(from_bytes("p", "ply", &bin, &no_files).unwrap().triangles(), 2);
    }

    #[test]
    fn off_with_colors() {
        let off = b"COFF\n# a triangle\n3 1 0\n0 0 0 255 255 0 255\n1 0 0 255 255 0 255\n0 1 0 255 255 0 255\n3 0 1 2\n";
        let m = from_bytes("o", "off", off, &no_files).unwrap();
        assert_eq!(m.vertices[0].color, [255, 255, 0, 255]);
    }

    #[test]
    fn gltf_from_memory() {
        // A single triangle, embedded buffer, translated by its node (normalization undoes that).
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0,"translation":[5,0,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
            "bufferViews":[{"buffer":0,"byteLength":36}],
            "buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}]}"#;
        let m = from_bytes("g", "gltf", json.as_bytes(), &no_files).unwrap();
        assert_eq!(m.triangles(), 1);
        assert_eq!(m.format, "glTF");
    }

    #[test]
    fn unknown_formats_and_garbage_fail_cleanly() {
        assert!(from_bytes("x", "fbx", b"", &no_files).is_err());
        assert!(from_bytes("x", "ply", b"garbage", &no_files).is_err());
        assert!(from_bytes("x", "stl", b"\x00\x01", &no_files).is_err());
        assert!(from_bytes("x", "off", b"OFF\n3 1", &no_files).is_err());
    }
}
