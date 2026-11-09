//! OWNER: worker U3. The Settings panel (iOSClient/HostSettingsPanel.swift): the Mac's stream
//! settings through the ledger (kinds 16/17), this PC's own preferences, Disconnect.
//!
//! Every control sends one [`Command`]; what it shows is the ledger's view (`settings_shown`, the
//! Mac's state with this PC's unanswered picks over it), else the Mac's own. A row whose shown
//! value differs from the Mac's is pending and says so with a small spinner. Words are
//! HostSettingsPanel.swift's and AwayCopy.swift's, with "PC" for "iPad".
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use egui::{Key, RichText};
use sill_proto::settings::{
    AwayQuality, HostSettingsChange, HostSettingsState, LinkReport, QualityPreset, RunningStream,
    StreamSettings,
};

use crate::model::{AppModel, Command, Preference};
use crate::theme;

/// How long a Mac that has sent no kind 16 may take before the panel says it is an older one
/// (HostSettingsPanel.swift:172-175).
const OLDER_MAC_AFTER: Duration = Duration::from_secs(2);
/// Room kept under the scrolling middle for the pinned Disconnect foot.
const FOOT_HEIGHT: f32 = theme::MIN_TARGET + 2.0 * theme::GAP + 4.0;

#[derive(Default)]
pub struct SettingsUi {
    /// When the panel first showed with no kind 16 yet (loading, then "Update Sill").
    waiting_since: Option<Instant>,
}

impl SettingsUi {
    /// Draws the panel's contents into `ui` (the stream screen gives it a right-side area of its
    /// own, 360 pt wide). Returns false when the person closed it (Done or Esc).
    pub fn show(&mut self, ui: &mut egui::Ui, model: &AppModel, tx: &Sender<Command>) -> bool {
        let mut open = true;
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            open = false;
        }
        let s = &model.stream;
        let mac = if s.mac_name.is_empty() {
            "Mac".to_string()
        } else {
            s.mac_name.clone()
        };
        let mac = mac.as_str();
        let state = s.settings.as_ref();
        let shown = s.settings_shown.as_ref().or(state);

        if self.header(ui, model, mac, state, &mut open) {
            // header said Done
        }
        ui.separator();

        let max_h = (ui.available_height() - FOOT_HEIGHT).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("settings-scroll")
            .max_height(max_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match (state, shown) {
                    (Some(state), Some(shown)) => {
                        self.waiting_since = None;
                        mac_group(ui, model, mac, state, shown, tx);
                        away_group(ui, mac, state);
                    }
                    _ => self.waiting(ui, mac),
                }
                this_pc_group(ui, model, tx);
            });

        ui.add_space(theme::GAP);
        if foot(ui, mac) {
            let _ = tx.send(Command::Disconnect);
        }
        open
    }

    fn header(
        &mut self,
        ui: &mut egui::Ui,
        model: &AppModel,
        mac: &str,
        state: Option<&HostSettingsState>,
        open: &mut bool,
    ) -> bool {
        let mut done = false;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 64.0).max(80.0));
                ui.add(egui::Label::new(RichText::new(mac).heading().strong()).wrap());
                if let Some(state) = state {
                    let muted = |t: String| RichText::new(t).small().color(theme::MUTED);
                    ui.add(egui::Label::new(muted(readout(state.stream.as_ref()))).wrap());
                    if state.stream.as_ref().is_some_and(|r| r.on_virtual_display) {
                        ui.add(egui::Label::new(muted("On the virtual display".into())).wrap());
                    }
                    if let Some(away) = &state.away {
                        ui.add(egui::Label::new(muted(away_header(away))).wrap());
                    }
                }
                if let Some(why) = &model.stream.locked {
                    ui.add(egui::Label::new(RichText::new(why).small()).wrap());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                let r = ui.add_sized(
                    [56.0, theme::MIN_TARGET],
                    egui::Button::new(RichText::new("Done").strong().color(theme::ACCENT)),
                );
                if r.clicked() {
                    done = true;
                }
            });
        });
        if done {
            *open = false;
        }
        done
    }

    /// No kind 16 yet: loading for 2 s, then the older-Mac line.
    fn waiting(&mut self, ui: &mut egui::Ui, mac: &str) {
        let since = *self.waiting_since.get_or_insert_with(Instant::now);
        if since.elapsed() < OLDER_MAC_AFTER {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(
                    RichText::new("Loading settings…")
                        .small()
                        .color(theme::MUTED),
                );
            });
            ui.ctx().request_repaint_after(OLDER_MAC_AFTER);
        } else {
            ui.add(
                egui::Label::new(format!(
                    "Update Sill on {mac} to change its settings from this PC."
                ))
                .wrap(),
            );
        }
        ui.add_space(theme::GAP);
    }
}

// ---- the Mac's group ------------------------------------------------------------------------

fn mac_group(
    ui: &mut egui::Ui,
    model: &AppModel,
    mac: &str,
    state: &HostSettingsState,
    shown: &HostSettingsState,
    tx: &Sender<Command>,
) {
    let theirs = &state.settings;
    let mine = &shown.settings;
    let send = |c: HostSettingsChange| {
        let _ = tx.send(Command::ChangeSettings(c));
    };

    if state.software_encoder {
        callout(
            ui,
            &format!("{mac}’s hardware encoder is busy or not responding, so for now streams run at up to 60 fps at Standard."),
            None,
        );
    }
    if let Some(problem) = &model.stream.settings_problem {
        callout(ui, problem, None);
    }
    if let Some((text, button)) = link_callout(state.link.as_ref(), state.away.as_ref(), mac) {
        if let Some(change) = callout(ui, &text, button.as_ref().map(|b| b.0.as_str())) {
            if change {
                if let Some((_, c)) = button {
                    send(c);
                }
            }
        }
    }

    // Quality
    row(ui, "Quality", mine.bitrate != theirs.bitrate, |ui| {
        let current = QualityPreset::title_for_bitrate(mine.bitrate);
        let mut picked = None;
        egui::ComboBox::from_id_salt("settings-quality")
            .selected_text(current)
            .width(ui.available_width().min(220.0))
            .show_ui(ui, |ui| {
                for p in QualityPreset::ALL {
                    let on = p.bitrate() == mine.bitrate;
                    if ui.selectable_label(on, p.title()).clicked() && !on {
                        picked = Some(p.bitrate());
                    }
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Quality"));
        if let Some(b) = picked {
            send(HostSettingsChange {
                bitrate: Some(b),
                ..Default::default()
            });
        }
    });
    // Resolution
    row(
        ui,
        "Resolution",
        mine.capture_scale != theirs.capture_scale,
        |ui| {
            let retina = mine.capture_scale >= 1.5;
            ui.horizontal(|ui| {
                if segment(ui, "Retina", retina, "Resolution: Retina") && !retina {
                    send(HostSettingsChange {
                        capture_scale: Some(2.0),
                        ..Default::default()
                    });
                }
                if segment(ui, "Standard", !retina, "Resolution: Standard") && retina {
                    send(HostSettingsChange {
                        capture_scale: Some(1.0),
                        ..Default::default()
                    });
                }
            });
        },
    );
    // Frame Rate
    row(ui, "Frame Rate", mine.max_fps != theirs.max_fps, |ui| {
        ui.horizontal(|ui| {
            for fps in [60_i64, 120] {
                let on = mine.max_fps == fps;
                if segment(
                    ui,
                    &format!("{fps} fps"),
                    on,
                    &format!("Frame Rate: {fps} fps"),
                ) && !on
                {
                    send(HostSettingsChange {
                        max_fps: Some(fps),
                        ..Default::default()
                    });
                }
            }
        });
    });
    footnote(ui, &stream_footer());
    if let Some(note) = state.away.as_ref().and_then(|a| away_footnote(a, mac)) {
        footnote(ui, &note);
    }

    // Prioritize Encoding Speed
    if let Some(v) = toggle(
        ui,
        "Prioritize Encoding Speed",
        mine.prioritize_speed,
        true,
        mine.prioritize_speed != theirs.prioritize_speed,
    ) {
        send(HostSettingsChange {
            prioritize_speed: Some(v),
            ..Default::default()
        });
    }
    footnote(
        ui,
        &format!("Lower latency when {mac} is busy, at a softer picture."),
    );

    // Virtual Display: off and impossible on this host → disabled with the reason; on, it can
    // always be turned off.
    let unavailable = !state.virtual_display_available && !mine.virtual_display;
    if let Some(v) = toggle(
        ui,
        "Virtual Display",
        mine.virtual_display,
        !unavailable,
        mine.virtual_display != theirs.virtual_display,
    ) {
        send(HostSettingsChange {
            virtual_display: Some(v),
            ..Default::default()
        });
    }
    if let Some(note) = state.virtual_display_note.as_ref().filter(|_| unavailable) {
        footnote_warn(ui, note);
    } else {
        footnote(ui, &format!("The window you pick moves onto an invisible display on {mac} while it streams, so it keeps updating when covered."));
    }
    footnote(
        ui,
        &format!("Applies to every device streaming from {mac}. The stream restarts for a moment."),
    );

    // Send Audio (Beta), then Mute right under it.
    let has_sound = mine.send_audio.is_some();
    let send_audio = mine.send_audio;
    if let Some(v) = toggle(
        ui,
        "Send Audio (Beta)",
        send_audio.unwrap_or(false),
        has_sound,
        mine.send_audio != theirs.send_audio,
    ) {
        send(HostSettingsChange {
            send_audio: Some(v),
            ..Default::default()
        });
    }
    if has_sound {
        if let Some(v) = toggle(ui, "Mute", model.prefs.mute, true, false) {
            let _ = tx.send(Command::SetPreference(Preference::Mute(v)));
        }
    }
    if let Some(note) = &state.audio_note {
        footnote_warn(ui, note);
    }
    footnote(ui, &send_audio_footer(send_audio, model.prefs.mute, mac));
    if !state.persistent {
        footnote(ui, "SillHost keeps these until it quits.");
    }
}

// ---- away from home ---------------------------------------------------------------------------

fn away_group(ui: &mut egui::Ui, mac: &str, state: &HostSettingsState) {
    let Some(a) = &state.away else { return };
    group_title(ui, "Away from home");
    ui.add(
        egui::Label::new(format!(
            "Away: {}",
            whole(&QualityPreset::short_title(
                a.away_bitrate,
                a.away_capture_scale
            ))
        ))
        .wrap(),
    );
    ui.add(
        egui::Label::new(format!(
            "Home: {}",
            whole(&QualityPreset::short_title(
                a.home_bitrate,
                a.home_capture_scale
            ))
        ))
        .wrap(),
    );
    footnote(
        ui,
        &format!(
            "Away from home, {mac} streams at {}.",
            whole(&QualityPreset::short_title(
                a.away_bitrate,
                a.away_capture_scale
            ))
        ),
    );
}

/// AwayCopy.headerLine.
fn away_header(a: &AwayQuality) -> String {
    if a.this_connection_away && a.away_running {
        format!(
            "Away: {}",
            whole(&QualityPreset::short_title(
                a.away_bitrate,
                a.away_capture_scale
            ))
        )
    } else if a.this_connection_away {
        format!(
            "Home quality: {} (a device at home is connected)",
            whole(&QualityPreset::short_title(
                a.home_bitrate,
                a.home_capture_scale
            ))
        )
    } else {
        String::new()
    }
}

/// AwayCopy.footnote (the two away cases; the at-home one needs the Mac's kind 18, which this PC
/// does not read).
fn away_footnote(a: &AwayQuality, mac: &str) -> Option<String> {
    let home = whole(&QualityPreset::short_title(
        a.home_bitrate,
        a.home_capture_scale,
    ));
    if a.this_connection_away && a.away_running {
        Some(format!("Away from home, {mac} streams at what you choose here and keeps it for next time. At home it goes back to {home}."))
    } else if a.this_connection_away {
        Some(format!("A device at home is connected, so {mac} streams at its home quality, {home}. What you choose here applies once every device is away."))
    } else {
        None
    }
}

/// AwayCopy.callout: the text, and a "Use ‹preset›" button with the change it sends. None while
/// the link keeps up ("stalled" is the Mac's alone).
fn link_callout(
    link: Option<&LinkReport>,
    away: Option<&AwayQuality>,
    mac: &str,
) -> Option<(String, Option<(String, HostSettingsChange)>)> {
    let link = link.filter(|l| l.is_behind())?;
    let quality = QualityPreset::name_for_bitrate(link.bitrate);
    if let Some(a) = away {
        if a.this_connection_away && !a.away_running {
            return Some((
                format!("The link can’t carry {quality}, which {mac} keeps while a device at home is connected."),
                None,
            ));
        }
    }
    if let Some(b) = link.suggested_bitrate {
        let scale = link.suggested_capture_scale;
        // "Low · Standard" when it changes the resolution, else the preset's name alone.
        let title = match scale {
            Some(sc) => QualityPreset::short_title(b, sc),
            None => QualityPreset::name_for_bitrate(b),
        };
        let title = whole(&title);
        return Some((
            format!("The link can’t carry {quality}. {title} is recommended."),
            Some((
                format!("Use {title}"),
                HostSettingsChange {
                    bitrate: Some(b),
                    capture_scale: scale,
                    ..Default::default()
                },
            )),
        ));
    }
    Some((
        format!("The link to {mac} is too slow for a steady picture, even at Low."),
        None,
    ))
}

// ---- this PC ----------------------------------------------------------------------------------

fn this_pc_group(ui: &mut egui::Ui, model: &AppModel, tx: &Sender<Command>) {
    group_title(ui, "This PC");
    let prefs = &model.prefs;
    let pref = |p: Preference| {
        let _ = tx.send(Command::SetPreference(p));
    };
    if model.stream.mac_clipboard {
        if let Some(v) = toggle(ui, "Share Clipboard", prefs.share_clipboard, true, false) {
            pref(Preference::ShareClipboard(v));
        }
        let mac = if model.stream.mac_name.is_empty() {
            "Mac"
        } else {
            model.stream.mac_name.as_str()
        };
        footnote(
            ui,
            &if prefs.share_clipboard {
                format!("Text you copy on {mac} is ready to paste on this PC, and text you copy here is ready to paste on {mac}.")
            } else {
                format!("Nothing you copy goes between this PC and {mac} while this is off.")
            },
        );
    }
    if let Some(v) = toggle(
        ui,
        "Reverse Scrolling",
        prefs.reverse_scrolling,
        true,
        false,
    ) {
        pref(Preference::ReverseScrolling(v));
    }
    footnote(
        ui,
        "Turns the mouse wheel and the trackpad’s two-finger scroll around.",
    );
    if let Some(v) = toggle(
        ui,
        "Windows key is Command",
        prefs.windows_key_is_command,
        true,
        false,
    ) {
        pref(Preference::WindowsKeyIsCommand(v));
    }
    footnote(
        ui,
        "Off: Ctrl does what Command does on the Mac, so Ctrl+C copies. On: the Windows key is Command.",
    );
}

fn foot(ui: &mut egui::Ui, mac: &str) -> bool {
    let label = format!("Disconnect from {mac}");
    let r = ui.add(
        egui::Button::new(RichText::new(&label).color(theme::ACCENT))
            .wrap()
            .min_size(egui::vec2(ui.available_width(), theme::MIN_TARGET)),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    r.clicked()
}

// ---- pieces -----------------------------------------------------------------------------------

fn group_title(ui: &mut egui::Ui, t: &str) {
    ui.add_space(theme::GAP);
    ui.add(egui::Label::new(RichText::new(t).strong().color(theme::BAR_LABEL)).wrap());
}

fn footnote(ui: &mut egui::Ui, t: &str) {
    ui.add(egui::Label::new(RichText::new(t).small().color(theme::MUTED)).wrap());
    ui.add_space(theme::GAP / 2.0);
}

fn footnote_warn(ui: &mut egui::Ui, t: &str) {
    ui.add(egui::Label::new(RichText::new(t).small().color(theme::PROBLEM)).wrap());
    ui.add_space(theme::GAP / 2.0);
}

/// A labelled row: the title (with a spinner while pending), then the control.
fn row(ui: &mut egui::Ui, title: &str, pending: bool, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).color(theme::TEXT));
        if pending {
            ui.add(egui::Spinner::new().size(12.0));
        }
    });
    control(ui);
    ui.add_space(theme::GAP / 2.0);
}

/// A segmented choice; true when clicked.
fn segment(ui: &mut egui::Ui, text: &str, on: bool, name: &str) -> bool {
    let r = ui.add(
        egui::Button::new(RichText::new(text).color(if on { theme::ACCENT } else { theme::TEXT }))
            .selected(on)
            .min_size(egui::vec2(0.0, theme::MIN_TARGET)),
    );
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, name));
    r.clicked()
}

/// A switch row. Returns the new value when clicked.
fn toggle(
    ui: &mut egui::Ui,
    title: &str,
    value: bool,
    enabled: bool,
    pending: bool,
) -> Option<bool> {
    let mut v = value;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let r = ui.add(egui::Checkbox::new(&mut v, title));
            changed = r.changed();
        });
        if pending {
            ui.add(egui::Spinner::new().size(12.0));
        }
    });
    changed.then_some(v)
}

/// A callout in the problem colour, with an optional action button. Returns Some(true) when the
/// button was pressed, Some(false) when shown without a click, None when there was no button.
fn callout(ui: &mut egui::Ui, text: &str, button: Option<&str>) -> Option<bool> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(theme::PROBLEM.gamma_multiply(0.16))
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text.to_string()).small()).wrap());
            if let Some(b) = button {
                let r = ui.add(
                    egui::Button::new(RichText::new(b).color(theme::ACCENT))
                        .wrap()
                        .min_size(egui::vec2(ui.available_width(), theme::MIN_TARGET)),
                );
                clicked = Some(r.clicked());
            }
        });
    ui.add_space(theme::GAP);
    clicked
}

// ---- words ------------------------------------------------------------------------------------

/// "2032×1312 · 60 fps · 150 Mbps"; "Nothing is streaming" with no stream.
fn readout(r: Option<&RunningStream>) -> String {
    match r {
        Some(r) => format!(
            "{}×{}\u{00A0}· {}\u{00A0}fps\u{00A0}· {}\u{00A0}Mbps",
            r.width, r.height, r.fps, r.mbps
        ),
        None => "Nothing is streaming".into(),
    }
}

/// A quality title kept whole on its line (no-break spaces).
fn whole(t: &str) -> String {
    t.replace(' ', "\u{00A0}")
}

fn stream_footer() -> String {
    "Quality is per 60 fps; a 120 fps stream gets twice as much. Ultra and Extreme need a wired network or very fast Wi\u{2011}Fi; if the picture lags, step down. A frame rate above what this PC’s screen refreshes changes nothing for it.".into()
}

fn send_audio_footer(send: Option<bool>, muted: bool, mac: &str) -> String {
    let mute = format!(
        " Mute silences it on this PC alone; {mac} keeps sending and playing it.{}",
        if muted { " This PC is muted." } else { "" }
    );
    match send {
        Some(true) => format!(
            "Beta. Every connected device hears what streams, and {mac} keeps playing it too.{mute}"
        ),
        Some(false) => format!(
            "Beta. Turn this on to hear what streams on every connected device; {mac} keeps playing it too.{mute}"
        ),
        None => format!("Beta. Update Sill on {mac} to hear its sound on this PC."),
    }
}

// ---- TEST ONLY sample data ----------------------------------------------------------------------

/// TEST ONLY: labelled sample Mac states for the panel ("settings-default", "settings-away",
/// "settings-pending", "settings-noaudio"). The pending case's Mac state is the default one; its
/// ledger view is [`mock_shown`].
#[allow(dead_code)] // TEST ONLY hook, wired by the stream screen
pub fn mock_state(case: &str) -> Option<HostSettingsState> {
    let base = |away: Option<AwayQuality>, audio: Option<bool>| HostSettingsState {
        settings: StreamSettings {
            max_fps: 60,
            bitrate: QualityPreset::Balanced.bitrate(),
            capture_scale: 2.0,
            prioritize_speed: false,
            virtual_display: true,
            direct_wireless: None,
            send_audio: audio,
        },
        persistent: true,
        virtual_display_available: true,
        virtual_display_note: None,
        software_encoder: false,
        stream: Some(RunningStream {
            width: 2032,
            height: 1312,
            fps: 60,
            mbps: 15,
            on_virtual_display: true,
        }),
        answering: None,
        away,
        link: None,
        audio_note: None,
        session_paired: None,
    };
    match case {
        "settings-default" | "settings-pending" => Some(base(None, Some(false))),
        "settings-noaudio" => Some(base(None, None)),
        "settings-away" => {
            let mut s = base(
                Some(AwayQuality {
                    home_bitrate: QualityPreset::Pro.bitrate(),
                    home_capture_scale: 2.0,
                    away_bitrate: QualityPreset::Low.bitrate(),
                    away_capture_scale: 1.0,
                    this_connection_away: true,
                    away_running: true,
                }),
                Some(true),
            );
            s.link = Some(LinkReport {
                state: LinkReport::BEHIND.into(),
                withheld_per_second: 52,
                bitrate: QualityPreset::Pro.bitrate(),
                carried_kbps: Some(6400),
                suggested_bitrate: Some(QualityPreset::Low.bitrate()),
                suggested_capture_scale: Some(1.0),
            });
            Some(s)
        }
        _ => None,
    }
}

/// TEST ONLY: the ledger's view for a case: for "settings-pending" Pro and 120 fps picked over
/// the Mac's Balanced and 60, unanswered.
#[allow(dead_code)]
pub fn mock_shown(case: &str) -> Option<HostSettingsState> {
    let mut s = mock_state(case)?;
    if case == "settings-pending" {
        s.settings.bitrate = QualityPreset::Pro.bitrate();
        s.settings.max_fps = 120;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_case(case: &str, width: f32) -> egui::Vec2 {
        let ctx = egui::Context::default();
        let mut model = AppModel::default();
        model.stream.mac_name = "Studio Mac with a rather long name indeed".into();
        model.stream.settings = mock_state(case);
        model.stream.settings_shown = mock_shown(case);
        model.stream.mac_clipboard = true;
        model.stream.settings_problem = Some("Studio kept its setting.".into());
        if case == "settings-away" {
            model.stream.locked = Some("Studio is locked.".into());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut ui_state = SettingsUi::default();
        let mut used = egui::Vec2::ZERO;
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 700.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                ui.set_max_width(width);
                let r = ui.scope(|ui| ui_state.show(ui, &model, &tx));
                used = r.response.rect.size();
            });
            out.textures_delta.clear();
        }
        assert!(rx.try_recv().is_err(), "no command without a click");
        used
    }

    #[test]
    fn lays_out_every_case_at_both_widths() {
        for case in [
            "settings-default",
            "settings-away",
            "settings-pending",
            "settings-noaudio",
            "none",
        ] {
            for w in [360.0, 280.0] {
                let used = run_case(case, w);
                assert!(used.x <= w + 1.0, "{case} at {w}: used {used:?}");
            }
        }
    }

    #[test]
    fn link_callout_words() {
        let s = mock_state("settings-away").unwrap();
        let (t, b) = link_callout(s.link.as_ref(), s.away.as_ref(), "Studio").unwrap();
        assert_eq!(
            t,
            "The link can’t carry Pro. Low\u{00A0}·\u{00A0}Standard is recommended."
        );
        let (title, ch) = b.unwrap();
        assert_eq!(title, "Use Low\u{00A0}·\u{00A0}Standard");
        assert_eq!(ch.bitrate, Some(4_000_000));
        assert_eq!(ch.capture_scale, Some(1.0));
    }

    #[test]
    fn away_words() {
        let s = mock_state("settings-away").unwrap();
        assert_eq!(
            away_header(s.away.as_ref().unwrap()),
            "Away: Low\u{00A0}·\u{00A0}Standard"
        );
    }
}
