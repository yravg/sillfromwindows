//! The pieces every screen draws with, in the iOS app's sizes and look (StreamScreen.swift's
//! BarButton, WindowThumbnail and settings rows). OWNER: the lead.
use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect,
    Response, RichText, Role, Sense, Stroke, StrokeKind, TextureHandle, Ui, WidgetInfo,
};
use sill_proto::switcher::WindowInfo;

use crate::theme::{self, BarMetrics};

/// The symbols the app draws itself (SF Symbols' equivalents): a vector at 22 pt, never a font.
#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Search,
    Menu,
    Command,
    Desktop,
    Gear,
    Warning,
    XCircle,
}

pub fn draw_icon(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let stroke = Stroke::new((s / 12.0).max(1.6), color);
    let line = |a: Pos2, b: Pos2| p.line_segment([a, b], stroke);
    match icon {
        Icon::Search => {
            let r = s * 0.27;
            let cc = c + vec2(-s * 0.07, -s * 0.07);
            p.circle_stroke(cc, r, stroke);
            line(cc + vec2(r * 0.72, r * 0.72), c + vec2(s * 0.32, s * 0.32));
        }
        Icon::Menu => {
            // A menu card: the iOS "Menus" symbol, not a hamburger.
            let card = Rect::from_center_size(c, vec2(s * 0.66, s * 0.52));
            p.rect_stroke(card, CornerRadius::same(2), stroke, StrokeKind::Inside);
            for i in -1..=1 {
                let y = c.y + i as f32 * s * 0.13;
                line(
                    pos2(card.left() + s * 0.12, y),
                    pos2(card.right() - s * 0.12, y),
                );
            }
        }
        Icon::Command => {
            // The four loops and the square between them.
            let q = s * 0.16;
            let rr = q * 0.55;
            for sx in [-1.0, 1.0] {
                for sy in [-1.0, 1.0] {
                    p.circle_stroke(c + vec2(sx * q * 1.55, sy * q * 1.55), rr, stroke);
                }
            }
            line(c + vec2(-q, -q), c + vec2(q, -q));
            line(c + vec2(-q, q), c + vec2(q, q));
            line(c + vec2(-q, -q), c + vec2(-q, q));
            line(c + vec2(q, -q), c + vec2(q, q));
            line(c + vec2(-q, -q), c + vec2(-q * 2.1, -q * 2.1));
            line(c + vec2(q, -q), c + vec2(q * 2.1, -q * 2.1));
            line(c + vec2(-q, q), c + vec2(-q * 2.1, q * 2.1));
            line(c + vec2(q, q), c + vec2(q * 2.1, q * 2.1));
        }
        Icon::Desktop => {
            let w = s * 0.64;
            let h = s * 0.42;
            let r = Rect::from_center_size(c + vec2(0.0, -s * 0.06), vec2(w, h));
            p.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Inside);
            line(pos2(c.x, r.bottom()), pos2(c.x, c.y + s * 0.27));
            line(
                pos2(c.x - w * 0.26, c.y + s * 0.29),
                pos2(c.x + w * 0.26, c.y + s * 0.29),
            );
        }
        Icon::Gear => {
            let rr = s * 0.24;
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let (sin, cos) = a.sin_cos();
                line(
                    c + vec2(cos * rr * 1.25, sin * rr * 1.25),
                    c + vec2(cos * rr * 1.78, sin * rr * 1.78),
                );
            }
            p.circle_stroke(c, rr, stroke);
            p.circle_stroke(c, rr * 0.36, stroke);
        }
        Icon::Warning => {
            let pts = vec![
                pos2(c.x, c.y - s * 0.32),
                pos2(c.x - s * 0.36, c.y + s * 0.30),
                pos2(c.x + s * 0.36, c.y + s * 0.30),
            ];
            p.add(egui::Shape::closed_line(pts, stroke));
            line(pos2(c.x, c.y - s * 0.10), pos2(c.x, c.y + s * 0.12));
            p.circle_filled(pos2(c.x, c.y + s * 0.23), stroke.width * 0.55, color);
        }
        Icon::XCircle => {
            let r = s * 0.34;
            p.circle_stroke(c, r, stroke);
            line(c + vec2(-r * 0.42, -r * 0.42), c + vec2(r * 0.42, r * 0.42));
            line(c + vec2(r * 0.42, -r * 0.42), c + vec2(-r * 0.42, r * 0.42));
        }
    }
}

/// A 66Ã—66 bar button: the icon over its label, as BarButton. Returns its response.
pub fn bar_button(
    ui: &mut Ui,
    m: &BarMetrics,
    icon: Option<Icon>,
    label: &str,
    open: bool,
    enabled: bool,
    hint: &str,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(m.button_w, m.button_h), sense);
    let radius: u8 = if m.button_h >= 60.0 { 16 } else { 14 };
    let hovered = enabled && resp.hovered();
    let fill = if open || hovered {
        theme::CONTROL_OPEN
    } else {
        theme::CONTROL
    };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.35) };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(radius), fade(fill));
    let icon_color = fade(if open { theme::ACCENT } else { theme::TEXT });
    let label_color = fade(if open {
        theme::ACCENT
    } else {
        theme::BAR_LABEL
    });
    if let Some(i) = icon {
        draw_icon(
            p,
            Rect::from_center_size(
                pos2(rect.center().x, rect.top() + m.button_h * 0.37),
                vec2(theme::ICON, theme::ICON),
            ),
            i,
            icon_color,
        );
    }
    p.text(
        pos2(rect.center().x, rect.top() + m.button_h * 0.70),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(theme::TEXT_BAR_LABEL),
        label_color,
    );
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, hint));
    resp.on_hover_text(hint)
}

/// The Aa button: a two-line label ("Aa" over the value), the only bar button whose top is text.
pub fn bar_text_button(
    ui: &mut Ui,
    m: &BarMetrics,
    top: &str,
    bottom: &str,
    open: bool,
    enabled: bool,
    hint: &str,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(m.button_w, m.button_h), sense);
    let radius: u8 = if m.button_h >= 60.0 { 16 } else { 14 };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.35) };
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same(radius),
        fade(if resp.hovered() && enabled {
            theme::CONTROL_OPEN
        } else {
            theme::CONTROL
        }),
    );
    p.text(
        pos2(rect.center().x, rect.top() + m.button_h * 0.37),
        Align2::CENTER_CENTER,
        top,
        FontId::proportional(20.0),
        fade(if open { theme::ACCENT } else { theme::TEXT }),
    );
    p.text(
        pos2(rect.center().x, rect.top() + m.button_h * 0.70),
        Align2::CENTER_CENTER,
        bottom,
        FontId::proportional(theme::TEXT_BAR_LABEL),
        fade(if open {
            theme::ACCENT
        } else {
            theme::BAR_LABEL
        }),
    );
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, hint));
    resp.on_hover_text(hint)
}

/// An iOS switch (46Ã—28); on is the accent, off a muted track. Returns its response; the caller
/// sends the new value when it is clicked.
pub fn switch(ui: &mut Ui, on: bool, enabled: bool, name: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(50.0, 30.0), Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("on"), on && enabled, 0.12);
    let p = ui.painter();
    let track = if !enabled {
        theme::SWITCH_OFF.gamma_multiply(0.35)
    } else if on {
        theme::ACCENT
    } else {
        theme::SWITCH_OFF
    };
    p.rect_filled(rect, CornerRadius::same(15), track);
    let x = rect.left() + 15.0 + (rect.width() - 30.0) * t;
    let knob = if enabled {
        Color32::WHITE
    } else {
        Color32::from_gray(170)
    };
    p.circle_filled(pos2(x, rect.center().y), 12.0, knob);
    p.circle_stroke(
        pos2(x, rect.center().y),
        12.0,
        Stroke::new(1.0, Color32::from_black_alpha(40)),
    );
    resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, on, name));
    resp
}

/// A segmented control (iOS's `.segmented`): a dark track with the picked pill lighter. Returns the
/// index clicked, if any.
pub fn segmented(ui: &mut Ui, options: &[(&str, bool)], name: &str) -> Option<usize> {
    let font = FontId::proportional(13.0);
    let pad = 3.0;
    let widths: Vec<f32> = options
        .iter()
        .map(|(text, _)| {
            ui.painter()
                .layout_no_wrap(text.to_string(), font.clone(), theme::TEXT)
                .size()
                .x
                + 24.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 2.0 * pad;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 32.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), theme::SEGMENT_TRACK);
    let mut x = rect.left() + pad;
    let mut picked = None;
    for (i, (text, on)) in options.iter().enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.top() + pad), vec2(widths[i], 32.0 - 2.0 * pad));
        let resp = ui.interact(
            seg,
            ui.scope_id().with(("segment", name, i)),
            Sense::click(),
        );
        if *on {
            p.rect_filled(seg, CornerRadius::same(7), theme::SEGMENT_ON);
        }
        p.text(
            seg.center(),
            Align2::CENTER_CENTER,
            *text,
            font.clone(),
            if *on { Color32::WHITE } else { theme::TEXT },
        );
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, *on, name));
        if resp.clicked() {
            picked = Some(i);
        }
        x += widths[i];
    }
    picked
}

/// A group of rows on the drawer's control colour, 12 pt corners (HostSettingsPanel's Rows).
pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(theme::CONTROL)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(0, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// A row in a card: at least 44 pt tall, inset 14, the control at the trailing edge.
pub fn settings_row(ui: &mut Ui, title: &str, pending: bool, control: impl FnOnce(&mut Ui)) {
    Frame::new()
        .inner_margin(Margin::symmetric(14, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(theme::ROW_H);
            ui.horizontal_centered(|ui| {
                ui.label(
                    RichText::new(title)
                        .size(theme::TEXT_ROW)
                        .color(theme::TEXT),
                );
                if pending {
                    ui.add(egui::Spinner::new().size(12.0));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| control(ui));
            });
        });
}

/// A toggle row: the title, the switch at the trailing edge; the new value when clicked.
pub fn toggle_row(
    ui: &mut Ui,
    title: &str,
    value: bool,
    enabled: bool,
    pending: bool,
) -> Option<bool> {
    let mut changed = None;
    settings_row(ui, title, pending, |ui| {
        if switch(ui, value, enabled, title).clicked() && enabled {
            changed = Some(!value);
        }
    });
    changed
}

/// A row divider inside a card.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0, theme::HAIRLINE);
}

/// A group's footer: small, muted, from the leading edge, 14 in from the panel's 14.
pub fn footnote(ui: &mut Ui, text: &str, warn: bool) {
    ui.add_space(6.0);
    ui.indent(14, |ui| {
        ui.add(
            egui::Label::new(RichText::new(text).size(theme::TEXT_SMALL).color(if warn {
                theme::WARN
            } else {
                theme::MUTED
            }))
            .wrap(),
        );
    });
    ui.add_space(12.0);
}

/// A group title ("This PC", "Away from home"), as the iOS footnote-weight headers.
pub fn group_title(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.indent(14, |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .size(theme::TEXT_GROUP)
                    .color(theme::MUTED),
            )
            .wrap(),
        );
    });
    ui.add_space(6.0);
}

/// A callout: an orange-tinted card with a warning, and an optional action button. Returns true
/// when the action was clicked.
pub fn callout(ui: &mut Ui, text: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    Frame::new()
        .fill(theme::WARN.gamma_multiply(0.14))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                draw_icon(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(ui.cursor().left() + 7.0, ui.cursor().top() + 8.0),
                        vec2(14.0, 14.0),
                    ),
                    Icon::Warning,
                    theme::WARN,
                );
                ui.add_space(20.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .size(theme::TEXT_SMALL)
                            .color(theme::TEXT),
                    )
                    .wrap(),
                );
            });
            if let Some(a) = action {
                ui.add_space(8.0);
                let r = Frame::new()
                    .fill(theme::CONTROL)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                RichText::new(a)
                                    .size(theme::TEXT_SMALL)
                                    .color(theme::ACCENT),
                            )
                            .wrap(),
                        );
                    })
                    .response;
                let r = r.interact(Sense::click());
                r.widget_info(|| WidgetInfo::labeled(Role::Button, true, a));
                clicked = r.clicked();
            }
        });
    ui.add_space(10.0);
    clicked
}

/// The floating panel's frame: the bar's colour, 20 pt corners, the iOS panel's rim and shadow.
pub fn panel_frame() -> Frame {
    Frame::new()
        .fill(theme::BAR)
        .corner_radius(CornerRadius::same(theme::PANEL_RADIUS))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(26)))
        .inner_margin(Margin::same(14))
        .shadow(egui::epaint::Shadow {
            offset: [0, 18],
            blur: 30,
            spread: 0,
            color: Color32::from_black_alpha(170),
        })
}

/// An app icon: the Mac's picture, or the first two letters on the fallback colour. The caller
/// draws the bar-coloured ring where it needs one.
pub fn app_icon(
    ui: &Ui,
    rect: Rect,
    name: &str,
    icon: Option<&TextureHandle>,
    radius: u8,
    font_size: f32,
) {
    let p = ui.painter();
    match icon {
        Some(h) => {
            egui::Image::from_texture(egui::load::SizedTexture::new(h.id(), rect.size()))
                .corner_radius(CornerRadius::same(radius))
                .paint_at(ui, rect);
        }
        None => {
            p.rect_filled(rect, CornerRadius::same(radius), theme::ICON_FALLBACK);
            let badge: String = name
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .collect::<String>()
                .to_uppercase();
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                badge,
                FontId::proportional(font_size),
                Color32::WHITE,
            );
        }
    }
}

/// A window thumbnail: the Mac's picture (centre-cropped), rim, active halo and app badge.
#[allow(clippy::too_many_arguments)]
pub fn thumbnail(
    ui: &Ui,
    rect: Rect,
    m: &BarMetrics,
    w: &WindowInfo,
    picture: Option<(&TextureHandle, u32, u32)>,
    icon: Option<&TextureHandle>,
    active: bool,
    hovered: bool,
) {
    let p = ui.painter();
    let radius = CornerRadius::same(m.thumb_radius);
    if active {
        p.rect_stroke(
            rect.expand(5.0),
            CornerRadius::same(m.thumb_radius + 5),
            Stroke::new(2.0, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
    match picture {
        Some((h, iw, ih)) => {
            let img = if iw > 0 && ih > 0 {
                let want = rect.width() / rect.height();
                let have = iw as f32 / ih as f32;
                if have > want {
                    let f = want / have;
                    Rect::from_min_max(pos2((1.0 - f) * 0.5, 0.0), pos2((1.0 + f) * 0.5, 1.0))
                } else {
                    let f = have / want;
                    Rect::from_min_max(pos2(0.0, (1.0 - f) * 0.5), pos2(1.0, (1.0 + f) * 0.5))
                }
            } else {
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
            };
            egui::Image::from_texture(egui::load::SizedTexture::new(h.id(), rect.size()))
                .uv(img)
                .corner_radius(radius)
                .paint_at(ui, rect);
        }
        None => {
            p.rect_filled(rect, radius, theme::THUMB_BODY);
            let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 9.0));
            p.rect_filled(
                bar,
                CornerRadius {
                    nw: m.thumb_radius,
                    ne: m.thumb_radius,
                    sw: 0,
                    se: 0,
                },
                theme::THUMB_TITLE_BAR,
            );
        }
    }
    p.rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0,
            if hovered {
                Color32::from_white_alpha(72)
            } else {
                Color32::from_white_alpha(30)
            },
        ),
        StrokeKind::Inside,
    );
    // The app badge, half outside the bottom-left corner, on a bar-coloured ring.
    let badge = m.badge;
    let b = Rect::from_min_size(
        pos2(rect.left() - 6.0, rect.bottom() + 6.0 - badge),
        vec2(badge, badge),
    );
    let ring = badge + 4.0;
    p.rect_filled(
        Rect::from_center_size(b.center(), vec2(ring, ring)),
        CornerRadius::same(((badge * 0.29).round() as u8) + 2),
        theme::BAR,
    );
    app_icon(
        ui,
        b,
        &w.app_name,
        icon,
        (badge * 0.29).round() as u8,
        (badge * 0.42).round(),
    );
}

/// The Desktop button's stops (DesktopDisplays.button): the label, dots and whether a tap steps.
#[derive(Clone, Debug, PartialEq)]
pub struct DesktopFace {
    pub label: String,
    pub dots: usize,
    pub lit: usize,
    pub counter: Option<String>,
    pub hint: String,
    pub value: Option<String>,
    pub steps: bool,
}

pub fn desktop_face(
    displays: Option<i64>,
    current: Option<i64>,
    names: &[String],
    streams: bool,
) -> DesktopFace {
    let today = DesktopFace {
        label: "Desktop".into(),
        dots: 0,
        lit: 0,
        counter: None,
        hint: "Show the whole desktop".into(),
        value: None,
        steps: false,
    };
    let (Some(n), Some(cur)) = (displays, current) else {
        return today;
    };
    if n <= 1 || cur < 1 || cur > n {
        return today;
    }
    let dots = if streams && n <= 6 { n as usize } else { 0 };
    let value = if names.len() as i64 == n && (1..=n).contains(&cur) {
        let name = names[(cur - 1) as usize].trim();
        if name.is_empty() {
            format!("Display {cur} of {n}")
        } else {
            format!("Display {cur} of {n}, {name}")
        }
    } else {
        format!("Display {cur} of {n}")
    };
    let hint = if streams {
        "Shows the next display".into()
    } else {
        "Show the whole desktop".into()
    };
    DesktopFace {
        label: format!("Desktop {cur}"),
        dots,
        lit: if dots > 0 { cur as usize } else { 0 },
        counter: (streams && n > 6).then(|| format!("{cur}/{n}")),
        hint,
        value: (n > 1).then_some(value),
        steps: streams,
    }
}

/// The Desktop button's dots or counter under its label (DesktopStopDots).
pub fn draw_desktop_stops(p: &egui::Painter, rect: Rect, face: &DesktopFace, dot: f32) {
    let y = rect.bottom() - 6.0;
    if let Some(c) = &face.counter {
        p.text(
            pos2(rect.center().x, y),
            Align2::CENTER_CENTER,
            c,
            FontId::proportional(dot + 3.0),
            theme::ACCENT,
        );
    } else if face.dots > 0 {
        let gap = dot * 0.75;
        let total = face.dots as f32 * dot + (face.dots - 1) as f32 * gap;
        let mut x = rect.center().x - total * 0.5 + dot * 0.5;
        for i in 1..=face.dots {
            p.circle_filled(
                pos2(x, y),
                dot * 0.5,
                if i == face.lit {
                    theme::ACCENT
                } else {
                    theme::ACCENT.gamma_multiply(0.32)
                },
            );
            x += dot + gap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_face_follows_desktopdisplays_button() {
        let f = desktop_face(Some(3), Some(2), &[], true);
        assert_eq!(f.label, "Desktop 2");
        assert_eq!((f.dots, f.lit), (3, 2));
        assert_eq!(f.counter, None);
        assert!(f.steps);
        assert_eq!(f.hint, "Shows the next display");
        assert_eq!(f.value.as_deref(), Some("Display 2 of 3"));

        let w = desktop_face(Some(3), Some(2), &[], false);
        assert_eq!(w.label, "Desktop 2");
        assert_eq!(w.dots, 0);
        assert!(!w.steps);
        assert_eq!(w.hint, "Show the whole desktop");

        assert_eq!(desktop_face(Some(1), Some(1), &[], true).label, "Desktop");
        assert_eq!(desktop_face(None, None, &[], true).label, "Desktop");
        assert_eq!(desktop_face(Some(3), Some(4), &[], true).label, "Desktop");
    }

    #[test]
    fn seven_stops_write_the_counter_and_names_stay_whole() {
        let names: Vec<String> = (1..=7).map(|i| format!("Display {i}")).collect();
        let f = desktop_face(Some(7), Some(3), &names, true);
        assert_eq!(f.counter.as_deref(), Some("3/7"));
        assert_eq!(f.dots, 0);
        assert_eq!(f.value.as_deref(), Some("Display 3 of 7, Display 3"));
        let f = desktop_face(Some(2), Some(2), &["A".into(), "B".into()], true);
        assert_eq!(f.value.as_deref(), Some("Display 2 of 2, B"));
    }

    #[test]
    fn an_unnamed_stop_only_counts() {
        let f = desktop_face(Some(2), Some(1), &["A".into()], true);
        assert_eq!(f.value.as_deref(), Some("Display 1 of 2"));
    }
}
