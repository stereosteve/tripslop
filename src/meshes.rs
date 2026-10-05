//! Draws 3D models (`model.rs`) on the GPU: rasterized with a depth buffer and 4× MSAA,
//! resolved into an ordinary render target. Three things use it:
//!
//! * model clips, with their own materials (`clip::ModelClip`);
//! * the Shape projector with *Model* picked: the input wrapped onto the model;
//! * Projection mapping with *Model* picked: a projector throws the input at the model, which
//!   casts a shadow (a shadow map rendered from the projector) on itself and the back wall.
//!
//! The cameras match the ray-traced solids in `shaders/fx/shape.wgsl` and `projection.wgsl`,
//! so switching an effect between a built-in solid and a model keeps the framing.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui_wgpu::wgpu;
use glam::{Mat3, Mat4, Vec3, Vec4};

use crate::model::{Model, Vertex};
use crate::modulation::Clock;
use crate::param::Param;

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
const SHADOW_SIZE: u32 = 2048;
/// GPU meshes not drawn for this many frames are freed.
const KEEP_FRAMES: u64 = 600;

/// The per-draw uniform block (`MeshU` in `shaders/mesh.wgsl`).
#[derive(Clone, Copy)]
pub struct Uniforms {
    pub view_proj: Mat4,
    pub model: Mat4,
    pub normal: Mat4,
    pub proj_vp: Mat4,
    /// a..g in the shader.
    pub v: [[f32; 4]; 7],
}

impl Uniforms {
    fn data(&self) -> Vec<[f32; 4]> {
        let mut out = Vec::with_capacity(23);
        for m in [self.view_proj, self.model, self.normal, self.proj_vp] {
            out.extend(m.to_cols_array_2d());
        }
        out.extend(self.v);
        out
    }
}

/// How a draw blends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Depth-tested, opaque.
    Solid,
    /// Additive with no depth test: every layer of the model glows through.
    Glow,
}

/// One model drawn into `target`.
pub struct Draw<'a> {
    pub model: &'a Model,
    pub u: Uniforms,
    pub style: Style,
    /// Projection mapping: render the shadow map first, then also draw the wall with these
    /// uniforms (when it's on).
    pub projection: Option<Option<Uniforms>>,
    /// The input image (shape projector, projection mapping).
    pub input: Option<&'a wgpu::TextureView>,
    pub target: &'a wgpu::TextureView,
    pub size: (u32, u32),
    pub clear: wgpu::Color,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    count: u32,
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    last_used: u64,
}

/// Multisampled colour and depth for one target size.
struct Msaa {
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    last_used: u64,
}

pub struct MeshRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    solid: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    clamp: wgpu::Sampler,
    repeat: wgpu::Sampler,
    compare: wgpu::Sampler,
    meshes: HashMap<u64, GpuMesh>,
    targets: HashMap<(u32, u32), Msaa>,
    shadow_map: wgpu::TextureView,
    /// Bound in place of the shadow map while it's being drawn, and when there's none.
    no_shadow: wgpu::TextureView,
    white: wgpu::TextureView,
    black: wgpu::TextureView,
    wall: wgpu::Buffer,
    uniforms: Vec<wgpu::Buffer>,
    next_uniform: usize,
    frame: u64,
}

fn depth_texture(device: &wgpu::Device, label: &str, (w, h): (u32, u32), samples: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            // Only the shadow map (single-sampled) is ever sampled.
            usage: if samples == 1 { wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING } else { wgpu::TextureUsages::RENDER_ATTACHMENT },
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn pixel(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4]) -> wgpu::TextureView {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mesh 1x1"),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        tex.as_image_copy(),
        &rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    tex.create_view(&Default::default())
}

impl MeshRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let src = format!("{}\n{}", include_str!("shaders/common.wgsl"), include_str!("shaders/mesh.wgsl"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("mesh"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let tex = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let sampler = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(ty), count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: vf,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                sampler(1, wgpu::SamplerBindingType::Filtering),
                tex(2, wgpu::TextureSampleType::Float { filterable: true }),
                tex(3, wgpu::TextureSampleType::Float { filterable: true }),
                tex(4, wgpu::TextureSampleType::Depth),
                sampler(5, wgpu::SamplerBindingType::Comparison),
                sampler(6, wgpu::SamplerBindingType::Filtering),
            ],
        });
        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("mesh"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x2, 4 => Unorm8x4, 5 => Uint32];
        let buffers = [wgpu::VertexBufferLayout { array_stride: size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs }];
        let pipeline = |label, style: Style| {
            let (blend, compare, write) = match style {
                Style::Solid => (None, wgpu::CompareFunction::Less, true),
                Style::Glow => (
                    Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    }),
                    wgpu::CompareFunction::Always,
                    false,
                ),
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_mesh"), compilation_options: Default::default(), buffers: &[Some(buffers[0].clone())] },
                // Models come with either winding (and some are open), so nothing is culled;
                // the fragment shader lights whichever side faces the camera.
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_mesh"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: COLOR, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid = pipeline("mesh", Style::Solid);
        let glow = pipeline("mesh glow", Style::Glow);
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh shadow"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_shadow"), compilation_options: Default::default(), buffers: &[Some(buffers[0].clone())] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                // Slope-scaled bias keeps lit surfaces from shadowing themselves (acne).
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.5, clamp: 0.0 },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let linear = |mode| wgpu::SamplerDescriptor {
            address_mode_u: mode,
            address_mode_v: mode,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        };
        let clamp = device.create_sampler(&linear(wgpu::AddressMode::ClampToEdge));
        let repeat = device.create_sampler(&linear(wgpu::AddressMode::Repeat));
        let compare = device.create_sampler(&wgpu::SamplerDescriptor { compare: Some(wgpu::CompareFunction::LessEqual), ..linear(wgpu::AddressMode::ClampToEdge) });
        // The wall: a unit quad facing the camera (-z), placed by its uniforms.
        let quad = [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|[x, y]| Vertex {
            pos: [x, y, 0.0],
            normal: [0.0, 0.0, -1.0],
            face: [0.0, 0.0, -1.0],
            color: [255; 4],
            ..Default::default()
        });
        let wall = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("projection wall"),
            size: size_of_val(&quad) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&wall, 0, bytemuck::cast_slice(&quad));
        Self {
            device: device.clone(),
            queue: queue.clone(),
            layout,
            solid,
            glow,
            shadow,
            clamp,
            repeat,
            compare,
            meshes: HashMap::new(),
            targets: HashMap::new(),
            shadow_map: depth_texture(device, "projector shadow map", (SHADOW_SIZE, SHADOW_SIZE), 1),
            no_shadow: depth_texture(device, "no shadow", (1, 1), 1),
            white: pixel(device, queue, [255; 4]),
            black: pixel(device, queue, [0, 0, 0, 255]),
            wall,
            uniforms: Vec::new(),
            next_uniform: 0,
            frame: 0,
        }
    }

    /// Call before a batch of draws that will be submitted together (uniform slots are reused
    /// once the previous batch was submitted). Once per output frame (`new_frame`), also frees
    /// what hasn't been drawn in a while.
    pub fn begin(&mut self, new_frame: bool) {
        self.next_uniform = 0;
        if !new_frame {
            return;
        }
        self.frame += 1;
        let frame = self.frame;
        self.meshes.retain(|_, m| frame - m.last_used < KEEP_FRAMES);
        self.targets.retain(|_, t| frame - t.last_used < 4);
    }

    fn uniform(&mut self, u: &Uniforms) -> usize {
        if self.next_uniform == self.uniforms.len() {
            self.uniforms.push(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh uniforms"),
                size: 512,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let i = self.next_uniform;
        self.next_uniform += 1;
        self.queue.write_buffer(&self.uniforms[i], 0, bytemuck::cast_slice(&u.data()));
        i
    }

    fn upload(&mut self, model: &Model) {
        let frame = self.frame;
        if let Some(m) = self.meshes.get_mut(&model.id) {
            m.last_used = frame;
            return;
        }
        let vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("model vertices"),
            size: (size_of::<Vertex>() * model.vertices.len()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&vertices, 0, bytemuck::cast_slice(&model.vertices));
        let texture = model.texture.as_ref().map(|img| {
            let size = wgpu::Extent3d { width: img.width(), height: img.height(), depth_or_array_layers: 1 };
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("model texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                tex.as_image_copy(),
                img.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * img.width()), rows_per_image: Some(img.height()) },
                size,
            );
            let view = tex.create_view(&Default::default());
            (tex, view)
        });
        self.meshes.insert(model.id, GpuMesh { vertices, count: model.vertices.len() as u32, texture, last_used: frame });
    }

    fn bind(&self, ub: usize, input: &wgpu::TextureView, model_tex: &wgpu::TextureView, shadow: &wgpu::TextureView) -> wgpu::BindGroup {
        let entries = [
            wgpu::BindGroupEntry { binding: 0, resource: self.uniforms[ub].as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamp) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(input) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(model_tex) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(shadow) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.compare) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.repeat) },
        ];
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.layout, entries: &entries })
    }

    pub fn draw(&mut self, enc: &mut wgpu::CommandEncoder, d: Draw) {
        self.upload(d.model);
        let frame = self.frame;
        let device = &self.device;
        self.targets
            .entry(d.size)
            .or_insert_with(|| {
                let color = device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("mesh msaa"),
                        size: wgpu::Extent3d { width: d.size.0, height: d.size.1, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: SAMPLES,
                        dimension: wgpu::TextureDimension::D2,
                        format: COLOR,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default());
                Msaa { color, depth: depth_texture(device, "mesh depth", d.size, SAMPLES), last_used: frame }
            })
            .last_used = frame;

        let ub = self.uniform(&d.u);
        let wall_ub = d.projection.flatten().map(|w| self.uniform(&w));
        let input = d.input.unwrap_or(&self.black);
        let mesh = &self.meshes[&d.model.id];
        let model_tex = mesh.texture.as_ref().map(|(_, v)| v).unwrap_or(&self.white);

        if d.projection.is_some() {
            let bg = self.bind(ub, input, model_tex, &self.no_shadow);
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("projector shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.shadow);
            rp.set_bind_group(0, &bg, &[]);
            rp.set_vertex_buffer(0, mesh.vertices.slice(..));
            rp.draw(0..mesh.count, 0..1);
        }

        let shadow = if d.projection.is_some() { &self.shadow_map } else { &self.no_shadow };
        let bg = self.bind(ub, input, model_tex, shadow);
        let wall_bg = wall_ub.map(|w| self.bind(w, input, &self.white, shadow));
        let msaa = &self.targets[&d.size];
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mesh"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &msaa.color,
                depth_slice: None,
                resolve_target: Some(d.target),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(d.clear), store: wgpu::StoreOp::Discard },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &msaa.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(if d.style == Style::Glow { &self.glow } else { &self.solid });
        rp.set_bind_group(0, &bg, &[]);
        rp.set_vertex_buffer(0, mesh.vertices.slice(..));
        rp.draw(0..mesh.count, 0..1);
        if let Some(wall_bg) = &wall_bg {
            rp.set_pipeline(&self.solid);
            rp.set_bind_group(0, wall_bg, &[]);
            rp.set_vertex_buffer(0, self.wall.slice(..));
            rp.draw(0..6, 0..1);
        }
    }

    /// GPU memory for meshes, their textures and the multisampled targets, in bytes.
    pub fn memory(&self) -> u64 {
        let meshes: u64 = self
            .meshes
            .values()
            .map(|m| m.vertices.size() + m.texture.as_ref().map_or(0, |(t, _)| 4 * t.width() as u64 * t.height() as u64))
            .sum();
        let targets: u64 = self.targets.keys().map(|(w, h)| 8 * SAMPLES as u64 * *w as u64 * *h as u64).sum();
        meshes + targets + 4 * (SHADOW_SIZE as u64).pow(2)
    }
}

// ------------------------------------------------------------------------------ cameras

/// A camera looking down +z (x right, y up), like the ray-traced solids. Depth maps to 0..1
/// between `near` and `far`.
fn perspective(focal: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let a = far / (far - near);
    Mat4::from_cols(
        Vec4::new(focal / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, focal, 0.0, 0.0),
        Vec4::new(0.0, 0.0, a, 1.0),
        Vec4::new(0.0, 0.0, -near * a, 0.0),
    )
}

/// World → camera space for a camera at `pos` with the given axes.
fn look(pos: Vec3, right: Vec3, up: Vec3, fwd: Vec3) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(right.x, up.x, fwd.x, 0.0),
        Vec4::new(right.y, up.y, fwd.y, 0.0),
        Vec4::new(right.z, up.z, fwd.z, 0.0),
        Vec4::new(-right.dot(pos), -up.dot(pos), -fwd.dot(pos), 1.0),
    )
}

/// The viewer: on -z looking at the origin, with the picture shifted by (x, y) (-1..1 moves
/// the object a full frame).
fn viewer(focal: f32, aspect: f32, cam: f32, x: f32, y: f32) -> Mat4 {
    let shift = Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(2.0 * x, 2.0 * y, 0.0, 1.0));
    shift * perspective(focal, aspect, 0.05, 100.0) * look(Vec3::new(0.0, 0.0, -cam), Vec3::X, Vec3::Y, Vec3::Z)
}

/// Fixed angles plus beat-synced spin (turns per 4-beat bar), applied x, then y, then z.
fn orientation(p: &[Param], rotate: usize, spin: usize, beat: f64) -> Mat3 {
    let bar = (beat / 4.0) as f32 * std::f32::consts::TAU;
    let a = |i: usize| p[rotate + i].get().to_radians() + p[spin + i].get() * bar;
    Mat3::from_rotation_z(a(2)) * Mat3::from_rotation_y(a(1)) * Mat3::from_rotation_x(a(0))
}

/// Object → world and its normal matrix.
fn placement(rot: Mat3, scale: Vec3) -> (Mat4, Mat4) {
    (Mat4::from_mat3(rot) * Mat4::from_scale(scale), Mat4::from_mat3(rot * Mat3::from_diagonal(scale.recip())))
}

/// How far back the camera sits so an object of `radius` fits (same rule as the solids).
fn distance(focal: f32, radius: f32) -> f32 {
    (0.9 * focal).max(2.2 * radius + 0.3)
}

fn flags(m: &Model) -> f32 {
    (m.texture.is_some() as u32 | (m.has_uvs as u32) << 1 | (m.has_colors as u32) << 2) as f32
}

fn opaque(c: f32) -> wgpu::Color {
    if c > 0.5 { wgpu::Color::BLACK } else { wgpu::Color::TRANSPARENT }
}

/// Materials of a model clip (`clip::MODEL_SPECS`): `Glow` ones are drawn additively.
pub const MATERIALS: &[&str] = &["Surface", "Normals", "Chrome", "Toon", "Hologram", "Wireframe"];

/// A model clip (see `clip::MODEL_SPECS` for the parameter order).
pub fn clip_draw<'a>(model: &'a Model, p: &[Param], clock: Clock, target: &'a wgpu::TextureView, size: (u32, u32)) -> Draw<'a> {
    let aspect = size.0 as f32 / size.1 as f32;
    let focal = 1.0 / (p[11].get().clamp(5.0, 150.0).to_radians() * 0.5).tan();
    let size_k = p[2].get().max(0.01);
    let cam = distance(focal, size_k);
    let (model_m, normal) = placement(orientation(p, 3, 6, clock.beat), Vec3::splat(size_k));
    let material = p[0].index() as f32;
    let u = Uniforms {
        view_proj: viewer(focal, aspect, cam, p[9].get(), p[10].get()),
        model: model_m,
        normal,
        proj_vp: Mat4::IDENTITY,
        v: [
            [0.0, material, p[1].get(), p[12].get()],
            [clock.time as f32, clock.beat as f32, p[13].get(), p[14].get()],
            [p[16].get(), p[17].get(), p[18].get(), p[15].get()],
            [0.0, flags(model), 0.0, 0.0],
            [0.0, 0.0, -cam, 0.0],
            [0.0, 0.0, 0.0, aspect],
            [0.0; 4],
        ],
    };
    let style = if material >= 4.0 { Style::Glow } else { Style::Solid };
    Draw { model, u, style, projection: None, input: None, target, size, clear: opaque(p[19].get()) }
}

/// The Shape projector with a model (see `effects.rs` for the parameter order).
pub fn shape_draw<'a>(model: &'a Model, p: &[Param], clock: Clock, input: &'a wgpu::TextureView, target: &'a wgpu::TextureView, size: (u32, u32)) -> Draw<'a> {
    let aspect = size.0 as f32 / size.1 as f32;
    let focal = 1.0 / (p[14].get().clamp(5.0, 150.0).to_radians() * 0.5).tan();
    let (s, h) = (p[2].get().max(0.01), p[3].get().max(0.01));
    let cam = distance(focal, s * h.max(1.0));
    let (model_m, normal) = placement(orientation(p, 4, 7, clock.beat), Vec3::new(s, s * h, s));
    let background = p[13].index();
    let u = Uniforms {
        view_proj: viewer(focal, aspect, cam, p[15].get(), p[16].get()),
        model: model_m,
        normal,
        proj_vp: Mat4::IDENTITY,
        v: [
            [1.0, 0.0, 0.0, p[12].get()],
            [clock.time as f32, clock.beat as f32, 0.0, 1.0],
            [0.0; 4],
            [p[17].get(), flags(model), 0.0, 0.0],
            [0.0, 0.0, -cam, 0.0],
            [0.0, 0.0, 0.0, aspect],
            [(background == 2) as u32 as f32, 0.0, 0.0, 0.0],
        ],
    };
    let clear = if background == 2 { wgpu::Color::BLACK } else { wgpu::Color::TRANSPARENT };
    Draw { model, u, style: Style::Solid, projection: None, input: Some(input), target, size, clear }
}

/// Projection mapping with a model (see `effects.rs` for the parameter order).
pub fn projection_draw<'a>(model: &'a Model, p: &[Param], clock: Clock, input: &'a wgpu::TextureView, target: &'a wgpu::TextureView, size: (u32, u32)) -> Draw<'a> {
    let aspect = size.0 as f32 / size.1 as f32;
    let focal = 1.0 / 22.5f32.to_radians().tan();
    let (s, h) = (p[2].get().max(0.01), p[3].get().max(0.01));
    let radius = s * h.max(1.0);
    let cam = distance(focal, radius);
    let (model_m, normal) = placement(orientation(p, 4, 7, clock.beat), Vec3::new(s, s * h, s));
    // The projector: as far away as the viewer, swung round by angle / elevation, aimed at
    // the object. Zoom narrows its beam.
    let (yaw, pitch) = (p[10].get().to_radians(), p[11].get().to_radians());
    let ppos = cam * Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
    let fwd = (-ppos).normalize();
    let right = Vec3::Y.cross(fwd).normalize();
    let up = fwd.cross(right);
    let proj_vp = perspective(focal * p[12].get().max(0.05), aspect, 0.3, 30.0) * look(ppos, right, up, fwd);
    let view_proj = viewer(focal, aspect, cam, p[18].get(), p[19].get());
    let v = [
        [2.0, 0.0, 0.0, 0.0],
        [clock.time as f32, clock.beat as f32, 0.0, 1.0],
        [0.0; 4],
        [0.0, flags(model), p[15].get(), p[16].get()],
        [0.0, 0.0, -cam, 0.0],
        [ppos.x, ppos.y, ppos.z, aspect],
        [0.0; 4],
    ];
    let u = Uniforms { view_proj, model: model_m, normal, proj_vp, v };
    // The wall: big enough to fill the view, behind the object.
    let wall_z = radius + p[14].get();
    let wall = (p[13].index() > 0).then(|| {
        let mut v = v;
        v[4][3] = 1.0;
        Uniforms { model: Mat4::from_translation(Vec3::new(0.0, 0.0, wall_z)) * Mat4::from_scale(Vec3::new(40.0, 40.0, 1.0)), normal: Mat4::IDENTITY, v, ..u }
    });
    let clear = if p[17].index() == 1 { wgpu::Color::BLACK } else { wgpu::Color::TRANSPARENT };
    Draw { model, u, style: Style::Solid, projection: Some(wall), input: Some(input), target, size, clear }
}

/// A model's card picture in the library: its own colours and texture, lit, turning when `t`
/// moves.
pub fn card_draw<'a>(model: &'a Arc<Model>, clock: Clock, target: &'a wgpu::TextureView, size: (u32, u32)) -> Draw<'a> {
    let params: Vec<Param> = crate::clip::MODEL_SPECS.iter().map(|s| Param::new(*s)).collect();
    let mut d = clip_draw(model, &params, clock, target, size);
    d.clear = wgpu::Color { r: 0.06, g: 0.04, b: 0.08, a: 1.0 };
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewer_matches_the_ray_traced_camera() {
        // A point straight ahead at the origin lands in the middle; the (x, y) shift moves it.
        let m = viewer(2.0, 16.0 / 9.0, 3.0, 0.0, 0.0);
        let c = m * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!((c.x / c.w).abs() < 1e-6 && (c.y / c.w).abs() < 1e-6);
        assert!((0.0..1.0).contains(&(c.z / c.w)));
        // The top of the frame is at y = cam / focal.
        let top = m * Vec4::new(0.0, 3.0 / 2.0, 0.0, 1.0);
        assert!((top.y / top.w - 1.0).abs() < 1e-5);
        let shifted = viewer(2.0, 16.0 / 9.0, 3.0, 0.25, -0.5) * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!((shifted.x / shifted.w - 0.5).abs() < 1e-5 && (shifted.y / shifted.w + 1.0).abs() < 1e-5);
    }

    #[test]
    fn rotations_match_the_wgsl_ones() {
        // shape.wgsl's rot_y(a) maps +x to (cos a, 0, -sin a).
        let r = Mat3::from_rotation_y(0.3) * Vec3::X;
        assert!((r - Vec3::new(0.3f32.cos(), 0.0, -(0.3f32.sin()))).length() < 1e-6);
    }

    #[test]
    fn uniforms_fit_the_buffer() {
        let u = Uniforms { view_proj: Mat4::IDENTITY, model: Mat4::IDENTITY, normal: Mat4::IDENTITY, proj_vp: Mat4::IDENTITY, v: [[0.0; 4]; 7] };
        assert!(u.data().len() * 16 <= 512);
    }

    #[test]
    fn mesh_shader_validates() {
        let src = format!("{}\n{}", include_str!("shaders/common.wgsl"), include_str!("shaders/mesh.wgsl"));
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }
}
