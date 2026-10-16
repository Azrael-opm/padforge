//! Binding row editor, shared by the preview inspector and the list view.

use super::icons::{self, Icon};
use super::theme::Pal;
use crate::config::*;
use crate::pad::{Control, ControllerKind, Side};
use eframe::egui::{self, vec2, RichText, Sense, Stroke, Vec2};
use std::collections::HashMap;

pub type MacroText = HashMap<(usize, usize, usize), (String, Option<String>)>;

pub const KINDS: [&str; 7] = ["Button", "Trigger", "Stick", "Stick scale", "Layer", "Macro", "Disable"];

/// Default action for a kind name from [`KINDS`].
pub fn default_action(kind: &str, layers: &[String]) -> Action {
    match kind {
        "Button" => Action::Button { button: Control::A, mode: ButtonMode::Hold, turbo_hz: 12.0 },
        "Trigger" => Action::Trigger { trigger: Side::Right, value: 1.0 },
        "Stick" => Action::Stick { stick: Side::Left, x: 0.0, y: 1.0 },
        "Stick scale" => Action::StickScale { stick: Side::Left, factor: 0.5 },
        "Layer" => Action::Layer { layer: layers.first().cloned().unwrap_or_else(|| "Layer 1".into()), mode: LayerMode::Hold },
        "Macro" => Action::Macro { steps: vec![MacroStep::Tap(Control::A)], repeat: false, tap_ms: 30 },
        _ => Action::Disable,
    }
}

pub fn action_kind_combo(ui: &mut egui::Ui, a: &mut Action, id: (usize, usize, usize), layers: &[String]) -> bool {
    let mut kind = a.kind_label();
    let before = kind;
    egui::ComboBox::from_id_salt(("kind", id)).width(108.0).selected_text(kind).show_ui(ui, |ui| {
        for k in KINDS {
            ui.selectable_value(&mut kind, k, k);
        }
    });
    if kind == before {
        return false;
    }
    *a = default_action(kind, layers);
    true
}

fn side_combo(ui: &mut egui::Ui, side: &mut Side, salt: impl std::hash::Hash + std::fmt::Debug, what: &str) -> bool {
    let mut changed = false;
    let name = |s: Side| match s {
        Side::Left => format!("Left {what}"),
        Side::Right => format!("Right {what}"),
    };
    egui::ComboBox::from_id_salt(salt).width(100.0).selected_text(name(*side)).show_ui(ui, |ui| {
        changed |= ui.selectable_value(side, Side::Left, name(Side::Left)).changed();
        changed |= ui.selectable_value(side, Side::Right, name(Side::Right)).changed();
    });
    changed
}

const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
pub const DIRS: [(&str, f32, f32); 8] = [
    ("Up", 0.0, 1.0),
    ("Down", 0.0, -1.0),
    ("Left", -1.0, 0.0),
    ("Right", 1.0, 0.0),
    ("Up-Left", -D, D),
    ("Up-Right", D, D),
    ("Down-Left", -D, -D),
    ("Down-Right", D, -D),
];

pub fn action_settings(ui: &mut egui::Ui, pal: &Pal, a: &mut Action, kind: ControllerKind, id: (usize, usize, usize), layers: &[String], macro_text: &mut MacroText) -> bool {
    let mut changed = false;
    match a {
        Action::Button { button, mode, turbo_hz } => {
            egui::ComboBox::from_id_salt(("btn", id)).width(112.0).selected_text(button.label(kind)).show_ui(ui, |ui| {
                for c in Control::ALL {
                    if kind == ControllerKind::Xbox360 && c == Control::Touchpad {
                        continue;
                    }
                    changed |= ui.selectable_value(button, c, c.label(kind)).changed();
                }
            });
            egui::ComboBox::from_id_salt(("mode", id)).width(78.0).selected_text(format!("{mode:?}")).show_ui(ui, |ui| {
                changed |= ui.selectable_value(mode, ButtonMode::Hold, "Hold").on_hover_text("Pressed while the key is held").changed();
                changed |= ui.selectable_value(mode, ButtonMode::Toggle, "Toggle").on_hover_text("Each press latches / releases").changed();
                changed |= ui.selectable_value(mode, ButtonMode::Turbo, "Turbo").on_hover_text("Rapid-fire while held").changed();
            });
            if *mode == ButtonMode::Turbo {
                changed |= ui.add(egui::DragValue::new(turbo_hz).range(1.0..=60.0).speed(0.2).suffix(" Hz")).changed();
            }
        }
        Action::Trigger { trigger, value } => {
            changed |= side_combo(ui, trigger, ("trig", id), "trigger");
            changed |= ui.add(egui::Slider::new(value, 0.0..=1.0).fixed_decimals(2)).changed();
        }
        Action::Stick { stick, x, y } => {
            changed |= side_combo(ui, stick, ("stick", id), "stick");
            let cur = DIRS.iter().find(|d| (d.1 - *x).abs() < 0.01 && (d.2 - *y).abs() < 0.01).map(|d| d.0).unwrap_or("Custom");
            egui::ComboBox::from_id_salt(("dir", id)).width(100.0).selected_text(cur).show_ui(ui, |ui| {
                for (n, dx, dy) in DIRS {
                    if ui.selectable_label(cur == n, n).clicked() {
                        *x = dx;
                        *y = dy;
                        changed = true;
                    }
                }
            });
            if cur == "Custom" {
                ui.label("x");
                changed |= ui.add(egui::DragValue::new(x).range(-1.0..=1.0).speed(0.01).fixed_decimals(2)).changed();
                ui.label("y");
                changed |= ui.add(egui::DragValue::new(y).range(-1.0..=1.0).speed(0.01).fixed_decimals(2)).changed();
            }
        }
        Action::StickScale { stick, factor } => {
            changed |= side_combo(ui, stick, ("scale", id), "stick");
            changed |= ui.add(egui::Slider::new(factor, 0.0..=1.0).fixed_decimals(2).text("x")).changed();
        }
        Action::Layer { layer, mode } => {
            egui::ComboBox::from_id_salt(("layer", id)).width(112.0).selected_text(layer.as_str()).show_ui(ui, |ui| {
                for n in layers {
                    changed |= ui.selectable_value(layer, n.clone(), n).changed();
                }
            });
            egui::ComboBox::from_id_salt(("lmode", id)).width(78.0).selected_text(format!("{mode:?}")).show_ui(ui, |ui| {
                changed |= ui.selectable_value(mode, LayerMode::Hold, "Hold").changed();
                changed |= ui.selectable_value(mode, LayerMode::Toggle, "Toggle").changed();
            });
            if layers.is_empty() {
                ui.label(RichText::new("add a layer first").color(pal.warn).small());
            }
        }
        Action::Macro { steps, repeat, tap_ms } => {
            let entry = macro_text.entry(id).or_insert_with(|| (macro_to_text(steps), None));
            let resp = ui.add(egui::TextEdit::singleline(&mut entry.0).desired_width(220.0).hint_text("tap a, wait 50, tap b"));
            if resp.changed() {
                match macro_from_text(&entry.0) {
                    Ok(s) => {
                        *steps = s;
                        entry.1 = None;
                        changed = true;
                    }
                    Err(e) => entry.1 = Some(e),
                }
            }
            if let Some(e) = &entry.1 {
                ui.label(RichText::new("(!)").color(pal.err)).on_hover_text(e);
            }
            changed |= ui.checkbox(repeat, "repeat").on_hover_text("Loop while the key is held").changed();
            changed |= ui.add(egui::DragValue::new(tap_ms).range(1..=1000).suffix(" ms tap")).changed();
        }
        Action::Disable => {
            ui.label(RichText::new("the key is blocked, nothing is sent").color(pal.weak));
        }
    }
    changed
}

#[derive(Default)]
pub struct RowOut {
    pub changed: bool,
    pub capture: bool,
    pub remove: bool,
    pub hovered: bool,
}

/// One binding: [input key] -> [action kind] [settings ...]   [block] [delete]
#[allow(clippy::too_many_arguments)]
pub fn bind_row(
    ui: &mut egui::Ui,
    pal: &Pal,
    bind: &mut Bind,
    id: (usize, usize, usize),
    kind: ControllerKind,
    layers: &[String],
    macro_text: &mut MacroText,
    capturing: bool,
    highlight: bool,
) -> RowOut {
    let mut out = RowOut::default();
    let frame = egui::Frame::new()
        .fill(if highlight { pal.gold_soft } else { pal.bg })
        .stroke(Stroke::new(1.0, if highlight { pal.gold } else { pal.line }))
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(8, 5));
    let resp = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let label = if capturing {
                    RichText::new("Press a key...").color(pal.gold).strong()
                } else if bind.input.is_empty() {
                    RichText::new("Set key").color(pal.weak)
                } else {
                    RichText::new(&bind.input).monospace().strong().color(pal.text)
                };
                let btn = egui::Button::new(label).min_size(vec2(112.0, 28.0)).stroke(Stroke::new(1.0, if capturing { pal.gold } else { pal.line })).fill(pal.raised);
                if ui.add(btn).on_hover_text("Click, then press the key or mouse button to use").clicked() {
                    out.capture = true;
                }
                let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                icons::draw(ui.painter(), r, Icon::Arrow, pal.weak, pal.bg);
                out.changed |= action_kind_combo(ui, &mut bind.action, id, layers);
                out.changed |= action_settings(ui, pal, &mut bind.action, kind, id, layers, macro_text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, pal, Icon::Trash, 26.0, false, "Remove this mapping").clicked() {
                        out.remove = true;
                    }
                    out.changed |= ui.checkbox(&mut bind.block, "Block").on_hover_text("Hide the original key from the game").changed();
                });
            });
        })
        .response;
    out.hovered = ui.rect_contains_pointer(resp.rect);
    out
}
