//! Mouse tuning, Settings and Help pages.

use super::icons::{self, Icon, Style};
use super::theme::{self, Pal};
use super::{card, open_url, CaptureTarget, GuiApp};
use crate::config::*;
use crate::pad::{ControllerKind, PadState};
use crate::win::app::load_config;
use crate::win::vpad::VIGEM_URL;
use eframe::egui::{self, pos2, vec2, Align2, RichText, Stroke, Vec2};

fn heading(ui: &mut egui::Ui, pal: &Pal, text: &str) {
    ui.label(RichText::new(text).font(theme::bold(15.0)).color(pal.text));
    ui.add_space(4.0);
}

/// Label column (with hint) + widget column.
fn row(ui: &mut egui::Ui, pal: &Pal, label: &str, hint: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let w = (ui.available_width() * 0.46).clamp(150.0, 250.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), egui::Sense::hover());
        let p = ui.painter();
        if hint.is_empty() {
            p.text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, label, theme::font(14.0), pal.text);
        } else {
            p.text(pos2(r.left(), r.top() + 10.0), Align2::LEFT_CENTER, label, theme::font(14.0), pal.text);
            super::preview::fit_text(p, pos2(r.left(), r.top() + 26.0), Align2::LEFT_CENTER, hint, theme::font(11.0), pal.weak, r.width() - 10.0);
        }
        add(ui);
    });
}

fn toggle(ui: &mut egui::Ui, pal: &Pal, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 22.0), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let p = ui.painter();
    let track = if *on { pal.gold_fill } else { pal.raised };
    p.rect(rect, 11.0, track, Stroke::new(1.0, if *on { pal.gold_fill } else { pal.line }), egui::StrokeKind::Inside);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    p.circle_filled(pos2(x, rect.center().y), 8.0, if *on { pal.on_gold } else { pal.weak });
    resp.clicked()
}

impl GuiApp {
    pub(super) fn mouse_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let pi = self.pi();
        let mut changed = false;
        let live = (self.status.active && self.status.profile == self.config.profiles[pi].name).then_some(self.status.pad);
        let pname = self.config.profiles[pi].name.clone();
        let source = self.status.mouse_source;
        let m = &mut self.config.profiles[pi].mouse;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Mouse tuning").font(theme::bold(24.0)).color(pal.text));
            ui.label(RichText::new(format!("profile: {pname}")).color(pal.weak));
        });
        ui.add_space(8.0);
        card(ui, &pal, 16, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Mouse moves").color(pal.weak));
                for (t, label) in [(StickTarget::Right, "Right stick"), (StickTarget::Left, "Left stick"), (StickTarget::None, "Nothing (normal mouse)")] {
                    if icons::text_button(ui, &pal, None, label, Style::Chip(m.target == t)).clicked() && m.target != t {
                        m.target = t;
                        changed = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if source.is_empty() {
                        return;
                    }
                    ui.label(RichText::new(format!("source: {source}")).small().color(pal.weak)).on_hover_text("Where mouse movement is read from (Settings > Input)");
                });
            });
        });
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.x = 16.0;
            ui.columns(2, |cols| {
                for c in cols.iter_mut() {
                    c.spacing_mut().item_spacing.x = theme::ITEM_X;
                }
            card(&mut cols[0], &pal, 16, |ui| {
                heading(ui, &pal, "Response");
                let sl = |ui: &mut egui::Ui, v: &mut f32, r: std::ops::RangeInclusive<f32>, log: bool| ui.add(egui::Slider::new(v, r).logarithmic(log)).changed();
                row(ui, &pal, "Sensitivity", "Stick deflection per mouse speed", |ui| changed |= sl(ui, &mut m.sensitivity, 0.01..=2.0, true));
                row(ui, &pal, "Vertical ratio", "Y speed relative to X", |ui| changed |= sl(ui, &mut m.y_ratio, 0.25..=3.0, false));
                row(ui, &pal, "Invert Y", "", |ui| changed |= toggle(ui, &pal, &mut m.invert_y));
                row(ui, &pal, "Curve exponent", "<1 boosts small moves, >1 = precision", |ui| changed |= sl(ui, &mut m.exponent, 0.3..=2.5, false));
                row(ui, &pal, "Anti-deadzone", "Match the game's stick dead zone", |ui| changed |= sl(ui, &mut m.anti_deadzone, 0.0..=0.6, false));
                row(ui, &pal, "Smoothing (ms)", "Lower = snappier, higher = smoother", |ui| changed |= sl(ui, &mut m.smoothing_ms, 1.0..=40.0, false));
                row(ui, &pal, "Release (ms)", "Re-centre time after the mouse stops", |ui| changed |= sl(ui, &mut m.release_ms, 0.0..=120.0, false));
                ui.add_space(6.0);
                if icons::text_button(ui, &pal, Some(Icon::Refresh), "Reset to defaults", Style::Normal).clicked() {
                    *m = MouseConfig { target: m.target, ..Default::default() };
                    changed = true;
                }
            });
            let ui = &mut cols[1];
            card(ui, &pal, 16, |ui| {
                heading(ui, &pal, "Mouse speed vs stick deflection");
                curve_plot(ui, &pal, m, live);
                ui.add_space(10.0);
                for tip in [
                    "Raise anti-deadzone until slow mouse movement turns the camera immediately.",
                    "Set sensitivity so a fast flick just reaches full stick.",
                    "Maximum turn speed is capped by the game's own stick sensitivity: raise that in-game too.",
                ] {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("-").color(pal.gold));
                        ui.label(RichText::new(tip).small().color(pal.weak));
                    });
                }
            });
        });
        if changed {
            self.changed();
        }
    }

    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let mut changed = false;
        let mut capture_toggle = false;
        let mut reload = false;
        let capturing_toggle = self.capture.as_ref().map(|c| c.target) == Some(CaptureTarget::Toggle);
        let config_path = self.paths.config.clone();
        ui.label(RichText::new("Settings").font(theme::bold(24.0)).color(pal.text));
        ui.add_space(8.0);
        let s = &mut self.config.settings;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.columns(2, |cols| {
                for c in cols.iter_mut() {
                    c.spacing_mut().item_spacing.x = theme::ITEM_X;
                }
                let ui = &mut cols[0];
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "General");
                    row(ui, &pal, "Toggle hotkey", "Works in-game; never sent to the game", |ui| {
                        let t = if capturing_toggle { RichText::new("Press a key...").color(pal.gold) } else { RichText::new(&s.toggle_hotkey).monospace().strong() };
                        if ui.add(egui::Button::new(t).min_size(vec2(110.0, 28.0))).clicked() {
                            capture_toggle = true;
                        }
                    });
                    row(ui, &pal, "Virtual controller", "", |ui| {
                        for (k, label) in [(ControllerKind::Xbox360, "Xbox 360"), (ControllerKind::Ds4, "DualShock 4")] {
                            if icons::text_button(ui, &pal, None, label, Style::Chip(s.controller == k)).clicked() && s.controller != k {
                                s.controller = k;
                                changed = true;
                            }
                        }
                    });
                    row(ui, &pal, "Enable on startup", "", |ui| changed |= toggle(ui, &pal, &mut s.start_enabled));
                    row(ui, &pal, "Keep controller connected", "No disconnects while disabled", |ui| changed |= toggle(ui, &pal, &mut s.keep_connected));
                    row(ui, &pal, "Only emulate in listed games", "Pass input through elsewhere", |ui| changed |= toggle(ui, &pal, &mut s.suspend_when_unmatched));
                    row(ui, &pal, "Sound on toggle", "Handy in fullscreen games", |ui| changed |= toggle(ui, &pal, &mut s.toggle_sound));
                });
                ui.add_space(10.0);
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Game launch");
                    row(ui, &pal, "Enable emulation on launch", "When starting a game from PadForge", |ui| changed |= toggle(ui, &pal, &mut s.enable_on_launch));
                    row(ui, &pal, "Hide to tray on launch", "Keeps the window out of the way", |ui| changed |= toggle(ui, &pal, &mut s.hide_on_launch));
                });
                ui.add_space(10.0);
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Appearance");
                    row(ui, &pal, "Theme", "", |ui| {
                        for (t, icon, label) in [(Theme::Dark, Icon::Moon, "Dark"), (Theme::Light, Icon::Sun, "Light")] {
                            if icons::text_button(ui, &pal, Some(icon), label, Style::Chip(s.theme == t)).clicked() && s.theme != t {
                                s.theme = t;
                                changed = true;
                            }
                        }
                    });
                });

                let ui = &mut cols[1];
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Input");
                    row(ui, &pal, "Opposite directions (A+D)", "", |ui| {
                        for (v, label) in [(Socd::LastWins, "Last wins"), (Socd::Neutral, "Cancel out")] {
                            if icons::text_button(ui, &pal, None, label, Style::Chip(s.socd == v)).clicked() && s.socd != v {
                                s.socd = v;
                                changed = true;
                            }
                        }
                    });
                    row(ui, &pal, "Mouse source", "Auto = Raw Input when available", |ui| {
                        for (v, label) in [(MouseSource::Auto, "Auto"), (MouseSource::Raw, "Raw"), (MouseSource::Hook, "Hook")] {
                            if icons::text_button(ui, &pal, None, label, Style::Chip(s.mouse_source == v)).clicked() && s.mouse_source != v {
                                s.mouse_source = v;
                                changed = true;
                            }
                        }
                    });
                    row(ui, &pal, "Re-centre frozen cursor", "", |ui| changed |= toggle(ui, &pal, &mut s.recenter_cursor));
                    row(ui, &pal, "Timed update rate", "Mouse decay, turbo, macros", |ui| changed |= ui.add(egui::Slider::new(&mut s.output_rate_hz, 125..=1000).suffix(" Hz")).changed());
                    row(ui, &pal, "Wheel press length", "", |ui| changed |= ui.add(egui::Slider::new(&mut s.wheel_press_ms, 10..=200).suffix(" ms")).changed());
                    row(ui, &pal, "Above-normal priority", "Applies on next start", |ui| changed |= toggle(ui, &pal, &mut s.high_priority));
                });
                ui.add_space(10.0);
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Window & tray");
                    row(ui, &pal, "Close button hides to tray", "", |ui| changed |= toggle(ui, &pal, &mut s.close_to_tray));
                    row(ui, &pal, "Start hidden in tray", "", |ui| changed |= toggle(ui, &pal, &mut s.start_in_tray));
                });
                ui.add_space(10.0);
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Configuration file");
                    ui.label(RichText::new(config_path.display().to_string()).small().monospace().color(pal.weak));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if icons::text_button(ui, &pal, Some(Icon::Folder), "Open folder", Style::Normal).clicked() {
                            if let Some(dir) = config_path.parent() {
                                open_url(&dir.display().to_string());
                            }
                        }
                        if icons::text_button(ui, &pal, Some(Icon::Refresh), "Reload from disk", Style::Normal).clicked() {
                            reload = true;
                        }
                    });
                });
            });
        });
        if reload {
            let (c, err) = load_config(&self.paths.config);
            self.config = c;
            self.notice = err.map(|e| (e, true));
            self.macro_text.clear();
            self.exes_text.0 = usize::MAX;
            self.sel_profile = 0;
            self.sel_layer = 0;
            self.selected = None;
            changed = true;
        }
        if capture_toggle {
            self.start_capture(CaptureTarget::Toggle);
        }
        if changed {
            self.changed();
        }
    }

    pub(super) fn help_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let hk = self.config.settings.toggle_hotkey.clone();
        ui.label(RichText::new("Help").font(theme::bold(24.0)).color(pal.text));
        ui.add_space(8.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.columns(2, |cols| {
                for c in cols.iter_mut() {
                    c.spacing_mut().item_spacing.x = theme::ITEM_X;
                }
                let ui = &mut cols[0];
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Quick start");
                    let steps = [
                        "Install the ViGEmBus driver once, then restart PadForge.".to_string(),
                        "Add a game profile and pick the running game or browse for its .exe.".to_string(),
                        "Map keys: click a key or controller button in the preview, or use the List view.".to_string(),
                        format!("Launch the game from PadForge (or start it yourself) and press {hk}."),
                        "The game sees a real Xbox 360 / DualShock 4 controller. Mapped keys are hidden from it.".to_string(),
                    ];
                    for (i, s) in steps.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{}", i + 1)).font(theme::bold(14.0)).color(pal.gold));
                            ui.label(RichText::new(s).color(pal.text));
                        });
                    }
                    ui.add_space(6.0);
                    if icons::text_button(ui, &pal, Some(Icon::Arrow), "Download ViGEmBus", Style::Link).clicked() {
                        open_url(VIGEM_URL);
                    }
                    ui.label(RichText::new("Emulation pauses while this window is focused, so the mouse works here.").small().color(pal.weak));
                });
                ui.add_space(10.0);
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Two ways to map");
                    ui.label(RichText::new("Preview: click any key, mouse button, stick or controller button. The panel below shows what it does and offers one-click assignments. Mapped elements are gold, with callouts.").color(pal.text));
                    ui.add_space(4.0);
                    ui.label(RichText::new("List: every mapping of the layer as rows. The preview stays visible and lights up the row under the pointer.").color(pal.text));
                });
                ui.add_space(10.0);
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Fair play");
                    ui.label(RichText::new("Controller emulation in online or competitive games may break their rules. PadForge is meant for single-player games, controller-only games, accessibility, and games that permit it.").color(pal.weak));
                });

                let ui = &mut cols[1];
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Actions");
                    for (n, d) in [
                        ("Button", "Hold, toggle (latch) or turbo a controller button."),
                        ("Trigger", "Analog trigger at a fixed value, e.g. 0.5 for a half press."),
                        ("Stick", "Push a stick in a direction; held keys combine and diagonals are normalised."),
                        ("Stick scale", "Scale a stick while held, e.g. 0.5 = walk."),
                        ("Layer", "Switch to another set of mappings while held or toggled. Unmapped keys fall through."),
                        ("Macro", "e.g. tap a, wait 50, press rb, wait 200, release rb. Repeat loops while held."),
                        ("Disable", "Swallow the key so the game never sees it."),
                    ] {
                        ui.label(RichText::new(n).font(theme::bold(14.0)).color(pal.gold));
                        ui.label(RichText::new(d).color(pal.text));
                        ui.add_space(3.0);
                    }
                });
                ui.add_space(10.0);
                card(ui, &pal, 16, |ui| {
                    heading(ui, &pal, "Command line");
                    for (a, d) in [
                        ("--launch \"<profile>\"", "start the profile's game and enable emulation"),
                        ("--headless", "tray only, no window"),
                        ("--enable", "start with emulation on"),
                        ("--config <file>", "use another config file"),
                    ] {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(a).monospace().color(pal.text));
                            ui.label(RichText::new(d).small().color(pal.weak));
                        });
                    }
                });
            });
        });
    }
}

fn curve_plot(ui: &mut egui::Ui, pal: &Pal, m: &MouseConfig, live: Option<PadState>) {
    let size = Vec2::new(ui.available_width(), 240.0);
    let (resp, p) = ui.allocate_painter(size, egui::Sense::hover());
    let r = resp.rect.shrink2(vec2(10.0, 14.0));
    p.rect_filled(resp.rect, 10.0, pal.bg);
    for i in 0..=4 {
        let y = r.bottom() - r.height() * i as f32 / 4.0;
        p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, pal.line));
    }
    // x axis: mouse speed (counts/ms) up to 1.3x the stick saturation speed.
    let sat = if m.sensitivity > 0.0 { 1.0 / m.sensitivity } else { 10.0 };
    let max_v = (sat * 1.3).max(0.5);
    let pts: Vec<egui::Pos2> = (0..=120)
        .map(|i| {
            let v = max_v * i as f32 / 120.0;
            let (o, _) = crate::mouse_stick::shape(v, 0.0, m);
            pos2(r.left() + r.width() * i as f32 / 120.0, r.bottom() - r.height() * o.abs())
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.5, pal.gold)));
    p.text(r.left_top(), Align2::LEFT_TOP, "full stick", theme::font(10.5), pal.weak);
    p.text(r.right_bottom() + vec2(0.0, 12.0), Align2::RIGHT_BOTTOM, format!("{max_v:.1} counts/ms"), egui::FontId::monospace(10.0), pal.weak);
    if m.anti_deadzone > 0.0 {
        let y = r.bottom() - r.height() * m.anti_deadzone;
        p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, pal.weak));
        p.text(pos2(r.right(), y - 2.0), Align2::RIGHT_BOTTOM, "anti-deadzone", theme::font(10.5), pal.weak);
    }
    if let Some(s) = live {
        let (x, y) = match m.target {
            StickTarget::Left => (s.lx, s.ly),
            _ => (s.rx, s.ry),
        };
        let mag = (x * x + y * y).sqrt().min(1.0);
        p.circle_filled(pos2(r.left() + 6.0, r.bottom() - r.height() * mag), 6.0, pal.gold_fill);
    }
}
