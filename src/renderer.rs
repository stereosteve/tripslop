//! GPU pipeline, per tick:
//!
//! ```text
//! for each layer (bottom → top):
//!     clip(s) ──clip pass (transform, fit, transition)──► layer tex
//!     layer tex ──effect → effect → …──► (ping-pong)          (delay rings per effect)
//!     comp ──composite (blend mode, opacity × crossfader)──► comp
//! comp ──master effects──► comp ──final (master fader)──► output ─► screen / recorder
//! ```
//!
//! GPU resources are keyed by the model's ids (clip, layer, effect) and created on demand;
//! anything not used in a frame is freed.

use std::collections::{HashMap, HashSet};

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

use crate::clip::{Clip, Media};
use crate::composition::Composition;
use crate::effects::{EFFECTS, Effect, EffectKind, HistorySource};
use crate::modulation::Clock;
use crate::punch::Punch;
use crate::shader::{CompileError, CustomShader};

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const RENDER: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);
const NOISE_SIZE: u32 = 256;
pub const THUMB_W: u32 = 192;
pub const THUMB_H: u32 = 108;
/// Moment at which generator / shader previews are rendered (seconds, beats).
const PREVIEW_TIME: f64 = 2.0;

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
    /// Composition ping-pong buffers (taken out of `self` while rendering).
    comp: Option<[Tex; 2]>,
    output: Tex,
    pub display_id: egui::TextureId,
    // User shaders.
    vs_module: wgpu::ShaderModule,
    flip_pipe: Pipe,
    repeat_sampler: wgpu::Sampler,
    noise: Tex,
    /// Last frame's output in GL orientation (`iChannel3`).
    prev_output_gl: Tex,
    custom: HashMap<u64, CustomGpu>,
    // Clip grid thumbnails.
    egui_renderer: std::sync::Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    thumbs: HashMap<u64, Thumb>,
    /// GL-oriented scratch target for shader previews.
    preview_gl: Tex,
    tick: u64,
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

fn ring(device: &wgpu::Device, len: u32) -> Ring {
    let tex = texture(
        device,
        "history ring",
        WIDTH,
        HEIGHT,
        len,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    Ring { tex, view, len, head: 0 }
}

fn copy_to_ring(enc: &mut wgpu::CommandEncoder, from: &wgpu::Texture, ring: &Ring) {
    enc.copy_texture_to_texture(
        from.as_image_copy(),
        wgpu::TexelCopyTextureInfo {
            texture: &ring.tex,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: 0, z: ring.head },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
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
        let prev_output_gl = tex2d(&device, "previous output (GL)", WIDTH, HEIGHT, RENDER);
        let preview_gl = tex2d(&device, "shader preview", THUMB_W, THUMB_H, RENDER);
        let dummy_ring = ring(&device, 1).view;
        let comp = Some([
            tex2d(&device, "comp a", WIDTH, HEIGHT, RENDER),
            tex2d(&device, "comp b", WIDTH, HEIGHT, RENDER),
        ]);
        let output = tex2d(&device, "output", WIDTH, HEIGHT, RENDER);
        let display_id = rs
            .renderer
            .write()
            .register_native_texture(&device, &output.view, wgpu::FilterMode::Linear);

        Self {
            device,
            queue,
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
            output,
            display_id,
            vs_module,
            flip_pipe,
            repeat_sampler,
            noise,
            prev_output_gl,
            custom: HashMap::new(),
            egui_renderer: rs.renderer.clone(),
            thumbs: HashMap::new(),
            preview_gl,
            tick: 0,
        }
    }

    /// A fresh uniform buffer for this pass (each pass in a frame needs its own data).
    fn uniform(&mut self, data: &[[f32; 4]]) -> usize {
        if self.next_uniform == self.uniforms.len() {
            self.uniforms.push(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass uniforms"),
                size: 256,
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
        let def = e.def();
        let mut taps = [0.0f32; 4];
        if let Some(h) = def.history {
            let ring = self.rings.entry(e.id).or_insert_with(|| ring(&self.device, h.frames));
            if h.source == HistorySource::Input {
                // The current input becomes tap "0 frames ago".
                copy_to_ring(enc, &src.tex, ring);
            }
            let t = (h.taps)(&e.params);
            for (i, ago) in t.iter().enumerate() {
                // Output history: the newest stored frame is 1 frame ago.
                let ago = if h.source == HistorySource::Output { (*ago).max(1) } else { *ago };
                taps[i] = ring.layer_ago(ago);
            }
        }
        let mut data = [[0.0f32; 4]; 9];
        data[0] = [clock.time as f32, WIDTH as f32 / HEIGHT as f32, 1.0 / WIDTH as f32, 1.0 / HEIGHT as f32];
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
            let ring = self.rings.get_mut(&e.id).unwrap();
            if h.source == HistorySource::Output {
                copy_to_ring(enc, &dst.tex, ring);
            }
            ring.head = (ring.head + 1) % ring.len;
        }
    }

    /// Run an effect chain, ping-ponging between `bufs`. Returns the index holding the result.
    fn chain(&mut self, enc: &mut wgpu::CommandEncoder, effects: Vec<&mut Effect>, bufs: &[Tex; 2], mut cur: usize, clock: Clock, used: &mut HashSet<u64>) -> usize {
        for e in effects.into_iter().filter(|e| e.enabled) {
            used.insert(e.id);
            if let Some(c) = e.custom.as_deref_mut() {
                used.insert(c.id);
                self.run_custom(enc, c, Some(&bufs[cur]), &bufs[1 - cur], clock);
            } else {
                self.effect(enc, e, &bufs[cur], &bufs[1 - cur], clock);
            }
            cur = 1 - cur;
        }
        cur
    }

    /// Compile a user shader's WGSL into a pipeline, catching GPU validation errors.
    fn build_custom(&self, wgsl: &str, entry: &str) -> Result<Pipe, String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fs = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("user shader"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let layout = bind_layout(&self.device, &[false; 4]);
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
            let gl = gpu.input_gl.take().unwrap_or_else(|| tex2d(&self.device, "user shader input", WIDTH, HEIGHT, RENDER));
            self.flip(enc, inp, &gl, 3.0);
            gpu.input_gl = Some(gl);
        }

        let data = shader_uniforms(shader, WIDTH, HEIGHT, gpu.frame, clock);
        let ub = self.uniform(&data);
        let pipe = gpu.pipe.as_ref().unwrap();
        // GLSL sees GL-oriented copies; WGSL sees tripslop's own (screen-space) textures.
        let (ch0, ch3) = if gpu.screen_space {
            (input.map(|t| &t.view).unwrap_or(&self.dummy_tex.view), &self.output.view)
        } else {
            (gpu.input_gl.as_ref().map(|t| &t.view).unwrap_or(&self.dummy_tex.view), &self.prev_output_gl.view)
        };
        let bufs = gpu.bufs.get_or_insert_with(|| {
            [
                tex2d(&self.device, "user shader a", WIDTH, HEIGHT, RENDER),
                tex2d(&self.device, "user shader b", WIDTH, HEIGHT, RENDER),
            ]
        });
        let prev = &bufs[1 - gpu.cur].view;
        let bg = self.bind_with(&self.repeat_sampler, &pipe.layout, ub, &[ch0, prev, &self.noise.view, ch3]);
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
        let mut used_sources = HashSet::new();
        let mut used_layers = HashSet::new();
        let mut used_fx = HashSet::new();
        let mut used_custom = HashSet::new();
        let aspect = WIDTH as f32 / HEIGHT as f32;
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
            let bufs = self
                .layers
                .remove(&layer.id)
                .unwrap_or_else(|| [tex2d(&self.device, "layer a", WIDTH, HEIGHT, RENDER), tex2d(&self.device, "layer b", WIDTH, HEIGHT, RENDER)]);

            // Clips -> layer texture.
            clear_pass(&mut enc, &bufs[0].view, wgpu::Color::TRANSPARENT);
            for (col, weight) in draws {
                let clip = layer.clips[col].as_mut().unwrap();
                used_sources.insert(clip.id);
                if let Media::Shader(s) = &mut clip.media {
                    used_custom.insert(s.id);
                    let src = self.sources.remove(&clip.id).unwrap_or_else(|| SourceTex {
                        tex: tex2d(&self.device, "shader clip", WIDTH, HEIGHT, RENDER),
                        size: (WIDTH, HEIGHT),
                    });
                    self.run_custom(&mut enc, s, None, &src.tex, clock);
                    self.sources.insert(clip.id, src);
                } else if !self.upload(clip) {
                    continue;
                }
                let (mode, tex_aspect, straight, gen_params) = match &clip.media {
                    Media::Generator(g) => (1.0, 1.0, 0.0, [g.pattern.get(), g.freq.get(), g.speed.get(), g.hue.get()]),
                    Media::Shader(_) => (0.0, aspect, 0.0, [0.0; 4]),
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
                    [gen_params[2], gen_params[3], straight, 0.0],
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

            // Composite onto the composition.
            if opacity > 0.0 {
                let ub = self.uniform(&[[layer.blend as u32 as f32, opacity, 0.0, 0.0]]);
                let bg = self.bind(&self.composite_pipe.layout, ub, &[&comp_bufs[cur].view, &bufs[lc].view]);
                pass(&mut enc, &comp_bufs[1 - cur].view, None, &self.composite_pipe, &bg);
                cur = 1 - cur;
            }
            self.layers.insert(layer.id, bufs);
        }

        // Master effects.
        let mut fx: Vec<&mut Effect> = comp.effects.iter_mut().collect();
        fx.extend(punch.master_effects());
        cur = self.chain(&mut enc, fx, &comp_bufs, cur, clock, &mut used_fx);

        let ub = self.uniform(&[[comp.master.get() * comp.master_perf, 0.0, 0.0, 0.0]]);
        let bg = self.bind(&self.final_pipe.layout, ub, &[&comp_bufs[cur].view]);
        pass(&mut enc, &self.output.view, None, &self.final_pipe, &bg);
        self.comp = Some(comp_bufs);
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
        let clock = Clock {
            beat: PREVIEW_TIME * 2.0,
            time: PREVIEW_TIME,
            bpm: 120.0,
        };
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
                    _ => continue,
                };
                if self.thumbs.get(&clip.id).is_some_and(|t| t.signature == Some(signature)) {
                    continue;
                }
                self.ensure_thumb(clip.id);
                match &clip.media {
                    Media::Generator(g) => {
                        let data = [
                            [clock.time as f32, WIDTH as f32 / HEIGHT as f32, 1.0, 1.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [0.0, 1.0, g.pattern.value, g.freq.value],
                            [g.speed.value, g.hue.value, 0.0, 0.0],
                        ];
                        let ub = self.uniform(&data);
                        let bg = self.bind(&self.clip_pipe.layout, ub, &[&self.dummy_tex.view]);
                        pass(enc, &self.thumbs[&clip.id].tex.view, Some(wgpu::Color::TRANSPARENT), &self.clip_pipe, &bg);
                    }
                    Media::Shader(sh) => {
                        let screen_space = self.custom[&sh.id].screen_space;
                        let data = shader_uniforms(sh, THUMB_W, THUMB_H, 0, clock);
                        let ub = self.uniform(&data);
                        let pipe = self.custom[&sh.id].pipe.as_ref().unwrap();
                        let d = &self.dummy_tex.view;
                        let bg = self.bind_with(&self.repeat_sampler, &pipe.layout, ub, &[d, d, &self.noise.view, d]);
                        pass(enc, &self.preview_gl.view, Some(wgpu::Color::TRANSPARENT), pipe, &bg);
                        let flip = if screen_space { 0.0 } else { 1.0 };
                        let ub = self.uniform(&[[sh.alpha.index() as f32, flip, 0.0, 0.0]]);
                        let bg = self.bind(&self.flip_pipe.layout, ub, &[&self.preview_gl.view]);
                        pass(enc, &self.thumbs[&clip.id].tex.view, Some(wgpu::Color::TRANSPARENT), &self.flip_pipe, &bg);
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

    /// Device, queue and the final output texture, for recording.
    pub fn output(&self) -> (&wgpu::Device, &wgpu::Queue, &wgpu::Texture) {
        (&self.device, &self.queue, &self.output.tex)
    }

    /// Read back the current output frame (blocking).
    pub fn snapshot(&self) -> Result<image::RgbaImage, String> {
        let row = 4 * WIDTH;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("snapshot"),
            size: (row * HEIGHT) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.output.tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(HEIGHT),
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        buf.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let data = buf.slice(..).get_mapped_range().map_err(|e| format!("{e:?}"))?.to_vec();
        image::RgbaImage::from_raw(WIDTH, HEIGHT, data).ok_or_else(|| "snapshot size mismatch".into())
    }
}

/// Shadertoy iDate: (year, month 0-11, day 1-31, seconds since midnight), local time ≈ UTC.
fn date_now() -> [f32; 4] {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
fn shader_uniforms(shader: &CustomShader, width: u32, height: u32, frame: i32, clock: Clock) -> [[f32; 4]; 12] {
    let (w, h) = (width as f32, height as f32);
    // iMouse is in output pixels; scale it to the render size.
    let sx = w / WIDTH as f32;
    let sy = h / HEIGHT as f32;
    let m = shader.mouse;
    let mut data = [[0.0f32; 4]; 12];
    data[0] = [w, h, 1.0, clock.time as f32];
    data[1] = [m[0] * sx, m[1] * sy, m[2] * sx, m[3] * sy];
    data[2] = date_now();
    data[3] = [w, h, 1.0, 0.0];
    data[4] = [w, h, 1.0, 0.0];
    data[5] = [NOISE_SIZE as f32, NOISE_SIZE as f32, 1.0, 0.0];
    data[6] = [w, h, 1.0, 0.0];
    data[7] = [1.0 / 60.0, f32::from_bits(frame as u32), clock.beat as f32, clock.bpm];
    for (i, p) in shader.params.iter().enumerate().take(crate::shader::MAX_PARAMS) {
        data[8 + i / 4][i % 4] = p.get();
    }
    data
}
