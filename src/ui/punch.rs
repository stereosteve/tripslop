//! The punch-in pad panel: 2 × 8 pads, hold to engage, Shift-click to latch.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, vec2};

use crate::punch::{PUNCHES, Punch};
use crate::ui::widgets::ACCENT;

const AMBER: Color32 = Color32::from_rgb(255, 190, 40);

pub fn pads(ui: &mut egui::Ui, punch: &mut Punch) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Punch-in FX").strong());
        ui.label(RichText::new("hold keys / click · Shift = latch · right-click: MIDI").small().weak());
        if punch.pads.iter().any(|p| p.latched) && ui.small_button("Unlatch all").clicked() {
            for p in &mut punch.pads {
                p.latched = false;
            }
        }
    });
    let gap = 4.0;
    let w = ((ui.available_width() - gap * 7.0) / 8.0).max(30.0);
    let size = vec2(w, 42.0);
    for row in 0..2 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for col in 0..8 {
                let i = row * 8 + col;
                let def = &PUNCHES[i];
                let pad = &mut punch.pads[i];
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let resp = resp.on_hover_text(format!("{} ({:?}, MIDI note {})\n{}", def.name, def.key, crate::midi::PAD_BASE_NOTE as usize + i, def.hint));
                let key = crate::ui::midi::LearnKey::Pad(i);
                resp.context_menu(|ui| crate::ui::midi::menu(ui, key));
                let learning = crate::ui::midi::is_learning(ui, key);
                pad.mouse_held = resp.is_pointer_button_down_on() && !ui.input(|i| i.modifiers.shift);
                if resp.clicked() && ui.input(|i| i.modifiers.shift) {
                    pad.latched = !pad.latched;
                }
                let painter = ui.painter_at(rect);
                let base = ui.visuals().widgets.inactive.bg_fill;
                let fill = if pad.amount > 0.0 {
                    Color32::from_rgb(
                        lerp(base.r(), ACCENT.r(), pad.amount),
                        lerp(base.g(), ACCENT.g(), pad.amount),
                        lerp(base.b(), ACCENT.b(), pad.amount),
                    )
                } else {
                    base
                };
                painter.rect_filled(rect, 4.0, fill);
                if pad.latched {
                    painter.rect_stroke(rect, 4.0, Stroke::new(2.0, AMBER), StrokeKind::Inside);
                }
                if learning {
                    painter.rect_stroke(rect, 4.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
                }
                let text = if pad.amount > 0.5 { Color32::BLACK } else { ui.visuals().text_color() };
                painter.text(
                    rect.left_top() + vec2(5.0, 3.0),
                    egui::Align2::LEFT_TOP,
                    format!("{:?}", def.key),
                    egui::FontId::monospace(11.0),
                    text,
                );
                painter.text(
                    rect.left_bottom() + vec2(5.0, -4.0),
                    egui::Align2::LEFT_BOTTOM,
                    def.name,
                    egui::FontId::proportional(11.0),
                    text,
                );
            }
        });
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)) as u8
}
