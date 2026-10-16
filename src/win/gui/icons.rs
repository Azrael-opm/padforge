//! Tiny vector icon set, drawn with the painter (no image assets, no
//! textures, crisp at any DPI, follows the theme colours).

use super::theme::Pal;
use eframe::egui::{self, pos2, vec2, Color32, Painter, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Gamepad,
    Keyboard,
    Mouse,
    List,
    Eye,
    Sun,
    Moon,
    Gear,
    Help,
    Plus,
    Play,
    Folder,
    Trash,
    Copy,
    Target,
    File,
    Sliders,
    Layers,
    Close,
    Search,
    Refresh,
    Arrow,
}

pub fn draw(p: &Painter, rect: Rect, icon: Icon, color: Color32, bg: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height());
    let w = (s * 0.09).max(1.2);
    let st = Stroke::new(w, color);
    let at = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    match icon {
        Icon::Gamepad => {
            let body = vec![
                at(-0.30, -0.22),
                at(0.30, -0.22),
                at(0.42, -0.10),
                at(0.48, 0.18),
                at(0.40, 0.28),
                at(0.28, 0.24),
                at(0.18, 0.10),
                at(-0.18, 0.10),
                at(-0.28, 0.24),
                at(-0.40, 0.28),
                at(-0.48, 0.18),
                at(-0.42, -0.10),
            ];
            p.add(Shape::closed_line(body, st));
            p.line_segment([at(-0.30, -0.04), at(-0.14, -0.04)], st);
            p.line_segment([at(-0.22, -0.12), at(-0.22, 0.04)], st);
            p.circle_filled(at(0.18, -0.08), w * 0.9, color);
            p.circle_filled(at(0.28, 0.0), w * 0.9, color);
        }
        Icon::Keyboard => {
            p.rect_stroke(r(-0.46, -0.26, 0.46, 0.26), s * 0.06, st, StrokeKind::Middle);
            for row in 0..2 {
                for k in 0..5 {
                    p.circle_filled(at(-0.30 + k as f32 * 0.15, -0.11 + row as f32 * 0.13), w * 0.7, color);
                }
            }
            p.line_segment([at(-0.18, 0.15), at(0.18, 0.15)], st);
        }
        Icon::Mouse => {
            p.rect_stroke(r(-0.24, -0.42, 0.24, 0.42), s * 0.24, st, StrokeKind::Middle);
            p.line_segment([at(0.0, -0.42), at(0.0, -0.12)], st);
            p.line_segment([at(-0.24, -0.12), at(0.24, -0.12)], Stroke::new(w * 0.7, color));
        }
        Icon::List => {
            for i in 0..3 {
                let y = -0.24 + i as f32 * 0.24;
                p.circle_filled(at(-0.34, y), w * 0.9, color);
                p.line_segment([at(-0.18, y), at(0.40, y)], st);
            }
        }
        Icon::Eye => {
            let mut pts = Vec::new();
            for i in 0..=16 {
                let t = i as f32 / 16.0 * std::f32::consts::PI;
                pts.push(at(-0.44 * t.cos(), -0.24 * t.sin()));
            }
            for i in 0..=16 {
                let t = i as f32 / 16.0 * std::f32::consts::PI;
                pts.push(at(0.44 * t.cos(), 0.24 * t.sin()));
            }
            p.add(Shape::closed_line(pts, st));
            p.circle_stroke(c, s * 0.11, st);
        }
        Icon::Sun => {
            p.circle_stroke(c, s * 0.17, st);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * s * 0.29, c + d * s * 0.42], st);
            }
        }
        Icon::Moon => {
            p.circle_filled(c, s * 0.34, color);
            p.circle_filled(c + vec2(s * 0.16, -s * 0.12), s * 0.29, bg);
        }
        Icon::Gear => {
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * s * 0.26, c + d * s * 0.42], Stroke::new(w * 1.8, color));
            }
            p.circle_filled(c, s * 0.30, color);
            p.circle_filled(c, s * 0.13, bg);
        }
        Icon::Help => {
            p.circle_stroke(c, s * 0.40, st);
            p.text(c + vec2(0.0, s * 0.02), egui::Align2::CENTER_CENTER, "?", super::theme::bold(s * 0.55), color);
        }
        Icon::Plus => {
            p.line_segment([at(-0.32, 0.0), at(0.32, 0.0)], Stroke::new(w * 1.2, color));
            p.line_segment([at(0.0, -0.32), at(0.0, 0.32)], Stroke::new(w * 1.2, color));
        }
        Icon::Play => {
            p.add(Shape::convex_polygon(vec![at(-0.22, -0.32), at(0.34, 0.0), at(-0.22, 0.32)], color, Stroke::NONE));
        }
        Icon::Folder => {
            let pts = vec![at(-0.44, -0.28), at(-0.12, -0.28), at(-0.04, -0.18), at(0.44, -0.18), at(0.44, 0.30), at(-0.44, 0.30)];
            p.add(Shape::closed_line(pts, st));
        }
        Icon::Trash => {
            p.line_segment([at(-0.36, -0.26), at(0.36, -0.26)], st);
            p.line_segment([at(-0.10, -0.36), at(0.10, -0.36)], st);
            p.add(Shape::line(vec![at(-0.26, -0.20), at(-0.20, 0.38), at(0.20, 0.38), at(0.26, -0.20)], st));
            p.line_segment([at(-0.06, -0.10), at(-0.04, 0.26)], Stroke::new(w * 0.8, color));
            p.line_segment([at(0.06, -0.10), at(0.04, 0.26)], Stroke::new(w * 0.8, color));
        }
        Icon::Copy => {
            p.rect_stroke(r(-0.34, -0.20, 0.16, 0.38), s * 0.06, st, StrokeKind::Middle);
            p.add(Shape::line(vec![at(-0.16, -0.38), at(0.34, -0.38), at(0.34, 0.20)], st));
        }
        Icon::Target => {
            p.circle_stroke(c, s * 0.30, st);
            p.circle_filled(c, w * 1.2, color);
            for (a, b) in [((0.0, -0.44), (0.0, -0.20)), ((0.0, 0.20), (0.0, 0.44)), ((-0.44, 0.0), (-0.20, 0.0)), ((0.20, 0.0), (0.44, 0.0))] {
                p.line_segment([at(a.0, a.1), at(b.0, b.1)], st);
            }
        }
        Icon::File => {
            let pts = vec![at(-0.30, -0.42), at(0.12, -0.42), at(0.32, -0.22), at(0.32, 0.42), at(-0.30, 0.42)];
            p.add(Shape::closed_line(pts, st));
            p.add(Shape::line(vec![at(0.12, -0.42), at(0.12, -0.22), at(0.32, -0.22)], st));
        }
        Icon::Sliders => {
            for (i, k) in [(-0.26f32, 0.14f32), (0.0, -0.16), (0.26, 0.06)] {
                p.line_segment([at(-0.40, i), at(0.40, i)], st);
                p.circle_filled(at(k, i), s * 0.08, color);
            }
        }
        Icon::Layers => {
            let d = |y: f32| vec![at(0.0, y - 0.18), at(0.40, y), at(0.0, y + 0.18), at(-0.40, y)];
            p.add(Shape::closed_line(d(0.10), st));
            p.add(Shape::convex_polygon(d(-0.10), bg, st));
        }
        Icon::Close => {
            p.line_segment([at(-0.26, -0.26), at(0.26, 0.26)], Stroke::new(w * 1.1, color));
            p.line_segment([at(0.26, -0.26), at(-0.26, 0.26)], Stroke::new(w * 1.1, color));
        }
        Icon::Search => {
            p.circle_stroke(at(-0.06, -0.06), s * 0.24, st);
            p.line_segment([at(0.12, 0.12), at(0.38, 0.38)], Stroke::new(w * 1.3, color));
        }
        Icon::Refresh => {
            let mut pts = Vec::new();
            for i in 0..=20 {
                let a = 0.6 + i as f32 / 20.0 * 4.9;
                pts.push(c + vec2(a.cos(), a.sin()) * s * 0.30);
            }
            let end = *pts.last().unwrap();
            p.add(Shape::line(pts, st));
            p.add(Shape::convex_polygon(vec![end + vec2(-s * 0.04, -s * 0.16), end + vec2(s * 0.14, s * 0.02), end + vec2(-s * 0.14, s * 0.06)], color, Stroke::NONE));
        }
        Icon::Arrow => {
            p.line_segment([at(-0.36, 0.0), at(0.30, 0.0)], st);
            p.add(Shape::line(vec![at(0.10, -0.20), at(0.32, 0.0), at(0.10, 0.20)], st));
        }
    }
}

/// Square icon button. `on` = highlighted (selected) state.
pub fn button(ui: &mut Ui, pal: &Pal, icon: Icon, size: f32, on: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hovered = resp.hovered();
    let bg = if on {
        pal.gold_soft
    } else if hovered {
        pal.raised_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 7.0, bg);
    let col = if on || hovered { pal.gold } else { pal.weak };
    let base = if hovered && !on { pal.raised_hover } else { pal.bg };
    draw(ui.painter(), rect.shrink(size * 0.22), icon, col, base);
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    /// Gold filled (primary action).
    Primary,
    /// Neutral filled.
    Normal,
    /// Text only, gold.
    Link,
    /// Tab: gold underline when selected.
    Tab(bool),
    /// Segmented / chip: gold fill when selected.
    Chip(bool),
}

/// Icon + text button in one of the design styles.
pub fn text_button(ui: &mut Ui, pal: &Pal, icon: Option<Icon>, text: &str, style: Style) -> Response {
    let font = match style {
        Style::Tab(_) => super::theme::bold(14.5),
        Style::Primary => super::theme::bold(14.0),
        _ => super::theme::font(14.0),
    };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 18.0 + 6.0 } else { 0.0 };
    let pad = match style {
        Style::Link => vec2(2.0, 4.0),
        Style::Tab(_) => vec2(6.0, 6.0),
        _ => vec2(12.0, 6.0),
    };
    let size = vec2(galley.size().x + icon_w + pad.x * 2.0, (galley.size().y + pad.y * 2.0).max(30.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let p = ui.painter();
    let (bg, fg, stroke) = match style {
        Style::Primary => (if hovered { pal.gold_fill.gamma_multiply(0.88) } else { pal.gold_fill }, pal.on_gold, Stroke::NONE),
        Style::Normal => (if hovered { pal.raised_hover } else { pal.raised }, pal.text, if hovered { Stroke::new(1.0, pal.gold) } else { Stroke::NONE }),
        Style::Link => (Color32::TRANSPARENT, if hovered { pal.text } else { pal.gold }, Stroke::NONE),
        Style::Tab(on) => (Color32::TRANSPARENT, if on { pal.gold } else if hovered { pal.text } else { pal.weak }, Stroke::NONE),
        Style::Chip(on) => (
            if on {
                pal.gold_fill
            } else if hovered {
                pal.raised_hover
            } else {
                pal.raised
            },
            if on { pal.on_gold } else { pal.text },
            Stroke::NONE,
        ),
    };
    p.rect(rect, 8.0, bg, stroke, StrokeKind::Inside);
    if let Style::Tab(true) = style {
        let y = rect.bottom() - 1.5;
        p.line_segment([pos2(rect.left() + pad.x, y), pos2(rect.right() - pad.x, y)], Stroke::new(2.5, pal.gold));
    }
    let mut x = rect.left() + pad.x;
    if let Some(i) = icon {
        let ir = Rect::from_center_size(pos2(x + 9.0, rect.center().y), vec2(18.0, 18.0));
        draw(p, ir, i, fg, bg_or(bg, pal.bg));
        x += icon_w;
    }
    p.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    resp
}

fn bg_or(c: Color32, fallback: Color32) -> Color32 {
    if c.a() == 0 {
        fallback
    } else {
        c
    }
}

/// PadForge logo mark: gold rounded badge with a black d-pad and buttons.
pub fn logo(p: &Painter, rect: Rect, gold: Color32, ink: Color32) {
    p.rect_filled(rect, rect.height() * 0.26, gold);
    let c = rect.center();
    let s = rect.height();
    let arm = Rect::from_center_size(c + vec2(-s * 0.16, 0.0), vec2(s * 0.30, s * 0.10));
    p.rect_filled(arm, 1.5, ink);
    p.rect_filled(Rect::from_center_size(arm.center(), vec2(s * 0.10, s * 0.30)), 1.5, ink);
    p.circle_filled(c + vec2(s * 0.20, -s * 0.07), s * 0.065, ink);
    p.circle_filled(c + vec2(s * 0.20, s * 0.11), s * 0.065, ink);
}

