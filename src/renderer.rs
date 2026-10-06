//! GPU pipeline, per tick:
//!
//! ```text
//! for each layer (bottom → top):
//!     clip(s) ──clip pass (transform, fit, transition)──► layer tex
//!     layer tex ──effect → effect → …──► (ping-pong)          (delay rings per effect)
//!     layer tex ──composite (blend mode, opacity)──► bank A and/or bank B
//! mix(bank A, bank B, crossfader) ──master effects──► final (master fader) ──► output
//! ```
//!
//! Banks only exist in bank crossfade mode when some layer is assigned to A or B: A layers
//! go into bank A, B layers into bank B and unassigned ones into both. Otherwise there's a
//! single composition. The output goes to the screen, output window and recorder.
//!
//! GPU resources are keyed by the model's ids (clip, layer, effect) and created on demand;
//! anything not used in a frame is freed.

use std::collections::{HashMap, HashSet};

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

use crate::clip::{Clip, Media};
use crate::composition::{Composition, Side};
use crate::effects::{EFFECTS, Effect, EffectKind, HistorySource};
use crate::modulation::Clock;
use crate::punch::Punch;
use crate::isf_library::{Kind, Library, Status};
use crate::meshes::{self, MeshRenderer};
use crate::model::Model;
use crate::shader::{CompileError, CustomShader};

/// Program (output) size until something changes it.
pub const DEFAULT_SIZE: (u32, u32) = (1280, 720);
/// Size that video files and cameras are imported at, whatever the program size.
pub const MEDIA_WIDTH: u32 = 1280;
pub const MEDIA_HEIGHT: u32 = 720;
/// Smallest program size accepted.
const MIN_SIDE: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const RENDER: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);
const NOISE_SIZE: u32 = 256;
const AUDIO_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST);
/// composite.wgsl mode that dissolves to the second texture instead of blending it on top.
const XFADE: f32 = 9.0;
pub const THUMB_W: u32 = 192;
pub const THUMB_H: u32 = 108;
/// Moment at which generator / shader previews are rendered (seconds, beats).
const PREVIEW_TIME: f64 = 2.0;
/// Shader library card pictures.
pub const LIB_W: u32 = 256;
pub const LIB_H: u32 = 144;
/// Library pictures kept on the GPU (about 150 KB each); the longest unseen go first.
const LIB_KEEP: usize = 160;
/// Time per frame spent compiling new library pictures (at least one is always done).
const LIB_BUDGET: std::time::Duration = std::time::Duration::from_millis(6);
/// What effects in the library are shown processing.
const LIB_INPUT: &[u8] = include_bytes!("../samples/pillars-of-creation.jpg");

/// A library card's picture (see `Renderer::library_previews`).
struct LibThumb {
    tex: Tex,
    id: egui::TextureId,
    what: CardSource,
    rendered: bool,
    /// `library_previews` call that last asked for it.
    last_seen: u64,
}

enum CardSource {
    Shader {
        shader: CustomShader,
        /// Pipeline and whether it works in screen space, or why it can't run.
        gpu: Result<(Pipe, bool), String>,
    },
    Model(std::sync::Arc<Model>),
}

/// A small GPU texture shown in the clip grid.
struct Thumb {
    tex: Tex,
    id: egui::TextureId,
    /// What the preview was rendered from; re-render only when this changes.
    signature: Option<u64>,
}

struct Tex {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
}

struct SourceTex {
    tex: Tex,
    size: (u32, u32),
}

struct Ring {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    len: u32,
    head: u32,
    size: (u32, u32),
}

impl Ring {
    fn layer_ago(&self, ago: u32) -> f32 {
        ((self.head + self.len - ago % self.len) % self.len) as f32
    }
}

/// GPU state of one user shader.
struct CustomGpu {
    pipe: Option<Pipe>,
    /// Output ping-pong (allocated when first drawn); the other one is `iChannel1`.
    bufs: Option<[Tex; 2]>,
    cur: usize,
    frame: i32,
    /// The effect input, flipped to GL orientation (`iChannel0`).
    input_gl: Option<Tex>,
    /// WGSL shaders run in screen space: no flips.
    screen_space: bool,
}

struct Pipe {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    sampler: wgpu::Sampler,
    clip_pipe: Pipe,
    composite_pipe: Pipe,
    final_pipe: Pipe,
    fx_pipes: HashMap<EffectKind, Pipe>,
    uniforms: Vec<wgpu::Buffer>,
    next_uniform: usize,
    dummy_tex: Tex,
    dummy_ring: wgpu::TextureView,
    sources: HashMap<u64, SourceTex>,
    layers: HashMap<u64, [Tex; 2]>,
    rings: HashMap<u64, Ring>,
    /// Composition ping-pong buffers (taken out of `self` while rendering). In bank
    /// crossfade mode these hold bank A.
    comp: Option<[Tex; 2]>,
    /// Bank B's ping-pong buffers; allocated the first time anything is assigned to B.
    bank_b: Option<[Tex; 2]>,
    output: Tex,
    pub display_id: egui::TextureId,
    /// Program size: every render target, the output and recordings.
    size: (u32, u32),
    // User shaders.
    vs_module: wgpu::ShaderModule,
    flip_pipe: Pipe,
    repeat_sampler: wgpu::Sampler,
    noise: Tex,
    /// Audio spectrum and waveform for shaders (`audio::TEX_LEN` × 1, value in red).
    audio_tex: [Tex; 2],
    /// Last frame's output in GL orientation (`iChannel3`).
    prev_output_gl: Tex,
    custom: HashMap<u64, CustomGpu>,
    // Clip grid thumbnails.
    egui_renderer: std::sync::Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    thumbs: HashMap<u64, Thumb>,
    /// GL-oriented scratch target for shader previews.
    preview_gl: Tex,
    tick: u64,
    // Shader library cards.
    lib_thumbs: HashMap<String, LibThumb>,
    /// GL-oriented scratch target, the size of a card picture.
    lib_gl: Tex,
    /// The picture effects are shown processing: screen-space and GL-oriented.
    lib_input: Option<[Tex; 2]>,
    lib_calls: u64,
    /// 3D models: model clips, and shape / projection effects set to *Model*.
    meshes: MeshRenderer,
    /// Where the Shape projector draws a model before laying it over its input.
    mesh_scratch: Option<Tex>,
    /// An effect's output when it's mixed with its input (wet < 1 or a wet blend).
    wet_scratch: Option<Tex>,
}

fn shader(device: &wgpu::Device, label: &str, parts: &[&str]) -> wgpu::ShaderModule {
    let mut src = String::from(include_str!("shaders/common.wgsl"));
    for p in parts {
        src.push('\n');
        src.push_str(p);
    }
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}

/// Explicit layout: uniform, sampler, then the given textures (`true` = 2D array). Explicit
/// so that shaders which ignore a binding (e.g. effects without history) still match.
fn bind_layout(device: &wgpu::Device, textures: &[bool]) -> wgpu::BindGroupLayout {
    let frag = wgpu::ShaderStages::FRAGMENT;
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: frag,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: frag,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    for (i, array) in textures.iter().enumerate() {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 2 + i as u32,
            visibility: frag,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: if *array { wgpu::TextureViewDimension::D2Array } else { wgpu::TextureViewDimension::D2 },
                multisampled: false,
            },
            count: None,
        });
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries })
}

fn pipe(device: &wgpu::Device, label: &str, module: &wgpu::ShaderModule, blend: Option<wgpu::BlendState>, textures: &[bool]) -> Pipe {
    let layout = bind_layout(device, textures);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    Pipe { pipeline, layout }
}

fn texture(device: &wgpu::Device, label: &str, w: u32, h: u32, layers: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage,
        view_formats: &[],
    })
}

fn tex2d(device: &wgpu::Device, label: &str, w: u32, h: u32, usage: wgpu::TextureUsages) -> Tex {
    let tex = texture(device, label, w, h, 1, usage);
    let view = tex.create_view(&Default::default());
    Tex { tex, view }
}

/// A ping-pong pair of render targets.
fn pair(device: &wgpu::Device, label: &str, size: (u32, u32)) -> [Tex; 2] {
    [tex2d(device, label, size.0, size.1, RENDER), tex2d(device, label, size.0, size.1, RENDER)]
}

fn ring(device: &wgpu::Device, len: u32, size: (u32, u32)) -> Ring {
    let tex = texture(
        device,
        "history ring",
        size.0,
        size.1,
        len,
        // Render attachment for reduced-size history, which is drawn (scaled) rather than copied.
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    Ring { tex, view, len, head: 0, size }
}

/// Size of an effect's history: the program size, or half of it.
fn history_size(size: (u32, u32), half: bool) -> (u32, u32) {
    if half { ((size.0 / 2).max(1), (size.1 / 2).max(1)) } else { size }
}

/// Bytes per row of a texture-to-buffer copy: wgpu wants a multiple of 256.
pub fn padded_row(width: u32) -> u32 {
    let a = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    (4 * width).div_ceil(a) * a
}

/// Tightly packed RGBA from rows of `padded_row(width)` bytes.
pub fn unpad_rows(data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (row, padded) = (4 * width as usize, padded_row(width) as usize);
    if row == padded {
        return data[..row * height as usize].to_vec();
    }
    data.chunks(padded).take(height as usize).flat_map(|r| &r[..row]).copied().collect()
}

/// Parse a program size: `1280x720`, `720p` or `1080p`. Sizes are rounded down to even numbers
/// (H.264 needs them).
pub fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = match s.trim().to_ascii_lowercase().as_str() {
        "720p" => (1280, 720),
        "1080p" => (1920, 1080),
        "1440p" => (2560, 1440),
        "4k" | "2160p" => (3840, 2160),
        other => {
            let (w, h) = other.split_once('x').ok_or_else(|| format!("size {s:?}: use WxH, 720p or 1080p"))?;
            let n = |v: &str| v.trim().parse::<u32>().map_err(|_| format!("size {s:?}: {v:?} isn't a number"));
            (n(w)?, n(h)?)
        }
    };
    if w < MIN_SIDE || h < MIN_SIDE {
        return Err(format!("size {w}x{h} is too small (at least {MIN_SIDE}x{MIN_SIDE})"));
    }
    Ok((w & !1, h & !1))
}

fn pass(enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, clear: Option<wgpu::Color>, pipe: &Pipe, bg: &wgpu::BindGroup) {
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(&pipe.pipeline);
    rp.set_bind_group(0, bg, &[]);
    rp.draw(0..3, 0..1);
}

fn clear_pass(enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, color: wgpu::Color) {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

impl Renderer {
    pub fn new(rs: &egui_wgpu::RenderState) -> Self {
        let (width, height) = DEFAULT_SIZE;
        let device = rs.device.clone();
        let queue = rs.queue.clone();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let clip_pipe = pipe(
            &device,
            "clip",
            &shader(&device, "clip", &[include_str!("shaders/clip.wgsl")]),
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            &[false],
        );
        let composite_pipe = pipe(
            &device,
            "composite",
            &shader(&device, "composite", &[include_str!("shaders/composite.wgsl")]),
            None,
            &[false, false],
        );
        let final_pipe = pipe(&device, "final", &shader(&device, "final", &[include_str!("shaders/final.wgsl")]), None, &[false]);
        let header = include_str!("shaders/fx/header.wgsl");
        let fx_pipes = EFFECTS
            .iter()
            .filter(|d| !d.shader.is_empty())
            .map(|d| (d.kind, pipe(&device, d.name, &shader(&device, d.name, &[header, d.shader]), None, &[false, true])))
            .collect();

        let dummy_tex = tex2d(&device, "dummy", 1, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let vs_module = shader(&device, "fullscreen vs", &[]);
        let flip_pipe = pipe(&device, "flip", &shader(&device, "flip", &[include_str!("shaders/flip.wgsl")]), None, &[false]);
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let noise = tex2d(
            &device,
            "noise",
            NOISE_SIZE,
            NOISE_SIZE,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let noise_bytes: Vec<u8> = (0..NOISE_SIZE * NOISE_SIZE * 4)
            .map(|i| {
                let mut z = (i as u64).wrapping_mul(0x9e3779b97f4a7c15);
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                (z >> 56) as u8
            })
            .collect();
        queue.write_texture(
            noise.tex.as_image_copy(),
            &noise_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * NOISE_SIZE),
                rows_per_image: Some(NOISE_SIZE),
            },
            wgpu::Extent3d {
                width: NOISE_SIZE,
                height: NOISE_SIZE,
                depth_or_array_layers: 1,
            },
        );
        let prev_output_gl = tex2d(&device, "previous output (GL)", width, height, RENDER);
        let preview_gl = tex2d(&device, "shader preview", THUMB_W, THUMB_H, RENDER);
        let lib_gl = tex2d(&device, "library scratch", LIB_W, LIB_H, RENDER);
        let audio_tex = [
            tex2d(&device, "audio spectrum", crate::audio::TEX_LEN as u32, 1, AUDIO_USAGE),
            tex2d(&device, "audio waveform", crate::audio::TEX_LEN as u32, 1, AUDIO_USAGE),
        ];
        let dummy_ring = ring(&device, 1, (1, 1)).view;
        let comp = Some(pair(&device, "comp", (width, height)));
        let output = tex2d(&device, "output", width, height, RENDER);
        let display_id = rs
            .renderer
            .write()
            .register_native_texture(&device, &output.view, wgpu::FilterMode::Linear);

        Self {
            sampler,
            clip_pipe,
            composite_pipe,
            final_pipe,
            fx_pipes,
            uniforms: Vec::new(),
            next_uniform: 0,
            dummy_tex,
            dummy_ring,
            sources: HashMap::new(),
            layers: HashMap::new(),
            rings: HashMap::new(),
            comp,
            bank_b: None,
            output,
            display_id,
            size: (width, height),
            vs_module,
            flip_pipe,
            repeat_sampler,
            noise,
            audio_tex,
            prev_output_gl,
            custom: HashMap::new(),
            egui_renderer: rs.renderer.clone(),
            thumbs: HashMap::new(),
            preview_gl,
            tick: 0,
            lib_thumbs: HashMap::new(),
            lib_gl,
            lib_input: None,
            lib_calls: 0,
            meshes: MeshRenderer::new(&device, &queue),
            mesh_scratch: None,
            wet_scratch: None,
            device,
            queue,
        }
    }

    /// Composite `src` onto `bufs[cur]` (composite.wgsl mode `blend`); returns the new index.
    fn composite(&mut self, enc: &mut wgpu::CommandEncoder, bufs: &[Tex; 2], cur: usize, src: &wgpu::TextureView, blend: f32, amount: f32) -> usize {
        let ub = self.uniform(&[[blend, amount, 0.0, 0.0]]);
        let bg = self.bind(&self.composite_pipe.layout, ub, &[&bufs[cur].view, src]);
        pass(enc, &bufs[1 - cur].view, None, &self.composite_pipe, &bg);
        1 - cur
    }

    /// A fresh uniform buffer for this pass (each pass in a frame needs its own data).
    fn uniform(&mut self, data: &[[f32; 4]]) -> usize {
        if self.next_uniform == self.uniforms.len() {
            self.uniforms.push(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass uniforms"),
                size: 512,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let i = self.next_uniform;
        self.next_uniform += 1;
        self.queue.write_buffer(&self.uniforms[i], 0, bytemuck::cast_slice(data));
        i
    }

    fn bind(&self, layout: &wgpu::BindGroupLayout, ub: usize, views: &[&wgpu::TextureView]) -> wgpu::BindGroup {
        self.bind_with(&self.sampler, layout, ub, views)
    }

    fn bind_with(&self, sampler: &wgpu::Sampler, layout: &wgpu::BindGroupLayout, ub: usize, views: &[&wgpu::TextureView]) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniforms[ub].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ];
        for (i, v) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + i as u32,
                resource: wgpu::BindingResource::TextureView(v),
            });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &entries,
        })
    }

    /// Upload a clip's newest frame. Returns false if it has nothing to show yet.
    fn upload(&mut self, clip: &mut Clip) -> bool {
        if matches!(clip.media, Media::Generator(_)) {
            return true;
        }
        if !self.sources.contains_key(&clip.id) {
            clip.invalidate_upload();
        }
        if let Some(f) = clip.poll_frame() {
            let size = (f.width, f.height);
            let entry = self.sources.entry(clip.id);
            let src = entry.or_insert_with(|| SourceTex {
                tex: tex2d(
                    &self.device,
                    "clip source",
                    size.0,
                    size.1,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                ),
                size,
            });
            if src.size != size {
                *src = SourceTex {
                    tex: tex2d(
                        &self.device,
                        "clip source",
                        size.0,
                        size.1,
                        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    ),
                    size,
                };
            }
            self.queue.write_texture(
                src.tex.tex.as_image_copy(),
                &f.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * size.0),
                    rows_per_image: Some(size.1),
                },
                wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.sources.contains_key(&clip.id)
    }

    /// Run one effect from `src` into `dst`.
    fn effect(&mut self, enc: &mut wgpu::CommandEncoder, e: &Effect, src: &Tex, dst: &Tex, clock: Clock) {
        if let Some(model) = e.drawn_model() {
            self.mesh_effect(enc, e, &model, src, dst, clock);
            return;
        }
        let def = e.def();
        let mut taps = [0.0f32; 4];
        if let Some(h) = def.history {
            let want = history_size(self.size, e.half_history);
            if self.rings.get(&e.id).is_none_or(|r| r.size != want) {
                self.rings.insert(e.id, ring(&self.device, h.frames, want));
            }
            if h.source == HistorySource::Input {
                // The current input becomes tap "0 frames ago".
                self.write_ring(enc, src, e.id);
            }
            let ring = &self.rings[&e.id];
            let t = (h.taps)(&e.params);
            for (i, ago) in t.iter().enumerate() {
                // Output history: the newest stored frame is 1 frame ago.
                let ago = if h.source == HistorySource::Output { (*ago).max(1) } else { *ago };
                taps[i] = ring.layer_ago(ago);
            }
        }
        let mut data = [[0.0f32; 4]; 9];
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        data[0] = [clock.time as f32, w / h, 1.0 / w, 1.0 / h];
        data[1] = [clock.beat as f32, clock.bpm, 0.0, 0.0];
        data[2] = taps;
        for (i, p) in e.params.iter().enumerate() {
            data[3 + i / 4][i % 4] = p.get();
        }
        let ub = self.uniform(&data);
        let pipe = &self.fx_pipes[&e.kind];
        let ring_view = self.rings.get(&e.id).map(|r| &r.view).unwrap_or(&self.dummy_ring);
        let bg = self.bind(&pipe.layout, ub, &[&src.view, ring_view]);
        pass(enc, &dst.view, Some(wgpu::Color::TRANSPARENT), pipe, &bg);

        if let Some(h) = def.history {
            if h.source == HistorySource::Output {
                self.write_ring(enc, dst, e.id);
            }
            let ring = self.rings.get_mut(&e.id).unwrap();
            ring.head = (ring.head + 1) % ring.len;
        }
    }

    /// The Shape projector or Projection mapping with a model instead of a built-in solid.
    fn mesh_effect(&mut self, enc: &mut wgpu::CommandEncoder, e: &Effect, model: &Model, src: &Tex, dst: &Tex, clock: Clock) {
        let size = self.size;
        if e.kind == EffectKind::ProjectionMapping {
            self.meshes.draw(enc, meshes::projection_draw(model, &e.params, clock, &src.view, &dst.view, size));
            return;
        }
        // Background "Input": draw the model on its own, then lay it over the input.
        if e.params[13].index() != 1 {
            self.meshes.draw(enc, meshes::shape_draw(model, &e.params, clock, &src.view, &dst.view, size));
            return;
        }
        let scratch = self.mesh_scratch.take().unwrap_or_else(|| tex2d(&self.device, "mesh scratch", size.0, size.1, RENDER));
        self.meshes.draw(enc, meshes::shape_draw(model, &e.params, clock, &src.view, &scratch.view, size));
        let ub = self.uniform(&[[0.0, 1.0, 0.0, 0.0]]);
        let bg = self.bind(&self.composite_pipe.layout, ub, &[&src.view, &scratch.view]);
        pass(enc, &dst.view, None, &self.composite_pipe, &bg);
        self.mesh_scratch = Some(scratch);
    }

    /// Store `from` in the ring's current slot: a plain copy at full size, otherwise a
    /// scaled draw (the linear sampler averages 2×2 pixels at half size).
    fn write_ring(&mut self, enc: &mut wgpu::CommandEncoder, from: &Tex, id: u64) {
        let ring = &self.rings[&id];
        if ring.size == self.size {
            enc.copy_texture_to_texture(
                from.tex.as_image_copy(),
                wgpu::TexelCopyTextureInfo {
                    texture: &ring.tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: ring.head },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: self.size.0,
                    height: self.size.1,
                    depth_or_array_layers: 1,
                },
            );
            return;
        }
        let ub = self.uniform(&[[3.0, 0.0, 0.0, 0.0]]);
        let bg = self.bind(&self.flip_pipe.layout, ub, &[&from.view]);
        let ring = &self.rings[&id];
        let slot = ring.tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: ring.head,
            array_layer_count: Some(1),
            ..Default::default()
        });
        pass(enc, &slot, None, &self.flip_pipe, &bg);
    }

    /// Run an effect chain, ping-ponging between `bufs`. Returns the index holding the result.
    fn chain(&mut self, enc: &mut wgpu::CommandEncoder, effects: Vec<&mut Effect>, bufs: &[Tex; 2], mut cur: usize, clock: Clock, used: &mut HashSet<u64>) -> usize {
        for e in effects.into_iter().filter(|e| e.enabled) {
            used.insert(e.id);
            if !e.is_mixed() {
                self.run_effect(enc, e, &bufs[cur], &bufs[1 - cur], clock, used);
                cur = 1 - cur;
                continue;
            }
            // Render the wet picture aside, then mix it with the dry input into the other buffer.
            let size = self.size;
            let wet = self.wet_scratch.take().unwrap_or_else(|| tex2d(&self.device, "wet", size.0, size.1, RENDER));
            self.run_effect(enc, e, &bufs[cur], &wet, clock, used);
            let blend = e.wet_blend.index();
            // Normal is a straight dissolve; the others lay the wet picture over the dry one.
            let mode = if blend == 0 { XFADE } else { blend as f32 };
            cur = self.composite(enc, bufs, cur, &wet.view, mode, e.wet.get());
            self.wet_scratch = Some(wet);
        }
        cur
    }

    fn run_effect(&mut self, enc: &mut wgpu::CommandEncoder, e: &mut Effect, src: &Tex, dst: &Tex, clock: Clock, used: &mut HashSet<u64>) {
        if let Some(c) = e.custom.as_deref_mut() {
            used.insert(c.id);
            self.run_custom(enc, c, Some(src), dst, clock);
        } else {
            self.effect(enc, e, src, dst, clock);
        }
    }

    /// Compile a user shader's WGSL into a pipeline, catching GPU validation errors.
    fn build_custom(&self, wgsl: &str, entry: &str) -> Result<Pipe, String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fs = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("user shader"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let layout = bind_layout(&self.device, &[false; 6]);
        let pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("user shader"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("user shader"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.vs_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fs,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        // The browser can't block on the scope; naga has already validated the shader on the
        // CPU (shader::compile), so dropping the scope there just skips the second check.
        #[cfg(target_arch = "wasm32")]
        {
            drop(scope);
            Ok(Pipe { pipeline, layout })
        }
        #[cfg(not(target_arch = "wasm32"))]
        match pollster::block_on(scope.pop()) {
            Some(e) => Err(e.to_string()),
            None => Ok(Pipe { pipeline, layout }),
        }
    }

    fn flip(&mut self, enc: &mut wgpu::CommandEncoder, src: &Tex, dst: &Tex, mode: f32) {
        let ub = self.uniform(&[[mode, 1.0, 0.0, 0.0]]);
        let bg = self.bind(&self.flip_pipe.layout, ub, &[&src.view]);
        pass(enc, &dst.view, Some(wgpu::Color::TRANSPARENT), &self.flip_pipe, &bg);
    }

    /// Compile a user shader if its code changed. Runs for every shader in the
    /// composition each tick, whether or not it's being drawn, so the editor always
    /// reflects the current code.
    fn compile_custom(&mut self, shader: &mut CustomShader) {
        let Some(compiled) = shader.poll_compile() else { return };
        let built = self.build_custom(&compiled.wgsl, &compiled.entry);
        let gpu = self.custom.entry(shader.id).or_insert_with(|| CustomGpu {
            pipe: None,
            bufs: None,
            cur: 0,
            frame: 0,
            input_gl: None,
            screen_space: false,
        });
        match built {
            Ok(p) => {
                gpu.pipe = Some(p);
                gpu.screen_space = compiled.screen_space;
                gpu.frame = 0;
                shader.running = true;
            }
            Err(e) => {
                shader.errors = vec![CompileError {
                    line: None,
                    message: format!("GPU rejected the shader: {e}"),
                }]
            }
        }
    }

    /// Run a user shader. `input` (an effect's input) becomes `iChannel0`; output goes to `dst`.
    fn run_custom(&mut self, enc: &mut wgpu::CommandEncoder, shader: &mut CustomShader, input: Option<&Tex>, dst: &Tex, clock: Clock) {
        let Some(mut gpu) = self.custom.remove(&shader.id) else {
            // Not compiled yet: pass the input through (or show nothing).
            match input {
                Some(i) => enc.copy_texture_to_texture(i.tex.as_image_copy(), dst.tex.as_image_copy(), i.tex.size()),
                None => clear_pass(enc, &dst.view, wgpu::Color::TRANSPARENT),
            }
            return;
        };
        if gpu.pipe.is_none() {
            // Nothing has compiled yet: pass the input through (or show nothing).
            match input {
                Some(i) => enc.copy_texture_to_texture(i.tex.as_image_copy(), dst.tex.as_image_copy(), i.tex.size()),
                None => clear_pass(enc, &dst.view, wgpu::Color::TRANSPARENT),
            }
            self.custom.insert(shader.id, gpu);
            return;
        }

        if let Some(inp) = input
            && !gpu.screen_space
        {
            let gl = gpu.input_gl.take().unwrap_or_else(|| tex2d(&self.device, "user shader input", self.size.0, self.size.1, RENDER));
            self.flip(enc, inp, &gl, 3.0);
            gpu.input_gl = Some(gl);
        }

        let data = shader_uniforms(shader, self.size, self.size, gpu.frame, clock);
        let ub = self.uniform(&data);
        let pipe = gpu.pipe.as_ref().unwrap();
        // GLSL sees GL-oriented copies; WGSL sees tripslop's own (screen-space) textures.
        let (ch0, ch3) = if gpu.screen_space {
            (input.map(|t| &t.view).unwrap_or(&self.dummy_tex.view), &self.output.view)
        } else {
            (gpu.input_gl.as_ref().map(|t| &t.view).unwrap_or(&self.dummy_tex.view), &self.prev_output_gl.view)
        };
        let bufs = gpu.bufs.get_or_insert_with(|| pair(&self.device, "user shader", self.size));
        let prev = &bufs[1 - gpu.cur].view;
        let (fft, wave) = (&self.audio_tex[0].view, &self.audio_tex[1].view);
        let bg = self.bind_with(&self.repeat_sampler, &pipe.layout, ub, &[ch0, prev, &self.noise.view, ch3, fft, wave]);
        pass(enc, &bufs[gpu.cur].view, Some(wgpu::Color::TRANSPARENT), pipe, &bg);
        let out = &bufs[gpu.cur];
        let mode = shader.alpha.index() as f32;
        let flip = if gpu.screen_space { 0.0 } else { 1.0 };
        let ub = self.uniform(&[[mode, flip, 0.0, 0.0]]);
        let bg = self.bind(&self.flip_pipe.layout, ub, &[&out.view]);
        pass(enc, &dst.view, Some(wgpu::Color::TRANSPARENT), &self.flip_pipe, &bg);
        gpu.cur = 1 - gpu.cur;
        gpu.frame += 1;
        self.custom.insert(shader.id, gpu);
    }

    pub fn render(&mut self, comp: &mut Composition, punch: &mut Punch, clock: Clock) {
        self.next_uniform = 0;
        self.meshes.begin(true);
        let mut used_sources = HashSet::new();
        let mut used_layers = HashSet::new();
        let mut used_fx = HashSet::new();
        let mut used_custom = HashSet::new();
        let size = self.size;
        let aspect = size.0 as f32 / size.1 as f32;
        // Compile every user shader, drawn or not; keep GPU state for all that exist.
        let mut existing_shaders = HashSet::new();
        comp.for_each_shader(&mut |s| {
            existing_shaders.insert(s.id);
            self.compile_custom(s);
        });
        let mut enc = self.device.create_command_encoder(&Default::default());

        let comp_bufs = self.comp.take().expect("composition buffers");
        clear_pass(&mut enc, &comp_bufs[0].view, wgpu::Color::BLACK);
        let mut cur = 0;
        // Bank crossfade: B layers go into `bank_b`, A layers into `comp_bufs`, unassigned
        // layers into both. A bank that's faded out entirely isn't composited at all.
        let mix = comp.bank_mix();
        let bank_b = mix.map(|_| self.bank_b.take().unwrap_or_else(|| pair(&self.device, "bank b", size)));
        if let Some(b) = &bank_b {
            clear_pass(&mut enc, &b[0].view, wgpu::Color::BLACK);
        }
        let mut cur_b = 0;
        let (into_a, into_b) = match mix {
            Some(t) => (t < 1.0, t > 0.0),
            None => (true, false),
        };

        for li in 0..comp.layers.len() {
            if !comp.layer_audible(li) {
                continue;
            }
            let gain = comp.side_gain(comp.layers[li].side);
            let layer = &mut comp.layers[li];
            let opacity = layer.opacity.get() * gain * layer.perf.opacity;
            let draws = layer.draw_list();
            // A layer faded to zero still renders (cheaply skipping the composite), so its
            // effects keep their feedback history through punch-in gating.
            if draws.is_empty() || (opacity <= 0.0 && layer.perf.opacity >= 1.0) {
                continue;
            }
            used_layers.insert(layer.id);
            let bufs = self.layers.remove(&layer.id).unwrap_or_else(|| pair(&self.device, "layer", size));

            // Clips -> layer texture.
            clear_pass(&mut enc, &bufs[0].view, wgpu::Color::TRANSPARENT);
            for (col, weight) in draws {
                let clip = layer.clips[col].as_mut().unwrap();
                used_sources.insert(clip.id);
                if let Media::Shader(s) = &mut clip.media {
                    used_custom.insert(s.id);
                    let src = self.sources.remove(&clip.id).unwrap_or_else(|| SourceTex {
                        tex: tex2d(&self.device, "shader clip", size.0, size.1, RENDER),
                        size,
                    });
                    self.run_custom(&mut enc, s, None, &src.tex, clock);
                    self.sources.insert(clip.id, src);
                } else if let Media::Model(m) = &clip.media {
                    let src = self.sources.remove(&clip.id).unwrap_or_else(|| SourceTex {
                        tex: tex2d(&self.device, "model clip", size.0, size.1, RENDER),
                        size,
                    });
                    self.meshes.draw(&mut enc, meshes::clip_draw(&m.model.model, &m.params, clock, &src.tex.view, size));
                    self.sources.insert(clip.id, src);
                } else if !self.upload(clip) {
                    continue;
                }
                let (mode, tex_aspect, straight, gen_params) = match &clip.media {
                    Media::Generator(g) => (1.0, 1.0, 0.0, [g.pattern.get(), g.freq.get(), g.speed.get(), g.hue.get()]),
                    Media::Shader(_) | Media::Model(_) => (0.0, aspect, 0.0, [0.0; 4]),
                    _ => {
                        let s = self.sources[&clip.id].size;
                        (0.0, s.0 as f32 / s.1 as f32, 1.0, [0.0; 4])
                    }
                };
                let data = [
                    [clock.time as f32, aspect, mode, weight],
                    [
                        layer.pos_x.get() + layer.perf.x,
                        layer.pos_y.get() + layer.perf.y,
                        layer.scale.get() * layer.perf.scale,
                        (layer.rotation.get() + layer.perf.rotate).to_radians(),
                    ],
                    [clip.fit as u32 as f32, tex_aspect, gen_params[0], gen_params[1]],
                    [gen_params[2], gen_params[3], straight, 1.0 / size.1 as f32],
                ];
                let ub = self.uniform(&data);
                let view = self.sources.get(&clip.id).map(|s| &s.tex.view).unwrap_or(&self.dummy_tex.view);
                let bg = self.bind(&self.clip_pipe.layout, ub, &[view]);
                pass(&mut enc, &bufs[0].view, None, &self.clip_pipe, &bg);
            }

            // Layer effects.
            // The layer's own chain, then any punch-in effects aimed at this layer.
            let mut fx: Vec<&mut Effect> = layer.effects.iter_mut().collect();
            fx.extend(punch.layer_effects(layer.id));
            let lc = self.chain(&mut enc, fx, &bufs, 0, clock, &mut used_fx);

            // Composite onto the composition (or the layer's bank).
            if opacity > 0.0 {
                let blend = layer.blend as u32 as f32;
                let side = layer.side;
                if into_a && side != Side::B {
                    cur = self.composite(&mut enc, &comp_bufs, cur, &bufs[lc].view, blend, opacity);
                }
                if let Some(b) = bank_b.as_ref().filter(|_| into_b && side != Side::A) {
                    cur_b = self.composite(&mut enc, b, cur_b, &bufs[lc].view, blend, opacity);
                }
            }
            self.layers.insert(layer.id, bufs);
        }

        // Dissolve the banks. At either end the other bank was never drawn: use this one as is.
        let mut main = &comp_bufs;
        if let (Some(t), Some(b)) = (mix, &bank_b) {
            if t >= 1.0 {
                main = b;
                cur = cur_b;
            } else if t > 0.0 {
                cur = self.composite(&mut enc, &comp_bufs, cur, &b[cur_b].view, XFADE, t);
            }
        }

        // Master effects.
        let mut fx: Vec<&mut Effect> = comp.effects.iter_mut().collect();
        fx.extend(punch.master_effects());
        cur = self.chain(&mut enc, fx, main, cur, clock, &mut used_fx);

        let ub = self.uniform(&[[comp.master.get() * comp.master_perf, 0.0, 0.0, 0.0]]);
        let bg = self.bind(&self.final_pipe.layout, ub, &[&main[cur].view]);
        pass(&mut enc, &self.output.view, None, &self.final_pipe, &bg);
        self.comp = Some(comp_bufs);
        if bank_b.is_some() {
            self.bank_b = bank_b;
        }
        // Keep a GL-oriented copy for user shaders' iChannel3.
        let ub = self.uniform(&[[3.0, 1.0, 0.0, 0.0]]);
        let bg = self.bind(&self.flip_pipe.layout, ub, &[&self.output.view]);
        pass(&mut enc, &self.prev_output_gl.view, None, &self.flip_pipe, &bg);
        self.previews(&mut enc, comp);
        self.queue.submit([enc.finish()]);
        self.tick += 1;

        self.sources.retain(|id, _| used_sources.contains(id));
        self.layers.retain(|id, _| used_layers.contains(id));
        self.rings.retain(|id, _| used_fx.contains(id));
        // Free frame buffers of shaders not drawn this frame; drop deleted shaders entirely.
        self.custom.retain(|id, _| existing_shaders.contains(id));
        for (id, gpu) in self.custom.iter_mut() {
            if !used_fx.contains(id) && !used_custom.contains(id) {
                gpu.bufs = None;
                gpu.input_gl = None;
            }
        }
    }

    fn ensure_thumb(&mut self, clip_id: u64) {
        if !self.thumbs.contains_key(&clip_id) {
            let tex = tex2d(&self.device, "thumbnail", THUMB_W, THUMB_H, RENDER);
            let id = self.egui_renderer.write().register_native_texture(&self.device, &tex.view, wgpu::FilterMode::Linear);
            self.thumbs.insert(clip_id, Thumb { tex, id, signature: None });
        }
    }

    /// Thumbnail texture for a clip, if one has been rendered.
    pub fn thumbnails(&self) -> HashMap<u64, egui::TextureId> {
        self.thumbs.iter().map(|(k, t)| (*k, t.id)).collect()
    }

    /// Single-frame preview icons for generator and shader clips, rendered at a fixed
    /// moment. They're redrawn only when the content changes (a shader recompiles, or a
    /// generator's settings change), never by playback or automation. Also frees the
    /// thumbnails of deleted clips.
    fn previews(&mut self, enc: &mut wgpu::CommandEncoder, comp: &mut Composition) {
        let clock = Clock::new(PREVIEW_TIME * 2.0, PREVIEW_TIME, 120.0);
        let mut existing = HashSet::new();
        for layer in &comp.layers {
            for clip in layer.clips.iter().flatten() {
                existing.insert(clip.id);
                let signature = match &clip.media {
                    Media::Generator(g) => {
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        for p in [&g.pattern, &g.freq, &g.speed, &g.hue] {
                            std::hash::Hash::hash(&p.value.to_bits(), &mut h);
                        }
                        std::hash::Hasher::finish(&h)
                    }
                    // Only once a working version exists.
                    Media::Shader(sh) if self.custom.get(&sh.id).is_some_and(|g| g.pipe.is_some()) => {
                        sh.compiled_rev.unwrap_or(0)
                    }
                    Media::Model(m) => {
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        std::hash::Hash::hash(&m.model.model.id, &mut h);
                        for p in &m.params {
                            std::hash::Hash::hash(&p.value.to_bits(), &mut h);
                        }
                        std::hash::Hasher::finish(&h)
                    }
                    _ => continue,
                };
                if self.thumbs.get(&clip.id).is_some_and(|t| t.signature == Some(signature)) {
                    continue;
                }
                self.ensure_thumb(clip.id);
                match &clip.media {
                    Media::Generator(g) => {
                        let data = [
                            [clock.time as f32, THUMB_W as f32 / THUMB_H as f32, 1.0, 1.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [0.0, 1.0, g.pattern.value, g.freq.value],
                            [g.speed.value, g.hue.value, 0.0, 1.0 / THUMB_H as f32],
                        ];
                        let ub = self.uniform(&data);
                        let bg = self.bind(&self.clip_pipe.layout, ub, &[&self.dummy_tex.view]);
                        pass(enc, &self.thumbs[&clip.id].tex.view, Some(wgpu::Color::TRANSPARENT), &self.clip_pipe, &bg);
                    }
                    Media::Shader(sh) => {
                        let screen_space = self.custom[&sh.id].screen_space;
                        let data = shader_uniforms(sh, (THUMB_W, THUMB_H), self.size, 0, clock);
                        let ub = self.uniform(&data);
                        let pipe = self.custom[&sh.id].pipe.as_ref().unwrap();
                        let d = &self.dummy_tex.view;
                        let bg = self.bind_with(&self.repeat_sampler, &pipe.layout, ub, &[d, d, &self.noise.view, d, d, d]);
                        pass(enc, &self.preview_gl.view, Some(wgpu::Color::TRANSPARENT), pipe, &bg);
                        let flip = if screen_space { 0.0 } else { 1.0 };
                        let ub = self.uniform(&[[sh.alpha.index() as f32, flip, 0.0, 0.0]]);
                        let bg = self.bind(&self.flip_pipe.layout, ub, &[&self.preview_gl.view]);
                        pass(enc, &self.thumbs[&clip.id].tex.view, Some(wgpu::Color::TRANSPARENT), &self.flip_pipe, &bg);
                    }
                    Media::Model(m) => {
                        let target = &self.thumbs[&clip.id].tex.view;
                        self.meshes.draw(enc, meshes::clip_draw(&m.model.model, &m.params, clock, target, (THUMB_W, THUMB_H)));
                    }
                    _ => unreachable!(),
                }
                self.thumbs.get_mut(&clip.id).unwrap().signature = Some(signature);
            }
        }
        let gone: Vec<u64> = self.thumbs.keys().filter(|k| !existing.contains(k)).copied().collect();
        for k in gone {
            if let Some(t) = self.thumbs.remove(&k) {
                self.egui_renderer.write().free_texture(&t.id);
            }
        }
    }

    /// Forget all delay/feedback history.
    pub fn clear_history(&mut self) {
        self.rings.clear();
    }

    /// Card pictures for the shader library. New ones are made for the `visible` keys (a few
    /// per frame, within a time budget); the `hovered` one is redrawn every frame, `t` seconds
    /// into its animation.
    pub fn library_previews(&mut self, lib: &Library, visible: &[String], hovered: Option<(&str, f64)>) {
        self.lib_calls += 1;
        // Uniform slots are free again: everything rendered so far has been submitted.
        self.next_uniform = 0;
        self.meshes.begin(false);
        let start = web_time::Instant::now();
        let mut enc = self.device.create_command_encoder(&Default::default());
        let mut work = false;
        let hovered_key = hovered.map(|(k, _)| k);
        let order = hovered_key.into_iter().chain(visible.iter().map(String::as_str).filter(|k| Some(*k) != hovered_key));
        for key in order {
            let animate = hovered_key == Some(key);
            match self.lib_thumbs.get_mut(key) {
                Some(t) => {
                    t.last_seen = self.lib_calls;
                    if t.rendered && !animate {
                        continue;
                    }
                }
                None => {
                    if work && start.elapsed() > LIB_BUDGET {
                        continue;
                    }
                    let Some(entry) = lib.find(key).filter(|e| e.status == Status::Ok) else { continue };
                    let card = match entry.kind {
                        Kind::Model => lib.model(key).map(|m| self.card_texture(CardSource::Model(m.model))),
                        _ => self.library_card(entry),
                    };
                    let Ok(card) = card else { continue };
                    self.lib_thumbs.insert(key.to_string(), card);
                }
            }
            let t = if animate { PREVIEW_TIME + hovered.map_or(0.0, |(_, t)| t) } else { PREVIEW_TIME };
            let effect = lib.find(key).is_some_and(|e| e.kind == Kind::Effect);
            self.draw_library_card(&mut enc, key, t, effect);
            work = true;
        }
        if work {
            self.queue.submit([enc.finish()]);
        }
        // Forget the pictures that haven't been on screen for the longest.
        if self.lib_thumbs.len() > LIB_KEEP {
            let mut ages: Vec<(u64, String)> = self.lib_thumbs.iter().map(|(k, t)| (t.last_seen, k.clone())).collect();
            ages.sort();
            for (_, k) in ages.into_iter().take(self.lib_thumbs.len() - LIB_KEEP) {
                if let Some(t) = self.lib_thumbs.remove(&k) {
                    self.egui_renderer.write().free_texture(&t.id);
                }
            }
        }
    }

    /// GPU state for a shader's library card; `Err` if the shader can't be read.
    fn library_card(&mut self, entry: &crate::isf_library::Entry) -> Result<LibThumb, String> {
        let mut shader = entry.shader()?;
        let gpu = match shader.poll_compile() {
            Some(c) => self.build_custom(&c.wgsl, &c.entry).map(|p| (p, c.screen_space)),
            None => Err(shader.errors.first().map(|e| e.message.clone()).unwrap_or_else(|| "doesn't compile".into())),
        };
        if entry.kind == Kind::Effect && self.lib_input.is_none() {
            self.lib_input = Some(self.library_input());
        }
        Ok(self.card_texture(CardSource::Shader { shader, gpu }))
    }

    fn card_texture(&mut self, what: CardSource) -> LibThumb {
        let tex = tex2d(&self.device, "library card", LIB_W, LIB_H, RENDER);
        let id = self.egui_renderer.write().register_native_texture(&self.device, &tex.view, wgpu::FilterMode::Linear);
        LibThumb { tex, id, what, rendered: false, last_seen: self.lib_calls }
    }

    /// Render a library shader's card picture once and read it back, for `--bake-library`.
    /// `Err` says why the shader can't run.
    pub fn bake_library_card(&mut self, entry: &crate::isf_library::Entry) -> Result<image::RgbaImage, String> {
        let card = self.library_card(entry)?;
        if let CardSource::Shader { gpu: Err(e), .. } = &card.what {
            let e = e.clone();
            self.egui_renderer.write().free_texture(&card.id);
            return Err(e);
        }
        self.next_uniform = 0;
        let key = format!("bake:{}", entry.key);
        self.lib_thumbs.insert(key.clone(), card);
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.draw_library_card(&mut enc, &key, PREVIEW_TIME, entry.kind == Kind::Effect);
        self.queue.submit([enc.finish()]);
        let card = self.lib_thumbs.remove(&key).unwrap();
        let img = self.read_back(&card.tex.tex, (LIB_W, LIB_H));
        self.egui_renderer.write().free_texture(&card.id);
        img
    }

    /// GPU time to draw a library shader once at 1280×720, in milliseconds: the median of a
    /// few frames, each waited for. For `--bake-library`.
    pub fn time_library_shader(&mut self, entry: &crate::isf_library::Entry) -> Result<f32, String> {
        let mut shader = entry.shader()?;
        let c = shader.poll_compile().ok_or_else(|| shader.errors.first().map(|e| e.message.clone()).unwrap_or_else(|| "doesn't compile".into()))?;
        let pipe = self.build_custom(&c.wgsl, &c.entry)?;
        let size = (MEDIA_WIDTH, MEDIA_HEIGHT);
        let target = tex2d(&self.device, "shader timing", size.0, size.1, RENDER);
        let input = tex2d(&self.device, "shader timing input", size.0, size.1, RENDER);
        let mut times = Vec::new();
        for frame in 0..7 {
            self.next_uniform = 0;
            let t = PREVIEW_TIME + frame as f64 / 60.0;
            let data = shader_uniforms(&shader, size, size, frame, Clock::new(t * 2.0, t, 120.0));
            let ub = self.uniform(&data);
            let d = &self.dummy_tex.view;
            let bg = self.bind_with(&self.repeat_sampler, &pipe.layout, ub, &[&input.view, d, &self.noise.view, d, d, d]);
            let start = web_time::Instant::now();
            let mut enc = self.device.create_command_encoder(&Default::default());
            pass(&mut enc, &target.view, Some(wgpu::Color::TRANSPARENT), &pipe, &bg);
            self.queue.submit([enc.finish()]);
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            times.push(start.elapsed().as_secs_f32() * 1000.0);
        }
        // The first frame pays for pipeline warm-up.
        let mut times = times.split_off(1);
        times.sort_by(f32::total_cmp);
        Ok(times[times.len() / 2])
    }

    /// Wait for the GPU to finish everything submitted so far.
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    }

    /// A rendered library card picture.
    pub fn library_thumb(&self, key: &str) -> Option<egui::TextureId> {
        self.lib_thumbs.get(key).filter(|t| t.rendered).map(|t| t.id)
    }

    fn draw_library_card(&mut self, enc: &mut wgpu::CommandEncoder, key: &str, time: f64, effect: bool) {
        let clock = Clock::new(time * 2.0, time, 120.0);
        let Some(t) = self.lib_thumbs.get(key) else { return };
        let (shader, screen_space) = match &t.what {
            CardSource::Model(m) => {
                self.meshes.draw(enc, meshes::card_draw(m, clock, &t.tex.view, (LIB_W, LIB_H)));
                self.lib_thumbs.get_mut(key).unwrap().rendered = true;
                return;
            }
            CardSource::Shader { shader, gpu: Ok((_, screen_space)) } => (shader, *screen_space),
            CardSource::Shader { gpu: Err(_), .. } => return,
        };
        let frame = ((time - PREVIEW_TIME) * 60.0) as i32;
        let data = shader_uniforms(shader, (LIB_W, LIB_H), (LIB_W, LIB_H), frame, clock);
        let alpha = shader.alpha.index() as f32;
        let ub = self.uniform(&data);
        let flip_ub = self.uniform(&[[alpha, if screen_space { 0.0 } else { 1.0 }, 0.0, 0.0]]);
        let t = &self.lib_thumbs[key];
        let CardSource::Shader { gpu: Ok((pipe, _)), .. } = &t.what else { return };
        let d = &self.dummy_tex.view;
        let input = match (&self.lib_input, effect) {
            (Some(inp), true) => &inp[if screen_space { 0 } else { 1 }].view,
            _ => d,
        };
        let bg = self.bind_with(&self.repeat_sampler, &pipe.layout, ub, &[input, d, &self.noise.view, d, d, d]);
        pass(enc, &self.lib_gl.view, Some(wgpu::Color::TRANSPARENT), pipe, &bg);
        let bg = self.bind(&self.flip_pipe.layout, flip_ub, &[&self.lib_gl.view]);
        pass(enc, &t.tex.view, Some(wgpu::Color::BLACK), &self.flip_pipe, &bg);
        self.lib_thumbs.get_mut(key).unwrap().rendered = true;
    }

    /// The sample picture effects process in the library, cropped to the card size.
    fn library_input(&self) -> [Tex; 2] {
        let img = image::load_from_memory(LIB_INPUT).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(LIB_W, LIB_H));
        // Fill the card: crop the middle to its aspect ratio.
        let (w, h) = img.dimensions();
        let (cw, ch) = if w * LIB_H > h * LIB_W { (h * LIB_W / LIB_H, h) } else { (w, w * LIB_H / LIB_W) };
        let img = image::imageops::crop_imm(&img, (w - cw) / 2, (h - ch) / 2, cw, ch).to_image();
        let img = image::imageops::resize(&img, LIB_W, LIB_H, image::imageops::FilterType::Triangle);
        let flipped = image::imageops::flip_vertical(&img);
        [("library input", &img), ("library input (GL)", &flipped)].map(|(label, img)| {
            let t = tex2d(&self.device, label, LIB_W, LIB_H, RENDER);
            self.queue.write_texture(
                t.tex.as_image_copy(),
                img.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * LIB_W),
                    rows_per_image: Some(LIB_H),
                },
                wgpu::Extent3d {
                    width: LIB_W,
                    height: LIB_H,
                    depth_or_array_layers: 1,
                },
            );
            t
        })
    }

    /// Upload this tick's audio spectrum (0..1) and waveform (-1..1) for shaders.
    pub fn set_audio(&mut self, spectrum: &[f32], waveform: &[f32]) {
        for (tex, values, map) in [
            (&self.audio_tex[0], spectrum, (|v: f32| v) as fn(f32) -> f32),
            (&self.audio_tex[1], waveform, |v: f32| v * 0.5 + 0.5),
        ] {
            let bytes: Vec<u8> = values.iter().flat_map(|v| {
                let b = (map(*v).clamp(0.0, 1.0) * 255.0).round() as u8;
                [b, b, b, 255]
            }).collect();
            self.queue.write_texture(
                tex.tex.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * values.len() as u32),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: values.len() as u32,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Program size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Change the program size. Every render target is rebuilt, which also clears feedback
    /// and delay history and user shaders' previous frames.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), String> {
        let max = self.device.limits().max_texture_dimension_2d;
        if size.0 > max || size.1 > max {
            return Err(format!("{}x{} is bigger than this GPU allows ({max})", size.0, size.1));
        }
        if size == self.size {
            return Ok(());
        }
        self.size = size;
        self.comp = Some(pair(&self.device, "comp", size));
        self.bank_b = None;
        self.output = tex2d(&self.device, "output", size.0, size.1, RENDER);
        self.egui_renderer
            .write()
            .update_egui_texture_from_wgpu_texture(&self.device, &self.output.view, wgpu::FilterMode::Linear, self.display_id);
        self.prev_output_gl = tex2d(&self.device, "previous output (GL)", size.0, size.1, RENDER);
        self.mesh_scratch = None;
        self.wet_scratch = None;
        self.layers.clear();
        self.rings.clear();
        // Video and image sources are re-uploaded on the next frame; shader clips re-render.
        self.sources.clear();
        for gpu in self.custom.values_mut() {
            gpu.bufs = None;
            gpu.input_gl = None;
        }
        Ok(())
    }

    /// Rough GPU memory in use for render targets, history and sources, in bytes.
    pub fn gpu_memory(&self) -> u64 {
        let px = |(w, h): (u32, u32)| 4 * w as u64 * h as u64;
        let full = px(self.size);
        let mut total = full * 4; // comp pair, output, previous output
        total += self.bank_b.as_ref().map_or(0, |_| 2 * full);
        total += self.layers.len() as u64 * 2 * full;
        total += self.rings.values().map(|r| px(r.size) * r.len as u64).sum::<u64>();
        total += self.sources.values().map(|s| px(s.size)).sum::<u64>();
        for gpu in self.custom.values() {
            total += gpu.bufs.as_ref().map_or(0, |_| 2 * full) + gpu.input_gl.as_ref().map_or(0, |_| full);
        }
        total += self.mesh_scratch.as_ref().map_or(0, |_| full);
        total += self.wet_scratch.as_ref().map_or(0, |_| full);
        total + self.meshes.memory() + self.thumbs.len() as u64 * px((THUMB_W, THUMB_H))
    }

    /// Device, queue and the final output texture, for recording.
    pub fn output(&self) -> (&wgpu::Device, &wgpu::Queue, &wgpu::Texture) {
        (&self.device, &self.queue, &self.output.tex)
    }

    /// Read back the current output frame (blocking).
    pub fn snapshot(&self) -> Result<image::RgbaImage, String> {
        self.read_back(&self.output.tex, self.size)
    }

    /// Read back a texture (blocking).
    fn read_back(&self, tex: &wgpu::Texture, (w, h): (u32, u32)) -> Result<image::RgbaImage, String> {
        let row = padded_row(w);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("snapshot"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        buf.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let data = unpad_rows(&buf.slice(..).get_mapped_range().map_err(|e| format!("{e:?}"))?, w, h);
        image::RgbaImage::from_raw(w, h, data).ok_or_else(|| "snapshot size mismatch".into())
    }
}

/// Shadertoy iDate: (year, month 0-11, day 1-31, seconds since midnight), local time ≈ UTC.
fn date_now() -> [f32; 4] {
    let secs = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let days = (secs / 86400.0).floor() as i64;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    [year as f32, (month - 1) as f32, day as f32, (secs - days as f64 * 86400.0) as f32]
}

/// Uniform block for user shaders (layout matches `shader.rs`'s GLSL and WGSL preludes).
fn shader_uniforms(shader: &CustomShader, (width, height): (u32, u32), output: (u32, u32), frame: i32, clock: Clock) -> [[f32; 4]; 10 + crate::shader::MAX_PARAMS / 4] {
    let (w, h) = (width as f32, height as f32);
    // iMouse is in output pixels; scale it to the render size.
    let sx = w / output.0 as f32;
    let sy = h / output.1 as f32;
    let m = shader.mouse;
    let mut data = [[0.0f32; 4]; 10 + crate::shader::MAX_PARAMS / 4];
    data[0] = [w, h, 1.0, clock.time as f32];
    data[1] = [m[0] * sx, m[1] * sy, m[2] * sx, m[3] * sy];
    data[2] = date_now();
    data[3] = [w, h, 1.0, 0.0];
    data[4] = [w, h, 1.0, 0.0];
    data[5] = [NOISE_SIZE as f32, NOISE_SIZE as f32, 1.0, 0.0];
    data[6] = [w, h, 1.0, 0.0];
    data[7] = [1.0 / 60.0, f32::from_bits(frame as u32), clock.beat as f32, clock.bpm];
    let a = clock.audio;
    data[8] = [a.level, a.bass, a.mid, a.high];
    data[9] = [a.kick, if a.active { 1.0 } else { 0.0 }, a.centroid, 0.0];
    for (i, p) in shader.params.iter().enumerate().take(crate::shader::MAX_PARAMS) {
        data[10 + i / 4][i % 4] = p.get();
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_and_round_to_even() {
        assert_eq!(parse_size("720p"), Ok((1280, 720)));
        assert_eq!(parse_size("1080P"), Ok((1920, 1080)));
        assert_eq!(parse_size("1366x768"), Ok((1366, 768)));
        assert_eq!(parse_size("1001x601"), Ok((1000, 600)));
        assert!(parse_size("32x32").is_err());
        assert!(parse_size("wide").is_err());
    }

    #[test]
    fn readback_rows_are_aligned_and_unpadded() {
        assert_eq!(padded_row(1280), 5120);
        assert_eq!(padded_row(1920), 7680);
        assert_eq!(padded_row(1366), 5632); // 5464 rounded up to a multiple of 256
        // Two rows of 3 pixels, each padded to 256 bytes.
        let mut data = vec![0u8; 512];
        data[..12].copy_from_slice(&[1; 12]);
        data[256..268].copy_from_slice(&[2; 12]);
        let out = unpad_rows(&data, 3, 2);
        assert_eq!(out.len(), 24);
        assert!(out[..12].iter().all(|b| *b == 1) && out[12..].iter().all(|b| *b == 2));
    }

    #[test]
    fn half_history_is_a_quarter_of_the_pixels() {
        assert_eq!(history_size((1920, 1080), false), (1920, 1080));
        assert_eq!(history_size((1920, 1080), true), (960, 540));
    }
}
