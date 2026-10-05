//! Parameter controls: knobs (and dropdown cells for choices) with a pop-out automation editor.
//!
//! Right-click a control and pick *Automate…* to attach a modulator (if there isn't one yet)
//! and open an editor with shape, rate, depth, polarity and phase, plus a live plot of the
//! signal. In Envelope mode you can draw the curve in the plot.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::audio::Band;
use crate::modulation::{BEAT_CHOICES, Clock, Modulator, Polarity, Rate, Shape};
use crate::param::Param;

/// Automation pink (see `theme`).
pub const ACCENT: Color32 = crate::ui::theme::MOD;

/// Width of one knob cell (knob, value and label).
pub const KNOB_W: f32 = 58.0;
const KNOB_R: f32 = 15.0;
/// The knob's sweep: 135° either side of 12 o'clock.
const SWEEP: f32 = 135.0;

fn accent_id() -> egui::Id {
    egui::Id::new("knob accent")
}

/// Color for the value arcs of the knobs drawn after this (the layer's color, say).
pub fn set_accent(ctx: &egui::Context, c: Color32) {
    ctx.data_mut(|d| d.insert_temp(accent_id(), c));
}

fn accent(ui: &egui::Ui) -> Color32 {
    ui.ctx().data(|d| d.get_temp(accent_id())).unwrap_or(crate::ui::theme::LIVE)
}

/// Point on a knob ring: `deg` is clockwise from 12 o'clock.
fn ring_point(c: Pos2, r: f32, deg: f32) -> Pos2 {
    let a = deg.to_radians();
    pos2(c.x + r * a.sin(), c.y - r * a.cos())
}

fn arc(painter: &egui::Painter, c: Pos2, r: f32, from: f32, to: f32, stroke: Stroke) {
    let (a, b) = if from <= to { (from, to) } else { (to, from) };
    if b - a < 0.5 {
        return;
    }
    let n = ((b - a) / 6.0).ceil().max(2.0) as usize;
    let pts: Vec<Pos2> = (0..=n).map(|i| ring_point(c, r, a + (b - a) * i as f32 / n as f32)).collect();
    painter.add(egui::Shape::line(pts, stroke));
}

fn format_value(spec: &crate::param::Spec, v: f32) -> String {
    if spec.int {
        return format!("{v:.0}");
    }
    let a = v.abs();
    if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

/// Width of a [`choice`] cell.
pub const CHOICE_W: f32 = 120.0;

/// Choice params as small dropdown cells, then the rest as a wrapping grid of knobs.
pub fn param_grid<'a>(ui: &mut egui::Ui, params: impl IntoIterator<Item = &'a mut Param>, clock: Clock) {
    let (mut knobs, mut choices) = (Vec::new(), Vec::new());
    for p in params {
        if p.spec.choices.is_empty() { knobs.push(p) } else { choices.push(p) }
    }
    if !choices.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            for p in choices {
                choice(ui, p, clock);
            }
        });
    }
    if knobs.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 6.0);
        for p in knobs {
            knob(ui, p, clock);
        }
    });
}

/// A rotary knob: drag up / down (Shift = fine), scroll to nudge, double-click to reset,
/// right-click for automation, MIDI learn and reset. A pink outer arc shows the automation
/// range, with a dot at the live value.
pub fn knob(ui: &mut egui::Ui, p: &mut Param, clock: Clock) -> egui::Response {
    knob_labeled(ui, p, p.spec.label, clock)
}

pub fn knob_labeled(ui: &mut egui::Ui, p: &mut Param, label: &str, clock: Clock) -> egui::Response {
    use crate::ui::theme;
    let size = vec2(KNOB_W, 40.0 + 14.0 + 14.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let spec = p.spec;

    // Drag accumulates an unsnapped position so integer knobs still move smoothly.
    let acc_id = resp.id.with("acc");
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(acc_id, p.normalized(p.value)));
    }
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let dy = -resp.drag_delta().y / if fine { 900.0 } else { 180.0 };
        let t = ui.data(|d| d.get_temp::<f32>(acc_id)).unwrap_or(p.normalized(p.value)) + dy;
        let t = t.clamp(0.0, 1.0);
        ui.data_mut(|d| d.insert_temp(acc_id, t));
        p.set_normalized(t);
        resp.mark_changed();
    }
    if resp.double_clicked() {
        p.set(spec.default);
        resp.mark_changed();
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let step = if spec.int { 1.0 / (spec.max - spec.min).max(1.0) } else { 0.01 };
            let t = p.normalized(p.value) + step * (scroll / 40.0).clamp(-1.0, 1.0).signum();
            p.set_normalized(t);
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            resp.mark_changed();
        }
    }

    let painter = ui.painter_at(rect.expand(2.0));
    let c = pos2(rect.center().x, rect.top() + 20.0);
    let angle = |t: f32| -SWEEP + 2.0 * SWEEP * t;
    let color = accent(ui);
    let hot = resp.hovered() || resp.dragged();
    arc(&painter, c, KNOB_R, -SWEEP, SWEEP, Stroke::new(3.5, theme::CONTROL));
    let t = p.normalized(p.value);
    let from = if spec.min < 0.0 && spec.max > 0.0 { angle(p.normalized(0.0)) } else { -SWEEP };
    arc(&painter, c, KNOB_R, from, angle(t), Stroke::new(3.5, color));
    let modulated = p.is_automated();
    if modulated && let Some(m) = &p.modulator {
        let a = p.normalized(m.apply(p.value, spec.min, spec.max, 0.0));
        let b = p.normalized(m.apply(p.value, spec.min, spec.max, 1.0));
        arc(&painter, c, KNOB_R + 3.5, angle(a), angle(b), Stroke::new(2.0, ACCENT.gamma_multiply(0.75)));
        painter.circle_filled(ring_point(c, KNOB_R + 3.5, angle(p.normalized(p.live))), 2.6, ACCENT);
    }
    let cap = if hot { theme::CONTROL_HI } else { theme::CONTROL };
    let ring = if resp.dragged() {
        theme::LIVE
    } else if modulated {
        ACCENT
    } else if hot {
        theme::FAINT
    } else {
        theme::LINE_HI
    };
    painter.circle(c, 10.5, cap, Stroke::new(1.0, ring));
    painter.line_segment([c, ring_point(c, 8.0, angle(t))], Stroke::new(2.0, theme::TEXT_STRONG));

    let key = crate::ui::midi::LearnKey::Param(p.seed);
    if crate::ui::midi::is_learning(ui, key) {
        painter.circle_stroke(c, KNOB_R + 4.0, Stroke::new(1.5, ACCENT));
    } else if crate::ui::midi::mapping(ui, key).is_some() {
        let r = egui::Rect::from_min_size(pos2(c.x + 12.0, rect.top()), vec2(10.0, 11.0));
        painter.rect_filled(r, 2.0, theme::LIVE);
        painter.text(r.center(), egui::Align2::CENTER_CENTER, "M", theme::bold(9.0), theme::ON_LIT);
    }

    let shown = if modulated { p.live } else { p.value };
    painter.text(
        pos2(rect.center().x, rect.top() + 41.0),
        egui::Align2::CENTER_TOP,
        format_value(&spec, shown),
        theme::mono(11.0),
        if hot { theme::TEXT_STRONG } else { theme::TEXT },
    );
    let galley = painter.layout(label.to_string(), theme::body(12.0), theme::MUTED, f32::INFINITY);
    let label_pos = pos2(rect.center().x - galley.size().x.min(KNOB_W) / 2.0, rect.top() + 54.0);
    let label_rect = egui::Rect::from_min_size(pos2(rect.left(), label_pos.y), vec2(KNOB_W, 14.0));
    ui.painter_at(label_rect).galley(label_pos, galley, theme::MUTED);

    let resp = resp.on_hover_ui(|ui| {
        ui.label(RichText::new(format!("{label}: {}", format_value(&spec, p.value))).strong());
        if let Some(m) = p.modulator.as_ref().filter(|m| m.enabled) {
            ui.label(RichText::new(format!("~ {} · {} · {:.0}%", m.shape.name(), source_label(m), m.depth * 100.0)).color(ACCENT));
        }
        ui.label(RichText::new("drag · Shift fine · double-click reset · right-click: automate, MIDI").small().weak());
    });

    automation_menu(&resp, p, label, clock);
    resp
}

/// Right-click menu (automate, MIDI learn, reset) and the automation editor popup for `resp`.
fn automation_menu(resp: &egui::Response, p: &mut Param, label: &str, clock: Clock) {
    let spec = p.spec;
    let key = crate::ui::midi::LearnKey::Param(p.seed);
    let popup_id = resp.id.with("automation");
    resp.context_menu(|ui| {
        let label_text = if p.modulator.is_some() { "Edit automation…" } else { "Automate…" };
        if ui.button(label_text).clicked() {
            egui::Popup::open_id(ui.ctx(), popup_id);
        }
        if p.modulator.is_some() && ui.button("Remove automation").clicked() {
            p.modulator = None;
            ui.close();
        }
        ui.separator();
        crate::ui::midi::menu(ui, key);
        ui.separator();
        if ui.button("Reset to default").clicked() {
            p.set(spec.default);
            ui.close();
        }
    });
    egui::Popup::from_response(resp)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let base = p.value;
            let seed = p.seed;
            let m = p.modulator.get_or_insert_with(|| Modulator::new(seed));
            if editor(ui, &spec, label, m, base, clock) {
                p.modulator = None;
                egui::Popup::close_id(ui.ctx(), popup_id);
            }
        });
}

/// A choice param as a small labelled dropdown cell (for device cards). Right-click to
/// automate or learn MIDI; it turns pink while automated.
pub fn choice(ui: &mut egui::Ui, p: &mut Param, clock: Clock) -> egui::Response {
    use crate::ui::theme;
    let spec = p.spec;
    let active = p.is_automated();
    let inner = ui.allocate_ui_with_layout(vec2(CHOICE_W, 40.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let label = RichText::new(spec.label).size(11.5).color(if active { ACCENT } else { theme::MUTED });
        ui.add(egui::Label::new(label).truncate().selectable(false));
        let mut idx = p.value.round() as usize;
        let shown = if active { p.index() } else { idx };
        let r = egui::ComboBox::from_id_salt(("choice", p.seed))
            .selected_text(RichText::new(spec.choices[shown.min(spec.choices.len() - 1)]).size(12.5))
            .width(CHOICE_W - 4.0)
            .show_ui(ui, |ui| {
                for (i, c) in spec.choices.iter().enumerate() {
                    ui.selectable_value(&mut idx, i, *c);
                }
            })
            .response;
        if idx as f32 != p.value.round() {
            p.set(idx as f32);
        }
        r
    });
    let resp = inner.inner;
    let key = crate::ui::midi::LearnKey::Param(p.seed);
    if crate::ui::midi::mapping(ui, key).is_some() {
        ui.painter().text(resp.rect.right_top() + vec2(-2.0, -12.0), egui::Align2::RIGHT_TOP, "M", theme::bold(9.5), theme::LIVE);
    }
    automation_menu(&resp, p, spec.label, clock);
    resp
}

fn beats_label(b: f32) -> String {
    if b < 1.0 { format!("1/{}", (1.0 / b).round()) } else { format!("{b}") }
}

/// Returns true when the user asked to remove the automation.
fn editor(ui: &mut egui::Ui, spec: &crate::param::Spec, label: &str, m: &mut Modulator, base: f32, clock: Clock) -> bool {
    ui.set_width(320.0);
    let mut remove = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Automate · {label}")).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            remove = ui.button("Remove").clicked();
            ui.checkbox(&mut m.enabled, "on");
        });
    });
    ui.separator();

    ui.horizontal_wrapped(|ui| {
        for s in Shape::ALL {
            if ui.selectable_value(&mut m.shape, s, s.name()).clicked() && s == Shape::Audio {
                // Audio is 0 in silence: push the value up from the slider rather than swing around it.
                m.polarity = Polarity::Up;
            }
        }
    });

    if m.shape == Shape::Audio {
        ui.horizontal(|ui| {
            ui.label("Follow");
            for b in Band::ALL {
                ui.selectable_value(&mut m.band, b, b.name());
            }
        });
        if !clock.audio.active {
            ui.label(RichText::new("No audio input: pick one in the top bar.").small().weak());
        }
    } else {
        rate_editor(ui, m);
    }

    ui.add(egui::Slider::new(&mut m.depth, 0.0..=1.0).text("depth"));
    ui.horizontal(|ui| {
        for p in Polarity::ALL {
            ui.selectable_value(&mut m.polarity, p, p.name());
        }
    });
    if m.shape != Shape::Audio {
        ui.add(egui::Slider::new(&mut m.phase, 0.0..=1.0).text("phase"));
    }
    if m.shape == Shape::Square {
        ui.add(egui::Slider::new(&mut m.width, 0.02..=0.98).text("pulse width"));
    }

    if m.shape == Shape::Audio {
        meter_plot(ui, m, clock);
    } else {
        plot(ui, m, clock);
    }

    let a = m.apply(base, spec.min, spec.max, 0.0);
    let b = m.apply(base, spec.min, spec.max, 1.0);
    ui.label(
        RichText::new(format!("slider {base:.3} · sweeps {:.3} … {:.3}", a.min(b), a.max(b)))
            .small()
            .weak(),
    );
    if m.shape == Shape::Envelope {
        ui.label(RichText::new("click: add point · drag: move · right-click: delete").small().weak());
    }
    remove
}

fn rate_editor(ui: &mut egui::Ui, m: &mut Modulator) {
    ui.horizontal(|ui| {
        ui.label("Rate");
        let synced = matches!(m.rate, Rate::Beats(_));
        if ui.selectable_label(synced, "Beats").clicked() && !synced {
            m.rate = Rate::Beats(4.0);
        }
        if ui.selectable_label(!synced, "Hz").clicked() && synced {
            m.rate = Rate::Hz(0.5);
        }
    });
    match &mut m.rate {
        Rate::Beats(beats) => {
            ui.horizontal_wrapped(|ui| {
                for b in BEAT_CHOICES {
                    ui.selectable_value(beats, b, beats_label(b));
                }
            });
        }
        Rate::Hz(hz) => {
            ui.add(egui::Slider::new(hz, 0.01..=20.0).logarithmic(true).text("Hz"));
        }
    }
}

/// The Audio shape's current value, as a bar.
fn meter_plot(ui: &mut egui::Ui, m: &Modulator, clock: Clock) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let v = m.signal(clock);
    let color = if m.enabled { ACCENT } else { ui.visuals().weak_text_color() };
    painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width() * v, rect.height())), 4.0, color);
    ui.ctx().request_repaint();
}

/// One cycle of the signal (four steps for the random shapes) with a playhead.
/// In Envelope mode the breakpoints are editable.
fn plot(ui: &mut egui::Ui, m: &mut Modulator, clock: Clock) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(4.0));
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    painter.line_segment(
        [pos2(rect.left(), rect.center().y), pos2(rect.right(), rect.center().y)],
        Stroke::new(1.0, ui.visuals().weak_text_color().gamma_multiply(0.3)),
    );

    if m.shape == Shape::Envelope {
        edit_envelope(ui, m, rect, &resp);
    }

    let pos = m.position(clock);
    let cycles = if m.shape.is_random() { 4.0 } else { 1.0 };
    let start = pos.floor();
    let to_y = |v: f32| rect.bottom() - v * rect.height();
    let line: Vec<Pos2> = (0..=200)
        .map(|i| {
            let t = i as f32 / 200.0;
            pos2(rect.left() + t * rect.width(), to_y(m.signal_at(start + t as f64 * cycles)))
        })
        .collect();
    let color = if m.enabled { ACCENT } else { ui.visuals().weak_text_color() };
    painter.add(egui::Shape::line(line, Stroke::new(1.5, color)));

    let t = ((pos - start) / cycles) as f32;
    let x = rect.left() + t * rect.width();
    painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, Color32::from_white_alpha(90)));
    painter.circle_filled(pos2(x, to_y(m.signal_at(pos))), 4.0, color);

    if m.shape == Shape::Envelope {
        for p in &m.points {
            let c = pos2(rect.left() + p[0] * rect.width(), to_y(p[1]));
            painter.circle(c, 4.5, ui.visuals().extreme_bg_color, Stroke::new(1.5, Color32::WHITE));
        }
    }
    // Keep the playhead moving while the editor is open.
    ui.ctx().request_repaint();
}

fn edit_envelope(ui: &egui::Ui, m: &mut Modulator, rect: Rect, resp: &egui::Response) {
    let to_screen = |p: [f32; 2]| pos2(rect.left() + p[0] * rect.width(), rect.bottom() - p[1] * rect.height());
    let from_screen = |q: Pos2| {
        [
            ((q.x - rect.left()) / rect.width()).clamp(0.0, 0.999),
            ((rect.bottom() - q.y) / rect.height()).clamp(0.0, 1.0),
        ]
    };
    let nearest = |pts: &[[f32; 2]], q: Pos2| {
        pts.iter()
            .enumerate()
            .map(|(i, p)| (i, to_screen(*p).distance(q)))
            .filter(|(_, d)| *d < 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let drag_id = resp.id.with("env-drag");

    if resp.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        let idx = nearest(&m.points, origin);
        ui.data_mut(|d| d.insert_temp(drag_id, idx));
    }
    if resp.dragged()
        && let Some(Some(i)) = ui.data(|d| d.get_temp::<Option<usize>>(drag_id))
        && let Some(q) = resp.interact_pointer_pos()
        && i < m.points.len()
    {
        // Points keep their order: x is clamped between the neighbours.
        let mut p = from_screen(q);
        let lo = if i == 0 { 0.0 } else { m.points[i - 1][0] + 0.002 };
        let hi = if i + 1 == m.points.len() { 0.999 } else { m.points[i + 1][0] - 0.002 };
        p[0] = p[0].clamp(lo, hi.max(lo));
        m.points[i] = p;
    }
    if resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<Option<usize>>(drag_id));
    }

    if let Some(q) = resp.interact_pointer_pos() {
        let hit = nearest(&m.points, q);
        if (resp.secondary_clicked() || resp.double_clicked()) && m.points.len() > 2 {
            if let Some(i) = hit {
                m.points.remove(i);
            }
        } else if resp.clicked() && hit.is_none() {
            let p = from_screen(q);
            let at = m.points.iter().position(|e| e[0] > p[0]).unwrap_or(m.points.len());
            m.points.insert(at, p);
        }
    }
}

fn rate_label(rate: Rate) -> String {
    match rate {
        Rate::Beats(b) => format!("{} beat{}", beats_label(b), if b == 1.0 { "" } else { "s" }),
        Rate::Hz(hz) => format!("{hz:.2} Hz"),
    }
}

/// What drives a modulator, for lists: its rate, or the audio band it follows.
pub fn source_label(m: &Modulator) -> String {
    match m.shape {
        Shape::Audio => m.band.name().to_string(),
        _ => rate_label(m.rate),
    }
}

/// What a click in the Modulators list asks for: show that layer's chain (`Some`) or the
/// master's (`None`).
pub struct Jump(pub Option<usize>);

/// The Modulators tab: every automated parameter, with a live plot of its signal. Click a
/// row's name to jump to its device.
pub fn modulators(ui: &mut egui::Ui, comp: &mut crate::composition::Composition, clock: Clock) -> Option<Jump> {
    use crate::ui::theme;
    let colors: Vec<Color32> = comp.layers.iter().map(|l| theme::layer_color(l.color)).collect();
    let mut jump = None;
    let mut any = false;
    egui::ScrollArea::vertical().id_salt("modulators").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("modulators grid").num_columns(7).spacing(vec2(14.0, 4.0)).min_row_height(32.0).show(ui, |ui| {
            for h in ["", "SIGNAL", "TARGET", "SHAPE", "RATE", "DEPTH", ""] {
                ui.label(RichText::new(h).font(theme::semibold(10.5)).color(theme::FAINT));
            }
            ui.end_row();
            comp.visit_all(&mut |owner, path, p| {
                let Some(m) = p.modulator.as_mut() else { return };
                any = true;
                ui.checkbox(&mut m.enabled, "").on_hover_text("On / off");
                mini_plot(ui, m, clock, vec2(120.0, 28.0));
                ui.horizontal(|ui| {
                    ui.set_min_width(260.0);
                    let c = owner.and_then(|i| colors.get(i).copied()).unwrap_or(theme::LIVE);
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, c);
                    let text = RichText::new(path).color(if m.enabled { theme::TEXT } else { theme::FAINT });
                    if ui.add(egui::Label::new(text).sense(Sense::click())).on_hover_text("Show this device").clicked() {
                        jump = Some(Jump(owner));
                    }
                });
                ui.label(RichText::new(m.shape.name()).color(theme::MUTED));
                ui.label(RichText::new(source_label(m)).color(theme::MUTED));
                let mut pct = m.depth * 100.0;
                if ui.add(egui::DragValue::new(&mut pct).range(0.0..=100.0).suffix("%").max_decimals(0)).changed() {
                    m.depth = pct / 100.0;
                }
                if ui.small_button("×").on_hover_text("Remove automation").clicked() {
                    p.modulator = None;
                }
                ui.end_row();
            });
        });
        if !any {
            ui.add_space(12.0);
            ui.label(RichText::new("Nothing automated yet. Right-click any knob → Automate…").color(theme::MUTED));
        }
    });
    if any {
        ui.ctx().request_repaint();
    }
    jump
}

/// A small live plot: one cycle of the signal and a playhead, or the level for Audio.
fn mini_plot(ui: &mut egui::Ui, m: &Modulator, clock: Clock, size: egui::Vec2) {
    use crate::ui::theme;
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, theme::GROUND);
    let color = if m.enabled { if m.shape == Shape::Audio { theme::AUDIO } else { ACCENT } } else { theme::FAINT };
    let inner = rect.shrink2(vec2(2.0, 3.0));
    if m.shape == Shape::Audio {
        let v = if m.enabled { m.signal(clock) } else { 0.0 };
        painter.rect_filled(Rect::from_min_size(inner.min, vec2(inner.width() * v, inner.height())), 2.0, color);
        return;
    }
    let pos = m.position(clock);
    let cycles = if m.shape.is_random() { 4.0 } else { 1.0 };
    let start = pos.floor();
    let to_y = |v: f32| inner.bottom() - v * inner.height();
    let line: Vec<Pos2> = (0..=60)
        .map(|i| {
            let t = i as f32 / 60.0;
            pos2(inner.left() + t * inner.width(), to_y(m.signal_at(start + t as f64 * cycles)))
        })
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(1.5, color)));
    let t = ((pos - start) / cycles) as f32;
    painter.circle_filled(pos2(inner.left() + t * inner.width(), to_y(m.signal_at(pos))), 3.0, color);
}
