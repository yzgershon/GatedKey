//! Virtual-key codes, chord representation, and display names.

use serde::{Deserialize, Serialize};

pub const VK_CONTROL: u32 = 0x11;
pub const VK_F23: u32 = 0x86;

/// One non-modifier key plus whichever modifiers were held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chord {
    pub vk: u32,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub win: bool,
}

impl Chord {
    /// What a Copilot key emits, per Microsoft's keyboard spec:
    /// Left Shift + Left Win + F23. F23 is used precisely because no physical
    /// keyboard has one, so nothing else can collide with it.
    pub fn copilot_key() -> Self {
        Chord {
            vk: VK_F23,
            ctrl: false,
            shift: true,
            alt: false,
            win: true,
        }
    }

    pub fn label(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.win {
            parts.push("Win");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        let key = vk_name(self.vk);
        if parts.is_empty() {
            key
        } else {
            format!("{} + {}", parts.join(" + "), key)
        }
    }
}

/// Modifier keys never form a chord on their own, so the hook passes them
/// straight through after recording that they are held.
pub fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

pub fn vk_name(vk: u32) -> String {
    let named = match vk {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "Print Screen",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5D => "Menu",
        0x90 => "Num Lock",
        0x91 => "Scroll Lock",
        0xA6 => "Browser Back",
        0xA7 => "Browser Forward",
        0xAD => "Mute",
        0xAE => "Volume Down",
        0xAF => "Volume Up",
        0xB0 => "Next Track",
        0xB1 => "Previous Track",
        0xB2 => "Stop Media",
        0xB3 => "Play/Pause",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    // 0-9 and A-Z map straight to their ASCII character.
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        if let Some(c) = char::from_u32(vk) {
            return c.to_string();
        }
    }
    // F1 through F24.
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x6F);
    }
    if (0x60..=0x69).contains(&vk) {
        return format!("Numpad {}", vk - 0x60);
    }
    format!("Key {vk}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_chord_matches_the_microsoft_spec() {
        let c = Chord::copilot_key();
        assert_eq!(c.vk, 0x86);
        assert!(c.shift && c.win);
        assert!(!c.ctrl && !c.alt);
        assert_eq!(c.label(), "Win + Shift + F23");
    }

    #[test]
    fn f_keys_are_named_correctly() {
        assert_eq!(vk_name(0x70), "F1");
        assert_eq!(vk_name(0x86), "F23");
        assert_eq!(vk_name(0x87), "F24");
    }

    #[test]
    fn every_modifier_variant_is_recognised() {
        for vk in [
            0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
        ] {
            assert!(is_modifier(vk), "vk {vk:#X} should be a modifier");
        }
        assert!(!is_modifier(0x86));
        assert!(!is_modifier(0x41));
    }
}
