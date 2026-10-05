//! Inspector panels for the selected layer, the selected clip and the composition.

use eframe::egui::{self, RichText};

use crate::clip::{Clip, Direction, Fit, LoopMode, Media, Sync};
use crate::composition::{Blend, Composition, Crossfade, FadeCurve, Quantize, Side};
use crate::effects::{EFFECTS, Effect, EffectKind, FEEDBACK_PRESETS, apply_feedback_preset};
use crate::modulation::Clock;
use crate::shader::{Role, TEMPLATES};
use crate::ui::shader_editor;
use crate::ui::widgets::{self, ACCENT};

pub fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(open)
        .show(ui, body);
}

pub fn layer_panel(ui: &mut egui::Ui, comp: &mut Composition, li: usize, clock: Clock) {
    let Some(layer) = comp.layers.get_mut(li) else {
        ui.label("No layer selected.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut layer.name);
    });
    section(ui, "Layer", true, |ui| {
        widgets::param(ui, &mut layer.opacity, clock);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("blend", layer.id))
                .selected_text(layer.blend.name())
                .show_ui(ui, |ui| {
                    for b in Blend::ALL {
                        ui.selectable_value(&mut layer.blend, b, b.name());
                    }
                });
            ui.label("blend mode");
        });
        ui.horizontal(|ui| {
            ui.label("Crossfader");
            ui.selectable_value(&mut layer.side, Side::Both, "Off");
            ui.selectable_value(&mut layer.side, Side::A, "A");
            ui.selectable_value(&mut layer.side, Side::B, "B");
            ui.separator();
            ui.toggle_value(&mut layer.bypass, "Bypass");
            ui.toggle_value(&mut layer.solo, "Solo");
        });
        widgets::param(ui, &mut layer.transition, clock).on_hover_text("Crossfade time when switching clips on this layer");
    });
    section(ui, "Transform", true, |ui| {
        for p in [&mut layer.pos_x, &mut layer.pos_y, &mut layer.scale, &mut layer.rotation] {
            widgets::param(ui, p, clock);
        }
        if ui.small_button("Reset transform").clicked() {
            for p in [&mut layer.pos_x, &mut layer.pos_y, &mut layer.scale, &mut layer.rotation] {
                p.set(p.spec.default);
            }
        }
        ui.label(RichText::new("Tip: drag / scroll on the output monitor to move / scale this layer.").small().weak());
    });
    section(ui, "Effects", true, |ui| effect_chain(ui, &mut layer.effects, layer.id, clock));
}

/// Add / remove / reorder / bypass effects and edit their parameters.
pub fn effect_chain(ui: &mut egui::Ui, effects: &mut Vec<Effect>, owner: u64, clock: Clock) {
    let mut remove = None;
    let mut swap = None;
    let n = effects.len();
    for (i, e) in effects.iter_mut().enumerate() {
        let id = ui.make_persistent_id(("fx", owner, e.id));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.checkbox(&mut e.enabled, "");
                ui.label(RichText::new(e.name()).strong().color(if e.enabled { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() }));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Remove effect").clicked() {
                        remove = Some(i);
                    }
                    if i + 1 < n && ui.small_button("▼").on_hover_text("Move down").clicked() {
                        swap = Some((i, i + 1));
                    }
                    if i > 0 && ui.small_button("▲").on_hover_text("Move up").clicked() {
                        swap = Some((i - 1, i));
                    }
                });
            })
            .body(|ui| {
                if e.kind == EffectKind::Feedback {
                    ui.horizontal(|ui| {
                        ui.label("Preset");
                        egui::ComboBox::from_id_salt(("fb preset", e.id))
                            .selected_text("choose…")
                            .show_ui(ui, |ui| {
                                for (pi, name) in FEEDBACK_PRESETS.iter().enumerate() {
                                    if ui.selectable_label(false, *name).clicked() {
                                        apply_feedback_preset(e, pi);
                                    }
                                }
                            });
                    });
                }
                for p in &mut e.params {
                    widgets::param(ui, p, clock);
                }
                if let Some(h) = e.def().history {
                    ui.checkbox(&mut e.half_history, "Half-size history").on_hover_text(format!(
                        "Keep the {} frames of history at half the output size: a quarter of the memory, a little softer. Changing it restarts the history.",
                        h.frames
                    ));
                }
                if let Some(c) = e.custom.as_deref_mut() {
                    ui.horizontal(|ui| {
                        if ui.button("Edit code").clicked() {
                            shader_editor::request_open(ui.ctx(), c.id);
                        }
                        shader_editor::status(ui, c);
                    });
                    shader_editor::params(ui, c, clock);
                }
            });
    }
    if let Some(i) = remove {
        effects.remove(i);
    }
    if let Some((a, b)) = swap {
        effects.swap(a, b);
    }
    ui.add_space(4.0);
    ui.menu_button(RichText::new("+ Add effect").color(ACCENT), |ui| {
        let mut last_cat = "";
        for d in EFFECTS {
            if d.category != last_cat {
                if !last_cat.is_empty() {
                    ui.separator();
                }
                ui.label(RichText::new(d.category).small().weak());
                last_cat = d.category;
            }
            if ui.button(d.name).clicked() {
                let mut e = Effect::new(d.kind);
                if d.kind == EffectKind::Feedback {
                    apply_feedback_preset(&mut e, 0);
                }
                effects.push(e);
                ui.close();
            }
        }
        ui.separator();
        ui.label(RichText::new("Code").small().weak());
        ui.menu_button("Custom shader (GLSL)", |ui| {
            for (name, role, code) in TEMPLATES {
                let tag = if *role == Role::Effect { "  (uses input)" } else { "" };
                if ui.button(format!("{name}{tag}")).clicked() {
                    let e = Effect::custom(name, code);
                    shader_editor::request_open(ui.ctx(), e.custom.as_ref().unwrap().id);
                    effects.push(e);
                    ui.close();
                }
            }
        });
    });
}

pub fn clip_panel(ui: &mut egui::Ui, clip: Option<&mut Clip>, clock: Clock) {
    let Some(clip) = clip else {
        ui.label(RichText::new("Select a clip in the grid. Right-click a cell to load media, a generator or a camera.").weak());
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut clip.name);
    });
    if let Some(e) = clip.error() {
        ui.colored_label(egui::Color32::RED, e);
    }

    if clip.is_timeline() {
        section(ui, "Transport", true, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut clip.direction, Direction::Reverse, "◀").on_hover_text("Play backwards");
                ui.selectable_value(&mut clip.direction, Direction::Paused, "⏸").on_hover_text("Pause");
                ui.selectable_value(&mut clip.direction, Direction::Forward, "▶").on_hover_text("Play forwards");
                ui.separator();
                let len = clip.length();
                let mut pos = clip.position as f32;
                ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(80.0);
                if ui
                    .add(egui::Slider::new(&mut pos, 0.0..=(len.max(2) - 1) as f32).show_value(false))
                    .on_hover_text("Scrub")
                    .changed()
                {
                    clip.position = pos as f64;
                    clip.finished = false;
                }
                ui.label(format!("{:.1}s", clip.position as f32 / clip.fps()));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Loop");
                for m in LoopMode::ALL {
                    if ui.selectable_label(clip.loop_mode == m, m.name()).clicked() {
                        clip.loop_mode = m;
                        clip.finished = false;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Timing");
                ui.selectable_value(&mut clip.sync, Sync::Timeline, "Timeline").on_hover_text("Native frame rate × speed");
                ui.selectable_value(&mut clip.sync, Sync::Bpm, "BPM sync").on_hover_text("Stretch the clip to a number of beats");
            });
            if clip.sync == Sync::Bpm {
                widgets::param(ui, &mut clip.beats, clock);
            }
            widgets::param(ui, &mut clip.speed, clock);
            ui.horizontal(|ui| {
                if ui.small_button("÷2").clicked() {
                    clip.speed.set(clip.speed.value / 2.0);
                }
                if ui.small_button("×2").clicked() {
                    clip.speed.set(clip.speed.value * 2.0);
                }
                if ui.small_button("1×").clicked() {
                    clip.speed.set(1.0);
                }
            });
        });
    }

    if let Media::Generator(g) = &mut clip.media {
        section(ui, "Generator", true, |ui| {
            for p in [&mut g.pattern, &mut g.freq, &mut g.speed, &mut g.hue] {
                widgets::param(ui, p, clock);
            }
        });
    }
    if let Media::Shader(s) = &mut clip.media {
        section(ui, "Shader", true, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Edit code").clicked() {
                    shader_editor::request_open(ui.ctx(), s.id);
                }
                shader_editor::status(ui, s);
            });
            shader_editor::params(ui, s, clock);
        });
    }

    section(ui, "Placement", true, |ui| {
        ui.horizontal(|ui| {
            for f in Fit::ALL {
                ui.selectable_value(&mut clip.fit, f, f.name());
            }
        });
    });

    section(ui, "Info", false, |ui| {
        match &clip.media {
            Media::Video(v) => {
                ui.label(format!("{}", v.path.display()));
                ui.label(format!(
                    "{} frames @ {:.2} fps · {:.1}s · {:.0} MB in memory",
                    v.frame_count(),
                    v.fps,
                    clip.duration_secs(),
                    v.memory_bytes() as f64 / 1e6
                ));
                if !v.is_loaded() {
                    ui.add(egui::ProgressBar::new(v.progress()).text("importing"));
                }
            }
            Media::Image { frame, .. } => {
                ui.label(format!("Still image {}×{}", frame.width, frame.height));
            }
            Media::Camera { index, .. } => {
                ui.label(format!("Live capture device {index}"));
            }
            Media::Generator(_) => {
                ui.label("Procedural generator");
            }
            Media::Shader(_) => {
                ui.label("User GLSL shader");
            }
        };
    });
}

pub fn composition_panel(ui: &mut egui::Ui, comp: &mut Composition, clock: Clock) {
    section(ui, "Composition", true, |ui| {
        widgets::param(ui, &mut comp.master, clock);
        widgets::param(ui, &mut comp.crossfader, clock);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("crossfade mode")
                .selected_text(comp.crossfade.name())
                .show_ui(ui, |ui| {
                    for m in Crossfade::ALL {
                        ui.selectable_value(&mut comp.crossfade, m, m.name());
                    }
                })
                .response
                .on_hover_text(
                    "Banks: A and B layers are composited separately (unassigned layers go in both) and the \
                     fader dissolves between the two pictures.\nLayer opacity: the fader fades the opacity of \
                     A / B layers in place.",
                );
            if comp.crossfade == Crossfade::Bank {
                egui::ComboBox::from_id_salt("fade curve")
                    .selected_text(comp.fade_curve.name())
                    .show_ui(ui, |ui| {
                        for c in FadeCurve::ALL {
                            ui.selectable_value(&mut comp.fade_curve, c, c.name());
                        }
                    });
            }
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("quantize")
                .selected_text(comp.quantize.name())
                .show_ui(ui, |ui| {
                    for q in Quantize::ALL {
                        ui.selectable_value(&mut comp.quantize, q, q.name());
                    }
                });
        });
    });
    section(ui, "Master effects", true, |ui| effect_chain(ui, &mut comp.effects, 0, clock));
    section(ui, "Automation", true, |ui| widgets::overview(ui, comp, clock));
}
