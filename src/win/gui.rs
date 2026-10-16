//! egui configuration window. Edits apply live to the engine and auto-save.
//! Repaints are on demand: idle window = no CPU. Live controller view only
//! refreshes (20 Hz) while emulation is actually active.

use super::app::{load_config, save_config, Paths};
use super::vpad::VIGEM_URL;
use crate::config::*;
use crate::engine::Msg;
use crate::keys;
use crate::pad::{Control, ControllerKind, PadState, Side};
use crate::shared::{Shared, Status};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::sync::atomic::Ordering::*;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(90, 170, 255);
const ON: Color32 = Color32::from_rgb(60, 190, 110);
const OFF: Color32 = Color32::from_rgb(120, 120, 130);
const WARN: Color32 = Color32::from_rgb(240, 170, 60);
const ERR: Color32 = Color32::from_rgb(235, 90, 90);

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Bindings,
    Mouse,
    Settings,
    Help,
}

#[derive(Clone, Copy, PartialEq)]
enum CaptureTarget {
    Bind { profile: usize, layer: usize, bind: usize },
    Toggle,
}

struct Capture {
    target: CaptureTarget,
    started: Instant,
}

pub struct GuiApp {
    shared: Arc<Shared>,
    tx: SyncSender<Msg>,
    config: Config,
    paths: Paths,
    notice: Option<(String, Color32)>,
    tab: Tab,
    sel_profile: usize,
    sel_layer: usize,
    capture: Option<Capture>,
    dirty_since: Option<Instant>,
    saved_at: Option<Instant>,
    status: Status,
    /// Per-bind macro text buffers keyed by (profile, layer, bind).
    macro_text: std::collections::HashMap<(usize, usize, usize), (String, Option<String>)>,
    exes_text: (usize, String),
}

pub fn run(shared: Arc<Shared>, tx: SyncSender<Msg>, config: Config, paths: Paths, load_error: Option<String>) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PadForge")
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([780.0, 520.0]),
        ..Default::default()
    };
    let app = GuiApp {
        shared,
        tx,
        exes_text: (usize::MAX, String::new()),
        config,
        paths,
        notice: load_error.map(|e| (e, ERR)),
        tab: Tab::Bindings,
        sel_profile: 0,
        sel_layer: 0,
        capture: None,
        dirty_since: None,
        saved_at: None,
        status: Status::default(),
        macro_text: Default::default(),
    };
    let _ = eframe::run_native("PadForge", options, Box::new(|_cc| Ok(Box::new(app))));
}

fn open_url(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (op, target) = (w("open"), w(url));
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

impl GuiApp {
    fn changed(&mut self) {
        self.config.normalize();
        let _ = self.tx.try_send(Msg::Config(Box::new(self.config.clone())));
        self.dirty_since = Some(Instant::now());
    }

    fn kind(&self) -> ControllerKind {
        self.config.settings.controller
    }

    fn start_capture(&mut self, target: CaptureTarget) {
        self.shared.captured.store(0, Relaxed);
        self.shared.capture_next.store(true, Relaxed);
        self.capture = Some(Capture { target, started: Instant::now() });
    }

    fn poll_capture(&mut self, ctx: &egui::Context) {
        let Some(cap) = &self.capture else { return };
        let got = self.shared.captured.swap(0, Relaxed);
        if got != 0 {
            let name = keys::input_name((got - 1) as u16);
            match cap.target {
                CaptureTarget::Bind { profile, layer, bind } => {
                    if let Some(b) = self.config.profiles.get_mut(profile).and_then(|p| p.layers.get_mut(layer)).and_then(|l| l.binds.get_mut(bind)) {
                        b.input = name;
                    }
                }
                CaptureTarget::Toggle => self.config.settings.toggle_hotkey = name,
            }
            self.capture = None;
            self.changed();
        } else if cap.started.elapsed() > Duration::from_secs(6) {
            self.shared.capture_next.store(false, Relaxed);
            self.capture = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.dirty_since {
            let wait = Duration::from_millis(600);
            if t.elapsed() >= wait {
                match save_config(&self.paths.config, &self.config) {
                    Ok(()) => self.saved_at = Some(Instant::now()),
                    Err(e) => self.notice = Some((format!("Could not save config: {e}"), ERR)),
                }
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(wait - t.elapsed());
            }
        }
    }

    // ----------------------------------------------------------- top bar --

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let st = self.status.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("PadForge").size(22.0).strong().color(ACCENT));
            ui.add_space(12.0);
            let (txt, col) = if st.active {
                ("EMULATING", ON)
            } else if st.user_enabled {
                ("ENABLED - PAUSED", WARN)
            } else {
                ("OFF", OFF)
            };
            let btn = egui::Button::new(RichText::new(txt).strong().color(Color32::WHITE)).fill(col.gamma_multiply(0.75)).min_size(Vec2::new(170.0, 30.0));
            if ui.add(btn).on_hover_text(format!("Toggle emulation (hotkey: {})", self.config.settings.toggle_hotkey)).clicked() {
                let _ = self.tx.try_send(Msg::SetEnabled(!st.user_enabled));
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                let pad = match (st.pad_connected, st.controller) {
                    (true, ControllerKind::Xbox360) => RichText::new("Virtual Xbox 360 controller connected").color(ON),
                    (true, ControllerKind::Ds4) => RichText::new("Virtual DualShock 4 controller connected").color(ON),
                    (false, _) => RichText::new("Virtual controller not connected").color(if st.pad_error.is_some() { ERR } else { OFF }),
                };
                ui.label(pad);
                let mut line = format!("Profile: {}", st.profile);
                if !st.foreground_exe.is_empty() {
                    line += &format!("   |   Last app: {}", st.foreground_exe);
                }
                if st.active_layers.len() > 1 {
                    line += &format!("   |   Layers: {}", st.active_layers.join(" + "));
                }
                ui.label(RichText::new(line).small());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let s = &self.shared.stats;
                let last = s.last_latency_us.load(Relaxed);
                let max = s.max_latency_us.load(Relaxed);
                let resp = ui.label(RichText::new(format!("latency {last} µs (max {max})\nevents {}  reports {}", s.events.load(Relaxed), s.reports.load(Relaxed))).small().monospace());
                if resp.on_hover_text("Internal latency: physical event to report submitted to the driver. Click to reset max.").clicked() {
                    s.max_latency_us.store(0, Relaxed);
                }
            });
        });
        if let Some(r) = &st.suspended_reason {
            if st.user_enabled {
                ui.label(RichText::new(format!("Paused: {r}")).color(WARN).small());
            }
        }
        if let Some(e) = &st.pad_error {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).color(ERR));
                if e.contains("ViGEmBus") && ui.button("Download ViGEmBus").clicked() {
                    open_url(VIGEM_URL);
                }
            });
        }
        if !st.config_errors.is_empty() {
            ui.collapsing(RichText::new(format!("{} config issue(s)", st.config_errors.len())).color(WARN), |ui| {
                for e in &st.config_errors {
                    ui.label(RichText::new(e).small());
                }
            });
        }
        let mut clear = false;
        if let Some((n, c)) = &self.notice {
            ui.horizontal(|ui| {
                ui.label(RichText::new(n).color(*c));
                clear = ui.small_button("x").clicked();
            });
        }
        if clear {
            self.notice = None;
        }
    }

    // ------------------------------------------------------- left panel --

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Profiles");
        ui.add_space(4.0);
        let mut select = None;
        for (i, p) in self.config.profiles.iter().enumerate() {
            let mut label = p.name.clone();
            if i == 0 {
                label += "  (fallback)";
            }
            let live = self.status.profile == p.name;
            let txt = if live { RichText::new(format!("> {label}  (active)")).color(ON) } else { RichText::new(label) };
            if ui.selectable_label(i == self.sel_profile, txt).clicked() {
                select = Some(i);
            }
        }
        if let Some(i) = select {
            self.sel_profile = i;
            self.sel_layer = 0;
        }
        ui.horizontal(|ui| {
            if ui.button("+ New").clicked() {
                let p = Profile { name: format!("Profile {}", self.config.profiles.len() + 1), ..Default::default() };
                self.config.profiles.push(p);
                self.sel_profile = self.config.profiles.len() - 1;
                self.sel_layer = 0;
                self.changed();
            }
            if ui.button("Duplicate").clicked() {
                let mut p = self.config.profiles[self.sel_profile].clone();
                p.name += " copy";
                p.executables.clear();
                self.config.profiles.push(p);
                self.sel_profile = self.config.profiles.len() - 1;
                self.changed();
            }
            let can_del = self.config.profiles.len() > 1;
            if ui.add_enabled(can_del, egui::Button::new("Delete")).clicked() {
                self.config.profiles.remove(self.sel_profile);
                self.sel_profile = self.sel_profile.saturating_sub(1);
                self.sel_layer = 0;
                self.macro_text.clear();
                self.changed();
            }
        });
        ui.add_space(12.0);
        ui.separator();
        ui.heading("Live controller");
        let pad = self.status.pad;
        controller_view(ui, &pad, self.kind());
        if !self.status.active {
            ui.label(RichText::new("Shows output while a game is focused and emulation is on.").small().weak());
        } else {
            ui.label(RichText::new(format!("Mouse source: {}", self.status.mouse_source)).small().weak());
        }
    }

    // --------------------------------------------------------- bindings --

    fn bindings_tab(&mut self, ui: &mut egui::Ui) {
        let pi = self.sel_profile.min(self.config.profiles.len() - 1);
        let kind = self.kind();
        let mut changed = false;

        egui::Grid::new("profile_meta").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            changed |= ui.add(egui::TextEdit::singleline(&mut self.config.profiles[pi].name).desired_width(260.0)).changed();
            ui.end_row();
            ui.label("Games").on_hover_text("Executable names that auto-select this profile, comma separated");
            if self.exes_text.0 != pi {
                self.exes_text = (pi, self.config.profiles[pi].executables.join(", "));
            }
            ui.horizontal(|ui| {
                if ui.add(egui::TextEdit::singleline(&mut self.exes_text.1).hint_text("e.g. eldenring.exe, hollow_knight.exe").desired_width(360.0)).changed() {
                    self.config.profiles[pi].executables = self.exes_text.1.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect();
                    changed = true;
                }
                let fg = self.status.foreground_exe.clone();
                if !fg.is_empty() && !self.config.profiles[pi].executables.contains(&fg) && ui.button(format!("+ {fg}")).on_hover_text("Add the last focused app").clicked() {
                    self.config.profiles[pi].executables.push(fg);
                    self.exes_text.0 = usize::MAX;
                    changed = true;
                }
            });
            ui.end_row();
        });
        ui.add_space(6.0);

        // Layer selector.
        ui.horizontal(|ui| {
            ui.label("Layer:");
            let n = self.config.profiles[pi].layers.len();
            for li in 0..n {
                let name = self.config.profiles[pi].layers[li].name.clone();
                if ui.selectable_label(self.sel_layer == li, if li == 0 { format!("{name} (base)") } else { name }).clicked() {
                    self.sel_layer = li;
                }
            }
            if ui.button("+ Layer").on_hover_text("Layers are activated by a 'Layer' action; unbound keys fall through to lower layers").clicked() {
                self.config.profiles[pi].layers.push(Layer { name: format!("Layer {n}"), binds: vec![] });
                self.sel_layer = n;
                changed = true;
            }
        });
        let li = self.sel_layer.min(self.config.profiles[pi].layers.len() - 1);
        if li > 0 {
            ui.horizontal(|ui| {
                ui.label("Layer name");
                let old = self.config.profiles[pi].layers[li].name.clone();
                if ui.text_edit_singleline(&mut self.config.profiles[pi].layers[li].name).changed() {
                    let new = self.config.profiles[pi].layers[li].name.clone();
                    // Keep layer actions pointing at the renamed layer.
                    for l in &mut self.config.profiles[pi].layers {
                        for b in &mut l.binds {
                            if let Action::Layer { layer, .. } = &mut b.action {
                                if *layer == old {
                                    *layer = new.clone();
                                }
                            }
                        }
                    }
                    changed = true;
                }
                if ui.button("Delete layer").clicked() {
                    self.config.profiles[pi].layers.remove(li);
                    self.sel_layer = 0;
                    self.macro_text.clear();
                    changed = true;
                }
            });
        }
        let li = self.sel_layer.min(self.config.profiles[pi].layers.len() - 1);
        ui.separator();

        let layer_names: Vec<String> = self.config.profiles[pi].layers.iter().skip(1).map(|l| l.name.clone()).collect();
        let capturing = self.capture.as_ref().map(|c| c.target);
        let mut start_capture = None;
        let mut remove = None;

        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 36.0).show(ui, |ui| {
            egui::Grid::new(("binds", pi, li)).num_columns(5).striped(true).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new("Input").strong());
                ui.label(RichText::new("Action").strong());
                ui.label(RichText::new("Settings").strong());
                ui.label(RichText::new("Block").strong()).on_hover_text("Hide the original key from the game");
                ui.label("");
                ui.end_row();

                let nb = self.config.profiles[pi].layers[li].binds.len();
                for bi in 0..nb {
                    let target = CaptureTarget::Bind { profile: pi, layer: li, bind: bi };
                    let bind = &mut self.config.profiles[pi].layers[li].binds[bi];
                    let label = if capturing == Some(target) {
                        RichText::new("Press a key...").color(WARN)
                    } else if bind.input.is_empty() {
                        RichText::new("(click to set)").weak()
                    } else {
                        RichText::new(&bind.input).monospace().strong()
                    };
                    if ui.add(egui::Button::new(label).min_size(Vec2::new(120.0, 0.0))).on_hover_text("Click, then press the key or mouse button").clicked() {
                        start_capture = Some(target);
                    }
                    changed |= action_kind_combo(ui, &mut bind.action, (pi, li, bi), &layer_names);
                    ui.horizontal(|ui| {
                        changed |= action_settings(ui, &mut bind.action, kind, (pi, li, bi), &layer_names, &mut self.macro_text);
                    });
                    changed |= ui.checkbox(&mut bind.block, "").changed();
                    if ui.small_button("Del").on_hover_text("Remove binding").clicked() {
                        remove = Some(bi);
                    }
                    ui.end_row();
                }
            });
        });
        if ui.button("+ Add binding").clicked() {
            let binds = &mut self.config.profiles[pi].layers[li].binds;
            binds.push(Bind { input: String::new(), action: Action::Button { button: Control::A, mode: ButtonMode::Hold, turbo_hz: 12.0 }, block: true });
            start_capture = Some(CaptureTarget::Bind { profile: pi, layer: li, bind: binds.len() - 1 });
            changed = true;
        }
        if let Some(bi) = remove {
            self.config.profiles[pi].layers[li].binds.remove(bi);
            self.macro_text.clear();
            changed = true;
        }
        if let Some(t) = start_capture {
            self.start_capture(t);
        }
        if changed {
            self.changed();
        }
    }

    // ------------------------------------------------------------ mouse --

    fn mouse_tab(&mut self, ui: &mut egui::Ui) {
        let pi = self.sel_profile.min(self.config.profiles.len() - 1);
        let mut changed = false;
        let live = if self.status.profile == self.config.profiles[pi].name { Some(self.status.pad) } else { None };
        let m = &mut self.config.profiles[pi].mouse;
        ui.horizontal(|ui| {
            ui.label("Mouse drives");
            changed |= ui.selectable_value(&mut m.target, StickTarget::Right, "Right stick").changed();
            changed |= ui.selectable_value(&mut m.target, StickTarget::Left, "Left stick").changed();
            changed |= ui.selectable_value(&mut m.target, StickTarget::None, "Nothing (mouse passes through)").changed();
        });
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            egui::Grid::new("mouse_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Sensitivity").on_hover_text("Stick deflection per mouse speed. Higher DPI = lower value.");
                changed |= ui.add(egui::Slider::new(&mut m.sensitivity, 0.01..=2.0).logarithmic(true)).changed();
                ui.end_row();
                ui.label("Vertical ratio");
                changed |= ui.add(egui::Slider::new(&mut m.y_ratio, 0.25..=3.0)).changed();
                ui.end_row();
                ui.label("Invert Y");
                changed |= ui.checkbox(&mut m.invert_y, "").changed();
                ui.end_row();
                ui.label("Curve exponent").on_hover_text("<1 boosts small movements, >1 gives more precision near centre");
                changed |= ui.add(egui::Slider::new(&mut m.exponent, 0.3..=2.5)).changed();
                ui.end_row();
                ui.label("Anti-deadzone").on_hover_text("Match the game's inner stick dead zone so tiny mouse moves register. Typical: 0.10-0.25");
                changed |= ui.add(egui::Slider::new(&mut m.anti_deadzone, 0.0..=0.6)).changed();
                ui.end_row();
                ui.label("Smoothing (ms)").on_hover_text("Velocity averaging window. Lower = snappier, higher = smoother");
                changed |= ui.add(egui::Slider::new(&mut m.smoothing_ms, 1.0..=40.0)).changed();
                ui.end_row();
                ui.label("Release (ms)").on_hover_text("How quickly the stick recentres after the mouse stops");
                changed |= ui.add(egui::Slider::new(&mut m.release_ms, 0.0..=120.0)).changed();
                ui.end_row();
            });
            if ui.button("Reset mouse settings").clicked() {
                *m = MouseConfig { target: m.target, ..Default::default() };
                changed = true;
            }
            ui.add_space(8.0);
            ui.label(RichText::new(
                "Tuning tips:\n- Raise anti-deadzone until slow mouse movement starts turning the camera immediately.\n- Set sensitivity so a fast flick just reaches full stick.\n- Max turn speed is capped by the game's own controller sensitivity, so raise that in-game too.",
            ).small());

            let ui = &mut cols[1];
            ui.label(RichText::new("Response: mouse speed vs stick deflection").small());
            curve_plot(ui, m, live);
        });
        if changed {
            self.changed();
        }
    }

    // --------------------------------------------------------- settings --

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut capture_toggle = false;
        let capturing_toggle = self.capture.as_ref().map(|c| c.target) == Some(CaptureTarget::Toggle);
        let s = &mut self.config.settings;
        egui::Grid::new("settings").num_columns(2).spacing([14.0, 10.0]).striped(true).show(ui, |ui| {
            ui.label("Toggle hotkey");
            let t = if capturing_toggle { RichText::new("Press a key...").color(WARN) } else { RichText::new(&s.toggle_hotkey).monospace().strong() };
            if ui.button(t).on_hover_text("Always works, even in-game. Never sent to the game.").clicked() {
                capture_toggle = true;
            }
            ui.end_row();

            ui.label("Virtual controller");
            ui.horizontal(|ui| {
                changed |= ui.selectable_value(&mut s.controller, ControllerKind::Xbox360, "Xbox 360 (XInput)").changed();
                changed |= ui.selectable_value(&mut s.controller, ControllerKind::Ds4, "DualShock 4").changed();
            });
            ui.end_row();

            ui.label("Enable on startup");
            changed |= ui.checkbox(&mut s.start_enabled, "").changed();
            ui.end_row();

            ui.label("Keep controller connected").on_hover_text("Stay plugged in while disabled so games don't see disconnects");
            changed |= ui.checkbox(&mut s.keep_connected, "").changed();
            ui.end_row();

            ui.label("Only emulate in listed games").on_hover_text("Pass input through when the focused app isn't listed in any profile");
            changed |= ui.checkbox(&mut s.suspend_when_unmatched, "").changed();
            ui.end_row();

            ui.label("Opposite directions (A+D)");
            ui.horizontal(|ui| {
                changed |= ui.selectable_value(&mut s.socd, Socd::LastWins, "Last pressed wins").changed();
                changed |= ui.selectable_value(&mut s.socd, Socd::Neutral, "Cancel out").changed();
            });
            ui.end_row();

            ui.label("Mouse delta source").on_hover_text("Auto uses unaccelerated Raw Input when available, otherwise the hook");
            ui.horizontal(|ui| {
                changed |= ui.selectable_value(&mut s.mouse_source, MouseSource::Auto, "Auto").changed();
                changed |= ui.selectable_value(&mut s.mouse_source, MouseSource::Raw, "Raw Input").changed();
                changed |= ui.selectable_value(&mut s.mouse_source, MouseSource::Hook, "Hook").changed();
            });
            ui.end_row();

            ui.label("Re-centre frozen cursor");
            changed |= ui.checkbox(&mut s.recenter_cursor, "").changed();
            ui.end_row();

            ui.label("Timed update rate (Hz)").on_hover_text("For mouse decay / turbo / macros. Key presses are always forwarded instantly.");
            changed |= ui.add(egui::Slider::new(&mut s.output_rate_hz, 125..=1000)).changed();
            ui.end_row();

            ui.label("Wheel press length (ms)");
            changed |= ui.add(egui::Slider::new(&mut s.wheel_press_ms, 10..=200)).changed();
            ui.end_row();

            ui.label("Above-normal priority").on_hover_text("Applies on next start");
            changed |= ui.checkbox(&mut s.high_priority, "").changed();
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Config file: {}", self.paths.config.display())).small());
            if ui.small_button("Open folder").clicked() {
                if let Some(dir) = self.paths.config.parent() {
                    open_url(&dir.display().to_string());
                }
            }
            if ui.small_button("Reload from disk").clicked() {
                let (c, err) = load_config(&self.paths.config);
                self.config = c;
                self.notice = err.map(|e| (e, ERR));
                self.macro_text.clear();
                self.exes_text.0 = usize::MAX;
                self.sel_profile = 0;
                self.sel_layer = 0;
                changed = true;
            }
        });
        if capture_toggle {
            self.start_capture(CaptureTarget::Toggle);
        }
        if changed {
            self.changed();
        }
    }

    fn help_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Quick start");
            ui.label("1. Install the ViGEmBus driver (one time) and restart PadForge.");
            if ui.link(VIGEM_URL).clicked() {
                open_url(VIGEM_URL);
            }
            ui.label(format!(
                "2. Start your game, press {} to enable emulation. The game now sees an Xbox 360 or DualShock 4 controller.",
                self.config.settings.toggle_hotkey
            ));
            ui.label("3. Mapped keys and mouse movement are hidden from the game, so it stays in controller mode.");
            ui.label("4. Emulation pauses automatically while this window is focused, so you can use your mouse here.");
            ui.add_space(8.0);
            ui.heading("Actions");
            for (n, d) in [
                ("Button", "Hold, toggle (latch) or turbo a controller button. LT/RT act as fully pressed triggers."),
                ("Trigger", "Analog trigger at a fixed value, e.g. 0.5 for a half press."),
                ("Stick", "Push a stick in a direction; held keys combine. Diagonals are normalised."),
                ("Stick scale", "Scale a stick while held, e.g. 0.5 = walk."),
                ("Layer", "Switch to another set of bindings while held (or toggled). Unbound keys fall through."),
                ("Macro", "Sequence like: tap a, wait 50, press rb, wait 200, release rb. Repeat loops while held."),
                ("Disable", "Swallow the key so the game never sees it."),
            ] {
                ui.label(RichText::new(n).strong());
                ui.label(d);
            }
            ui.add_space(8.0);
            ui.heading("Fair play");
            ui.label("Using controller emulation in online/competitive games may break their rules and get your account banned. PadForge is meant for single-player games, controller-only games, accessibility, and games that permit it.");
        });
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.status = self.shared.status_snapshot();
        self.poll_capture(&ctx);
        self.autosave(&ctx);
        if self.status.active {
            ctx.request_repaint_after(Duration::from_millis(50)); // live view
        } else {
            ctx.request_repaint_after(Duration::from_millis(500)); // status changes
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::left("left").resizable(false).exact_size(230.0).show(ui, |ui| {
            ui.add_space(6.0);
            self.left_panel(ui);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (t, n) in [(Tab::Bindings, "Bindings"), (Tab::Mouse, "Mouse to Stick"), (Tab::Settings, "Settings"), (Tab::Help, "Help")] {
                    ui.selectable_value(&mut self.tab, t, RichText::new(n).size(15.0));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.dirty_since.is_some() {
                        ui.label(RichText::new("saving...").small().weak());
                    } else if self.saved_at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
                        ui.label(RichText::new("saved").small().color(ON));
                    }
                });
            });
            ui.separator();
            match self.tab {
                Tab::Bindings => self.bindings_tab(ui),
                Tab::Mouse => self.mouse_tab(ui),
                Tab::Settings => self.settings_tab(ui),
                Tab::Help => self.help_tab(ui),
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shared.capture_next.store(false, Relaxed);
        if self.dirty_since.is_some() {
            let _ = save_config(&self.paths.config, &self.config);
        }
    }
}

// ----------------------------------------------------------- bind widgets --

const KINDS: [&str; 7] = ["Button", "Trigger", "Stick", "Stick scale", "Layer", "Macro", "Disable"];

fn action_kind_combo(ui: &mut egui::Ui, a: &mut Action, id: (usize, usize, usize), layers: &[String]) -> bool {
    let mut kind = a.kind_label();
    let before = kind;
    egui::ComboBox::from_id_salt(("kind", id)).width(110.0).selected_text(kind).show_ui(ui, |ui| {
        for k in KINDS {
            ui.selectable_value(&mut kind, k, k);
        }
    });
    if kind == before {
        return false;
    }
    *a = match kind {
        "Button" => Action::Button { button: Control::A, mode: ButtonMode::Hold, turbo_hz: 12.0 },
        "Trigger" => Action::Trigger { trigger: Side::Right, value: 1.0 },
        "Stick" => Action::Stick { stick: Side::Left, x: 0.0, y: 1.0 },
        "Stick scale" => Action::StickScale { stick: Side::Left, factor: 0.5 },
        "Layer" => Action::Layer { layer: layers.first().cloned().unwrap_or_else(|| "Layer 1".into()), mode: LayerMode::Hold },
        "Macro" => Action::Macro { steps: vec![MacroStep::Tap(Control::A)], repeat: false, tap_ms: 30 },
        _ => Action::Disable,
    };
    true
}

fn side_combo(ui: &mut egui::Ui, side: &mut Side, salt: impl std::hash::Hash + std::fmt::Debug, what: &str) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(salt).width(90.0).selected_text(match side {
        Side::Left => format!("Left {what}"),
        Side::Right => format!("Right {what}"),
    }).show_ui(ui, |ui| {
        changed |= ui.selectable_value(side, Side::Left, format!("Left {what}")).changed();
        changed |= ui.selectable_value(side, Side::Right, format!("Right {what}")).changed();
    });
    changed
}

const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
const DIRS: [(&str, f32, f32); 8] = [
    ("Up", 0.0, 1.0),
    ("Down", 0.0, -1.0),
    ("Left", -1.0, 0.0),
    ("Right", 1.0, 0.0),
    ("Up-Left", -D, D),
    ("Up-Right", D, D),
    ("Down-Left", -D, -D),
    ("Down-Right", D, -D),
];

fn action_settings(
    ui: &mut egui::Ui,
    a: &mut Action,
    kind: ControllerKind,
    id: (usize, usize, usize),
    layers: &[String],
    macro_text: &mut std::collections::HashMap<(usize, usize, usize), (String, Option<String>)>,
) -> bool {
    let mut changed = false;
    match a {
        Action::Button { button, mode, turbo_hz } => {
            egui::ComboBox::from_id_salt(("btn", id)).width(110.0).selected_text(button.label(kind)).show_ui(ui, |ui| {
                for c in Control::ALL {
                    if kind == ControllerKind::Xbox360 && c == Control::Touchpad {
                        continue;
                    }
                    changed |= ui.selectable_value(button, c, c.label(kind)).changed();
                }
            });
            egui::ComboBox::from_id_salt(("mode", id)).width(70.0).selected_text(format!("{mode:?}")).show_ui(ui, |ui| {
                changed |= ui.selectable_value(mode, ButtonMode::Hold, "Hold").changed();
                changed |= ui.selectable_value(mode, ButtonMode::Toggle, "Toggle").changed();
                changed |= ui.selectable_value(mode, ButtonMode::Turbo, "Turbo").changed();
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
            egui::ComboBox::from_id_salt(("dir", id)).width(95.0).selected_text(cur).show_ui(ui, |ui| {
                for (n, dx, dy) in DIRS {
                    if ui.selectable_label(cur == n, n).clicked() {
                        *x = dx;
                        *y = dy;
                        changed = true;
                    }
                }
            });
            ui.label("x");
            changed |= ui.add(egui::DragValue::new(x).range(-1.0..=1.0).speed(0.01).fixed_decimals(2)).changed();
            ui.label("y");
            changed |= ui.add(egui::DragValue::new(y).range(-1.0..=1.0).speed(0.01).fixed_decimals(2)).changed();
        }
        Action::StickScale { stick, factor } => {
            changed |= side_combo(ui, stick, ("scale", id), "stick");
            changed |= ui.add(egui::Slider::new(factor, 0.0..=1.0).fixed_decimals(2).text("×")).changed();
        }
        Action::Layer { layer, mode } => {
            egui::ComboBox::from_id_salt(("layer", id)).width(110.0).selected_text(layer.as_str()).show_ui(ui, |ui| {
                for n in layers {
                    changed |= ui.selectable_value(layer, n.clone(), n).changed();
                }
            });
            egui::ComboBox::from_id_salt(("lmode", id)).width(70.0).selected_text(format!("{mode:?}")).show_ui(ui, |ui| {
                changed |= ui.selectable_value(mode, LayerMode::Hold, "Hold").changed();
                changed |= ui.selectable_value(mode, LayerMode::Toggle, "Toggle").changed();
            });
            if layers.is_empty() {
                ui.label(RichText::new("add a layer first").color(WARN).small());
            }
        }
        Action::Macro { steps, repeat, tap_ms } => {
            let entry = macro_text.entry(id).or_insert_with(|| (macro_to_text(steps), None));
            let resp = ui.add(egui::TextEdit::singleline(&mut entry.0).desired_width(230.0).hint_text("tap a, wait 50, tap b"));
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
                ui.label(RichText::new("(!)").color(ERR)).on_hover_text(e);
            }
            changed |= ui.checkbox(repeat, "repeat").on_hover_text("Loop while the key is held").changed();
            changed |= ui.add(egui::DragValue::new(tap_ms).range(1..=1000).suffix(" ms tap")).changed();
        }
        Action::Disable => {
            ui.label(RichText::new("key is swallowed").weak());
        }
    }
    changed
}

// ------------------------------------------------------------- drawing --

fn stick(painter: &egui::Painter, c: egui::Pos2, r: f32, x: f32, y: f32, pressed: bool) {
    painter.circle_filled(c, r, Color32::from_gray(38));
    painter.circle_stroke(c, r, Stroke::new(1.5, Color32::from_gray(90)));
    let p = c + Vec2::new(x * r * 0.8, -y * r * 0.8);
    painter.line_segment([c, p], Stroke::new(1.0, ACCENT.gamma_multiply(0.5)));
    painter.circle_filled(p, r * 0.35, if pressed { ON } else { ACCENT });
}

fn button_dot(painter: &egui::Painter, c: egui::Pos2, r: f32, on: bool, label: &str) {
    painter.circle_filled(c, r, if on { ON } else { Color32::from_gray(55) });
    painter.text(c, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(r * 1.0), Color32::WHITE);
}

fn controller_view(ui: &mut egui::Ui, s: &PadState, kind: ControllerKind) {
    let (resp, p) = ui.allocate_painter(Vec2::new(210.0, 150.0), egui::Sense::hover());
    let r = resp.rect;
    p.rect_filled(r, 10.0, Color32::from_gray(24));
    let (lt, rt) = s.triggers();
    // Triggers as bars.
    let bar = |x0: f32, v: f32| {
        let full = egui::Rect::from_min_size(egui::pos2(x0, r.top() + 6.0), Vec2::new(50.0, 8.0));
        p.rect_filled(full, 3.0, Color32::from_gray(50));
        let mut f = full;
        f.set_width(50.0 * v);
        p.rect_filled(f, 3.0, WARN);
    };
    bar(r.left() + 12.0, lt);
    bar(r.right() - 62.0, rt);
    let lb = s.pressed(Control::Lb);
    let rb = s.pressed(Control::Rb);
    p.rect_filled(egui::Rect::from_min_size(egui::pos2(r.left() + 12.0, r.top() + 18.0), Vec2::new(50.0, 7.0)), 3.0, if lb { ON } else { Color32::from_gray(55) });
    p.rect_filled(egui::Rect::from_min_size(egui::pos2(r.right() - 62.0, r.top() + 18.0), Vec2::new(50.0, 7.0)), 3.0, if rb { ON } else { Color32::from_gray(55) });

    stick(&p, egui::pos2(r.left() + 45.0, r.top() + 62.0), 24.0, s.lx, s.ly, s.pressed(Control::Ls));
    stick(&p, egui::pos2(r.left() + 135.0, r.top() + 112.0), 24.0, s.rx, s.ry, s.pressed(Control::Rs));

    // Face buttons.
    let fc = egui::pos2(r.right() - 38.0, r.top() + 62.0);
    let (a, b, x, y) = if kind == ControllerKind::Ds4 { ("X", "O", "[]", "/\\") } else { ("A", "B", "X", "Y") };
    button_dot(&p, fc + Vec2::new(0.0, 16.0), 9.0, s.pressed(Control::A), a);
    button_dot(&p, fc + Vec2::new(16.0, 0.0), 9.0, s.pressed(Control::B), b);
    button_dot(&p, fc + Vec2::new(-16.0, 0.0), 9.0, s.pressed(Control::X), x);
    button_dot(&p, fc + Vec2::new(0.0, -16.0), 9.0, s.pressed(Control::Y), y);

    // D-pad.
    let dc = egui::pos2(r.left() + 75.0, r.top() + 112.0);
    for (c, off) in [(Control::DpadUp, Vec2::new(0.0, -12.0)), (Control::DpadDown, Vec2::new(0.0, 12.0)), (Control::DpadLeft, Vec2::new(-12.0, 0.0)), (Control::DpadRight, Vec2::new(12.0, 0.0))] {
        p.rect_filled(egui::Rect::from_center_size(dc + off, Vec2::splat(10.0)), 2.0, if s.pressed(c) { ON } else { Color32::from_gray(55) });
    }
    // Back / Guide / Start.
    let mc = egui::pos2(r.center().x, r.top() + 50.0);
    button_dot(&p, mc + Vec2::new(-20.0, 0.0), 5.0, s.pressed(Control::Back), "");
    button_dot(&p, mc, 7.0, s.pressed(Control::Guide), "");
    button_dot(&p, mc + Vec2::new(20.0, 0.0), 5.0, s.pressed(Control::Start), "");
}

fn curve_plot(ui: &mut egui::Ui, m: &MouseConfig, live: Option<PadState>) {
    let size = Vec2::new(ui.available_width().min(380.0), 220.0);
    let (resp, p) = ui.allocate_painter(size, egui::Sense::hover());
    let r = resp.rect.shrink(8.0);
    p.rect_filled(resp.rect, 6.0, Color32::from_gray(24));
    for i in 0..=4 {
        let y = r.bottom() - r.height() * i as f32 / 4.0;
        p.line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], Stroke::new(0.5, Color32::from_gray(50)));
    }
    // x axis: mouse speed in counts/ms up to the speed that saturates the stick (×1.3).
    let sat = if m.sensitivity > 0.0 { 1.0 / m.sensitivity } else { 10.0 };
    let max_v = (sat * 1.3).max(0.5);
    let pts: Vec<egui::Pos2> = (0..=120)
        .map(|i| {
            let v = max_v * i as f32 / 120.0;
            let (o, _) = crate::mouse_stick::shape(v, 0.0, m);
            egui::pos2(r.left() + r.width() * i as f32 / 120.0, r.bottom() - r.height() * o.abs())
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, ACCENT)));
    p.text(r.left_top(), egui::Align2::LEFT_TOP, "1.0", egui::FontId::monospace(10.0), Color32::GRAY);
    p.text(r.right_bottom(), egui::Align2::RIGHT_BOTTOM, format!("{max_v:.1} counts/ms"), egui::FontId::monospace(10.0), Color32::GRAY);
    if m.anti_deadzone > 0.0 {
        let y = r.bottom() - r.height() * m.anti_deadzone;
        p.line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], Stroke::new(1.0, WARN.gamma_multiply(0.6)));
        p.text(egui::pos2(r.right(), y - 2.0), egui::Align2::RIGHT_BOTTOM, "anti-deadzone", egui::FontId::proportional(10.0), WARN);
    }
    if let Some(s) = live {
        let (x, y) = match m.target {
            StickTarget::Left => (s.lx, s.ly),
            _ => (s.rx, s.ry),
        };
        let mag = (x * x + y * y).sqrt().min(1.0);
        let yy = r.bottom() - r.height() * mag;
        p.circle_filled(egui::pos2(r.left() + 6.0, yy), 5.0, ON);
    }
}
