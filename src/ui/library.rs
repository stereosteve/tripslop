//! The ISF browser: generators and effects from the shader folders, by category. Drag one
//! onto a grid cell (generators) or a layer (effects), or double-click it.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, RichText, Sense};

use crate::isf_library::{Drag, Kind, Library, Status};
use crate::ui::widgets::ACCENT;

pub enum LibraryAction {
    /// Double-clicked: a generator goes into the selected cell, an effect onto the selected
    /// layer (or the master chain on the Composition tab).
    Use(Drag),
    ChooseFolder,
    Rescan,
}

pub struct LibraryView {
    pub search: String,
    pub kind: Kind,
    pub show_unsupported: bool,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self { search: String::new(), kind: Kind::Effect, show_unsupported: false }
    }
}

impl LibraryView {
    pub fn show(&mut self, ui: &mut egui::Ui, lib: &Library) -> Vec<LibraryAction> {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.label(RichText::new("ISF library").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⟳").on_hover_text("Rescan folders").clicked() {
                    actions.push(LibraryAction::Rescan);
                }
                if ui.small_button("Folder…").on_hover_text("Choose an ISF folder").clicked() {
                    actions.push(LibraryAction::ChooseFolder);
                }
            });
        });
        if lib.dirs.is_empty() {
            ui.label(RichText::new("No ISF folder found. Choose one, e.g. /Library/Graphics/ISF.").weak());
            return actions;
        }
        for d in &lib.dirs {
            ui.label(RichText::new(d.display().to_string()).small().weak());
        }

        let count = |k: Kind| lib.entries.iter().filter(|e| e.kind == k).count();
        let usable = |k: Kind| lib.entries.iter().filter(|e| e.kind == k && e.status == Status::Ok).count();
        ui.horizontal(|ui| {
            for (k, label) in [(Kind::Generator, "Generators"), (Kind::Effect, "Effects")] {
                ui.selectable_value(&mut self.kind, k, format!("{label} {}", usable(k)));
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("search").desired_width(ui.available_width() - 24.0));
            if !self.search.is_empty() && ui.small_button("×").clicked() {
                self.search.clear();
            }
        });
        let hidden = count(self.kind) - usable(self.kind);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_unsupported, RichText::new(format!("show unsupported ({hidden})")).small());
            if lib.scanning {
                ui.spinner();
                ui.label(RichText::new("checking…").small().weak());
            }
        });
        ui.separator();

        let needle = self.search.to_lowercase();
        let mut by_cat: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, e) in lib.entries.iter().enumerate() {
            let visible = e.kind == self.kind
                && (self.show_unsupported || !matches!(e.status, Status::Unsupported(_)))
                && (needle.is_empty()
                    || e.name.to_lowercase().contains(&needle)
                    || e.category.to_lowercase().contains(&needle)
                    || e.description.to_lowercase().contains(&needle));
            if visible {
                by_cat.entry(&e.category).or_default().push(i);
            }
        }
        let searching = !needle.is_empty();
        egui::ScrollArea::vertical().id_salt("library scroll").auto_shrink([false, false]).show(ui, |ui| {
            if by_cat.is_empty() && !lib.scanning {
                ui.label(RichText::new("Nothing matches.").weak());
            }
            for (cat, items) in by_cat {
                let header = egui::CollapsingHeader::new(RichText::new(format!("{cat}  ({})", items.len())).strong())
                    .id_salt(("isf cat", self.kind as u8, cat));
                let header = if searching { header.open(Some(true)) } else { header };
                header.show(ui, |ui| {
                    for i in items {
                        self.item(ui, lib, i, &mut actions);
                    }
                });
            }
        });
        ui.separator();
        let hint = match self.kind {
            Kind::Generator => "Drag onto a cell, or double-click to load into the selected cell.",
            _ => "Drag onto a layer, or double-click to add to the selected layer (master on the Composition tab).",
        };
        ui.label(RichText::new(hint).small().weak());

        // The dragged item follows the pointer.
        if let Some(d) = egui::DragAndDrop::payload::<Drag>(ui.ctx())
            && let Some(pos) = ui.ctx().pointer_interact_pos()
        {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("isf drag")));
            let text = painter.layout_no_wrap(d.name.clone(), egui::FontId::proportional(13.0), Color32::BLACK);
            let rect = egui::Rect::from_min_size(pos + egui::vec2(12.0, 4.0), text.size() + egui::vec2(12.0, 6.0));
            painter.rect_filled(rect, 4.0, ACCENT);
            painter.galley(rect.min + egui::vec2(6.0, 3.0), text, Color32::BLACK);
        }
        actions
    }

    fn item(&self, ui: &mut egui::Ui, lib: &Library, i: usize, actions: &mut Vec<LibraryAction>) {
        let e = &lib.entries[i];
        let ok = e.status == Status::Ok;
        let text = match &e.status {
            Status::Ok => RichText::new(&e.name),
            Status::Checking => RichText::new(&e.name).weak(),
            Status::Unsupported(_) => RichText::new(&e.name).weak().strikethrough(),
        };
        let sense = if ok { Sense::click_and_drag() } else { Sense::hover() };
        let resp = ui.add(egui::Button::selectable(false, text).sense(sense));
        let resp = resp.on_hover_ui(|ui| {
            ui.set_max_width(320.0);
            ui.label(RichText::new(&e.name).strong());
            if !e.description.is_empty() {
                ui.label(&e.description);
            }
            if !e.credit.is_empty() {
                ui.label(RichText::new(format!("by {}", e.credit)).small().weak());
            }
            ui.label(RichText::new(e.path.display().to_string()).small().weak());
            if let Status::Unsupported(why) = &e.status {
                ui.colored_label(Color32::from_rgb(255, 120, 100), why);
            }
        });
        if !ok {
            return;
        }
        let drag = Drag { path: e.path.clone(), name: e.name.clone(), kind: e.kind };
        if resp.double_clicked() {
            actions.push(LibraryAction::Use(drag.clone()));
        }
        resp.dnd_set_drag_payload(drag);
    }
}
