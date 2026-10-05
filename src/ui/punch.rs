//! The punch-in pads: 4 × 4, hold to engage, Shift-click to latch.

use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, vec2};

use crate::punch::{PUNCHES, Punch};
use crate::ui::theme;
use crate::ui::widgets::ACCENT;

/// The pad grid, filling the available width; `height` is for all four rows. `beat` drives
/// the hold-time bars.
pub fn pads(ui: &mut egui::Ui, punch: &mut Punch, beat: f64, height: f32) {
    ui.horizontal(|ui| {
        ui.label(theme::caption("Punch-in"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if punch.pads.iter().any(|p| p.latched) && ui.small_button("Unlatch all").clicked() {
                for p in &mut punch.pads {
                    p.latched = false;
                }
            }
            ui.label(egui::RichText::new("hold · shift = latch").small().color(theme::FAINT))
                .on_hover_text("Hold a key or pad for a momentary effect. Shift+key / Shift+click latches it. Right-click a pad to learn MIDI.");
        });
    });
    let gap = 6.0;
    let w = ((ui.available_width() - gap * 3.0) / 4.0).max(30.0);
    let h = ((height - gap * 3.0) / 4.0).clamp(40.0, 72.0);
    for row in 0..4 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for col in 0..4 {
                pad(ui, punch, row * 4 + col, beat, vec2(w, h));
            }
        });
        ui.add_space(gap - ui.spacing().item_spacing.y);
    }
}

fn pad(ui: &mut egui::Ui, punch: &mut Punch, i: usize, beat: f64, size: egui::Vec2) {
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

    let painter = ui.painter_at(rect.expand(8.0));
    let lit = pad.amount;
    let base = if pad.latched { Color32::from_rgb(0x26, 0x2E, 0x14) } else if resp.hovered() { theme::RAISED_HI } else { theme::RAISED };
    let fill = mix(base, theme::LIVE, lit);
    if lit > 0.3 {
        // Glow around a lit pad.
        painter.rect_filled(rect.expand(3.0), 8.0, theme::LIVE.gamma_multiply(0.12 * lit));
    }
    painter.rect_filled(rect, 6.0, fill);
    let stroke = if learning {
        Stroke::new(2.0, ACCENT)
    } else if pad.latched || lit > 0.0 {
        Stroke::new(1.0, theme::LIVE)
    } else {
        Stroke::new(1.0, theme::LINE)
    };
    painter.rect_stroke(rect, 6.0, stroke, StrokeKind::Inside);

    let text = if lit > 0.5 {
        theme::ON_LIT
    } else if pad.latched {
        theme::LIVE
    } else {
        theme::TEXT
    };
    painter.text(rect.left_top() + vec2(8.0, 6.0), egui::Align2::LEFT_TOP, format!("{:?}", def.key), theme::mono(11.0), text.gamma_multiply(0.8));
    if pad.latched {
        painter.text(rect.right_top() + vec2(-7.0, 7.0), egui::Align2::RIGHT_TOP, "LATCH", theme::bold(9.5), text);
    }
    if crate::ui::midi::mapping(ui, key).is_some() {
        painter.text(rect.right_bottom() + vec2(-7.0, -6.0), egui::Align2::RIGHT_BOTTOM, "M", theme::bold(9.5), text.gamma_multiply(0.7));
    }
    let name_font = theme::semibold(if size.y > 60.0 { 14.0 } else { 12.5 });
    let galley = painter.layout(def.name.to_string(), name_font, text, size.x - 14.0);
    painter.galley(rect.left_bottom() + vec2(8.0, -6.0 - galley.size().y), galley, text);

    // How long it's been held (builds intensify over two bars).
    if pad.active() {
        let held = ((beat - pad.start_beat) / 8.0).clamp(0.0, 1.0) as f32;
        let bar = Rect::from_min_size(rect.left_top() + vec2(8.0, 22.0), vec2((size.x - 16.0) * held, 3.0));
        painter.rect_filled(bar, 1.5, if lit > 0.5 { theme::ON_LIT.gamma_multiply(0.6) } else { theme::LIVE });
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
