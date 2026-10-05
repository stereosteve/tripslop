//! Sliders with a pop-out automation editor.
//!
//! Every slider gets a small `~` button. Clicking it attaches a modulator (if there isn't one
//! yet) and opens an editor with shape, rate, depth, polarity and phase, plus a live plot of
//! the signal. In Envelope mode you can draw the curve in the plot.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::audio::Band;
use crate::modulation::{BEAT_CHOICES, Clock, Modulator, Polarity, Rate, Shape};
use crate::param::Param;

/// Automation pink (see `theme`).
pub const ACCENT: Color32 = crate::ui::theme::MOD;

/// A parameter slider (or dropdown, for choices) with an automation button.
pub fn param(ui: &mut egui::Ui, p: &mut Param, clock: Clock) -> egui::Response {
    param_labeled(ui, p, p.spec.label, clock)
}

pub fn param_labeled(ui: &mut egui::Ui, p: &mut Param, label: &str, clock: Clock) -> egui::Response {
    ui.horizontal(|ui| {
        let active = p.is_automated();
        let text = RichText::new("~").monospace().color(if active { Color32::BLACK } else { ui.visuals().weak_text_color() });
        let mut button = egui::Button::new(text).small().selected(active);
        if active {
            button = button.fill(ACCENT);
        }
        let btn = ui.add(button).on_hover_text(if p.modulator.is_some() {
            "Edit automation"
        } else {
            "Automate this parameter"
        });

        let spec = p.spec;
        let resp = if !spec.choices.is_empty() {
            let mut idx = p.value.round() as usize;
            let shown = if active { p.index() } else { idx };
            let r = egui::ComboBox::from_id_salt(ui.next_auto_id())
                .selected_text(spec.choices[shown.min(spec.choices.len() - 1)])
                .show_ui(ui, |ui| {
                    for (i, c) in spec.choices.iter().enumerate() {
                        ui.selectable_value(&mut idx, i, *c);
                    }
                })
                .response;
            if idx as f32 != p.value.round() {
                p.set(idx as f32);
            }
            ui.label(label);
            r
        } else {
            let mut slider = egui::Slider::new(&mut p.value, spec.min..=spec.max).logarithmic(spec.log).text(label);
            if spec.int {
                slider = slider.integer();
            }
            let r = ui.add(slider);
            if active && let Some(m) = &p.modulator {
                paint_live_marker(ui, &r, p, m);
            }
            r
        };
        let key = crate::ui::midi::LearnKey::Param(p.seed);
        resp.context_menu(|ui| crate::ui::midi::menu(ui, key));
        crate::ui::midi::badge(ui, key);

        egui::Popup::from_toggle_button_response(&btn)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                let base = p.value;
                let seed = p.seed;
                let m = p.modulator.get_or_insert_with(|| Modulator::new(seed));
                if editor(ui, &spec, label, m, base, clock) {
                    p.modulator = None;
                    ui.close();
                }
            });
        resp
    })
    .inner
}

/// Dot at the automated value plus a band showing the sweep range, drawn over the rail.
fn paint_live_marker(ui: &egui::Ui, resp: &egui::Response, p: &Param, m: &Modulator) {
    let rail_h = ui.spacing().interact_size.y;
    let r = rail_h / 2.5;
    let left = resp.rect.left() + r;
    let width = ui.spacing().slider_width - 2.0 * r;
    let y = resp.rect.center().y + rail_h * 0.5 - 1.0;
    let x_of = |v: f32| left + p.normalized(v) * width;
    let a = m.apply(p.value, p.spec.min, p.spec.max, 0.0);
    let b = m.apply(p.value, p.spec.min, p.spec.max, 1.0);
    let painter = ui.painter();
    painter.line_segment([pos2(x_of(a), y), pos2(x_of(b), y)], Stroke::new(2.0, ACCENT.gamma_multiply(0.5)));
    painter.circle_filled(pos2(x_of(p.live), y), 3.0, ACCENT);
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

/// Compact list of every automated parameter in the composition.
pub fn overview(ui: &mut egui::Ui, comp: &mut crate::composition::Composition, clock: Clock) {
    let mut any = false;
    comp.visit_all(&mut |path, p| {
        let Some(m) = p.modulator.as_mut() else { return };
        any = true;
        let mut remove = false;
        ui.horizontal(|ui| {
            ui.checkbox(&mut m.enabled, "");
            // Tiny live meter.
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
            let v = if m.enabled { m.signal(clock) } else { 0.0 };
            ui.painter().rect_filled(r, 1.0, ui.visuals().extreme_bg_color);
            ui.painter().rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - v * r.height()), r.max), 1.0, ACCENT);
            ui.label(path);
            ui.label(RichText::new(format!("{} · {} · {:.0}%", m.shape.name(), source_label(m), m.depth * 100.0)).weak().small());
            remove = ui.small_button("×").on_hover_text("Remove").clicked();
        });
        if remove {
            p.modulator = None;
        }
    });
    if !any {
        ui.label(RichText::new("Nothing automated yet. Click ~ next to any parameter.").weak());
    }
}
