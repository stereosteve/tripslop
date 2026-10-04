//! KodeLife-style live code editor for user shaders.

use eframe::egui::{self, Color32, RichText, text::LayoutJob};

use crate::composition::Composition;
use crate::modulation::Clock;
use crate::shader::{CustomShader, Role, TEMPLATES};
use crate::ui::widgets;

const OPEN_REQUEST: &str = "trippy-open-shader";

/// Ask the app to open the editor for a shader (from anywhere in the UI).
pub fn request_open(ctx: &egui::Context, id: u64) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(OPEN_REQUEST), id));
}

pub fn take_request(ctx: &egui::Context) -> Option<u64> {
    ctx.data_mut(|d| d.remove_temp::<u64>(egui::Id::new(OPEN_REQUEST)))
}

/// One-line compile status.
pub fn status(ui: &mut egui::Ui, s: &CustomShader) {
    let pending = s.compiled_rev != Some(s.rev);
    if !s.errors.is_empty() {
        let n = s.errors.len();
        let tail = if s.running { " (last good version still running)" } else { "" };
        ui.colored_label(Color32::from_rgb(255, 90, 90), format!("{n} error{}{tail}", if n == 1 { "" } else { "s" }));
    } else if pending {
        ui.label(RichText::new("… compiling").weak());
    } else if s.running {
        ui.colored_label(Color32::from_rgb(110, 220, 120), "running");
    }
}

/// The shader's sliders (from `uniform float x; // min max default`) and its alpha mode.
pub fn params(ui: &mut egui::Ui, s: &mut CustomShader, clock: Clock) {
    if s.params.is_empty() {
        ui.label(RichText::new("No sliders. Add one with: uniform float amount; // 0 1 0.5").small().weak());
    }
    for p in &mut s.params {
        widgets::param(ui, p, clock);
    }
    widgets::param(ui, &mut s.alpha, clock);
}

pub fn show(ctx: &egui::Context, comp: &mut Composition, open: &mut Option<u64>, clock: Clock) {
    let Some(id) = *open else { return };
    let Some(s) = comp.find_shader_mut(id) else {
        *open = None;
        return;
    };
    let mut keep = true;
    egui::Window::new(format!("Shader · {}", s.name))
        .id(egui::Id::new("shader editor"))
        .open(&mut keep)
        .default_size([780.0, 720.0])
        .resizable(true)
        .show(ctx, |ui| editor(ui, s, clock));
    if !keep {
        *open = None;
    }
}

fn editor(ui: &mut egui::Ui, s: &mut CustomShader, clock: Clock) {
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
        s.compile_requested = true;
    }

    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(140.0));
        ui.checkbox(&mut s.live, "Live").on_hover_text("Recompile automatically as you type");
        if ui.button("Compile (Cmd/Ctrl+Enter)").clicked() {
            s.compile_requested = true;
        }
        ui.menu_button("Templates", |ui| {
            ui.label(RichText::new("Replaces the current code").small().weak());
            for (name, role, code) in TEMPLATES {
                let tag = if *role == Role::Effect { "  (uses input)" } else { "" };
                if ui.button(format!("{name}{tag}")).clicked() {
                    s.source = code.to_string();
                    s.edited();
                    s.compile_requested = true;
                    ui.close();
                }
            }
        });
        if ui.button("Load…").clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("Shader", &["glsl", "frag", "fs", "shader", "wgsl", "txt"]).pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(code) => {
                    s.source = code;
                    s.edited();
                    s.compile_requested = true;
                }
                Err(e) => s.errors = vec![crate::shader::CompileError { line: None, message: e.to_string() }],
            }
        }
        if ui.button("Save…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Shader", &["glsl", "frag", "wgsl"])
                .set_file_name(format!("{}.{}", s.name, if crate::shader::lang_of(&s.source) == crate::shader::Lang::Wgsl { "wgsl" } else { "glsl" }))
                .save_file()
            && let Err(e) = std::fs::write(&path, &s.source)
        {
            s.errors = vec![crate::shader::CompileError { line: None, message: e.to_string() }];
        }
        status(ui, s);
    });
    ui.separator();

    // Bottom area first (errors, sliders, help) so the code gets the remaining height.
    egui::Panel::bottom(egui::Id::new(("shader bottom", s.id)))
        .resizable(true)
        .default_size(230.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("shader bottom scroll").show(ui, |ui| {
                for e in &s.errors {
                    let loc = e.line.map(|l| format!("line {l}: ")).unwrap_or_default();
                    ui.colored_label(Color32::from_rgb(255, 110, 110), format!("{loc}{}", e.message));
                }
                egui::CollapsingHeader::new(RichText::new("Sliders").strong())
                    .default_open(true)
                    .show(ui, |ui| params(ui, s, clock));
                egui::CollapsingHeader::new(RichText::new("Help").strong()).show(ui, help);
            });
        });

    code_area(ui, s);
}

fn code_area(ui: &mut egui::Ui, s: &mut CustomShader) {
    let error_lines: Vec<usize> = s.errors.iter().filter_map(|e| e.line).collect();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        // WGSL reads well with the Rust highlighter (fn / let / var / ->).
        let lang = if crate::shader::lang_of(buf.as_str()) == crate::shader::Lang::Wgsl { "rs" } else { "c" };
        let mut job = egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &theme, buf.as_str(), lang);
        job.wrap.max_width = f32::INFINITY;
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    egui::ScrollArea::both()
        .id_salt(("shader code", s.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Line-number gutter, error lines in red.
                let lines = s.source.lines().count().max(1) + usize::from(s.source.ends_with('\n'));
                let mut gutter = LayoutJob::default();
                for n in 1..=lines {
                    let color = if error_lines.contains(&n) {
                        Color32::from_rgb(255, 80, 80)
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    gutter.append(&format!("{n:>4}\n"), 0.0, egui::TextFormat::simple(font.clone(), color));
                }
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(gutter);
                });
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut s.source)
                        .code_editor()
                        .lock_focus(true)
                        .margin(egui::Margin::symmetric(4, 2))
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .layouter(&mut layouter),
                );
                if resp.changed() {
                    s.edited();
                }
            });
        });
}

fn help(ui: &mut egui::Ui) {
    let mono = |t: &str| RichText::new(t).monospace();
    ui.label("Paste a Shadertoy shader (single pass), a GLSL Sandbox shader, or WGSL.");
    ui.label(mono("void mainImage(out vec4 fragColor, in vec2 fragCoord)"));
    ui.add_space(4.0);
    ui.label(RichText::new("Inputs").strong());
    ui.label(mono("iTime iTimeDelta iFrame iResolution iDate iChannelResolution[4]"));
    ui.label("iMouse: Alt-drag on the output monitor while the editor is open.");
    ui.label("iBeat / iBpm: the global tempo clock (e.g. pulse = exp(-4.0 * fract(iBeat))).");
    ui.add_space(4.0);
    ui.label(RichText::new("Channels").strong());
    ui.label("iChannel0: the layer's input (when used as an effect)");
    ui.label("iChannel1: this shader's previous frame (feedback)");
    ui.label("iChannel2: 256×256 RGBA noise");
    ui.label("iChannel3: the composition output from the previous frame");
    ui.add_space(4.0);
    ui.label(RichText::new("Sliders").strong());
    ui.label(mono("uniform float amount; // min max default"));
    ui.label(mono("uniform int count;    // 1 8 3"));
    ui.label("Each becomes a slider with automation (the ~ button).");
    ui.add_space(4.0);
    ui.label(RichText::new("WGSL").strong());
    ui.label("Detected by @fragment; your own entry point is used. Screen space: pos.y grows downward.");
    ui.label(mono("inputs.size inputs.time inputs.mouse inputs.date inputs.frame inputs.time_delta inputs.beat inputs.bpm"));
    ui.label(mono("textureSample(iChannel0, samp, uv)   // iChannel0..3"));
    ui.label(mono("// @param speed 0 4 1     → inputs.speed"));
    ui.add_space(4.0);
    ui.label(RichText::new("Limits").strong());
    ui.label("No multipass buffers (A–D), audio, video or cubemap channels. Samplers can't be passed to functions.");
    ui.label("The compiler (naga) is stricter than WebGL: e.g. write ivec2(p) % ivec2(4), not ivec2(p) % 4.");
    ui.label("Cmd/Ctrl+Enter compiles now. While there are errors, the last working version keeps running.");
}
