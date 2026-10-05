//! The device panel: the selected layer's chain left to right, like Bitwig. Source (the
//! clip) → effects → Layer out. With the master selected it shows Output → master effects →
//! Master out instead. Each device is a card of knobs; double-click a card's header to fold
//! it.

use std::f32::consts::FRAC_PI_2;

use eframe::egui::{self, Color32, Id, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::clip::{Clip, Direction, Fit, LoopMode, Media, Sync};
use crate::composition::Composition;
use crate::effects::{EFFECTS, Effect, EffectKind, FEEDBACK_PRESETS, apply_feedback_preset};
use crate::isf_library::{Drag, Kind, Library, Status};
use crate::model::ModelRef;
use crate::modulation::Clock;
use crate::param::Param;
use crate::shader::{Role, TEMPLATES};
use crate::ui::shader_editor;
use crate::ui::theme;
use crate::ui::widgets::{self, KNOB_W};

const HEADER_H: f32 = 30.0;
const GAP: f32 = 8.0;
const KNOB_ROW_H: f32 = 74.0;

/// The selected layer's chain. `clip_col`: the clip shown in the Source card.
pub fn layer_chain(ui: &mut egui::Ui, comp: &mut Composition, lib: &Library, li: usize, clip_col: Option<usize>, clock: Clock) {
    let Some(layer) = comp.layers.get_mut(li) else {
        ui.label(RichText::new("No layer selected.").color(theme::MUTED));
        return;
    };
    let color = theme::layer_color(layer.color);
    widgets::set_accent(ui.ctx(), color);
    let owner = layer.id;
    row(ui, ("chain", owner), |ui| {
        let clip = clip_col.and_then(|c| layer.clips.get_mut(c)?.as_mut());
        source_card(ui, Id::new(("source", owner)), clip, color, lib, clock);
        effect_cards(ui, &mut layer.effects, owner, lib, clock);
        let h = ui.available_height();
        let width = 3.0 * (KNOB_W + 2.0) + 24.0;
        card(ui, Id::new(("layer out", owner)), width, h, "Layer out", |ui| {
            ui.label(RichText::new("LAYER OUT").font(theme::semibold(11.0)).color(color));
        }, |ui| {
            widgets::param_grid(ui, [&mut layer.opacity, &mut layer.scale, &mut layer.rotation, &mut layer.pos_x, &mut layer.pos_y, &mut layer.transition], clock);
            if ui.small_button("Reset transform").on_hover_text("Tip: drag / scroll on the output monitor to move / scale the layer").clicked() {
                for p in [&mut layer.pos_x, &mut layer.pos_y, &mut layer.scale, &mut layer.rotation] {
                    p.set(p.spec.default);
                }
            }
        });
    });
}

/// The master chain. `output` draws the Output card's body (size, memory).
pub fn master_chain(ui: &mut egui::Ui, comp: &mut Composition, lib: &Library, clock: Clock, output: impl FnOnce(&mut egui::Ui)) {
    widgets::set_accent(ui.ctx(), theme::LIVE);
    row(ui, "master chain", |ui| {
        let h = ui.available_height();
        card(ui, Id::new("output card"), 250.0, h, "Output", |ui| {
            ui.label(RichText::new("OUTPUT").font(theme::semibold(11.0)).color(theme::LIVE));
        }, output);
        effect_cards(ui, &mut comp.effects, 0, lib, clock);
        let h = ui.available_height();
        card(ui, Id::new("master out"), 2.0 * (KNOB_W + 2.0) + 24.0, h, "Master out", |ui| {
            ui.label(RichText::new("MASTER OUT").font(theme::semibold(11.0)).color(theme::LIVE));
        }, |ui| {
            widgets::param_grid(ui, [&mut comp.master, &mut comp.crossfader], clock);
        });
    });
}

/// A horizontally scrolling row of cards.
fn row(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, add: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::horizontal().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            ui.set_min_height(ui.available_height());
            add(ui);
        });
    });
}

/// One device: a header strip and a scrolling body, `width` wide. Double-click the header to
/// fold it into a narrow strip showing `title`.
fn card(
    ui: &mut egui::Ui,
    id: Id,
    width: f32,
    height: f32,
    title: &str,
    header: impl FnOnce(&mut egui::Ui),
    body: impl FnOnce(&mut egui::Ui),
) {
    if let Some((mut head, mut inner)) = card_frame(ui, id, width, height, title) {
        header(&mut head);
        card_body(&mut inner, id, body);
    }
}

/// Paints a card and returns the Uis for its header and body (`None` when folded).
fn card_frame(ui: &mut egui::Ui, id: Id, width: f32, height: f32, title: &str) -> Option<(egui::Ui, egui::Ui)> {
    let folded_id = id.with("folded");
    let mut folded = ui.data_mut(|d| *d.get_persisted_mut_or_default::<bool>(folded_id));
    let w = if folded { 34.0 } else { width };
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let header_rect = Rect::from_min_size(rect.min, vec2(w, if folded { height } else { HEADER_H }));
    let toggle = ui.interact(header_rect, id.with("header"), Sense::click());
    if toggle.double_clicked() {
        folded = !folded;
        ui.data_mut(|d| d.insert_persisted(folded_id, folded));
    }

    let painter = ui.painter_at(rect.expand(1.0));
    painter.rect_filled(rect, 6.0, theme::RAISED);
    if folded {
        let galley = painter.layout_no_wrap(title.to_string(), theme::semibold(13.0), theme::TEXT);
        let pos = pos2(rect.center().x - galley.size().y / 2.0, rect.top() + 14.0 + galley.size().x);
        painter.add(egui::epaint::TextShape::new(pos, galley, theme::TEXT).with_angle(-FRAC_PI_2));
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, theme::LINE), StrokeKind::Inside);
        toggle.on_hover_text(format!("{title}: double-click to unfold"));
        return None;
    }
    painter.rect_filled(header_rect, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, theme::RAISED_HI);
    painter.hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, theme::LINE));
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, theme::LINE), StrokeKind::Inside);

    let mut head = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header_rect.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    head.spacing_mut().item_spacing.x = 5.0;
    let body_rect = Rect::from_min_max(pos2(rect.left(), header_rect.bottom() + 1.0), rect.max).shrink2(vec2(8.0, 6.0));
    let inner = ui.new_child(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    Some((head, inner))
}

fn card_body(inner: &mut egui::Ui, id: Id, body: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().id_salt(id.with("body")).auto_shrink([false, false]).show(inner, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        body(ui);
    });
}

/// Card width that fits `knobs` knobs in the rows the height allows, plus choice rows.
fn knob_card_width(knobs: usize, choices: usize, extra_rows: usize, height: f32) -> f32 {
    // Choice cells sit two to a row above the knobs.
    let room = height - HEADER_H - 16.0 - choices.div_ceil(2) as f32 * 44.0 - extra_rows as f32 * 28.0;
    let rows = (room / KNOB_ROW_H).floor().max(1.0);
    let cols = (knobs as f32 / rows).ceil().max(2.0);
    let w = cols * (KNOB_W + 2.0) + 24.0;
    let w = if choices > 0 { w.max(2.0 * widgets::CHOICE_W + 30.0) } else { w };
    w.clamp(150.0, 620.0)
}

fn count(params: &[Param]) -> (usize, usize) {
    let choices = params.iter().filter(|p| !p.spec.choices.is_empty()).count();
    (params.len() - choices, choices)
}

/// Every effect in the chain, then the + button.
fn effect_cards(ui: &mut egui::Ui, effects: &mut Vec<Effect>, owner: u64, lib: &Library, clock: Clock) {
    let mut remove = None;
    let mut swap = None;
    let n = effects.len();
    for (i, e) in effects.iter_mut().enumerate() {
        let h = ui.available_height();
        let (mut knobs, mut choices) = count(&e.params);
        let mut extra = usize::from(e.kind == EffectKind::Feedback) + usize::from(e.def().history.is_some()) + usize::from(e.takes_model());
        if let Some(c) = e.custom.as_deref() {
            let (k, ch) = count(&c.params);
            knobs += k;
            choices += ch + 1;
            extra += 1;
        }
        let width = knob_card_width(knobs, choices, extra, h);
        let title = e.name().to_string();
        let enabled = e.enabled;
        let id = Id::new(("fx", owner, e.id));
        let Some((mut head, mut inner)) = card_frame(ui, id, width, h, &title) else { continue };
        if e.takes_model() {
            // A model dragged from the browser onto the card becomes its object. Registered
            // before the card's controls, so they stay on top for the pointer.
            let r = ui.interact(head.max_rect().union(inner.max_rect()), id.with("model drop"), Sense::hover());
            if r.dnd_hover_payload::<Drag>().is_some_and(|d| d.kind == Kind::Model) {
                ui.painter().rect_stroke(r.rect.expand(4.0), 6.0, Stroke::new(2.0, theme::QUEUED), StrokeKind::Inside);
            }
            if let Some(d) = r.dnd_release_payload::<Drag>().filter(|d| d.kind == Kind::Model) {
                if let Some(m) = loaded(ui, id, Some(lib.model(&d.key))) {
                    e.set_model(m);
                }
            }
        }
        {
            let ui = &mut head;
            // On / bypass light.
            let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
            let c = r.center();
            if enabled {
                ui.painter().circle_filled(c, 7.0, theme::LIVE.gamma_multiply(0.18));
                ui.painter().circle_filled(c, 4.0, theme::LIVE);
            } else {
                ui.painter().circle_stroke(c, 4.0, Stroke::new(1.5, theme::FAINT));
            }
            if resp.on_hover_text(if enabled { "On: click to bypass" } else { "Bypassed: click to turn on" }).clicked() {
                e.enabled = !enabled;
            }
            let name = RichText::new(&title).font(theme::semibold(13.5)).color(if enabled { theme::TEXT_STRONG } else { theme::MUTED });
            ui.add(egui::Label::new(name).truncate().selectable(false)).on_hover_text("Double-click the header to fold");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let small = |t: &str| egui::Button::new(RichText::new(t).color(theme::MUTED)).frame(false).min_size(vec2(16.0, 18.0));
                if ui.add(small("×")).on_hover_text("Remove effect").clicked() {
                    remove = Some(i);
                }
                if i + 1 < n && ui.add(small("▶")).on_hover_text("Move right").clicked() {
                    swap = Some((i, i + 1));
                }
                if i > 0 && ui.add(small("◀")).on_hover_text("Move left").clicked() {
                    swap = Some((i - 1, i));
                }
            });
        }
        card_body(&mut inner, id, |ui| effect_body(ui, e, lib, clock));
    }
    if let Some(i) = remove {
        effects.remove(i);
    }
    if let Some((a, b)) = swap {
        effects.swap(a, b);
    }
    add_button(ui, effects);
}

fn effect_body(ui: &mut egui::Ui, e: &mut Effect, lib: &Library, clock: Clock) {
    if e.takes_model() {
        let id = Id::new(("fx model", e.id));
        let current = e.model.as_ref().map_or("Utah Teapot", |m| m.name()).to_string();
        let picked = model_menu(ui, id, &current, lib, "Pick the object for the shape \"Model\" (or drag one from the browser onto this card)");
        if let Some(m) = loaded(ui, id, picked) {
            e.set_model(m);
        }
    }
    if e.kind == EffectKind::Feedback {
        egui::ComboBox::from_id_salt(("fb preset", e.id))
            .selected_text("Preset…")
            .width(ui.available_width().min(200.0))
            .show_ui(ui, |ui| {
                for (pi, name) in FEEDBACK_PRESETS.iter().enumerate() {
                    if ui.selectable_label(false, *name).clicked() {
                        apply_feedback_preset(e, pi);
                    }
                }
            });
    }
    if let Some(c) = e.custom.as_deref_mut() {
        ui.horizontal(|ui| {
            if ui.button("{ } Edit code").clicked() {
                shader_editor::request_open(ui.ctx(), c.id);
            }
            shader_editor::status(ui, c);
        });
    }
    widgets::param_grid(ui, e.params.iter_mut(), clock);
    if let Some(c) = e.custom.as_deref_mut() {
        shader_editor::params(ui, c, clock);
    }
    if let Some(h) = e.def().history {
        ui.checkbox(&mut e.half_history, RichText::new("Half-size history").small()).on_hover_text(format!(
            "Keep the {} frames of history at half the output size: a quarter of the memory, a little softer. Changing it restarts the history.",
            h.frames
        ));
    }
}

/// A "Model ▾" row listing the library's models by category; returns the one picked (loaded).
/// Shows the last load error under it.
fn model_menu(ui: &mut egui::Ui, id: Id, current: &str, lib: &Library, hint: &str) -> Option<Result<ModelRef, String>> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Model").color(theme::MUTED));
        egui::ComboBox::from_id_salt(id)
            .selected_text(current)
            .width((ui.available_width() - 8.0).clamp(80.0, 220.0))
            .height(420.0)
            .show_ui(ui, |ui| {
                let mut last = "";
                for e in lib.entries.iter().filter(|e| e.kind == Kind::Model && e.status == Status::Ok) {
                    if e.category != last {
                        ui.label(theme::caption(&e.category));
                        last = &e.category;
                    }
                    if ui.selectable_label(e.name == current, &e.name).on_hover_text(&e.description).clicked() {
                        picked = Some(e.key.clone());
                    }
                }
            })
            .response
            .on_hover_text(hint);
    });
    if let Some(err) = ui.data(|d| d.get_temp::<String>(id.with("error"))) {
        ui.colored_label(theme::RECORD, err);
    }
    picked.map(|k| lib.model(&k))
}

/// The picked model, if it loaded; otherwise remember why not, for `model_menu` to show.
fn loaded(ui: &egui::Ui, id: Id, picked: Option<Result<ModelRef, String>>) -> Option<ModelRef> {
    match picked? {
        Ok(m) => {
            ui.data_mut(|d| d.remove::<String>(id.with("error")));
            Some(m)
        }
        Err(e) => {
            ui.data_mut(|d| d.insert_temp(id.with("error"), e));
            None
        }
    }
}

/// The dashed "+" slot at the end of a chain: opens the effect menu.
fn add_button(ui: &mut egui::Ui, effects: &mut Vec<Effect>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, ui.available_height()), Sense::click());
    let resp = resp.on_hover_text("Add an effect (or drag one from the browser onto the layer)");
    let painter = ui.painter_at(rect);
    let c = if resp.hovered() { theme::TEXT } else { theme::FAINT };
    let r = rect.shrink(0.5);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, if resp.hovered() { theme::LINE_HI } else { theme::LINE }), 4.0, 3.0));
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, "+", theme::body(20.0), c);
    egui::Popup::menu(&resp).show(|ui| add_effect_menu(ui, effects));
}

fn add_effect_menu(ui: &mut egui::Ui, effects: &mut Vec<Effect>) {
    let mut last_cat = "";
    for d in EFFECTS {
        if d.category != last_cat {
            if !last_cat.is_empty() {
                ui.separator();
            }
            ui.label(theme::caption(d.category));
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
    ui.label(theme::caption("Code"));
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
}

/// The clip: transport, timing, generator / shader controls and fit.
fn source_card(ui: &mut egui::Ui, id: Id, clip: Option<&mut Clip>, color: Color32, lib: &Library, clock: Clock) {
    let h = ui.available_height();
    let Some(clip) = clip else {
        card(ui, id, 220.0, h, "Source", |ui| {
            ui.label(RichText::new("SOURCE").font(theme::semibold(11.0)).color(color));
        }, |ui| {
            ui.label(RichText::new("Nothing playing. Select a clip in the grid, or right-click an empty cell to load media, a generator or a camera.").color(theme::MUTED));
        });
        return;
    };
    let width = match &clip.media {
        Media::Shader(s) => knob_card_width(count(&s.params).0, count(&s.params).1 + 1, 3, h).max(280.0),
        Media::Model(m) => knob_card_width(count(&m.params).0, count(&m.params).1 + 1, 3, h).max(300.0),
        _ => 300.0,
    };
    let mut name = std::mem::take(&mut clip.name);
    card(ui, id, width, h, &name.clone(), |ui| {
        ui.label(RichText::new("SOURCE").font(theme::semibold(11.0)).color(color));
        ui.add(egui::TextEdit::singleline(&mut name).frame(egui::Frame::NONE).font(theme::semibold(13.5)).desired_width(f32::INFINITY))
            .on_hover_text("Clip name");
    }, |ui| source_body(ui, clip, lib, clock));
    clip.name = name;
}

fn source_body(ui: &mut egui::Ui, clip: &mut Clip, lib: &Library, clock: Clock) {
    if let Some(e) = clip.error() {
        ui.colored_label(theme::RECORD, e);
    }
    if clip.is_timeline() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.selectable_value(&mut clip.direction, Direction::Reverse, "◀").on_hover_text("Play backwards");
            ui.selectable_value(&mut clip.direction, Direction::Paused, "⏸").on_hover_text("Pause");
            ui.selectable_value(&mut clip.direction, Direction::Forward, "▶").on_hover_text("Play forwards");
            ui.add_space(6.0);
            let len = clip.length();
            let mut pos = clip.position as f32;
            ui.spacing_mut().slider_width = (ui.available_width() - 48.0).max(60.0);
            if ui
                .add(egui::Slider::new(&mut pos, 0.0..=(len.max(2) - 1) as f32).show_value(false).trailing_fill(true))
                .on_hover_text("Scrub")
                .changed()
            {
                clip.position = pos as f64;
                clip.finished = false;
            }
            ui.label(RichText::new(format!("{:.1}s", clip.position as f32 / clip.fps())).font(theme::mono(11.0)));
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("loop mode", clip.id))
                .selected_text(clip.loop_mode.name())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for m in LoopMode::ALL {
                        if ui.selectable_label(clip.loop_mode == m, m.name()).clicked() {
                            clip.loop_mode = m;
                            clip.finished = false;
                        }
                    }
                })
                .response
                .on_hover_text("Loop mode");
            ui.selectable_value(&mut clip.sync, Sync::Timeline, "Free").on_hover_text("Native frame rate × speed");
            ui.selectable_value(&mut clip.sync, Sync::Bpm, "BPM sync").on_hover_text("Stretch the clip to a number of beats");
        });
        ui.horizontal(|ui| {
            if clip.sync == Sync::Bpm {
                widgets::knob(ui, &mut clip.beats, clock);
            }
            widgets::knob(ui, &mut clip.speed, clock);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (label, f) in [("×2", 2.0), ("÷2", 0.5)] {
                    if ui.small_button(label).clicked() {
                        clip.speed.set(clip.speed.value * f);
                    }
                }
                if ui.small_button("1×").clicked() {
                    clip.speed.set(1.0);
                }
            });
        });
    }
    if let Media::Generator(g) = &mut clip.media {
        widgets::param_grid(ui, [&mut g.pattern, &mut g.freq, &mut g.speed, &mut g.hue], clock);
    }
    if let Media::Model(m) = &mut clip.media {
        let id = Id::new(("clip model", clip.id));
        let picked = model_menu(ui, id, &m.model.name().to_string(), lib, "Swap the model (keeps the settings)");
        if let Some(r) = loaded(ui, id, picked) {
            m.model = r;
        }
        widgets::param_grid(ui, m.params.iter_mut(), clock);
    }
    if let Media::Shader(s) = &mut clip.media {
        ui.horizontal(|ui| {
            if ui.button("{ } Edit code").clicked() {
                shader_editor::request_open(ui.ctx(), s.id);
            }
            shader_editor::status(ui, s);
        });
        shader_editor::params(ui, s, clock);
    }
    ui.horizontal(|ui| {
        for f in Fit::ALL {
            ui.selectable_value(&mut clip.fit, f, f.name());
        }
    });
    let info = match &clip.media {
        Media::Video(v) if !v.is_loaded() => {
            ui.add(egui::ProgressBar::new(v.progress()).text("importing").desired_height(14.0));
            None
        }
        Media::Video(v) => Some(format!(
            "{} frames @ {:.2} fps · {:.1} s · {:.0} MB",
            v.frame_count(),
            v.fps,
            clip.duration_secs(),
            v.memory_bytes() as f64 / 1e6
        )),
        Media::Image { frame, .. } => Some(format!("Still image {}×{}", frame.width, frame.height)),
        Media::Camera { index, .. } => Some(format!("Live capture device {index}")),
        Media::Generator(_) => Some("Generator".into()),
        Media::Shader(_) => None,
        Media::Model(m) => Some(m.model.model.summary()),
    };
    if let Some(info) = info {
        let r = ui.label(RichText::new(info).small().color(theme::FAINT));
        match &clip.media {
            Media::Video(v) => {
                r.on_hover_text(v.path.display().to_string());
            }
            Media::Model(m) => {
                r.on_hover_text(&m.model.key);
            }
            _ => {}
        }
    }
}
