//! PadForge window (egui). Edits apply live to the engine and auto-save.
//!
//! Efficiency rules: repaint only on input or engine events; the 20 Hz live
//! view runs only while emulating AND the live controller is on screen;
//! nothing at all is rendered while hidden in the tray. All graphics are
//! vector-drawn (no images or textures to load).

mod editor;
mod icons;
mod pages;
mod preview;
mod theme;

use super::app::{save_config, Paths};
use super::games::{self, RunningApp};
use super::tray;
use super::vpad::VIGEM_URL;
use crate::config::*;
use crate::engine::Msg;
use crate::keys::{self, InputId};
use crate::pad::{Control, ControllerKind, Side};
use crate::shared::{Shared, Status};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Margin, Rect, RichText, Sense, Stroke, StrokeKind};
use icons::{Icon, Style};
use preview::{Device, Elem, PadEl};
use std::sync::atomic::Ordering::*;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant};
use theme::Pal;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Mapping,
    Mouse,
    Settings,
    Help,
}

#[derive(PartialEq, Clone, Copy)]
enum MapMode {
    Preview,
    List,
}

#[derive(Clone, Copy, PartialEq)]
enum CaptureTarget {
    Bind { profile: usize, layer: usize, bind: usize },
    Toggle,
    /// "Detect key": select whatever key is pressed next in the preview.
    Select,
}

struct Capture {
    target: CaptureTarget,
    started: Instant,
}

struct Picker {
    apps: Vec<RunningApp>,
    filter: String,
}

pub struct GuiApp {
    shared: Arc<Shared>,
    tx: SyncSender<Msg>,
    config: Config,
    paths: Paths,
    /// (message, is_error)
    notice: Option<(String, bool)>,
    page: Page,
    device: Device,
    mode: MapMode,
    sel_profile: usize,
    sel_layer: usize,
    selected: Option<Elem>,
    /// Elements of the list row under the pointer (last frame), highlighted
    /// in the preview.
    row_hover: Vec<Elem>,
    /// Preview element under the pointer; its list rows light up.
    preview_hover: Option<Elem>,
    capture: Option<Capture>,
    dirty_since: Option<Instant>,
    saved_at: Option<Instant>,
    status: Status,
    macro_text: editor::MacroText,
    exes_text: (usize, String),
    told_about_tray: bool,
    /// "Start in tray": eframe always shows the window after the first
    /// painted frame, so we hide it right after that.
    hide_after_first_frame: bool,
    pal: Pal,
    applied_theme: Option<Theme>,
    picker: Option<Picker>,
}

pub fn run(shared: Arc<Shared>, tx: SyncSender<Msg>, config: Config, paths: Paths, load_error: Option<String>, start_hidden: bool) {
    let start_hidden = start_hidden || config.settings.start_in_tray;
    let icon = egui::IconData { rgba: crate::icon::render_rgba(64, crate::icon::COLOR_BRAND), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PadForge")
            .with_icon(icon)
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([1040.0, 680.0]),
        ..Default::default()
    };
    let pal = theme::pal(config.settings.theme);
    let app = GuiApp {
        shared,
        tx,
        exes_text: (usize::MAX, String::new()),
        config,
        paths,
        notice: load_error.map(|e| (e, true)),
        page: Page::Mapping,
        device: Device::Keyboard,
        mode: MapMode::Preview,
        sel_profile: 0,
        sel_layer: 0,
        selected: None,
        row_hover: Vec::new(),
        preview_hover: None,
        capture: None,
        dirty_since: None,
        saved_at: None,
        status: Status::default(),
        macro_text: Default::default(),
        told_about_tray: false,
        hide_after_first_frame: start_hidden,
        pal,
        applied_theme: None,
        picker: None,
    };
    let _ = eframe::run_native(
        "PadForge",
        options,
        Box::new(move |cc| {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = cc.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    app.shared.gui_window.store(w.hwnd.get() as usize, Relaxed);
                }
            }
            theme::fonts(&cc.egui_ctx);
            theme::style(&cc.egui_ctx);
            cc.egui_ctx.set_visuals_of(egui::Theme::Dark, theme::visuals(&theme::pal(Theme::Dark)));
            cc.egui_ctx.set_visuals_of(egui::Theme::Light, theme::visuals(&theme::pal(Theme::Light)));
            // Engine status changes wake the GUI (no polling).
            let ctx = cc.egui_ctx.clone();
            app.shared.add_listener(move || ctx.request_repaint());
            Ok(Box::new(app))
        }),
    );
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

/// File name with its original capitalisation.
fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn input_title(id: InputId) -> (String, &'static str) {
    match id {
        keys::MOUSE_LEFT => ("Left mouse button".into(), "Mouse button"),
        keys::MOUSE_RIGHT => ("Right mouse button".into(), "Mouse button"),
        keys::MOUSE_MIDDLE => ("Wheel click".into(), "Mouse button"),
        keys::MOUSE_X1 => ("Side button X1".into(), "Mouse button (back)"),
        keys::MOUSE_X2 => ("Side button X2".into(), "Mouse button (forward)"),
        keys::WHEEL_UP => ("Wheel up".into(), "Mouse wheel"),
        keys::WHEEL_DOWN => ("Wheel down".into(), "Mouse wheel"),
        keys::WHEEL_LEFT => ("Wheel tilt left".into(), "Mouse wheel"),
        keys::WHEEL_RIGHT => ("Wheel tilt right".into(), "Mouse wheel"),
        id => (format!("Key {}", keys::input_name(id)), "Keyboard key"),
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

    fn pi(&self) -> usize {
        self.sel_profile.min(self.config.profiles.len() - 1)
    }

    fn li(&self) -> usize {
        self.sel_layer.min(self.config.profiles[self.pi()].layers.len() - 1)
    }

    fn layer_names(&self, pi: usize) -> Vec<String> {
        self.config.profiles[pi].layers.iter().skip(1).map(|l| l.name.clone()).collect()
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
            let id = (got - 1) as InputId;
            let name = keys::input_name(id);
            match cap.target {
                CaptureTarget::Bind { profile, layer, bind } => {
                    if let Some(b) = self.config.profiles.get_mut(profile).and_then(|p| p.layers.get_mut(layer)).and_then(|l| l.binds.get_mut(bind)) {
                        b.input = name;
                    }
                }
                CaptureTarget::Toggle => self.config.settings.toggle_hotkey = name,
                CaptureTarget::Select => {
                    self.selected = Some(Elem::Input(id));
                    self.device = if id >= keys::MOUSE_BASE { Device::Mouse } else { Device::Keyboard };
                }
            }
            let was_select = cap.target == CaptureTarget::Select;
            self.capture = None;
            if !was_select {
                self.changed();
            }
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
                    Err(e) => self.notice = Some((format!("Could not save config: {e}"), true)),
                }
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(wait - t.elapsed());
            }
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let t = self.config.settings.theme;
        if self.applied_theme != Some(t) {
            self.applied_theme = Some(t);
            self.pal = theme::pal(t);
            ctx.set_theme(theme::egui_theme(t));
        }
    }

    fn set_theme(&mut self, t: Theme) {
        if self.config.settings.theme != t {
            self.config.settings.theme = t;
            self.changed();
        }
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.shared.gui_hidden.store(true, Relaxed);
        if self.dirty_since.is_some() {
            let _ = save_config(&self.paths.config, &self.config);
            self.dirty_since = None;
        }
    }

    fn launch_game(&mut self, ctx: &egui::Context, pi: usize) {
        let p = &self.config.profiles[pi];
        match games::launch(&p.game_path, &p.launch_args) {
            Ok(()) => {
                self.notice = Some((format!("Launched {}. The \"{}\" profile activates as soon as the game window is focused.", file_name(&p.game_path), p.name), false));
                if self.config.settings.enable_on_launch {
                    let _ = self.tx.try_send(Msg::SetEnabled(true));
                }
                if self.config.settings.hide_on_launch {
                    self.hide_to_tray(ctx);
                }
            }
            Err(e) => self.notice = Some((e, true)),
        }
    }

    // ------------------------------------------------------------ sidebar --

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
            icons::logo(ui.painter(), r, pal.gold_fill, pal.on_gold);
            ui.add_space(2.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(RichText::new("Pad").font(theme::bold(22.0)).color(pal.text));
            ui.label(RichText::new("Forge").font(theme::bold(22.0)).color(pal.gold));
            ui.add_space(6.0);
            ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).small().color(pal.weak));
        });
        ui.add_space(16.0);
        self.status_card(ui);
        ui.add_space(14.0);

        if icons::text_button(ui, &pal, Some(Icon::Plus), "Add game profile", Style::Link).clicked() {
            let p = Profile { name: format!("Game {}", self.config.profiles.len()), ..Default::default() };
            self.config.profiles.push(p);
            self.sel_profile = self.config.profiles.len() - 1;
            self.sel_layer = 0;
            self.selected = None;
            self.page = Page::Mapping;
            self.changed();
        }
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, pal.line);
        ui.add_space(8.0);
        ui.label(RichText::new("PROFILES").font(theme::bold(11.0)).color(pal.weak));
        ui.add_space(2.0);

        let footer = 58.0;
        let mut select = None;
        let mut action: Option<(usize, &str)> = None;
        egui::ScrollArea::vertical().max_height((ui.available_height() - footer).max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
            for i in 0..self.config.profiles.len() {
                let (clicked, act) = self.profile_item(ui, i);
                if clicked {
                    select = Some(i);
                }
                if act.is_some() {
                    action = act.map(|a| (i, a));
                }
            }
        });
        if let Some(i) = select {
            if i != self.sel_profile {
                self.sel_layer = 0;
                self.selected = None;
                self.macro_text.clear();
            }
            self.sel_profile = i;
            self.page = Page::Mapping;
        }
        if let Some((i, a)) = action {
            self.profile_action(ui.ctx(), i, a);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            self.driver_line(ui);
        });
    }

    fn status_card(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let st = &self.status;
        let hk = &self.config.settings.toggle_hotkey;
        let (title, sub, fill, fg, ring) = if st.active {
            ("EMULATING", format!("{} controller live. {hk} turns it off", if st.controller == ControllerKind::Ds4 { "DualShock 4" } else { "Xbox 360" }), pal.gold_fill, pal.on_gold, false)
        } else if st.user_enabled {
            ("PAUSED", st.suspended_reason.clone().unwrap_or_else(|| "Waiting for a game".into()), pal.raised, pal.gold, true)
        } else {
            ("OFF", format!("Click or press {hk} to start"), pal.raised, pal.text, false)
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
        let p = ui.painter();
        let stroke = if ring || resp.hovered() { Stroke::new(1.0, pal.gold) } else { Stroke::NONE };
        p.rect(rect, 12.0, fill, stroke, StrokeKind::Inside);
        // Power glyph.
        let c = pos2(rect.left() + 26.0, rect.center().y);
        let ink = if st.active { pal.on_gold } else if st.user_enabled { pal.gold } else { pal.weak };
        let mut arc = Vec::new();
        for i in 0..=24 {
            let a = -std::f32::consts::FRAC_PI_2 + 0.75 + i as f32 / 24.0 * (std::f32::consts::TAU - 1.5);
            arc.push(c + vec2(a.cos(), a.sin()) * 10.0);
        }
        p.add(egui::Shape::line(arc, Stroke::new(2.0, ink)));
        p.line_segment([c + vec2(0.0, -12.0), c + vec2(0.0, -2.0)], Stroke::new(2.0, ink));
        p.text(pos2(rect.left() + 48.0, rect.center().y - 9.0), Align2::LEFT_CENTER, title, theme::bold(16.0), fg);
        preview::fit_text(p, pos2(rect.left() + 48.0, rect.center().y + 11.0), Align2::LEFT_CENTER, &sub, theme::font(11.5), if st.active { pal.on_gold } else { pal.weak }, rect.width() - 56.0);
        if resp.on_hover_text(format!("Toggle emulation (hotkey: {hk})")).clicked() {
            let _ = self.tx.try_send(Msg::SetEnabled(!self.status.user_enabled));
        }
    }

    /// Returns (clicked, context-menu action).
    fn profile_item(&mut self, ui: &mut egui::Ui, i: usize) -> (bool, Option<&'static str>) {
        let pal = self.pal;
        let p = &self.config.profiles[i];
        let selected = i == self.pi();
        // Badge only while emulation is switched on (live or paused).
        let live = self.status.user_enabled && self.status.profile == p.name;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
        let painter = ui.painter();
        let fill = if selected {
            pal.raised
        } else if resp.hovered() {
            pal.raised.gamma_multiply(0.6)
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect, 9.0, fill);
        if selected {
            painter.rect_filled(Rect::from_min_size(rect.left_top() + vec2(0.0, 10.0), vec2(3.0, rect.height() - 20.0)), 2.0, pal.gold);
        }
        let name_col = if selected { pal.text } else { pal.text.gamma_multiply(0.85) };
        let right_pad = if live { 52.0 } else { 12.0 };
        preview::fit_text(painter, pos2(rect.left() + 14.0, rect.top() + 16.0), Align2::LEFT_CENTER, &p.name, theme::bold(14.0), name_col, rect.width() - 14.0 - right_pad);
        let sub = if let Some(exe) = (!p.game_path.is_empty()).then(|| file_name(&p.game_path).to_string()) {
            exe
        } else if !p.executables.is_empty() {
            p.executables.join(", ")
        } else if i == 0 {
            "Fallback - all other apps".into()
        } else {
            "No game set".into()
        };
        preview::fit_text(painter, pos2(rect.left() + 14.0, rect.top() + 35.0), Align2::LEFT_CENTER, &sub, theme::font(11.5), pal.weak, rect.width() - 14.0 - right_pad);
        if live {
            let active = self.status.active;
            let badge = Rect::from_center_size(pos2(rect.right() - 28.0, rect.center().y), vec2(40.0, 18.0));
            if active {
                painter.rect_filled(badge, 9.0, pal.gold_fill);
                painter.text(badge.center(), Align2::CENTER_CENTER, "LIVE", theme::bold(10.5), pal.on_gold);
            } else {
                painter.rect_stroke(badge, 9.0, Stroke::new(1.0, pal.weak), StrokeKind::Inside);
                painter.text(badge.center(), Align2::CENTER_CENTER, "IN USE", theme::bold(9.0), pal.weak);
            }
        }
        let mut act = None;
        let has_game = !p.game_path.is_empty();
        let can_delete = self.config.profiles.len() > 1;
        resp.context_menu(|ui| {
            if has_game && ui.button("Launch game").clicked() {
                act = Some("launch");
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                act = Some("duplicate");
                ui.close();
            }
            if ui.add_enabled(can_delete, egui::Button::new("Delete")).clicked() {
                act = Some("delete");
                ui.close();
            }
        });
        let tip = if live { "Currently used profile" } else { "Right-click for more" };
        (resp.on_hover_text(tip).clicked(), act)
    }

    fn profile_action(&mut self, ctx: &egui::Context, i: usize, a: &str) {
        match a {
            "launch" => self.launch_game(ctx, i),
            "duplicate" => {
                let mut p = self.config.profiles[i].clone();
                p.name += " copy";
                p.executables.clear();
                p.game_path.clear();
                self.config.profiles.push(p);
                self.sel_profile = self.config.profiles.len() - 1;
                self.sel_layer = 0;
                self.selected = None;
                self.changed();
            }
            "delete" if self.config.profiles.len() > 1 => {
                self.config.profiles.remove(i);
                if self.sel_profile > i {
                    self.sel_profile -= 1;
                }
                self.sel_profile = self.sel_profile.min(self.config.profiles.len() - 1);
                self.sel_layer = 0;
                self.selected = None;
                self.macro_text.clear();
                self.changed();
            }
            _ => {}
        }
    }

    fn driver_line(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let st = &self.status;
        let (dot, text) = match (st.pad_connected, &st.pad_error) {
            (true, _) => (pal.gold, format!("Virtual {} connected", if st.controller == ControllerKind::Ds4 { "DualShock 4" } else { "Xbox 360" })),
            (false, Some(_)) => (pal.err, "Virtual controller unavailable".to_string()),
            (false, None) => (pal.weak, "Virtual controller idle".to_string()),
        };
        if st.pad_error.as_deref().is_some_and(|e| e.contains("ViGEmBus")) && icons::text_button(ui, &pal, Some(Icon::Arrow), "Install ViGEmBus driver", Style::Link).clicked() {
            open_url(VIGEM_URL);
        }
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            ui.label(RichText::new(text).small().color(pal.weak));
        });
    }

    // ---------------------------------------------------------------- nav --

    fn nav(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            for (page, icon, label) in [(Page::Mapping, Icon::Gamepad, "Mapping"), (Page::Mouse, Icon::Sliders, "Mouse tuning"), (Page::Settings, Icon::Gear, "Settings"), (Page::Help, Icon::Help, "Help")] {
                if icons::text_button(ui, &pal, Some(icon), label, Style::Tab(self.page == page)).clicked() {
                    self.page = page;
                }
                ui.add_space(4.0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dark = self.config.settings.theme == Theme::Dark;
                let (icon, tip) = if dark { (Icon::Sun, "Switch to light mode") } else { (Icon::Moon, "Switch to dark mode") };
                if icons::button(ui, &pal, icon, 32.0, false, tip).clicked() {
                    self.set_theme(if dark { Theme::Light } else { Theme::Dark });
                }
                ui.add_space(6.0);
                if self.dirty_since.is_some() {
                    ui.label(RichText::new("saving...").small().color(pal.weak));
                } else if self.saved_at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
                    ui.label(RichText::new("saved").small().color(pal.gold));
                }
                let s = &self.shared.stats;
                let last = s.last_latency_us.load(Relaxed);
                let max = s.max_latency_us.load(Relaxed);
                if s.events.load(Relaxed) > 0 {
                    let r = ui.label(RichText::new(format!("{last} us  (max {max})")).monospace().small().color(pal.weak));
                    if r.on_hover_text(format!("Internal latency from key/mouse event to the virtual controller.\nEvents {}  Reports {}\nClick to reset max.", s.events.load(Relaxed), s.reports.load(Relaxed))).clicked() {
                        s.max_latency_us.store(0, Relaxed);
                    }
                }
            });
        });
        self.banners(ui);
    }

    fn banners(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let st = self.status.clone();
        let banner = |ui: &mut egui::Ui, col: Color32, add: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Frame::new().fill(col.gamma_multiply(0.12)).stroke(Stroke::new(1.0, col.gamma_multiply(0.6))).corner_radius(9).inner_margin(Margin::symmetric(12, 7)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| add(ui));
            });
        };
        if let Some(e) = &st.pad_error {
            ui.add_space(6.0);
            banner(ui, pal.err, &mut |ui| {
                ui.label(RichText::new(e).color(pal.text));
                if e.contains("ViGEmBus") && icons::text_button(ui, &pal, None, "Download ViGEmBus", Style::Primary).clicked() {
                    open_url(VIGEM_URL);
                }
            });
        }
        if !st.config_errors.is_empty() {
            ui.add_space(6.0);
            banner(ui, pal.warn, &mut |ui| {
                ui.label(RichText::new(format!("{} mapping issue(s): {}", st.config_errors.len(), st.config_errors.join("; "))).color(pal.text));
            });
        }
        let mut clear = false;
        if let Some((n, err)) = &self.notice {
            ui.add_space(6.0);
            banner(ui, if *err { pal.err } else { pal.gold }, &mut |ui| {
                ui.label(RichText::new(n).color(pal.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clear = icons::button(ui, &pal, Icon::Close, 22.0, false, "Dismiss").clicked();
                });
            });
        }
        if clear {
            self.notice = None;
        }
    }

    // ------------------------------------------------------------ mapping --

    fn mapping_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let pi = self.pi();
        let mut changed = false;
        let mut act = None;

        // Profile name + controller type.
        ui.horizontal(|ui| {
            let name = &mut self.config.profiles[pi].name;
            let edit = egui::TextEdit::singleline(name).font(theme::bold(24.0)).frame(egui::Frame::NONE).desired_width(420.0).text_color(pal.text);
            changed |= ui.add(edit).on_hover_text("Click to rename").changed();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let k = self.config.settings.controller;
                for (kind, label) in [(ControllerKind::Ds4, "DualShock 4"), (ControllerKind::Xbox360, "Xbox 360")] {
                    if icons::text_button(ui, &pal, None, label, Style::Chip(k == kind)).on_hover_text("Virtual controller type (all profiles)").clicked() && k != kind {
                        self.config.settings.controller = kind;
                        changed = true;
                    }
                }
                ui.label(RichText::new("Controller").small().color(pal.weak));
                ui.add_space(16.0);
                if self.config.profiles.len() > 1 && icons::button(ui, &pal, Icon::Trash, 30.0, false, "Delete this profile").clicked() {
                    act = Some("delete");
                }
                if icons::button(ui, &pal, Icon::Copy, 30.0, false, "Duplicate this profile").clicked() {
                    act = Some("duplicate");
                }
            });
        });
        if let Some(a) = act {
            self.profile_action(ui.ctx(), pi, a);
            return;
        }
        ui.add_space(4.0);
        changed |= self.game_card(ui, pi);
        ui.add_space(10.0);

        // Layers | devices | mode.
        let li = self.li();
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            icons::draw(ui.painter(), r, Icon::Layers, pal.weak, pal.bg);
            let n = self.config.profiles[pi].layers.len();
            let mut delete = None;
            for l in 0..n {
                let name = self.config.profiles[pi].layers[l].name.clone();
                let trig = self.layer_trigger(pi, &name);
                let resp = icons::text_button(ui, &pal, None, &name, Style::Chip(l == li));
                let resp = if l == 0 { resp.on_hover_text("Base layer (always active)") } else { resp.on_hover_text(trig.unwrap_or_else(|| "Not activated by any key yet: map a key to Layer".into())) };
                if resp.clicked() {
                    self.sel_layer = l;
                    self.selected = None;
                }
                if l > 0 {
                    resp.context_menu(|ui| {
                        if ui.button("Delete layer").clicked() {
                            delete = Some(l);
                            ui.close();
                        }
                    });
                }
            }
            if icons::button(ui, &pal, Icon::Plus, 28.0, false, "Add a layer (a shift state activated by a key)").clicked() {
                self.config.profiles[pi].layers.push(Layer { name: format!("Layer {n}"), binds: vec![] });
                self.sel_layer = n;
                changed = true;
            }
            if let Some(l) = delete {
                self.config.profiles[pi].layers.remove(l);
                self.sel_layer = 0;
                self.macro_text.clear();
                changed = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (m, icon, label) in [(MapMode::List, Icon::List, "List"), (MapMode::Preview, Icon::Eye, "Preview")] {
                    if icons::text_button(ui, &pal, Some(icon), label, Style::Chip(self.mode == m)).on_hover_text(if m == MapMode::List { "Map with the classic list (preview stays visible)" } else { "Map by clicking the device" }).clicked() {
                        self.mode = m;
                    }
                }
                ui.add_space(14.0);
                for (d, icon, label) in [(Device::Mouse, Icon::Mouse, "Mouse"), (Device::Keyboard, Icon::Keyboard, "Keyboard"), (Device::Gamepad, Icon::Gamepad, "Gamepad")] {
                    if icons::text_button(ui, &pal, Some(icon), label, Style::Tab(self.device == d)).clicked() {
                        self.device = d;
                    }
                }
            });
        });

        // Layer rename (non-base layers).
        if li > 0 {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Layer name").small().color(pal.weak));
                let old = self.config.profiles[pi].layers[li].name.clone();
                if ui.add(egui::TextEdit::singleline(&mut self.config.profiles[pi].layers[li].name).desired_width(160.0)).changed() {
                    let new = self.config.profiles[pi].layers[li].name.clone();
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
                let trig = self.layer_trigger(pi, &self.config.profiles[pi].layers[li].name.clone());
                ui.label(RichText::new(trig.unwrap_or_else(|| "Not activated yet: map a key to the Layer action on the Base layer".into())).small().color(pal.weak));
                ui.label(RichText::new("Keys not mapped here fall through to the layers below.").small().color(pal.weak));
            });
        }
        ui.add_space(6.0);

        // Preview card.
        let li = self.li();
        let kind = self.kind();
        let summary = preview::Summary::build(&self.config.profiles[pi].layers[li].binds, self.config.profiles[pi].mouse.target, kind);
        let live = (self.status.active && self.status.profile == self.config.profiles[pi].name).then_some(self.status.pad);
        let avail = ui.available_height();
        let prev_h = match self.mode {
            // Empty inspector is one compact row: give the device the room.
            MapMode::Preview if self.selected.is_none() => (avail - 96.0).clamp(230.0, 620.0),
            MapMode::Preview => {
                // Leave room for the selected element's rows + quick-add bar.
                let n = self.selected.map_or(0, |e| self.config.profiles[pi].layers[li].binds.iter().filter(|b| preview::bind_matches(b, e)).count());
                let need = 120.0 + 46.0 * n.clamp(1, 4) as f32;
                (avail - need).clamp(230.0, 560.0)
            }
            MapMode::List => (avail * 0.48).clamp(200.0, 380.0),
        };
        let row_hover = std::mem::take(&mut self.row_hover);
        let mut out = None;
        card(ui, &pal, 10, |ui| {
            let size = vec2(ui.available_width(), prev_h - 20.0);
            let v = preview::View { device: self.device, kind, pal: &pal, sum: &summary, selected: self.selected, highlight: &row_hover, live };
            out = Some(preview::show(ui, size, &v));
        });
        if let Some(o) = out {
            self.preview_hover = o.hovered;
            if let Some(e) = o.clicked {
                self.selected = if self.selected == Some(e) { None } else { Some(e) };
            }
        }
        ui.add_space(10.0);
        match self.mode {
            MapMode::Preview => self.inspector(ui, pi, li),
            MapMode::List => self.list(ui, pi, li),
        }
        if changed {
            self.changed();
        }
    }

    /// "Activated by CapsLock (hold)" for a layer name.
    fn layer_trigger(&self, pi: usize, name: &str) -> Option<String> {
        let keys: Vec<String> = self.config.profiles[pi]
            .layers
            .iter()
            .flat_map(|l| l.binds.iter())
            .filter_map(|b| match &b.action {
                Action::Layer { layer, mode } if layer == name && !b.input.is_empty() => Some(format!("{} ({})", b.input, if *mode == LayerMode::Hold { "hold" } else { "toggle" })),
                _ => None,
            })
            .collect();
        (!keys.is_empty()).then(|| format!("Activated by {}", keys.join(", ")))
    }

    fn game_card(&mut self, ui: &mut egui::Ui, pi: usize) -> bool {
        let pal = self.pal;
        let mut changed = false;
        let mut browse = false;
        let mut pick = false;
        let mut launch = false;
        let mut clear = false;
        card(ui, &pal, 12, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                ui.painter().rect_filled(r, 10.0, pal.raised);
                let p = &self.config.profiles[pi];
                let has_game = !p.game_path.is_empty();
                icons::draw(ui.painter(), r.shrink(10.0), if has_game { Icon::Gamepad } else { Icon::File }, if has_game { pal.gold } else { pal.weak }, pal.raised);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if has_game {
                        ui.label(RichText::new(file_name(&p.game_path)).font(theme::bold(15.0)).color(pal.text));
                        ui.label(RichText::new(&p.game_path).small().color(pal.weak)).on_hover_text(&p.game_path);
                    } else if pi == 0 {
                        ui.label(RichText::new("Fallback profile").font(theme::bold(15.0)).color(pal.text));
                        ui.label(RichText::new("Used for every app no other profile claims. Add a game profile for per-game mappings.").small().color(pal.weak));
                    } else {
                        ui.label(RichText::new("No game selected").font(theme::bold(15.0)).color(pal.text));
                        ui.label(RichText::new("Pick the running game or browse for its .exe. Launching from PadForge applies this profile instantly.").small().color(pal.weak));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if has_game {
                        clear = icons::button(ui, &pal, Icon::Close, 28.0, false, "Remove game").clicked();
                    }
                    browse = icons::text_button(ui, &pal, Some(Icon::Folder), "Browse .exe", Style::Normal).on_hover_text("Choose the game's executable file").clicked();
                    pick = icons::text_button(ui, &pal, Some(Icon::Target), "Running game", Style::Normal).on_hover_text("Pick from apps that are running now").clicked();
                    if has_game {
                        launch = icons::text_button(ui, &pal, Some(Icon::Play), "Launch", Style::Primary).on_hover_text("Start the game from PadForge").clicked();
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Auto-detect").small().color(pal.weak)).on_hover_text("Executable names that switch to this profile when focused, comma separated");
                if self.exes_text.0 != pi {
                    self.exes_text = (pi, self.config.profiles[pi].executables.join(", "));
                }
                let w = (ui.available_width() - 330.0).max(160.0);
                if ui.add(egui::TextEdit::singleline(&mut self.exes_text.1).hint_text("e.g. eldenring.exe").desired_width(w)).changed() {
                    self.config.profiles[pi].executables = self.exes_text.1.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect();
                    changed = true;
                }
                let fg = self.status.foreground_exe.clone();
                if !fg.is_empty() && !self.config.profiles[pi].executables.contains(&fg) && icons::text_button(ui, &pal, Some(Icon::Plus), &fg, Style::Link).on_hover_text("Add the last focused app").clicked() {
                    self.config.profiles[pi].executables.push(fg);
                    self.exes_text.0 = usize::MAX;
                    changed = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.config.profiles[pi].launch_args).hint_text("launch options").desired_width(150.0)).on_hover_text("Command-line arguments used when launching").changed();
                });
            });
        });
        if clear {
            self.config.profiles[pi].game_path.clear();
            changed = true;
        }
        if browse {
            let owner = self.shared.gui_window.load(Relaxed);
            if let Some(path) = games::browse_exe(owner, &self.config.profiles[pi].game_path) {
                self.set_game(pi, path);
                changed = true;
            }
        }
        if pick {
            self.picker = Some(Picker { apps: games::running_apps(), filter: String::new() });
        }
        if launch {
            let ctx = ui.ctx().clone();
            self.launch_game(&ctx, pi);
        }
        changed
    }

    fn set_game(&mut self, pi: usize, path: String) {
        let p = &mut self.config.profiles[pi];
        p.game_path = path;
        // Name a fresh profile after its game.
        if p.name.starts_with("Game ") || p.name == "New profile" {
            let stem = file_name(&p.game_path).trim_end_matches(".exe").trim_end_matches(".EXE");
            if !stem.is_empty() {
                p.name = stem.to_string();
            }
        }
        self.exes_text.0 = usize::MAX;
    }

    fn picker_modal(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let Some(pk) = &mut self.picker else { return };
        let mut close = false;
        let mut chosen: Option<RunningApp> = None;
        let mut refresh = false;
        let frame = egui::Frame::new().fill(pal.surface).stroke(Stroke::new(1.0, pal.line)).corner_radius(14).inner_margin(18);
        let resp = egui::Modal::new(egui::Id::unique("game-picker")).frame(frame).show(ctx, |ui| {
            ui.set_width(540.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Pick a running game").font(theme::bold(19.0)).color(pal.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close |= icons::button(ui, &pal, Icon::Close, 28.0, false, "Close").clicked();
                    refresh |= icons::button(ui, &pal, Icon::Refresh, 28.0, false, "Refresh").clicked();
                });
            });
            ui.label(RichText::new("Start the game first. Apps with a visible window are listed.").small().color(pal.weak));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                icons::draw(ui.painter(), r, Icon::Search, pal.weak, pal.surface);
                ui.add(egui::TextEdit::singleline(&mut pk.filter).hint_text("Filter").desired_width(f32::INFINITY));
            });
            ui.add_space(6.0);
            let f = pk.filter.to_ascii_lowercase();
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut any = false;
                for app in pk.apps.iter().filter(|a| f.is_empty() || a.exe.contains(&f) || a.title.to_ascii_lowercase().contains(&f)) {
                    any = true;
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                    if r.hovered() {
                        ui.painter().rect(rect, 9.0, pal.raised, Stroke::new(1.0, pal.gold), StrokeKind::Inside);
                    }
                    let p = ui.painter();
                    p.text(pos2(rect.left() + 12.0, rect.top() + 15.0), Align2::LEFT_CENTER, &app.exe, theme::bold(14.0), pal.text);
                    preview::fit_text(p, pos2(rect.left() + 12.0, rect.top() + 33.0), Align2::LEFT_CENTER, &app.title, theme::font(11.5), pal.weak, rect.width() - 24.0);
                    if r.clicked() {
                        chosen = Some(app.clone());
                    }
                }
                if !any {
                    ui.label(RichText::new("Nothing found.").color(pal.weak));
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        if refresh {
            pk.apps = games::running_apps();
        }
        if let Some(app) = chosen {
            let pi = self.pi();
            match games::process_path(app.pid) {
                Some(path) => self.set_game(pi, path),
                None => {
                    let exes = &mut self.config.profiles[pi].executables;
                    if !exes.contains(&app.exe) {
                        exes.push(app.exe.clone());
                    }
                    self.exes_text.0 = usize::MAX;
                    self.notice = Some((format!("{} is auto-detected now. Its file path is not readable (protected or elevated process), so use Browse .exe to enable launching.", app.exe), false));
                }
            }
            self.changed();
            close = true;
        }
        if close {
            self.picker = None;
        }
    }

    // ---------------------------------------------------- binding editing --

    /// Draw binding rows; returns (changed, capture, remove, hovered) indices.
    fn rows(&mut self, ui: &mut egui::Ui, pi: usize, li: usize, idx: &[usize], highlight_sel: bool) -> (bool, Option<usize>, Option<usize>, Option<usize>) {
        let pal = self.pal;
        let kind = self.kind();
        let layers = self.layer_names(pi);
        let capturing = self.capture.as_ref().map(|c| c.target);
        let sel = self.selected;
        let hover = self.preview_hover;
        let (mut changed, mut cap, mut rem, mut hov) = (false, None, None, None);
        for &bi in idx {
            let bind = &mut self.config.profiles[pi].layers[li].binds[bi];
            let hl = highlight_sel && [sel, hover].into_iter().flatten().any(|e| preview::bind_matches(bind, e));
            let o = editor::bind_row(ui, &pal, bind, (pi, li, bi), kind, &layers, &mut self.macro_text, capturing == Some(CaptureTarget::Bind { profile: pi, layer: li, bind: bi }), hl);
            changed |= o.changed;
            if o.capture {
                cap = Some(bi);
            }
            if o.remove {
                rem = Some(bi);
            }
            if o.hovered {
                hov = Some(bi);
            }
        }
        (changed, cap, rem, hov)
    }

    fn apply_row_ops(&mut self, pi: usize, li: usize, changed: bool, cap: Option<usize>, rem: Option<usize>) {
        let mut changed = changed;
        if let Some(bi) = rem {
            self.config.profiles[pi].layers[li].binds.remove(bi);
            self.macro_text.clear();
            changed = true;
        }
        if let (Some(bi), None) = (cap, rem) {
            self.start_capture(CaptureTarget::Bind { profile: pi, layer: li, bind: bi });
        }
        if changed {
            self.changed();
        }
    }

    /// Add a binding (empty input = ask for a key right away).
    fn add_bind(&mut self, pi: usize, li: usize, input: String, action: Action) {
        let ask = input.is_empty();
        let binds = &mut self.config.profiles[pi].layers[li].binds;
        binds.push(Bind { input, action, block: true });
        let bi = binds.len() - 1;
        self.changed();
        if ask {
            self.start_capture(CaptureTarget::Bind { profile: pi, layer: li, bind: bi });
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui, pi: usize, li: usize) {
        let pal = self.pal;
        let kind = self.kind();
        card(ui, &pal, 14, |ui| {
            let Some(sel) = self.selected else {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new("Click any key, button or stick above to map it").font(theme::bold(15.0)).color(pal.text));
                        ui.label(RichText::new("Mapped elements are gold and show what they do. Switch to List to see every mapping at once.").small().color(pal.weak));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let detecting = self.capture.as_ref().is_some_and(|c| c.target == CaptureTarget::Select);
                        let label = if detecting { "Press any key or mouse button..." } else { "Detect a key" };
                        if icons::text_button(ui, &pal, Some(Icon::Target), label, Style::Normal).on_hover_text("Press a physical key to find it on the preview").clicked() {
                            self.start_capture(CaptureTarget::Select);
                        }
                    });
                });
                return;
            };
            ui.set_min_height(ui.available_height() - 28.0);

            let (title, sub) = match sel {
                Elem::Input(id) => {
                    let (t, s) = input_title(id);
                    (t, s.to_string())
                }
                Elem::MouseMove => ("Mouse movement".into(), "Moves a controller stick".into()),
                Elem::Pad(PadEl::Stick(s)) => (format!("{} stick", if s == Side::Left { "Left" } else { "Right" }), "Controller output. Keys and mouse that move it:".into()),
                Elem::Pad(PadEl::Dpad) => ("D-pad".into(), "Controller output. Keys that press it:".into()),
                Elem::Pad(PadEl::Btn(c)) => (c.label(kind).to_string(), "Controller output. Keys that press it:".into()),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(theme::bold(18.0)).color(pal.text));
                ui.label(RichText::new(sub).color(pal.weak));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, &pal, Icon::Close, 26.0, false, "Deselect").clicked() {
                        self.selected = None;
                    }
                });
            });
            ui.add_space(4.0);
            if self.selected.is_none() {
                return;
            }

            if sel == Elem::MouseMove {
                self.mouse_move_editor(ui, pi);
                return;
            }

            let idx: Vec<usize> = self.config.profiles[pi].layers[li].binds.iter().enumerate().filter(|(_, b)| preview::bind_matches(b, sel)).map(|(i, _)| i).collect();
            let mut ops = (false, None, None);
            egui::ScrollArea::vertical().max_height((ui.available_height() - 44.0).max(60.0)).auto_shrink([false, true]).show(ui, |ui| {
                let (c, cap, rem, _) = self.rows(ui, pi, li, &idx, false);
                ops = (c, cap, rem);
            });
            self.apply_row_ops(pi, li, ops.0, ops.1, ops.2);

            // Quick add.
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| match sel {
                Elem::Input(id) => {
                    if idx.is_empty() {
                        let name = keys::input_name(id);
                        ui.label(RichText::new(format!("Map {name} to")).color(pal.weak));
                        let layers = self.layer_names(pi);
                        for (k, label) in [("Button", "Button"), ("Stick", "Stick direction"), ("Trigger", "Trigger"), ("Stick scale", "Walk modifier"), ("Macro", "Macro"), ("Layer", "Layer"), ("Disable", "Block key")] {
                            if icons::text_button(ui, &pal, Some(Icon::Plus), label, Style::Normal).clicked() {
                                self.add_bind(pi, li, name.clone(), editor::default_action(k, &layers));
                            }
                        }
                    }
                }
                Elem::Pad(PadEl::Btn(c)) => {
                    if icons::text_button(ui, &pal, Some(Icon::Plus), "Assign a key", Style::Primary).on_hover_text("Then press the key or mouse button").clicked() {
                        self.add_bind(pi, li, String::new(), Action::Button { button: c, mode: ButtonMode::Hold, turbo_hz: 12.0 });
                    }
                    if matches!(c, Control::Lt | Control::Rt) && icons::text_button(ui, &pal, Some(Icon::Plus), "Half press", Style::Normal).clicked() {
                        self.add_bind(pi, li, String::new(), Action::Trigger { trigger: if c == Control::Lt { Side::Left } else { Side::Right }, value: 0.5 });
                    }
                }
                Elem::Pad(PadEl::Dpad) => {
                    ui.label(RichText::new("Assign a key to").color(pal.weak));
                    for (c, label) in [(Control::DpadUp, "Up"), (Control::DpadDown, "Down"), (Control::DpadLeft, "Left"), (Control::DpadRight, "Right")] {
                        if icons::text_button(ui, &pal, Some(Icon::Plus), label, Style::Normal).clicked() {
                            self.add_bind(pi, li, String::new(), Action::Button { button: c, mode: ButtonMode::Hold, turbo_hz: 12.0 });
                        }
                    }
                }
                Elem::Pad(PadEl::Stick(side)) => {
                    let target = if side == Side::Left { StickTarget::Left } else { StickTarget::Right };
                    let mouse_on = self.config.profiles[pi].mouse.target == target;
                    if icons::text_button(ui, &pal, Some(Icon::Mouse), if mouse_on { "Mouse moves this stick" } else { "Use mouse for this stick" }, Style::Chip(mouse_on)).clicked() {
                        self.config.profiles[pi].mouse.target = if mouse_on { StickTarget::None } else { target };
                        self.changed();
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("Assign a key to").color(pal.weak));
                    for (label, x, y) in [("Up", 0.0, 1.0), ("Down", 0.0, -1.0), ("Left", -1.0, 0.0), ("Right", 1.0, 0.0)] {
                        if icons::text_button(ui, &pal, Some(Icon::Plus), label, Style::Normal).clicked() {
                            self.add_bind(pi, li, String::new(), Action::Stick { stick: side, x, y });
                        }
                    }
                    let click = if side == Side::Left { Control::Ls } else { Control::Rs };
                    if icons::text_button(ui, &pal, Some(Icon::Plus), "Click", Style::Normal).clicked() {
                        self.add_bind(pi, li, String::new(), Action::Button { button: click, mode: ButtonMode::Hold, turbo_hz: 12.0 });
                    }
                    if icons::text_button(ui, &pal, Some(Icon::Plus), "Walk modifier", Style::Normal).on_hover_text("Scale the stick while held").clicked() {
                        self.add_bind(pi, li, String::new(), Action::StickScale { stick: side, factor: 0.5 });
                    }
                }
                Elem::MouseMove => {}
            });
        });
    }

    fn mouse_move_editor(&mut self, ui: &mut egui::Ui, pi: usize) {
        let pal = self.pal;
        let mut changed = false;
        let m = &mut self.config.profiles[pi].mouse;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Mouse moves").color(pal.weak));
            for (t, label) in [(StickTarget::Right, "Right stick"), (StickTarget::Left, "Left stick"), (StickTarget::None, "Nothing (normal mouse)")] {
                if icons::text_button(ui, &pal, None, label, Style::Chip(m.target == t)).clicked() {
                    m.target = t;
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Sensitivity").color(pal.weak));
            changed |= ui.add(egui::Slider::new(&mut m.sensitivity, 0.01..=2.0).logarithmic(true)).changed();
            ui.add_space(12.0);
            if icons::text_button(ui, &pal, Some(Icon::Sliders), "Full mouse tuning", Style::Link).clicked() {
                self.page = Page::Mouse;
            }
        });
        if changed {
            self.changed();
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, pi: usize, li: usize) {
        let pal = self.pal;
        let kind = self.kind();
        let n = self.config.profiles[pi].layers[li].binds.len();
        ui.horizontal(|ui| {
            let lname = self.config.profiles[pi].layers[li].name.clone();
            ui.label(RichText::new(format!("{n} mapping{} in {lname}", if n == 1 { "" } else { "s" })).font(theme::bold(15.0)).color(pal.text));
            ui.label(RichText::new("Hover a row to see it on the preview").small().color(pal.weak));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::text_button(ui, &pal, Some(Icon::Plus), "Add mapping", Style::Primary).on_hover_text("Then press the key or mouse button").clicked() {
                    self.add_bind(pi, li, String::new(), Action::Button { button: Control::A, mode: ButtonMode::Hold, turbo_hz: 12.0 });
                }
            });
        });
        ui.add_space(4.0);
        let idx: Vec<usize> = (0..n).collect();
        let mut res = (false, None, None, None);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            res = self.rows(ui, pi, li, &idx, true);
        });
        if let Some(bi) = res.3 {
            if let Some(b) = self.config.profiles[pi].layers[li].binds.get(bi) {
                let mut hl: Vec<Elem> = preview::action_targets(&b.action).into_iter().map(Elem::Pad).collect();
                if let Some(id) = keys::parse_input(&b.input) {
                    hl.push(Elem::Input(id));
                }
                self.row_hover = hl;
            }
        }
        let _ = kind;
        self.apply_row_ops(pi, li, res.0, res.1, res.2);
    }
}

/// Rounded surface card.
fn card<R>(ui: &mut egui::Ui, pal: &Pal, margin: i8, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(pal.surface)
        .stroke(Stroke::new(1.0, pal.line))
        .corner_radius(14)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

impl eframe::App for GuiApp {
    /// Runs every pass, including while the window is hidden in the tray.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        use crate::shared::{GUI_REQ_NONE, GUI_REQ_QUIT, GUI_REQ_SHOW};
        match self.shared.gui_request.swap(GUI_REQ_NONE, Relaxed) {
            GUI_REQ_SHOW => {
                self.shared.gui_hidden.store(false, Relaxed);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.shared.notify(); // tray menu state
            }
            GUI_REQ_QUIT => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint(); // the close is handled on the next pass
            }
            _ => {}
        }

        // Close button -> hide to tray (unless quitting from the tray menu).
        if ctx.input(|i| i.viewport().close_requested()) && self.config.settings.close_to_tray && !self.shared.quit.load(Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_to_tray(ctx);
            if !self.told_about_tray {
                self.told_about_tray = true;
                tray::balloon("PadForge is still running", "It keeps working from the tray. Right-click the icon to quit.");
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_theme(&ctx);
        self.status = self.shared.status_snapshot();
        self.poll_capture(&ctx);
        self.autosave(&ctx);

        if self.hide_after_first_frame {
            if ctx.cumulative_pass_nr() >= 1 {
                self.hide_after_first_frame = false;
                self.hide_to_tray(&ctx);
            } else {
                ctx.request_repaint();
            }
        }

        // Repaint policy: hidden = never; 20 Hz only while emulating and the
        // live controller (gamepad preview / mouse curve) is on screen;
        // otherwise only on input or engine status events.
        let live_visible = (self.page == Page::Mapping && self.device == Device::Gamepad) || self.page == Page::Mouse;
        if self.status.active && live_visible && !self.shared.gui_hidden.load(Relaxed) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        let pal = self.pal;
        egui::Panel::left("side")
            .resizable(false)
            .exact_size(262.0)
            .frame(egui::Frame::new().fill(pal.side).inner_margin(Margin { left: 16, right: 16, top: 18, bottom: 14 }).stroke(Stroke::new(1.0, pal.line)))
            .show(ui, |ui| self.sidebar(ui));
        egui::Panel::top("nav").frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin { left: 20, right: 20, top: 12, bottom: 6 })).show(ui, |ui| self.nav(ui));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin { left: 22, right: 22, top: 8, bottom: 16 })).show(ui, |ui| match self.page {
            Page::Mapping => self.mapping_page(ui),
            Page::Mouse => self.mouse_page(ui),
            Page::Settings => self.settings_page(ui),
            Page::Help => self.help_page(ui),
        });
        self.picker_modal(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shared.capture_next.store(false, Relaxed);
        if self.dirty_since.is_some() {
            let _ = save_config(&self.paths.config, &self.config);
        }
    }
}
