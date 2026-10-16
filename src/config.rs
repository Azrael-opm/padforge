//! User-facing configuration (TOML). Human-editable, also edited by the GUI.

use crate::pad::{Control, ControllerKind, Side};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub settings: Settings,
    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Config { settings: Settings::default(), profiles: vec![Profile::default_fps()] }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseSource {
    /// Prefer unaccelerated Raw Input deltas if Windows still delivers them while
    /// moves are blocked; otherwise fall back to hook-derived deltas.
    #[default]
    Auto,
    Raw,
    Hook,
}

/// Opposing directions held at once (e.g. A + D).
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Socd {
    /// Opposites cancel to neutral.
    Neutral,
    /// The most recently pressed direction wins.
    #[default]
    LastWins,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Global hotkey that toggles emulation on/off (always intercepted).
    pub toggle_hotkey: String,
    pub start_enabled: bool,
    pub controller: ControllerKind,
    /// Upper bound for timed updates (mouse decay, turbo, macros). Key and mouse
    /// events are forwarded immediately regardless of this rate.
    pub output_rate_hz: u32,
    pub mouse_source: MouseSource,
    /// Pass input through untouched when the focused app matches no profile's
    /// executable list (the first profile is the fallback otherwise).
    pub suspend_when_unmatched: bool,
    /// Re-centre the (frozen) cursor if it sits near a screen/clip edge, which
    /// would otherwise clamp hook-derived mouse deltas.
    pub recenter_cursor: bool,
    /// How long a wheel notch holds its bound control.
    pub wheel_press_ms: u32,
    /// Plug the virtual controller in at startup and keep it connected, so
    /// games don't see a disconnect every time emulation is toggled.
    pub keep_connected: bool,
    pub high_priority: bool,
    pub socd: Socd,
    /// Closing the window keeps PadForge running in the system tray.
    pub close_to_tray: bool,
    /// Start hidden in the tray.
    pub start_in_tray: bool,
    /// Short system sound when emulation is toggled (handy in fullscreen).
    pub toggle_sound: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            toggle_hotkey: "F8".into(),
            start_enabled: false,
            controller: ControllerKind::Xbox360,
            output_rate_hz: 1000,
            mouse_source: MouseSource::Auto,
            suspend_when_unmatched: false,
            recenter_cursor: true,
            wheel_press_ms: 40,
            keep_connected: true,
            high_priority: true,
            socd: Socd::LastWins,
            close_to_tray: true,
            start_in_tray: false,
            toggle_sound: true,
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StickTarget {
    None,
    Left,
    #[default]
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MouseConfig {
    pub target: StickTarget,
    /// Stick deflection per (count / ms) of mouse speed.
    pub sensitivity: f32,
    /// Vertical sensitivity multiplier.
    pub y_ratio: f32,
    pub invert_y: bool,
    /// Response curve exponent: <1 boosts small movements, >1 adds precision.
    pub exponent: f32,
    /// Compensates the game's inner dead zone: any movement starts at this
    /// deflection so tiny mouse motions still register.
    pub anti_deadzone: f32,
    /// Velocity averaging window. Lower = snappier, higher = smoother.
    pub smoothing_ms: f32,
    /// Stick returns to centre this long after the mouse stops.
    pub release_ms: f32,
}

impl Default for MouseConfig {
    fn default() -> Self {
        MouseConfig {
            target: StickTarget::Right,
            sensitivity: 0.18,
            y_ratio: 1.0,
            invert_y: false,
            exponent: 0.85,
            anti_deadzone: 0.20,
            smoothing_ms: 6.0,
            release_ms: 12.0,
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonMode {
    #[default]
    Hold,
    /// Press once to latch, press again to release.
    Toggle,
    /// Rapid repeat while held.
    Turbo,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LayerMode {
    /// Layer is active while the key is held.
    #[default]
    Hold,
    /// Press to switch the layer on, press again to switch it off.
    Toggle,
}

/// One step of a macro. `tap` presses for the macro's `tap_ms` then releases.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MacroStep {
    Press(Control),
    Release(Control),
    Tap(Control),
    Wait(u32),
}

fn default_one() -> f32 {
    1.0
}
fn default_turbo() -> f32 {
    12.0
}
fn default_tap() -> u32 {
    30
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Press a controller button (or LT/RT fully).
    Button {
        button: Control,
        #[serde(default)]
        mode: ButtonMode,
        #[serde(default = "default_turbo")]
        turbo_hz: f32,
    },
    /// Analog trigger at a fixed value (0..1).
    Trigger {
        trigger: Side,
        #[serde(default = "default_one")]
        value: f32,
    },
    /// Push a stick in a direction; held directions are combined.
    Stick { stick: Side, x: f32, y: f32 },
    /// Scale a stick while held (e.g. walk modifier = 0.5).
    StickScale { stick: Side, factor: f32 },
    /// Activate another layer (bindings fall through to lower layers).
    Layer {
        layer: String,
        #[serde(default)]
        mode: LayerMode,
    },
    /// Play a sequence. `repeat` loops while the key is held.
    Macro {
        steps: Vec<MacroStep>,
        #[serde(default)]
        repeat: bool,
        #[serde(default = "default_tap")]
        tap_ms: u32,
    },
    /// Swallow the key without doing anything.
    Disable,
}

impl Action {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Action::Button { .. } => "Button",
            Action::Trigger { .. } => "Trigger",
            Action::Stick { .. } => "Stick",
            Action::StickScale { .. } => "Stick scale",
            Action::Layer { .. } => "Layer",
            Action::Macro { .. } => "Macro",
            Action::Disable => "Disable",
        }
    }
}

/// Compact text form used by the GUI: `tap b, wait 60, press a, release a`.
pub fn macro_to_text(steps: &[MacroStep]) -> String {
    steps
        .iter()
        .map(|s| match s {
            MacroStep::Press(c) => format!("press {}", c.name()),
            MacroStep::Release(c) => format!("release {}", c.name()),
            MacroStep::Tap(c) => format!("tap {}", c.name()),
            MacroStep::Wait(ms) => format!("wait {ms}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn macro_from_text(text: &str) -> Result<Vec<MacroStep>, String> {
    let mut out = Vec::new();
    for raw in text.split([',', ';', '\n']) {
        let tok = raw.trim();
        if tok.is_empty() {
            continue;
        }
        let mut it = tok.split_whitespace();
        let verb = it.next().unwrap_or("").to_ascii_lowercase();
        let arg = it.next().unwrap_or("");
        if it.next().is_some() {
            return Err(format!("'{tok}': too many words"));
        }
        let ctl = || Control::parse(arg).ok_or_else(|| format!("'{tok}': unknown button '{arg}'"));
        out.push(match verb.as_str() {
            "tap" => MacroStep::Tap(ctl()?),
            "press" | "down" => MacroStep::Press(ctl()?),
            "release" | "up" => MacroStep::Release(ctl()?),
            "wait" | "delay" => MacroStep::Wait(arg.trim_end_matches("ms").parse().map_err(|_| format!("'{tok}': wait needs milliseconds"))?),
            _ => return Err(format!("'{tok}': expected tap/press/release/wait")),
        });
    }
    Ok(out)
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bind {
    pub input: String,
    pub action: Action,
    /// Suppress the original key/mouse button so the game never sees it.
    #[serde(default = "default_true")]
    pub block: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Layer {
    pub name: String,
    #[serde(default)]
    pub binds: Vec<Bind>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    /// Executable file names (e.g. "eldenring.exe") that auto-select this profile.
    pub executables: Vec<String>,
    pub mouse: MouseConfig,
    /// Layer 0 is the base layer and always active.
    pub layers: Vec<Layer>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            name: "New profile".into(),
            executables: vec![],
            mouse: MouseConfig::default(),
            layers: vec![Layer { name: "Base".into(), binds: vec![] }],
        }
    }
}

fn b(input: &str, button: Control) -> Bind {
    Bind { input: input.into(), action: Action::Button { button, mode: ButtonMode::Hold, turbo_hz: 12.0 }, block: true }
}
fn st(input: &str, stick: Side, x: f32, y: f32) -> Bind {
    Bind { input: input.into(), action: Action::Stick { stick, x, y }, block: true }
}

impl Profile {
    /// Sensible defaults for a typical third/first-person action game.
    pub fn default_fps() -> Profile {
        use Control::*;
        let base = vec![
            st("W", Side::Left, 0.0, 1.0),
            st("S", Side::Left, 0.0, -1.0),
            st("A", Side::Left, -1.0, 0.0),
            st("D", Side::Left, 1.0, 0.0),
            Bind { input: "LAlt".into(), action: Action::StickScale { stick: Side::Left, factor: 0.5 }, block: true },
            b("Space", A),
            b("C", B),
            b("LCtrl", B),
            b("R", X),
            b("E", X),
            b("1", Y),
            b("WheelUp", Y),
            b("Mouse1", Rt),
            b("Mouse2", Lt),
            b("Q", Lb),
            b("G", Rb),
            b("LShift", Ls),
            b("V", Rs),
            b("Tab", Back),
            b("Escape", Start),
            b("Up", DpadUp),
            b("Down", DpadDown),
            b("Left", DpadLeft),
            b("Right", DpadRight),
            Bind { input: "CapsLock".into(), action: Action::Layer { layer: "Alt".into(), mode: LayerMode::Hold }, block: true },
        ];
        let alt = vec![
            b("1", DpadUp),
            b("2", DpadRight),
            b("3", DpadDown),
            b("4", DpadLeft),
            Bind {
                input: "Space".into(),
                action: Action::Button { button: A, mode: ButtonMode::Turbo, turbo_hz: 15.0 },
                block: true,
            },
            Bind {
                input: "F".into(),
                action: Action::Macro { steps: vec![MacroStep::Tap(B), MacroStep::Wait(60), MacroStep::Tap(B)], repeat: false, tap_ms: 30 },
                block: true,
            },
        ];
        Profile {
            name: "Default".into(),
            executables: vec![],
            mouse: MouseConfig::default(),
            layers: vec![Layer { name: "Base".into(), binds: base }, Layer { name: "Alt".into(), binds: alt }],
        }
    }
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Config, String> {
        let mut c: Config = toml::from_str(s).map_err(|e| e.to_string())?;
        c.normalize();
        Ok(c)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Repair values that would make the engine misbehave.
    pub fn normalize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles.push(Profile::default_fps());
        }
        let s = &mut self.settings;
        s.output_rate_hz = s.output_rate_hz.clamp(60, 2000);
        s.wheel_press_ms = s.wheel_press_ms.clamp(5, 500);
        for p in &mut self.profiles {
            if p.layers.is_empty() {
                p.layers.push(Layer { name: "Base".into(), binds: vec![] });
            }
            let m = &mut p.mouse;
            m.sensitivity = m.sensitivity.clamp(0.0, 10.0);
            m.y_ratio = m.y_ratio.clamp(0.05, 10.0);
            m.exponent = m.exponent.clamp(0.2, 3.0);
            m.anti_deadzone = m.anti_deadzone.clamp(0.0, 0.9);
            m.smoothing_ms = m.smoothing_ms.clamp(1.0, 100.0);
            m.release_ms = m.release_ms.clamp(0.0, 500.0);
            for e in &mut p.executables {
                *e = e.trim().to_ascii_lowercase();
            }
            p.executables.retain(|e| !e.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macro_text_roundtrip() {
        let steps = macro_from_text("tap Cross, wait 60ms; press rb, release rb").unwrap();
        assert_eq!(steps, vec![MacroStep::Tap(Control::A), MacroStep::Wait(60), MacroStep::Press(Control::Rb), MacroStep::Release(Control::Rb)]);
        assert_eq!(macro_from_text(&macro_to_text(&steps)).unwrap(), steps);
        assert!(macro_from_text("tap zz").is_err());
        assert!(macro_from_text("jump a").is_err());
    }

    #[test]
    fn default_roundtrips_through_toml() {
        let c = Config::default();
        let s = c.to_toml();
        let back = Config::from_toml(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn handwritten_toml_parses_with_aliases() {
        let src = r#"
            [settings]
            controller = "ps4"
            [[profiles]]
            name = "Test"
            executables = ["Game.EXE"]
            [[profiles.layers]]
            name = "Base"
            binds = [
              { input = "Space", action = { type = "button", button = "cross" } },
              { input = "F", action = { type = "macro", steps = [ { tap = "circle" }, { wait = 50 } ] } },
              { input = "W", action = { type = "stick", stick = "left", x = 0.0, y = 1.0 }, block = false },
            ]
        "#;
        let c = Config::from_toml(src).unwrap();
        assert_eq!(c.settings.controller, ControllerKind::Ds4);
        assert_eq!(c.profiles[0].executables, vec!["game.exe".to_string()]);
        let binds = &c.profiles[0].layers[0].binds;
        assert_eq!(binds[0].action, Action::Button { button: Control::A, mode: ButtonMode::Hold, turbo_hz: 12.0 });
        assert!(binds[0].block);
        assert!(!binds[2].block);
    }
}
