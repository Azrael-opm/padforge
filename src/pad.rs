//! Platform-neutral virtual controller model plus exact Xbox 360 / DualShock 4
//! report encodings. Everything here is pure so it can be unit tested anywhere.

use serde::{Deserialize, Serialize};

/// Every digital control a binding can drive. `Lt`/`Rt` act as fully pressed
/// triggers when used as a button; use the `trigger` action for analog values.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    #[serde(alias = "cross")]
    A,
    #[serde(alias = "circle")]
    B,
    #[serde(alias = "square")]
    X,
    #[serde(alias = "triangle")]
    Y,
    #[serde(alias = "l1")]
    Lb,
    #[serde(alias = "r1")]
    Rb,
    #[serde(alias = "l3")]
    Ls,
    #[serde(alias = "r3")]
    Rs,
    #[serde(alias = "share")]
    Back,
    #[serde(alias = "options")]
    Start,
    #[serde(alias = "ps")]
    Guide,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    /// DualShock 4 touchpad click (ignored on Xbox 360).
    Touchpad,
    #[serde(alias = "l2")]
    Lt,
    #[serde(alias = "r2")]
    Rt,
}

impl Control {
    pub const ALL: [Control; 18] = [
        Control::A,
        Control::B,
        Control::X,
        Control::Y,
        Control::Lb,
        Control::Rb,
        Control::Ls,
        Control::Rs,
        Control::Back,
        Control::Start,
        Control::Guide,
        Control::DpadUp,
        Control::DpadDown,
        Control::DpadLeft,
        Control::DpadRight,
        Control::Touchpad,
        Control::Lt,
        Control::Rt,
    ];

    /// Config name (snake_case), e.g. `dpad_up`.
    pub fn name(self) -> String {
        match toml::Value::try_from(self) {
            Ok(toml::Value::String(s)) => s,
            _ => format!("{self:?}").to_ascii_lowercase(),
        }
    }

    /// Parse a config name or alias (`cross`, `r2`, `ps`, ...), case-insensitive.
    pub fn parse(s: &str) -> Option<Control> {
        use serde::de::{value::StrDeserializer, IntoDeserializer};
        let lower = s.trim().to_ascii_lowercase();
        let de: StrDeserializer<'_, serde::de::value::Error> = lower.as_str().into_deserializer();
        Control::deserialize(de).ok()
    }

    #[inline]
    pub fn bit(self) -> u32 {
        1u32 << (self as u32)
    }

    pub fn label(self, kind: ControllerKind) -> &'static str {
        use Control::*;
        let ds4 = kind == ControllerKind::Ds4;
        match self {
            A => if ds4 { "Cross" } else { "A" },
            B => if ds4 { "Circle" } else { "B" },
            X => if ds4 { "Square" } else { "X" },
            Y => if ds4 { "Triangle" } else { "Y" },
            Lb => if ds4 { "L1" } else { "LB" },
            Rb => if ds4 { "R1" } else { "RB" },
            Ls => if ds4 { "L3" } else { "LS (click)" },
            Rs => if ds4 { "R3" } else { "RS (click)" },
            Back => if ds4 { "Share" } else { "Back" },
            Start => if ds4 { "Options" } else { "Start" },
            Guide => if ds4 { "PS" } else { "Guide" },
            DpadUp => "D-pad Up",
            DpadDown => "D-pad Down",
            DpadLeft => "D-pad Left",
            DpadRight => "D-pad Right",
            Touchpad => "Touchpad",
            Lt => if ds4 { "L2" } else { "LT" },
            Rt => if ds4 { "R2" } else { "RT" },
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ControllerKind {
    #[default]
    #[serde(alias = "x360", alias = "xbox")]
    Xbox360,
    #[serde(alias = "dualshock4", alias = "ps4")]
    Ds4,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

/// Normalised controller state. Sticks use +y = up, range [-1, 1].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PadState {
    /// Bitset of `Control::bit()` (Lt/Rt bits are folded into triggers on encode).
    pub buttons: u32,
    pub lt: f32,
    pub rt: f32,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
}

impl PadState {
    #[inline]
    pub fn pressed(&self, c: Control) -> bool {
        self.buttons & c.bit() != 0
    }
    /// Effective trigger values including digital Lt/Rt presses.
    #[inline]
    pub fn triggers(&self) -> (f32, f32) {
        let lt = if self.pressed(Control::Lt) { 1.0 } else { self.lt };
        let rt = if self.pressed(Control::Rt) { 1.0 } else { self.rt };
        (lt.clamp(0.0, 1.0), rt.clamp(0.0, 1.0))
    }
}

/// Raw XInput gamepad fields (identical layout to XINPUT_GAMEPAD).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct X360Report {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

/// DualShock 4 input report fields (as accepted by ViGEmBus DS4_REPORT).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ds4Report {
    pub thumb_lx: u8,
    pub thumb_ly: u8,
    pub thumb_rx: u8,
    pub thumb_ry: u8,
    pub buttons: u16,
    pub special: u8,
    pub trigger_l: u8,
    pub trigger_r: u8,
}

impl Default for Ds4Report {
    fn default() -> Self {
        Ds4Report {
            thumb_lx: 0x80,
            thumb_ly: 0x80,
            thumb_rx: 0x80,
            thumb_ry: 0x80,
            buttons: 0x8, // d-pad neutral
            special: 0,
            trigger_l: 0,
            trigger_r: 0,
        }
    }
}

#[inline]
fn axis_i16(v: f32) -> i16 {
    let v = v.clamp(-1.0, 1.0);
    if v >= 0.0 {
        (v * 32767.0).round() as i16
    } else {
        (v * 32768.0).round().max(-32768.0) as i16
    }
}

#[inline]
fn unit_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// [-1,1] -> [0,255] with 128 as centre (DS4 axes).
#[inline]
fn axis_u8(v: f32) -> u8 {
    let v = v.clamp(-1.0, 1.0);
    if v >= 0.0 {
        (128.0 + v * 127.0).round() as u8
    } else {
        (128.0 + v * 128.0).round() as u8
    }
}

pub fn encode_x360(s: &PadState) -> X360Report {
    use Control::*;
    const MAP: [(Control, u16); 14] = [
        (DpadUp, 0x0001),
        (DpadDown, 0x0002),
        (DpadLeft, 0x0004),
        (DpadRight, 0x0008),
        (Start, 0x0010),
        (Back, 0x0020),
        (Ls, 0x0040),
        (Rs, 0x0080),
        (Lb, 0x0100),
        (Rb, 0x0200),
        (Guide, 0x0400),
        (A, 0x1000),
        (B, 0x2000),
        (X, 0x4000),
    ];
    let mut b = 0u16;
    for (c, bit) in MAP {
        if s.pressed(c) {
            b |= bit;
        }
    }
    if s.pressed(Y) {
        b |= 0x8000;
    }
    let (lt, rt) = s.triggers();
    X360Report {
        buttons: b,
        left_trigger: unit_u8(lt),
        right_trigger: unit_u8(rt),
        thumb_lx: axis_i16(s.lx),
        thumb_ly: axis_i16(s.ly),
        thumb_rx: axis_i16(s.rx),
        thumb_ry: axis_i16(s.ry),
    }
}

pub fn encode_ds4(s: &PadState) -> Ds4Report {
    use Control::*;
    let up = s.pressed(DpadUp);
    let down = s.pressed(DpadDown);
    let left = s.pressed(DpadLeft);
    let right = s.pressed(DpadRight);
    // Opposing directions cancel (a real d-pad can't report both).
    let v = (up as i8) - (down as i8);
    let h = (right as i8) - (left as i8);
    let hat: u16 = match (v, h) {
        (1, 0) => 0,
        (1, 1) => 1,
        (0, 1) => 2,
        (-1, 1) => 3,
        (-1, 0) => 4,
        (-1, -1) => 5,
        (0, -1) => 6,
        (1, -1) => 7,
        _ => 8,
    };
    let (lt, rt) = s.triggers();
    let mut b = hat;
    let map: [(bool, u16); 12] = [
        (s.pressed(X), 0x0010),
        (s.pressed(A), 0x0020),
        (s.pressed(B), 0x0040),
        (s.pressed(Y), 0x0080),
        (s.pressed(Lb), 0x0100),
        (s.pressed(Rb), 0x0200),
        (lt > 0.0, 0x0400),
        (rt > 0.0, 0x0800),
        (s.pressed(Back), 0x1000),
        (s.pressed(Start), 0x2000),
        (s.pressed(Ls), 0x4000),
        (s.pressed(Rs), 0x8000),
    ];
    for (on, bit) in map {
        if on {
            b |= bit;
        }
    }
    let mut special = 0u8;
    if s.pressed(Guide) {
        special |= 0x01;
    }
    if s.pressed(Touchpad) {
        special |= 0x02;
    }
    Ds4Report {
        // DS4 Y axes grow downwards.
        thumb_lx: axis_u8(s.lx),
        thumb_ly: axis_u8(-s.ly),
        thumb_rx: axis_u8(s.rx),
        thumb_ry: axis_u8(-s.ry),
        buttons: b,
        special,
        trigger_l: unit_u8(lt),
        trigger_r: unit_u8(rt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_names_roundtrip() {
        for c in Control::ALL {
            assert_eq!(Control::parse(&c.name()), Some(c));
        }
        assert_eq!(Control::parse("Cross"), Some(Control::A));
        assert_eq!(Control::parse("r2"), Some(Control::Rt));
        assert_eq!(Control::parse("nope"), None);
    }

    #[test]
    fn x360_neutral_and_extremes() {
        let r = encode_x360(&PadState::default());
        assert_eq!(r, X360Report::default());
        let s = PadState { lx: 1.0, ly: -1.0, rx: 0.5, buttons: Control::A.bit() | Control::Y.bit(), rt: 1.0, ..Default::default() };
        let r = encode_x360(&s);
        assert_eq!(r.thumb_lx, 32767);
        assert_eq!(r.thumb_ly, -32768);
        assert_eq!(r.thumb_rx, 16384);
        assert_eq!(r.buttons, 0x1000 | 0x8000);
        assert_eq!(r.right_trigger, 255);
    }

    #[test]
    fn digital_triggers_fold_into_analog() {
        let s = PadState { buttons: Control::Lt.bit(), ..Default::default() };
        assert_eq!(encode_x360(&s).left_trigger, 255);
        let d = encode_ds4(&s);
        assert_eq!(d.trigger_l, 255);
        assert_eq!(d.buttons & 0x0400, 0x0400);
    }

    #[test]
    fn ds4_axes_and_hat() {
        assert_eq!(encode_ds4(&PadState::default()), Ds4Report::default());
        let s = PadState { ly: 1.0, lx: -1.0, buttons: Control::DpadUp.bit() | Control::DpadRight.bit() | Control::Guide.bit(), ..Default::default() };
        let d = encode_ds4(&s);
        assert_eq!(d.thumb_ly, 0); // up = 0 on DS4
        assert_eq!(d.thumb_lx, 0);
        assert_eq!(d.buttons & 0xF, 1); // NE
        assert_eq!(d.special, 1);
        let s = PadState { buttons: Control::DpadUp.bit() | Control::DpadDown.bit(), ..Default::default() };
        assert_eq!(encode_ds4(&s).buttons & 0xF, 8); // cancelled
    }
}
