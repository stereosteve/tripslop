//! "Plum Console": the colors, fonts and egui style for the whole app.
//!
//! Grounds are tinted from the logo's plum. Each signal color has one meaning: lime is live
//! (playing, held, selected), pink is automation, amber is queued, red is recording or an
//! error, cyan is audio.

use std::collections::BTreeMap;
use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, vec2};

// Grounds, darkest first.
pub const GROUND: Color32 = Color32::from_rgb(0x0E, 0x09, 0x12);
pub const PANEL: Color32 = Color32::from_rgb(0x16, 0x0F, 0x1C);
pub const SUNKEN: Color32 = Color32::from_rgb(0x13, 0x0D, 0x18);
pub const RAISED: Color32 = Color32::from_rgb(0x1F, 0x16, 0x27);
pub const RAISED_HI: Color32 = Color32::from_rgb(0x25, 0x1B, 0x2E);
/// Background of the selected layer's header.
pub const SELECTED: Color32 = Color32::from_rgb(0x21, 0x1A, 0x2C);
pub const CONTROL: Color32 = Color32::from_rgb(0x2A, 0x1F, 0x34);
pub const CONTROL_HI: Color32 = Color32::from_rgb(0x34, 0x27, 0x40);
pub const LINE: Color32 = Color32::from_rgb(0x34, 0x28, 0x3F);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x2C, 0x21, 0x35);
pub const LINE_HI: Color32 = Color32::from_rgb(0x46, 0x38, 0x4F);

// Text.
pub const TEXT: Color32 = Color32::from_rgb(0xE9, 0xDF, 0xCC);
pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xF4, 0xEC, 0xDC);
pub const MUTED: Color32 = Color32::from_rgb(0xA7, 0x9C, 0xB0);
pub const FAINT: Color32 = Color32::from_rgb(0x7D, 0x72, 0x88);
/// Text on a lit (lime, amber, layer-colored) fill.
pub const ON_LIT: Color32 = Color32::from_rgb(0x12, 0x0A, 0x17);

// Signals.
pub const LIVE: Color32 = Color32::from_rgb(0xD9, 0xFF, 0x58);
pub const MOD: Color32 = Color32::from_rgb(0xFF, 0x78, 0xDC);
pub const QUEUED: Color32 = Color32::from_rgb(0xFF, 0xB5, 0x47);
pub const RECORD: Color32 = Color32::from_rgb(0xFF, 0x5A, 0x5F);
pub const AUDIO: Color32 = Color32::from_rgb(0x5E, 0xE6, 0xFF);

/// Layer colors (`Layer::color` indexes them); a layer keeps its color everywhere in the UI.
pub const LAYER_COLORS: [Color32; crate::composition::LAYER_COLORS] = [
    Color32::from_rgb(0xFF, 0x9E, 0x5E),
    Color32::from_rgb(0x5E, 0xE6, 0xFF),
    Color32::from_rgb(0xB9, 0x8C, 0xFF),
    Color32::from_rgb(0x6C, 0xF2, 0xB0),
    Color32::from_rgb(0x7F, 0xA8, 0xFF),
    Color32::from_rgb(0xE8, 0xC9, 0x9B),
    Color32::from_rgb(0xF2, 0x8D, 0xBE),
    Color32::from_rgb(0xA8, 0xB4, 0xC8),
];

/// Crossfader sides.
pub const SIDE_A: Color32 = LAYER_COLORS[2];
pub const SIDE_B: Color32 = LAYER_COLORS[0];

pub fn layer_color(i: usize) -> Color32 {
    LAYER_COLORS[i % LAYER_COLORS.len()]
}

const SEMIBOLD: &str = "semibold";
const BOLD: &str = "bold";

/// Barlow Semi Condensed SemiBold: names and headings.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Barlow Semi Condensed Bold: titles, key letters on pads.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Small uppercase section label ("OUTPUT", "PUNCH-IN").
pub fn caption(text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase()).font(semibold(11.0)).color(FAINT).extra_letter_spacing(0.8)
}

fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        defs.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add("barlow", include_bytes!("../../assets/fonts/BarlowSemiCondensed-Regular.ttf"));
    add("barlow-semibold", include_bytes!("../../assets/fonts/BarlowSemiCondensed-SemiBold.ttf"));
    add("barlow-bold", include_bytes!("../../assets/fonts/BarlowSemiCondensed-Bold.ttf"));
    add("jetbrains", include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"));
    // egui's own fonts stay as fallbacks: they have the symbols (▶ ⏺ …) and emoji.
    let fallbacks = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mono_fallbacks = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let with = |first: &str| [vec![first.to_string()], fallbacks.clone()].concat();
    defs.families.insert(FontFamily::Proportional, with("barlow"));
    defs.families.insert(FontFamily::Name(SEMIBOLD.into()), with("barlow-semibold"));
    defs.families.insert(FontFamily::Name(BOLD.into()), with("barlow-bold"));
    defs.families.insert(FontFamily::Monospace, [vec!["jetbrains".to_string()], mono_fallbacks].concat());
    defs
}

fn widget(fill: Color32, stroke: Color32, text: Color32) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, stroke),
        corner_radius: CornerRadius::same(4),
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// Install the fonts and the dark style. Call once at startup.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = BTreeMap::from([
            (TextStyle::Small, body(11.5)),
            (TextStyle::Body, body(14.0)),
            (TextStyle::Button, body(14.0)),
            (TextStyle::Monospace, mono(12.5)),
            (TextStyle::Heading, semibold(20.0)),
        ]);

        let s = &mut style.spacing;
        s.item_spacing = vec2(6.0, 5.0);
        s.button_padding = vec2(8.0, 3.0);
        s.interact_size = vec2(36.0, 22.0);
        s.window_margin = Margin::same(10);
        s.menu_margin = Margin::same(6);
        s.combo_height = 320.0;
        s.slider_rail_height = 4.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.weak_text_color = Some(MUTED);
        v.hyperlink_color = LIVE;
        v.faint_bg_color = RAISED;
        v.extreme_bg_color = GROUND;
        v.code_bg_color = SUNKEN;
        v.warn_fg_color = QUEUED;
        v.error_fg_color = RECORD;
        v.window_fill = RAISED;
        v.window_stroke = Stroke::new(1.0, LINE_HI);
        v.window_corner_radius = CornerRadius::same(8);
        v.window_shadow = Shadow { offset: [0, 14], blur: 40, spread: 0, color: Color32::from_black_alpha(140) };
        v.popup_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(130) };
        v.menu_corner_radius = CornerRadius::same(6);
        v.panel_fill = PANEL;
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.8 };
        v.selection.bg_fill = Color32::from_rgb(0x4A, 0x55, 0x22);
        v.selection.stroke = Stroke::new(1.0, LIVE);
        v.text_cursor.stroke = Stroke::new(2.0, LIVE);
        v.collapsing_header_frame = false;
        v.indent_has_left_vline = false;

        let w = &mut v.widgets;
        w.noninteractive = widget(PANEL, LINE_SOFT, TEXT);
        w.inactive = widget(CONTROL, LINE, TEXT);
        w.hovered = widget(CONTROL_HI, LINE_HI, TEXT_STRONG);
        w.active = widget(CONTROL_HI, LIVE, TEXT_STRONG);
        w.open = widget(CONTROL_HI, LINE_HI, TEXT_STRONG);
        w.inactive.weak_bg_fill = RAISED_HI;
        // Controls at rest and under the pointer are told apart by fill alone, with no outline.
        // Besides the look, egui pads a selectable at rest (drawn frameless) as if it still had
        // its at-rest outline, so with one it grew a pixel each side on hover and nudged its
        // neighbours. Pressed keeps the LIVE outline.
        for v in [&mut w.inactive, &mut w.hovered, &mut w.open] {
            v.bg_stroke = Stroke::NONE;
        }
        // Slider rails: the "trailing fill" uses selection.bg_fill; the rail uses inactive.bg_fill.
    });
}
