//! Sill's palette as tokens (iOSClient/StreamScreen.swift:19-30, Palette), and egui's style from
//! them. OWNER: the lead. Use these, never raw colors, in the UI files.
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
/// Problems (refusals, failures). Not in the iOS palette (iOS uses the system's red).
pub const PROBLEM: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x80);

/// Spacing and sizes, in points.
pub const GAP: f32 = 8.0;
pub const BAR_HEIGHT: f32 = 56.0;
pub const RADIUS: u8 = 10;
pub const MIN_TARGET: f32 = 32.0;

pub fn apply(ctx: &egui::Context) {
    // Sill is dark-only, as the iOS stream screen (Palette): one theme, whatever Windows says.
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        v.panel_fill = PANEL;
        v.window_fill = BAR;
        v.extreme_bg_color = BAR;
        v.override_text_color = Some(TEXT);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = CONTROL_OPEN;
        v.selection.stroke.color = ACCENT;
        v.widgets.inactive.weak_bg_fill = CONTROL;
        v.widgets.inactive.bg_fill = CONTROL;
        v.widgets.hovered.weak_bg_fill = CONTROL_OPEN;
        v.widgets.active.weak_bg_fill = CONTROL_OPEN;
        s.spacing.item_spacing = egui::vec2(GAP, GAP);
        s.spacing.interact_size.y = MIN_TARGET;
    });
}
