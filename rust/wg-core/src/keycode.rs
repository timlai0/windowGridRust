//! Typed wrappers for the numeric enums stored in `WindowGrid.settings`.
//!
//! The legacy file stores enums by name (`"KEY_SPACE"`, `"Resize"`, `"Control, Shift"`)
//! and the legacy reader accepted plain integers too (`Enum.TryParse`), so parsing here
//! accepts both. Values that have no name are kept and written back as numbers.

use crate::keycode_table::KEY_TABLE;
use core::fmt;

/// Parses a (possibly signed) decimal integer, or returns `None` if `s` is not one.
fn parse_int(s: &str) -> Option<i32> {
    let first = s.chars().next()?;
    if first.is_ascii_digit() || first == '-' || first == '+' {
        s.parse::<i32>().ok()
    } else {
        None
    }
}

/// Every key and mouse-button name that settings files may use, in table order.
pub fn all_key_names() -> impl Iterator<Item = &'static str> {
    KEY_TABLE.iter().map(|(n, _)| *n)
}

/// A key or mouse-button code (scan code, or 257-261 for mouse buttons).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct KeyCode(pub i32);

impl KeyCode {
    pub const UNASSIGNED: KeyCode = KeyCode(0);
    pub const ESCAPE: KeyCode = KeyCode(1);
    pub const LCONTROL: KeyCode = KeyCode(29);
    pub const SPACE: KeyCode = KeyCode(57);
    pub const MOUSE_LBUTTON: KeyCode = KeyCode(257);
    pub const MOUSE_RBUTTON: KeyCode = KeyCode(258);
    pub const MOUSE_MBUTTON: KeyCode = KeyCode(259);
    pub const MOUSE_X1BUTTON: KeyCode = KeyCode(260);
    pub const MOUSE_X2BUTTON: KeyCode = KeyCode(261);

    /// The enum name, if this value has one.
    pub fn name(self) -> Option<&'static str> {
        KEY_TABLE.iter().find(|(_, v)| *v == self.0).map(|(n, _)| *n)
    }

    /// Parses a name (case-sensitive, like `Enum.TryParse`) or an integer.
    pub fn parse(s: &str) -> Option<KeyCode> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Some(v) = parse_int(s) {
            return Some(KeyCode(v));
        }
        KEY_TABLE.iter().find(|(n, _)| *n == s).map(|(_, v)| KeyCode(*v))
    }

    pub fn is_mouse(self) -> bool {
        (257..=261).contains(&self.0)
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "{}", self.0),
        }
    }
}

/// What a bound input does. `0` is never configured but must round-trip as `"0"`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Command(pub i32);

impl Command {
    pub const NONE: Command = Command(0);
    pub const RESIZE: Command = Command(1);
    pub const MOVE: Command = Command(2);

    pub fn parse(s: &str) -> Option<Command> {
        let s = s.trim();
        match s {
            "Resize" => Some(Command::RESIZE),
            "Move" => Some(Command::MOVE),
            _ => parse_int(s).map(Command),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            1 => f.write_str("Resize"),
            2 => f.write_str("Move"),
            n => write!(f, "{n}"),
        }
    }
}

/// WPF `ModifierKeys` flags: Alt=1, Control=2, Shift=4, Windows=8.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Modifiers(pub i32);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const ALT: Modifiers = Modifiers(1);
    pub const CONTROL: Modifiers = Modifiers(2);
    pub const SHIFT: Modifiers = Modifiers(4);
    pub const WINDOWS: Modifiers = Modifiers(8);

    const NAMES: [(&'static str, i32); 4] =
        [("Alt", 1), ("Control", 2), ("Shift", 4), ("Windows", 8)];

    /// Parses `"None"`, `"Control"`, `"Control, Shift"` or an integer.
    pub fn parse(s: &str) -> Option<Modifiers> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Some(v) = parse_int(s) {
            return Some(Modifiers(v));
        }
        let mut bits = 0;
        for part in s.split(',') {
            let part = part.trim();
            if part == "None" {
                continue;
            }
            let (_, v) = Self::NAMES.iter().find(|(n, _)| *n == part)?;
            bits |= v;
        }
        Some(Modifiers(bits))
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("None");
        }
        let known: i32 = Self::NAMES.iter().map(|(_, v)| v).sum();
        if self.0 & !known != 0 {
            return write!(f, "{}", self.0);
        }
        let mut first = true;
        for (name, v) in Self::NAMES {
            if self.0 & v != 0 {
                if !first {
                    f.write_str(", ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip() {
        for (name, value) in KEY_TABLE {
            let k = KeyCode::parse(name).unwrap();
            assert_eq!(k.0, *value);
            assert_eq!(k.to_string(), *name);
        }
    }

    #[test]
    fn key_table_matches_known_codes() {
        assert_eq!(KeyCode::parse("KEY_SPACE"), Some(KeyCode::SPACE));
        assert_eq!(KeyCode::parse("KEY_LCONTROL"), Some(KeyCode::LCONTROL));
        assert_eq!(KeyCode::parse("MOUSE_RBUTTON"), Some(KeyCode::MOUSE_RBUTTON));
        assert_eq!(KeyCode::parse("MOUSE_X2BUTTON"), Some(KeyCode::MOUSE_X2BUTTON));
        assert!(KeyCode::MOUSE_MBUTTON.is_mouse() && !KeyCode::SPACE.is_mouse());
    }

    #[test]
    fn key_parse_edge_cases() {
        assert_eq!(KeyCode::parse(" 57 "), Some(KeyCode::SPACE));
        assert_eq!(KeyCode::parse("1234"), Some(KeyCode(1234)));
        assert_eq!(KeyCode(1234).to_string(), "1234");
        assert_eq!(KeyCode::parse("key_space"), None);
        assert_eq!(KeyCode::parse(""), None);
        assert_eq!(KeyCode::parse("nope"), None);
    }

    #[test]
    fn command_parse_and_display() {
        assert_eq!(Command::parse("Resize"), Some(Command::RESIZE));
        assert_eq!(Command::parse("Move"), Some(Command::MOVE));
        assert_eq!(Command::parse("2"), Some(Command::MOVE));
        assert_eq!(Command::parse("resize"), None);
        assert_eq!(Command::NONE.to_string(), "0");
        assert_eq!(Command::MOVE.to_string(), "Move");
    }

    #[test]
    fn modifiers_parse_and_display() {
        assert_eq!(Modifiers::parse("Control, Shift"), Some(Modifiers(6)));
        assert_eq!(Modifiers::parse("None"), Some(Modifiers::NONE));
        assert_eq!(Modifiers::parse("Alt,Windows"), Some(Modifiers(9)));
        assert_eq!(Modifiers::parse("Meta"), None);
        assert_eq!(Modifiers(6).to_string(), "Control, Shift");
        assert_eq!(Modifiers(0).to_string(), "None");
        assert_eq!(Modifiers(16).to_string(), "16");
    }
}
