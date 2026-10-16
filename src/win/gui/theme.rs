//! Gold / black / white design system, dark (default) and light.

use crate::config::Theme;
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};
use std::sync::Arc;

/// Colours used by custom-painted UI (device previews, cards, icons).
#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub dark: bool,
    /// Main content background.
    pub bg: Color32,
    /// Sidebar background.
    pub side: Color32,
    /// Cards.
    pub surface: Color32,
    /// Controls, key caps.
    pub raised: Color32,
    pub raised_hover: Color32,
    /// Hairlines, separators, outlines.
    pub line: Color32,
    pub text: Color32,
    pub weak: Color32,
    /// Accent for text and strokes (readable on `bg`).
    pub gold: Color32,
    /// Accent fill for primary buttons / active state.
    pub gold_fill: Color32,
    /// Text on `gold_fill`.
    pub on_gold: Color32,
    /// Translucent accent behind mapped keys and selected rows.
    pub gold_soft: Color32,
    /// Device outlines in the previews.
    pub device: Color32,
    pub warn: Color32,
    pub err: Color32,
}

const GOLD: Color32 = Color32::from_rgb(242, 193, 78);

/// Default horizontal item spacing.
pub const ITEM_X: f32 = 8.0;

pub fn pal(theme: Theme) -> Pal {
    match theme {
        Theme::Dark => Pal {
            dark: true,
            bg: Color32::from_rgb(17, 17, 19),
            side: Color32::from_rgb(10, 10, 11),
            surface: Color32::from_rgb(24, 24, 27),
            raised: Color32::from_rgb(34, 34, 38),
            raised_hover: Color32::from_rgb(46, 46, 51),
            line: Color32::from_rgb(52, 52, 58),
            text: Color32::from_rgb(242, 242, 242),
            weak: Color32::from_rgb(150, 150, 158),
            gold: GOLD,
            gold_fill: GOLD,
            on_gold: Color32::from_rgb(17, 17, 17),
            gold_soft: Color32::from_rgba_unmultiplied(242, 193, 78, 34),
            device: Color32::from_rgb(214, 214, 214),
            warn: Color32::from_rgb(255, 176, 72),
            err: Color32::from_rgb(255, 98, 98),
        },
        Theme::Light => Pal {
            dark: false,
            bg: Color32::from_rgb(247, 246, 242),
            side: Color32::from_rgb(255, 255, 255),
            surface: Color32::from_rgb(255, 255, 255),
            raised: Color32::from_rgb(239, 237, 230),
            raised_hover: Color32::from_rgb(228, 225, 215),
            line: Color32::from_rgb(216, 212, 201),
            text: Color32::from_rgb(20, 20, 20),
            weak: Color32::from_rgb(110, 108, 102),
            gold: Color32::from_rgb(166, 120, 16),
            gold_fill: GOLD,
            on_gold: Color32::from_rgb(20, 20, 20),
            gold_soft: Color32::from_rgba_unmultiplied(242, 193, 78, 64),
            device: Color32::from_rgb(40, 40, 40),
            warn: Color32::from_rgb(196, 110, 0),
            err: Color32::from_rgb(210, 50, 50),
        },
    }
}

pub fn egui_theme(t: Theme) -> egui::Theme {
    match t {
        Theme::Dark => egui::Theme::Dark,
        Theme::Light => egui::Theme::Light,
    }
}

pub fn visuals(p: &Pal) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.extreme_bg_color = if p.dark { Color32::from_rgb(12, 12, 13) } else { Color32::from_rgb(242, 240, 234) };
    v.text_edit_bg_color = Some(if p.dark { Color32::from_rgb(12, 12, 13) } else { Color32::from_rgb(242, 240, 234) });
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.gold;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.err;
    v.selection.bg_fill = p.gold_fill;
    v.selection.stroke = Stroke::new(1.0, p.on_gold);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.slider_trailing_fill = true;
    v.striped = false;
    v.button_frame = true;
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }) };
    v.window_shadow = v.popup_shadow;

    let r = CornerRadius::same(7);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.raised_hover;
    w.hovered.weak_bg_fill = p.raised_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.gold);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.gold_fill;
    w.active.weak_bg_fill = p.gold_fill;
    w.active.bg_stroke = Stroke::new(1.0, p.gold);
    w.active.fg_stroke = Stroke::new(1.5, p.on_gold);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.gold);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = r;
    v
}

/// Text sizes and spacing shared by both themes.
pub fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Name("bold".into()))),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(ITEM_X, 7.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.combo_width = 110.0;
        s.spacing.scroll.bar_width = 6.0;
        s.spacing.scroll.floating = true;
    });
}

/// Use Segoe UI from Windows when present (loaded from disk, not embedded,
/// so the exe stays small). Falls back to egui's built-in fonts.
pub fn fonts(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    let mut load = |file: &str, name: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                defs.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoeui.ttf", "segoe");
    let semibold = load("seguisb.ttf", "segoe-sb");
    let mut prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if regular {
        prop.insert(0, "segoe".into());
    }
    let mut bold = prop.clone();
    if semibold {
        bold.insert(0, "segoe-sb".into());
    }
    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(defs);
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}
