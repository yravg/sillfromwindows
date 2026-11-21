//! OWNER: worker U2. egui pointer, wheel and key events to Sill input events (kind 8).
//!
//! The wire (Sources/StreamProtocol/Input.swift): positions are fractions of the streamed frame;
//! pointer buttons are left and right only (no middle, no move-with-button event: a drag is
//! leftDown, moves, leftUp); a trackpad scroll is `scrollGesture began`, `scroll` deltas,
//! `scrollGesture ended`, a wheel is bare `scroll` deltas.
//!
//! Keys: a key goes as a HID usage from its physical key (keymap.rs). Whether a character key goes
//! as a key or as text follows iOSClient's `KeyTyping` (TextComposition.swift): while the Mac says
//! it repeats a held key (`WindowList.keyRepeat`, `AppModel::stream.mac_key_repeat`) the characters
//! go as keys, so the Mac's layout, dead keys and repeat apply; otherwise they go as `text`, and a
//! shortcut (Ctrl or Alt held) always goes as keys. Return, Tab, Backspace, the arrows, F-keys and
//! the rest always go as keys. Everything held is released by [`InputState::release_all`].
use std::collections::BTreeSet;

use egui::{Event, MouseWheelUnit, PointerButton, Pos2, Rect, TouchPhase};
use sill_proto::input::{InputEvent, PointerAction, ScrollPhase};

use crate::keymap::{self, MacMods, ModTracker, PcMods};

/// One wheel notch, in points of the picture (a Windows notch is 3 lines of about 16 points).
pub const LINE_POINTS: f32 = 48.0;
/// A trackpad gesture with no further deltas for this long has ended (Windows precision touchpads
/// give no phase through winit).
pub const GESTURE_IDLE: f64 = 0.15;

/// Largest rectangle of the frame's aspect ratio centred in `panel`: the picture, letterbox out.
pub fn video_rect(panel: Rect, frame_w: u32, frame_h: u32) -> Rect {
    if frame_w == 0 || frame_h == 0 || panel.width() <= 0.0 || panel.height() <= 0.0 {
        return Rect::from_center_size(panel.center(), egui::Vec2::ZERO);
    }
    let aspect = frame_w as f32 / frame_h as f32;
    let (w, h) = if panel.width() / panel.height() > aspect {
        (panel.height() * aspect, panel.height())
    } else {
        (panel.width(), panel.width() / aspect)
    };
    Rect::from_center_size(panel.center(), egui::vec2(w, h))
}

/// A screen position as fractions of the picture. Outside it: None, or clamped to 0…1 (a drag
/// that left the picture still lands on its edge, as iOS holds a drag to the source).
pub fn fraction(pos: Pos2, video: Rect, clamp: bool) -> Option<(f64, f64)> {
    if video.width() <= 0.0 || video.height() <= 0.0 {
        return None;
    }
    if !clamp && !video.contains(pos) {
        return None;
    }
    let x = ((pos.x - video.left()) / video.width()) as f64;
    let y = ((pos.y - video.top()) / video.height()) as f64;
    Some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
}

#[derive(Clone, Copy, Debug)]
pub struct Cfg {
    /// The Mac repeats held keys: send characters as keys.
    pub key_repeat: bool,
    /// D2 swap.
    pub windows_key_is_command: bool,
}

/// What one egui frame offers.
pub struct Frame<'a> {
    pub events: &'a [Event],
    pub mods: egui::Modifiers,
    pub now: f64,
    /// The picture's rectangle on screen.
    pub video: Rect,
    /// The pointer is over the picture and nothing covers it.
    pub pointer_over: bool,
    /// The picture has the keyboard (no text field has it).
    pub keys_active: bool,
    pub cfg: Cfg,
}

#[derive(Debug, Default)]
pub struct InputState {
    held_keys: BTreeSet<u16>,
    left: bool,
    right: bool,
    last_pos: Option<(f64, f64)>,
    mods: ModTracker,
    gesture_open: bool,
    last_scroll: f64,
}

impl InputState {
    #[cfg(test)]
    pub fn anything_held(&self) -> bool {
        !self.held_keys.is_empty()
            || self.left
            || self.right
            || self.gesture_open
            || self.mods.current() != MacMods::default()
    }

    /// Forget everything without sending (a new session starts clean).
    pub fn reset(&mut self) {
        *self = InputState::default();
    }

    fn move_to(&mut self, out: &mut Vec<InputEvent>, p: (f64, f64)) {
        if self.last_pos != Some(p) {
            self.last_pos = Some(p);
            out.push(InputEvent::Pointer {
                action: PointerAction::Move,
                x: p.0,
                y: p.1,
            });
        }
    }

    /// Let go of every key, button and modifier this PC holds on the Mac, and end a scroll.
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for u in std::mem::take(&mut self.held_keys) {
            out.push(InputEvent::Key {
                hid_usage: u,
                down: false,
                modifiers: 0,
            });
        }
        let (x, y) = self.last_pos.unwrap_or((0.5, 0.5));
        if std::mem::take(&mut self.left) {
            out.push(InputEvent::Pointer {
                action: PointerAction::LeftUp,
                x,
                y,
            });
        }
        if std::mem::take(&mut self.right) {
            out.push(InputEvent::Pointer {
                action: PointerAction::RightUp,
                x,
                y,
            });
        }
        out.extend(self.mods.sync(MacMods::default()));
        if std::mem::take(&mut self.gesture_open) {
            out.push(InputEvent::ScrollGesture {
                phase: ScrollPhase::Ended,
                x,
                y,
            });
        }
        out
    }

    /// End a trackpad gesture that has gone quiet. Call every frame while one is open.
    pub fn tick(&mut self, now: f64) -> Vec<InputEvent> {
        if self.gesture_open && now - self.last_scroll > GESTURE_IDLE {
            self.gesture_open = false;
            let (x, y) = self.last_pos.unwrap_or((0.5, 0.5));
            return vec![InputEvent::ScrollGesture {
                phase: ScrollPhase::Ended,
                x,
                y,
            }];
        }
        Vec::new()
    }

    pub fn gesture_open(&self) -> bool {
        self.gesture_open
    }

    pub fn process(&mut self, f: &Frame) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let swap = f.cfg.windows_key_is_command;
        // Characters a key press just sent as a key must not arrive again as text this frame.
        let mut text_skip = false;

        if !f.keys_active
            && (!self.held_keys.is_empty() || self.mods.current() != MacMods::default())
        {
            // The picture lost the keyboard: whatever was held goes up now.
            let mut r = self.release_keys_and_mods();
            out.append(&mut r);
        }

        for ev in f.events {
            match ev {
                Event::WindowFocused(false) => out.extend(self.release_all()),
                Event::PointerMoved(pos) => {
                    let held = self.left || self.right;
                    if let Some(p) = fraction(*pos, f.video, held) {
                        if held || f.pointer_over {
                            self.move_to(&mut out, p);
                        }
                    }
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let action = match (button, pressed) {
                        (PointerButton::Primary, true) => PointerAction::LeftDown,
                        (PointerButton::Primary, false) => PointerAction::LeftUp,
                        (PointerButton::Secondary, true) => PointerAction::RightDown,
                        (PointerButton::Secondary, false) => PointerAction::RightUp,
                        _ => continue,
                    };
                    let flag = match button {
                        PointerButton::Primary => &mut self.left,
                        _ => &mut self.right,
                    };
                    if *pressed {
                        if !f.pointer_over {
                            continue;
                        }
                        let Some(p) = fraction(*pos, f.video, false) else {
                            continue;
                        };
                        *flag = true;
                        self.move_to(&mut out, p);
                        let (x, y) = p;
                        out.push(InputEvent::Pointer { action, x, y });
                    } else if *flag {
                        *flag = false;
                        let p = fraction(*pos, f.video, true)
                            .or(self.last_pos)
                            .unwrap_or((0.5, 0.5));
                        self.move_to(&mut out, p);
                        out.push(InputEvent::Pointer {
                            action,
                            x: p.0,
                            y: p.1,
                        });
                    }
                }
                Event::MouseWheel {
                    unit, delta, phase, ..
                } if f.pointer_over => {
                    self.wheel(&mut out, f, *unit, *delta, *phase);
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } if f.keys_active => {
                    let k = physical_key.unwrap_or(*key);
                    let Some(u) = keymap::hid_usage(k) else {
                        continue;
                    };
                    out.extend(
                        self.mods
                            .sync(keymap::mac_mods(PcMods::from_egui(*modifiers), swap)),
                    );
                    let flags = self.mods.current().flags();
                    if *pressed {
                        let altgr = modifiers.ctrl && modifiers.alt;
                        let shortcut = modifiers.ctrl || modifiers.alt;
                        let as_key = if keymap::is_character_usage(u) {
                            !altgr && (f.cfg.key_repeat || shortcut)
                        } else {
                            true
                        };
                        if !as_key {
                            continue;
                        }
                        if keymap::is_character_usage(u) {
                            text_skip = true;
                        }
                        if *repeat && f.cfg.key_repeat {
                            // The host repeats a held key at the Mac's own rate (KeyRepeat).
                            continue;
                        }
                        self.held_keys.insert(u);
                        out.push(InputEvent::Key {
                            hid_usage: u,
                            down: true,
                            modifiers: flags,
                        });
                    } else if self.held_keys.remove(&u) {
                        out.push(InputEvent::Key {
                            hid_usage: u,
                            down: false,
                            modifiers: flags,
                        });
                    }
                }
                // egui-winit turns Ctrl+C, X and V into these and drops the key event.
                Event::Copy | Event::Cut | Event::Paste(_) if f.keys_active => {
                    let usage = match ev {
                        Event::Copy => 0x06,
                        Event::Cut => 0x1B,
                        _ => 0x19,
                    };
                    out.extend(
                        self.mods
                            .sync(keymap::mac_mods(PcMods::from_egui(f.mods), swap)),
                    );
                    let flags = self.mods.current().flags();
                    out.push(InputEvent::Key {
                        hid_usage: usage,
                        down: true,
                        modifiers: flags,
                    });
                    out.push(InputEvent::Key {
                        hid_usage: usage,
                        down: false,
                        modifiers: flags,
                    });
                }
                Event::Text(s) if f.keys_active && !text_skip => push_text(&mut out, s),
                Event::Ime(egui::ImeEvent::Commit(s)) if f.keys_active => push_text(&mut out, s),
                _ => {}
            }
        }

        if f.keys_active {
            out.extend(
                self.mods
                    .sync(keymap::mac_mods(PcMods::from_egui(f.mods), swap)),
            );
        }
        out.extend(self.tick(f.now));
        out
    }

    fn release_keys_and_mods(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for u in std::mem::take(&mut self.held_keys) {
            out.push(InputEvent::Key {
                hid_usage: u,
                down: false,
                modifiers: 0,
            });
        }
        out.extend(self.mods.sync(MacMods::default()));
        out
    }

    fn wheel(
        &mut self,
        out: &mut Vec<InputEvent>,
        f: &Frame,
        unit: MouseWheelUnit,
        delta: egui::Vec2,
        phase: TouchPhase,
    ) {
        let (px, py) = match unit {
            MouseWheelUnit::Line => (delta.x * LINE_POINTS, delta.y * LINE_POINTS),
            MouseWheelUnit::Page => (delta.x * f.video.width(), delta.y * f.video.height()),
            MouseWheelUnit::Point => (delta.x, delta.y),
        };
        if f.video.width() <= 0.0 || f.video.height() <= 0.0 {
            return;
        }
        // egui: positive delta moves the content down, the wire's natural-scrolling sign.
        let dx = (px / f.video.width()) as f64;
        let dy = (py / f.video.height()) as f64;
        let (x, y) = self.last_pos.unwrap_or((0.5, 0.5));
        match unit {
            MouseWheelUnit::Line | MouseWheelUnit::Page => {
                out.push(InputEvent::Scroll { x, y, dx, dy });
            }
            MouseWheelUnit::Point => {
                if phase == TouchPhase::Start && self.gesture_open {
                    out.push(InputEvent::ScrollGesture {
                        phase: ScrollPhase::Ended,
                        x,
                        y,
                    });
                    self.gesture_open = false;
                }
                if !self.gesture_open {
                    self.gesture_open = true;
                    out.push(InputEvent::ScrollGesture {
                        phase: ScrollPhase::Began,
                        x,
                        y,
                    });
                }
                if dx != 0.0 || dy != 0.0 {
                    out.push(InputEvent::Scroll { x, y, dx, dy });
                }
                self.last_scroll = f.now;
                if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
                    self.gesture_open = false;
                    out.push(InputEvent::ScrollGesture {
                        phase: ScrollPhase::Ended,
                        x,
                        y,
                    });
                }
            }
        }
    }
}

fn push_text(out: &mut Vec<InputEvent>, s: &str) {
    let t: String = s.chars().filter(|c| !c.is_control()).collect();
    if !t.is_empty() {
        out.push(InputEvent::Text { text: t });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{MOD_COMMAND, MOD_CONTROL, MOD_SHIFT};
    use egui::Key;
    use egui::{pos2, vec2, Modifiers};

    fn video() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 200.0))
    }

    fn frame<'a>(events: &'a [Event], mods: Modifiers, key_repeat: bool) -> Frame<'a> {
        Frame {
            events,
            mods,
            now: 1.0,
            video: video(),
            pointer_over: true,
            keys_active: true,
            cfg: Cfg {
                key_repeat,
                windows_key_is_command: false,
            },
        }
    }

    fn key(k: Key, pressed: bool, mods: Modifiers) -> Event {
        Event::Key {
            key: k,
            physical_key: Some(k),
            pressed,
            repeat: false,
            modifiers: mods,
        }
    }

    fn ctrl() -> Modifiers {
        Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        }
    }

    #[test]
    fn video_rect_letterboxes_wide_and_tall_panels() {
        let panel = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0));
        let r = video_rect(panel, 1000, 1000);
        assert_eq!(r, Rect::from_min_size(pos2(250.0, 0.0), vec2(500.0, 500.0)));
        let r = video_rect(panel, 2000, 500);
        assert_eq!(
            r,
            Rect::from_min_size(pos2(0.0, 125.0), vec2(1000.0, 250.0))
        );
        assert_eq!(video_rect(panel, 0, 0).width(), 0.0);
    }

    #[test]
    fn fractions_exclude_the_letterbox_and_clamp_drags() {
        let v = video();
        assert_eq!(fraction(pos2(100.0, 50.0), v, false), Some((0.0, 0.0)));
        assert_eq!(fraction(pos2(300.0, 150.0), v, false), Some((0.5, 0.5)));
        assert_eq!(fraction(pos2(99.0, 150.0), v, false), None);
        assert_eq!(fraction(pos2(50.0, 400.0), v, true), Some((0.0, 1.0)));
        assert_eq!(fraction(pos2(9999.0, -5.0), v, true), Some((1.0, 0.0)));
    }

    #[test]
    fn click_sends_move_then_down_then_up() {
        let mut s = InputState::default();
        let ev = [
            Event::PointerMoved(pos2(300.0, 150.0)),
            Event::PointerButton {
                pos: pos2(300.0, 150.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos: pos2(300.0, 150.0),
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ];
        let out = s.process(&frame(&ev, Modifiers::NONE, true));
        assert_eq!(
            out,
            vec![
                InputEvent::Pointer {
                    action: PointerAction::Move,
                    x: 0.5,
                    y: 0.5
                },
                InputEvent::Pointer {
                    action: PointerAction::LeftDown,
                    x: 0.5,
                    y: 0.5
                },
                InputEvent::Pointer {
                    action: PointerAction::LeftUp,
                    x: 0.5,
                    y: 0.5
                },
            ]
        );
        assert!(!s.anything_held());
    }

    #[test]
    fn press_in_the_letterbox_is_ignored_and_its_release_sends_nothing() {
        let mut s = InputState::default();
        let ev = [
            Event::PointerButton {
                pos: pos2(10.0, 10.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos: pos2(10.0, 10.0),
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ];
        assert!(s.process(&frame(&ev, Modifiers::NONE, true)).is_empty());
    }

    #[test]
    fn drag_out_of_the_picture_clamps_and_release_lands_on_the_edge() {
        let mut s = InputState::default();
        let down = [Event::PointerButton {
            pos: pos2(300.0, 150.0),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }];
        s.process(&frame(&down, Modifiers::NONE, true));
        let mut f = frame(&[], Modifiers::NONE, true);
        let ev = [
            Event::PointerMoved(pos2(900.0, 150.0)),
            Event::PointerButton {
                pos: pos2(900.0, 150.0),
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ];
        f.events = &ev;
        f.pointer_over = false;
        let out = s.process(&f);
        assert_eq!(
            out.last(),
            Some(&InputEvent::Pointer {
                action: PointerAction::LeftUp,
                x: 1.0,
                y: 0.5
            })
        );
        assert!(!s.anything_held());
    }

    #[test]
    fn middle_button_sends_nothing() {
        let mut s = InputState::default();
        let ev = [Event::PointerButton {
            pos: pos2(300.0, 150.0),
            button: PointerButton::Middle,
            pressed: true,
            modifiers: Modifiers::NONE,
        }];
        assert!(s.process(&frame(&ev, Modifiers::NONE, true)).is_empty());
    }

    #[test]
    fn letters_go_as_keys_when_the_mac_repeats_and_text_is_dropped() {
        let mut s = InputState::default();
        let ev = [
            key(Key::A, true, Modifiers::NONE),
            Event::Text("a".into()),
            key(Key::A, false, Modifiers::NONE),
        ];
        let out = s.process(&frame(&ev, Modifiers::NONE, true));
        assert_eq!(
            out,
            vec![
                InputEvent::Key {
                    hid_usage: 0x04,
                    down: true,
                    modifiers: 0
                },
                InputEvent::Key {
                    hid_usage: 0x04,
                    down: false,
                    modifiers: 0
                },
            ]
        );
    }

    #[test]
    fn letters_go_as_text_when_the_mac_does_not_repeat() {
        let mut s = InputState::default();
        let ev = [
            key(Key::A, true, Modifiers::NONE),
            Event::Text("a".into()),
            key(Key::A, false, Modifiers::NONE),
        ];
        let out = s.process(&frame(&ev, Modifiers::NONE, false));
        assert_eq!(out, vec![InputEvent::Text { text: "a".into() }]);
    }

    #[test]
    fn arrows_and_return_are_keys_in_both_modes_and_held_repeats_follow_the_mode() {
        for kr in [true, false] {
            let mut s = InputState::default();
            let out = s.process(&frame(
                &[key(Key::ArrowLeft, true, Modifiers::NONE)],
                Modifiers::NONE,
                kr,
            ));
            assert_eq!(
                out,
                vec![InputEvent::Key {
                    hid_usage: 0x50,
                    down: true,
                    modifiers: 0
                }]
            );
            let rep = Event::Key {
                key: Key::ArrowLeft,
                physical_key: Some(Key::ArrowLeft),
                pressed: true,
                repeat: true,
                modifiers: Modifiers::NONE,
            };
            let out = s.process(&frame(&[rep], Modifiers::NONE, kr));
            // The host repeats for itself; a host that does not needs the device's repeats.
            assert_eq!(out.len(), if kr { 0 } else { 1 });
            let out = s.process(&frame(
                &[key(Key::Enter, true, Modifiers::NONE)],
                Modifiers::NONE,
                kr,
            ));
            assert_eq!(
                out,
                vec![InputEvent::Key {
                    hid_usage: 0x28,
                    down: true,
                    modifiers: 0
                }]
            );
        }
    }

    #[test]
    fn ctrl_c_goes_as_command_c_even_though_egui_swallows_the_key() {
        let mut s = InputState::default();
        let ev = [Event::Copy, key(Key::C, false, ctrl())];
        let out = s.process(&frame(&ev, ctrl(), false));
        assert_eq!(
            out,
            vec![
                InputEvent::Key {
                    hid_usage: 0xE3,
                    down: true,
                    modifiers: MOD_COMMAND
                },
                InputEvent::Key {
                    hid_usage: 0x06,
                    down: true,
                    modifiers: MOD_COMMAND
                },
                InputEvent::Key {
                    hid_usage: 0x06,
                    down: false,
                    modifiers: MOD_COMMAND
                },
            ]
        );
        let out = s.process(&frame(&[], Modifiers::NONE, false));
        assert_eq!(
            out,
            vec![InputEvent::Key {
                hid_usage: 0xE3,
                down: false,
                modifiers: 0
            }]
        );
    }

    #[test]
    fn ctrl_s_goes_as_command_s_in_text_mode_and_swapped_it_is_control() {
        let mut s = InputState::default();
        let ev = [key(Key::S, true, ctrl()), Event::Text("s".into())];
        let out = s.process(&frame(&ev, ctrl(), false));
        assert!(out.contains(&InputEvent::Key {
            hid_usage: 0x16,
            down: true,
            modifiers: MOD_COMMAND
        }));
        assert!(!out.iter().any(|e| matches!(e, InputEvent::Text { .. })));
        let mut s = InputState::default();
        let mut f = frame(&ev, ctrl(), false);
        f.cfg.windows_key_is_command = true;
        let out = s.process(&f);
        assert!(out.contains(&InputEvent::Key {
            hid_usage: 0x16,
            down: true,
            modifiers: MOD_CONTROL
        }));
    }

    #[test]
    fn shift_letter_in_text_mode_is_text_with_shift_held_on_the_mac() {
        let mut s = InputState::default();
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        let ev = [key(Key::A, true, shift), Event::Text("A".into())];
        let out = s.process(&frame(&ev, shift, false));
        assert_eq!(
            out,
            vec![
                InputEvent::Key {
                    hid_usage: 0xE1,
                    down: true,
                    modifiers: MOD_SHIFT
                },
                InputEvent::Text { text: "A".into() },
            ]
        );
    }

    #[test]
    fn altgr_characters_are_text_not_command_option_keys() {
        let mut s = InputState::default();
        let altgr = Modifiers {
            ctrl: true,
            alt: true,
            command: true,
            ..Default::default()
        };
        let ev = [key(Key::Q, true, altgr), Event::Text("@".into())];
        let out = s.process(&frame(&ev, altgr, true));
        assert!(out.contains(&InputEvent::Text { text: "@".into() }));
        assert!(!out.iter().any(|e| matches!(
            e,
            InputEvent::Key {
                hid_usage: 0x14,
                ..
            }
        )));
    }

    #[test]
    fn ime_commit_is_text_in_both_modes() {
        for kr in [true, false] {
            let mut s = InputState::default();
            let ev = [Event::Ime(egui::ImeEvent::Commit("你好".into()))];
            let out = s.process(&frame(&ev, Modifiers::NONE, kr));
            assert_eq!(
                out,
                vec![InputEvent::Text {
                    text: "你好".into()
                }]
            );
        }
    }

    #[test]
    fn control_characters_in_text_are_dropped() {
        let mut s = InputState::default();
        let out = s.process(&frame(
            &[Event::Text("\u{8}\r".into())],
            Modifiers::NONE,
            false,
        ));
        assert!(out.is_empty());
    }

    #[test]
    fn release_all_lets_go_of_keys_buttons_modifiers_and_scroll() {
        let mut s = InputState::default();
        let ev = [
            Event::PointerButton {
                pos: pos2(300.0, 150.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            key(Key::ArrowDown, true, ctrl()),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, 5.0),
                phase: TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Modifiers::NONE,
            },
        ];
        s.process(&frame(&ev, ctrl(), true));
        assert!(s.anything_held());
        let out = s.release_all();
        assert!(out.contains(&InputEvent::Key {
            hid_usage: 0x51,
            down: false,
            modifiers: 0
        }));
        assert!(out.contains(&InputEvent::Pointer {
            action: PointerAction::LeftUp,
            x: 0.5,
            y: 0.5
        }));
        assert!(out.contains(&InputEvent::Key {
            hid_usage: 0xE3,
            down: false,
            modifiers: 0
        }));
        assert!(out.contains(&InputEvent::ScrollGesture {
            phase: ScrollPhase::Ended,
            x: 0.5,
            y: 0.5
        }));
        assert!(!s.anything_held());
        assert!(s.release_all().is_empty());
    }

    #[test]
    fn losing_the_window_focus_or_the_keyboard_releases_keys() {
        let mut s = InputState::default();
        s.process(&frame(
            &[key(Key::A, true, Modifiers::NONE)],
            Modifiers::NONE,
            true,
        ));
        let out = s.process(&frame(
            &[Event::WindowFocused(false)],
            Modifiers::NONE,
            true,
        ));
        assert_eq!(
            out,
            vec![InputEvent::Key {
                hid_usage: 0x04,
                down: false,
                modifiers: 0
            }]
        );
        s.process(&frame(
            &[key(Key::B, true, Modifiers::NONE)],
            Modifiers::NONE,
            true,
        ));
        let mut f = frame(&[], Modifiers::NONE, true);
        f.keys_active = false;
        let out = s.process(&f);
        assert_eq!(
            out,
            vec![InputEvent::Key {
                hid_usage: 0x05,
                down: false,
                modifiers: 0
            }]
        );
    }

    #[test]
    fn keys_are_not_sent_while_the_picture_lacks_the_keyboard() {
        let mut s = InputState::default();
        let ev = [key(Key::A, true, Modifiers::NONE), Event::Text("a".into())];
        let mut f = frame(&ev, Modifiers::NONE, true);
        f.keys_active = false;
        assert!(s.process(&f).is_empty());
    }

    #[test]
    fn wheel_notch_is_a_bare_scroll_with_the_natural_sign() {
        let mut s = InputState::default();
        let ev = [
            Event::PointerMoved(pos2(300.0, 150.0)),
            Event::MouseWheel {
                unit: MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Modifiers::NONE,
            },
        ];
        let out = s.process(&frame(&ev, Modifiers::NONE, true));
        let want = (LINE_POINTS / 200.0) as f64;
        assert_eq!(
            out[1],
            InputEvent::Scroll {
                x: 0.5,
                y: 0.5,
                dx: 0.0,
                dy: want
            }
        );
        assert!(!s.gesture_open());
    }

    #[test]
    fn touchpad_scroll_is_a_gesture_that_ends_on_its_own_when_it_goes_quiet() {
        let mut s = InputState::default();
        let ev = [Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, 20.0),
            phase: TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        }];
        let out = s.process(&frame(&ev, Modifiers::NONE, true));
        assert_eq!(
            out[0],
            InputEvent::ScrollGesture {
                phase: ScrollPhase::Began,
                x: 0.5,
                y: 0.5
            }
        );
        assert_eq!(
            out[1],
            InputEvent::Scroll {
                x: 0.5,
                y: 0.5,
                dx: 0.0,
                dy: 0.1_f32 as f64
            }
        );
        assert!(s.gesture_open());
        assert!(s.tick(1.1).is_empty());
        assert_eq!(
            s.tick(1.2),
            vec![InputEvent::ScrollGesture {
                phase: ScrollPhase::Ended,
                x: 0.5,
                y: 0.5
            }]
        );
        assert!(!s.gesture_open());
    }

    #[test]
    fn touchpad_phase_end_closes_the_gesture_at_once() {
        let mut s = InputState::default();
        let mk = |p| {
            [Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, 4.0),
                phase: p,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Modifiers::NONE,
            }]
        };
        s.process(&frame(&mk(TouchPhase::Start), Modifiers::NONE, true));
        let out = s.process(&frame(&mk(TouchPhase::End), Modifiers::NONE, true));
        assert_eq!(
            out.last(),
            Some(&InputEvent::ScrollGesture {
                phase: ScrollPhase::Ended,
                x: 0.5,
                y: 0.5
            })
        );
        assert!(!s.gesture_open());
    }

    #[test]
    fn wheel_off_the_picture_is_ignored() {
        let mut s = InputState::default();
        let ev = [Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, 1.0),
            phase: TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        }];
        let mut f = frame(&ev, Modifiers::NONE, true);
        f.pointer_over = false;
        assert!(s.process(&f).is_empty());
    }

    #[test]
    fn unmapped_keys_send_nothing() {
        let mut s = InputState::default();
        assert!(s
            .process(&frame(
                &[key(Key::F20, true, Modifiers::NONE)],
                Modifiers::NONE,
                true
            ))
            .is_empty());
    }
}
