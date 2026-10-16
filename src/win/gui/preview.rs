//! Interactive device previews (keyboard, mouse, Xbox 360 / DualShock 4),
//! drawn as vector line art with callouts. Clicking any key, button or stick
//! selects it for mapping; the previews always show the current mapping,
//! whichever way it was edited.
//!
//! Everything is painted procedurally each frame (no images, no textures),
//! and only when the window actually repaints.

use super::theme::{self, Pal};
use crate::config::{Action, Bind, ButtonMode, MacroStep, StickTarget};
use crate::keys::{self, InputId};
use crate::pad::{Control, ControllerKind, PadState, Side};
use eframe::egui::{pos2, vec2, Align2, Color32, CursorIcon, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use std::collections::HashMap;
use std::sync::OnceLock;

// ------------------------------------------------------------- model --

/// A selectable element of the controller.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PadEl {
    Btn(Control),
    Stick(Side),
    Dpad,
}

/// Anything that can be selected in a preview.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Elem {
    /// A keyboard key or mouse button / wheel direction.
    Input(InputId),
    /// Mouse movement (mouse to stick).
    MouseMove,
    Pad(PadEl),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Device {
    Gamepad,
    Keyboard,
    Mouse,
}

pub fn pad_el(c: Control) -> PadEl {
    match c {
        Control::DpadUp | Control::DpadDown | Control::DpadLeft | Control::DpadRight => PadEl::Dpad,
        Control::Ls => PadEl::Stick(Side::Left),
        Control::Rs => PadEl::Stick(Side::Right),
        c => PadEl::Btn(c),
    }
}

/// Controller elements an action drives.
pub fn action_targets(a: &Action) -> Vec<PadEl> {
    match a {
        Action::Button { button, .. } => vec![pad_el(*button)],
        Action::Trigger { trigger: Side::Left, .. } => vec![PadEl::Btn(Control::Lt)],
        Action::Trigger { trigger: Side::Right, .. } => vec![PadEl::Btn(Control::Rt)],
        Action::Stick { stick, .. } | Action::StickScale { stick, .. } => vec![PadEl::Stick(*stick)],
        Action::Macro { steps, .. } => {
            let mut v: Vec<PadEl> = Vec::new();
            for s in steps {
                if let MacroStep::Press(c) | MacroStep::Release(c) | MacroStep::Tap(c) = s {
                    let e = pad_el(*c);
                    if !v.contains(&e) {
                        v.push(e);
                    }
                }
            }
            v
        }
        Action::Layer { .. } | Action::Disable => vec![],
    }
}

pub fn bind_matches(b: &Bind, e: Elem) -> bool {
    match e {
        Elem::Input(id) => keys::parse_input(&b.input) == Some(id),
        Elem::Pad(pe) => action_targets(&b.action).contains(&pe),
        Elem::MouseMove => false,
    }
}

/// Short controller label used on key caps and callouts.
pub fn short_ctrl(c: Control, kind: ControllerKind) -> &'static str {
    use Control::*;
    let ds4 = kind == ControllerKind::Ds4;
    match c {
        Ls => if ds4 { "L3" } else { "LS click" },
        Rs => if ds4 { "R3" } else { "RS click" },
        DpadUp => "D-Up",
        DpadDown => "D-Down",
        DpadLeft => "D-Left",
        DpadRight => "D-Right",
        c => c.label(kind),
    }
}

pub fn dir_name(x: f32, y: f32) -> &'static str {
    let (h, v) = (x.abs() > 0.3, y.abs() > 0.3);
    match (h, v, x > 0.0, y > 0.0) {
        (false, true, _, true) => "Up",
        (false, true, _, false) => "Down",
        (true, false, false, _) => "Left",
        (true, false, true, _) => "Right",
        (true, true, false, true) => "Up-Left",
        (true, true, true, true) => "Up-Right",
        (true, true, false, false) => "Down-Left",
        (true, true, true, false) => "Down-Right",
        _ => "Custom",
    }
}

fn stick_name(s: Side, kind: ControllerKind) -> &'static str {
    match (s, kind) {
        (Side::Left, ControllerKind::Ds4) => "L-stick",
        (Side::Right, ControllerKind::Ds4) => "R-stick",
        (Side::Left, _) => "LS",
        (Side::Right, _) => "RS",
    }
}

/// Compact description of what an action outputs (shown on key caps).
pub fn short_action(a: &Action, kind: ControllerKind) -> String {
    match a {
        Action::Button { button, mode, .. } => {
            let base = short_ctrl(*button, kind);
            match mode {
                ButtonMode::Hold => base.to_string(),
                ButtonMode::Toggle => format!("{base} (toggle)"),
                ButtonMode::Turbo => format!("{base} (turbo)"),
            }
        }
        Action::Trigger { trigger, value } => {
            let t = short_ctrl(if *trigger == Side::Left { Control::Lt } else { Control::Rt }, kind);
            if *value >= 0.999 {
                t.to_string()
            } else {
                format!("{t} {:.0}%", value * 100.0)
            }
        }
        Action::Stick { stick, x, y } => format!("{} {}", stick_name(*stick, kind), dir_name(*x, *y)),
        Action::StickScale { stick, factor } => format!("{} x{factor:.2}", stick_name(*stick, kind)),
        Action::Layer { layer, .. } => format!("Layer {layer}"),
        Action::Macro { .. } => "Macro".into(),
        Action::Disable => "Blocked".into(),
    }
}

/// Precomputed view of a layer's mapping for drawing.
#[derive(Default)]
pub struct Summary {
    /// Input -> what it outputs.
    pub out: HashMap<InputId, String>,
    /// Controller element -> inputs that drive it (already formatted).
    pub pad: HashMap<PadEl, Vec<String>>,
    pub mouse: StickTarget,
}

impl Summary {
    pub fn build(binds: &[Bind], mouse: StickTarget, kind: ControllerKind) -> Summary {
        let mut s = Summary { mouse, ..Default::default() };
        // (order, text) per element so stick directions read "W A S D".
        let mut pad: HashMap<PadEl, Vec<(u8, String)>> = HashMap::new();
        if let Some(side) = match mouse {
            StickTarget::Left => Some(Side::Left),
            StickTarget::Right => Some(Side::Right),
            StickTarget::None => None,
        } {
            pad.entry(PadEl::Stick(side)).or_default().push((0, "Mouse".into()));
        }
        for b in binds {
            let Some(id) = keys::parse_input(&b.input) else { continue };
            let name = keys::input_name(id);
            s.out.insert(id, short_action(&b.action, kind));
            let mut add = |e: PadEl, order: u8, text: String| pad.entry(e).or_default().push((order, text));
            match &b.action {
                Action::Button { button, .. } => match button {
                    Control::DpadUp => add(PadEl::Dpad, 1, name),
                    Control::DpadLeft => add(PadEl::Dpad, 2, name),
                    Control::DpadDown => add(PadEl::Dpad, 3, name),
                    Control::DpadRight => add(PadEl::Dpad, 4, name),
                    Control::Ls => add(PadEl::Stick(Side::Left), 8, format!("click {name}")),
                    Control::Rs => add(PadEl::Stick(Side::Right), 8, format!("click {name}")),
                    c => add(PadEl::Btn(*c), 1, name),
                },
                Action::Trigger { trigger, value } => {
                    let c = if *trigger == Side::Left { Control::Lt } else { Control::Rt };
                    add(PadEl::Btn(c), 2, format!("{name} {:.0}%", value * 100.0));
                }
                Action::Stick { stick, x, y } => {
                    let order = match dir_name(*x, *y) {
                        "Up" => 1,
                        "Left" => 2,
                        "Down" => 3,
                        "Right" => 4,
                        _ => 5,
                    };
                    add(PadEl::Stick(*stick), order, name);
                }
                Action::StickScale { stick, factor } => add(PadEl::Stick(*stick), 7, format!("{name} x{factor:.1}")),
                Action::Macro { .. } => {
                    for e in action_targets(&b.action) {
                        add(e, 6, format!("{name} macro"));
                    }
                }
                Action::Layer { .. } | Action::Disable => {}
            }
        }
        for (e, mut v) in pad {
            v.sort_by_key(|(o, _)| *o);
            s.pad.insert(e, v.into_iter().map(|(_, t)| t).collect());
        }
        s
    }
}

// ---------------------------------------------------------- plumbing --

pub struct View<'a> {
    pub device: Device,
    pub kind: ControllerKind,
    pub pal: &'a Pal,
    pub sum: &'a Summary,
    pub selected: Option<Elem>,
    /// Extra highlighted elements (e.g. the row hovered in the list).
    pub highlight: &'a [Elem],
    /// Live controller output while emulating.
    pub live: Option<PadState>,
}

pub struct Out {
    pub clicked: Option<Elem>,
    pub hovered: Option<Elem>,
}

/// Design space -> screen transform (uniform scale, centred).
#[derive(Clone, Copy)]
struct Tf {
    o: Pos2,
    k: f32,
}

impl Tf {
    fn fit(rect: Rect, w: f32, h: f32) -> Tf {
        let k = (rect.width() / w).min(rect.height() / h);
        Tf { o: rect.center() - vec2(w * k / 2.0, h * k / 2.0), k }
    }
    fn p(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x * self.k, self.o.y + y * self.k)
    }
    fn r(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }
    fn l(&self, v: f32) -> f32 {
        v * self.k
    }
    /// Font size in screen px, kept readable.
    fn font(&self, design: f32, min: f32, max: f32) -> f32 {
        (design * self.k).clamp(min, max)
    }
}

#[derive(Clone, Copy)]
enum Hit {
    R(Rect),
    C(Pos2, f32),
}

impl Hit {
    fn contains(&self, p: Pos2) -> bool {
        match *self {
            Hit::R(r) => r.contains(p),
            Hit::C(c, r) => c.distance(p) <= r,
        }
    }
}

/// Visual state of one element.
#[derive(Clone, Copy, Default)]
struct St {
    sel: bool,
    hot: bool,
    pressed: bool,
}

struct Ctx<'a> {
    p: &'a Painter,
    tf: Tf,
    v: &'a View<'a>,
    hovered: Option<Elem>,
}

impl Ctx<'_> {
    fn st(&self, e: Elem) -> St {
        St { sel: self.v.selected == Some(e), hot: self.hovered == Some(e) || self.v.highlight.contains(&e), pressed: false }
    }
    /// Outline colour for an element.
    fn ink(&self, s: St) -> Color32 {
        if s.sel || s.hot || s.pressed {
            self.v.pal.gold
        } else {
            self.v.pal.device
        }
    }
    fn width(&self, s: St) -> f32 {
        let base = self.tf.l(2.0).clamp(1.2, 2.4);
        if s.sel {
            base * 1.6
        } else {
            base
        }
    }
    fn fill(&self, s: St) -> Color32 {
        if s.pressed {
            self.v.pal.gold.gamma_multiply(0.55)
        } else if s.sel || s.hot {
            self.v.pal.gold_soft
        } else {
            Color32::TRANSPARENT
        }
    }
}

pub fn show(ui: &mut Ui, size: Vec2, v: &View) -> Out {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    let ptr = resp.hover_pos();
    let (w, h) = match v.device {
        Device::Keyboard => (KB_W, KB_H),
        Device::Mouse => (820.0, 430.0),
        Device::Gamepad => (900.0, 440.0),
    };
    let tf = Tf::fit(rect.shrink(6.0), w, h);
    let hits = match v.device {
        Device::Keyboard => keyboard_hits(tf),
        Device::Mouse => mouse_hits(tf),
        Device::Gamepad => gamepad_hits(tf, v.kind),
    };
    let hovered = ptr.and_then(|p| hits.iter().find(|(_, h)| h.contains(p)).map(|(e, _)| *e));
    let cx = Ctx { p: &painter, tf, v, hovered };
    match v.device {
        Device::Keyboard => draw_keyboard(&cx),
        Device::Mouse => draw_mouse(&cx),
        Device::Gamepad => draw_gamepad(&cx),
    }
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    Out { clicked: if resp.clicked() { hovered } else { None }, hovered }
}

/// Text that shrinks (then truncates) to fit `max_w`.
pub fn fit_text(p: &Painter, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32, max_w: f32) {
    let mut f = font.clone();
    let mut g = p.layout_no_wrap(text.to_string(), f.clone(), color);
    while g.size().x > max_w && f.size > 7.5 {
        f.size -= 0.5;
        g = p.layout_no_wrap(text.to_string(), f.clone(), color);
    }
    if g.size().x > max_w {
        let mut t: String = text.to_string();
        while t.chars().count() > 2 {
            t.pop();
            g = p.layout_no_wrap(format!("{t}.."), f.clone(), color);
            if g.size().x <= max_w {
                break;
            }
        }
    }
    let r = align.anchor_size(pos, g.size());
    p.galley(r.min, g, color);
}

fn bezier(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, out: &mut Vec<Pos2>) {
    let n = 14;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        out.push(pos2(w[0] * p0.x + w[1] * p1.x + w[2] * p2.x + w[3] * p3.x, w[0] * p0.y + w[1] * p1.y + w[2] * p2.y + w[3] * p3.y));
    }
}

// ------------------------------------------------------------ callouts --

struct Callout {
    elem: Elem,
    anchor: (f32, f32),
    /// Label y (design units); x comes from the column.
    y: f32,
    left: bool,
    name: String,
}

/// Columns in design units: labels end at `l_edge` (left) or start at `r_edge`.
struct Cols {
    l_edge: f32,
    r_edge: f32,
    width: f32,
}

fn callout_hit(tf: Tf, c: &Callout, cols: &Cols) -> Hit {
    let (x0, x1) = if c.left { (cols.l_edge - cols.width, cols.l_edge + 4.0) } else { (cols.r_edge - 4.0, cols.r_edge + cols.width) };
    Hit::R(tf.r(x0, c.y - 24.0, x1, c.y + 12.0))
}

fn draw_callout(cx: &Ctx, c: &Callout, cols: &Cols, value: &str) {
    let (tf, pal) = (cx.tf, cx.v.pal);
    let s = cx.st(c.elem);
    let lit = s.sel || s.hot;
    let mapped = !value.is_empty();
    let line_col = if lit {
        pal.gold
    } else if mapped {
        pal.gold.gamma_multiply(0.55)
    } else {
        pal.weak.gamma_multiply(0.6)
    };
    let lw = if lit { 1.6 } else { 1.0 };
    let a = tf.p(c.anchor.0, c.anchor.1);
    let edge = if c.left { cols.l_edge + 8.0 } else { cols.r_edge - 8.0 };
    let mid = if c.left { (c.anchor.0 - 36.0).max(edge + 10.0) } else { (c.anchor.0 + 36.0).min(edge - 10.0) };
    cx.p.add(Shape::line(vec![tf.p(edge, c.y), tf.p(mid, c.y), a], Stroke::new(lw, line_col)));
    cx.p.circle_filled(a, tf.l(3.2).clamp(2.0, 4.0), line_col);

    let x = if c.left { cols.l_edge } else { cols.r_edge };
    let align_v = if c.left { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
    let align_n = if c.left { Align2::RIGHT_BOTTOM } else { Align2::LEFT_BOTTOM };
    let max_w = tf.l(cols.width);
    let name_col = if lit { pal.gold } else { pal.weak };
    fit_text(cx.p, tf.p(x, c.y - 8.0), align_n, &c.name, theme::font(tf.font(12.5, 10.0, 13.0)), name_col, max_w);
    let (txt, col, font) = if value.is_empty() {
        ("Not mapped", pal.weak.gamma_multiply(0.75), theme::font(tf.font(14.0, 11.0, 15.0)))
    } else {
        (value, if lit { pal.text } else { pal.gold }, theme::bold(tf.font(15.0, 11.5, 16.0)))
    };
    fit_text(cx.p, tf.p(x, c.y + 3.0), align_v, txt, font, col, max_w);
}

// ------------------------------------------------------------ keyboard --

const KB_W: f32 = 23.3;
const KB_H: f32 = 7.15;

struct Cap {
    id: InputId,
    label: &'static str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn caps() -> &'static [Cap] {
    static CAPS: OnceLock<Vec<Cap>> = OnceLock::new();
    CAPS.get_or_init(|| {
        let mut v = Vec::with_capacity(110);
        let mut k = |id: u16, label: &'static str, x: f32, y: f32, w: f32, h: f32| v.push(Cap { id, label, x: x + 0.4, y: y + 0.45, w, h });
        // Function row.
        k(0x1B, "Esc", 0.0, 0.0, 1.0, 1.0);
        let f = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"];
        for (i, n) in f.iter().enumerate() {
            let x = 2.0 + i as f32 + (i / 4) as f32 * 0.5;
            k(0x70 + i as u16, n, x, 0.0, 1.0, 1.0);
        }
        k(0x2C, "PrtSc", 15.25, 0.0, 1.0, 1.0);
        k(0x91, "Scroll", 16.25, 0.0, 1.0, 1.0);
        k(0x13, "Pause", 17.25, 0.0, 1.0, 1.0);
        // Number row.
        let y = 1.25;
        k(0xC0, "`", 0.0, y, 1.0, 1.0);
        for (i, n) in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].iter().enumerate() {
            k(if i == 9 { 0x30 } else { 0x31 + i as u16 }, n, 1.0 + i as f32, y, 1.0, 1.0);
        }
        k(0xBD, "-", 11.0, y, 1.0, 1.0);
        k(0xBB, "=", 12.0, y, 1.0, 1.0);
        k(0x08, "Backspace", 13.0, y, 2.0, 1.0);
        // QWERTY row.
        let y = 2.25;
        k(0x09, "Tab", 0.0, y, 1.5, 1.0);
        for (i, c) in "QWERTYUIOP".bytes().enumerate() {
            k(c as u16, LETTERS[(c - b'A') as usize], 1.5 + i as f32, y, 1.0, 1.0);
        }
        k(0xDB, "[", 11.5, y, 1.0, 1.0);
        k(0xDD, "]", 12.5, y, 1.0, 1.0);
        k(0xDC, "\\", 13.5, y, 1.5, 1.0);
        // Home row.
        let y = 3.25;
        k(0x14, "Caps", 0.0, y, 1.75, 1.0);
        for (i, c) in "ASDFGHJKL".bytes().enumerate() {
            k(c as u16, LETTERS[(c - b'A') as usize], 1.75 + i as f32, y, 1.0, 1.0);
        }
        k(0xBA, ";", 10.75, y, 1.0, 1.0);
        k(0xDE, "'", 11.75, y, 1.0, 1.0);
        k(0x0D, "Enter", 12.75, y, 2.25, 1.0);
        // Bottom letter row.
        let y = 4.25;
        k(0xA0, "Shift", 0.0, y, 2.25, 1.0);
        for (i, c) in "ZXCVBNM".bytes().enumerate() {
            k(c as u16, LETTERS[(c - b'A') as usize], 2.25 + i as f32, y, 1.0, 1.0);
        }
        k(0xBC, ",", 9.25, y, 1.0, 1.0);
        k(0xBE, ".", 10.25, y, 1.0, 1.0);
        k(0xBF, "/", 11.25, y, 1.0, 1.0);
        k(0xA1, "Shift", 12.25, y, 2.75, 1.0);
        // Space row.
        let y = 5.25;
        k(0xA2, "Ctrl", 0.0, y, 1.25, 1.0);
        k(0x5B, "Win", 1.25, y, 1.25, 1.0);
        k(0xA4, "Alt", 2.5, y, 1.25, 1.0);
        k(0x20, "Space", 3.75, y, 6.25, 1.0);
        k(0xA5, "Alt", 10.0, y, 1.25, 1.0);
        k(0x5C, "Win", 11.25, y, 1.25, 1.0);
        k(0x5D, "Menu", 12.5, y, 1.25, 1.0);
        k(0xA3, "Ctrl", 13.75, y, 1.25, 1.0);
        // Navigation cluster.
        k(0x2D, "Ins", 15.25, 1.25, 1.0, 1.0);
        k(0x24, "Home", 16.25, 1.25, 1.0, 1.0);
        k(0x21, "PgUp", 17.25, 1.25, 1.0, 1.0);
        k(0x2E, "Del", 15.25, 2.25, 1.0, 1.0);
        k(0x23, "End", 16.25, 2.25, 1.0, 1.0);
        k(0x22, "PgDn", 17.25, 2.25, 1.0, 1.0);
        k(0x26, "", 16.25, 4.25, 1.0, 1.0);
        k(0x25, "", 15.25, 5.25, 1.0, 1.0);
        k(0x28, "", 16.25, 5.25, 1.0, 1.0);
        k(0x27, "", 17.25, 5.25, 1.0, 1.0);
        // Numpad.
        let nx = 18.5;
        k(0x90, "Num", nx, 1.25, 1.0, 1.0);
        k(0x6F, "/", nx + 1.0, 1.25, 1.0, 1.0);
        k(0x6A, "*", nx + 2.0, 1.25, 1.0, 1.0);
        k(0x6D, "-", nx + 3.0, 1.25, 1.0, 1.0);
        for (r, row) in [["7", "8", "9"], ["4", "5", "6"], ["1", "2", "3"]].iter().enumerate() {
            for (c, n) in row.iter().enumerate() {
                let digit = n.as_bytes()[0] - b'0';
                k(0x60 + digit as u16, n, nx + c as f32, 2.25 + r as f32, 1.0, 1.0);
            }
        }
        k(0x6B, "+", nx + 3.0, 2.25, 1.0, 2.0);
        k(0x0D, "Enter", nx + 3.0, 4.25, 1.0, 2.0);
        k(0x60, "0", nx, 5.25, 2.0, 1.0);
        k(0x6E, ".", nx + 2.0, 5.25, 1.0, 1.0);
        v
    })
}

const LETTERS: [&str; 26] = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z"];

fn cap_rect(tf: Tf, c: &Cap) -> Rect {
    tf.r(c.x, c.y, c.x + c.w, c.y + c.h).shrink(tf.l(0.06))
}

fn keyboard_hits(tf: Tf) -> Vec<(Elem, Hit)> {
    caps().iter().map(|c| (Elem::Input(c.id), Hit::R(cap_rect(tf, c)))).collect()
}

fn arrow_glyph(p: &Painter, c: Pos2, size: f32, id: InputId, col: Color32) {
    let s = size * 0.5;
    let pts = match id {
        0x26 => vec![c + vec2(0.0, -s), c + vec2(s, s * 0.6), c + vec2(-s, s * 0.6)],
        0x28 => vec![c + vec2(0.0, s), c + vec2(-s, -s * 0.6), c + vec2(s, -s * 0.6)],
        0x25 => vec![c + vec2(-s, 0.0), c + vec2(s * 0.6, -s), c + vec2(s * 0.6, s)],
        _ => vec![c + vec2(s, 0.0), c + vec2(-s * 0.6, s), c + vec2(-s * 0.6, -s)],
    };
    p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
}

fn draw_keyboard(cx: &Ctx) {
    let (tf, pal, p) = (cx.tf, cx.v.pal, cx.p);
    // Keyboard body.
    p.rect(tf.r(0.05, 0.05, KB_W - 0.05, KB_H - 0.05), tf.l(0.35), Color32::TRANSPARENT, Stroke::new(1.5, pal.device.gamma_multiply(0.8)), StrokeKind::Middle);
    let u = tf.k;
    for c in caps() {
        let e = Elem::Input(c.id);
        let s = cx.st(e);
        let r = cap_rect(tf, c);
        let mapped = cx.v.sum.out.get(&c.id);
        let fill = if s.sel || s.hot {
            if mapped.is_some() {
                pal.gold.gamma_multiply(0.30)
            } else {
                pal.raised_hover
            }
        } else if mapped.is_some() {
            pal.gold_soft
        } else {
            pal.raised
        };
        let stroke = if s.sel {
            Stroke::new(2.2, pal.gold)
        } else if s.hot {
            Stroke::new(1.5, pal.gold)
        } else if mapped.is_some() {
            Stroke::new(1.0, pal.gold.gamma_multiply(0.75))
        } else {
            Stroke::new(1.0, pal.line)
        };
        p.rect(r, u * 0.12, fill, stroke, StrokeKind::Inside);
        let is_arrow = (0x25..=0x28).contains(&c.id);
        match mapped {
            Some(out) => {
                let small = theme::font((u * 0.24).clamp(8.0, 12.0));
                let tl = r.left_top() + vec2(u * 0.10, u * 0.06);
                if is_arrow {
                    arrow_glyph(p, tl + vec2(u * 0.12, u * 0.14), u * 0.2, c.id, pal.weak);
                } else {
                    fit_text(p, tl, Align2::LEFT_TOP, c.label, small, pal.weak, r.width() - u * 0.15);
                }
                fit_text(p, pos2(r.center().x, r.bottom() - u * 0.30), Align2::CENTER_CENTER, out, theme::bold((u * 0.30).clamp(8.5, 14.0)), pal.gold, r.width() - u * 0.12);
            }
            None => {
                let col = if s.sel || s.hot { pal.gold } else { pal.text };
                if is_arrow {
                    arrow_glyph(p, r.center(), u * 0.34, c.id, col);
                } else {
                    fit_text(p, r.center(), Align2::CENTER_CENTER, c.label, theme::font((u * 0.30).clamp(8.5, 15.0)), col, r.width() - u * 0.12);
                }
            }
        }
    }
}

// --------------------------------------------------------------- mouse --

const MX: f32 = 410.0;
const MY0: f32 = 28.0;
const MY1: f32 = 408.0;
const MW: f32 = 118.0;
const MSPLIT: f32 = MY0 + 150.0;

fn mouse_half(y: f32) -> f32 {
    let t = ((y - MY0) / (MY1 - MY0)).clamp(0.0, 1.0);
    let u = (2.0 * t - 1.0).abs();
    MW * (0.86 + 0.14 * t) * (1.0 - u.powf(2.6)).max(0.0).powf(1.0 / 2.6)
}

const M_COLS: Cols = Cols { l_edge: 245.0, r_edge: 575.0, width: 215.0 };

fn mouse_callouts() -> Vec<Callout> {
    let side_x = MX - mouse_half(MY0 + 220.0) - 4.0;
    let mk = |elem, anchor, y, left, name: &str| Callout { elem, anchor, y, left, name: name.into() };
    vec![
        mk(Elem::Input(keys::MOUSE_LEFT), (MX - 62.0, MY0 + 70.0), 62.0, true, "Left button (Mouse1)"),
        mk(Elem::Input(keys::WHEEL_LEFT), (MX - 30.0, MY0 + 80.0), 128.0, true, "Wheel tilt left"),
        mk(Elem::Input(keys::MOUSE_X2), (side_x - 8.0, MY0 + 198.0), 205.0, true, "Side button X2 (Mouse5)"),
        mk(Elem::Input(keys::MOUSE_X1), (side_x - 8.0, MY0 + 252.0), 268.0, true, "Side button X1 (Mouse4)"),
        mk(Elem::Input(keys::MOUSE_RIGHT), (MX + 62.0, MY0 + 70.0), 62.0, false, "Right button (Mouse2)"),
        mk(Elem::Input(keys::WHEEL_UP), (MX + 8.0, MY0 + 58.0), 112.0, false, "Wheel up"),
        mk(Elem::Input(keys::MOUSE_MIDDLE), (MX + 10.0, MY0 + 80.0), 160.0, false, "Wheel click (Mouse3)"),
        mk(Elem::Input(keys::WHEEL_DOWN), (MX + 8.0, MY0 + 102.0), 208.0, false, "Wheel down"),
        mk(Elem::Input(keys::WHEEL_RIGHT), (MX + 30.0, MY0 + 80.0), 256.0, false, "Wheel tilt right"),
        mk(Elem::MouseMove, (MX + 44.0, MY0 + 285.0), 330.0, false, "Mouse movement"),
    ]
}

fn side_button(n: u8) -> (f32, f32, f32, f32) {
    let (y0, y1) = if n == 2 { (MY0 + 175.0, MY0 + 222.0) } else { (MY0 + 228.0, MY0 + 276.0) };
    let x = MX - mouse_half((y0 + y1) / 2.0);
    (x - 14.0, y0, x + 9.0, y1)
}

fn mouse_hits(tf: Tf) -> Vec<(Elem, Hit)> {
    let mut v = vec![];
    for (n, id) in [(2u8, keys::MOUSE_X2), (1, keys::MOUSE_X1)] {
        let (x0, y0, x1, y1) = side_button(n);
        v.push((Elem::Input(id), Hit::R(tf.r(x0, y0, x1, y1))));
    }
    v.push((Elem::Input(keys::WHEEL_UP), Hit::R(tf.r(MX - 15.0, MY0 + 42.0, MX + 15.0, MY0 + 74.0))));
    v.push((Elem::Input(keys::MOUSE_MIDDLE), Hit::R(tf.r(MX - 15.0, MY0 + 74.0, MX + 15.0, MY0 + 86.0))));
    v.push((Elem::Input(keys::WHEEL_DOWN), Hit::R(tf.r(MX - 15.0, MY0 + 86.0, MX + 15.0, MY0 + 118.0))));
    v.push((Elem::Input(keys::WHEEL_LEFT), Hit::C(tf.p(MX - 30.0, MY0 + 80.0), tf.l(11.0))));
    v.push((Elem::Input(keys::WHEEL_RIGHT), Hit::C(tf.p(MX + 30.0, MY0 + 80.0), tf.l(11.0))));
    v.push((Elem::MouseMove, Hit::C(tf.p(MX, MY0 + 285.0), tf.l(55.0))));
    v.push((Elem::Input(keys::MOUSE_LEFT), Hit::R(tf.r(MX - MW, MY0, MX, MSPLIT))));
    v.push((Elem::Input(keys::MOUSE_RIGHT), Hit::R(tf.r(MX, MY0, MX + MW, MSPLIT))));
    for c in mouse_callouts() {
        v.push((c.elem, callout_hit(tf, &c, &M_COLS)));
    }
    v
}

fn chevron(cx: &Ctx, c: Pos2, w: f32, up: bool, s: Stroke) {
    let d = if up { -1.0 } else { 1.0 };
    cx.p.add(Shape::line(vec![c + vec2(-w, -d * w * 0.5), c + vec2(0.0, d * w * 0.5), c + vec2(w, -d * w * 0.5)], s));
}

fn draw_mouse(cx: &Ctx) {
    let (tf, pal, p) = (cx.tf, cx.v.pal, cx.p);
    let body_w = cx.width(St::default());
    let ink = pal.device;

    // Button highlight regions (drawn under the outline).
    for (id, left) in [(keys::MOUSE_LEFT, true), (keys::MOUSE_RIGHT, false)] {
        let s = cx.st(Elem::Input(id));
        let fill = cx.fill(s);
        if fill.a() > 0 {
            let mut pts = vec![tf.p(MX, MY0 + 2.0)];
            let n = 24;
            for i in 0..=n {
                let y = MY0 + (MSPLIT - MY0) * i as f32 / n as f32;
                let x = if left { MX - mouse_half(y) } else { MX + mouse_half(y) };
                pts.push(tf.p(x, y));
            }
            pts.push(tf.p(MX, MSPLIT));
            p.add(Shape::convex_polygon(pts, fill, Stroke::NONE));
        }
    }

    // Body outline.
    let n = 64;
    let mut pts = Vec::with_capacity(2 * n + 2);
    for i in 0..=n {
        let y = MY0 + (MY1 - MY0) * i as f32 / n as f32;
        pts.push(tf.p(MX + mouse_half(y), y));
    }
    for i in (0..=n).rev() {
        let y = MY0 + (MY1 - MY0) * i as f32 / n as f32;
        pts.push(tf.p(MX - mouse_half(y), y));
    }
    p.add(Shape::closed_line(pts, Stroke::new(body_w, ink)));

    // Button split.
    let hs = mouse_half(MSPLIT);
    let mut split = vec![];
    for i in 0..=20 {
        let t = i as f32 / 20.0;
        let x = MX - hs + 2.0 * hs * t;
        let sag = 10.0 * (1.0 - (2.0 * t - 1.0).powi(2));
        split.push(tf.p(x, MSPLIT + sag));
    }
    p.add(Shape::line(split, Stroke::new(body_w, ink)));
    p.line_segment([tf.p(MX, MY0 + 1.0), tf.p(MX, MY0 + 40.0)], Stroke::new(body_w, ink));
    p.line_segment([tf.p(MX, MY0 + 120.0), tf.p(MX, MSPLIT + 10.0)], Stroke::new(body_w, ink));
    // Button outlines for the selected/hovered state.
    for (id, left) in [(keys::MOUSE_LEFT, true), (keys::MOUSE_RIGHT, false)] {
        let s = cx.st(Elem::Input(id));
        if s.sel || s.hot {
            let x = if left { MX - 70.0 } else { MX + 70.0 };
            p.circle_filled(tf.p(x, MY0 + 95.0), tf.l(4.0), pal.gold);
        }
    }

    // Wheel.
    let wheel = tf.r(MX - 15.0, MY0 + 42.0, MX + 15.0, MY0 + 118.0);
    let any = [keys::WHEEL_UP, keys::WHEEL_DOWN, keys::MOUSE_MIDDLE].map(|id| cx.st(Elem::Input(id)));
    let wheel_fill = if any.iter().any(|s| s.sel || s.hot) { pal.gold_soft } else { pal.bg };
    p.rect(wheel, tf.l(15.0), wheel_fill, Stroke::new(body_w, ink), StrokeKind::Middle);
    let su = cx.st(Elem::Input(keys::WHEEL_UP));
    let sd = cx.st(Elem::Input(keys::WHEEL_DOWN));
    let sm = cx.st(Elem::Input(keys::MOUSE_MIDDLE));
    chevron(cx, tf.p(MX, MY0 + 60.0), tf.l(8.0), true, Stroke::new(cx.width(su), cx.ink(su)));
    chevron(cx, tf.p(MX, MY0 + 100.0), tf.l(8.0), false, Stroke::new(cx.width(sd), cx.ink(sd)));
    p.line_segment([tf.p(MX - 9.0, MY0 + 80.0), tf.p(MX + 9.0, MY0 + 80.0)], Stroke::new(cx.width(sm), cx.ink(sm)));
    for (id, dx) in [(keys::WHEEL_LEFT, -30.0f32), (keys::WHEEL_RIGHT, 30.0)] {
        let s = cx.st(Elem::Input(id));
        let c = tf.p(MX + dx, MY0 + 80.0);
        let w = tf.l(5.0);
        let d = dx.signum();
        p.add(Shape::line(vec![c + vec2(-d * w * 0.5, -w), c + vec2(d * w * 0.5, 0.0), c + vec2(-d * w * 0.5, w)], Stroke::new(cx.width(s), cx.ink(s))));
    }

    // Side buttons.
    for (n, id, label) in [(2u8, keys::MOUSE_X2, "X2"), (1, keys::MOUSE_X1, "X1")] {
        let s = cx.st(Elem::Input(id));
        let (x0, y0, x1, y1) = side_button(n);
        let r = tf.r(x0, y0, x1, y1);
        p.rect(r, tf.l(9.0), if s.sel || s.hot { pal.gold_soft } else { pal.bg }, Stroke::new(cx.width(s), cx.ink(s)), StrokeKind::Middle);
        p.text(r.center(), Align2::CENTER_CENTER, label, theme::bold(tf.font(12.0, 8.0, 13.0)), cx.ink(s));
    }

    // Movement cross.
    let s = cx.st(Elem::MouseMove);
    let c = tf.p(MX, MY0 + 285.0);
    if s.sel || s.hot {
        p.circle_filled(c, tf.l(52.0), pal.gold_soft);
    }
    let st = Stroke::new(cx.width(s), cx.ink(s));
    for d in [vec2(0.0, -1.0), vec2(0.0, 1.0), vec2(-1.0, 0.0), vec2(1.0, 0.0)] {
        let a = c + d * tf.l(12.0);
        let b = c + d * tf.l(42.0);
        p.line_segment([a, b], st);
        let perp = vec2(-d.y, d.x);
        p.add(Shape::line(vec![b - d * tf.l(10.0) + perp * tf.l(9.0), b, b - d * tf.l(10.0) - perp * tf.l(9.0)], st));
    }
    p.circle_stroke(c, tf.l(4.0), st);

    // Callouts.
    for co in mouse_callouts() {
        let value = match co.elem {
            Elem::MouseMove => match cx.v.sum.mouse {
                StickTarget::Right => "Right stick".to_string(),
                StickTarget::Left => "Left stick".to_string(),
                StickTarget::None => String::new(),
            },
            Elem::Input(id) => cx.v.sum.out.get(&id).cloned().unwrap_or_default(),
            _ => String::new(),
        };
        draw_callout(cx, &co, &M_COLS, &value);
    }
}

// ------------------------------------------------------------- gamepad --

struct PadGeo {
    outline: Vec<(f32, f32)>,
    lstick: (f32, f32),
    rstick: (f32, f32),
    dpad: (f32, f32),
    face: (f32, f32),
    back: (f32, f32),
    start: (f32, f32),
    guide: (f32, f32),
    guide_r: f32,
    touchpad: Option<(f32, f32, f32, f32)>,
}

const G_COLS: Cols = Cols { l_edge: 205.0, r_edge: 695.0, width: 200.0 };

fn pad_geo(kind: ControllerKind) -> PadGeo {
    // Right half as cubic segments, mirrored for the left half.
    let right: &[[(f32, f32); 4]] = match kind {
        ControllerKind::Xbox360 => &[
            [(450.0, 112.0), (520.0, 112.0), (560.0, 84.0), (600.0, 82.0)],
            [(600.0, 82.0), (652.0, 80.0), (680.0, 104.0), (692.0, 142.0)],
            [(692.0, 142.0), (708.0, 192.0), (728.0, 282.0), (724.0, 332.0)],
            [(724.0, 332.0), (721.0, 374.0), (690.0, 388.0), (668.0, 380.0)],
            [(668.0, 380.0), (640.0, 371.0), (622.0, 331.0), (596.0, 302.0)],
            [(596.0, 302.0), (570.0, 274.0), (530.0, 270.0), (450.0, 270.0)],
        ],
        ControllerKind::Ds4 => &[
            [(450.0, 96.0), (500.0, 96.0), (540.0, 96.0), (578.0, 96.0)],
            [(578.0, 96.0), (640.0, 94.0), (690.0, 108.0), (702.0, 160.0)],
            [(702.0, 160.0), (714.0, 214.0), (736.0, 300.0), (731.0, 350.0)],
            [(731.0, 350.0), (727.0, 386.0), (690.0, 394.0), (671.0, 376.0)],
            [(671.0, 376.0), (650.0, 354.0), (630.0, 302.0), (600.0, 284.0)],
            [(600.0, 284.0), (570.0, 268.0), (520.0, 264.0), (450.0, 264.0)],
        ],
    };
    let mut half: Vec<Pos2> = vec![pos2(right[0][0].0, right[0][0].1)];
    for seg in right {
        let q = seg.map(|(x, y)| pos2(x, y));
        bezier(q[0], q[1], q[2], q[3], &mut half);
    }
    let mut outline: Vec<(f32, f32)> = half.iter().map(|p| (p.x, p.y)).collect();
    outline.extend(half.iter().rev().skip(1).map(|p| (900.0 - p.x, p.y)));
    match kind {
        ControllerKind::Xbox360 => PadGeo {
            outline,
            lstick: (322.0, 166.0),
            rstick: (518.0, 238.0),
            dpad: (382.0, 238.0),
            face: (580.0, 166.0),
            back: (412.0, 168.0),
            start: (488.0, 168.0),
            guide: (450.0, 134.0),
            guide_r: 18.0,
            touchpad: None,
        },
        ControllerKind::Ds4 => PadGeo {
            outline,
            lstick: (386.0, 232.0),
            rstick: (514.0, 232.0),
            dpad: (302.0, 164.0),
            face: (598.0, 164.0),
            back: (364.0, 116.0),
            start: (536.0, 116.0),
            guide: (450.0, 212.0),
            guide_r: 12.0,
            touchpad: Some((382.0, 98.0, 518.0, 176.0)),
        },
    }
}

fn face_offsets() -> [(Control, f32, f32); 4] {
    [(Control::Y, 0.0, -31.0), (Control::B, 31.0, 0.0), (Control::A, 0.0, 31.0), (Control::X, -31.0, 0.0)]
}

fn gamepad_callouts(kind: ControllerKind) -> Vec<Callout> {
    let g = pad_geo(kind);
    let f = g.face;
    let name = |c: Control| c.label(kind).to_string();
    let ds4 = kind == ControllerKind::Ds4;
    let mut v = vec![
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Lt)), anchor: (300.0, 44.0), y: 42.0, left: true, name: format!("{} trigger", name(Control::Lt)) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Lb)), anchor: (298.0, 82.0), y: 90.0, left: true, name: format!("{} bumper", name(Control::Lb)) },
        Callout { elem: Elem::Pad(PadEl::Stick(Side::Left)), anchor: (g.lstick.0 - 36.0, g.lstick.1), y: if ds4 { 300.0 } else { 166.0 }, left: true, name: "Left stick".into() },
        Callout { elem: Elem::Pad(PadEl::Dpad), anchor: (g.dpad.0 - 34.0, g.dpad.1), y: if ds4 { 164.0 } else { 238.0 }, left: true, name: "D-pad".into() },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Back)), anchor: (g.back.0 - 10.0, g.back.1), y: if ds4 { 230.0 } else { 312.0 }, left: true, name: name(Control::Back) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Guide)), anchor: (g.guide.0 - g.guide_r, g.guide.1), y: 372.0, left: true, name: name(Control::Guide) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Rt)), anchor: (600.0, 44.0), y: 42.0, left: false, name: format!("{} trigger", name(Control::Rt)) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Rb)), anchor: (602.0, 82.0), y: 90.0, left: false, name: format!("{} bumper", name(Control::Rb)) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Y)), anchor: (f.0 + 17.0, f.1 - 31.0), y: 134.0, left: false, name: name(Control::Y) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::B)), anchor: (f.0 + 48.0, f.1), y: 176.0, left: false, name: name(Control::B) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::A)), anchor: (f.0 + 17.0, f.1 + 31.0), y: 218.0, left: false, name: name(Control::A) },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::X)), anchor: (f.0 - 31.0, f.1 + 17.0), y: 260.0, left: false, name: name(Control::X) },
        Callout { elem: Elem::Pad(PadEl::Stick(Side::Right)), anchor: (g.rstick.0 + 36.0, g.rstick.1), y: 306.0, left: false, name: "Right stick".into() },
        Callout { elem: Elem::Pad(PadEl::Btn(Control::Start)), anchor: (g.start.0 + 10.0, g.start.1), y: 352.0, left: false, name: name(Control::Start) },
    ];
    if let Some((x0, y0, _, y1)) = g.touchpad {
        v.push(Callout { elem: Elem::Pad(PadEl::Btn(Control::Touchpad)), anchor: (x0 + 30.0, (y0 + y1) / 2.0), y: 412.0, left: true, name: "Touchpad click".into() });
    }
    v
}

fn bumper_pts(left: bool) -> Vec<(f32, f32)> {
    let base = [(272.0, 112.0), (290.0, 86.0), (326.0, 72.0), (372.0, 74.0)];
    base.iter().map(|&(x, y)| if left { (x, y) } else { (900.0 - x, y) }).collect()
}

fn gamepad_hits(tf: Tf, kind: ControllerKind) -> Vec<(Elem, Hit)> {
    let g = pad_geo(kind);
    let pe = |p: PadEl| Elem::Pad(p);
    let mut v = vec![];
    for (c, dx, dy) in face_offsets() {
        v.push((pe(PadEl::Btn(c)), Hit::C(tf.p(g.face.0 + dx, g.face.1 + dy), tf.l(17.0))));
    }
    v.push((pe(PadEl::Btn(Control::Back)), Hit::C(tf.p(g.back.0, g.back.1), tf.l(13.0))));
    v.push((pe(PadEl::Btn(Control::Start)), Hit::C(tf.p(g.start.0, g.start.1), tf.l(13.0))));
    v.push((pe(PadEl::Btn(Control::Guide)), Hit::C(tf.p(g.guide.0, g.guide.1), tf.l(g.guide_r + 3.0))));
    v.push((pe(PadEl::Stick(Side::Left)), Hit::C(tf.p(g.lstick.0, g.lstick.1), tf.l(36.0))));
    v.push((pe(PadEl::Stick(Side::Right)), Hit::C(tf.p(g.rstick.0, g.rstick.1), tf.l(36.0))));
    v.push((pe(PadEl::Dpad), Hit::R(Rect::from_center_size(tf.p(g.dpad.0, g.dpad.1), Vec2::splat(tf.l(72.0))))));
    if let Some((x0, y0, x1, y1)) = g.touchpad {
        v.push((pe(PadEl::Btn(Control::Touchpad)), Hit::R(tf.r(x0, y0, x1, y1))));
    }
    v.push((pe(PadEl::Btn(Control::Lt)), Hit::R(tf.r(290.0, 26.0, 360.0, 60.0))));
    v.push((pe(PadEl::Btn(Control::Rt)), Hit::R(tf.r(540.0, 26.0, 610.0, 60.0))));
    v.push((pe(PadEl::Btn(Control::Lb)), Hit::R(tf.r(266.0, 64.0, 378.0, 100.0))));
    v.push((pe(PadEl::Btn(Control::Rb)), Hit::R(tf.r(522.0, 64.0, 634.0, 100.0))));
    for c in gamepad_callouts(kind) {
        v.push((c.elem, callout_hit(tf, &c, &G_COLS)));
    }
    v
}

fn ds4_symbol(p: &Painter, c: Pos2, r: f32, ctl: Control, st: Stroke) {
    match ctl {
        Control::A => {
            p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], st);
            p.line_segment([c + vec2(r, -r), c + vec2(-r, r)], st);
        }
        Control::B => {
            p.circle_stroke(c, r * 1.05, st);
        }
        Control::X => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::splat(r * 1.9)), 1.0, st, StrokeKind::Middle);
        }
        _ => {
            p.add(Shape::closed_line(vec![c + vec2(0.0, -r * 1.1), c + vec2(r * 1.1, r * 0.8), c + vec2(-r * 1.1, r * 0.8)], st));
        }
    }
}

fn draw_gamepad(cx: &Ctx) {
    let (tf, pal, p) = (cx.tf, cx.v.pal, cx.p);
    let kind = cx.v.kind;
    let g = pad_geo(kind);
    let live = cx.v.live;
    let pressed = |c: Control| live.is_some_and(|s| s.pressed(c));
    let st_btn = |c: Control| {
        let mut s = cx.st(Elem::Pad(pad_el(c)));
        s.pressed = pressed(c);
        s
    };
    let bw = cx.width(St::default());

    // Body.
    let outline: Vec<Pos2> = g.outline.iter().map(|&(x, y)| tf.p(x, y)).collect();
    p.add(Shape::closed_line(outline, Stroke::new(bw, pal.device)));

    // Triggers.
    let (lt, rt) = live.map(|s| s.triggers()).unwrap_or((0.0, 0.0));
    for (c, x0, val) in [(Control::Lt, 300.0f32, lt), (Control::Rt, 540.0, rt)] {
        let mut s = cx.st(Elem::Pad(PadEl::Btn(c)));
        s.pressed = val > 0.05;
        let r = tf.r(x0, 30.0, x0 + 60.0, 56.0);
        p.rect(r, tf.l(10.0), cx.fill(s), Stroke::new(cx.width(s), cx.ink(s)), StrokeKind::Middle);
        if val > 0.0 {
            let mut f = r.shrink(tf.l(3.0));
            f.set_width(f.width() * val);
            p.rect_filled(f, tf.l(7.0), pal.gold.gamma_multiply(0.6));
        }
        p.text(r.center(), Align2::CENTER_CENTER, c.label(kind), theme::bold(tf.font(12.0, 8.0, 13.0)), cx.ink(s));
    }
    // Bumpers.
    for (c, left) in [(Control::Lb, true), (Control::Rb, false)] {
        let s = st_btn(c);
        let pts: Vec<Pos2> = bumper_pts(left).iter().map(|&(x, y)| tf.p(x, y)).collect();
        let w = tf.l(if s.sel || s.hot || s.pressed { 9.0 } else { 7.0 });
        let col = if s.sel || s.hot || s.pressed { pal.gold } else { pal.device.gamma_multiply(0.85) };
        p.add(Shape::line(pts, Stroke::new(w, col)));
    }

    // Touchpad (DS4).
    if let Some((x0, y0, x1, y1)) = g.touchpad {
        let s = st_btn(Control::Touchpad);
        let r = tf.r(x0, y0, x1, y1);
        p.rect(r, tf.l(12.0), cx.fill(s), Stroke::new(cx.width(s), cx.ink(s)), StrokeKind::Middle);
    }

    // Sticks.
    for (side, (x, y)) in [(Side::Left, g.lstick), (Side::Right, g.rstick)] {
        let mut s = cx.st(Elem::Pad(PadEl::Stick(side)));
        let click = if side == Side::Left { Control::Ls } else { Control::Rs };
        s.pressed = pressed(click);
        let c = tf.p(x, y);
        let (sx, sy) = match (live, side) {
            (Some(l), Side::Left) => (l.lx, l.ly),
            (Some(l), Side::Right) => (l.rx, l.ry),
            _ => (0.0, 0.0),
        };
        let moved = sx.abs() + sy.abs() > 0.02;
        if s.sel || s.hot {
            p.circle_filled(c, tf.l(34.0), pal.gold_soft);
        }
        p.circle_stroke(c, tf.l(34.0), Stroke::new(cx.width(s), cx.ink(s)));
        let cap = c + vec2(sx, -sy) * tf.l(12.0);
        p.circle(cap, tf.l(21.0), if s.pressed { pal.gold.gamma_multiply(0.55) } else { Color32::TRANSPARENT }, Stroke::new(cx.width(s), if moved { pal.gold } else { cx.ink(s) }));
        p.circle_stroke(cap, tf.l(13.0), Stroke::new(1.0, if moved { pal.gold } else { cx.ink(s).gamma_multiply(0.6) }));
    }

    // D-pad.
    {
        let s = cx.st(Elem::Pad(PadEl::Dpad));
        let c = tf.p(g.dpad.0, g.dpad.1);
        let (a, w) = (tf.l(34.0), tf.l(12.0));
        let pts = vec![
            c + vec2(-w, -a),
            c + vec2(w, -a),
            c + vec2(w, -w),
            c + vec2(a, -w),
            c + vec2(a, w),
            c + vec2(w, w),
            c + vec2(w, a),
            c + vec2(-w, a),
            c + vec2(-w, w),
            c + vec2(-a, w),
            c + vec2(-a, -w),
            c + vec2(-w, -w),
        ];
        if s.sel || s.hot {
            p.circle_filled(c, tf.l(36.0), pal.gold_soft);
        }
        p.add(Shape::closed_line(pts, Stroke::new(cx.width(s), cx.ink(s))));
        for (ctl, d) in [(Control::DpadUp, vec2(0.0, -1.0)), (Control::DpadDown, vec2(0.0, 1.0)), (Control::DpadLeft, vec2(-1.0, 0.0)), (Control::DpadRight, vec2(1.0, 0.0))] {
            let tip = c + d * tf.l(27.0);
            let perp = vec2(-d.y, d.x) * tf.l(6.0);
            let base = c + d * tf.l(19.0);
            let col = if pressed(ctl) { pal.gold } else { cx.ink(s).gamma_multiply(0.8) };
            p.add(Shape::convex_polygon(vec![tip, base + perp, base - perp], col, Stroke::NONE));
        }
    }

    // Face buttons.
    for (ctl, dx, dy) in face_offsets() {
        let s = st_btn(ctl);
        let c = tf.p(g.face.0 + dx, g.face.1 + dy);
        p.circle(c, tf.l(17.0), cx.fill(s), Stroke::new(cx.width(s), cx.ink(s)));
        let ink = if s.pressed { pal.on_gold } else { cx.ink(s) };
        if kind == ControllerKind::Ds4 {
            ds4_symbol(p, c, tf.l(7.0), ctl, Stroke::new(cx.width(s), ink));
        } else {
            p.text(c, Align2::CENTER_CENTER, ctl.label(kind), theme::bold(tf.font(17.0, 9.0, 19.0)), ink);
        }
    }

    // Back / Start / Guide.
    for ctl in [Control::Back, Control::Start] {
        let s = st_btn(ctl);
        let (x, y) = if ctl == Control::Back { g.back } else { g.start };
        let c = tf.p(x, y);
        if kind == ControllerKind::Ds4 {
            let r = Rect::from_center_size(c, vec2(tf.l(10.0), tf.l(20.0)));
            p.rect(r, tf.l(5.0), cx.fill(s), Stroke::new(cx.width(s), cx.ink(s)), StrokeKind::Middle);
        } else {
            p.circle(c, tf.l(10.0), cx.fill(s), Stroke::new(cx.width(s), cx.ink(s)));
            let ink = cx.ink(s);
            if ctl == Control::Start {
                for dy in [-3.0, 0.0, 3.0] {
                    p.line_segment([c + vec2(-tf.l(4.0), tf.l(dy)), c + vec2(tf.l(4.0), tf.l(dy))], Stroke::new(1.0, ink));
                }
            } else {
                p.rect_stroke(Rect::from_center_size(c + vec2(-tf.l(1.5), -tf.l(1.5)), Vec2::splat(tf.l(6.0))), 1.0, Stroke::new(1.0, ink), StrokeKind::Middle);
                p.rect_stroke(Rect::from_center_size(c + vec2(tf.l(1.5), tf.l(1.5)), Vec2::splat(tf.l(6.0))), 1.0, Stroke::new(1.0, ink), StrokeKind::Middle);
            }
        }
    }
    {
        let s = st_btn(Control::Guide);
        let c = tf.p(g.guide.0, g.guide.1);
        p.circle(c, tf.l(g.guide_r), cx.fill(s), Stroke::new(cx.width(s), cx.ink(s)));
        p.circle_stroke(c, tf.l(g.guide_r * 0.55), Stroke::new(1.0, cx.ink(s).gamma_multiply(0.7)));
    }

    // Callouts.
    for co in gamepad_callouts(kind) {
        let value = match co.elem {
            Elem::Pad(pe) => cx.v.sum.pad.get(&pe).map(|v| v.join("  ")).unwrap_or_default(),
            _ => String::new(),
        };
        draw_callout(cx, &co, &G_COLS, &value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_groups_stick_directions() {
        let c = crate::config::Config::default();
        let p = &c.profiles[0];
        let s = Summary::build(&p.layers[0].binds, p.mouse.target, ControllerKind::Xbox360);
        let ls = &s.pad[&PadEl::Stick(Side::Left)];
        assert_eq!(&ls[..4], &["W", "A", "S", "D"]);
        assert_eq!(s.pad[&PadEl::Stick(Side::Right)][0], "Mouse");
        assert_eq!(s.out[&0x57], "LS Up");
    }
}
