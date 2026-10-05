//! MIDI learn in the UI: a right-click menu on sliders, pads and scene buttons, and a small
//! badge on whatever is mapped. Widgets don't own the MIDI state: the app publishes what's
//! mapped each frame ([`publish`]) and collects what was asked for ([`take_requests`]).

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, RichText};

use crate::ui::widgets::ACCENT;

/// Something that can be mapped, as the UI knows it (parameters by their seed).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LearnKey {
    Param(u64),
    Pad(usize),
    Scene(usize),
    Shift,
}

#[derive(Clone, Default)]
pub struct View {
    /// The control each mapped thing is mapped to ("CC 1 · ch 1").
    pub mapped: HashMap<LearnKey, String>,
    pub learning: Option<LearnKey>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Request {
    Learn(LearnKey),
    Forget(LearnKey),
    Cancel,
}

fn view_id() -> egui::Id {
    egui::Id::new("midi view")
}

fn requests_id() -> egui::Id {
    egui::Id::new("midi requests")
}

pub fn publish(ctx: &egui::Context, view: View) {
    ctx.data_mut(|d| d.insert_temp(view_id(), Arc::new(view)));
}

fn view(ui: &egui::Ui) -> Arc<View> {
    ui.ctx().data(|d| d.get_temp::<Arc<View>>(view_id())).unwrap_or_default()
}

pub fn request(ctx: &egui::Context, r: Request) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Request>>(requests_id()).push(r));
}

pub fn take_requests(ctx: &egui::Context) -> Vec<Request> {
    ctx.data_mut(|d| d.remove_temp::<Vec<Request>>(requests_id())).unwrap_or_default()
}

/// The control `key` is mapped to, if any ("CC 1 · ch 1").
pub fn mapping(ui: &egui::Ui, key: LearnKey) -> Option<String> {
    view(ui).mapped.get(&key).cloned()
}

pub fn is_learning(ui: &egui::Ui, key: LearnKey) -> bool {
    view(ui).learning == Some(key)
}

/// The right-click menu body: learn / cancel, and the current mapping with *Forget*.
pub fn menu(ui: &mut egui::Ui, key: LearnKey) {
    let v = view(ui);
    if v.learning == Some(key) {
        ui.label(RichText::new("Move a knob or hit a pad…").color(ACCENT));
        if ui.button("Cancel MIDI learn").clicked() {
            request(ui.ctx(), Request::Cancel);
            ui.close();
        }
    } else if ui.button("Learn MIDI").on_hover_text("Then move a knob or hit a pad on your controller").clicked() {
        request(ui.ctx(), Request::Learn(key));
        ui.close();
    }
    if let Some(control) = v.mapped.get(&key) {
        ui.separator();
        ui.label(RichText::new(format!("MIDI: {control}")).small());
        if ui.button("Forget MIDI").clicked() {
            request(ui.ctx(), Request::Forget(key));
            ui.close();
        }
    }
}
