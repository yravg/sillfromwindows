//! Sill's palette and sizes as tokens (iOSClient/StreamScreen.swift, Palette; BarMetrics,
//! HostSettingsPanel), and egui's style from them. OWNER: the lead. Use these, never raw colors or
//! numbers, in the UI files.
use egui::Color32;

pub const BAR: Color32 = Color32::from_rgb(0x16, 0x18, 0x1C);
pub const CONTROL: Color32 = Color32::from_rgb(0x23, 0x26, 0x2C);
pub const CONTROL_OPEN: Color32 = Color32::from_rgb(0x2C, 0x35, 0x50);
pub const ACCENT: Color32 = Color32::from_rgb(0x9B, 0xB7, 0xFF);
pub const TEXT: Color32 = Color32::from_rgb(0xEE, 0xF0, 0xF3);
pub const BAR_LABEL: Color32 = Color32::from_rgb(0xC4, 0xC9, 0xD1);
pub const MUTED: Color32 = Color32::from_rgb(0xA4, 0xAB, 0xB5);
pub const PANEL: Color32 = Color32::from_rgb(0x1F, 0x21, 0x26);
pub const THUMB_BODY: Color32 = Color32::from_rgb(0x26, 0x28, 0x2D);
pub const THUMB_TITLE_BAR: Color32 = Color32::from_rgb(0x34, 0x37, 0x3D);
pub const ICON_FALLBACK: Color32 = Color32::from_rgb(0x3A, 0x3D, 0x44);
/// Problems and callouts: iOS's orange (the iOS app uses the system orange).
pub const WARN: Color32 = Color32::from_rgb(0xFF, 0x9F, 0x0A);
/// The old name of WARN, still used by the connect screen and the menus.
pub const PROBLEM: Color32 = WARN;
/// A segmented control's track and selected pill (iOS dark, as in the Settings screenshot).
pub const SEGMENT_TRACK: Color32 = Color32::from_rgb(0x2C, 0x2E, 0x33);
pub const SEGMENT_ON: Color32 = Color32::from_rgb(0x63, 0x66, 0x6C);
/// A switch's track while off; on uses ACCENT.
pub const SWITCH_OFF: Color32 = Color32::from_rgb(0x3A, 0x3D, 0x44);
/// Hairlines: row dividers, the bar's edge, the thumbnail's rim.
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 20);

/// Spacing and sizes, in points, from BarMetrics.regular and the Settings panel.
pub const GAP: f32 = 12.0;
pub const RADIUS: u8 = 12;
pub const MIN_TARGET: f32 = 44.0;
pub const TEXT_BODY: f32 = 15.0;
pub const TEXT_SMALL: f32 = 12.0;
pub const TEXT_ROW: f32 = 15.0;
pub const TEXT_GROUP: f32 = 13.0;
pub const TEXT_HEADING: f32 = 17.0;
pub const TEXT_BAR_LABEL: f32 = 11.0;
pub const ICON: f32 = 22.0;
pub const ROW_H: f32 = 44.0;
pub const SETTINGS_W: f32 = 360.0;
pub const DRAWER_W: f32 = 380.0;
pub const PANEL_RADIUS: u8 = 20;

/// The top bar's numbers: BarMetrics.regular for a desktop, compact when the window is narrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarMetrics {
    pub height: f32,
    pub padding: f32,
    pub gap: f32,
    pub button_w: f32,
    pub button_h: f32,
    pub button_spacing: f32,
    pub thumb_w: f32,
    pub thumb_h: f32,
    pub thumb_radius: u8,
    pub thumb_spacing: f32,
    pub thumb_pad: f32,
    pub badge: f32,
    pub settings_w: f32,
    pub drawer_w: f32,
}

impl BarMetrics {
    pub const fn regular() -> Self {
        BarMetrics {
            height: 86.0,
            padding: 22.0,
            gap: 12.0,
            button_w: 66.0,
            button_h: 66.0,
            button_spacing: 4.0,
            thumb_w: 104.0,
            thumb_h: 66.0,
            thumb_radius: 10,
            thumb_spacing: 16.0,
            thumb_pad: 10.0,
            badge: 24.0,
            settings_w: SETTINGS_W,
            drawer_w: DRAWER_W,
        }
    }

    pub const fn compact() -> Self {
        BarMetrics {
            height: 78.0,
            padding: 14.0,
            gap: 12.0,
            button_w: 64.0,
            button_h: 58.0,
            button_spacing: 3.0,
            thumb_w: 92.0,
            thumb_h: 58.0,
            thumb_radius: 9,
            thumb_spacing: 14.0,
            thumb_pad: 10.0,
            badge: 22.0,
            settings_w: 340.0,
            drawer_w: 340.0,
        }
    }

    /// Regular from the width the iPad's regular bar has room for; compact below it, as the iOS
    /// compact bars do on the outer display.
    pub fn for_width(width: f32) -> Self {
        if width < 1_000.0 {
            BarMetrics::compact()
        } else {
            BarMetrics::regular()
        }
    }
}

impl Default for BarMetrics {
    fn default() -> Self {
        BarMetrics::regular()
    }
}

pub fn apply(ctx: &egui::Context) {
    add_fonts(ctx);
    // Sill is dark-only, as the iOS stream screen (Palette): one theme, whatever Windows says.
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        v.panel_fill = BAR;
        v.window_fill = BAR;
        v.extreme_bg_color = CONTROL;
        v.faint_bg_color = PANEL;
        v.override_text_color = Some(TEXT);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = CONTROL_OPEN;
        v.selection.stroke.color = ACCENT;
        v.window_stroke = egui::Stroke::new(1.0, HAIRLINE);
        v.window_corner_radius = egui::CornerRadius::same(RADIUS);
        v.menu_corner_radius = egui::CornerRadius::same(RADIUS);
        v.widgets.inactive.weak_bg_fill = CONTROL;
        v.widgets.inactive.bg_fill = CONTROL;
        // Unchecked boxes and fields need an edge on PANEL (MUTED at 1 pt).
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, MUTED);
        v.widgets.hovered.weak_bg_fill = CONTROL_OPEN;
        v.widgets.hovered.bg_fill = CONTROL_OPEN;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, HAIRLINE);
        v.widgets.active.weak_bg_fill = CONTROL_OPEN;
        v.widgets.active.bg_fill = CONTROL_OPEN;
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, HAIRLINE);
        // The Settings footnotes and group titles are indented for their 14 pt inset, not a tree.
        v.indent_has_left_vline = false;
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.interact_size.y = MIN_TARGET;
        s.spacing.button_padding = egui::vec2(12.0, 8.0);
        s.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(TEXT_HEADING, egui::FontFamily::Proportional),
        );
        s.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(TEXT_BODY, egui::FontFamily::Proportional),
        );
        s.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(TEXT_BODY, egui::FontFamily::Proportional),
        );
        s.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(TEXT_SMALL, egui::FontFamily::Proportional),
        );
        s.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(12.0, egui::FontFamily::Monospace),
        );
    });
}

/// Segoe UI as the UI face, as the iPad uses SF Pro; the Mac's key symbols and arrows from Segoe UI
/// Symbol. Read at run time and never shipped. Linux falls back to DejaVu, as before.
fn add_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, path: &str, family: egui::FontFamily, front: bool| {
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        let list = fonts.families.entry(family).or_default();
        if front {
            list.insert(0, name.into());
        } else {
            list.push(name.into());
        }
    };
    if cfg!(windows) {
        add(
            "ui",
            r"C:\Windows\Fonts\segoeui.ttf",
            egui::FontFamily::Proportional,
            true,
        );
        add(
            "mono",
            r"C:\Windows\Fonts\consola.ttf",
            egui::FontFamily::Monospace,
            true,
        );
        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
            fonts.font_data.insert(
                "symbols".into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("symbols".into());
            }
        }
    } else if let Ok(bytes) = std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf") {
        fonts.font_data.insert(
            "symbols".into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("symbols".into());
        }
    }
    ctx.set_fonts(fonts);
}
