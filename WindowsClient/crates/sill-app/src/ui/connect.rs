//! OWNER: worker U1. The connect screen and pairing (iOSClient/ContentView.swift ConnectScreen,
//! AddMacCard.swift), for mouse and keyboard.
use std::sync::mpsc::Sender;

use egui::{
    vec2, Align2, Color32, CornerRadius, FontId, Key, Modifiers, Rect, RichText, Role, Sense,
    Stroke, StrokeKind, TextEdit, Ui, UiBuilder, WidgetInfo,
};
use sill_net::damm;

use crate::model::{AppModel, Command, MacRow, PairingState};
use crate::theme;

const COLUMN_MAX: f32 = 520.0;
const ROW_HEIGHT: f32 = 52.0;
const SIDE: f32 = 16.0;
const SIZE_BODY: f32 = 13.0;
const SIZE_ROW: f32 = 15.0;
const SIZE_TITLE: f32 = 24.0;

#[derive(Default)]
pub struct ConnectUi {
    adding: bool,
    address: String,
    code: String,
    /// Focus the Address field on the frame the card opens.
    focus_address: bool,
    /// The last PairingState seen, to open the card when a Mac shows its code.
    last_code_shown: bool,
}

impl ConnectUi {
    pub fn show(&mut self, ui: &mut Ui, model: &AppModel, tx: &Sender<Command>) {
        let c = &model.connect;
        let shown = matches!(c.pairing, PairingState::CodeShown { .. });
        if shown && !self.last_code_shown {
            self.open_card();
        }
        self.last_code_shown = shown;

        egui::Panel::bottom("connect-footer")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ui, footer);

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let avail = ui.available_width();
                    let w = (avail - 2.0 * SIDE).clamp(0.0, COLUMN_MAX);
                    let left = ui.max_rect().left() + (avail - w) / 2.0;
                    let rect = Rect::from_min_size(
                        egui::pos2(left, ui.cursor().top() + 24.0),
                        vec2(w, 0.0),
                    );
                    let rect = Rect::from_min_max(rect.min, egui::pos2(rect.max.x, f32::INFINITY));
                    let used = ui
                        .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                            ui.set_width(w);
                            self.column(ui, model, tx);
                        })
                        .response
                        .rect;
                    ui.allocate_rect(
                        Rect::from_min_max(
                            egui::pos2(left, ui.cursor().top()),
                            egui::pos2(left + w, used.bottom() + 16.0),
                        ),
                        Sense::hover(),
                    );
                });
        });
    }

    fn open_card(&mut self) {
        self.adding = true;
        self.focus_address = true;
    }

    fn close_card(&mut self, tx: &Sender<Command>) {
        self.adding = false;
        self.code.clear();
        self.address.clear();
        let _ = tx; // Cancel only folds the card; a pairing in flight is the controller's.
    }

    fn column(&mut self, ui: &mut Ui, model: &AppModel, tx: &Sender<Command>) {
        let c = &model.connect;
        let title = if self.adding {
            "Add a Mac"
        } else {
            "Connect to a Mac"
        };
        ui.add(
            egui::Label::new(RichText::new(title).size(SIZE_TITLE).strong())
                .wrap_mode(egui::TextWrapMode::Wrap),
        )
        .widget_info(|| WidgetInfo::labeled(Role::Label, true, title));
        if !c.status.is_empty() {
            ui.add(egui::Label::new(
                RichText::new(&c.status).size(SIZE_BODY).color(theme::MUTED),
            ));
        }
        if let Some(n) = &c.notice {
            ui.add(egui::Label::new(
                RichText::new(n).size(SIZE_BODY).color(theme::TEXT),
            ));
        }
        ui.add_space(theme::GAP);

        if self.adding {
            self.card(ui, model, tx);
            return;
        }

        if c.rows.is_empty() && !c.nothing_found {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(theme::MUTED));
                ui.label(
                    RichText::new("Looking for Macs…")
                        .size(SIZE_BODY)
                        .color(theme::MUTED),
                );
            });
        }
        for row in &c.rows {
            self.row(ui, row, tx);
        }
        if c.public_network {
            problem_line(
                ui,
                "Windows is treating this network as public, so Sill can’t find your Mac. Set it to Private in Settings › Network & internet.",
            );
        }
        if c.nothing_found && c.rows.is_empty() {
            ui.add(egui::Label::new(
                RichText::new(
                    "Your Mac has to be on the same network as this PC, with Sill open. If it isn’t, use Add a Mac… with an address from your VPN, such as Tailscale.",
                )
                .size(SIZE_BODY)
                .color(theme::MUTED),
            ));
        }
        pairing_state(ui, &c.pairing);
        ui.add_space(theme::GAP);
        let add = ui.add(
            egui::Button::new(
                RichText::new("Add a Mac…")
                    .size(SIZE_ROW)
                    .color(theme::ACCENT)
                    .strong(),
            )
            .frame(false)
            .min_size(vec2(0.0, theme::MIN_TARGET)),
        );
        add.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Add a Mac"));
        if add.clicked() {
            self.open_card();
        }
    }

    fn row(&mut self, ui: &mut Ui, row: &MacRow, tx: &Sender<Command>) {
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_HEIGHT), Sense::click());
        let label = match &row.word {
            Some(word) => format!("{}, {}", row.name, word),
            None => row.name.clone(),
        };
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
        let lit = row.busy || resp.hovered();
        let fill = if lit {
            theme::CONTROL_OPEN
        } else {
            theme::CONTROL
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
        if resp.has_focus() {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(theme::RADIUS),
                Stroke::new(2.0, theme::ACCENT),
                StrokeKind::Inside,
            );
        }
        let icon = Rect::from_center_size(
            egui::pos2(rect.left() + 28.0, rect.center().y),
            vec2(26.0, 20.0),
        );
        desktop_icon(
            ui,
            icon,
            if row.busy {
                theme::ACCENT
            } else {
                theme::BAR_LABEL
            },
        );

        let mut right = rect.right() - 14.0;
        let mut word_w = 0.0;
        if let Some(word) = &row.word {
            let g = ui.painter().layout_no_wrap(
                word.clone(),
                FontId::proportional(SIZE_BODY),
                theme::MUTED,
            );
            word_w = g.size().x;
            ui.painter().galley(
                egui::pos2(right - word_w, rect.center().y - g.size().y / 2.0),
                g,
                theme::MUTED,
            );
            right -= word_w + 12.0;
        }
        let name_left = rect.left() + 56.0;
        let max_w = (right - name_left).max(20.0);
        let mut job = egui::text::LayoutJob::single_section(
            row.name.clone(),
            egui::TextFormat::simple(FontId::proportional(SIZE_ROW), theme::TEXT),
        );
        job.wrap.max_width = max_w;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        let truncated = g.size().x > max_w - 1.0
            || g.rows.len() > 1
            || row.name.chars().count() > g.text().chars().count();
        ui.painter().galley(
            egui::pos2(name_left, rect.center().y - g.size().y / 2.0),
            g,
            theme::TEXT,
        );
        let _ = word_w;

        let resp = if truncated {
            resp.on_hover_text(&row.name)
        } else {
            resp
        };
        if resp.clicked() {
            let _ = tx.send(Command::Connect(row.id.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Connect").clicked() {
                let _ = tx.send(Command::Connect(row.id.clone()));
                ui.close();
            }
            if let Some(id) = &row.mac_id {
                if ui.button("Connect Remotely").clicked() {
                    let _ = tx.send(Command::ConnectRemotely(id.clone()));
                    ui.close();
                }
                if ui.button(format!("Forget {}", row.name)).clicked() {
                    let _ = tx.send(Command::Forget(id.clone()));
                    ui.close();
                }
            }
        });
    }

    fn card(&mut self, ui: &mut Ui, model: &AppModel, tx: &Sender<Command>) {
        let pairing = &model.connect.pairing;
        let working = matches!(
            pairing,
            PairingState::Working(_) | PairingState::Paired { .. }
        );
        let shown = match pairing {
            PairingState::CodeShown { mac } => Some(mac.clone()),
            _ => None,
        };
        ui.add(egui::Label::new(
            RichText::new(match &shown {
                Some(mac) => format!("{mac} is showing a code now. Type it here."),
                None => "Pair once, and Sill reaches your Mac from anywhere, through your VPN or the internet.".to_string(),
            })
            .size(SIZE_BODY)
            .color(theme::MUTED),
        ));
        if shown.is_none() {
            ui.add(egui::Label::new(
                RichText::new("On your Mac, choose Pair iPhone or iPad… in the Sill menu, then type the address and code it shows.")
                    .size(SIZE_BODY)
                    .color(theme::TEXT),
            ));
        }
        ui.add_space(theme::GAP);

        let mut link: Option<String> = None;
        ui.add_enabled_ui(!working, |ui| {
            let addr_label = ui.label(RichText::new("Address").size(SIZE_BODY).color(theme::MUTED));
            let addr = ui.add(
                TextEdit::singleline(&mut self.address)
                    .hint_text("100.101.102.103 or mac.example.net")
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 8.0)),
            );
            addr.clone().labelled_by(addr_label.id);
            if self.focus_address {
                addr.request_focus();
                self.focus_address = false;
            }
            let code_label = ui.label(RichText::new("Code").size(SIZE_BODY).color(theme::MUTED));
            let code = ui.add(
                TextEdit::singleline(&mut self.code)
                    .hint_text("0000 0000 0000")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 8.0)),
            );
            code.clone().labelled_by(code_label.id);
            if code.changed() {
                let before = self.code.clone();
                let digits: String = before
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .take(damm::LENGTH)
                    .collect();
                let grouped = damm::grouped(&digits);
                if grouped != before {
                    self.code = grouped;
                    if let Some(mut st) = TextEdit::load_state(ui.ctx(), code.id) {
                        let end = egui::text::CCursor::new(self.code.chars().count());
                        st.cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(end)));
                        st.store(ui.ctx(), code.id);
                    }
                }
            }
            for s in [&self.address, &self.code] {
                if s.trim_start().starts_with("sill://pair") {
                    link = Some(s.trim().to_string());
                }
            }
            let enter =
                (addr.lost_focus() || code.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
            let checked = damm::check(&self.code);
            if let Err(damm::Problem::Typo) = checked {
                problem_line(ui, "That code has a typo. Check it against your Mac.");
            }
            ui.add_space(theme::GAP);
            let ready = checked.is_ok() && !self.address.trim().is_empty();
            let pair = ui.add_enabled(
                ready,
                egui::Button::new(
                    RichText::new("Pair")
                        .size(SIZE_ROW)
                        .strong()
                        .color(Color32::BLACK),
                )
                .fill(theme::ACCENT)
                .min_size(vec2(96.0, 40.0)),
            );
            pair.widget_info(|| WidgetInfo::labeled(Role::Button, ready, "Pair"));
            if (pair.clicked() || (enter && ready)) && link.is_none() {
                if let Ok(code) = checked {
                    let _ = tx.send(Command::PairWithCode {
                        addr: self.address.trim().to_string(),
                        code,
                    });
                }
            }
        });
        for ev in ui.input(|i| i.events.clone()) {
            if let egui::Event::Paste(t) = ev {
                if t.trim_start().starts_with("sill://pair") {
                    link = Some(t.trim().to_string());
                }
            }
        }
        if let Some(l) = link {
            let _ = tx.send(Command::PairWithLink(l));
            self.address.clear();
            self.code.clear();
        }

        if shown.is_none() {
            pairing_state(ui, pairing);
        }
        ui.add_space(theme::GAP);
        let cancel = ui.add(
            egui::Button::new(
                RichText::new("Cancel")
                    .size(SIZE_ROW)
                    .color(theme::ACCENT)
                    .strong(),
            )
            .frame(false)
            .min_size(vec2(0.0, theme::MIN_TARGET)),
        );
        cancel.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Cancel"));
        if cancel.clicked() || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.close_card(tx);
        }
    }
}

fn pairing_state(ui: &mut Ui, p: &PairingState) {
    match p {
        PairingState::Idle => {}
        PairingState::Working(words) => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(theme::MUTED));
                ui.label(RichText::new(words).size(SIZE_BODY).color(theme::MUTED));
            });
        }
        PairingState::CodeShown { mac } => {
            ui.label(
                RichText::new(format!("{mac} is showing a code now."))
                    .size(SIZE_BODY)
                    .color(theme::TEXT),
            );
        }
        PairingState::Paired { mac } => {
            ui.label(
                RichText::new(format!("✓ Paired with {mac}."))
                    .size(SIZE_BODY)
                    .color(theme::TEXT),
            );
        }
        PairingState::Failed(words) => problem_line(ui, words),
    }
}

fn problem_line(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(
        RichText::new(text).size(SIZE_BODY).color(theme::PROBLEM),
    ));
}

fn footer(ui: &mut Ui) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("Needs the free Sill app on your Mac.")
                .size(SIZE_BODY)
                .color(theme::MUTED),
        );
        ui.horizontal_wrapped(|ui| {
            let links = [
                ("Get it", "https://getsill.app/download"),
                ("Support", "https://getsill.app/support"),
                ("Privacy Policy", "https://getsill.app/privacy"),
            ];
            let widths: f32 = links
                .iter()
                .map(|(n, _)| {
                    ui.painter()
                        .layout_no_wrap((*n).into(), FontId::proportional(SIZE_BODY), theme::ACCENT)
                        .size()
                        .x
                        + 8.0
                })
                .sum();
            let total = widths + 2.0 * ui.spacing().item_spacing.x;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            for (name, url) in links {
                let b = ui.add(
                    egui::Button::new(RichText::new(name).size(SIZE_BODY).color(theme::ACCENT))
                        .frame(false)
                        .min_size(vec2(0.0, theme::MIN_TARGET)),
                );
                b.widget_info(|| WidgetInfo::labeled(Role::Link, true, name));
                if b.on_hover_text(url).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
        });
    });
}

/// The desktop-computer glyph of a Mac row, drawn from shapes (no asset).
fn desktop_icon(ui: &Ui, r: Rect, color: Color32) {
    let p = ui.painter();
    let screen = Rect::from_min_max(r.min, egui::pos2(r.max.x, r.max.y - 5.0));
    p.rect_stroke(
        screen,
        CornerRadius::same(3),
        Stroke::new(1.5, color),
        StrokeKind::Inside,
    );
    p.line_segment(
        [
            egui::pos2(r.center().x, screen.bottom()),
            egui::pos2(r.center().x, r.max.y - 1.0),
        ],
        Stroke::new(1.5, color),
    );
    p.line_segment(
        [
            egui::pos2(r.center().x - 6.0, r.max.y - 1.0),
            egui::pos2(r.center().x + 6.0, r.max.y - 1.0),
        ],
        Stroke::new(1.5, color),
    );
    let _ = Align2::CENTER_CENTER;
}

/// TEST ONLY: fill `model` for a connect-screen SILL_TEST_MOCK case; false when it is not one.
pub fn mock_case(case: &str, model: &mut AppModel) -> bool {
    use crate::model::Screen;
    let row = |id: &str, name: &str, word: Option<&str>, mac: Option<&str>, busy: bool| MacRow {
        id: id.into(),
        name: name.into(),
        word: word.map(Into::into),
        mac_id: mac.map(Into::into),
        busy,
    };
    let c = &mut model.connect;
    model.screen = Screen::Connect;
    match case {
        "connect-empty" => {
            c.status = "Looking for Macs on this network".into();
            c.nothing_found = true;
        }
        "connect-rows" => {
            c.status = "Looking for Macs on this network".into();
            c.rows = vec![
                row("saved:S1", "Sample Mac mini", Some("Wi-Fi"), Some("S1"), false),
                row("bonjour:Sample Studio", "Sample Studio", Some("Not paired"), None, true),
                row("saved:S3", "Sample MacBook Pro with a very long name that keeps going past the edge of the row", Some("Remote"), Some("S3"), false),
            ];
        }
        "connect-card" => {
            c.pairing = PairingState::CodeShown {
                mac: "Sample Studio".into(),
            };
        }
        "connect-pairing" => {
            c.pairing = PairingState::Working("Connecting to Sample Mac mini…".into());
        }
        "connect-failed" => {
            c.pairing = PairingState::Failed(
                "That code didn’t work. Check the code on your Mac. 4 tries left.".into(),
            );
        }
        "connect-public" => {
            c.status = "Looking for Macs on this network".into();
            c.nothing_found = true;
            c.public_network = true;
        }
        _ => return false,
    }
    true
}
