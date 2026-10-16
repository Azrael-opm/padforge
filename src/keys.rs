//! Physical input identifiers: Windows virtual-key codes 0..=255 plus mouse
//! buttons and wheel directions packed after them, so every input maps to a
//! dense index usable for O(1) table lookups and an atomic block bitset.

pub type InputId = u16;

pub const MOUSE_BASE: InputId = 256;
pub const MOUSE_LEFT: InputId = 256;
pub const MOUSE_RIGHT: InputId = 257;
pub const MOUSE_MIDDLE: InputId = 258;
pub const MOUSE_X1: InputId = 259;
pub const MOUSE_X2: InputId = 260;
pub const WHEEL_UP: InputId = 261;
pub const WHEEL_DOWN: InputId = 262;
pub const WHEEL_LEFT: InputId = 263;
pub const WHEEL_RIGHT: InputId = 264;
/// Total number of distinct inputs (rounded to a multiple of 64).
pub const NUM_INPUTS: usize = 320;

#[inline]
pub fn is_wheel(id: InputId) -> bool {
    (WHEEL_UP..=WHEEL_RIGHT).contains(&id)
}

static NAMED: &[(&str, InputId)] = &[
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Pause", 0x13),
    ("CapsLock", 0x14),
    ("Escape", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("PrintScreen", 0x2C),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("LWin", 0x5B),
    ("RWin", 0x5C),
    ("Menu", 0x5D),
    ("Numpad0", 0x60),
    ("Numpad1", 0x61),
    ("Numpad2", 0x62),
    ("Numpad3", 0x63),
    ("Numpad4", 0x64),
    ("Numpad5", 0x65),
    ("Numpad6", 0x66),
    ("Numpad7", 0x67),
    ("Numpad8", 0x68),
    ("Numpad9", 0x69),
    ("NumpadMultiply", 0x6A),
    ("NumpadAdd", 0x6B),
    ("NumpadSubtract", 0x6D),
    ("NumpadDecimal", 0x6E),
    ("NumpadDivide", 0x6F),
    ("NumLock", 0x90),
    ("ScrollLock", 0x91),
    ("LShift", 0xA0),
    ("RShift", 0xA1),
    ("LCtrl", 0xA2),
    ("RCtrl", 0xA3),
    ("LAlt", 0xA4),
    ("RAlt", 0xA5),
    ("Semicolon", 0xBA),
    ("Equals", 0xBB),
    ("Comma", 0xBC),
    ("Minus", 0xBD),
    ("Period", 0xBE),
    ("Slash", 0xBF),
    ("Backquote", 0xC0),
    ("LBracket", 0xDB),
    ("Backslash", 0xDC),
    ("RBracket", 0xDD),
    ("Quote", 0xDE),
    ("Mouse1", MOUSE_LEFT),
    ("Mouse2", MOUSE_RIGHT),
    ("Mouse3", MOUSE_MIDDLE),
    ("Mouse4", MOUSE_X1),
    ("Mouse5", MOUSE_X2),
    ("WheelUp", WHEEL_UP),
    ("WheelDown", WHEEL_DOWN),
    ("WheelLeft", WHEEL_LEFT),
    ("WheelRight", WHEEL_RIGHT),
];

static ALIASES: &[(&str, InputId)] = &[
    ("lmb", MOUSE_LEFT),
    ("mouseleft", MOUSE_LEFT),
    ("rmb", MOUSE_RIGHT),
    ("mouseright", MOUSE_RIGHT),
    ("mmb", MOUSE_MIDDLE),
    ("mousemiddle", MOUSE_MIDDLE),
    ("esc", 0x1B),
    ("return", 0x0D),
    ("shift", 0xA0),
    ("ctrl", 0xA2),
    ("control", 0xA2),
    ("alt", 0xA4),
    ("arrowup", 0x26),
    ("arrowdown", 0x28),
    ("arrowleft", 0x25),
    ("arrowright", 0x27),
];

/// Canonical display/config name for an input.
pub fn input_name(id: InputId) -> String {
    if (0x30..=0x39).contains(&id) || (0x41..=0x5A).contains(&id) {
        return (id as u8 as char).to_string();
    }
    if (0x70..=0x87).contains(&id) {
        return format!("F{}", id - 0x70 + 1);
    }
    if let Some((n, _)) = NAMED.iter().find(|(_, v)| *v == id) {
        return (*n).to_string();
    }
    if id < 256 {
        format!("VK{:#04X}", id)
    } else {
        format!("Input{}", id)
    }
}

/// Parse a config name (case-insensitive) into an input id.
pub fn parse_input(name: &str) -> Option<InputId> {
    let n = name.trim();
    if n.is_empty() {
        return None;
    }
    let lower = n.to_ascii_lowercase();
    if n.chars().count() == 1 {
        let c = n.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as InputId);
        }
    }
    if let Some(rest) = lower.strip_prefix('f') {
        if let Ok(k) = rest.parse::<u16>() {
            if (1..=24).contains(&k) {
                return Some(0x70 + k - 1);
            }
        }
    }
    if let Some(hex) = lower.strip_prefix("vk") {
        let hex = hex.trim_start_matches("0x");
        if let Ok(v) = u16::from_str_radix(hex, 16) {
            if v < 256 {
                return Some(v);
            }
        }
    }
    NAMED
        .iter()
        .chain(ALIASES.iter())
        .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
        .map(|(_, v)| *v)
}

/// Inputs offered in the GUI's picker (besides click-to-capture).
pub fn all_named_inputs() -> Vec<InputId> {
    let mut v: Vec<InputId> = (0x41..=0x5A).collect();
    v.extend(0x30..=0x39);
    v.extend(0x70..=0x7B);
    v.extend(NAMED.iter().map(|(_, id)| *id));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_names() {
        for id in all_named_inputs() {
            let n = input_name(id);
            assert_eq!(parse_input(&n), Some(id), "{n}");
        }
        assert_eq!(parse_input("w"), Some(0x57));
        assert_eq!(parse_input("lmb"), Some(MOUSE_LEFT));
        assert_eq!(parse_input("f12"), Some(0x7B));
        assert_eq!(parse_input("vk0xA0"), Some(0xA0));
        assert_eq!(parse_input("nope"), None);
    }
}
