//! trippy — a two-deck video DJ mixer with an emulated analog video-feedback rig.

mod engine;
mod params;
mod source;

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Color32, Key, Rect, RichText, pos2};

use engine::{Engine, HEIGHT, MAX_DELAY, WIDTH};
use params::*;
use source::{Source, Stream};

fn main() -> eframe::Result {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut preset = 1;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--preset" {
            preset = args.next().and_then(|v| v.parse().ok()).filter(|n| (1..=9).contains(n)).unwrap_or(1);
        } else {
            files.push(PathBuf::from(arg));
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("trippy — video feedback DJ")
            .with_inner_size([1600.0, 900.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "trippy",
        options,
        Box::new(move |cc| {
            let rs = cc
                .wgpu_render_state
                .as_ref()
                .ok_or("trippy needs the wgpu renderer")?;
            Ok(Box::new(App::new(Engine::new(rs), files, preset - 1)))
        }),
    )
}

struct Deck {
    source: Option<Source>,
    camera_index: u32,
    /// Screen rect of this deck's controls, for drag-and-drop targeting.
    rect: Rect,
}

impl Deck {
    fn new() -> Self {
        Self {
            source: None,
            camera_index: 0,
            rect: Rect::NOTHING,
        }
    }
}

struct App {
    engine: Engine,
    params: Params,
    decks: [Deck; 2],
    sim_time: f64,
    last_frame: Instant,
    beat: f64,
    taps: Vec<Instant>,
    freeze: bool,
    perform: bool,
    show_output: bool,
    status: String,
    fps: f32,
    preset: Option<usize>,
    /// Which deck dragging / scrolling on the preview moves and resizes.
    grab_deck: usize,
    frame_count: u64,
    /// Testing hook: `TRIPPY_SNAPSHOT=<frames>:<out.png>` saves a frame and quits.
    auto_snapshot: Option<(u64, PathBuf)>,
}

impl App {
    fn new(engine: Engine, files: Vec<PathBuf>, preset: usize) -> Self {
        let mut app = Self {
            engine,
            params: Params::default(),
            decks: [Deck::new(), Deck::new()],
            sim_time: 0.0,
            last_frame: Instant::now(),
            beat: 0.0,
            taps: Vec::new(),
            freeze: false,
            perform: false,
            show_output: false,
            status: "Drop images/videos on a deck, or press 1-9 for presets. F = performance mode.".into(),
            fps: 60.0,
            preset: Some(0),
            grab_deck: 0,
            frame_count: 0,
            auto_snapshot: std::env::var("TRIPPY_SNAPSHOT").ok().and_then(|v| {
                let (n, p) = v.split_once(':')?;
                Some((n.parse().ok()?, PathBuf::from(p)))
            }),
        };
        apply_preset(&mut app.params, preset);
        app.preset = Some(preset);
        for (i, f) in files.into_iter().take(2).enumerate() {
            app.load_file(i, f);
        }
        app
    }

    fn deck_params(&mut self, i: usize) -> &mut DeckParams {
        if i == 0 { &mut self.params.deck_a } else { &mut self.params.deck_b }
    }

    fn load_file(&mut self, deck: usize, path: PathBuf) {
        match source::open_file(&path, WIDTH, HEIGHT) {
            Ok(src) => {
                self.status = format!("Deck {}: loaded {}", deck_name(deck), src.name());
                self.decks[deck].source = Some(src);
                self.deck_params(deck).use_pattern = false;
            }
            Err(e) => self.status = format!("Error: {e}"),
        }
    }

    fn open_camera(&mut self, deck: usize) {
        match Stream::camera(self.decks[deck].camera_index, WIDTH, HEIGHT) {
            Ok(s) => {
                self.decks[deck].source = Some(Source::Stream(s));
                self.deck_params(deck).use_pattern = false;
                self.status = format!("Deck {}: camera opened", deck_name(deck));
            }
            Err(e) => self.status = format!("Error: {e}"),
        }
    }

    fn tap(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.taps.last()
            && now.duration_since(*last).as_secs_f32() > 2.0
        {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        if self.taps.len() >= 2 {
            let span = now.duration_since(self.taps[0]).as_secs_f32();
            let bpm = 60.0 * (self.taps.len() - 1) as f32 / span;
            self.params.bpm = bpm.clamp(30.0, 300.0);
            // Re-align phase to the tap so LFOs land on the beat.
            self.beat = self.beat.round();
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        // Drag and drop: onto a deck's controls, else into the deck that is *not* live.
        let (dropped, pointer) = ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.hover_pos()));
        for (n, f) in dropped.iter().enumerate() {
            let target = match pointer {
                Some(p) if self.decks[0].rect.contains(p) => 0,
                Some(p) if self.decks[1].rect.contains(p) => 1,
                _ if dropped.len() > 1 => n.min(1),
                _ => {
                    if self.params.crossfade >= 0.5 { 0 } else { 1 }
                }
            };
            self.load_file(target, f.path().to_path_buf());
        }

        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let keys = [
            Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9,
        ];
        for (i, k) in keys.iter().enumerate() {
            if ctx.input(|inp| inp.key_pressed(*k)) {
                apply_preset(&mut self.params, i);
                self.preset = Some(i);
            }
        }
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::F) {
            self.perform = !self.perform;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.perform));
        }
        if pressed(Key::Escape) && self.perform {
            self.perform = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        if pressed(Key::Space) {
            self.freeze = !self.freeze;
        }
        if pressed(Key::T) {
            self.tap();
        }
        if pressed(Key::C) {
            self.engine.clear();
        }
        if pressed(Key::G) {
            self.grab_deck = 1 - self.grab_deck;
        }
        if pressed(Key::S) {
            self.save_snapshot(None);
        }
        if pressed(Key::Z) {
            self.params.crossfade = 0.0;
        }
        if pressed(Key::X) {
            self.params.crossfade = 1.0;
        }
        let held = |k: Key| ctx.input(|i| i.key_down(k));
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if held(Key::ArrowLeft) {
            self.params.crossfade = (self.params.crossfade - dt).max(0.0);
        }
        if held(Key::ArrowRight) {
            self.params.crossfade = (self.params.crossfade + dt).min(1.0);
        }
        if held(Key::ArrowUp) {
            self.params.fx.zoom = (self.params.fx.zoom + dt * 0.1).min(2.0);
        }
        if held(Key::ArrowDown) {
            self.params.fx.zoom = (self.params.fx.zoom - dt * 0.1).max(0.05);
        }
    }

    fn save_snapshot(&mut self, path: Option<PathBuf>) {
        let path = path.unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            PathBuf::from(format!("trippy-{ts}.png"))
        });
        self.status = match self.engine.snapshot().and_then(|img| img.save(&path).map_err(|e| e.to_string())) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Snapshot failed: {e}"),
        };
    }

    /// Advance the feedback loop on a fixed 60Hz clock, so the trip looks the same on a
    /// 60Hz, 120Hz or unthrottled display and delay times are real-time frames.
    fn render_frame(&mut self) {
        const TICK: f64 = 1.0 / 60.0;
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64();
        if dt < TICK {
            return;
        }
        let ticks = ((dt / TICK) as u32).min(3);
        // Carry the remainder, but drop time we couldn't keep up with.
        self.last_frame = if dt > TICK * 4.0 { now } else { self.last_frame + std::time::Duration::from_secs_f64(TICK * ticks as f64) };
        self.fps = self.fps * 0.95 + (1.0 / dt as f32) * 0.05;
        for _ in 0..ticks {
            self.beat += TICK * self.params.bpm as f64 / 60.0;
            self.sim_time += TICK;
            self.frame_count += 1;
            let p = self.params.modulated(self.beat);
            let [a, b] = &mut self.decks;
            self.engine
                .render(&p, self.sim_time as f32, self.freeze, a.source.as_mut(), b.source.as_mut());
        }
    }

    // ------------------------------------------------------------------ UI

    /// Drag on the preview to move the grabbed deck, scroll / pinch to resize it.
    fn preview_interaction(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect) {
        let i = self.grab_deck;
        let d = self.deck_params(i);
        if resp.dragged() {
            let delta = resp.drag_delta();
            d.pos_x += delta.x / rect.width();
            d.pos_y -= delta.y / rect.height();
        }
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|inp| (inp.smooth_scroll_delta.y, inp.zoom_delta()));
            if scroll != 0.0 || zoom != 1.0 {
                d.scale = (d.scale * (scroll * 0.004).exp() * zoom).clamp(0.05, 3.0);
            }
            // Outline the grabbed deck's box and show a hint.
            let center = rect.center() + egui::vec2(d.pos_x * rect.width(), -d.pos_y * rect.height());
            let deck_rect = Rect::from_center_size(center, rect.size() * d.scale);
            let color = deck_color(i);
            let painter = ui.painter_at(rect);
            painter.rect_stroke(deck_rect, 0.0, egui::Stroke::new(1.5, color), egui::StrokeKind::Middle);
            painter.text(
                rect.left_top() + egui::vec2(8.0, 8.0),
                egui::Align2::LEFT_TOP,
                format!("Deck {}: drag = move, scroll = size  (G switches deck)", deck_name(i)),
                egui::FontId::proportional(13.0),
                color,
            );
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("◉ trippy").strong().color(Color32::from_rgb(255, 120, 220)));
            ui.separator();
            ui.label("BPM");
            ui.add(egui::DragValue::new(&mut self.params.bpm).range(30.0..=300.0).speed(0.5).max_decimals(1));
            if ui.button("Tap (T)").clicked() {
                self.tap();
            }
            let beat_on = self.beat.fract() < 0.15;
            ui.label(RichText::new(if beat_on { "●" } else { "○" }).color(Color32::from_rgb(255, 200, 0)));
            ui.separator();
            ui.toggle_value(&mut self.freeze, "❄ Freeze (Space)");
            if ui.button("Clear (C)").clicked() {
                self.engine.clear();
            }
            ui.toggle_value(&mut self.show_output, "🖵 Output window");
            ui.label("Grab:");
            ui.selectable_value(&mut self.grab_deck, 0, "A");
            ui.selectable_value(&mut self.grab_deck, 1, "B");
            if ui.button("Perform (F)").clicked() {
                self.perform = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
            ui.separator();
            ui.label("Presets:");
            for (i, name) in PRESET_NAMES.iter().enumerate() {
                if ui.selectable_label(self.preset == Some(i), *name).clicked() {
                    apply_preset(&mut self.params, i);
                    self.preset = Some(i);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.status).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0} fps", self.fps)).weak());
            });
        });
    }

    fn deck_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let color = deck_color(i);
        let resp = egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("DECK {}", deck_name(i))).strong().size(16.0).color(color));
            let src_name = self.decks[i].source.as_ref().map(|s| s.name().to_string());
            let err = self.decks[i].source.as_ref().and_then(|s| s.error());
            ui.label(match &src_name {
                Some(n) => format!("▶ {n}"),
                None => "(no media — drop a file here)".into(),
            });
            if let Some(e) = err {
                ui.colored_label(Color32::RED, e);
            }
            ui.horizontal(|ui| {
                if ui.button("Open…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter(
                            "media",
                            &["png", "jpg", "jpeg", "gif", "webp", "bmp", "mp4", "mov", "m4v", "mkv", "webm", "avi"],
                        )
                        .pick_file()
                {
                    self.load_file(i, path);
                }
                if ui.button("Camera").clicked() {
                    self.open_camera(i);
                }
                ui.add(egui::DragValue::new(&mut self.decks[i].camera_index).range(0..=9).prefix("#"));
                if src_name.is_some() && ui.button("Eject").clicked() {
                    self.decks[i].source = None;
                    self.deck_params(i).use_pattern = true;
                }
            });
            let has_src = src_name.is_some();
            let d = self.deck_params(i);
            ui.horizontal(|ui| {
                ui.add_enabled(has_src, egui::Checkbox::new(&mut d.use_pattern, "Oscillator"));
                combo(ui, ("pattern", i), &mut d.pattern, &Pattern::ALL, Pattern::name);
            });
            if d.use_pattern || !has_src {
                ui.add(egui::Slider::new(&mut d.osc_freq, 0.5..=40.0).logarithmic(true).text("freq"));
                ui.add(egui::Slider::new(&mut d.osc_speed, 0.0..=4.0).text("speed"));
            }
            ui.add(egui::Slider::new(&mut d.gain, 0.0..=2.0).text("gain"));
            ui.add(egui::Slider::new(&mut d.hue, 0.0..=1.0).text("hue"));
            ui.checkbox(&mut d.invert, "invert");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Placement");
                ui.selectable_value(&mut d.fit_whole, false, "Fill");
                ui.selectable_value(&mut d.fit_whole, true, "Fit");
                if ui.small_button("Reset").clicked() {
                    d.reset_placement();
                }
            });
            ui.add(egui::Slider::new(&mut d.scale, 0.05..=3.0).logarithmic(true).text("size"));
            ui.add(egui::Slider::new(&mut d.pos_x, -1.0..=1.0).text("x"));
            ui.add(egui::Slider::new(&mut d.pos_y, -1.0..=1.0).text("y"));
        });
        self.decks[i].rect = resp.response.rect;
    }

    fn mixer_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("MIXER").strong().size(16.0));
            ui.horizontal(|ui| {
                ui.label("A");
                ui.spacing_mut().slider_width = ui.available_width() - 30.0;
                ui.add(egui::Slider::new(&mut self.params.crossfade, 0.0..=1.0).show_value(false));
                ui.label("B");
            });
            combo(ui, "blend", &mut self.params.blend, &BlendMode::ALL, BlendMode::name);
            ui.label(RichText::new("Z / X = cut to A / B, ←/→ = fade").weak().small());
        });
    }

    fn fx_ui(&mut self, ui: &mut egui::Ui) {
        let fx = &mut self.params.fx;
        egui::CollapsingHeader::new(RichText::new("FEEDBACK / FRACTAL").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.add(egui::Slider::new(&mut fx.feedback, 0.0..=1.2).text("feedback"));
                ui.add(egui::Slider::new(&mut fx.copies, 1..=8).text("copies (monitors)"));
                ui.add(egui::Slider::new(&mut fx.zoom, 0.05..=2.0).text("copy scale"));
                ui.add(egui::Slider::new(&mut fx.rotate, -180.0..=180.0).text("rotate °"));
                ui.add(egui::Slider::new(&mut fx.spread, 0.0..=1.5).text("spread"));
                ui.add(egui::Slider::new(&mut fx.twist, -180.0..=180.0).text("twist ° / copy"));
                ui.add(egui::Slider::new(&mut fx.center_x, -0.8..=0.8).text("center x"));
                ui.add(egui::Slider::new(&mut fx.center_y, -0.5..=0.5).text("center y"));
                combo(ui, "combine", &mut fx.combine, &CopyCombine::ALL, CopyCombine::name);
                combo(ui, "edges", &mut fx.edge, &EdgeMode::ALL, EdgeMode::name);
                combo(ui, "symmetry", &mut fx.symmetry, &Symmetry::ALL, Symmetry::name);
                if fx.symmetry == Symmetry::Kaleido {
                    ui.add(egui::Slider::new(&mut fx.kaleido_segments, 2..=16).text("segments"));
                }
            });
        egui::CollapsingHeader::new(RichText::new("LOOP COLOR").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.add(egui::Slider::new(&mut fx.hue_shift, -0.1..=0.1).text("hue / pass"));
                ui.add(egui::Slider::new(&mut fx.saturation, 0.0..=2.0).text("saturation"));
                ui.add(egui::Slider::new(&mut fx.contrast, 0.5..=2.0).text("contrast"));
                ui.add(egui::Slider::new(&mut fx.blur, 0.0..=1.0).text("blur / soften"));
                ui.add(egui::Slider::new(&mut fx.noise, 0.0..=1.0).text("noise"));
            });
        egui::CollapsingHeader::new(RichText::new("KEYER (input over loop)").strong())
            .default_open(true)
            .show(ui, |ui| {
                combo(ui, "input mode", &mut fx.input_mode, &InputMode::ALL, InputMode::name);
                ui.add(egui::Slider::new(&mut fx.input_level, 0.0..=1.5).text("input level"));
                if fx.input_mode == InputMode::LumaKey {
                    ui.add(egui::Slider::new(&mut fx.key_threshold, 0.0..=1.0).text("key threshold"));
                    ui.add(egui::Slider::new(&mut fx.key_softness, 0.0..=0.5).text("key softness"));
                }
            });
        egui::CollapsingHeader::new(RichText::new("VIDEO DELAY").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.add(egui::Slider::new(&mut fx.loop_delay, 1..=MAX_DELAY).text("loop delay (frames)"));
                ui.add(egui::Slider::new(&mut fx.echo_amount, 0.0..=1.0).text("echo"));
                ui.add(egui::Slider::new(&mut fx.echo_spacing, 1..=MAX_DELAY / 3).text("echo spacing"));
                ui.add(egui::Slider::new(&mut fx.chroma_delay, 0..=MAX_DELAY / 2).text("RGB split (frames)"));
                ui.add(egui::Slider::new(&mut fx.chroma_amount, 0.0..=1.0).text("RGB split amount"));
            });
        egui::CollapsingHeader::new(RichText::new("OUTPUT").strong())
            .default_open(false)
            .show(ui, |ui| {
                ui.add(egui::Slider::new(&mut fx.out_hue, 0.0..=1.0).text("hue"));
                ui.add(egui::Slider::new(&mut fx.brightness, 0.0..=2.0).text("brightness"));
                ui.add(egui::Slider::new(&mut fx.posterize, 0..=16).text("posterize"));
                ui.add(egui::Slider::new(&mut fx.scanlines, 0.0..=1.0).text("scanlines"));
                ui.add(egui::Slider::new(&mut fx.vignette, 0.0..=1.0).text("vignette"));
                ui.checkbox(&mut fx.out_invert, "invert");
            });
        egui::CollapsingHeader::new(RichText::new("LFOs (beat-synced)").strong())
            .default_open(true)
            .show(ui, |ui| {
                for (n, lfo) in self.params.lfos.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}", n + 1));
                        combo(ui, ("lfo target", n), &mut lfo.target, &LfoTarget::ALL, LfoTarget::name);
                        combo(ui, ("lfo shape", n), &mut lfo.shape, &LfoShape::ALL, LfoShape::name);
                    });
                    if lfo.target != LfoTarget::Off {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt(("lfo beats", n))
                                .selected_text(beats_label(lfo.beats))
                                .width(70.0)
                                .show_ui(ui, |ui| {
                                    for b in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0] {
                                        ui.selectable_value(&mut lfo.beats, b, beats_label(b));
                                    }
                                });
                            ui.add(egui::Slider::new(&mut lfo.depth, 0.0..=1.0).text("depth"));
                        });
                    }
                    ui.add_space(4.0);
                }
            });
    }
}

fn deck_name(i: usize) -> &'static str {
    if i == 0 { "A" } else { "B" }
}

fn deck_color(i: usize) -> Color32 {
    if i == 0 { Color32::from_rgb(80, 200, 255) } else { Color32::from_rgb(255, 140, 60) }
}

fn beats_label(b: f32) -> String {
    if b < 1.0 { format!("1/{} beat", (1.0 / b).round()) } else { format!("{b} beats") }
}

fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + Copy + std::fmt::Debug,
    value: &mut T,
    all: &[T],
    name: fn(T) -> &'static str,
) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(name(*value))
            .show_ui(ui, |ui| {
                for v in all {
                    ui.selectable_value(value, *v, name(*v));
                }
            });
    });
}

/// Paint the output texture letterboxed to 16:9 into the available space.
/// Returns the response for the whole area and the rect the picture occupies.
fn paint_output(ui: &mut egui::Ui, tex: egui::TextureId) -> (egui::Response, Rect) {
    let avail = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(avail, egui::Sense::click_and_drag());
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut size = avail.size();
    if size.x / size.y > aspect {
        size.x = size.y * aspect;
    } else {
        size.y = size.x / aspect;
    }
    let rect = Rect::from_center_size(avail.center(), size);
    let painter = ui.painter();
    painter.rect_filled(avail, 0.0, Color32::BLACK);
    painter.image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    (resp, rect)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_input(&ctx);
        self.render_frame();
        let tex = self.engine.display_id;

        if self.show_output {
            let mut open = true;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("trippy output"),
                egui::ViewportBuilder::default()
                    .with_title("trippy output (double-click = fullscreen)")
                    .with_inner_size([960.0, 540.0]),
                |ui, _class| {
                    let (resp, _) = paint_output(ui, tex);
                    if resp.double_clicked() {
                        let fs = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
                    }
                    if ui.input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                },
            );
            self.show_output = open;
        }

        if self.perform {
            let (resp, _) = paint_output(ui, tex);
            if resp.double_clicked() {
                self.perform = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        } else {
            egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
            egui::Panel::left("decks")
                .default_size(300.0)
                .resizable(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.deck_ui(ui, 0);
                        ui.add_space(6.0);
                        self.mixer_ui(ui);
                        ui.add_space(6.0);
                        self.deck_ui(ui, 1);
                    });
                });
            egui::Panel::right("fx")
                .default_size(320.0)
                .resizable(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.fx_ui(ui));
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(Color32::BLACK))
                .show(ui, |ui| {
                    let (resp, rect) = paint_output(ui, tex);
                    self.preview_interaction(ui, &resp, rect);
                });
        }

        if let Some((n, path)) = self.auto_snapshot.clone()
            && self.frame_count >= n
        {
            self.save_snapshot(Some(path));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 120.0));
    }
}
