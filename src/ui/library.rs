//! The library browser: generators, effects and 3D models as picture cards, filed by
//! category. Hover a card to see it move; drag it onto a grid cell (generators, models) or a
//! layer (effects), or double-click it.

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, TextStyle, TextWrapMode};

use crate::isf_library::{Drag, Entry, Kind, Library, Source, Status};
use crate::ui::theme;

pub enum LibraryAction {
    /// Double-clicked: a generator goes into the selected cell, an effect onto the selected
    /// layer (or the master chain on the Composition tab).
    Use(Drag),
    AddFolder,
    /// Pick model files to list under My Models.
    AddFiles,
    RemoveFolder(usize),
    Rescan,
}

/// Baked card pictures kept as textures; the longest unseen go first.
const KEEP_BAKED: usize = 200;

/// Narrowest a card gets before the grid drops a column.
const MIN_CARD: f32 = 116.0;
const GAP: f32 = 6.0;

pub struct LibraryView {
    pub search: String,
    pub kind: Kind,
    /// `None` = all categories.
    pub category: Option<String>,
    pub show_unsupported: bool,
    /// Cards on screen in the last frame that need the renderer to draw their picture (no
    /// baked one).
    pub visible: Vec<String>,
    /// Decoded baked pictures, and the frame each was last on screen.
    baked: HashMap<String, (egui::TextureHandle, u64)>,
    frame: u64,
    /// The card under the pointer and since when.
    pub hovered: Option<(String, Instant)>,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self {
            search: String::new(),
            kind: Kind::Generator,
            category: None,
            show_unsupported: false,
            visible: Vec::new(),
            hovered: None,
            baked: HashMap::new(),
            frame: 0,
        }
    }
}

impl LibraryView {
    /// The hovered card and how long it's been hovered (drives its animation).
    pub fn hover(&self) -> Option<(&str, f64)> {
        self.hovered.as_ref().map(|(k, t)| (k.as_str(), t.elapsed().as_secs_f64()))
    }

    pub fn show(&mut self, ui: &mut egui::Ui, lib: &Library, thumb: &dyn Fn(&str) -> Option<egui::TextureId>) -> Vec<LibraryAction> {
        let mut actions = Vec::new();
        self.visible.clear();
        self.frame += 1;
        let mut hovered_now = None;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Library").strong());
            if lib.scanning {
                ui.spinner().on_hover_text("Checking which shaders tripslop can run…");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⟳").on_hover_text("Rescan").clicked() {
                    actions.push(LibraryAction::Rescan);
                }
                if ui.small_button("Add folder…").on_hover_text("Also list the ISF shaders (.fs) and 3D models in a folder").clicked() {
                    actions.push(LibraryAction::AddFolder);
                }
                if self.kind == Kind::Model
                    && ui.small_button("Add models…").on_hover_text(format!("List model files ({})", crate::model::formats::EXTENSIONS.join(", "))).clicked()
                {
                    actions.push(LibraryAction::AddFiles);
                }
            });
        });
        for (i, d) in lib.dirs.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("×").on_hover_text("Stop listing this folder").clicked() {
                    actions.push(LibraryAction::RemoveFolder(i));
                }
                ui.label(RichText::new(d.display().to_string()).small().weak());
            });
        }

        let usable = |e: &Entry| e.status == Status::Ok;
        let listed = |e: &Entry| self.show_unsupported || !matches!(e.status, Status::Unsupported(_));
        ui.horizontal(|ui| {
            for (k, label) in [(Kind::Generator, "Generators"), (Kind::Effect, "Effects"), (Kind::Model, "Models")] {
                let n = lib.entries.iter().filter(|e| e.kind == k && usable(e)).count();
                if ui.selectable_label(self.kind == k, format!("{label} {n}")).clicked() && self.kind != k {
                    self.kind = k;
                    self.category = None;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("search").desired_width(ui.available_width() - 24.0));
            if !self.search.is_empty() && ui.small_button("×").clicked() {
                self.search.clear();
            }
        });

        // Category chips, with how many of each match the search.
        let needle = self.search.to_lowercase();
        let matches = |e: &Entry| {
            needle.is_empty()
                || e.name.to_lowercase().contains(&needle)
                || e.category.to_lowercase().contains(&needle)
                || e.description.to_lowercase().contains(&needle)
                || e.tags.iter().any(|t| t.to_lowercase().contains(&needle))
        };
        let shown: Vec<&Entry> = lib.entries.iter().filter(|e| e.kind == self.kind && listed(e) && matches(e)).collect();
        let cats: Vec<(&str, usize)> = lib
            .categories
            .iter()
            .map(|c| (c.as_str(), shown.iter().filter(|e| &e.category == c).count()))
            .filter(|(_, n)| *n > 0)
            .collect();
        if self.category.as_ref().is_some_and(|c| !cats.iter().any(|(n, _)| n == c)) {
            self.category = None;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            if chip(ui, self.category.is_none(), &format!("All {}", shown.len())).clicked() {
                self.category = None;
            }
            for (c, n) in &cats {
                let on = self.category.as_deref() == Some(*c);
                if chip(ui, on, &format!("{c} {n}")).clicked() {
                    self.category = if on { None } else { Some(c.to_string()) };
                }
            }
        });
        let hidden = lib.entries.iter().filter(|e| e.kind == self.kind && matches!(e.status, Status::Unsupported(_))).count();
        if hidden > 0 {
            ui.checkbox(&mut self.show_unsupported, RichText::new(format!("show unsupported ({hidden})")).small());
        }
        ui.separator();

        let footer = 34.0;
        egui::ScrollArea::vertical()
            .id_salt("library scroll")
            .auto_shrink([false, false])
            .max_height(ui.available_height() - footer)
            .show(ui, |ui| {
                if shown.is_empty() && !lib.scanning {
                    ui.label(RichText::new("Nothing matches.").weak());
                }
                let one = self.category.clone();
                let sections: Vec<&str> = match &one {
                    Some(c) => vec![c.as_str()],
                    None => cats.iter().map(|(c, _)| *c).collect(),
                };
                for cat in sections {
                    let items: Vec<&Entry> = shown.iter().copied().filter(|e| e.category == cat).collect();
                    if one.is_none() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{cat}  ·  {}", items.len())).strong());
                    }
                    self.grid(ui, &items, thumb, &mut actions, &mut hovered_now);
                }
            });
        ui.separator();
        let hint = match self.kind {
            Kind::Generator => "Hover to preview. Drag onto a cell, or double-click to load into the selected cell.",
            Kind::Model => "Hover to spin. Drag onto a cell (or double-click) to play it as a clip; drag onto a Shape projector or Projection mapping card to map onto it.",
            _ => "Hover to preview. Drag onto a layer, or double-click to add to the selected layer (master on the Composition tab).",
        };
        ui.label(RichText::new(hint).small().weak());

        if self.baked.len() > KEEP_BAKED {
            let mut ages: Vec<(u64, String)> = self.baked.iter().map(|(k, (_, seen))| (*seen, k.clone())).collect();
            ages.sort();
            for (_, k) in ages.into_iter().take(self.baked.len() - KEEP_BAKED) {
                self.baked.remove(&k);
            }
        }
        match (hovered_now, &self.hovered) {
            (Some(k), Some((h, _))) if *h == k => {}
            (Some(k), _) => self.hovered = Some((k, Instant::now())),
            (None, _) => self.hovered = None,
        }

        // The dragged item follows the pointer.
        if let Some(d) = egui::DragAndDrop::payload::<Drag>(ui.ctx())
            && let Some(pos) = ui.ctx().pointer_interact_pos()
        {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("isf drag")));
            let text = painter.layout_no_wrap(d.name.clone(), egui::FontId::proportional(13.0), Color32::BLACK);
            let rect = Rect::from_min_size(pos + egui::vec2(12.0, 4.0), text.size() + egui::vec2(12.0, 6.0));
            painter.rect_filled(rect, 4.0, theme::LIVE);
            painter.galley(rect.min + egui::vec2(6.0, 3.0), text, Color32::BLACK);
        }
        actions
    }

    fn grid(
        &mut self,
        ui: &mut egui::Ui,
        items: &[&Entry],
        thumb: &dyn Fn(&str) -> Option<egui::TextureId>,
        actions: &mut Vec<LibraryAction>,
        hovered: &mut Option<String>,
    ) {
        let width = ui.available_width();
        let cols = (((width + GAP) / (MIN_CARD + GAP)).floor() as usize).max(1);
        let card_w = (width - GAP * (cols - 1) as f32) / cols as f32;
        let pic_h = card_w * 9.0 / 16.0;
        let card_h = pic_h + 20.0;
        for row in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for e in row {
                    self.card(ui, e, egui::vec2(card_w, card_h), pic_h, thumb, actions, hovered);
                }
            });
            ui.add_space(GAP - ui.spacing().item_spacing.y);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn card(
        &mut self,
        ui: &mut egui::Ui,
        e: &Entry,
        size: egui::Vec2,
        pic_h: f32,
        thumb: &dyn Fn(&str) -> Option<egui::TextureId>,
        actions: &mut Vec<LibraryAction>,
        hovered: &mut Option<String>,
    ) {
        let ok = e.status == Status::Ok;
        let sense = if ok { Sense::click_and_drag() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(size, sense);
        if !ui.is_rect_visible(rect) {
            return;
        }
        if e.thumb.is_none() {
            self.visible.push(e.key.clone());
        }
        let pic = Rect::from_min_size(rect.min, egui::vec2(size.x, pic_h));
        let painter = ui.painter_at(rect);
        let radius = CornerRadius::same(4);
        painter.rect_filled(pic, radius, Color32::from_gray(24));
        // The live picture while hovered (once the renderer has one), else the baked one.
        let live = (resp.hovered() || e.thumb.is_none()).then(|| thumb(&e.key)).flatten();
        match live.or_else(|| self.baked_texture(ui.ctx(), e)) {
            Some(id) => {
                let tint = if ok { Color32::WHITE } else { Color32::from_gray(90) };
                egui::Image::new((id, pic.size())).corner_radius(radius).tint(tint).paint_at(ui, pic);
            }
            None => {
                let msg = match &e.status {
                    Status::Unsupported(_) => "unsupported",
                    _ => "…",
                };
                painter.text(pic.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(11.0), Color32::from_gray(110));
            }
        }
        if resp.hovered() && ok {
            painter.rect_stroke(pic, radius, Stroke::new(1.5, theme::LIVE), StrokeKind::Inside);
            *hovered = Some(e.key.clone());
            ui.ctx().request_repaint();
        }
        let text = RichText::new(&e.name).size(12.0);
        let text = if ok { text } else { text.weak().strikethrough() };
        let galley = egui::WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Truncate), size.x, TextStyle::Small);
        let color = if ok { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
        painter.galley(egui::pos2(rect.left(), pic.bottom() + 3.0), galley, color);

        let resp = resp.on_hover_ui(|ui| {
            ui.set_max_width(320.0);
            ui.label(RichText::new(&e.name).strong());
            let what = match e.kind {
                Kind::Generator => "generator",
                Kind::Model => "3D model",
                _ => "effect",
            };
            ui.label(RichText::new(format!("{} · {what}", e.category)).small());
            if !e.description.is_empty() {
                ui.label(&e.description);
            }
            if !e.credit.is_empty() {
                ui.label(RichText::new(format!("by {}", e.credit)).small().weak());
            }
            if matches!(e.source, Source::File(_)) {
                ui.label(RichText::new(e.location()).small().weak());
            }
            if let Status::Unsupported(why) = &e.status {
                ui.colored_label(Color32::from_rgb(255, 120, 100), why);
            }
        });
        if !ok {
            return;
        }
        let drag = Drag { key: e.key.clone(), name: e.name.clone(), kind: e.kind };
        if resp.double_clicked() {
            actions.push(LibraryAction::Use(drag.clone()));
        }
        resp.dnd_set_drag_payload(drag);
    }
}

impl LibraryView {
    /// The entry's baked picture as a texture (decoded the first time it's on screen).
    fn baked_texture(&mut self, ctx: &egui::Context, e: &Entry) -> Option<egui::TextureId> {
        let jpg = e.thumb?;
        if let Some((tex, seen)) = self.baked.get_mut(&e.key) {
            *seen = self.frame;
            return Some(tex.id());
        }
        let img = image::load_from_memory(jpg).ok()?.to_rgba8();
        let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let tex = ctx.load_texture(format!("library {}", e.key), color, egui::TextureOptions::LINEAR);
        let id = tex.id();
        self.baked.insert(e.key.clone(), (tex, self.frame));
        Some(id)
    }
}

/// A small toggle button for the category row.
fn chip(ui: &mut egui::Ui, on: bool, text: &str) -> egui::Response {
    let text = RichText::new(text).size(11.5);
    let text = if on { text.color(theme::ON_LIT) } else { text };
    let mut b = egui::Button::new(text).corner_radius(10.0).min_size(egui::vec2(0.0, 20.0));
    b = if on { b.fill(theme::LIVE) } else { b.fill(theme::RAISED_HI) };
    ui.add(b)
}
