//! The clip grid: layers are rows (top layer at the top), columns are scenes.

use std::collections::HashMap;

use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::clip::{Media, PATTERNS};
use crate::composition::{Composition, Launch, Side};
use crate::isf_library::{Drag, Kind};
use crate::ui::theme;
use crate::ui::widgets::ACCENT;

pub const CELL: egui::Vec2 = egui::Vec2::new(124.0, 80.0);
const HEADER_W: f32 = 204.0;
const GAP: f32 = 6.0;
/// Height of the name strip at the bottom of a cell.
const STRIP: f32 = 19.0;
const AMBER: Color32 = theme::QUEUED;

/// Things the grid asks the app to do (they need dialogs, devices or app state).
pub enum GridAction {
    Launch(Launch),
    Select { layer: usize, col: Option<usize> },
    /// Show the master chain in the device panel.
    SelectMaster,
    LoadFile { layer: usize, col: usize },
    Camera { layer: usize, col: usize, index: u32 },
    Generator { layer: usize, col: usize, pattern: usize },
    /// New shader clip from `shader::TEMPLATES[template]`.
    Shader { layer: usize, col: usize, template: usize },
    /// Something dropped from the ISF browser: a generator onto a cell (`col: None` = the
    /// layer's first free cell), or an effect onto a layer.
    Library { layer: usize, col: Option<usize>, drag: Drag },
    /// An effect dropped from the browser onto the master row.
    MasterEffect(Drag),
    Remove { layer: usize, col: usize },
    Clear(usize),
    AddLayer,
    AddColumn,
    RemoveLayer(usize),
}

pub struct GridView {
    pub selected_layer: usize,
    pub selected_clip: Option<(usize, usize)>,
    /// The master row is selected (the device panel shows the master chain).
    pub master_selected: bool,
    /// Cell rects from this frame, for drag-and-drop targeting.
    pub cells: Vec<((usize, usize), Rect)>,
}

impl GridView {
    /// `thumbs`: rendered preview icons for generator and shader clips, by clip id.
    pub fn show(&mut self, ui: &mut egui::Ui, comp: &mut Composition, blink: bool, thumbs: &HashMap<u64, egui::TextureId>) -> Vec<GridAction> {
        let mut actions = Vec::new();
        self.cells.clear();
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);

        // Column (scene) headers.
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(HEADER_W, 26.0), Sense::hover());
            ui.painter().text(r.left_center() + vec2(2.0, 0.0), egui::Align2::LEFT_CENTER, "LAYERS", theme::semibold(11.0), theme::FAINT);
            ui.painter().text(r.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, "SCENES", theme::semibold(11.0), theme::FAINT);
            for col in 0..comp.columns {
                let pending = comp.is_pending(Launch::Column(col));
                let active = comp.active_column == Some(col);
                let (fill, text, stroke) = if active {
                    (theme::LIVE, theme::ON_LIT, Stroke::new(1.0, theme::LIVE))
                } else if pending {
                    (theme::CONTROL, theme::QUEUED, Stroke::new(1.0, if blink { theme::QUEUED } else { theme::LINE }))
                } else {
                    (theme::RAISED, theme::TEXT, Stroke::new(1.0, theme::LINE_SOFT))
                };
                let label = match comp.scene_name(col) {
                    Some(n) => format!("▶ {}  {n}", col + 1),
                    None => format!("▶  {}", col + 1),
                };
                let mut b = egui::Button::new(RichText::new(label).font(theme::semibold(13.0)).color(text))
                    .truncate()
                    .min_size(vec2(CELL.x, 26.0))
                    .fill(fill)
                    .stroke(stroke);
                let key = crate::ui::midi::LearnKey::Scene(col);
                if crate::ui::midi::is_learning(ui, key) {
                    b = b.stroke(Stroke::new(2.0, ACCENT));
                }
                let hint = if col < 9 { format!("Launch scene {} (key {})", col + 1, col + 1) } else { format!("Launch scene {}", col + 1) };
                let r = ui.add_sized(vec2(CELL.x, 26.0), b).on_hover_text(hint);
                r.context_menu(|ui| crate::ui::midi::menu(ui, key));
                if r.clicked() {
                    actions.push(GridAction::Launch(Launch::Column(col)));
                }
            }
            if ui.add(egui::Button::new("+").min_size(vec2(26.0, 26.0))).on_hover_text("Add a scene").clicked() {
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
        self.master_header(ui, comp, &mut actions);
        let add = egui::Button::new(RichText::new("+ Add layer").color(theme::MUTED)).min_size(vec2(HEADER_W, 28.0)).fill(egui::Color32::TRANSPARENT);
        if ui.add(add).clicked() {
            actions.push(GridAction::AddLayer);
        }
        actions
    }

    /// The master row under the layers: click it to edit the master chain.
    fn master_header(&mut self, ui: &mut egui::Ui, comp: &mut Composition, actions: &mut Vec<GridAction>) {
        let selected = self.master_selected;
        let (rect, resp) = ui.allocate_exact_size(vec2(HEADER_W, 44.0), Sense::click());
        let resp = resp.on_hover_text("Master: click to edit the master effects and output");
        if resp.clicked() {
            actions.push(GridAction::SelectMaster);
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 5.0, if selected { theme::SELECTED } else { theme::PANEL });
        if selected {
            painter.rect_stroke(rect, 5.0, Stroke::new(1.0, theme::LINE_HI), StrokeKind::Inside);
        }
        painter.rect_filled(Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(4.0, rect.height() - 14.0)), 2.0, theme::LIVE);
        painter.text(rect.left_top() + vec2(18.0, 7.0), egui::Align2::LEFT_TOP, "Master", theme::semibold(14.5), theme::TEXT_STRONG);
        let fx = comp.effects.len();
        let summary = format!("{} fx · {:.0}%", fx, comp.master.value * 100.0);
        painter.text(rect.left_bottom() + vec2(18.0, -7.0), egui::Align2::LEFT_BOTTOM, summary, theme::mono(10.5), theme::MUTED);
        if let Some(d) = resp.dnd_hover_payload::<Drag>()
            && !matches!(d.kind, Kind::Generator | Kind::Model)
        {
            drop_hint(ui, rect, &d, None);
        }
        if let Some(d) = resp.dnd_release_payload::<Drag>()
            && !matches!(d.kind, Kind::Generator | Kind::Model)
        {
            actions.push(GridAction::MasterEffect((*d).clone()));
        }
    }

    fn layer_header(&mut self, ui: &mut egui::Ui, comp: &mut Composition, li: usize, actions: &mut Vec<GridAction>) {
        let selected = self.selected_layer == li && !self.master_selected;
        let audible = comp.layer_audible(li);
        let (rect, resp) = ui.allocate_exact_size(vec2(HEADER_W, CELL.y), Sense::click());
        let l = &mut comp.layers[li];
        let color = theme::layer_color(l.color);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 5.0, if selected { theme::SELECTED } else { theme::PANEL });
        if selected {
            painter.rect_stroke(rect, 5.0, Stroke::new(1.0, theme::LINE_HI), StrokeKind::Inside);
        }
        let strip = Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(4.0, rect.height() - 14.0));
        painter.rect_filled(strip, 2.0, if audible { color } else { color.gamma_multiply(0.35) });
        if resp.clicked() {
            actions.push(GridAction::Select { layer: li, col: None });
        }

        let inner = Rect::from_min_max(rect.min + vec2(18.0, 6.0), rect.max - vec2(8.0, 6.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        let ui = &mut child;
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        ui.horizontal(|ui| {
            let name = RichText::new(&l.name).font(theme::semibold(14.5)).color(if audible { theme::TEXT_STRONG } else { theme::FAINT });
            if ui.add(egui::Label::new(name).sense(Sense::click()).truncate()).clicked() {
                actions.push(GridAction::Select { layer: li, col: None });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.menu_button(RichText::new("…").color(theme::MUTED), |ui| {
                    if ui.button("Stop layer").clicked() {
                        actions.push(GridAction::Clear(li));
                        ui.close();
                    }
                    if ui.button("Remove layer").clicked() {
                        actions.push(GridAction::RemoveLayer(li));
                        ui.close();
                    }
                    ui.separator();
                    ui.label(theme::caption("Color"));
                    ui.horizontal(|ui| {
                        for (i, c) in theme::LAYER_COLORS.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                            ui.painter().rect_filled(r, 3.0, *c);
                            if l.color % theme::LAYER_COLORS.len() == i {
                                ui.painter().rect_stroke(r.expand(2.0), 4.0, Stroke::new(1.5, theme::TEXT_STRONG), StrokeKind::Inside);
                            }
                            if resp.clicked() {
                                l.color = i;
                            }
                        }
                    });
                });
                toggle(ui, &mut l.solo, "S", theme::QUEUED, "Solo");
                toggle(ui, &mut l.bypass, "M", theme::RECORD, "Mute (bypass) this layer");
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            egui::ComboBox::from_id_salt(("blend", l.id))
                .selected_text(RichText::new(l.blend.name()).size(12.5))
                .width(ui.available_width() - 56.0)
                .show_ui(ui, |ui| {
                    for b in crate::composition::Blend::ALL {
                        ui.selectable_value(&mut l.blend, b, b.name());
                    }
                })
                .response
                .on_hover_text("Blend mode");
            for (side, label, c) in [(Side::A, "A", theme::SIDE_A), (Side::B, "B", theme::SIDE_B)] {
                let on = l.side == side;
                let b = egui::Button::new(RichText::new(label).font(theme::bold(11.5)).color(if on { theme::ON_LIT } else { theme::FAINT }))
                    .min_size(vec2(22.0, 20.0))
                    .fill(if on { c } else { theme::GROUND });
                if ui.add(b).on_hover_text("Crossfader side").clicked() {
                    l.side = if on { Side::Both } else { side };
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = ui.available_width() - 40.0;
            ui.visuals_mut().selection.bg_fill = color;
            ui.add(egui::Slider::new(&mut l.opacity.value, 0.0..=1.0).show_value(false)).on_hover_text("Layer opacity");
            ui.label(RichText::new(format!("{:.0}%", l.opacity.get() * 100.0)).font(theme::mono(10.5)).color(theme::MUTED));
        });

        if let Some(d) = resp.dnd_hover_payload::<Drag>() {
            drop_hint(ui, rect, &d, None);
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
        let color = theme::layer_color(layer.color);
        let playing = layer.active == Some(col);
        let pending = comp.is_pending(Launch::Clip { layer: li, col });
        let selected = self.selected_clip == Some((li, col));
        let painter = ui.painter_at(rect.expand(3.0));

        if let Some(clip) = layer.clips[col].as_ref() {
            painter.rect_filled(rect, 4.0, egui::Color32::BLACK);
            let dim = if playing || pending { egui::Color32::WHITE } else { egui::Color32::from_gray(205) };
            clip_picture(&painter, rect, clip, thumbs, dim);
            let tag = match &clip.media {
                Media::Generator(_) => Some("GEN"),
                Media::Shader(_) => Some("GLSL"),
                Media::Camera { .. } => Some("CAM"),
                Media::Model(_) => Some("3D"),
                _ => None,
            };
            if let Some(tag) = tag {
                let galley = painter.layout_no_wrap(tag.into(), theme::mono(9.0), theme::TEXT);
                let tr = Rect::from_min_size(rect.left_top() + vec2(4.0, 4.0), galley.size() + vec2(6.0, 2.0));
                painter.rect_filled(tr, 2.0, egui::Color32::from_black_alpha(170));
                painter.galley(tr.min + vec2(3.0, 1.0), galley, theme::TEXT);
            }
            let shader_error = matches!(&clip.media, Media::Shader(sh) if !sh.errors.is_empty());
            if shader_error || clip.error().is_some() {
                let galley = painter.layout_no_wrap("ERROR".into(), theme::bold(9.5), theme::ON_LIT);
                let tr = Rect::from_min_size(rect.right_top() + vec2(-galley.size().x - 10.0, 4.0), galley.size() + vec2(6.0, 2.0));
                painter.rect_filled(tr, 2.0, theme::RECORD);
                painter.galley(tr.min + vec2(3.0, 1.0), galley, theme::ON_LIT);
            }
            if let Media::Video(v) = &clip.media {
                let p = v.progress();
                if p < 1.0 {
                    painter.rect_filled(rect, 4.0, egui::Color32::from_black_alpha(120));
                    painter.text(rect.center() - vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, "importing…", theme::body(12.0), theme::TEXT);
                    let r = Rect::from_min_size(rect.left_bottom() - vec2(0.0, STRIP + 3.0), vec2(rect.width() * p, 3.0));
                    painter.rect_filled(r, 0.0, theme::QUEUED);
                }
            }

            // Name strip: the layer's color while playing, amber while queued.
            let name_rect = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STRIP), rect.max);
            let (strip_fill, strip_text) = if playing {
                (color, theme::ON_LIT)
            } else if pending {
                (theme::QUEUED, theme::ON_LIT)
            } else {
                (egui::Color32::from_rgba_unmultiplied(14, 9, 18, 215), theme::TEXT)
            };
            painter.rect_filled(name_rect, egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }, strip_fill);
            painter.text(name_rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, truncate(&clip.name, 18), theme::semibold(12.0), strip_text);
            if playing {
                painter.text(name_rect.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, "▶", theme::body(9.0), strip_text);
                if clip.is_timeline() {
                    let t = clip.position as f32 / clip.length().max(1) as f32;
                    let r = Rect::from_min_size(pos2(rect.left(), rect.bottom() - 2.0), vec2(rect.width() * t, 2.0));
                    painter.rect_filled(r, 0.0, theme::TEXT_STRONG);
                }
            }
            let ring = if pending && blink {
                Some(theme::QUEUED)
            } else if playing {
                Some(color)
            } else {
                None
            };
            if let Some(c) = ring {
                painter.rect_stroke(rect, 4.0, Stroke::new(2.0, c), StrokeKind::Inside);
            }
        } else {
            // Empty slot: a dashed outline, and a hint on hover.
            let stroke = Stroke::new(1.0, if resp.hovered() { theme::LINE_HI } else { theme::LINE_SOFT });
            let r = rect.shrink(0.5);
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.extend(egui::Shape::dashed_line(&pts, stroke, 4.0, 3.0));
            if resp.hovered() {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "drop · right-click", theme::body(11.5), theme::FAINT);
            }
        }
        if selected {
            painter.rect_stroke(rect.expand(2.0), 5.0, Stroke::new(1.5, theme::TEXT_STRONG), StrokeKind::Inside);
        }

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
        if resp.double_clicked() && !has_clip && crate::dialog::AVAILABLE {
            actions.push(GridAction::LoadFile { layer: li, col });
        }
        resp.context_menu(|ui| {
            if crate::dialog::AVAILABLE && ui.button("Load file…").clicked() {
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
            // Cameras come through ffmpeg, which the browser build doesn't have.
            if !crate::WEB {
                ui.menu_button("Camera / capture", |ui| {
                    for index in 0..6 {
                        if ui.button(format!("Device {index}")).clicked() {
                            actions.push(GridAction::Camera { layer: li, col, index });
                            ui.close();
                        }
                    }
                    ui.label(RichText::new("List devices: ffmpeg -f avfoundation -list_devices true -i \"\"").small().weak());
                });
            }
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
        (Kind::Generator | Kind::Model, Some(_)) => "load here".to_string(),
        (Kind::Generator | Kind::Model, None) => "load into first free cell".to_string(),
        (_, Some(name)) => format!("add to {name}"),
        (_, None) => "add effect to layer".to_string(),
    };
    let text = painter.layout_no_wrap(label, egui::FontId::proportional(11.0), Color32::BLACK);
    let r = Rect::from_min_size(rect.left_bottom() - vec2(0.0, text.size().y + 4.0), text.size() + vec2(8.0, 4.0));
    painter.rect_filled(r, 2.0, AMBER);
    painter.galley(r.min + vec2(4.0, 2.0), text, Color32::BLACK);
}

/// A clip's picture cropped to fill `rect`: its rendered preview (generators and shaders),
/// its thumbnail, or a stand-in.
pub fn clip_picture(painter: &egui::Painter, rect: Rect, clip: &crate::clip::Clip, thumbs: &HashMap<u64, egui::TextureId>, tint: Color32) {
    let procedural = matches!(clip.media, Media::Generator(_) | Media::Shader(_) | Media::Model(_));
    if let Some(t) = thumbs.get(&clip.id).filter(|_| procedural) {
        painter.image(*t, rect, cover_uv(rect, 16.0 / 9.0), tint);
        return;
    }
    match (&clip.thumbnail, &clip.media) {
        (Some(t), _) => {
            let [w, h] = t.size();
            painter.image(t.id(), rect, cover_uv(rect, w as f32 / h.max(1) as f32), tint);
        }
        (None, Media::Generator(g)) => {
            let hue = (g.pattern.value * 0.14 + g.hue.value).fract();
            painter.rect_filled(rect, 4.0, Color32::from(egui::ecolor::Hsva::new((hue + 0.33) % 1.0, 0.7, 0.3, 1.0)));
            painter.circle_filled(rect.center(), (rect.height() * 0.2).min(14.0), Color32::from(egui::ecolor::Hsva::new(hue, 0.7, 0.6, 1.0)));
        }
        (None, Media::Shader(_)) => {
            painter.rect_filled(rect, 4.0, theme::RAISED_HI);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "{ }", theme::mono(16.0), theme::TEXT);
        }
        (None, Media::Camera { index, .. }) => {
            painter.rect_filled(rect, 4.0, theme::RAISED_HI);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, format!("CAM {index}"), theme::mono(12.0), theme::TEXT);
        }
        _ => {}
    }
}

/// A small toggle that lights up in `on_color` (M / S on the layer header).
pub fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str, on_color: Color32, hint: &str) {
    let on = *value;
    let b = egui::Button::new(RichText::new(label).font(theme::bold(11.0)).color(if on { theme::ON_LIT } else { theme::MUTED }))
        .min_size(vec2(20.0, 18.0))
        .fill(if on { on_color } else { theme::CONTROL });
    if ui.add(b).on_hover_text(hint).clicked() {
        *value = !on;
    }
}

/// UVs that crop a picture of `aspect` (w / h) to fill `rect` (like CSS `object-fit: cover`).
fn cover_uv(rect: Rect, aspect: f32) -> Rect {
    let target = rect.width() / rect.height();
    if aspect > target {
        let w = target / aspect;
        Rect::from_min_max(pos2(0.5 - w / 2.0, 0.0), pos2(0.5 + w / 2.0, 1.0))
    } else {
        let h = aspect / target;
        Rect::from_min_max(pos2(0.0, 0.5 - h / 2.0), pos2(1.0, 0.5 + h / 2.0))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}
