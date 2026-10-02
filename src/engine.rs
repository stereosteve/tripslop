//! GPU pipeline:
//!
//! ```text
//!  deck A ─┐
//!          ├─ mix pass ─► mix_tex ─┐
//!  deck B ─┘                       ├─ fx pass ─► out_tex ─┬─ copy ─► ring[head]  (video delay line)
//!            ring[head - delay] ───┘   ▲                  └─ post pass ─► display_tex ─► screen
//!                                      └── feedback: N copies of a delayed frame
//! ```

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

use crate::params::Params;
use crate::source::Source;

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
/// Frames kept in the delay line. ~1s at 60fps, ~236MB of VRAM at 720p.
pub const RING_LAYERS: u32 = 64;
pub const MAX_DELAY: u32 = RING_LAYERS - 4;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct DeckTex {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
    has_content: bool,
}

pub struct Engine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    sampler: wgpu::Sampler,

    mix_pipe: wgpu::RenderPipeline,
    fx_pipe: wgpu::RenderPipeline,
    post_pipe: wgpu::RenderPipeline,

    mix_ubuf: wgpu::Buffer,
    fx_ubuf: wgpu::Buffer,
    post_ubuf: wgpu::Buffer,

    decks: [DeckTex; 2],
    mix_view: wgpu::TextureView,
    out_tex: wgpu::Texture,
    out_view: wgpu::TextureView,
    display_tex: wgpu::Texture,
    display_view: wgpu::TextureView,
    ring: wgpu::Texture,

    mix_bg: wgpu::BindGroup,
    fx_bg: wgpu::BindGroup,
    post_bg: wgpu::BindGroup,

    /// Ring layer that the next output frame will be written to.
    head: u32,
    pub display_id: egui::TextureId,
}

fn shader(device: &wgpu::Device, label: &str, body: &str) -> wgpu::ShaderModule {
    let src = format!("{}\n{}", include_str!("shaders/common.wgsl"), body);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}

fn pipeline(device: &wgpu::Device, label: &str, module: &wgpu::ShaderModule) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: None,
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
            targets: &[Some(FORMAT.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
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

fn ubuf(device: &wgpu::Device, label: &str, vec4s: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (vec4s * 16) as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn bind_group(
    device: &wgpu::Device,
    pipe: &wgpu::RenderPipeline,
    ubuf: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    views: &[&wgpu::TextureView],
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: ubuf.as_entire_binding(),
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
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipe.get_bind_group_layout(0),
        entries: &entries,
    })
}

const RENDER: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING);

impl Engine {
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

        let mix_pipe = pipeline(&device, "mix", &shader(&device, "mix", include_str!("shaders/mix.wgsl")));
        let fx_pipe = pipeline(&device, "fx", &shader(&device, "fx", include_str!("shaders/fx.wgsl")));
        let post_pipe = pipeline(&device, "post", &shader(&device, "post", include_str!("shaders/post.wgsl")));

        let mix_ubuf = ubuf(&device, "mix u", 7);
        let fx_ubuf = ubuf(&device, "fx u", 9);
        let post_ubuf = ubuf(&device, "post u", 3);

        let decks = [Self::deck_tex(&device, 1, 1), Self::deck_tex(&device, 1, 1)];
        let mix_tex = texture(&device, "mix", WIDTH, HEIGHT, 1, RENDER);
        let mix_view = mix_tex.create_view(&Default::default());
        let out_tex = texture(&device, "out", WIDTH, HEIGHT, 1, RENDER | wgpu::TextureUsages::COPY_SRC);
        let out_view = out_tex.create_view(&Default::default());
        let display_tex = texture(&device, "display", WIDTH, HEIGHT, 1, RENDER | wgpu::TextureUsages::COPY_SRC);
        let display_view = display_tex.create_view(&Default::default());
        let ring = texture(
            &device,
            "delay ring",
            WIDTH,
            HEIGHT,
            RING_LAYERS,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let ring_view = ring.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let mix_bg = bind_group(&device, &mix_pipe, &mix_ubuf, &sampler, &[&decks[0].view, &decks[1].view]);
        let fx_bg = bind_group(&device, &fx_pipe, &fx_ubuf, &sampler, &[&mix_view, &ring_view]);
        let post_bg = bind_group(&device, &post_pipe, &post_ubuf, &sampler, &[&out_view, &ring_view]);

        let display_id = rs.renderer.write().register_native_texture(&device, &display_view, wgpu::FilterMode::Linear);

        Self {
            device,
            queue,
            sampler,
            mix_pipe,
            fx_pipe,
            post_pipe,
            mix_ubuf,
            fx_ubuf,
            post_ubuf,
            decks,
            mix_view,
            out_tex,
            out_view,
            display_tex,
            display_view,
            ring,
            mix_bg,
            fx_bg,
            post_bg,
            head: 0,
            display_id,
        }
    }

    fn deck_tex(device: &wgpu::Device, w: u32, h: u32) -> DeckTex {
        let tex = texture(
            device,
            "deck",
            w,
            h,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let view = tex.create_view(&Default::default());
        DeckTex {
            tex,
            view,
            size: (w, h),
            has_content: false,
        }
    }

    fn upload_deck(&mut self, i: usize, src: Option<&mut Source>) {
        let Some(src) = src else {
            self.decks[i].has_content = false;
            return;
        };
        let Some(frame) = src.poll() else { return };
        let f = frame.get();
        if self.decks[i].size != (f.width, f.height) {
            self.decks[i] = Self::deck_tex(&self.device, f.width, f.height);
            self.mix_bg = bind_group(
                &self.device,
                &self.mix_pipe,
                &self.mix_ubuf,
                &self.sampler,
                &[&self.decks[0].view, &self.decks[1].view],
            );
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.decks[i].tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &f.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * f.width),
                rows_per_image: Some(f.height),
            },
            wgpu::Extent3d {
                width: f.width,
                height: f.height,
                depth_or_array_layers: 1,
            },
        );
        self.decks[i].has_content = true;
    }

    /// Ring layer that was written `frames_ago` frames ago (1 = last frame).
    fn layer_ago(&self, frames_ago: u32) -> f32 {
        let ago = frames_ago.clamp(1, RING_LAYERS - 1);
        ((self.head + RING_LAYERS - ago) % RING_LAYERS) as f32
    }

    /// Wipe the delay line / feedback memory.
    pub fn clear(&mut self) {
        let zeros = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        for layer in 0..RING_LAYERS {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ring,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                &zeros,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * WIDTH),
                    rows_per_image: Some(HEIGHT),
                },
                wgpu::Extent3d {
                    width: WIDTH,
                    height: HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    pub fn render(&mut self, p: &Params, time: f32, freeze: bool, deck_a: Option<&mut Source>, deck_b: Option<&mut Source>) {
        self.upload_deck(0, deck_a);
        self.upload_deck(1, deck_b);

        let aspect = WIDTH as f32 / HEIGHT as f32;
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        let deck_u = |d: &crate::params::DeckParams, has: bool| -> [[f32; 4]; 3] {
            [
                [b(d.use_pattern || !has), d.pattern as u32 as f32, d.osc_freq, d.osc_speed],
                [d.gain, d.hue, b(d.invert), b(has)],
                [d.scale, d.pos_x, d.pos_y, b(d.fit_whole)],
            ]
        };
        let da = deck_u(&p.deck_a, self.decks[0].has_content);
        let db = deck_u(&p.deck_b, self.decks[1].has_content);
        let mix_u: [[f32; 4]; 7] = [
            [time, p.crossfade, p.blend as u32 as f32, aspect],
            da[0],
            da[1],
            db[0],
            db[1],
            da[2],
            db[2],
        ];
        self.queue.write_buffer(&self.mix_ubuf, 0, bytemuck::cast_slice(&mix_u));

        let fx = &p.fx;
        let spacing = fx.echo_spacing.max(1);
        let fx_u: [[f32; 4]; 9] = [
            [time, aspect, fx.feedback, fx.copies as f32],
            [fx.zoom, fx.rotate.to_radians(), fx.spread, fx.twist.to_radians()],
            [fx.center_x, fx.center_y, fx.combine as u32 as f32, fx.edge as u32 as f32],
            [fx.symmetry as u32 as f32, fx.kaleido_segments as f32, fx.hue_shift, fx.saturation],
            [fx.contrast, fx.blur, fx.noise, fx.input_mode as u32 as f32],
            [fx.input_level, fx.key_threshold, fx.key_softness, fx.echo_amount],
            [
                self.layer_ago(fx.loop_delay),
                self.layer_ago(spacing),
                self.layer_ago(spacing * 2),
                self.layer_ago(spacing * 3),
            ],
            [0.0; 4],
            [1.0 / WIDTH as f32, 1.0 / HEIGHT as f32, 0.0, 0.0],
        ];
        self.queue.write_buffer(&self.fx_ubuf, 0, bytemuck::cast_slice(&fx_u));

        let mut enc = self.device.create_command_encoder(&Default::default());
        Self::pass(&mut enc, &self.mix_view, &self.mix_pipe, &self.mix_bg);
        if !freeze {
            Self::pass(&mut enc, &self.out_view, &self.fx_pipe, &self.fx_bg);
            enc.copy_texture_to_texture(
                self.out_tex.as_image_copy(),
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ring,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: self.head },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: WIDTH,
                    height: HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
            self.head = (self.head + 1) % RING_LAYERS;
        }

        // Post uniforms are written after the head moved: layer_ago(1) is this frame.
        let post_u: [[f32; 4]; 3] = [
            [time, fx.out_hue, b(fx.out_invert), fx.posterize as f32],
            [fx.scanlines, fx.vignette, fx.brightness, HEIGHT as f32],
            [
                aspect,
                if fx.chroma_delay > 0 { fx.chroma_amount } else { 0.0 },
                self.layer_ago(1 + fx.chroma_delay),
                self.layer_ago(1 + fx.chroma_delay * 2),
            ],
        ];
        self.queue.write_buffer(&self.post_ubuf, 0, bytemuck::cast_slice(&post_u));
        Self::pass(&mut enc, &self.display_view, &self.post_pipe, &self.post_bg);
        self.queue.submit([enc.finish()]);
    }

    /// Device, queue and the final output texture, for recording.
    pub fn output(&self) -> (&wgpu::Device, &wgpu::Queue, &wgpu::Texture) {
        (&self.device, &self.queue, &self.display_tex)
    }

    /// Read back the current output frame (blocking).
    pub fn snapshot(&self) -> Result<image::RgbaImage, String> {
        let row = 4 * WIDTH; // 5120: already a multiple of COPY_BYTES_PER_ROW_ALIGNMENT
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("snapshot"),
            size: (row * HEIGHT) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.display_tex.as_image_copy(),
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

    fn pass(enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipe);
        rp.set_bind_group(0, bg, &[]);
        rp.draw(0..3, 0..1);
    }
}
