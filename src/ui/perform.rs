//! The Perform view: what you touch during a set. Big scene buttons with a picture per
//! layer, and a channel strip per layer (the rail with the pads and crossfader is drawn by
//! the app).

use std::collections::HashMap;

use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, StrokeKind, vec2};

use crate::composition::{Composition, Launch};
use crate::ui::grid::{clip_picture, toggle};
use crate::ui::theme;

/// One button per scene; returns the scene to launch.
pub fn scenes(ui: &mut egui::Ui, comp: &Composition, blink: bool, thumbs: &HashMap<u64, egui::TextureId>) -> Option<usize> {
    let mut launch = None;
    let gap = 10.0;
    // Up to 8 across; more scroll sideways.
    let w = ((ui.available_width() - gap * 7.0) / 8.0).max(120.0);
    let h = ui.available_height();
    egui::ScrollArea::horizontal().id_salt("perform scenes").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for col in 0..comp.columns {
                let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                let resp = resp.on_hover_text(if col < 9 { format!("Launch scene {} (key {})", col + 1, col + 1) } else { format!("Launch scene {}", col + 1) });
                if resp.clicked() {
                    launch = Some(col);
                }
                let active = comp.active_column == Some(col);
                let pending = comp.is_pending(Launch::Column(col));
                let empty = comp.layers.iter().all(|l| l.clips[col].is_none());
                let painter = ui.painter_at(rect);
                let (fill, text) = if active {
                    (theme::LIVE, theme::ON_LIT)
                } else if pending {
                    (theme::CONTROL, theme::QUEUED)
                } else if empty {
                    (Color32::TRANSPARENT, theme::FAINT)
                } else if resp.hovered() {
                    (theme::RAISED_HI, theme::TEXT)
                } else {
                    (theme::RAISED, theme::TEXT)
                };
                painter.rect_filled(rect, 6.0, fill);
                let stroke = if pending && blink {
                    Stroke::new(2.0, theme::QUEUED)
                } else if active {
                    Stroke::new(1.0, theme::LIVE)
                } else {
                    Stroke::new(1.0, theme::LINE)
                };
                painter.rect_stroke(rect, 6.0, stroke, StrokeKind::Inside);

                // A picture for each layer's clip in this scene, top layer first.
                let inner = rect.shrink(10.0);
                let n = comp.layers.len().max(1);
                let tw = ((inner.width() - 3.0 * (n as f32 - 1.0)) / n as f32).max(8.0);
                let th = (tw * 9.0 / 16.0).min(inner.height() - 26.0);
                for (i, l) in comp.layers.iter().rev().enumerate() {
                    let r = Rect::from_min_size(inner.min + vec2(i as f32 * (tw + 3.0), 0.0), vec2(tw, th));
                    match &l.clips[col] {
                        Some(c) => clip_picture(&painter, r, c, thumbs, Color32::WHITE),
                        None => {
                            painter.rect_filled(r, 2.0, theme::GROUND.gamma_multiply(if active { 0.4 } else { 1.0 }));
                        }
                    }
                }
                painter.text(inner.left_bottom(), egui::Align2::LEFT_BOTTOM, comp.scene_name(col).map_or_else(|| format!("Scene {}", col + 1), str::to_string), theme::bold(16.0), text);
                if col < 9 {
                    painter.text(inner.right_bottom(), egui::Align2::RIGHT_BOTTOM, format!("{}", col + 1), theme::mono(12.0), text.gamma_multiply(0.7));
                }
            }
        });
    });
    launch
}

/// A strip per layer: color, what's playing, opacity, mute / solo.
pub fn channels(ui: &mut egui::Ui, comp: &mut Composition, thumbs: &HashMap<u64, egui::TextureId>) {
    let n = comp.layers.len();
    let cols = n.clamp(1, 4);
    let gap = 10.0;
    let w = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    let order: Vec<usize> = (0..n).rev().collect();
    for row in order.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &li in row {
                ui.allocate_ui_with_layout(vec2(w, 70.0), egui::Layout::top_down(egui::Align::Min), |ui| channel(ui, comp, li, w, thumbs));
            }
        });
        ui.add_space(gap);
    }
}

fn channel(ui: &mut egui::Ui, comp: &mut Composition, li: usize, w: f32, thumbs: &HashMap<u64, egui::TextureId>) {
    let audible = comp.layer_audible(li);
    let l = &mut comp.layers[li];
    let color = theme::layer_color(l.color);
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0, theme::LINE_SOFT))
        .corner_radius(6)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(w - 22.0);
            ui.set_height(48.0);
            ui.horizontal(|ui| {
                let (strip, _) = ui.allocate_exact_size(vec2(4.0, 48.0), Sense::hover());
                ui.painter().rect_filled(strip, 2.0, if audible { color } else { color.gamma_multiply(0.35) });
                let (pic, _) = ui.allocate_exact_size(vec2(80.0, 45.0), Sense::hover());
                ui.painter().rect_filled(pic, 3.0, theme::GROUND);
                if let Some(c) = l.active_clip() {
                    clip_picture(ui.painter(), pic, c, thumbs, Color32::WHITE);
                }
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&l.name).font(theme::semibold(15.0)).color(if audible { theme::TEXT_STRONG } else { theme::FAINT }));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            toggle(ui, &mut l.solo, "S", theme::QUEUED, "Solo");
                            toggle(ui, &mut l.bypass, "M", theme::RECORD, "Mute (bypass) this layer");
                            let playing = l.active_clip().map(|c| c.name.clone()).unwrap_or_else(|| "—".into());
                            ui.add(egui::Label::new(RichText::new(playing).small().color(theme::MUTED)).truncate());
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().slider_width = ui.available_width() - 44.0;
                        ui.visuals_mut().selection.bg_fill = color;
                        ui.add(egui::Slider::new(&mut l.opacity.value, 0.0..=1.0).show_value(false)).on_hover_text("Layer opacity");
                        ui.label(RichText::new(format!("{:.0}%", l.opacity.get() * 100.0)).font(theme::mono(11.0)).color(theme::MUTED));
                    });
                });
            });
        });
}
