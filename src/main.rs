//! tripslop — a live video mixer: layered clip grid with scenes, per-layer effects and an
//! emulated analog video-feedback rig.

mod audio;
mod automation;
mod clip;
mod composition;
mod effects;
mod isf_library;
mod midi;
mod modulation;
mod param;
mod punch;
mod recorder;
mod renderer;
mod script;
mod shader;
mod source;
mod ui;
mod video;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, Rect, RichText, pos2};

use clip::{Clip, LoopMode};
use composition::{Blend, Composition, Launch, Quantize};
use effects::{Effect, EffectKind, apply_feedback_preset};
use modulation::{Clock, Shape};
use recorder::{Finishing, Recorder};
use renderer::{MEDIA_HEIGHT, MEDIA_WIDTH, Renderer};
use ui::grid::{GridAction, GridView};

const TICK: f64 = 1.0 / 60.0;

const USAGE: &str =
    "usage: tripslop [--demo] [--record] [--script FILE] [--isf DIR]... [--size WxH|720p|1080p] [--fixed-step | --realtime] [MEDIA...]
       tripslop --bake-library   (re-check the bundled shaders and render their library pictures)";

/// Window / dock icon: the color flower, rasterized centered on a square transparent canvas.
fn app_icon() -> egui::IconData {
    const SIZE: u32 = 512;
    let svg = include_bytes!("../logos/tripslop-flower-color.svg");
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).expect("flower logo svg");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE).unwrap();
    let (w, h) = (tree.size().width(), tree.size().height());
    let scale = SIZE as f32 * 0.9 / w.max(h);
    let (dx, dy) = ((SIZE as f32 - w * scale) / 2.0, (SIZE as f32 - h * scale) / 2.0);
    let xf = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, xf, &mut pixmap.as_mut());
    // tiny-skia is premultiplied; IconData wants straight alpha.
    let rgba = pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    egui::IconData { rgba: rgba.collect(), width: SIZE, height: SIZE }
}

fn main() -> eframe::Result {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut record = false;
    let mut demo = false;
    let mut script_path: Option<PathBuf> = None;
    let mut fixed_step: Option<bool> = None;
    let mut isf_dirs: Vec<PathBuf> = Vec::new();
    let mut size = None;
    let mut bake = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--record" => record = true,
            "--demo" => demo = true,
            "--script" => match args.next() {
                Some(p) => script_path = Some(PathBuf::from(p)),
                None => exit_with(USAGE, 2),
            },
            "--isf" => match args.next() {
                Some(p) => isf_dirs.push(PathBuf::from(p)),
                None => exit_with(USAGE, 2),
            },
            "--size" => match args.next().map(|s| renderer::parse_size(&s)) {
                Some(Ok(s)) => size = Some(s),
                Some(Err(e)) => exit_with(&e, 2),
                None => exit_with(USAGE, 2),
            },
            "--bake-library" => bake = true,
            "--fixed-step" => fixed_step = Some(true),
            "--realtime" => fixed_step = Some(false),
            "-h" | "--help" => exit_with(USAGE, 0),
            a if a.starts_with("--") => exit_with(&format!("unknown option {a}\n{USAGE}"), 2),
            _ => files.push(PathBuf::from(arg)),
        }
    }
    let mut events = Vec::new();
    if let Some(path) = &script_path {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| exit_with(&format!("{}: {e}", path.display()), 2));
        events = script::parse(&src).unwrap_or_else(|e| exit_with(&format!("{}: {e}", path.display()), 2));
    }
    // Shortcut kept for quick checks: TRIPSLOP_SNAPSHOT=<frames>:<out.png>.
    if let Some((n, p)) = std::env::var("TRIPSLOP_SNAPSHOT").ok().and_then(|v| {
        let (n, p) = v.split_once(':')?;
        Some((n.parse::<u64>().ok()?, PathBuf::from(p)))
    }) {
        for cmd in [script::Cmd::Snapshot(p), script::Cmd::Quit] {
            events.push(script::Event { at: script::When::Frame(n), cmd, line: 0 });
        }
    }
    // Scripts run in fixed-step mode unless told otherwise: one tick per frame, so results
    // don't depend on machine speed.
    let fixed_step = fixed_step.unwrap_or(script_path.is_some());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("tripslop")
            .with_inner_size([1680.0, 1000.0])
            .with_drag_and_drop(true)
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "tripslop",
        options,
        Box::new(move |cc| {
            let rs = cc.wgpu_render_state.as_ref().ok_or("tripslop needs the wgpu renderer")?;
            ui::theme::apply(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            if bake {
                // A build-time chore that needs the GPU: write assets/isf/baked.json and thumbs/, then quit.
                let mut renderer = Renderer::new(rs);
                match isf_library::bake(|e| renderer.bake_library_card(e)) {
                    Ok(summary) => exit_with(&summary, 0),
                    Err(e) => exit_with(&format!("bake failed: {e}"), 1),
                }
            }
            let mut app = App::new(Renderer::new(rs), isf_dirs);
            app.fixed_step = fixed_step;
            // Scripts stay away from the hardware and the user's saved mappings, unless a config
            // dir is given explicitly (the MIDI tests do, to check mappings survive a restart).
            if script_path.is_some() {
                app.midi = midi::Midi::new(std::env::var_os("TRIPSLOP_CONFIG_DIR").map(PathBuf::from));
                app.midi.port = midi::Port::Off;
            } else {
                app.midi = midi::Midi::new(midi::config_dir());
                let port = app.midi.port.clone();
                app.midi.connect(port);
            }
            if let Some(s) = size {
                app.set_size(s)?;
            }
            app.script = events.into_iter().map(|event| automation::Pending { event, done: false }).collect();
            if demo {
                app.load_demo();
            }
            let any_files = !files.is_empty();
            for (col, f) in files.into_iter().enumerate() {
                app.load_file(0, col, f);
            }
            if any_files && app.comp.layers[0].active.is_none() {
                app.comp.layers[0].launch(0);
            }
            if record {
                app.toggle_recording();
            }
            Ok(Box::new(app))
        }),
    )?;
    if let Some(s) = automation::summary() {
        println!("{s}");
    }
    if automation::FAILED.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(1);
    }
    Ok(())
}

fn exit_with(msg: &str, code: i32) -> ! {
    if code == 0 {
        println!("{msg}");
    } else {
        eprintln!("{msg}");
    }
    std::process::exit(code)
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Layer,
    Composition,
}

struct App {
    renderer: Renderer,
    comp: Composition,
    punch: punch::Punch,
    grid: GridView,
    tab: Tab,
    sim_time: f64,
    beat: f64,
    last_frame: Instant,
    taps: Vec<Instant>,
    perform: bool,
    show_output: bool,
    status: String,
    /// Ticks (output frames) per second, measured by `update_fps`.
    fps: f32,
    /// Start of the current fps measurement and the tick count then.
    fps_window: (Instant, u64),
    frame_count: u64,
    recorder: Option<Recorder>,
    finishing: Vec<Finishing>,
    /// Shader open in the code editor.
    editing: Option<u64>,
    library: isf_library::Library,
    library_view: ui::library::LibraryView,
    show_library: bool,
    /// Pointer position (output pixels, y up) where the current iMouse drag started.
    mouse_click: Option<[f32; 2]>,
    /// `--script` events (see `script.rs`).
    script: Vec<automation::Pending>,
    /// One simulation tick per UI frame instead of real time (deterministic; for scripts).
    fixed_step: bool,
    /// Output frame to save after the current tick renders.
    pending_snapshot: Option<PathBuf>,
    /// Full-window screenshot to save (egui delivers it a frame later).
    pending_screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    quit_requested: bool,
    /// Sound input and its analysis.
    audio: audio::Audio,
    /// MIDI input, mappings and learn state.
    midi: midi::Midi,
    /// Custom size typed into the Output settings, applied with its button.
    size_edit: (u32, u32),
}

impl App {
    fn new(renderer: Renderer, isf_dirs: Vec<PathBuf>) -> Self {
        Self {
            renderer,
            comp: Composition::new(3, 8),
            punch: punch::Punch::new(),
            grid: GridView {
                selected_layer: 0,
                selected_clip: None,
                cells: Vec::new(),
            },
            tab: Tab::Layer,
            sim_time: 0.0,
            beat: 0.0,
            last_frame: Instant::now(),
            taps: Vec::new(),
            perform: false,
            show_output: false,
            status: "Drop media onto the grid, or right-click a cell. Number keys launch scenes.".into(),
            fps: 60.0,
            fps_window: (Instant::now(), 0),
            frame_count: 0,
            recorder: None,
            finishing: Vec::new(),
            editing: None,
            show_library: true,
            library: isf_library::Library::new(isf_dirs),
            library_view: Default::default(),
            mouse_click: None,
            script: Vec::new(),
            fixed_step: false,
            pending_snapshot: None,
            pending_screenshot: None,
            screenshot_requested: false,
            quit_requested: false,
            audio: Default::default(),
            // Replaced in `main` (scripts don't touch the saved mappings or the hardware).
            midi: midi::Midi::new(None),
            size_edit: renderer::DEFAULT_SIZE,
        }
    }

    fn clock(&self) -> Clock {
        Clock {
            beat: self.beat,
            time: self.sim_time,
            bpm: self.comp.bpm,
            audio: self.audio.levels,
        }
    }

    fn load_file(&mut self, layer: usize, col: usize, path: PathBuf) {
        if let Err(e) = self.try_load(layer, col, &path) {
            self.status = format!("Error: {e}");
        }
    }

    fn try_load(&mut self, layer: usize, col: usize, path: &std::path::Path) -> Result<(), String> {
        let c = Clip::open(path, MEDIA_WIDTH, MEDIA_HEIGHT)?;
        self.status = format!("Loaded {} into {} / column {}", c.name, self.comp.layers[layer].name, col + 1);
        self.comp.set_clip(layer, col, c);
        self.grid.selected_clip = Some((layer, col));
        Ok(())
    }

    /// Example set built from `samples/`.
    fn load_demo(&mut self) {
        let s = |f: &str| PathBuf::from("samples").join(f);
        // Layer 1: full-frame footage.
        self.load_file(0, 0, s("jellyfish.mp4"));
        self.load_file(0, 1, s("big-buck-bunny.mp4"));
        self.comp.set_clip(0, 2, Clip::generator(2));
        self.load_file(0, 3, s("jellyfish.mp4"));
        if let Some(c) = self.comp.clip_mut(0, 3) {
            c.loop_mode = LoopMode::Bounce;
            c.speed.set(0.5);
        }
        // Layer 2: small seeds feeding a fractal feedback rig.
        self.load_file(1, 0, s("crab-nebula.jpg"));
        self.comp.set_clip(1, 1, Clip::generator(4));
        self.load_file(1, 2, s("pillars-of-creation.jpg"));
        self.load_file(1, 3, s("crab-nebula.jpg"));
        for col in [0, 2, 3] {
            if let Some(c) = self.comp.clip_mut(1, col) {
                c.fit = clip::Fit::Contain;
            }
        }
        let l2 = &mut self.comp.layers[1];
        l2.name = "Fractal".into();
        l2.scale.set(0.42);
        l2.blend = Blend::Screen;
        let mut fb = Effect::new(EffectKind::Feedback);
        apply_feedback_preset(&mut fb, 1);
        l2.effects.push(fb);
        // Layer 3: generator overlay through a kaleidoscope.
        self.comp.set_clip(2, 0, Clip::generator(1));
        self.comp.set_clip(2, 2, Clip::generator(0));
        self.comp.set_clip(2, 3, Clip::generator(5));
        let l3 = &mut self.comp.layers[2];
        l3.name = "Overlay".into();
        l3.blend = Blend::Add;
        l3.opacity.set(0.35);
        let mut k = Effect::new(EffectKind::Kaleidoscope);
        k.params[1] = k.params[1].clone().lfo(Shape::Triangle, 32.0, 0.25);
        l3.effects.push(k);
        self.comp.layers[0].name = "Footage".into();
        for l in &mut self.comp.layers {
            l.launch(0);
        }
        self.comp.active_column = Some(0);
        self.comp.quantize = Quantize::Beat;
        self.grid.selected_layer = 1;
        self.grid.selected_clip = Some((1, 0));
        self.status = "Demo loaded: number keys 1–4 launch scenes.".into();
    }

    fn handle_grid(&mut self, actions: Vec<GridAction>) {
        for a in actions {
            match a {
                GridAction::Launch(l) => self.comp.launch(l),
                GridAction::Select { layer, col } => {
                    self.grid.selected_layer = layer;
                    self.tab = Tab::Layer;
                    if let Some(c) = col {
                        self.grid.selected_clip = Some((layer, c));
                    }
                }
                GridAction::LoadFile { layer, col } => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter(
                            "media",
                            &["png", "jpg", "jpeg", "gif", "webp", "bmp", "mp4", "mov", "m4v", "mkv", "webm", "avi", "glsl", "frag", "fs", "isf", "wgsl"],
                        )
                        .pick_file()
                    {
                        self.load_file(layer, col, path);
                    }
                }
                GridAction::Camera { layer, col, index } => match Clip::camera(index, MEDIA_WIDTH, MEDIA_HEIGHT) {
                    Ok(c) => {
                        self.comp.set_clip(layer, col, c);
                        self.grid.selected_clip = Some((layer, col));
                    }
                    Err(e) => self.status = format!("Error: {e}"),
                },
                GridAction::Shader { layer, col, template } => {
                    let (name, _, code) = shader::TEMPLATES[template];
                    let c = Clip::shader(name, code);
                    if let clip::Media::Shader(s) = &c.media {
                        self.editing = Some(s.id);
                    }
                    self.comp.set_clip(layer, col, c);
                    self.grid.selected_clip = Some((layer, col));
                }
                GridAction::Generator { layer, col, pattern } => {
                    self.comp.set_clip(layer, col, Clip::generator(pattern));
                    self.grid.selected_clip = Some((layer, col));
                }
                GridAction::Library { layer, col, drag } => self.use_library(&drag, Some(layer), col),
                GridAction::Remove { layer, col } => self.comp.layers[layer].remove_clip(col),
                GridAction::Clear(layer) => self.comp.layers[layer].clear(),
                GridAction::AddLayer => self.comp.add_layer(),
                GridAction::AddColumn => self.comp.add_column(),
                GridAction::RemoveLayer(i) => {
                    if self.comp.layers.len() > 1 {
                        self.comp.layers.remove(i);
                        self.punch.retain_layers(&self.comp);
                        self.grid.selected_layer = self.grid.selected_layer.min(self.comp.layers.len() - 1);
                        self.grid.selected_clip = None;
                    }
                }
            }
        }
    }

    /// Put an ISF shader from the browser to use. Generators load into `col` (or the
    /// selected / first free cell of `layer`); effects go on `layer`, or the master chain when
    /// `layer` is `None`.
    fn use_library(&mut self, d: &isf_library::Drag, layer: Option<usize>, col: Option<usize>) {
        let shader = self.library.find(&d.key).ok_or_else(|| format!("{} is no longer in the library", d.name)).and_then(|e| e.shader());
        let result = match d.kind {
            isf_library::Kind::Generator => {
                let layer = layer.unwrap_or(self.grid.selected_layer);
                let free = |comp: &Composition| comp.layers[layer].clips.iter().position(|c| c.is_none());
                let col = col
                    .or(self.grid.selected_clip.filter(|(l, c)| *l == layer && self.comp.layers[layer].clips[*c].is_none()).map(|(_, c)| c))
                    .or_else(|| free(&self.comp))
                    .unwrap_or_else(|| {
                        self.comp.add_column();
                        self.comp.columns - 1
                    });
                shader.map(|s| {
                    let c = Clip::from_shader(s);
                    self.status = format!("Loaded {} into {} / column {}", c.name, self.comp.layers[layer].name, col + 1);
                    self.comp.set_clip(layer, col, c);
                    self.grid.selected_clip = Some((layer, col));
                })
            }
            _ => shader.map(|s| {
                let mut e = Effect::new(EffectKind::Shader);
                e.custom = Some(Box::new(s));
                let chain = match layer {
                    Some(l) => &mut self.comp.layers[l].effects,
                    None => &mut self.comp.effects,
                };
                chain.push(e);
                let target = layer.map(|l| self.comp.layers[l].name.clone()).unwrap_or_else(|| "master".into());
                self.status = format!("Added {} to {target}", d.name);
                if let Some(l) = layer {
                    self.grid.selected_layer = l;
                }
                self.tab = if layer.is_some() { Tab::Layer } else { Tab::Composition };
            }),
        };
        if let Err(e) = result {
            self.status = format!("Error: {e}");
        }
    }

    fn handle_library(&mut self, actions: Vec<ui::library::LibraryAction>) {
        use ui::library::LibraryAction;
        for a in actions {
            match a {
                LibraryAction::Use(d) => {
                    let layer = (self.tab == Tab::Layer || d.kind == isf_library::Kind::Generator).then_some(self.grid.selected_layer);
                    self.use_library(&d, layer, None);
                }
                LibraryAction::AddFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder()
                        && !self.library.dirs.contains(&dir)
                    {
                        self.library.dirs.push(dir);
                        self.library.rescan();
                    }
                }
                LibraryAction::RemoveFolder(i) => {
                    if i < self.library.dirs.len() {
                        self.library.dirs.remove(i);
                        self.library.rescan();
                    }
                }
                LibraryAction::Rescan => self.library.rescan(),
            }
        }
    }

    fn tap(&mut self) {
        let now = Instant::now();
        if self.taps.last().is_some_and(|l| now.duration_since(*l).as_secs_f32() > 2.0) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        if self.taps.len() >= 2 {
            let span = now.duration_since(self.taps[0]).as_secs_f32();
            self.comp.bpm = (60.0 * (self.taps.len() - 1) as f32 / span).clamp(30.0, 300.0);
            self.beat = self.beat.round();
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        // Drop files onto a cell (several fill the following cells); elsewhere they go into
        // the selected layer's first free cells.
        let (dropped, pointer) = ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.hover_pos()));
        if !dropped.is_empty() {
            let target = pointer.and_then(|p| self.grid.cells.iter().find(|(_, r)| r.contains(p)).map(|(k, _)| *k));
            let (layer, mut col) = target.unwrap_or_else(|| {
                let l = self.grid.selected_layer;
                let free = self.comp.layers[l].clips.iter().position(|c| c.is_none()).unwrap_or(self.comp.columns);
                (l, free)
            });
            for f in dropped {
                self.load_file(layer, col, f.path().to_path_buf());
                col += 1;
            }
        }

        // Punch-in pads: held while their key is down (not while typing, not with Cmd/Ctrl).
        let typing = ctx.egui_wants_keyboard_input();
        let (command, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        for (pad, def) in self.punch.pads.iter_mut().zip(punch::PUNCHES.iter()) {
            let (down, pressed) = ctx.input(|i| (i.key_down(def.key), i.key_pressed(def.key)));
            if typing || command {
                pad.key_held = false;
                continue;
            }
            if shift {
                // Shift + key latches / unlatches instead of holding.
                if pressed {
                    pad.latched = !pad.latched;
                }
                pad.key_held = false;
            } else {
                pad.key_held = down;
            }
        }
        if typing {
            return;
        }
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        let cmd = |k: Key| ctx.input(|i| i.modifiers.command && i.key_pressed(k));
        let scene_keys = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        for (i, k) in scene_keys.iter().enumerate() {
            if pressed(*k) && i < self.comp.columns {
                self.comp.launch(Launch::Column(i));
            }
        }
        if cmd(Key::F) {
            self.perform = !self.perform;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.perform));
        }
        if pressed(Key::Escape) && self.perform {
            self.perform = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        if pressed(Key::Space) {
            self.comp.playing = !self.comp.playing;
        }
        if pressed(Key::Enter) {
            self.tap();
        }
        if pressed(Key::B) && !command {
            self.show_library = !self.show_library;
        }
        if cmd(Key::K) {
            self.renderer.clear_history();
        }
        if cmd(Key::R) {
            self.toggle_recording();
        }
        if cmd(Key::S) {
            self.save_snapshot(None);
        }
        if (pressed(Key::Delete) || pressed(Key::Backspace))
            && let Some((l, c)) = self.grid.selected_clip
        {
            self.comp.layers[l].remove_clip(c);
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let held = |k: Key| ctx.input(|i| i.key_down(k));
        let xf = &mut self.comp.crossfader;
        if held(Key::ArrowLeft) {
            xf.set(xf.value - dt);
        }
        if held(Key::ArrowRight) {
            xf.set(xf.value + dt);
        }
    }

    fn save_snapshot(&mut self, path: Option<PathBuf>) {
        let path = path.unwrap_or_else(|| PathBuf::from(format!("tripslop-{}.png", unix_time())));
        self.status = match self.renderer.snapshot().and_then(|img| img.save(&path).map_err(|e| e.to_string())) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Snapshot failed: {e}"),
        };
    }

    fn toggle_recording(&mut self) {
        if let Some(rec) = self.recorder.take() {
            let (device, _, _) = self.renderer.output();
            let fin = rec.stop(device);
            self.status = format!("Finishing {} …", fin.path.display());
            self.finishing.push(fin);
            return;
        }
        let path = std::env::current_dir().unwrap_or_default().join(format!("tripslop-{}.mp4", unix_time()));
        let (device, _, _) = self.renderer.output();
        // Fixed-step runs (scripts) want every tick in the file; live sets must never stall.
        let mode = if self.fixed_step { recorder::Mode::Exact } else { recorder::Mode::Live };
        match Recorder::start(device, path, mode, self.renderer.size()) {
            Ok(rec) => {
                self.status = format!("Recording to {} ({})", rec.path.display(), rec.encoder);
                self.recorder = Some(rec);
            }
            Err(e) => self.status = format!("Recording failed: {e}"),
        }
    }

    /// Change the program size. A running recording is stopped first (its frames have the old
    /// size).
    fn set_size(&mut self, size: (u32, u32)) -> Result<(), String> {
        if size == self.renderer.size() {
            return Ok(());
        }
        if self.recorder.is_some() {
            self.toggle_recording();
        }
        self.renderer.resize(size)?;
        self.size_edit = size;
        Ok(())
    }

    /// Carry out what MIDI messages asked for.
    fn apply_midi(&mut self, actions: Vec<midi::Action>) {
        use midi::Action;
        for a in actions {
            match a {
                Action::Param(path, t) => {
                    if let Err(e) = self.with_param(&path, |p| p.set_normalized(t)) {
                        self.status = format!("MIDI: {e}");
                    }
                }
                Action::Pad(i, down) => self.punch.pads[i].midi_held = down,
                Action::ToggleLatch(i) => self.punch.pads[i].latched = !self.punch.pads[i].latched,
                Action::Launch(c) => {
                    if c < self.comp.columns {
                        self.comp.launch(Launch::Column(c));
                    }
                }
                Action::Bpm(b) => self.comp.bpm = b,
                // Keep the beat in phase with the incoming quarter notes.
                Action::Beat => {
                    let nearest = self.beat.round();
                    if (self.beat - nearest).abs() < 0.25 {
                        self.beat = nearest;
                    }
                }
                Action::Start => self.beat = (self.beat / 4.0).round() * 4.0,
                Action::Learned(m) => self.status = format!("MIDI: {} → {}", m.control.describe(), m.target.describe()),
            }
        }
    }

    /// Every parameter's script path and seed (MIDI mappings use paths, the UI uses seeds).
    fn param_paths(&mut self) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        self.comp.visit_paths(&mut |path, p| out.push((path.to_string(), p.seed)));
        out
    }

    /// Take the UI's MIDI learn requests, and tell it what's mapped.
    fn sync_midi_ui(&mut self, ctx: &egui::Context) {
        use midi::Target;
        use ui::midi::{LearnKey, Request};
        let requests = ui::midi::take_requests(ctx);
        let paths = self.param_paths();
        let target_of = |k: LearnKey| match k {
            LearnKey::Param(seed) => paths.iter().find(|(_, s)| *s == seed).map(|(p, _)| Target::Param(p.clone())),
            LearnKey::Pad(i) => Some(Target::Pad(i)),
            LearnKey::Scene(c) => Some(Target::Scene(c)),
            LearnKey::Shift => Some(Target::Shift),
        };
        for r in requests {
            match r {
                Request::Learn(k) => {
                    self.midi.learning = target_of(k);
                    if let Some(t) = &self.midi.learning {
                        self.status = format!("MIDI learn: move a control for {}", t.describe());
                    }
                }
                Request::Forget(k) => {
                    if let Some(t) = target_of(k) {
                        self.midi.forget(&t);
                    }
                }
                Request::Cancel => self.midi.learning = None,
            }
        }
        let key_of = |t: &Target| match t {
            Target::Param(path) => paths.iter().find(|(p, _)| p == path).map(|(_, s)| LearnKey::Param(*s)),
            Target::Pad(i) => Some(LearnKey::Pad(*i)),
            Target::Scene(c) => Some(LearnKey::Scene(*c)),
            Target::Shift => Some(LearnKey::Shift),
        };
        let view = ui::midi::View {
            mapped: self.midi.mappings.iter().filter_map(|m| Some((key_of(&m.target)?, m.control.describe()))).collect(),
            learning: self.midi.learning.as_ref().and_then(key_of),
        };
        ui::midi::publish(ctx, view);
        if self.midi.learning.is_some() {
            ctx.request_repaint();
        }
    }

    /// MIDI input picker, clock, Shift and the list of mappings, in the top bar.
    fn midi_controls(&mut self, ui: &mut egui::Ui) {
        let label = match (&self.midi.port, self.midi.connected()) {
            (midi::Port::Off, _) => "🎹 MIDI off".to_string(),
            (_, 0) => "🎹 MIDI (no device)".to_string(),
            (midi::Port::All, n) => format!("🎹 MIDI ({n})"),
            (midi::Port::Named(name), _) => format!("🎹 {name}"),
        };
        ui.menu_button(label, |ui| {
            ui.set_min_width(260.0);
            let mut choice = None;
            ui.label(RichText::new("Input").strong());
            if ui.selectable_label(self.midi.port == midi::Port::Off, "Off").clicked() {
                choice = Some(midi::Port::Off);
            }
            if ui.selectable_label(self.midi.port == midi::Port::All, "All devices").clicked() {
                choice = Some(midi::Port::All);
            }
            for name in midi::ports() {
                let named = midi::Port::Named(name.clone());
                if ui.selectable_label(self.midi.port == named, &name).clicked() {
                    choice = Some(named);
                }
            }
            if ui.button("Rescan devices").clicked() {
                choice = Some(self.midi.port.clone());
            }
            if let Some(p) = choice {
                self.midi.connect(p);
            }
            if let Some(e) = &self.midi.error {
                ui.colored_label(Color32::from_rgb(255, 120, 100), e);
            }
            ui.separator();
            let mut follow = self.midi.follow_clock;
            if ui.checkbox(&mut follow, "Follow MIDI clock").on_hover_text("Tempo and beat from the incoming clock; Start resets to a downbeat").changed() {
                self.midi.set_follow_clock(follow);
            }
            ui.horizontal(|ui| {
                ui::midi::menu(ui, ui::midi::LearnKey::Shift);
                ui.label(RichText::new("Shift: hold + pad = latch").small().weak());
            });
            ui.label(
                RichText::new(format!(
                    "Pads: notes {}–{} · right-click a slider, pad or scene to learn",
                    midi::PAD_BASE_NOTE,
                    midi::PAD_BASE_NOTE + 15
                ))
                .small()
                .weak(),
            );
            if !self.midi.mappings.is_empty() {
                ui.separator();
                ui.label(RichText::new("Mappings").strong());
                let mut forget = None;
                for m in &self.midi.mappings {
                    ui.horizontal(|ui| {
                        if ui.small_button("×").clicked() {
                            forget = Some(m.target.clone());
                        }
                        ui.label(RichText::new(format!("{} → {}", m.control.describe(), m.target.describe())).small());
                    });
                }
                if let Some(t) = forget {
                    self.midi.forget(&t);
                }
                if ui.button("Forget all").clicked() {
                    self.midi.forget_all();
                }
            }
            if let Some(last) = &self.midi.last {
                ui.separator();
                ui.label(RichText::new(format!("last: {last}")).small().weak());
            }
        });
    }

    /// Audio input picker and level meter, in the top bar.
    fn audio_controls(&mut self, ui: &mut egui::Ui) {
        // Drawn first: the top bar lays this out right to left, so the meter ends up after the menu.
        // Level, bass, mid, high bars and a kick light.
        let l = self.audio.levels;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 20.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, ui::theme::GROUND);
        for (i, v) in [l.level, l.bass, l.mid, l.high].into_iter().enumerate() {
            let x = rect.left() + 2.0 + i as f32 * 10.0;
            let h = (rect.height() - 4.0) * v.clamp(0.0, 1.0);
            painter.rect_filled(Rect::from_min_max(pos2(x, rect.bottom() - 2.0 - h), pos2(x + 8.0, rect.bottom() - 2.0)), 1.0, ui::theme::AUDIO);
        }
        painter.circle_filled(pos2(rect.right() - 6.0, rect.center().y), 4.0, ui::theme::QUEUED.gamma_multiply(l.kick.max(0.08)));
        if self.audio.source_name().is_some() {
            ui.ctx().request_repaint();
        }
        let current = self.audio.source_name().map(str::to_string);
        let label = match &current {
            Some(n) => format!("🔊 {n}"),
            None => "🔇 Audio off".to_string(),
        };
        let mut pick: Option<Option<Option<String>>> = None;
        let mut file = false;
        egui::ComboBox::from_id_salt("audio input")
            .selected_text(label)
            .width(150.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), "Off").clicked() {
                    pick = Some(None);
                }
                if ui.selectable_label(false, "Default input").clicked() {
                    pick = Some(Some(None));
                }
                for name in audio::input_devices() {
                    if ui.selectable_label(current.as_deref() == Some(&name), &name).clicked() {
                        pick = Some(Some(Some(name)));
                    }
                }
                ui.separator();
                if ui.selectable_label(self.audio.is_file(), "WAV file…").on_hover_text("Analyse a WAV file in step with the clock (silent)").clicked() {
                    file = true;
                }
            })
            .response
            .on_hover_text("Sound input for the Audio automation shape and audio-reactive shaders. For what the computer is playing, use a loopback device such as BlackHole.");
        match pick {
            Some(None) => {
                self.audio.stop();
                self.status = "Audio off".into();
            }
            Some(Some(name)) => {
                self.status = match self.audio.open_device(name.as_deref()) {
                    Ok(()) => format!("Listening to {}", self.audio.source_name().unwrap_or("input")),
                    Err(e) => format!("Audio input failed: {e}"),
                };
            }
            None => {}
        }
        if file && let Some(path) = rfd::FileDialog::new().add_filter("WAV", &["wav"]).pick_file() {
            self.status = match self.audio.open_file(&path, self.sim_time) {
                Ok(()) => format!("Analysing {}", path.display()),
                Err(e) => format!("Can't read audio: {e}"),
            };
        }
    }

    /// Program size and memory, on the Composition tab.
    fn output_settings(&mut self, ui: &mut egui::Ui) {
        let (w, h) = self.renderer.size();
        let mut request = None;
        ui.horizontal(|ui| {
            ui.label("Size");
            egui::ComboBox::from_id_salt("output size")
                .selected_text(format!("{w}×{h}"))
                .show_ui(ui, |ui| {
                    for (name, s) in [("720p", (1280, 720)), ("1080p", (1920, 1080)), ("1440p", (2560, 1440))] {
                        if ui.selectable_label((w, h) == s, format!("{name}  {}×{}", s.0, s.1)).clicked() {
                            request = Some(s);
                        }
                    }
                });
            ui.add(egui::DragValue::new(&mut self.size_edit.0).range(64..=8192).suffix(" w"));
            ui.add(egui::DragValue::new(&mut self.size_edit.1).range(64..=8192).suffix(" h"));
            let custom = (self.size_edit.0 & !1, self.size_edit.1 & !1);
            if ui
                .add_enabled(custom != (w, h), egui::Button::new("Apply"))
                .on_hover_text("Rebuilds every render target: clears feedback history and stops a recording")
                .clicked()
            {
                request = Some(custom);
            }
        });
        if let Some(s) = request {
            self.status = match self.set_size(s) {
                Ok(()) => format!("Output size {}×{}", s.0, s.1),
                Err(e) => format!("Can't resize: {e}"),
            };
        }
        let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
        let clips: usize = self
            .comp
            .layers
            .iter()
            .flat_map(|l| l.clips.iter().flatten())
            .map(|c| match &c.media {
                clip::Media::Video(v) => v.memory_bytes(),
                _ => 0,
            })
            .sum();
        ui.label(format!("GPU memory ≈ {:.0} MB · video clips {:.0} MB", mb(self.renderer.gpu_memory()), mb(clips as u64)))
            .on_hover_text("Render targets, effect history and textures (estimate). Effects with history can keep it at half size to save memory.");
    }

    fn capture_frame(&mut self) {
        let Some(rec) = self.recorder.as_mut() else { return };
        let (device, queue, tex) = self.renderer.output();
        if let Err(e) = rec.capture(device, queue, tex) {
            self.status = format!("Recording stopped: {e}");
            if let Some(rec) = self.recorder.take() {
                self.finishing.push(rec.stop(device));
            }
        }
    }

    fn poll_finished_recordings(&mut self) {
        let (done, pending): (Vec<_>, Vec<_>) = self.finishing.drain(..).partition(|f| f.is_done());
        self.finishing = pending;
        for f in done {
            let dropped = f.dropped;
            self.status = match f.join() {
                Ok((path, frames)) => {
                    let mut msg = format!("Saved {} ({:.1}s)", path.display(), frames as f64 / recorder::FPS as f64);
                    if dropped > 0 {
                        msg += &format!(", {dropped} dropped frames repeated");
                    }
                    msg
                }
                Err(e) => format!("Recording failed: {e}"),
            };
        }
    }

    /// Advance on a fixed 60Hz clock so effects and delays behave the same on any display.
    fn render_frame(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64();
        let ticks = if self.fixed_step {
            // As many ticks as fit in a short budget per drawn frame, so scripted runs don't
            // depend on how often the OS redraws the window (macOS throttles hidden windows).
            self.last_frame = now;
            let budget = Duration::from_millis(12);
            let mut n = 0;
            while n == 0 || (now.elapsed() < budget && !self.quit_requested && self.pending_screenshot.is_none()) {
                self.tick_once();
                n += 1;
            }
            return;
        } else {
            if dt < TICK {
                return;
            }
            let t = ((dt / TICK) as u32).min(3);
            self.last_frame = if dt > TICK * 4.0 { now } else { self.last_frame + Duration::from_secs_f64(TICK * t as f64) };
            t
        };
        for _ in 0..ticks {
            self.tick_once();
        }
    }

    /// Output frames rendered per second of wall-clock time, over half-second windows.
    fn update_fps(&mut self) {
        let (start, ticks) = self.fps_window;
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= 0.5 {
            self.fps = ((self.frame_count - ticks) as f64 / elapsed) as f32;
            self.fps_window = (Instant::now(), self.frame_count);
        }
    }

    /// One 60Hz step: script events, punch-ins, composition, render, captures.
    fn tick_once(&mut self) {
        self.run_script();
        let prev_beat = self.beat;
        self.beat += TICK * self.comp.bpm as f64 / 60.0;
        self.sim_time += TICK;
        self.frame_count += 1;
        self.audio.tick(self.sim_time, TICK as f32);
        if self.audio.source_name().is_some() {
            self.renderer.set_audio(&self.audio.spectrum, &self.audio.waveform);
        }
        let clock = self.clock();
        self.punch.update(&mut self.comp, clock, TICK);
        self.comp.tick(TICK, prev_beat, clock);
        self.renderer.render(&mut self.comp, &mut self.punch, clock);
        self.capture_frame();
        if let Some(path) = self.pending_snapshot.take() {
            self.save_snapshot(Some(path));
            println!("{} {}", automation_stamp(self.frame_count, self.beat), self.status);
        }
    }

    /// Create grid thumbnails for clips that have a first frame.
    fn make_thumbnails(&mut self, ctx: &egui::Context) {
        for l in &mut self.comp.layers {
            for c in l.clips.iter_mut().flatten() {
                if c.thumbnail.is_some() {
                    continue;
                }
                let img = match &c.media {
                    clip::Media::Video(v) => v.thumbnail(160, 90),
                    clip::Media::Image { frame, .. } => image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
                        .map(|i| image::DynamicImage::ImageRgba8(i).thumbnail_exact(160, 90).to_rgba8()),
                    _ => None,
                };
                if let Some(img) = img {
                    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                    c.thumbnail = Some(ctx.load_texture(format!("thumb-{}", c.id), ci, egui::TextureOptions::LINEAR));
                }
            }
        }
    }

    // ------------------------------------------------------------------ UI

    fn transport_bar(&mut self, ui: &mut egui::Ui) {
        use ui::theme;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(egui::Image::new(egui::include_image!("../logos/tripslop-wordmark-color.svg")).fit_to_exact_size(egui::vec2(76.0, 27.0)))
                .on_hover_text("tripslop");
            divider(ui);

            let playing = self.comp.playing;
            let play = egui::Button::new(RichText::new(if playing { "⏸" } else { "▶" }).color(if playing { theme::ON_LIT } else { theme::TEXT }))
                .min_size(egui::vec2(30.0, 26.0))
                .fill(if playing { theme::LIVE } else { theme::CONTROL });
            if ui.add(play).on_hover_text("Play / pause all clips (Space)").clicked() {
                self.comp.playing = !playing;
            }
            egui::Frame::new()
                .fill(theme::GROUND)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(8, 1))
                .show(ui, |ui| {
                    ui.label(RichText::new("BPM").font(theme::semibold(10.5)).color(theme::FAINT));
                    ui.style_mut().drag_value_text_style = egui::TextStyle::Name("bpm".into());
                    ui.style_mut().text_styles.insert(egui::TextStyle::Name("bpm".into()), theme::mono(17.0));
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                    ui.add(egui::DragValue::new(&mut self.comp.bpm).range(30.0..=300.0).speed(0.5).fixed_decimals(2))
                        .on_hover_text("Drag or type. MIDI clock can drive it (MIDI menu).");
                });
            if ui.small_button("÷2").clicked() {
                self.comp.bpm = (self.comp.bpm / 2.0).max(30.0);
            }
            if ui.small_button("×2").clicked() {
                self.comp.bpm = (self.comp.bpm * 2.0).min(300.0);
            }
            if ui.button(RichText::new("TAP").font(theme::semibold(13.0))).on_hover_text("Tap tempo (Enter)").clicked() {
                self.tap();
            }
            if ui.small_button("⟲").on_hover_text("Resync: make now beat 1").clicked() {
                self.beat = 0.0;
            }
            // Beat lights: 4 per bar, then bar.beat.
            let (r, _) = ui.allocate_exact_size(egui::vec2(4.0 * 14.0, 12.0), egui::Sense::hover());
            let beat = self.beat.max(0.0);
            let beat_in_bar = (beat.floor() as i64).rem_euclid(4) as usize;
            for i in 0..4 {
                let c = Rect::from_center_size(r.left_center() + egui::vec2(5.0 + i as f32 * 14.0, 0.0), egui::vec2(10.0, 10.0));
                ui.painter().rect_filled(c, 2.0, if i == beat_in_bar { theme::LIVE } else { theme::CONTROL_HI });
            }
            ui.label(RichText::new(format!("{}.{}", (beat / 4.0).floor() as i64 + 1, beat_in_bar + 1)).font(theme::mono(12.0)).color(theme::MUTED));
            egui::ComboBox::from_id_salt("quantize bar")
                .selected_text(self.comp.quantize.name())
                .width(130.0)
                .show_ui(ui, |ui| {
                    for q in Quantize::ALL {
                        ui.selectable_value(&mut self.comp.quantize, q, q.name());
                    }
                })
                .response
                .on_hover_text("When clip and scene launches happen");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if ui.button("⛶ Perform").on_hover_text("Fullscreen output (Cmd/Ctrl+F, Esc to leave)").clicked() {
                    self.perform = true;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                }
                divider(ui);
                ui.toggle_value(&mut self.show_library, "Browser").on_hover_text("Show the shader browser (B)");
                ui.toggle_value(&mut self.show_output, "🖵").on_hover_text("Output window: drag it to a projector, double-click for fullscreen");
                if ui.button("📷").on_hover_text("Save a PNG snapshot (Cmd/Ctrl+S)").clicked() {
                    self.save_snapshot(None);
                }
                if ui.button("✱").on_hover_text("Clear all feedback / delay memory (Cmd/Ctrl+K)").clicked() {
                    self.renderer.clear_history();
                }
                let rec = match &self.recorder {
                    Some(r) => {
                        let secs = r.frames() / recorder::FPS as u64;
                        let dropped = match r.dropped() {
                            0 => String::new(),
                            n => format!(" · {n} dropped"),
                        };
                        egui::Button::new(RichText::new(format!("⏺ {}:{:02}{dropped}", secs / 60, secs % 60)).color(theme::TEXT_STRONG))
                            .fill(egui::Color32::from_rgb(0x5A, 0x1A, 0x22))
                            .stroke(egui::Stroke::new(1.0, theme::RECORD))
                    }
                    None => egui::Button::new(RichText::new("⏺ REC").color(theme::RECORD)),
                };
                if ui.add(rec).on_hover_text("Record the output to MP4 (Cmd/Ctrl+R)").clicked() {
                    self.toggle_recording();
                }
                divider(ui);
                // Right-to-left: these appear as audio, then MIDI.
                self.midi_controls(ui);
                self.audio_controls(ui);
            });
        });
    }

    /// Bottom line: the latest message, and output size / memory / fps.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        use ui::theme;
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(&self.status).small().color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mono = |s: String| RichText::new(s).font(theme::mono(11.0)).color(theme::MUTED);
                ui.label(mono(format!("{:.0} fps", self.fps)));
                let (w, h) = self.renderer.size();
                ui.label(mono(format!("{w}×{h} · GPU {:.0} MB", self.renderer.gpu_memory() as f64 / (1024.0 * 1024.0))));
            });
        });
    }

    /// Output monitor; drag / scroll moves and scales the selected layer.
    fn monitor(&mut self, ui: &mut egui::Ui) {
        let (resp, rect) = paint_output(ui, self.renderer.display_id, self.renderer.size());
        // Alt-drag drives iMouse of the shader being edited.
        let alt = ui.input(|i| i.modifiers.alt);
        if alt && let Some(id) = self.editing && let Some(s) = self.comp.find_shader_mut(id) {
            if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on()) {
                let (w, h) = self.renderer.size();
                let x = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * w as f32;
                let y = (1.0 - (p.y - rect.top()) / rect.height()).clamp(0.0, 1.0) * h as f32;
                let click = *self.mouse_click.get_or_insert([x, y]);
                s.mouse = [x, y, click[0], click[1]];
            } else if self.mouse_click.take().is_some() {
                // Shadertoy convention: negative click position once released.
                s.mouse[2] = -s.mouse[2].abs();
                s.mouse[3] = -s.mouse[3].abs();
            }
            return;
        }
        let Some(layer) = self.comp.layers.get_mut(self.grid.selected_layer) else { return };
        if resp.dragged() {
            let d = resp.drag_delta();
            layer.pos_x.set(layer.pos_x.value + d.x / rect.width());
            layer.pos_y.set(layer.pos_y.value - d.y / rect.height());
        }
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 || zoom != 1.0 {
                layer.scale.set(layer.scale.value * (scroll * 0.004).exp() * zoom);
            }
            let center = rect.center() + egui::vec2(layer.pos_x.get() * rect.width(), -layer.pos_y.get() * rect.height());
            let r = Rect::from_center_size(center, rect.size() * layer.scale.get());
            let painter = ui.painter_at(rect);
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, ui::theme::LIVE), egui::StrokeKind::Middle);
            painter.text(
                rect.left_top() + egui::vec2(6.0, 6.0),
                egui::Align2::LEFT_TOP,
                format!("{}: drag = move, scroll = scale", layer.name),
                egui::FontId::proportional(12.0),
                ui::theme::LIVE,
            );
        }
        if self.recorder.is_some() {
            ui.painter().circle_filled(rect.right_top() + egui::vec2(-14.0, 14.0), 6.0, ui::theme::RECORD);
        }
    }

    fn crossfader(&mut self, ui: &mut egui::Ui) {
        use ui::theme;
        egui::Frame::new()
            .fill(theme::RAISED)
            .stroke(egui::Stroke::new(1.0, theme::LINE_SOFT))
            .corner_radius(6)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let side_color = |side| {
                    self.comp.layers.iter().position(|l| l.side == side).map(|_| theme::TEXT_STRONG).unwrap_or(theme::FAINT)
                };
                let (a, b) = (side_color(composition::Side::A), side_color(composition::Side::B));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("A").font(theme::bold(14.0)).color(a));
                    ui.spacing_mut().slider_width = ui.available_width() - 22.0;
                    ui.add(egui::Slider::new(&mut self.comp.crossfader.value, 0.0..=1.0).show_value(false).trailing_fill(false))
                        .on_hover_text("Crossfader between the A and B layers (←/→)");
                    ui.label(RichText::new("B").font(theme::bold(14.0)).color(b));
                });
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("crossfade mode")
                        .selected_text(self.comp.crossfade.name())
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            for m in composition::Crossfade::ALL {
                                ui.selectable_value(&mut self.comp.crossfade, m, m.name());
                            }
                        })
                        .response
                        .on_hover_text(
                            "Banks: A and B layers are composited separately (unassigned layers go in both) and the \
                             fader dissolves between the two pictures.\nLayer opacity: the fader fades the opacity of \
                             A / B layers in place.",
                        );
                    if self.comp.crossfade == composition::Crossfade::Bank {
                        egui::ComboBox::from_id_salt("fade curve")
                            .selected_text(self.comp.fade_curve.name())
                            .width(70.0)
                            .show_ui(ui, |ui| {
                                for c in composition::FadeCurve::ALL {
                                    ui.selectable_value(&mut self.comp.fade_curve, c, c.name());
                                }
                            });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{:>3.0}%", self.comp.master.value * 100.0)).font(theme::mono(11.0)));
                        ui.spacing_mut().slider_width = (ui.available_width() - 60.0).clamp(40.0, 140.0);
                        ui.add(egui::Slider::new(&mut self.comp.master.value, 0.0..=1.0).show_value(false)).on_hover_text("Master");
                        ui.label(theme::caption("Master"));
                    });
                });
            });
    }

    /// Audio input bands, kick and spectrum.
    fn audio_panel(&mut self, ui: &mut egui::Ui) {
        use ui::theme;
        let l = self.audio.levels;
        ui.horizontal(|ui| {
            ui.label(theme::caption("Audio in"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let kick = theme::QUEUED.gamma_multiply(l.kick.max(0.15));
                ui.label(RichText::new("● KICK").font(theme::mono(10.5)).color(kick));
                for (name, v) in [("HIGH", l.high), ("MID", l.mid), ("BASS", l.bass)] {
                    ui.label(RichText::new(format!("{name} {v:.2}")).font(theme::mono(10.5)).color(if l.active { theme::AUDIO } else { theme::FAINT }));
                }
            });
        });
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, theme::GROUND);
        let spec = &self.audio.spectrum;
        if !l.active || spec.is_empty() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "no input · pick one in the top bar", theme::body(12.0), theme::FAINT);
            return;
        }
        let bars = 48;
        let inner = rect.shrink(4.0);
        let w = inner.width() / bars as f32;
        for i in 0..bars {
            let a = i * spec.len() / bars;
            let b = ((i + 1) * spec.len() / bars).max(a + 1);
            let v = spec[a..b].iter().cloned().fold(0.0, f32::max).clamp(0.0, 1.0);
            let x = inner.left() + i as f32 * w;
            let r = Rect::from_min_max(pos2(x, inner.bottom() - v * inner.height()), pos2(x + w - 1.5, inner.bottom()));
            painter.rect_filled(r, 1.0, theme::AUDIO.gamma_multiply(1.0 - 0.5 * i as f32 / bars as f32));
        }
    }
}

/// A thin vertical line between groups in a toolbar.
fn divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(9.0, 22.0), egui::Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), egui::Stroke::new(1.0, ui::theme::LINE));
}

fn automation_stamp(frame: u64, beat: f64) -> String {
    format!("[f{frame} b{beat:.2}]")
}

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Paint the output letterboxed to its aspect ratio; returns the response and the picture rect.
fn paint_output(ui: &mut egui::Ui, tex: egui::TextureId, (w, h): (u32, u32)) -> (egui::Response, Rect) {
    let avail = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(avail, egui::Sense::click_and_drag());
    let aspect = w as f32 / h as f32;
    let mut size = avail.size();
    if size.x / size.y > aspect {
        size.x = size.y * aspect;
    } else {
        size.y = size.x / aspect;
    }
    let rect = Rect::from_center_size(avail.center(), size);
    ui.painter().rect_filled(avail, 0.0, Color32::BLACK);
    ui.painter()
        .image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    (resp, rect)
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        if let Some(rec) = self.recorder.take() {
            let (device, _, _) = self.renderer.output();
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
        let midi = self.midi.poll();
        self.apply_midi(midi);
        self.sync_midi_ui(&ctx);
        self.render_frame();
        self.update_fps();
        self.make_thumbnails(&ctx);
        self.library.poll();
        if let Some(id) = ui::shader_editor::take_request(&ctx) {
            self.editing = Some(id);
        }
        let tex = self.renderer.display_id;
        let out_size = self.renderer.size();
        let clock = self.clock();
        let blink = (self.beat * 4.0).fract() < 0.5;

        if self.show_output {
            let mut open = true;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("tripslop output"),
                egui::ViewportBuilder::default()
                    .with_title("tripslop output (double-click = fullscreen)")
                    .with_inner_size([960.0, 540.0]),
                |ui, _class| {
                    let (resp, _) = paint_output(ui, tex, out_size);
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
            let (resp, _) = paint_output(ui, tex, out_size);
            if resp.double_clicked() {
                self.perform = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        } else {
            use ui::theme;
            let bar = |fill| egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 0));
            egui::Panel::top("transport").exact_size(46.0).frame(bar(theme::PANEL)).show(ui, |ui| self.transport_bar(ui));
            egui::Panel::bottom("status").exact_size(24.0).frame(bar(theme::PANEL)).show(ui, |ui| self.status_bar(ui));

            // Right rail: everything you touch while playing.
            egui::Panel::right("rail")
                .resizable(true)
                .default_size(440.0)
                .size_range(300.0..=760.0)
                .frame(egui::Frame::new().fill(theme::SUNKEN).inner_margin(12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        ui.label(theme::caption("Output"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (w, h) = self.renderer.size();
                            ui.label(RichText::new(format!("{w}×{h}")).font(theme::mono(10.5)).color(theme::FAINT));
                        });
                    });
                    let (ow, oh) = self.renderer.size();
                    let h = ui.available_width() * oh as f32 / ow as f32;
                    ui.allocate_ui(egui::vec2(ui.available_width(), h), |ui| self.monitor(ui));
                    ui.add_space(4.0);
                    self.crossfader(ui);
                    ui.add_space(4.0);
                    self.audio_panel(ui);
                    ui.add_space(4.0);
                    let beat = self.beat;
                    let rest = ui.available_height() - 30.0;
                    ui::punch::pads(ui, &mut self.punch, beat, rest);
                });

            // Inspector (becomes the device panel).
            egui::Panel::bottom("inspector")
                .resizable(true)
                .default_size(330.0)
                .size_range(160.0..=700.0)
                .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(12, 8)))
                .show(ui, |ui| {
                    egui::Panel::right("clip")
                        .resizable(true)
                        .default_size(380.0)
                        .frame(egui::Frame::new().inner_margin(egui::Margin { left: 12, ..Default::default() }))
                        .show(ui, |ui| {
                            ui.label(theme::caption("Clip"));
                            egui::ScrollArea::vertical().id_salt("clip scroll").show(ui, |ui| {
                                let sel = self.grid.selected_clip;
                                let clip = sel.and_then(|(l, c)| self.comp.clip_mut(l, c));
                                ui::panels::clip_panel(ui, clip, clock);
                            });
                        });
                    egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.tab, Tab::Layer, RichText::new("Layer").font(theme::semibold(14.0)));
                            ui.selectable_value(&mut self.tab, Tab::Composition, RichText::new("Master").font(theme::semibold(14.0)));
                        });
                        egui::ScrollArea::vertical().id_salt("inspector").show(ui, |ui| match self.tab {
                            Tab::Layer => ui::panels::layer_panel(ui, &mut self.comp, self.grid.selected_layer, clock),
                            Tab::Composition => {
                                ui::panels::section(ui, "Output", true, |ui| self.output_settings(ui));
                                ui::panels::composition_panel(ui, &mut self.comp, clock);
                            }
                        });
                    });
                });

            if self.show_library {
                egui::Panel::left("library")
                    .resizable(true)
                    .default_size(260.0)
                    .size_range(200.0..=520.0)
                    .frame(egui::Frame::new().fill(theme::SUNKEN).inner_margin(10))
                    .show(ui, |ui| {
                        let renderer = &self.renderer;
                        let actions = self.library_view.show(ui, &self.library, &|k| renderer.library_thumb(k));
                        self.handle_library(actions);
                    });
                // Draw the pictures for the cards that were on screen (and animate the hovered one).
                self.renderer.library_previews(&self.library, &self.library_view.visible, self.library_view.hover());
            }

            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::GROUND).inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 0 }))
                .show(ui, |ui| {
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        let thumbs = self.renderer.thumbnails();
                        let actions = self.grid.show(ui, &mut self.comp, blink, &thumbs);
                        self.handle_grid(actions);
                    });
                });
        }

        if !self.perform {
            ui::shader_editor::show(&ctx, &mut self.comp, &mut self.editing, clock);
        }
        // Script screenshots: ask egui for the window image, save it when it arrives.
        if self.pending_screenshot.is_some() && !self.screenshot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.screenshot_requested = true;
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot
            && let Some(path) = self.pending_screenshot.take()
        {
            self.screenshot_requested = false;
            let [w, h] = img.size;
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            match image::RgbaImage::from_raw(w as u32, h as u32, rgba).map(|i| i.save(&path)) {
                Some(Ok(())) => println!("{} saved screenshot {}", automation_stamp(self.frame_count, self.beat), path.display()),
                _ => {
                    eprintln!("could not save screenshot {}", path.display());
                    automation::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        if self.quit_requested && self.pending_screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.fixed_step {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 120.0));
        }
    }
}
