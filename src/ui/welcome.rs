//! The welcome screen: the bundled sets as cards to pick from, over whatever is playing.
//! Number keys pick a card; Esc (or a click outside) closes it.

use std::collections::HashMap;

use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, vec2};

use crate::set::Builtin;
use crate::ui::theme;

pub enum WelcomeAction {
    /// Open a bundled set (its index in the list).
    Open(usize),
    Empty,
    OpenFile,
    Close,
}

#[derive(Default)]
pub struct WelcomeView {
    /// Card pictures, decoded the first time they're shown (`None`: the set has none).
    thumbs: HashMap<&'static str, Option<TextureHandle>>,
}

const GAP: f32 = 16.0;
const TEXT_H: f32 = 122.0;

impl WelcomeView {
    /// Draw over the whole window. `playing` is the name of the set that's open.
    pub fn show(&mut self, ctx: &egui::Context, sets: &[Builtin], playing: &str) -> Option<WelcomeAction> {
        let mut action = None;
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("welcome"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                // The backdrop takes the clicks meant for the app behind it.
                let backdrop = ui.allocate_rect(screen, Sense::click());
                ui.painter().rect_filled(screen, 0.0, theme::GROUND.gamma_multiply(0.86));
                if backdrop.clicked() {
                    action = Some(WelcomeAction::Close);
                }

                let width = (screen.width() - 48.0).clamp(300.0, 1120.0);
                let cols = if width > 880.0 { 3 } else if width > 560.0 { 2 } else { 1 };
                let card_w = (width - 2.0 * 24.0 - GAP * (cols as f32 - 1.0)) / cols as f32;
                let rows = sets.len().div_ceil(cols);
                let card_h = card_w * 9.0 / 16.0 + TEXT_H;
                let panel_h = (150.0 + rows as f32 * (card_h + GAP)).min(screen.height() - 48.0);
                let panel = Rect::from_center_size(screen.center(), vec2(width, panel_h));
                let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(panel));
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::LINE_HI))
                    .corner_radius(12)
                    .shadow(egui::Shadow { offset: [0, 18], blur: 60, spread: 0, color: Color32::from_black_alpha(160) })
                    .inner_margin(24)
                    .show(&mut ui, |ui| {
                        ui.set_width(width - 48.0);
                        ui.set_min_height(panel_h - 48.0);
                        // Clicks on the panel's background mustn't count as clicking outside.
                        ui.interact(ui.max_rect(), ui.id().with("panel"), Sense::click());
                        header(ui);
                        ui.add_space(14.0);
                        let scroll_h = ui.available_height() - 44.0;
                        egui::ScrollArea::vertical().max_height(scroll_h).auto_shrink([false, true]).show(ui, |ui| {
                            egui::Grid::new("welcome cards").spacing(vec2(GAP, GAP)).show(ui, |ui| {
                                for (i, b) in sets.iter().enumerate() {
                                    let tex = self.thumb(ctx, b);
                                    if card(ui, i, b, tex, vec2(card_w, card_h), b.set.name == playing) {
                                        action = Some(WelcomeAction::Open(i));
                                    }
                                    if (i + 1) % cols == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                        });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new("Start empty").font(theme::semibold(14.0))).on_hover_text("A blank composition to fill yourself").clicked() {
                                action = Some(WelcomeAction::Empty);
                            }
                            if crate::dialog::AVAILABLE && ui.button(RichText::new("Open a set…").font(theme::semibold(14.0))).on_hover_text("A .tripset file (Cmd/Ctrl+O)").clicked() {
                                action = Some(WelcomeAction::OpenFile);
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let close = format!("Keep playing {playing}");
                                if ui.button(close).on_hover_text("Close (Esc)").clicked() {
                                    action = Some(WelcomeAction::Close);
                                }
                                ui.label(RichText::new("The Sets menu, top left, brings this back").small().color(theme::FAINT));
                            });
                        });
                    });
            });
        let keys = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8, egui::Key::Num9];
        if !ctx.egui_wants_keyboard_input() {
            for (i, k) in keys.iter().enumerate().take(sets.len()) {
                if ctx.input(|inp| inp.key_pressed(*k) && !inp.modifiers.command) {
                    action = Some(WelcomeAction::Open(i));
                }
            }
        }
        action
    }

    fn thumb(&mut self, ctx: &egui::Context, b: &Builtin) -> Option<TextureHandle> {
        self.thumbs
            .entry(b.id)
            .or_insert_with(|| {
                let img = image::load_from_memory(b.thumb?).ok()?.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                Some(ctx.load_texture(format!("set-{}", b.id), ci, egui::TextureOptions::LINEAR))
            })
            .clone()
    }
}

fn header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.add(egui::Image::new(egui::include_image!("../../logos/tripslop-wordmark-color.svg")).fit_to_exact_size(vec2(168.0, 60.0)));
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("Pick a set to play").font(theme::bold(24.0)).color(theme::TEXT_STRONG));
            ui.label(
                RichText::new("Each one is a different look built from the same parts. Number keys launch its scenes; every layer, effect and knob is yours to change, and Cmd/Ctrl+S saves what you make.")
                    .color(theme::MUTED),
            );
        });
    });
}

/// One set's card; returns whether it was clicked.
fn card(ui: &mut egui::Ui, i: usize, b: &Builtin, tex: Option<TextureHandle>, size: egui::Vec2, playing: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(rect.expand(2.0));
    painter.rect_filled(rect, 8.0, if hovered { theme::RAISED_HI } else { theme::RAISED });
    let stroke = if hovered { Stroke::new(1.5, theme::LIVE) } else { Stroke::new(1.0, theme::LINE_SOFT) };

    let pic = Rect::from_min_size(rect.min, vec2(size.x, size.x * 9.0 / 16.0));
    match tex {
        Some(t) => {
            egui::Image::new(&t).corner_radius(egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }).paint_at(ui, pic);
        }
        None => {
            painter.rect_filled(pic, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, theme::layer_color(i).gamma_multiply(0.25));
            painter.text(pic.center(), egui::Align2::CENTER_CENTER, &b.set.name, theme::bold(22.0), theme::TEXT);
        }
    }
    if playing {
        let badge = Rect::from_min_size(pic.min + vec2(10.0, 10.0), vec2(74.0, 22.0));
        painter.rect_filled(badge, 4.0, theme::LIVE);
        painter.text(badge.center(), egui::Align2::CENTER_CENTER, "▶ PLAYING", theme::semibold(12.0), theme::ON_LIT);
    }
    if i < 9 {
        let key = Rect::from_min_size(pic.right_top() + vec2(-34.0, 10.0), vec2(24.0, 24.0));
        painter.rect_filled(key, 4.0, Color32::from_black_alpha(170));
        painter.text(key.center(), egui::Align2::CENTER_CENTER, format!("{}", i + 1), theme::mono(13.0), theme::TEXT_STRONG);
    }

    let text = Rect::from_min_max(pic.left_bottom() + vec2(14.0, 10.0), rect.right_bottom() - vec2(14.0, 10.0));
    painter.text(text.left_top(), egui::Align2::LEFT_TOP, &b.set.name, theme::bold(19.0), theme::TEXT_STRONG);
    let wrap = |s: &str, font: egui::FontId, color: Color32, rows: usize| {
        let mut job = egui::text::LayoutJob::simple(s.to_string(), font, color, text.width());
        job.wrap.max_rows = rows;
        job.wrap.overflow_character = Some('…');
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let desc = wrap(&b.set.description, theme::body(13.0), theme::MUTED, 3);
    painter.galley(text.left_top() + vec2(0.0, 26.0), desc, theme::MUTED);
    let scenes: Vec<String> = b.set.scenes.iter().enumerate().map(|(i, s)| format!("{} {s}", i + 1)).collect();
    let scenes = wrap(&scenes.join("  ·  "), theme::semibold(12.0), theme::FAINT, 1);
    painter.galley(text.left_bottom() - vec2(0.0, scenes.size().y), scenes, theme::FAINT);
    painter.rect_stroke(rect, 8.0, stroke, StrokeKind::Inside);
    resp.clicked()
}
