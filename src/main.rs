//! trippy — a two-deck video DJ mixer with an emulated analog video-feedback rig.

mod automation_ui;
mod engine;
mod modulation;
mod params;
mod recorder;
mod source;

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Color32, Key, Rect, RichText, pos2};

use automation_ui::AutoCtx;
use engine::{Engine, HEIGHT, WIDTH};
use modulation::Clock;
use recorder::{Finishing, Recorder};
use params::*;
use source::{Source, Stream};

fn main() -> eframe::Result {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut preset = 1;
    let mut record = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--record" {
            record = true;
        } else if arg == "--preset" {
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
            let mut app = App::new(Engine::new(rs), files, preset - 1);
            if record {
                app.toggle_recording();
            }
            Ok(Box::new(app))
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
    /// Automated value of every slider in the last frame, indexed like `PARAMS`.
    live: Vec<f32>,
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
    recorder: Option<Recorder>,
    /// Recordings whose encoder is still finalizing the file.
    finishing: Vec<Finishing>,
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
            live: Vec::new(),
            taps: Vec::new(),
            freeze: false,
            perform: false,
            show_output: false,
            status: "Drop images/videos on a deck, or press 1-9 for presets. F = performance mode.".into(),
            fps: 60.0,
            preset: Some(0),
            grab_deck: 0,
            frame_count: 0,
            recorder: None,
            finishing: Vec::new(),
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
        if pressed(Key::R) {
            self.toggle_recording();
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

    fn toggle_recording(&mut self) {
        if let Some(rec) = self.recorder.take() {
            let (device, _, _) = self.engine.output();
            let fin = rec.stop(device);
            self.status = format!("Finishing {} …", fin.path.display());
            self.finishing.push(fin);
            return;
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = std::env::current_dir().unwrap_or_default().join(format!("trippy-{ts}.mp4"));
        let (device, _, _) = self.engine.output();
        match Recorder::start(device, path) {
            Ok(rec) => {
                self.status = format!("Recording to {} ({})", rec.path.display(), rec.encoder);
                self.recorder = Some(rec);
            }
            Err(e) => self.status = format!("Recording failed: {e}"),
        }
    }

    fn capture_frame(&mut self) {
        let Some(rec) = self.recorder.as_mut() else { return };
        let (device, queue, tex) = self.engine.output();
        if let Err(e) = rec.capture(device, queue, tex) {
            self.status = format!("Recording stopped: {e}");
            if let Some(rec) = self.recorder.take() {
                self.finishing.push(rec.stop(device));
            }
        }
    }

    /// Report recordings whose file has been finalized.
    fn poll_finished_recordings(&mut self) {
        let (done, pending): (Vec<_>, Vec<_>) = self.finishing.drain(..).partition(|f| f.is_done());
        self.finishing = pending;
        for f in done {
            self.status = match f.join() {
                Ok((path, frames)) => format!(
                    "Saved {} ({:.1}s)",
                    path.display(),
                    frames as f64 / recorder::FPS as f64
                ),
                Err(e) => format!("Recording failed: {e}"),
            };
        }
    }

    fn clock(&self) -> Clock {
        Clock {
            beat: self.beat,
            time: self.sim_time,
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
            let mut p = self.params.modulated(self.clock());
            self.live = PARAMS.iter().map(|d| d.get(&mut p)).collect();
            let [a, b] = &mut self.decks;
            self.engine
                .render(&p, self.sim_time as f32, self.freeze, a.source.as_mut(), b.source.as_mut());
            self.capture_frame();
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
            ui.label(RichText::new("trippy").strong().color(Color32::from_rgb(255, 120, 220)));
            ui.separator();
            ui.label("BPM");
            ui.add(egui::DragValue::new(&mut self.params.bpm).range(30.0..=300.0).speed(0.5).max_decimals(1));
            if ui.button("Tap (T)").clicked() {
                self.tap();
            }
            // Beat light.
            let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            let amber = Color32::from_rgb(255, 200, 0);
            if self.beat.fract() < 0.15 {
                ui.painter().circle_filled(r.center(), 5.0, amber);
            } else {
                ui.painter().circle_stroke(r.center(), 5.0, egui::Stroke::new(1.0, amber));
            }
            ui.separator();
            ui.toggle_value(&mut self.freeze, "❄ Freeze (Space)");
            if ui.button("Clear (C)").clicked() {
                self.engine.clear();
            }
            let rec_label = match &self.recorder {
                Some(r) => {
                    let secs = r.frames / recorder::FPS as u64;
                    RichText::new(format!("⏺ {}:{:02}  Stop (R)", secs / 60, secs % 60)).color(Color32::WHITE)
                }
                None => RichText::new("⏺ Record (R)"),
            };
            let mut rec_btn = egui::Button::new(rec_label);
            if self.recorder.is_some() {
                rec_btn = rec_btn.fill(Color32::from_rgb(200, 30, 40));
            }
            if ui.add(rec_btn).clicked() {
                self.toggle_recording();
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
            let clock = self.clock();
            let mut ac = AutoCtx {
                params: &mut self.params,
                live: &self.live,
                clock,
            };
            let k = |a: &'static str, b: &'static str| if i == 0 { a } else { b };
            let d = if i == 0 { &mut ac.params.deck_a } else { &mut ac.params.deck_b };
            ui.horizontal(|ui| {
                ui.add_enabled(has_src, egui::Checkbox::new(&mut d.use_pattern, "Oscillator"));
                combo(ui, ("pattern", i), &mut d.pattern, &Pattern::ALL, Pattern::name);
            });
            let show_osc = d.use_pattern || !has_src;
            ui.checkbox(&mut d.invert, "invert");
            if show_osc {
                ac.slider(ui, k("a.osc_freq", "b.osc_freq"));
                ac.slider(ui, k("a.osc_speed", "b.osc_speed"));
            }
            ac.slider(ui, k("a.gain", "b.gain"));
            ac.slider(ui, k("a.hue", "b.hue"));
            ui.separator();
            let d = if i == 0 { &mut ac.params.deck_a } else { &mut ac.params.deck_b };
            ui.horizontal(|ui| {
                ui.label("Placement");
                ui.selectable_value(&mut d.fit_whole, false, "Fill");
                ui.selectable_value(&mut d.fit_whole, true, "Fit");
                if ui.small_button("Reset").clicked() {
                    d.reset_placement();
                }
            });
            ac.slider(ui, k("a.scale", "b.scale"));
            ac.slider(ui, k("a.pos_x", "b.pos_x"));
            ac.slider(ui, k("a.pos_y", "b.pos_y"));
        });
        self.decks[i].rect = resp.response.rect;
    }

    fn mixer_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("MIXER").strong().size(16.0));
            let clock = self.clock();
            let mut ac = AutoCtx {
                params: &mut self.params,
                live: &self.live,
                clock,
            };
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(100.0);
            ac.slider(ui, "crossfade");
            combo(ui, "blend", &mut self.params.blend, &BlendMode::ALL, BlendMode::name);
            ui.label(RichText::new("Z / X = cut to A / B, left / right arrows = fade").weak().small());
        });
    }

    fn fx_ui(&mut self, ui: &mut egui::Ui) {
        let clock = self.clock();
        let mut ac = AutoCtx {
            params: &mut self.params,
            live: &self.live,
            clock,
        };
        egui::CollapsingHeader::new(RichText::new("FEEDBACK / FRACTAL").strong())
            .default_open(true)
            .show(ui, |ui| {
                for key in ["feedback", "copies", "zoom", "rotate", "spread", "twist", "center_x", "center_y"] {
                    ac.slider(ui, key);
                }
                let fx = &mut ac.params.fx;
                combo(ui, "combine", &mut fx.combine, &CopyCombine::ALL, CopyCombine::name);
                combo(ui, "edges", &mut fx.edge, &EdgeMode::ALL, EdgeMode::name);
                combo(ui, "symmetry", &mut fx.symmetry, &Symmetry::ALL, Symmetry::name);
                if fx.symmetry == Symmetry::Kaleido {
                    ac.slider(ui, "kaleido_segments");
                }
            });
        egui::CollapsingHeader::new(RichText::new("LOOP COLOR").strong())
            .default_open(true)
            .show(ui, |ui| {
                for key in ["hue_shift", "saturation", "contrast", "blur", "noise"] {
                    ac.slider(ui, key);
                }
            });
        egui::CollapsingHeader::new(RichText::new("KEYER (input over loop)").strong())
            .default_open(true)
            .show(ui, |ui| {
                combo(ui, "input mode", &mut ac.params.fx.input_mode, &InputMode::ALL, InputMode::name);
                ac.slider(ui, "input_level");
                if ac.params.fx.input_mode == InputMode::LumaKey {
                    ac.slider(ui, "key_threshold");
                    ac.slider(ui, "key_softness");
                }
            });
        egui::CollapsingHeader::new(RichText::new("VIDEO DELAY").strong())
            .default_open(true)
            .show(ui, |ui| {
                for key in ["loop_delay", "echo_amount", "echo_spacing", "chroma_delay", "chroma_amount"] {
                    ac.slider(ui, key);
                }
            });
        egui::CollapsingHeader::new(RichText::new("OUTPUT").strong())
            .default_open(false)
            .show(ui, |ui| {
                for key in ["out_hue", "brightness", "posterize", "scanlines", "vignette"] {
                    ac.slider(ui, key);
                }
                ui.checkbox(&mut ac.params.fx.out_invert, "invert");
            });
        egui::CollapsingHeader::new(RichText::new("AUTOMATION").strong())
            .default_open(true)
            .show(ui, |ui| automation_ui::overview(ui, ac.params, clock));
    }
}

fn deck_name(i: usize) -> &'static str {
    if i == 0 { "A" } else { "B" }
}

fn deck_color(i: usize) -> Color32 {
    if i == 0 { Color32::from_rgb(80, 200, 255) } else { Color32::from_rgb(255, 140, 60) }
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
    /// Make sure a recording in progress ends up as a playable file.
    fn on_exit(&mut self) {
        if let Some(rec) = self.recorder.take() {
            let (device, _, _) = self.engine.output();
            self.finishing.push(rec.stop(device));
        }
        for f in self.finishing.drain(..) {
            match f.join() {
                Ok((path, frames)) => eprintln!("saved {} ({frames} frames)", path.display()),
                Err(e) => eprintln!("recording failed: {e}"),
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_finished_recordings();
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
                    if self.recorder.is_some() {
                        // Shown on the preview only; the recorded frames come from the engine.
                        let c = rect.right_top() + egui::vec2(-16.0, 16.0);
                        ui.painter().circle_filled(c, 7.0, Color32::from_rgb(230, 30, 40));
                    }
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
