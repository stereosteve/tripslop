//! Sliders with a pop-out automation editor.
//!
//! Every slider gets a small `~` button. Clicking it attaches a modulator (if there isn't one
//! yet) and opens an editor with shape, rate, depth, polarity and phase, plus a live plot of
//! the signal. In Envelope mode you can draw the curve in the plot.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::modulation::{BEAT_CHOICES, Clock, Modulator, Polarity, Rate, Shape};
use crate::params::{Access, PARAMS, ParamDef, Params, param_def};

pub const ACCENT: Color32 = Color32::from_rgb(255, 120, 220);

/// What a slider needs to draw itself: the editable params, the live (automated) values
/// from the last frame, and the clock for the editor's playhead.
pub struct AutoCtx<'a> {
    pub params: &'a mut Params,
    pub live: &'a [f32],
    pub clock: Clock,
}

impl AutoCtx<'_> {
    pub fn slider(&mut self, ui: &mut egui::Ui, key: &'static str) {
        let idx = PARAMS.iter().position(|d| d.key == key).unwrap_or_else(|| panic!("unknown parameter {key}"));
        let def = &PARAMS[idx];
        ui.horizontal(|ui| {
            let modulator = self.params.mods.get(key);
            let active = modulator.is_some_and(|m| m.enabled);
            let label = RichText::new("~").monospace().color(if active { Color32::BLACK } else { ui.visuals().weak_text_color() });
            let mut button = egui::Button::new(label).small().selected(active);
            if active {
                button = button.fill(ACCENT);
            }
            let btn = ui.add(button).on_hover_text(if modulator.is_some() {
                "Edit automation"
            } else {
                "Automate this slider"
            });

            let resp = match def.access {
                Access::F(f) => ui.add(
                    egui::Slider::new(f(self.params), def.min..=def.max)
                        .logarithmic(def.log)
                        .text(def.label),
                ),
                Access::U(f) => ui.add(egui::Slider::new(f(self.params), def.min as u32..=def.max as u32).text(def.label)),
            };

            let base = def.get(self.params);
            if active && let Some(m) = self.params.mods.get(key) {
                paint_live_marker(ui, &resp, def, m, base, self.live.get(idx).copied().unwrap_or(base));
            }

            egui::Popup::from_toggle_button_response(&btn)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    let base = def.get(self.params);
                    let m = self.params.mods.entry(key).or_insert_with(|| Modulator::new(key));
                    if editor(ui, def, m, base, self.clock) {
                        self.params.mods.remove(key);
                        ui.close();
                    }
                });
        });
    }
}

/// Dot at the automated value plus a band showing the sweep range, drawn over the rail.
fn paint_live_marker(ui: &egui::Ui, resp: &egui::Response, def: &ParamDef, m: &Modulator, base: f32, live: f32) {
    let rail_h = ui.spacing().interact_size.y;
    let r = rail_h / 2.5;
    let left = resp.rect.left() + r;
    let width = ui.spacing().slider_width - 2.0 * r;
    let y = resp.rect.center().y + rail_h * 0.5 - 1.0;
    let x_of = |v: f32| left + def.normalized(v) * width;
    let a = m.apply(base, def.min, def.max, 0.0);
    let b = m.apply(base, def.min, def.max, 1.0);
    let painter = ui.painter();
    painter.line_segment([pos2(x_of(a), y), pos2(x_of(b), y)], Stroke::new(2.0, ACCENT.gamma_multiply(0.5)));
    painter.circle_filled(pos2(x_of(live), y), 3.0, ACCENT);
}

fn beats_label(b: f32) -> String {
    if b < 1.0 { format!("1/{}", (1.0 / b).round()) } else { format!("{b}") }
}

/// Returns true when the user asked to remove the automation.
fn editor(ui: &mut egui::Ui, def: &ParamDef, m: &mut Modulator, base: f32, clock: Clock) -> bool {
    ui.set_width(320.0);
    let mut remove = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Automate · {}", def.label)).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            remove = ui.button("Remove").clicked();
            ui.checkbox(&mut m.enabled, "on");
        });
    });
    ui.separator();

    ui.horizontal_wrapped(|ui| {
        for s in Shape::ALL {
            ui.selectable_value(&mut m.shape, s, s.name());
        }
    });

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

    ui.add(egui::Slider::new(&mut m.depth, 0.0..=1.0).text("depth"));
    ui.horizontal(|ui| {
        for p in Polarity::ALL {
            ui.selectable_value(&mut m.polarity, p, p.name());
        }
    });
    ui.add(egui::Slider::new(&mut m.phase, 0.0..=1.0).text("phase"));
    if m.shape == Shape::Square {
        ui.add(egui::Slider::new(&mut m.width, 0.02..=0.98).text("pulse width"));
    }

    plot(ui, m, clock);

    let a = m.apply(base, def.min, def.max, 0.0);
    let b = m.apply(base, def.min, def.max, 1.0);
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

/// Human-readable name for a parameter key, including its deck.
pub fn display_name(key: &str) -> String {
    let def = param_def(key);
    match key.split_once('.') {
        Some(("a", _)) => format!("Deck A {}", def.label),
        Some(("b", _)) => format!("Deck B {}", def.label),
        _ => def.label.to_string(),
    }
}

pub fn rate_label(rate: Rate) -> String {
    match rate {
        Rate::Beats(b) => format!("{} beat{}", beats_label(b), if b == 1.0 { "" } else { "s" }),
        Rate::Hz(hz) => format!("{hz:.2} Hz"),
    }
}

/// Compact list of every automated parameter.
pub fn overview(ui: &mut egui::Ui, params: &mut Params, clock: Clock) {
    if params.mods.is_empty() {
        ui.label(RichText::new("Nothing automated yet. Click ~ next to any slider.").weak());
        return;
    }
    let mut remove = None;
    for (key, m) in params.mods.iter_mut() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut m.enabled, "");
            // Tiny live meter.
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
            let v = if m.enabled { m.signal(clock) } else { 0.0 };
            ui.painter().rect_filled(r, 1.0, ui.visuals().extreme_bg_color);
            ui.painter().rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - v * r.height()), r.max), 1.0, ACCENT);
            ui.label(display_name(key));
            ui.label(RichText::new(format!("{} · {} · {:.0}%", m.shape.name(), rate_label(m.rate), m.depth * 100.0)).weak().small());
            if ui.small_button("×").on_hover_text("Remove").clicked() {
                remove = Some(*key);
            }
        });
    }
    if let Some(k) = remove {
        params.mods.remove(k);
    }
    if ui.small_button("Clear all automation").clicked() {
        params.mods.clear();
    }
}
