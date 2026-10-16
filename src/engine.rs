//! Mapping engine: turns physical input events into virtual controller state.
//!
//! Runs on its own thread. Fully event driven: it sleeps on the channel and
//! only schedules timed wake-ups while something is animating (mouse stick
//! decay, turbo, macros, wheel releases). Idle CPU usage is therefore zero.
//!
//! Pure logic (no OS calls) so it is unit tested on any platform; the OS side
//! is abstracted by [`Backend`].

use crate::config::{Action, ButtonMode, Config, LayerMode, MacroStep, MouseConfig, MouseSource, Profile, Socd, StickTarget};
use crate::keys::{self, InputId, NUM_INPUTS};
use crate::mouse_stick::MouseStick;
use crate::pad::{Control, ControllerKind, PadState, Side};
use crate::shared::{set_bit, BlockBits, Shared};
use std::sync::atomic::Ordering::*;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Messages delivered to the engine thread.
#[derive(Debug)]
pub enum Msg {
    /// Key or mouse button transition (autorepeat downs are filtered here).
    Input { id: InputId, down: bool, t: Instant },
    /// One wheel notch (pressed, auto-released after `wheel_press_ms`).
    Wheel { id: InputId, t: Instant },
    MouseDelta { dx: i32, dy: i32, raw: bool, t: Instant },
    ToggleEmulation,
    SetEnabled(bool),
    /// Foreground application changed. `own` = it's PadForge itself.
    Focus { exe: String, own: bool },
    Config(Box<Config>),
    Shutdown,
}

/// OS-facing operations. Implemented with ViGEmBus on Windows and a mock in tests.
pub trait Backend {
    fn pad_connect(&mut self, kind: ControllerKind) -> Result<(), String>;
    fn pad_disconnect(&mut self);
    fn pad_connected(&self) -> bool;
    fn pad_send(&mut self, s: &PadState) -> Result<(), String>;
    /// Request ~1 ms scheduler granularity while timed updates are needed.
    fn set_high_res_timer(&mut self, _on: bool) {}
}

// ----------------------------------------------------------------- compile --

#[derive(Copy, Clone, Debug, PartialEq)]
enum CAction {
    Button { ctl: Control, mode: ButtonMode, half: Duration },
    Trigger { side: Side, value: f32 },
    Stick { side: Side, x: f32, y: f32 },
    StickScale { side: Side, factor: f32 },
    Layer { idx: usize, mode: LayerMode },
    Macro { idx: usize, repeat: bool },
    Disable,
}

#[derive(Copy, Clone, Debug)]
struct CBind {
    action: CAction,
    block: bool,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum CStep {
    Press(Control),
    Release(Control),
    Wait(Duration),
}

struct CProfile {
    name: String,
    exes: Vec<String>,
    mouse: MouseConfig,
    layer_names: Vec<String>,
    /// layers[layer][input] -> binding
    layers: Vec<Box<[Option<CBind>]>>,
    macros: Vec<Vec<CStep>>,
}

fn compile(p: &Profile, errors: &mut Vec<String>) -> CProfile {
    let layer_names: Vec<String> = p.layers.iter().map(|l| l.name.clone()).collect();
    let mut macros = Vec::new();
    let mut layers = Vec::with_capacity(p.layers.len());
    for (li, layer) in p.layers.iter().enumerate() {
        let mut table: Box<[Option<CBind>]> = vec![None; NUM_INPUTS].into_boxed_slice();
        for bind in &layer.binds {
            let Some(id) = keys::parse_input(&bind.input) else {
                errors.push(format!("[{}/{}] unknown input '{}'", p.name, layer.name, bind.input));
                continue;
            };
            let action = match &bind.action {
                Action::Button { button, mode, turbo_hz } => {
                    let hz = turbo_hz.clamp(1.0, 60.0);
                    CAction::Button { ctl: *button, mode: *mode, half: Duration::from_secs_f32(0.5 / hz) }
                }
                Action::Trigger { trigger, value } => CAction::Trigger { side: *trigger, value: value.clamp(0.0, 1.0) },
                Action::Stick { stick, x, y } => CAction::Stick { side: *stick, x: x.clamp(-1.0, 1.0), y: y.clamp(-1.0, 1.0) },
                Action::StickScale { stick, factor } => CAction::StickScale { side: *stick, factor: factor.clamp(0.0, 1.0) },
                Action::Layer { layer: name, mode } => match layer_names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
                    Some(0) => {
                        errors.push(format!("[{}/{}] layer action targets the base layer", p.name, layer.name));
                        continue;
                    }
                    Some(idx) => CAction::Layer { idx, mode: *mode },
                    None => {
                        errors.push(format!("[{}/{}] unknown layer '{}'", p.name, layer.name, name));
                        continue;
                    }
                },
                Action::Macro { steps, repeat, tap_ms } => {
                    let tap = Duration::from_millis((*tap_ms).clamp(1, 5000) as u64);
                    let mut out = Vec::with_capacity(steps.len() * 2);
                    for s in steps {
                        match s {
                            MacroStep::Press(c) => out.push(CStep::Press(*c)),
                            MacroStep::Release(c) => out.push(CStep::Release(*c)),
                            MacroStep::Tap(c) => {
                                out.push(CStep::Press(*c));
                                out.push(CStep::Wait(tap));
                                out.push(CStep::Release(*c));
                            }
                            MacroStep::Wait(ms) => out.push(CStep::Wait(Duration::from_millis((*ms).min(60_000) as u64))),
                        }
                    }
                    if *repeat && !out.iter().any(|s| matches!(s, CStep::Wait(d) if !d.is_zero())) {
                        // A repeating macro without delays would spin; give it a frame.
                        out.push(CStep::Wait(Duration::from_millis(16)));
                    }
                    macros.push(out);
                    CAction::Macro { idx: macros.len() - 1, repeat: *repeat }
                }
                Action::Disable => CAction::Disable,
            };
            if table[id as usize].is_some() {
                errors.push(format!("[{}/{}] '{}' bound twice; last one wins", p.name, layer.name, bind.input));
            }
            table[id as usize] = Some(CBind { action, block: bind.block });
        }
        let _ = li;
        layers.push(table);
    }
    CProfile { name: p.name.clone(), exes: p.executables.clone(), mouse: p.mouse.clone(), layer_names, layers, macros }
}

// ----------------------------------------------------------------- runtime --

#[derive(Copy, Clone, Debug)]
struct Held {
    input: InputId,
    action: CAction,
    since: Instant,
    seq: u64,
}

#[derive(Clone, Debug)]
struct MacroRun {
    input: InputId,
    idx: usize,
    step: usize,
    next: Instant,
    pressed: u32,
    repeat: bool,
}

pub struct Engine<B: Backend> {
    pub backend: B,
    shared: Arc<Shared>,
    config: Config,
    profiles: Vec<CProfile>,
    current: usize,
    toggle_id: InputId,

    user_enabled: bool,
    foreground: String,
    foreground_own: bool,
    active: bool,

    // runtime state (reset whenever activity/profile changes)
    down: BlockBits,
    held: Vec<Held>,
    toggled_buttons: u32,
    toggled_layers: u64,
    macros: Vec<MacroRun>,
    wheel_release: Vec<(InputId, Instant)>,
    mouse: MouseStick,
    last_raw: Option<Instant>,
    mouse_src_label: &'static str,
    seq: u64,
    last_sent: Option<PadState>,
    timer_hi: bool,
    status_layer_mask: u64,
}

impl<B: Backend> Engine<B> {
    pub fn new(backend: B, shared: Arc<Shared>, config: Config) -> Self {
        let mut e = Engine {
            backend,
            shared,
            config: Config::default(),
            profiles: Vec::new(),
            current: 0,
            toggle_id: 0x77,
            user_enabled: false,
            foreground: String::new(),
            foreground_own: false,
            active: false,
            down: [0; NUM_INPUTS / 64],
            held: Vec::with_capacity(32),
            toggled_buttons: 0,
            toggled_layers: 0,
            macros: Vec::with_capacity(8),
            wheel_release: Vec::with_capacity(4),
            mouse: MouseStick::new(),
            last_raw: None,
            mouse_src_label: "-",
            seq: 0,
            last_sent: None,
            timer_hi: false,
            status_layer_mask: 1,
        };
        let start = config.settings.start_enabled;
        e.apply_config(config, true);
        e.user_enabled = start;
        e.refresh_activity(Instant::now());
        e
    }

    fn cur(&self) -> &CProfile {
        &self.profiles[self.current]
    }

    // ------------------------------------------------------------ config --

    fn apply_config(&mut self, config: Config, initial: bool) {
        let mut errors = Vec::new();
        let profiles: Vec<CProfile> = config.profiles.iter().map(|p| compile(p, &mut errors)).collect();
        let toggle_id = match keys::parse_input(&config.settings.toggle_hotkey) {
            Some(id) if !keys::is_wheel(id) => id,
            _ => {
                errors.push(format!("unknown toggle hotkey '{}', using F8", config.settings.toggle_hotkey));
                0x77
            }
        };
        let kind_changed = initial || config.settings.controller != self.config.settings.controller;
        let keep_changed = config.settings.keep_connected != self.config.settings.keep_connected;
        let current_name = self.profiles.get(self.current).map(|p| p.name.clone());

        self.profiles = profiles;
        self.toggle_id = toggle_id;
        self.config = config;
        self.current = current_name
            .and_then(|n| self.profiles.iter().position(|p| p.name == n))
            .unwrap_or(0);

        let s = &self.config.settings;
        self.shared.toggle_input.store(toggle_id, Relaxed);
        self.shared.recenter_cursor.store(s.recenter_cursor, Relaxed);
        self.shared.toggle_sound.store(s.toggle_sound, Relaxed);
        self.shared.mouse_source.store(
            match s.mouse_source {
                MouseSource::Auto => 0,
                MouseSource::Raw => 1,
                MouseSource::Hook => 2,
            },
            Relaxed,
        );
        self.shared.status_mut(|st| {
            st.config_errors = errors;
            st.controller = s.controller;
        });

        if kind_changed && self.backend.pad_connected() {
            self.reset_runtime();
            self.backend.pad_disconnect();
        }
        if (kind_changed || keep_changed) && self.config.settings.keep_connected {
            self.ensure_pad();
        }
        // Re-evaluate everything against the new config.
        let was_active = self.active;
        self.active = false;
        if was_active {
            self.reset_runtime();
        }
        self.last_sent = None;
        self.refresh_activity(Instant::now());
    }

    fn ensure_pad(&mut self) -> bool {
        if self.backend.pad_connected() {
            return true;
        }
        let kind = self.config.settings.controller;
        let r = self.backend.pad_connect(kind);
        let ok = r.is_ok();
        self.shared.status_mut(|st| {
            st.pad_connected = ok;
            st.pad_error = r.err();
        });
        self.last_sent = None;
        ok
    }

    // ---------------------------------------------------------- activity --

    fn select_profile(&self) -> Option<usize> {
        if self.foreground_own {
            return None;
        }
        if let Some(i) = self.profiles.iter().position(|p| p.exes.contains(&self.foreground)) {
            return Some(i);
        }
        if self.config.settings.suspend_when_unmatched {
            None
        } else {
            Some(0)
        }
    }

    /// Recompute whether emulation should be active and which profile applies.
    fn refresh_activity(&mut self, now: Instant) {
        let sel = self.select_profile();
        let reason = if !self.user_enabled {
            Some(format!("disabled (press {} to enable)", keys::input_name(self.toggle_id)))
        } else if self.foreground_own {
            Some("PadForge window focused".to_string())
        } else if sel.is_none() {
            Some("no profile for this app".to_string())
        } else {
            None
        };

        let mut want = reason.is_none();
        if want && !self.ensure_pad() {
            want = false;
        }
        let new_profile = sel.unwrap_or(self.current);
        let profile_changed = new_profile != self.current;

        if want != self.active || (want && profile_changed) {
            self.reset_runtime();
            self.current = new_profile;
            self.active = want;
            if !want && !self.config.settings.keep_connected && self.backend.pad_connected() {
                self.backend.pad_disconnect();
                self.shared.status_mut(|st| st.pad_connected = false);
            }
        } else {
            self.current = new_profile;
        }
        let capture = self.active && self.cur().mouse.target != StickTarget::None;
        self.shared.capture_mouse.store(capture, Relaxed);
        self.publish_block();
        self.shared.active.store(self.active, Release);

        let pad_conn = self.backend.pad_connected();
        let (pname, fg, user, active) = (self.cur().name.clone(), self.foreground.clone(), self.user_enabled, self.active);
        let layers = self.active_layer_names();
        self.shared.status_mut(|st| {
            st.profile = pname;
            st.foreground_exe = fg;
            st.user_enabled = user;
            st.active = active;
            st.suspended_reason = if active { None } else { reason };
            st.pad_connected = pad_conn;
            st.active_layers = layers;
        });
        self.output(now, None);
        self.shared.notify();
    }

    /// Release everything and centre the controller.
    fn reset_runtime(&mut self) {
        self.down = [0; NUM_INPUTS / 64];
        self.held.clear();
        self.toggled_buttons = 0;
        self.toggled_layers = 0;
        self.macros.clear();
        self.wheel_release.clear();
        self.mouse.reset();
        self.last_raw = None;
        if self.backend.pad_connected() {
            let neutral = PadState::default();
            if self.last_sent != Some(neutral) {
                let _ = self.backend.pad_send(&neutral);
                self.last_sent = Some(neutral);
            }
        }
        self.shared.status_mut(|st| st.pad = PadState::default());
    }

    fn active_layers_mask(&self) -> u64 {
        let mut m = 1u64 | self.toggled_layers;
        for h in &self.held {
            if let CAction::Layer { idx, mode: LayerMode::Hold } = h.action {
                m |= 1u64 << idx.min(63);
            }
        }
        m
    }

    fn active_layer_names(&self) -> Vec<String> {
        let m = self.active_layers_mask();
        self.cur().layer_names.iter().enumerate().filter(|(i, _)| m & (1u64 << (*i).min(63)) != 0).map(|(_, n)| n.clone()).collect()
    }

    fn publish_block(&self) {
        if !self.active {
            self.shared.clear_block();
            return;
        }
        let mut bits: BlockBits = [0; NUM_INPUTS / 64];
        let mask = self.active_layers_mask();
        let p = self.cur();
        for (li, table) in p.layers.iter().enumerate() {
            if mask & (1u64 << li.min(63)) == 0 {
                continue;
            }
            for (id, b) in table.iter().enumerate() {
                if let Some(b) = b {
                    if b.block {
                        set_bit(&mut bits, id as InputId);
                    }
                }
            }
        }
        // Higher layers can unblock a key blocked lower down.
        for id in 0..NUM_INPUTS {
            if let Some(b) = self.lookup(id as InputId) {
                if !b.block {
                    bits[id / 64] &= !(1u64 << (id % 64));
                }
            }
        }
        self.shared.publish_block(&bits);
    }

    fn lookup(&self, id: InputId) -> Option<CBind> {
        let mask = self.active_layers_mask();
        let p = self.cur();
        for li in (0..p.layers.len()).rev() {
            if mask & (1u64 << li.min(63)) == 0 {
                continue;
            }
            if let Some(b) = p.layers[li][id as usize] {
                return Some(b);
            }
        }
        None
    }

    // ------------------------------------------------------------ events --

    fn is_down(&self, id: InputId) -> bool {
        let i = id as usize;
        self.down[i / 64] & (1u64 << (i % 64)) != 0
    }
    fn set_down(&mut self, id: InputId, v: bool) {
        let i = id as usize;
        if v {
            self.down[i / 64] |= 1u64 << (i % 64);
        } else {
            self.down[i / 64] &= !(1u64 << (i % 64));
        }
    }

    fn press(&mut self, id: InputId, t: Instant) {
        if (id as usize) >= NUM_INPUTS || self.is_down(id) {
            return; // autorepeat
        }
        self.set_down(id, true);
        let Some(bind) = self.lookup(id) else { return };
        self.seq += 1;
        let held = Held { input: id, action: bind.action, since: t, seq: self.seq };
        match bind.action {
            CAction::Button { ctl, mode: ButtonMode::Toggle, .. } => {
                self.toggled_buttons ^= ctl.bit();
            }
            CAction::Layer { idx, mode: LayerMode::Toggle } => {
                self.toggled_layers ^= 1u64 << idx.min(63);
                self.publish_block();
            }
            CAction::Layer { mode: LayerMode::Hold, .. } => {
                self.held.push(held);
                self.publish_block();
            }
            CAction::Macro { idx, repeat } => {
                // Restart if the same input re-triggers.
                self.macros.retain(|m| m.input != id);
                self.macros.push(MacroRun { input: id, idx, step: 0, next: t, pressed: 0, repeat });
                if repeat {
                    self.held.push(held);
                }
            }
            CAction::Disable => {}
            _ => self.held.push(held),
        }
    }

    fn release(&mut self, id: InputId) {
        if (id as usize) >= NUM_INPUTS || !self.is_down(id) {
            return;
        }
        self.set_down(id, false);
        let mut layer_changed = false;
        let mut stop_macro = false;
        self.held.retain(|h| {
            if h.input != id {
                return true;
            }
            match h.action {
                CAction::Layer { .. } => layer_changed = true,
                CAction::Macro { repeat: true, .. } => stop_macro = true,
                _ => {}
            }
            false
        });
        if stop_macro {
            self.macros.retain(|m| !(m.input == id && m.repeat));
        }
        if layer_changed {
            self.publish_block();
        }
    }

    fn mouse_delta(&mut self, dx: i32, dy: i32, raw: bool, t: Instant) -> bool {
        if !self.shared.capture_mouse.load(Relaxed) {
            return false;
        }
        let use_it = match self.config.settings.mouse_source {
            MouseSource::Raw => raw,
            MouseSource::Hook => !raw,
            MouseSource::Auto => {
                if raw {
                    self.last_raw = Some(t);
                    true
                } else {
                    // Prefer raw while it keeps arriving; hook deltas otherwise.
                    self.last_raw.is_none_or(|r| t.saturating_duration_since(r) > Duration::from_millis(1000))
                }
            }
        };
        if use_it {
            self.mouse_src_label = if raw { "raw input" } else { "hook" };
            self.mouse.push(t, dx as f32, dy as f32);
        }
        use_it
    }

    /// Process one message. Returns false on shutdown.
    pub fn handle(&mut self, msg: Msg, now: Instant) -> bool {
        let mut origin = None;
        match msg {
            Msg::Input { id, down, t } => {
                self.shared.stats.events.fetch_add(1, Relaxed);
                if !self.active {
                    return true;
                }
                if down {
                    self.press(id, t);
                } else {
                    self.release(id);
                }
                origin = Some(t);
            }
            Msg::Wheel { id, t } => {
                self.shared.stats.events.fetch_add(1, Relaxed);
                if !self.active {
                    return true;
                }
                let until = t + Duration::from_millis(self.config.settings.wheel_press_ms as u64);
                if let Some(w) = self.wheel_release.iter_mut().find(|w| w.0 == id) {
                    w.1 = until;
                } else {
                    self.press(id, t);
                    self.wheel_release.push((id, until));
                }
                origin = Some(t);
            }
            Msg::MouseDelta { dx, dy, raw, t } => {
                if !self.active || !self.mouse_delta(dx, dy, raw, t) {
                    return true;
                }
                origin = Some(t);
            }
            Msg::ToggleEmulation => {
                self.user_enabled = !self.user_enabled;
                self.refresh_activity(now);
            }
            Msg::SetEnabled(v) => {
                self.user_enabled = v;
                self.refresh_activity(now);
            }
            Msg::Focus { exe, own } => {
                // Remember the last *external* app so the GUI can offer it for
                // a profile even while PadForge itself is focused.
                if !own {
                    self.foreground = exe;
                }
                self.foreground_own = own;
                self.refresh_activity(now);
            }
            Msg::Config(c) => self.apply_config(*c, false),
            Msg::Shutdown => {
                self.active = false;
                self.shared.active.store(false, Release);
                self.shared.capture_mouse.store(false, Relaxed);
                self.shared.clear_block();
                self.reset_runtime();
                self.backend.pad_disconnect();
                self.set_timer(false);
                return false;
            }
        }
        self.output(now, origin);
        true
    }

    // ------------------------------------------------------------ timing --

    fn advance_timers(&mut self, now: Instant) {
        // Wheel auto-release.
        let mut i = 0;
        while i < self.wheel_release.len() {
            if self.wheel_release[i].1 <= now {
                let (id, _) = self.wheel_release.swap_remove(i);
                self.release(id);
            } else {
                i += 1;
            }
        }
        // Macros.
        let mut mi = 0;
        while mi < self.macros.len() {
            let mut finished = false;
            {
                let idx = self.macros[mi].idx;
                let steps = &self.profiles[self.current].macros[idx];
                let steps_len = steps.len();
                let run = &mut self.macros[mi];
                let mut guard = 0;
                while run.next <= now && guard < 1024 {
                    guard += 1;
                    if run.step >= steps_len {
                        if run.repeat {
                            run.step = 0;
                            if steps_len == 0 {
                                finished = true;
                                break;
                            }
                        } else {
                            finished = true;
                            break;
                        }
                    }
                    match steps[run.step] {
                        CStep::Press(c) => run.pressed |= c.bit(),
                        CStep::Release(c) => run.pressed &= !c.bit(),
                        CStep::Wait(d) => run.next += d,
                    }
                    run.step += 1;
                }
            }
            if finished {
                self.macros.swap_remove(mi);
            } else {
                mi += 1;
            }
        }
    }

    /// Earliest instant at which the output may change without new input.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if !self.active {
            return None;
        }
        let tick = Duration::from_secs_f64(1.0 / self.config.settings.output_rate_hz.max(1) as f64);
        let mut best: Option<Instant> = None;
        let mut consider = |t: Instant| {
            best = Some(best.map_or(t, |b: Instant| b.min(t)));
        };
        if self.mouse.is_animating() {
            consider(now + tick);
        }
        for w in &self.wheel_release {
            consider(w.1);
        }
        for m in &self.macros {
            consider(m.next.max(now));
        }
        for h in &self.held {
            if let CAction::Button { mode: ButtonMode::Turbo, half, .. } = h.action {
                let el = now.saturating_duration_since(h.since).as_nanos();
                let hn = half.as_nanos().max(1);
                let next = h.since + Duration::from_nanos(((el / hn + 1) * hn) as u64);
                consider(next);
            }
        }
        best
    }

    fn set_timer(&mut self, on: bool) {
        if on != self.timer_hi {
            self.timer_hi = on;
            self.backend.set_high_res_timer(on);
        }
    }

    // ------------------------------------------------------------ output --

    fn compute(&mut self, now: Instant) -> PadState {
        let mut s = PadState { buttons: self.toggled_buttons, ..Default::default() };
        let socd = self.config.settings.socd;
        // Per stick: accumulated vector and per-axis latest (seq, value).
        let mut sum = [(0.0f32, 0.0f32); 2];
        let mut last_x = [(0u64, 0.0f32); 2];
        let mut last_y = [(0u64, 0.0f32); 2];
        let mut any = [false; 2];
        let mut scale = [1.0f32; 2];
        for h in &self.held {
            match h.action {
                CAction::Button { ctl, mode: ButtonMode::Hold, .. } => s.buttons |= ctl.bit(),
                CAction::Button { ctl, mode: ButtonMode::Turbo, half } => {
                    let el = now.saturating_duration_since(h.since).as_nanos();
                    if (el / half.as_nanos().max(1)).is_multiple_of(2) {
                        s.buttons |= ctl.bit();
                    }
                }
                CAction::Trigger { side, value } => match side {
                    Side::Left => s.lt = s.lt.max(value),
                    Side::Right => s.rt = s.rt.max(value),
                },
                CAction::Stick { side, x, y } => {
                    let i = side as usize;
                    any[i] = true;
                    sum[i].0 += x;
                    sum[i].1 += y;
                    if x != 0.0 && h.seq > last_x[i].0 {
                        last_x[i] = (h.seq, x);
                    }
                    if y != 0.0 && h.seq > last_y[i].0 {
                        last_y[i] = (h.seq, y);
                    }
                }
                CAction::StickScale { side, factor } => scale[side as usize] *= factor,
                _ => {}
            }
        }
        for m in &self.macros {
            s.buttons |= m.pressed;
        }
        let mut sticks = [(0.0f32, 0.0f32); 2];
        for i in 0..2 {
            if !any[i] {
                continue;
            }
            let (mut x, mut y) = match socd {
                Socd::Neutral => sum[i],
                Socd::LastWins => (last_x[i].1, last_y[i].1),
            };
            x = x.clamp(-1.0, 1.0);
            y = y.clamp(-1.0, 1.0);
            let m = (x * x + y * y).sqrt();
            if m > 1.0 {
                x /= m;
                y /= m;
            }
            sticks[i] = (x * scale[i], y * scale[i]);
        }
        let mcfg = &self.profiles[self.current].mouse;
        if mcfg.target != StickTarget::None {
            let (mx, my) = self.mouse.output(now, mcfg);
            let i = if mcfg.target == StickTarget::Left { 0 } else { 1 };
            let (mut x, mut y) = (sticks[i].0 + mx, sticks[i].1 + my);
            let m = (x * x + y * y).sqrt();
            if m > 1.0 {
                x /= m;
                y /= m;
            }
            sticks[i] = (x, y);
        }
        s.lx = sticks[0].0;
        s.ly = sticks[0].1;
        s.rx = sticks[1].0;
        s.ry = sticks[1].1;
        s
    }

    fn output(&mut self, now: Instant, origin: Option<Instant>) {
        if !self.active {
            self.set_timer(false);
            return;
        }
        self.advance_timers(now);
        let s = self.compute(now);
        if self.last_sent != Some(s) {
            match self.backend.pad_send(&s) {
                Ok(()) => {
                    self.last_sent = Some(s);
                    self.shared.stats.reports.fetch_add(1, Relaxed);
                    if let Some(o) = origin {
                        let us = Instant::now().saturating_duration_since(o).as_micros().min(u32::MAX as u128) as u32;
                        self.shared.record_latency(us);
                    }
                    let label = self.mouse_src_label;
                    // Only rebuild layer names (allocates) when the mask changed.
                    let mask = self.active_layers_mask();
                    let names = if mask != self.status_layer_mask {
                        self.status_layer_mask = mask;
                        Some(self.active_layer_names())
                    } else {
                        None
                    };
                    self.shared.status_mut(|st| {
                        st.pad = s;
                        st.mouse_source = label;
                        if let Some(n) = names {
                            st.active_layers = n;
                        }
                    });
                }
                Err(e) => {
                    self.shared.status_mut(|st| st.pad_error = Some(e));
                }
            }
        }
        let needs_timer = self.next_deadline(now).is_some();
        self.set_timer(needs_timer);
    }

    /// Run the engine loop until shutdown.
    pub fn run(mut self, rx: Receiver<Msg>) -> B {
        loop {
            let now = Instant::now();
            let msg = match self.next_deadline(now) {
                Some(d) if d <= now => None,
                Some(d) => match rx.recv_timeout(d - now) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => Some(Msg::Shutdown),
                },
                None => Some(rx.recv().unwrap_or(Msg::Shutdown)),
            };
            let now = Instant::now();
            match msg {
                Some(m) => {
                    if !self.handle(m, now) {
                        break;
                    }
                    // Drain whatever else is queued before the next wait so a
                    // burst of events results in a single report.
                    while let Ok(m) = rx.try_recv() {
                        if !self.handle(m, Instant::now()) {
                            return self.backend;
                        }
                    }
                }
                None => self.output(now, None),
            }
        }
        self.backend
    }
}

// ------------------------------------------------------------------- tests --

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Bind, Layer};

    #[derive(Default)]
    struct Mock {
        connected: bool,
        sent: Vec<PadState>,
        fail_connect: bool,
    }
    impl Backend for Mock {
        fn pad_connect(&mut self, _k: ControllerKind) -> Result<(), String> {
            if self.fail_connect {
                return Err("ViGEmBus not installed".into());
            }
            self.connected = true;
            Ok(())
        }
        fn pad_disconnect(&mut self) {
            self.connected = false;
        }
        fn pad_connected(&self) -> bool {
            self.connected
        }
        fn pad_send(&mut self, s: &PadState) -> Result<(), String> {
            self.sent.push(*s);
            Ok(())
        }
    }

    fn engine(cfg: Config) -> (Engine<Mock>, Instant) {
        let shared = Arc::new(Shared::default());
        let mut e = Engine::new(Mock::default(), shared, cfg);
        let t = Instant::now();
        e.handle(Msg::Focus { exe: "game.exe".into(), own: false }, t);
        e.handle(Msg::SetEnabled(true), t);
        (e, t)
    }

    fn key(e: &mut Engine<Mock>, name: &str, down: bool, t: Instant) {
        let id = keys::parse_input(name).unwrap();
        e.handle(Msg::Input { id, down, t }, t);
    }

    fn last(e: &Engine<Mock>) -> PadState {
        *e.backend.sent.last().unwrap()
    }

    fn ms(t: Instant, m: u64) -> Instant {
        t + Duration::from_millis(m)
    }

    #[test]
    fn hold_button_and_block_table() {
        let (mut e, t) = engine(Config::default());
        assert!(e.active);
        assert!(e.shared.is_blocked(keys::parse_input("Space").unwrap()));
        assert!(!e.shared.is_blocked(keys::parse_input("P").unwrap()));
        key(&mut e, "Space", true, t);
        assert!(last(&e).pressed(Control::A));
        let n = e.backend.sent.len();
        key(&mut e, "Space", true, t); // autorepeat: no new report
        assert_eq!(e.backend.sent.len(), n);
        key(&mut e, "Space", false, t);
        assert!(!last(&e).pressed(Control::A));
    }

    #[test]
    fn wasd_last_wins_and_walk_modifier() {
        let (mut e, t) = engine(Config::default());
        key(&mut e, "A", true, t);
        assert_eq!((last(&e).lx, last(&e).ly), (-1.0, 0.0));
        key(&mut e, "D", true, t);
        assert_eq!(last(&e).lx, 1.0); // last wins
        key(&mut e, "D", false, t);
        assert_eq!(last(&e).lx, -1.0);
        key(&mut e, "W", true, t);
        let s = last(&e);
        assert!(((s.lx * s.lx + s.ly * s.ly).sqrt() - 1.0).abs() < 1e-5); // diagonal normalised
        key(&mut e, "LAlt", true, t);
        let s = last(&e);
        assert!(((s.lx * s.lx + s.ly * s.ly).sqrt() - 0.5).abs() < 1e-5);
    }

    #[test]
    fn socd_neutral() {
        let mut c = Config::default();
        c.settings.socd = Socd::Neutral;
        let (mut e, t) = engine(c);
        key(&mut e, "A", true, t);
        key(&mut e, "D", true, t);
        assert_eq!(last(&e).lx, 0.0);
    }

    #[test]
    fn layer_hold_changes_bindings_and_release_pairs_with_press() {
        let (mut e, t) = engine(Config::default());
        key(&mut e, "1", true, t);
        assert!(last(&e).pressed(Control::Y));
        key(&mut e, "CapsLock", true, t); // Alt layer while 1 still held
        assert!(e.active_layer_names().contains(&"Alt".to_string()));
        key(&mut e, "1", false, t); // must release Y (the action it pressed)
        assert!(!last(&e).pressed(Control::Y));
        key(&mut e, "1", true, t);
        assert!(last(&e).pressed(Control::DpadUp));
        key(&mut e, "CapsLock", false, t);
        key(&mut e, "1", false, t);
        assert!(!last(&e).pressed(Control::DpadUp));
    }

    #[test]
    fn turbo_alternates_and_schedules() {
        let (mut e, t) = engine(Config::default());
        key(&mut e, "CapsLock", true, t);
        key(&mut e, "Space", true, t); // turbo A at 15 Hz -> 33.3 ms half period
        assert!(last(&e).pressed(Control::A));
        let d = e.next_deadline(t).unwrap();
        assert!(d > t && d <= ms(t, 34));
        e.output(ms(t, 40), None);
        assert!(!last(&e).pressed(Control::A));
        e.output(ms(t, 70), None);
        assert!(last(&e).pressed(Control::A));
        key(&mut e, "Space", false, ms(t, 75));
        assert!(!last(&e).pressed(Control::A));
    }

    #[test]
    fn macro_runs_to_completion() {
        let (mut e, t) = engine(Config::default());
        key(&mut e, "CapsLock", true, t);
        key(&mut e, "F", true, t); // tap B (30ms), wait 60, tap B
        key(&mut e, "F", false, t);
        assert!(last(&e).pressed(Control::B));
        e.output(ms(t, 31), None);
        assert!(!last(&e).pressed(Control::B));
        e.output(ms(t, 95), None);
        assert!(last(&e).pressed(Control::B));
        e.output(ms(t, 130), None);
        assert!(!last(&e).pressed(Control::B));
        assert!(e.macros.is_empty());
        assert_eq!(e.next_deadline(ms(t, 130)), None);
    }

    #[test]
    fn wheel_press_auto_releases() {
        let (mut e, t) = engine(Config::default());
        e.handle(Msg::Wheel { id: keys::WHEEL_UP, t }, t);
        assert!(last(&e).pressed(Control::Y));
        e.output(ms(t, 41), None);
        assert!(!last(&e).pressed(Control::Y));
    }

    #[test]
    fn mouse_moves_right_stick_and_decays() {
        let (mut e, t) = engine(Config::default());
        e.handle(Msg::MouseDelta { dx: 10, dy: 0, raw: false, t }, t);
        assert!(last(&e).rx > 0.2);
        assert!(e.next_deadline(t).is_some());
        e.output(ms(t, 200), None);
        assert_eq!(last(&e).rx, 0.0);
        assert_eq!(e.next_deadline(ms(t, 200)), None, "idle engine must not tick");
    }

    #[test]
    fn auto_mouse_source_prefers_raw() {
        let (mut e, t) = engine(Config::default());
        assert!(e.mouse_delta(5, 0, true, t));
        assert!(!e.mouse_delta(5, 0, false, t));
        assert!(e.mouse_delta(5, 0, false, ms(t, 1500)));
    }

    #[test]
    fn disable_toggle_releases_everything() {
        let (mut e, t) = engine(Config::default());
        key(&mut e, "Mouse1", true, t);
        assert_eq!(last(&e).triggers().1, 1.0);
        e.handle(Msg::ToggleEmulation, t);
        assert!(!e.active);
        assert_eq!(last(&e), PadState::default());
        assert!(!e.shared.is_blocked(keys::MOUSE_LEFT));
        assert!(!e.shared.capture_mouse.load(Relaxed));
    }

    #[test]
    fn own_window_and_unmatched_suspend() {
        let mut c = Config::default();
        c.settings.suspend_when_unmatched = true;
        let p2 = Profile {
            name: "Elden".into(),
            executables: vec!["eldenring.exe".into()],
            layers: vec![Layer { name: "Base".into(), binds: vec![Bind { input: "K".into(), action: Action::Button { button: Control::B, mode: ButtonMode::Hold, turbo_hz: 10.0 }, block: true }] }],
            ..Default::default()
        };
        c.profiles.push(p2);
        let (mut e, t) = engine(c);
        assert!(!e.active, "game.exe matches nothing");
        e.handle(Msg::Focus { exe: "eldenring.exe".into(), own: false }, t);
        assert!(e.active);
        assert_eq!(e.cur().name, "Elden");
        key(&mut e, "K", true, t);
        assert!(last(&e).pressed(Control::B));
        e.handle(Msg::Focus { exe: "padforge.exe".into(), own: true }, t);
        assert!(!e.active);
        assert_eq!(last(&e), PadState::default());
    }

    #[test]
    fn missing_driver_keeps_inactive_with_error() {
        let shared = Arc::new(Shared::default());
        let mut e = Engine::new(Mock { fail_connect: true, ..Default::default() }, shared.clone(), Config::default());
        e.handle(Msg::SetEnabled(true), Instant::now());
        assert!(!e.active);
        assert!(shared.status_snapshot().pad_error.is_some());
        assert!(!shared.active.load(Relaxed));
    }

    #[test]
    fn toggle_button_mode() {
        let mut c = Config::default();
        c.profiles[0].layers[0].binds.push(Bind { input: "T".into(), action: Action::Button { button: Control::Ls, mode: ButtonMode::Toggle, turbo_hz: 10.0 }, block: true });
        let (mut e, t) = engine(c);
        key(&mut e, "T", true, t);
        key(&mut e, "T", false, t);
        assert!(last(&e).pressed(Control::Ls));
        key(&mut e, "T", true, t);
        key(&mut e, "T", false, t);
        assert!(!last(&e).pressed(Control::Ls));
    }

    #[test]
    fn config_errors_reported() {
        let mut c = Config::default();
        c.profiles[0].layers[0].binds.push(Bind { input: "Nope".into(), action: Action::Disable, block: true });
        c.profiles[0].layers[0].binds.push(Bind { input: "J".into(), action: Action::Layer { layer: "Missing".into(), mode: LayerMode::Hold }, block: true });
        let (e, _) = engine(c);
        let errs = e.shared.status_snapshot().config_errors;
        assert_eq!(errs.len(), 2, "{errs:?}");
    }
}
