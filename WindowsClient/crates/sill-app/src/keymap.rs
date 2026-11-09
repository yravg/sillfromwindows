//! OWNER: worker U2. Physical keys to USB HID usages, modifiers to the Mac's (D2).
//!
//! Sill's wire carries a key as a USB HID usage (page 0x07) and UIKeyModifierFlags bits
//! (Sources/StreamProtocol/Input.swift `.key`; the host maps only the usages in
//! Sources/SillHost/KeyStrokes.swift:184-240, anything else is dropped and counted). The usage
//! comes from the PHYSICAL key (egui's `physical_key`), as the iOS client sends the usage of the
//! hardware key (iOSClient/KeyChords.swift `ForwardedKeys`): the Mac's own layout makes the
//! character.
//!
//! D2 (docs/windows-client-plan.md §6): Ctrl is the Mac's ⌘ and the Windows key its ⌃ by default;
//! `Preferences::windows_key_is_command` swaps them.
use egui::Key;
use sill_proto::input::InputEvent;

pub const MOD_SHIFT: u64 = 0x2_0000;
pub const MOD_CONTROL: u64 = 0x4_0000;
pub const MOD_OPTION: u64 = 0x8_0000;
pub const MOD_COMMAND: u64 = 0x10_0000;

pub const USAGE_LCTRL: u16 = 0xE0;
pub const USAGE_LSHIFT: u16 = 0xE1;
pub const USAGE_LOPTION: u16 = 0xE2;
pub const USAGE_LCOMMAND: u16 = 0xE3;

/// The USB HID usage of a physical key, or None for a key the Mac host does not take
/// (KeyStrokes.swift:184-240 maps no numpad key, no ISO 0x32/0x64 and nothing above F12).
pub fn hid_usage(key: Key) -> Option<u16> {
    use Key::*;
    Some(match key {
        A => 0x04,
        B => 0x05,
        C => 0x06,
        D => 0x07,
        E => 0x08,
        F => 0x09,
        G => 0x0A,
        H => 0x0B,
        I => 0x0C,
        J => 0x0D,
        K => 0x0E,
        L => 0x0F,
        M => 0x10,
        N => 0x11,
        O => 0x12,
        P => 0x13,
        Q => 0x14,
        R => 0x15,
        S => 0x16,
        T => 0x17,
        U => 0x18,
        V => 0x19,
        W => 0x1A,
        X => 0x1B,
        Y => 0x1C,
        Z => 0x1D,
        Num1 => 0x1E,
        Num2 => 0x1F,
        Num3 => 0x20,
        Num4 => 0x21,
        Num5 => 0x22,
        Num6 => 0x23,
        Num7 => 0x24,
        Num8 => 0x25,
        Num9 => 0x26,
        Num0 => 0x27,
        Enter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus => 0x2D,
        Equals => 0x2E,
        OpenBracket => 0x2F,
        CloseBracket => 0x30,
        Backslash => 0x31,
        Semicolon => 0x33,
        Quote => 0x34,
        Backtick => 0x35,
        Comma => 0x36,
        Period => 0x37,
        Slash => 0x38,
        F1 => 0x3A,
        F2 => 0x3B,
        F3 => 0x3C,
        F4 => 0x3D,
        F5 => 0x3E,
        F6 => 0x3F,
        F7 => 0x40,
        F8 => 0x41,
        F9 => 0x42,
        F10 => 0x43,
        F11 => 0x44,
        F12 => 0x45,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        _ => return None,
    })
}

/// A key whose character the Mac types (ForwardedKeys.characterUsages in iOSClient/KeyChords.swift:
/// letters, digits, Return, Delete, Tab, Space and the punctuation keys). Whether such a key goes
/// as a key or as text depends on `WindowList.keyRepeat` (KeyTyping, TextComposition.swift).
pub fn is_character_usage(u: u16) -> bool {
    matches!(u, 0x04..=0x27 | 0x2C..=0x31 | 0x33..=0x38)
}

/// The modifier keys this PC holds. `win` is the Windows key; egui does not report it (it has no
/// such modifier), so the app feeds `false` until a platform hook supplies it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcMods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

impl PcMods {
    pub fn from_egui(m: egui::Modifiers) -> Self {
        PcMods {
            ctrl: m.ctrl,
            shift: m.shift,
            alt: m.alt,
            win: false,
        }
    }
}

/// The modifiers the Mac should hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MacMods {
    pub control: bool,
    pub shift: bool,
    pub option: bool,
    pub command: bool,
}

impl MacMods {
    pub fn flags(self) -> u64 {
        let mut f = 0;
        if self.control {
            f |= MOD_CONTROL;
        }
        if self.shift {
            f |= MOD_SHIFT;
        }
        if self.option {
            f |= MOD_OPTION;
        }
        if self.command {
            f |= MOD_COMMAND;
        }
        f
    }
}

/// D2. Default: Ctrl is ⌘, Windows key is ⌃. Swapped (`windows_key_is_command`): Ctrl is ⌃,
/// Windows key is ⌘. Shift is ⇧ and Alt is ⌥ either way.
pub fn mac_mods(pc: PcMods, windows_key_is_command: bool) -> MacMods {
    let (command, control) = if windows_key_is_command {
        (pc.win, pc.ctrl)
    } else {
        (pc.ctrl, pc.win)
    };
    MacMods {
        control,
        shift: pc.shift,
        option: pc.alt,
        command,
    }
}

type Step = (bool, bool, u16, fn(&mut MacMods, bool));

/// Sends a modifier's own key when its state changes, as a hardware keyboard's flags-changed does
/// (KeyStrokes.swift: "a modifier's own key is a flags-changed event with what is held"), so a
/// click or a key made while one is held carries it.
#[derive(Debug, Default)]
pub struct ModTracker {
    sent: MacMods,
}

impl ModTracker {
    pub fn current(&self) -> MacMods {
        self.sent
    }

    /// Events that bring the Mac's held modifiers to `want`; each carries the flags after it.
    pub fn sync(&mut self, want: MacMods) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let mut now = self.sent;
        let steps: [Step; 4] = [
            (self.sent.control, want.control, USAGE_LCTRL, |m, v| {
                m.control = v
            }),
            (self.sent.shift, want.shift, USAGE_LSHIFT, |m, v| {
                m.shift = v
            }),
            (self.sent.option, want.option, USAGE_LOPTION, |m, v| {
                m.option = v
            }),
            (self.sent.command, want.command, USAGE_LCOMMAND, |m, v| {
                m.command = v
            }),
        ];
        for (was, will, usage, set) in steps {
            if was != will {
                set(&mut now, will);
                out.push(InputEvent::Key {
                    hid_usage: usage,
                    down: will,
                    modifiers: now.flags(),
                });
            }
        }
        self.sent = now;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_the_hosts_usages() {
        assert_eq!(hid_usage(Key::A), Some(0x04));
        assert_eq!(hid_usage(Key::Z), Some(0x1D));
        assert_eq!(hid_usage(Key::Num1), Some(0x1E));
        assert_eq!(hid_usage(Key::Num0), Some(0x27));
        assert_eq!(hid_usage(Key::Enter), Some(0x28));
        assert_eq!(hid_usage(Key::Escape), Some(0x29));
        assert_eq!(hid_usage(Key::Backspace), Some(0x2A));
        assert_eq!(hid_usage(Key::Tab), Some(0x2B));
        assert_eq!(hid_usage(Key::Space), Some(0x2C));
        assert_eq!(hid_usage(Key::Minus), Some(0x2D));
        assert_eq!(hid_usage(Key::Slash), Some(0x38));
        assert_eq!(hid_usage(Key::F1), Some(0x3A));
        assert_eq!(hid_usage(Key::F12), Some(0x45));
        assert_eq!(hid_usage(Key::Home), Some(0x4A));
        assert_eq!(hid_usage(Key::Delete), Some(0x4C));
        assert_eq!(hid_usage(Key::PageDown), Some(0x4E));
        assert_eq!(hid_usage(Key::ArrowRight), Some(0x4F));
        assert_eq!(hid_usage(Key::ArrowUp), Some(0x52));
    }

    #[test]
    fn unmapped_keys_are_none_and_usages_are_unique() {
        assert_eq!(hid_usage(Key::F13), None);
        assert_eq!(hid_usage(Key::IntlBackslash), None);
        assert_eq!(hid_usage(Key::Insert), None);
        let all = [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
            Key::Enter,
            Key::Escape,
            Key::Backspace,
            Key::Tab,
            Key::Space,
            Key::Minus,
            Key::Equals,
            Key::OpenBracket,
            Key::CloseBracket,
            Key::Backslash,
            Key::Semicolon,
            Key::Quote,
            Key::Backtick,
            Key::Comma,
            Key::Period,
            Key::Slash,
            Key::F1,
            Key::F2,
            Key::F3,
            Key::F4,
            Key::F5,
            Key::F6,
            Key::F7,
            Key::F8,
            Key::F9,
            Key::F10,
            Key::F11,
            Key::F12,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Delete,
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
        ];
        let mut seen = std::collections::HashSet::new();
        for k in all {
            let u = hid_usage(k).unwrap_or_else(|| panic!("{k:?} unmapped"));
            assert!(seen.insert(u), "{k:?} repeats usage {u:#x}");
        }
        assert_eq!(seen.len(), 73);
    }

    #[test]
    fn character_keys_exclude_navigation_and_function_keys() {
        assert!(is_character_usage(0x04));
        assert!(is_character_usage(0x2C));
        assert!(is_character_usage(0x38));
        assert!(!is_character_usage(0x29)); // Esc
        assert!(!is_character_usage(0x3A)); // F1
        assert!(!is_character_usage(0x52)); // Up
                                            // Return, Backspace and Tab are not character usages: they always go as keys.
        assert!(
            !is_character_usage(0x28) && !is_character_usage(0x2A) && !is_character_usage(0x2B)
        );
    }

    #[test]
    fn d2_default_ctrl_is_command_and_windows_is_control() {
        let ctrl = PcMods {
            ctrl: true,
            ..Default::default()
        };
        let m = mac_mods(ctrl, false);
        assert!(m.command && !m.control);
        assert_eq!(m.flags(), MOD_COMMAND);
        let win = PcMods {
            win: true,
            ..Default::default()
        };
        let m = mac_mods(win, false);
        assert!(m.control && !m.command);
        assert_eq!(m.flags(), MOD_CONTROL);
    }

    #[test]
    fn d2_swapped_windows_is_command_and_ctrl_is_control() {
        let ctrl = PcMods {
            ctrl: true,
            ..Default::default()
        };
        let m = mac_mods(ctrl, true);
        assert!(m.control && !m.command);
        let win = PcMods {
            win: true,
            ..Default::default()
        };
        let m = mac_mods(win, true);
        assert!(m.command && !m.control);
    }

    #[test]
    fn shift_and_alt_are_shift_and_option_in_both_settings() {
        let pc = PcMods {
            shift: true,
            alt: true,
            ..Default::default()
        };
        for swap in [false, true] {
            let m = mac_mods(pc, swap);
            assert_eq!(m.flags(), MOD_SHIFT | MOD_OPTION);
        }
    }

    #[test]
    fn tracker_sends_only_changes_with_flags_after() {
        let mut t = ModTracker::default();
        let cmd = MacMods {
            command: true,
            ..Default::default()
        };
        let ev = t.sync(cmd);
        assert_eq!(
            ev,
            vec![InputEvent::Key {
                hid_usage: USAGE_LCOMMAND,
                down: true,
                modifiers: MOD_COMMAND
            }]
        );
        assert!(t.sync(cmd).is_empty());
        let both = MacMods {
            command: true,
            shift: true,
            ..Default::default()
        };
        let ev = t.sync(both);
        assert_eq!(
            ev,
            vec![InputEvent::Key {
                hid_usage: USAGE_LSHIFT,
                down: true,
                modifiers: MOD_COMMAND | MOD_SHIFT
            }]
        );
        let ev = t.sync(MacMods::default());
        assert_eq!(ev.len(), 2);
        assert_eq!(
            ev[1],
            InputEvent::Key {
                hid_usage: USAGE_LCOMMAND,
                down: false,
                modifiers: 0
            }
        );
    }
}
