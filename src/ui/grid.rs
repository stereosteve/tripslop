//! The clip grid: layers are rows (top layer at the top), columns are scenes.

use std::collections::HashMap;

use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::clip::{Media, PATTERNS};
use crate::composition::{Composition, Launch, Side};
use crate::isf_library::{Drag, Kind};
use crate::ui::widgets::ACCENT;

pub const CELL: egui::Vec2 = egui::Vec2::new(112.0, 82.0);
const HEADER_W: f32 = 236.0;
const AMBER: Color32 = Color32::from_rgb(255, 190, 40);

/// Things the grid asks the app to do (they need dialogs, devices or app state).
pub enum GridAction {
    Launch(Launch),
    Select { layer: usize, col: Option<usize> },
    LoadFile { layer: usize, col: usize },
    Camera { layer: usize, col: usize, index: u32 },
    Generator { layer: usize, col: usize, pattern: usize },
    /// New shader clip from `shader::TEMPLATES[template]`.
    Shader { layer: usize, col: usize, template: usize },
    /// Something dropped from the ISF browser: a generator onto a cell (`col: None` = the
    /// layer's first free cell), or an effect onto a layer.
    Library { layer: usize, col: Option<usize>, drag: Drag },
    Remove { layer: usize, col: usize },
    Clear(usize),
    AddLayer,
    AddColumn,
    RemoveLayer(usize),
}

pub struct GridView {
    pub selected_layer: usize,
    pub selected_clip: Option<(usize, usize)>,
    /// Cell rects from this frame, for drag-and-drop targeting.
    pub cells: Vec<((usize, usize), Rect)>,
}

impl GridView {
    /// `thumbs`: rendered preview icons for generator and shader clips, by clip id.
    pub fn show(&mut self, ui: &mut egui::Ui, comp: &mut Composition, blink: bool, thumbs: &HashMap<u64, egui::TextureId>) -> Vec<GridAction> {
        let mut actions = Vec::new();
        self.cells.clear();
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);

        // Column (scene) headers.
        ui.horizontal(|ui| {
            ui.add_sized(vec2(HEADER_W, 22.0), egui::Label::new(RichText::new("Scenes").weak()));
            for col in 0..comp.columns {
                let pending = comp.is_pending(Launch::Column(col));
                let active = comp.active_column == Some(col);
                let mut b = egui::Button::new(RichText::new(format!("▶ {}", col + 1)).strong()).min_size(vec2(CELL.x, 22.0));
                if pending && blink {
                    b = b.fill(AMBER);
                } else if active {
                    b = b.fill(ACCENT.gamma_multiply(0.6));
                }
                let hint = if col < 9 { format!("Launch scene {} (key {})", col + 1, col + 1) } else { format!("Launch scene {}", col + 1) };
                let key = crate::ui::midi::LearnKey::Scene(col);
                if crate::ui::midi::is_learning(ui, key) {
                    b = b.stroke(egui::Stroke::new(2.0, ACCENT));
                }
                let r = ui.add(b).on_hover_text(hint);
                r.context_menu(|ui| crate::ui::midi::menu(ui, key));
                if r.clicked() {
                    actions.push(GridAction::Launch(Launch::Column(col)));
                }
            }
            if ui.add(egui::Button::new("+").min_size(vec2(26.0, 22.0))).on_hover_text("Add column").clicked() {
                actions.push(GridAction::AddColumn);
            }
        });

        for li in (0..comp.layers.len()).rev() {
            ui.horizontal(|ui| {
                self.layer_header(ui, comp, li, &mut actions);
                for col in 0..comp.columns {
                    self.cell(ui, comp, li, col, blink, thumbs, &mut actions);
                }
            });
        }
        if ui.button("+ Add layer").clicked() {
            actions.push(GridAction::AddLayer);
        }
        actions
    }

    fn layer_header(&mut self, ui: &mut egui::Ui, comp: &mut Composition, li: usize, actions: &mut Vec<GridAction>) {
        let selected = self.selected_layer == li;
        let audible = comp.layer_audible(li);
        let frame = egui::Frame::group(ui.style())
            .inner_margin(4.0)
            .fill(if selected { ui.visuals().selection.bg_fill.gamma_multiply(0.35) } else { ui.visuals().faint_bg_color });
        let header = frame.show(ui, |ui| {
            ui.set_width(HEADER_W - 12.0);
            ui.set_height(CELL.y - 10.0);
            ui.vertical(|ui| {
            ui.set_width(HEADER_W - 12.0);
            ui.spacing_mut().item_spacing = vec2(3.0, 2.0);
            let l = &mut comp.layers[li];
            ui.horizontal(|ui| {
                let name = RichText::new(&l.name).strong().color(if audible { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() });
                if ui.add(egui::Label::new(name).sense(Sense::click())).clicked() {
                    actions.push(GridAction::Select { layer: li, col: None });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Clear layer (stop its clip)").clicked() {
                        actions.push(GridAction::Clear(li));
                    }
                    ui.menu_button("…", |ui| {
                        if ui.button("Remove layer").clicked() {
                            actions.push(GridAction::RemoveLayer(li));
                            ui.close();
                        }
                    });
                });
            });
            ui.horizontal(|ui| {
                ui.toggle_value(&mut l.bypass, "B").on_hover_text("Bypass (hide) layer");
                ui.toggle_value(&mut l.solo, "S").on_hover_text("Solo");
                ui.separator();
                for (side, label) in [(Side::A, "A"), (Side::B, "B")] {
                    let on = l.side == side;
                    if ui.selectable_label(on, label).on_hover_text("Crossfader side").clicked() {
                        l.side = if on { Side::Both } else { side };
                    }
                }
                ui.separator();
                ui.label(RichText::new(l.blend.name()).small().weak());
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = HEADER_W - 64.0;
                ui.add(egui::Slider::new(&mut l.opacity.value, 0.0..=1.0).show_value(false))
                    .on_hover_text("Layer opacity");
                ui.label(RichText::new(format!("{:.0}%", l.opacity.get() * 100.0)).small());
            });
            });
        });
        let resp = &header.response;
        if let Some(d) = resp.dnd_hover_payload::<Drag>() {
            drop_hint(ui, resp.rect, &d, None);
        }
        if let Some(d) = resp.dnd_release_payload::<Drag>() {
            actions.push(GridAction::Library { layer: li, col: None, drag: (*d).clone() });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn cell(
        &mut self,
        ui: &mut egui::Ui,
        comp: &mut Composition,
        li: usize,
        col: usize,
        blink: bool,
        thumbs: &HashMap<u64, egui::TextureId>,
        actions: &mut Vec<GridAction>,
    ) {
        let (rect, resp) = ui.allocate_exact_size(CELL, Sense::click());
        self.cells.push(((li, col), rect));
        let layer = &comp.layers[li];
        let playing = layer.active == Some(col);
        let pending = comp.is_pending(Launch::Clip { layer: li, col });
        let selected = self.selected_clip == Some((li, col));
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);

        let thumb_rect = Rect::from_min_size(rect.min, vec2(CELL.x, CELL.y - 18.0));
        if let Some(clip) = layer.clips[col].as_ref() {
            let procedural = matches!(clip.media, Media::Generator(_) | Media::Shader(_));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            let gpu_thumb = thumbs.get(&clip.id).filter(|_| procedural);
            if let Some(t) = gpu_thumb {
                painter.rect_filled(thumb_rect, 3.0, Color32::BLACK);
                painter.image(*t, thumb_rect, uv, Color32::WHITE);
                let tag = match &clip.media {
                    Media::Generator(_) => Some("GEN"),
                    Media::Shader(_) => Some("GLSL"),
                    _ => None,
                };
                if let Some(tag) = tag {
                    let tr = Rect::from_min_size(thumb_rect.left_top(), vec2(30.0, 12.0));
                    painter.rect_filled(tr, 2.0, Color32::from_black_alpha(160));
                    painter.text(tr.left_center() + vec2(3.0, 0.0), egui::Align2::LEFT_CENTER, tag, egui::FontId::monospace(9.0), Color32::WHITE);
                }
                if let Media::Shader(sh) = &clip.media
                    && !sh.errors.is_empty()
                {
                    painter.text(thumb_rect.center(), egui::Align2::CENTER_CENTER, "error", egui::FontId::proportional(12.0), Color32::RED);
                }
            } else {
            match (&clip.thumbnail, &clip.media) {
                (Some(t), _) => {
                    painter.image(t.id(), thumb_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                (None, Media::Generator(g)) => {
                    // Gradient swatch for generators.
                    let hue = (g.pattern.value * 0.14 + g.hue.value).fract();
                    let c1 = egui::ecolor::Hsva::new(hue, 0.7, 0.6, 1.0);
                    let c2 = egui::ecolor::Hsva::new((hue + 0.33) % 1.0, 0.7, 0.3, 1.0);
                    painter.rect_filled(thumb_rect, 3.0, Color32::from(c2));
                    painter.circle_filled(thumb_rect.center(), 14.0, Color32::from(c1));
                    painter.text(thumb_rect.left_top() + vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "GEN", egui::FontId::monospace(9.0), Color32::WHITE);
                }
                (None, Media::Shader(sh)) => {
                    painter.rect_filled(thumb_rect, 3.0, Color32::from_rgb(30, 24, 48));
                    painter.text(thumb_rect.left_top() + vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "GLSL", egui::FontId::monospace(9.0), ACCENT);
                    let mark = if !sh.errors.is_empty() { "error" } else { "{ }" };
                    painter.text(thumb_rect.center(), egui::Align2::CENTER_CENTER, mark, egui::FontId::monospace(18.0), Color32::WHITE);
                }
                (None, Media::Camera { index, .. }) => {
                    painter.rect_filled(thumb_rect, 3.0, Color32::from_gray(40));
                    painter.text(thumb_rect.center(), egui::Align2::CENTER_CENTER, format!("CAM {index}"), egui::FontId::monospace(13.0), Color32::WHITE);
                }
                _ => {}
            }
            }
            if let Media::Video(v) = &clip.media {
                let p = v.progress();
                if p < 1.0 {
                    let r = Rect::from_min_size(thumb_rect.left_bottom() - vec2(0.0, 4.0), vec2(CELL.x * p, 4.0));
                    painter.rect_filled(r, 0.0, AMBER);
                    painter.text(thumb_rect.center(), egui::Align2::CENTER_CENTER, "importing…", egui::FontId::proportional(11.0), Color32::WHITE);
                }
            }
            if clip.error().is_some() {
                painter.text(thumb_rect.center(), egui::Align2::CENTER_CENTER, "⚠ error", egui::FontId::proportional(12.0), Color32::RED);
            }
            if playing && clip.is_timeline() {
                let t = clip.position as f32 / clip.length().max(1) as f32;
                let r = Rect::from_min_size(pos2(rect.left(), thumb_rect.bottom() - 2.0), vec2(CELL.x * t, 2.0));
                painter.rect_filled(r, 0.0, ACCENT);
            }
            let name_rect = Rect::from_min_max(pos2(rect.left(), thumb_rect.bottom()), rect.max);
            painter.rect_filled(name_rect, 0.0, if playing { ACCENT.gamma_multiply(0.7) } else { visuals.faint_bg_color });
            painter.text(
                name_rect.left_center() + vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                truncate(&clip.name, 16),
                egui::FontId::proportional(11.5),
                if playing { Color32::BLACK } else { visuals.text_color() },
            );
        } else if resp.hovered() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "drop / right-click", egui::FontId::proportional(10.0), visuals.weak_text_color());
        }

        let stroke = if pending && blink {
            Stroke::new(2.0, AMBER)
        } else if selected {
            Stroke::new(2.0, Color32::WHITE)
        } else if playing {
            Stroke::new(2.0, ACCENT)
        } else {
            Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color)
        };
        painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);

        if let Some(d) = resp.dnd_hover_payload::<Drag>() {
            drop_hint(ui, rect, &d, Some(&comp.layers[li].name));
        }
        if let Some(d) = resp.dnd_release_payload::<Drag>() {
            actions.push(GridAction::Library { layer: li, col: Some(col), drag: (*d).clone() });
        }

        let has_clip = comp.layers[li].clips[col].is_some();
        if resp.clicked() {
            actions.push(GridAction::Select { layer: li, col: Some(col) });
            if has_clip {
                actions.push(GridAction::Launch(Launch::Clip { layer: li, col }));
            }
        }
        if resp.double_clicked() && !has_clip {
            actions.push(GridAction::LoadFile { layer: li, col });
        }
        resp.context_menu(|ui| {
            if ui.button("Load file…").clicked() {
                actions.push(GridAction::LoadFile { layer: li, col });
                ui.close();
            }
            ui.menu_button("Generator", |ui| {
                for (i, name) in PATTERNS.iter().enumerate() {
                    if ui.button(*name).clicked() {
                        actions.push(GridAction::Generator { layer: li, col, pattern: i });
                        ui.close();
                    }
                }
            });
            ui.menu_button("Shader (GLSL)", |ui| {
                for (i, (name, _, _)) in crate::shader::TEMPLATES.iter().enumerate() {
                    if ui.button(*name).clicked() {
                        actions.push(GridAction::Shader { layer: li, col, template: i });
                        ui.close();
                    }
                }
                ui.label(RichText::new("Or drop a .glsl / .frag file").small().weak());
            });
            ui.menu_button("Camera / capture", |ui| {
                for index in 0..6 {
                    if ui.button(format!("Device {index}")).clicked() {
                        actions.push(GridAction::Camera { layer: li, col, index });
                        ui.close();
                    }
                }
                ui.label(RichText::new("List devices: ffmpeg -f avfoundation -list_devices true -i \"\"").small().weak());
            });
            if has_clip {
                ui.separator();
                if ui.button("Remove clip").clicked() {
                    actions.push(GridAction::Remove { layer: li, col });
                    ui.close();
                }
            }
        });
    }
}

/// Outline + label on a drop target while an ISF shader is dragged over it.
fn drop_hint(ui: &egui::Ui, rect: Rect, d: &Drag, layer: Option<&str>) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("isf drop hint")));
    painter.rect_stroke(rect, 3.0, Stroke::new(2.5, AMBER), StrokeKind::Inside);
    let label = match (d.kind, layer) {
        (Kind::Generator, Some(_)) => "load here".to_string(),
        (Kind::Generator, None) => "load into first free cell".to_string(),
        (_, Some(name)) => format!("add to {name}"),
        (_, None) => "add effect to layer".to_string(),
    };
    let text = painter.layout_no_wrap(label, egui::FontId::proportional(11.0), Color32::BLACK);
    let r = Rect::from_min_size(rect.left_bottom() - vec2(0.0, text.size().y + 4.0), text.size() + vec2(8.0, 4.0));
    painter.rect_filled(r, 2.0, AMBER);
    painter.galley(r.min + vec2(4.0, 2.0), text, Color32::BLACK);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}
