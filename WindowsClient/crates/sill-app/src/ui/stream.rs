//! OWNER: worker U2. The stream screen: video, bar, thumbnails, Apps drawer, the Mac's pointer.
//! iOSClient/StreamScreen.swift (bar 1413-1520, thumbnails 1765-2135, drawer 2169-2326, locked
//! cover 1337-1380, link line 1290-1336), HEVCDisplayView.swift (pointer sprite).
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, RichText,
    Role, Sense, Stroke, StrokeKind, TextureHandle, TextureId, TextureOptions, WidgetInfo,
};
use egui_wgpu::wgpu;
use sill_proto::switcher::{StreamSource, WindowAction, WindowInfo};

use super::menus::{self, MenusUi};
use super::settings::SettingsUi;

use crate::input::{self, Cfg, InputState};
use crate::model::{AppModel, Command, Cover, Picture, Screen, VideoFrame, VideoSlot};
use crate::theme;

const DRAWER_WIDTH: f32 = 320.0;
const SETTINGS_WIDTH: f32 = 360.0;
/// Aa's detents (TextScale.swift): Off, then 0.5× to 1.5× in quarter steps.
const SCALES: [f64; 5] = [0.5, 0.75, 1.0, 1.25, 1.5];
const BTN_W: f32 = 84.0;
const THUMB_H: f32 = 40.0;
const ROW_H: f32 = 44.0;
/// The panel's size must be steady this long before a viewport goes out (StreamScreen.swift sends
/// it "after layout settles, 0.25 s").
const VIEWPORT_SETTLE: f64 = 0.25;
/// The picture's letterbox is the picture's own black, not chrome.
const LETTERBOX: Color32 = Color32::BLACK;

struct VideoTex {
    texture: wgpu::Texture,
    id: TextureId,
    width: u32,
    height: u32,
    serial: u64,
}

pub struct StreamUi {
    rs: Option<egui_wgpu::RenderState>,
    video: Option<VideoTex>,
    textures: HashMap<String, (u64, TextureHandle)>,
    drawer: bool,
    settings: bool,
    search: String,
    /// Thumbnail order, kept per session (B11); windows not in it follow in the host's order.
    order: Vec<u32>,
    input: InputState,
    last_pass: Option<u64>,
    vp_sent: Option<Size2>,
    vp_candidate: Option<(Size2, f64)>,
    settings_ui: SettingsUi,
    menus: MenusUi,
    /// Aa's scale (None = Off); kept across windows (TextScale: the value stays for the next).
    aa: Option<f64>,
    /// TEST ONLY: SILL_TEST_STREAM_OPEN=settings,drawer,menu,aa opens those at start.
    test_open: Vec<String>,
    drawer_focus: bool,
    /// The picture's size in points, as last laid out.
    panel: Size2,
}

type Size2 = (f32, f32);

impl StreamUi {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // TEST ONLY: the drawer case opens the drawer, since a mock has no one to click Apps.
        let mut test_open: Vec<String> = std::env::var("SILL_TEST_STREAM_OPEN")
            .unwrap_or_default()
            .split(',')
            .map(str::to_string)
            .collect();
        if std::env::var("SILL_TEST_MOCK").is_ok_and(|c| c == "stream-drawer") {
            test_open.push("drawer".into());
        }
        let drawer = test_open.iter().any(|o| o == "drawer");
        StreamUi {
            rs: cc.wgpu_render_state.clone(),
            video: None,
            textures: HashMap::new(),
            drawer,
            settings: test_open.iter().any(|o| o == "settings"),
            search: String::new(),
            order: Vec::new(),
            input: InputState::default(),
            last_pass: None,
            vp_sent: None,
            vp_candidate: None,
            settings_ui: SettingsUi::default(),
            menus: MenusUi::default(),
            aa: None,
            drawer_focus: drawer,
            panel: (0.0, 0.0),
            test_open,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        _frame: &mut eframe::Frame,
        model: &AppModel,
        tx: &Sender<Command>,
        video: &VideoSlot,
    ) {
        let ctx = ui.ctx().clone();
        let pass = ctx.cumulative_pass_nr();
        if self.last_pass.is_some_and(|p| pass != p && pass != p + 1) {
            // The screen was left and is back: a new session starts with nothing held.
            self.input.reset();
        }
        self.last_pass = Some(pass);

        self.upload(video);
        let thumbs = self.prepare_textures(&ctx, model);
        self.sync_order(&model.stream.windows);

        egui::Panel::top("stream-bar")
            .exact_size(theme::BAR_HEIGHT)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::BAR)
                    .inner_margin(egui::Margin::symmetric(8, 8)),
            )
            .show(ui, |ui| self.bar(ui, model, tx, &thumbs));

        // Below the bar: the picture, and over it (not beside it, so the viewport the Mac is told
        // never changes) the Apps drawer and the Settings panel, as on iOS.
        let rest = ui.available_rect_before_wrap();
        self.overlays(&ctx, rest, model, tx);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(LETTERBOX))
            .show(ui, |ui| self.picture(ui, model, tx));
    }

    fn overlays(
        &mut self,
        ctx: &egui::Context,
        rest: Rect,
        model: &AppModel,
        tx: &Sender<Command>,
    ) {
        if self.drawer {
            // The dimmed backdrop: a click on it closes the drawer and never reaches the Mac.
            egui::Area::new(Id::unique("drawer-backdrop"))
                .order(egui::Order::Middle)
                .fixed_pos(rest.min)
                .show(ctx, |ui| {
                    let (r, resp) = ui.allocate_exact_size(rest.size(), Sense::click());
                    ui.painter()
                        .rect_filled(r, 0, Color32::from_black_alpha(110));
                    if resp.clicked() {
                        self.drawer = false;
                    }
                });
            egui::Area::new(Id::unique("drawer"))
                .order(egui::Order::Foreground)
                .fixed_pos(rest.min)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(DRAWER_WIDTH - 24.0);
                            ui.set_height(rest.height() - 24.0);
                            self.drawer_ui(ui, model, tx);
                        });
                });
        }
        if self.settings {
            let mut keep = true;
            egui::Area::new(Id::unique("settings"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(rest.right() - SETTINGS_WIDTH, rest.top()))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(SETTINGS_WIDTH - 24.0);
                            ui.set_height(rest.height() - 24.0);
                            keep = self.settings_ui.show(ui, model, tx);
                        });
                });
            if !keep {
                self.settings = false;
            }
        }
    }

    // ---- video ---------------------------------------------------------------------------

    /// Copies the newest decoded frame to the GPU texture, only when its serial changed.
    fn upload(&mut self, video: &VideoSlot) {
        let Some(rs) = &self.rs else { return };
        let Ok(guard) = video.lock() else { return };
        let Some(f) = guard.as_ref() else { return };
        if f.width == 0 || f.height == 0 {
            return;
        }
        if let Some(t) = &self.video {
            if t.serial == f.serial && t.width == f.width && t.height == f.height {
                return;
            }
        }
        let recreate = !self
            .video
            .as_ref()
            .is_some_and(|t| t.width == f.width && t.height == f.height);
        if recreate {
            if let Some(old) = self.video.take() {
                rs.renderer.write().free_texture(&old.id);
                old.texture.destroy();
            }
            let texture = rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("sill-video"),
                size: wgpu::Extent3d {
                    width: f.width,
                    height: f.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // The decoder gives sRGB-encoded BGRA; the Srgb view makes egui's shader decode
                // it as it does its own textures, so colors match.
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let id = rs.renderer.write().register_native_texture(
                &rs.device,
                &view,
                wgpu::FilterMode::Linear,
            );
            self.video = Some(VideoTex {
                texture,
                id,
                width: f.width,
                height: f.height,
                serial: 0,
            });
        }
        let Some(t) = self.video.as_mut() else { return };
        write_frame(&rs.queue, &t.texture, f);
        t.serial = f.serial;
    }

    fn picture(&mut self, ui: &mut egui::Ui, model: &AppModel, tx: &Sender<Command>) {
        let ctx = ui.ctx().clone();
        let panel = ui.max_rect();
        let now = ctx.input(|i| i.time);
        let (fw, fh) = self.video.as_ref().map_or((0, 0), |t| (t.width, t.height));
        let vrect = input::video_rect(panel, fw, fh);

        self.viewport(&ctx, panel, now, tx);

        // The picture is one focusable surface: it keeps the keyboard unless a text field has it.
        let id = Id::unique("sill-stream-picture");
        let resp = ui.interact(panel, id, Sense::click_and_drag());
        resp.widget_info(|| WidgetInfo::labeled(Role::Image, true, "Mac screen"));
        // A text field (the drawer's search, the Settings panel's) keeps its keys; otherwise the
        // picture takes the keyboard back, also after a bar button was clicked.
        let typing = ctx.text_edit_focused();
        if !typing {
            ctx.memory_mut(|m| {
                m.request_focus(id);
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
        }
        let keys_active = !typing && ctx.memory(|m| m.has_focus(id));

        let hover = ctx.pointer_hover_pos();
        let pointer_over = hover.is_some_and(|p| {
            vrect.contains(p) && ctx.layer_id_at(p) == Some(ui.layer_id()) && panel.contains(p)
        });

        let events = ctx.input(|i| i.events.clone());
        let mods = ctx.input(|i| i.modifiers);
        let out = self.input.process(&input::Frame {
            events: &events,
            mods,
            now,
            video: vrect,
            pointer_over,
            keys_active,
            cfg: Cfg {
                key_repeat: model.stream.mac_key_repeat,
                windows_key_is_command: model.prefs.windows_key_is_command,
            },
        });
        for e in out {
            let _ = tx.send(Command::Input(e));
        }
        if self.input.gesture_open() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if keys_active {
            // Lets Windows' IME compose over the picture; the committed text is sent as text.
            let cursor = Rect::from_min_size(vrect.left_bottom(), vec2(1.0, 16.0));
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect: panel,
                    cursor_rect: cursor,
                    should_interrupt_composition: false,
                });
            });
        }

        let painter = ui.painter_at(panel);
        painter.rect_filled(panel, 0, LETTERBOX);
        if let Some(t) = &self.video {
            painter.image(
                t.id,
                vrect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // The Mac's pointer. This PC's own pointer is hidden over the picture and the Mac's shape
        // is drawn at it instead (egui cannot swap the OS cursor's image), so what shows is the
        // Mac's cursor; while the Mac moves it (kind 26) it is drawn where the Mac says.
        let c = &model.stream.cursor;
        let at = if let Some((fx, fy)) = c.mac_position.filter(|_| !pointer_over) {
            Some(pos2(
                vrect.left() + fx as f32 * vrect.width(),
                vrect.top() + fy as f32 * vrect.height(),
            ))
        } else if pointer_over {
            ctx.set_cursor_icon(egui::CursorIcon::None);
            hover
        } else {
            None
        };
        if let (Some(p), true) = (at, vrect.width() > 0.0) {
            let tex = c.shape.as_ref().and_then(|pic| {
                self.textures
                    .get("cursor")
                    .filter(|(g, _)| *g == pic.generation)
            });
            match (tex, &c.shape) {
                (Some((_, h)), Some(_)) => {
                    let min = p - vec2(c.hotspot.0, c.hotspot.1);
                    let r = Rect::from_min_size(min, vec2(c.size.0, c.size.1));
                    painter.image(
                        h.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                _ => draw_arrow(&painter, p),
            }
        }

        // The cover (S06): only while another user is on the Mac, then "Unlocking…". A Mac whose
        // own screen is locked has none: its lock screen streams and the password is typed here.
        if let Some(cover) = &model.stream.cover {
            painter.rect_filled(panel, 0, Color32::from_black_alpha(190));
            let mid = panel.center();
            let h = painter.text(
                mid - vec2(0.0, if cover.detail.is_some() { 14.0 } else { 0.0 }),
                Align2::CENTER_CENTER,
                &cover.title,
                FontId::proportional(20.0),
                theme::TEXT,
            );
            if let Some(d) = &cover.detail {
                painter.text(
                    pos2(mid.x, h.bottom() + 16.0),
                    Align2::CENTER_CENTER,
                    d,
                    FontId::proportional(14.0),
                    theme::MUTED,
                );
            }
        }
        if let Some(line) = &model.stream.link_line {
            capsule(
                &painter,
                Align2::CENTER_BOTTOM,
                panel.center_bottom() - vec2(0.0, 16.0),
                line,
                theme::TEXT,
            );
        }
        if let Some(line) = &model.stream.reconnecting {
            capsule(
                &painter,
                Align2::CENTER_TOP,
                panel.center_top() + vec2(0.0, 16.0),
                line,
                theme::TEXT,
            );
        }
        if self.video.is_none()
            && model.stream.cover.is_none()
            && model.stream.reconnecting.is_none()
        {
            painter.text(
                panel.center(),
                Align2::CENTER_CENTER,
                "Waiting for the first picture…",
                FontId::proportional(14.0),
                theme::MUTED,
            );
        }
    }

    fn viewport(&mut self, ctx: &egui::Context, panel: Rect, now: f64, tx: &Sender<Command>) {
        let size = (panel.width(), panel.height());
        self.panel = size;
        if size.0 < 16.0 || size.1 < 16.0 {
            return;
        }
        let same = |a: Size2, b: Size2| (a.0 - b.0).abs() < 1.0 && (a.1 - b.1).abs() < 1.0;
        match self.vp_candidate {
            Some((c, _)) if same(c, size) => {}
            _ => self.vp_candidate = Some((size, now)),
        }
        let Some((cand, since)) = self.vp_candidate else {
            return;
        };
        if self.vp_sent.is_some_and(|s| same(s, cand)) {
            return;
        }
        if self.vp_sent.is_none() || now - since >= VIEWPORT_SETTLE {
            self.vp_sent = Some(cand);
            let _ = tx.send(Command::Viewport {
                width: cand.0 as f64,
                height: cand.1 as f64,
                scale: self.aa,
            });
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    // ---- textures ------------------------------------------------------------------------

    fn picture_texture(
        &mut self,
        ctx: &egui::Context,
        key: String,
        pic: &Picture,
    ) -> TextureHandle {
        if let Some((g, h)) = self.textures.get(&key) {
            if *g == pic.generation {
                return h.clone();
            }
        }
        let img = egui::ColorImage::from_rgba_unmultiplied(
            [pic.width as usize, pic.height as usize],
            &pic.rgba,
        );
        let h = ctx.load_texture(key.clone(), img, TextureOptions::LINEAR);
        self.textures.insert(key, (pic.generation, h.clone()));
        h
    }

    /// Thumbnails by window id and icons by bundle ID, as textures; also keeps the cursor's.
    fn prepare_textures(&mut self, ctx: &egui::Context, model: &AppModel) -> Textures {
        let mut t = Textures::default();
        let mut keep = vec!["cursor".to_string()];
        for (id, pic) in &model.stream.thumbnails {
            let k = format!("t:{id}");
            t.thumbs
                .insert(*id, self.picture_texture(ctx, k.clone(), pic));
            keep.push(k);
        }
        for (b, pic) in &model.stream.icons {
            let k = format!("i:{b}");
            t.icons
                .insert(b.clone(), self.picture_texture(ctx, k.clone(), pic));
            keep.push(k);
        }
        if let Some(pic) = &model.stream.cursor.shape {
            let _ = self.picture_texture(ctx, "cursor".into(), pic);
        }
        self.textures.retain(|k, _| keep.contains(k));
        t
    }

    fn sync_order(&mut self, windows: &[WindowInfo]) {
        self.order.retain(|id| windows.iter().any(|w| w.id == *id));
        for w in windows {
            if !self.order.contains(&w.id) {
                self.order.push(w.id);
            }
        }
    }

    // ---- bar -----------------------------------------------------------------------------

    fn bar(&mut self, ui: &mut egui::Ui, model: &AppModel, tx: &Sender<Command>, tex: &Textures) {
        let s = &model.stream;
        let has_menus = menus::available(model);
        let right = 3 + usize::from(has_menus) + usize::from(s.mac_gestures);
        ui.horizontal_centered(|ui| {
            if bar_button(ui, "Apps", self.drawer, "Open or close the app list", true).clicked() {
                self.drawer = !self.drawer;
                self.drawer_focus = self.drawer;
            }
            let strip_w = (ui.available_width() - right as f32 * (BTN_W + theme::GAP)).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(strip_w, THUMB_H),
                Layout::left_to_right(Align::Center),
                |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("thumbs")
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| self.thumbnails(ui, model, tx, tex))
                        });
                },
            );
            if has_menus {
                let r = bar_button(ui, "Menus", false, &format!("{}’s menus", s.mac_name), true);
                let file = self.test_open.iter().any(|o| o == "menufile");
                self.popup(ui, &r, "menu", |this, ui| {
                    if file {
                        this.menus.submenu(ui, "1", "File", model, tx);
                    } else {
                        this.menus.tree(ui, model, tx);
                    }
                });
            }
            if s.mac_gestures {
                let r = bar_button(
                    ui,
                    "Mac",
                    false,
                    "Mac actions: Mission Control, Spaces…",
                    true,
                );
                self.popup(ui, &r, "mac", |_, ui| gesture_menu(ui, tx));
            }
            self.aa_button(ui, model, tx);
            let desktop = matches!(s.active, Some(StreamSource::Desktop));
            if bar_button(ui, "Desktop", desktop, "Show the whole desktop", true).clicked() {
                let _ = tx.send(Command::Pick(StreamSource::Desktop));
            }
            let hint = format!("Settings for {}", s.mac_name);
            if bar_button(ui, "Settings", self.settings, &hint, true).clicked() {
                self.settings = !self.settings;
            }
        });
    }

    /// A menu popup under a bar button; TEST ONLY: opened at start when SILL_TEST_STREAM_OPEN says.
    fn popup(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        name: &str,
        content: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let mut p = egui::Popup::menu(resp);
        if let Some(at) = self
            .test_open
            .iter()
            .position(|o| o == name || (name == "menu" && o == "menufile"))
        {
            if ui.ctx().cumulative_pass_nr() < 4 {
                p = p.open_memory(Some(egui::SetOpenCommand::Bool(true)));
            } else if self.test_open[at] == name {
                self.test_open.remove(at);
            }
        }
        p.show(|ui| content(self, ui));
    }

    /// Aa (B04): usable only while a window streams; the value stays for the next window.
    fn aa_button(&mut self, ui: &mut egui::Ui, model: &AppModel, tx: &Sender<Command>) {
        let usable = aa_selectable(model.stream.active);
        let offered = aa_offers_off(model);
        let label = format!("Aa {}", aa_title(self.aa, offered));
        let hint = if usable {
            "Text size"
        } else {
            "Text size, available for a window"
        };
        let r = bar_button(ui, &label, false, hint, usable);
        if !usable {
            return;
        }
        let mut chosen: Option<Option<f64>> = None;
        let cur = self.aa;
        self.popup(ui, &r, "aa", |_, ui| {
            for d in aa_detents(offered) {
                if ui
                    .selectable_label(cur == d, aa_title(d, offered))
                    .clicked()
                {
                    chosen = Some(d);
                    ui.close();
                }
            }
        });
        if let Some(d) = chosen {
            self.aa = d;
            let (w, h) = self.panel;
            if w > 0.0 {
                self.vp_sent = Some((w, h));
                let _ = tx.send(Command::Viewport {
                    width: w as f64,
                    height: h as f64,
                    scale: d,
                });
            }
        }
    }

    fn thumbnails(
        &mut self,
        ui: &mut egui::Ui,
        model: &AppModel,
        tx: &Sender<Command>,
        tex: &Textures,
    ) {
        let s = &model.stream;
        let mut rects: Vec<(u32, Rect)> = Vec::new();
        let mut dropped: Option<(u32, Pos2)> = None;
        let order = self.order.clone();
        for id in order {
            let Some(w) = s.windows.iter().find(|w| w.id == id) else {
                continue;
            };
            let aspect = if w.height > 0 {
                w.width as f32 / w.height as f32
            } else {
                1.6
            };
            let width = (THUMB_H * aspect).clamp(48.0, 96.0);
            let (rect, resp) =
                ui.allocate_exact_size(vec2(width, THUMB_H), Sense::click_and_drag());
            rects.push((id, rect));
            let active = matches!(s.active, Some(StreamSource::Window(a)) if a == id);
            let label = format!("{}, {}", w.title, w.app_name);
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
            draw_thumbnail(
                ui,
                rect,
                w,
                tex.thumbs.get(&id),
                tex.icons.get(&w.bundle_id),
                active,
                resp.hovered() || resp.dragged(),
            );
            let resp = resp.on_hover_text(format!("{} — {}", w.title, w.app_name));
            if resp.clicked() {
                let _ = tx.send(Command::Pick(StreamSource::Window(id)));
            }
            if resp.drag_stopped() {
                if let Some(p) = ui.ctx().pointer_interact_pos() {
                    dropped = Some((id, p));
                }
            }
            // B10: the Mac's three lights, from a right click.
            resp.context_menu(|ui| {
                for (name, action) in [
                    ("Close", WindowAction::Close),
                    ("Minimize", WindowAction::Minimize),
                    ("Full Screen", WindowAction::FullScreen),
                ] {
                    if ui.button(name).clicked() {
                        let _ = tx.send(Command::WindowCommand {
                            window_id: id,
                            action,
                        });
                        ui.close();
                    }
                }
            });
        }
        if let Some((id, p)) = dropped {
            if let Some(from) = self.order.iter().position(|x| *x == id) {
                let target = rects
                    .iter()
                    .filter(|(i, _)| *i != id)
                    .position(|(_, r)| p.x < r.center().x)
                    .unwrap_or(rects.len().saturating_sub(1));
                let item = self.order.remove(from);
                self.order.insert(target.min(self.order.len()), item);
            }
        }
    }

    // ---- drawer --------------------------------------------------------------------------

    fn drawer_ui(&mut self, ui: &mut egui::Ui, model: &AppModel, tx: &Sender<Command>) {
        let s = &model.stream;
        let search = egui::TextEdit::singleline(&mut self.search)
            .id(Id::unique("drawer-search"))
            .hint_text(format!("Search apps on {}", s.mac_name))
            .desired_width(f32::INFINITY)
            .margin(vec2(10.0, 8.0));
        let field = ui.add(search);
        if std::mem::take(&mut self.drawer_focus) {
            field.request_focus();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drawer = false;
        }
        ui.add_space(theme::GAP);

        if let Some(words) = &s.locked {
            ui.label(RichText::new(words).size(13.0).color(theme::MUTED));
            return;
        }
        let q = self.search.trim().to_lowercase();
        let hit = |name: &str| q.is_empty() || name.to_lowercase().contains(&q);

        // Open now: apps with windows, in the host's order.
        let mut open: Vec<(String, String, Vec<u32>)> = Vec::new();
        for w in &s.windows {
            match open.iter_mut().find(|(b, _, _)| *b == w.bundle_id) {
                Some((_, _, ids)) => ids.push(w.id),
                None => open.push((w.bundle_id.clone(), w.app_name.clone(), vec![w.id])),
            }
        }
        let active_id = match s.active {
            Some(StreamSource::Window(i)) => Some(i),
            _ => None,
        };
        let icons: HashMap<String, TextureHandle> = self
            .textures
            .iter()
            .filter_map(|(k, (_, h))| k.strip_prefix("i:").map(|b| (b.to_string(), h.clone())))
            .collect();
        let mut picked: Option<Command> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let open: Vec<_> = open.into_iter().filter(|(_, n, _)| hit(n)).collect();
                if !open.is_empty() {
                    section(ui, "Open now");
                    for (bundle, name, ids) in &open {
                        let active = active_id.is_some_and(|a| ids.contains(&a));
                        let sub = if ids.len() == 1 {
                            "1 window".to_string()
                        } else {
                            format!("{} windows", ids.len())
                        };
                        if app_row(ui, name, Some(&sub), icons.get(bundle), active).clicked() {
                            // Another tap on the app shows its next window (StreamScreen.swift 2297).
                            let next =
                                match active_id.and_then(|a| ids.iter().position(|i| *i == a)) {
                                    Some(p) => ids[(p + 1) % ids.len()],
                                    None => ids[0],
                                };
                            picked = Some(Command::Pick(StreamSource::Window(next)));
                        }
                    }
                    ui.add_space(theme::GAP);
                }
                let mut all: Vec<_> = s.apps.iter().filter(|a| hit(&a.name)).collect();
                all.sort_by_key(|a| a.name.to_lowercase());
                if !all.is_empty() {
                    section(ui, "All apps");
                    for a in all {
                        if app_row(ui, &a.name, None, icons.get(&a.bundle_id), false).clicked() {
                            picked = Some(Command::Launch(a.bundle_id.clone()));
                        }
                    }
                }
                if !q.is_empty()
                    && !s.apps.iter().any(|a| hit(&a.name))
                    && !s.windows.iter().any(|w| hit(&w.app_name))
                {
                    ui.label(
                        RichText::new("No apps match.")
                            .size(13.0)
                            .color(theme::MUTED),
                    );
                }
            });
        if let Some(c) = picked {
            let _ = tx.send(c);
            self.drawer = false;
        }
    }
}

#[derive(Default)]
struct Textures {
    thumbs: HashMap<u32, TextureHandle>,
    icons: HashMap<String, TextureHandle>,
}

fn write_frame(queue: &wgpu::Queue, texture: &wgpu::Texture, f: &VideoFrame) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &f.bgra,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(f.stride),
            rows_per_image: Some(f.height),
        },
        wgpu::Extent3d {
            width: f.width,
            height: f.height,
            depth_or_array_layers: 1,
        },
    );
}

// ---- Aa and the Mac's gestures (pure) ------------------------------------------------------

/// TextScale.selectable: only a window can be sized.
fn aa_selectable(active: Option<StreamSource>) -> bool {
    matches!(active, Some(StreamSource::Window(_)))
}

/// TextScale.offersOff: no Off while the running window streams on the Mac's virtual display (no
/// scale is 1× there); else not while the setting is on; offered before the Mac has said.
fn aa_offers_off(model: &AppModel) -> bool {
    let s = model.stream.settings.as_ref();
    if let (Some(StreamSource::Window(_)), Some(run)) =
        (model.stream.active, s.and_then(|s| s.stream.as_ref()))
    {
        return !run.on_virtual_display;
    }
    s.is_none_or(|s| !s.settings.virtual_display)
}

/// The ruler's detents left to right, as scales (None = Off).
fn aa_detents(offered: bool) -> Vec<Option<f64>> {
    let mut v: Vec<Option<f64>> = Vec::new();
    if offered {
        v.push(None);
    }
    v.extend(SCALES.iter().map(|s| Some(*s)));
    v
}

/// "Off", or the scale with trailing zeros dropped and "×": 0.5×, 1×, 1.25× (TextScale.title).
/// An untouched Aa without Off reads 1×.
fn aa_title(scale: Option<f64>, offered: bool) -> String {
    match scale {
        None if offered => "Off".into(),
        None => "1×".into(),
        Some(s) => {
            let mut t = format!("{s:.2}");
            while t.ends_with('0') {
                t.pop();
            }
            if t.ends_with('.') {
                t.pop();
            }
            format!("{t}×")
        }
    }
}

/// The Mac menu's actions, by the gesture name GestureChords.swift:133-138 maps to each:
/// swipeUp Mission Control, swipeDown App Exposé, swipeLeft next Space, swipeRight previous Space,
/// spread Show Desktop, pinch Apps (Launchpad).
const GESTURES: [(&str, &str); 6] = [
    ("Mission Control", "swipeUp"),
    ("App Exposé", "swipeDown"),
    ("Next Space", "swipeLeft"),
    ("Previous Space", "swipeRight"),
    ("Show Desktop", "spread"),
    ("Launchpad", "pinch"),
];

fn gesture_menu(ui: &mut egui::Ui, tx: &Sender<Command>) {
    ui.set_min_width(170.0);
    for (name, g) in GESTURES {
        if ui.button(name).clicked() {
            let _ = tx.send(Command::Gesture(g.to_string()));
            ui.close();
        }
    }
}

// ---- small drawing helpers ---------------------------------------------------------------

fn bar_button(
    ui: &mut egui::Ui,
    label: &str,
    open: bool,
    hint: &str,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(BTN_W, THUMB_H), sense);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, hint));
    let fill = if open || (enabled && resp.hovered()) {
        theme::CONTROL_OPEN
    } else {
        theme::CONTROL
    };
    // Dimmed to 35 % when unusable, as Aa is over the Desktop (StreamScreen.swift).
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.35) };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS), fade(fill));
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::RADIUS),
            Stroke::new(2.0, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
    let color = fade(if open {
        theme::ACCENT
    } else {
        theme::BAR_LABEL
    });
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        color,
    );
    resp.on_hover_text(hint)
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(12.0).color(theme::MUTED).strong());
}

fn app_row(
    ui: &mut egui::Ui,
    name: &str,
    sub: Option<&str>,
    icon: Option<&TextureHandle>,
    active: bool,
) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));
    let fill = if active {
        theme::CONTROL_OPEN
    } else if resp.hovered() {
        theme::CONTROL
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    let ir = Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), vec2(28.0, 28.0));
    draw_icon(ui.painter(), ir, name, icon);
    let x = rect.left() + 52.0;
    let (ny, color) = match sub {
        Some(sub) => {
            ui.painter().text(
                pos2(x, rect.center().y + 9.0),
                Align2::LEFT_CENTER,
                sub,
                FontId::proportional(11.0),
                theme::MUTED,
            );
            (rect.center().y - 7.0, theme::TEXT)
        }
        None => (rect.center().y, theme::TEXT),
    };
    ui.painter().text(
        pos2(x, ny),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(14.0),
        color,
    );
    resp
}

/// App icon, or the two-letter badge (D05).
fn draw_icon(p: &egui::Painter, r: Rect, name: &str, icon: Option<&TextureHandle>) {
    match icon {
        Some(h) => {
            p.image(
                h.id(),
                r,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            p.rect_filled(r, CornerRadius::same(7), theme::ICON_FALLBACK);
            let badge: String = name
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .collect::<String>()
                .to_uppercase();
            p.text(
                r.center(),
                Align2::CENTER_CENTER,
                badge,
                FontId::proportional(11.0),
                theme::BAR_LABEL,
            );
        }
    }
}

fn draw_thumbnail(
    ui: &egui::Ui,
    rect: Rect,
    w: &WindowInfo,
    thumb: Option<&TextureHandle>,
    icon: Option<&TextureHandle>,
    active: bool,
    lit: bool,
) {
    let p = ui.painter();
    let r = CornerRadius::same(6);
    match thumb {
        Some(h) => {
            p.rect_filled(rect, r, theme::THUMB_BODY);
            p.image(
                h.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            // A placeholder drawn as a window: title bar and body.
            p.rect_filled(rect, r, theme::THUMB_BODY);
            let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 9.0));
            p.rect_filled(
                bar,
                CornerRadius {
                    nw: 6,
                    ne: 6,
                    sw: 0,
                    se: 0,
                },
                theme::THUMB_TITLE_BAR,
            );
        }
    }
    // The app's badge in the corner, as the iOS strip shows it.
    let badge = Rect::from_min_size(rect.right_bottom() - vec2(16.0, 16.0), vec2(14.0, 14.0));
    draw_icon(p, badge, &w.app_name, icon);
    if active {
        p.rect_stroke(rect, r, Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
    } else if lit {
        p.rect_stroke(rect, r, Stroke::new(1.0, theme::MUTED), StrokeKind::Inside);
    }
}

fn capsule(p: &egui::Painter, anchor: Align2, at: Pos2, text: &str, color: Color32) {
    let g = p.layout_no_wrap(text.to_string(), FontId::proportional(13.0), color);
    let size = g.size() + vec2(24.0, 14.0);
    let rect = anchor.anchor_size(at, size);
    p.rect_filled(rect, CornerRadius::same(255), theme::BAR);
    p.rect_stroke(
        rect,
        CornerRadius::same(255),
        Stroke::new(1.0, theme::CONTROL_OPEN),
        StrokeKind::Inside,
    );
    p.galley(rect.min + vec2(12.0, 7.0), g, color);
}

/// Fallback pointer when the Mac has sent no shape.
fn draw_arrow(p: &egui::Painter, at: Pos2) {
    let pts = vec![
        at,
        at + vec2(0.0, 17.0),
        at + vec2(4.0, 13.0),
        at + vec2(7.0, 20.0),
        at + vec2(10.0, 19.0),
        at + vec2(7.0, 12.0),
        at + vec2(13.0, 12.0),
    ];
    p.add(egui::Shape::convex_polygon(
        pts,
        Color32::WHITE,
        Stroke::new(1.0, Color32::BLACK),
    ));
}

// ---- TEST ONLY mock cases ----------------------------------------------------------------

/// A labelled sample picture: a gradient with a grid and some blocks (no real Mac).
fn sample_picture(w: u32, h: u32, hue: f32) -> Vec<u8> {
    let mut v = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let mut rgb = [
                (40.0 + 150.0 * fx * (1.0 - hue)) as u8,
                (60.0 + 120.0 * fy) as u8,
                (110.0 + 120.0 * (1.0 - fx) * hue + 60.0 * fy) as u8,
            ];
            let step = (w / 16).max(8);
            if x % step == 0 || y % step == 0 {
                rgb = [
                    rgb[0].saturating_add(60),
                    rgb[1].saturating_add(60),
                    rgb[2].saturating_add(60),
                ];
            }
            let i = ((y * w + x) * 4) as usize;
            v[i] = rgb[2];
            v[i + 1] = rgb[1];
            v[i + 2] = rgb[0];
            v[i + 3] = 255;
        }
    }
    v
}

fn rgba_of_bgra(bgra: &[u8]) -> Vec<u8> {
    bgra.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], 255])
        .collect()
}

/// TEST ONLY: fill `model` (and `video` with a sample frame) for a stream-screen SILL_TEST_MOCK case.
pub fn mock_case(case: &str, model: &mut AppModel, video: &VideoSlot) {
    use sill_proto::switcher::AppInfo;
    if !matches!(
        case,
        "stream-desktop"
            | "stream-drawer"
            | "stream-locked"
            | "stream-reconnecting"
            | "stream-other-user"
            | "stream-menu"
    ) {
        return;
    }
    model.screen = Screen::Stream;
    let s = &mut model.stream;
    s.mac_name = "Sample Mac mini".into();
    s.mac_key_repeat = true;
    s.mac_gestures = true;
    s.menus = Some(sample_menus(&mut s.menu_items));
    let win = |id, title: &str, app: &str, bundle: &str| WindowInfo {
        id,
        title: title.into(),
        app_name: app.into(),
        bundle_id: bundle.into(),
        width: 1200,
        height: 800,
    };
    s.windows = vec![
        win(1, "Sample notes", "Sample Notes", "sample.notes"),
        win(2, "Sample browser", "Sample Browser", "sample.browser"),
        win(3, "Sample editor", "Sample Editor", "sample.editor"),
    ];
    s.apps = [
        "Sample Browser",
        "Sample Calendar",
        "Sample Editor",
        "Sample Notes",
        "Sample Terminal",
    ]
    .iter()
    .map(|n| AppInfo {
        name: n.to_string(),
        bundle_id: format!("sample.{}", n.to_lowercase().replace("sample ", "")),
    })
    .collect();
    s.active = Some(if case == "stream-desktop" {
        StreamSource::Desktop
    } else {
        StreamSource::Window(2)
    });
    for (i, id) in [1u32, 2].iter().enumerate() {
        let bgra = sample_picture(160, 100, i as f32 * 0.6);
        s.thumbnails.insert(
            *id,
            Picture {
                width: 160,
                height: 100,
                rgba: std::sync::Arc::new(rgba_of_bgra(&bgra)),
                generation: 1,
            },
        );
    }
    for (i, b) in ["sample.notes", "sample.browser"].iter().enumerate() {
        let bgra = sample_picture(32, 32, 0.3 + i as f32 * 0.4);
        s.icons.insert(
            b.to_string(),
            Picture {
                width: 32,
                height: 32,
                rgba: std::sync::Arc::new(rgba_of_bgra(&bgra)),
                generation: 1,
            },
        );
    }
    match case {
        // The Mac's own screen locked: its lock screen streams under no cover.
        "stream-locked" => {
            s.locked = Some("Sample Mac mini is locked. Unlock it to keep using it.".into())
        }
        "stream-other-user" => {
            s.locked = Some("Another user started controlling Sample Mac mini.".into());
            s.cover = Some(Cover {
                title: "Another user started controlling this Mac.".into(),
                detail: Some("Sill shows it again when they sign out.".into()),
            });
        }
        "stream-reconnecting" => s.reconnecting = Some("Reconnecting to Sample Mac mini…".into()),
        _ => {}
    }
    s.link_line = (case == "stream-desktop")
        .then(|| "The link can’t keep up with Pro. Lower it in Settings.".into());
    s.cursor.mac_position = Some((0.62, 0.4));
    let (w, h) = (1280u32, 800u32);
    if let Ok(mut slot) = video.lock() {
        *slot = Some(VideoFrame {
            width: w,
            height: h,
            stride: w * 4,
            bgra: sample_picture(w, h, 0.5),
            serial: 1,
        });
    }
}

/// TEST ONLY: sample menus (and the fetched items of two of them).
fn sample_menus(
    items: &mut HashMap<String, sill_proto::menu::MacMenu>,
) -> sill_proto::menu::MacMenu {
    use sill_proto::menu::{MacMenu, MacMenuItem};
    let top = |id: &str, t: &str| MacMenuItem {
        id: Some(id.into()),
        title: Some(t.into()),
        submenu: Some(true),
        ..Default::default()
    };
    let leaf =
        |id: &str, t: &str, key: Option<&str>, enabled: bool, mark: Option<&str>| MacMenuItem {
            id: Some(id.into()),
            title: Some(t.into()),
            key: key.map(Into::into),
            enabled: Some(enabled),
            mark: mark.map(Into::into),
            ..Default::default()
        };
    let sep = MacMenuItem {
        separator: Some(true),
        ..Default::default()
    };
    let file = vec![
        leaf("1.0", "New Window", Some("⌘N"), true, None),
        leaf("1.1", "Open…", Some("⌘O"), true, None),
        MacMenuItem {
            submenu: Some(true),
            ..top("1.2", "Open Recent")
        },
        sep.clone(),
        leaf("1.4", "Save", Some("⌘S"), false, None),
        leaf("1.5", "Save As…", Some("⇧⌘S"), true, None),
        sep,
        leaf("1.7", "Word Wrap", None, true, Some("✓")),
    ];
    items.insert(
        "1".into(),
        MacMenu {
            version: Some(1),
            menu: Some("File".into()),
            items: Some(file),
            ..Default::default()
        },
    );
    items.insert(
        "2".into(),
        MacMenu {
            version: Some(1),
            menu: Some("Edit".into()),
            items: Some(vec![
                leaf("2.0", "Undo", Some("⌘Z"), true, None),
                leaf("2.1", "Redo", Some("⇧⌘Z"), false, None),
            ]),
            stale: Some(true),
            ..Default::default()
        },
    );
    MacMenu {
        version: Some(1),
        app: Some("Sample Editor".into()),
        menus: Some(vec![
            top("1", "File"),
            top("2", "Edit"),
            top("3", "View"),
            top("4", "Window"),
        ]),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aa_detents_and_titles_follow_textscale() {
        assert_eq!(aa_detents(true).len(), 6);
        assert_eq!(
            aa_detents(false),
            vec![Some(0.5), Some(0.75), Some(1.0), Some(1.25), Some(1.5)]
        );
        assert_eq!(aa_title(None, true), "Off");
        assert_eq!(aa_title(None, false), "1×");
        assert_eq!(aa_title(Some(1.0), true), "1×");
        assert_eq!(aa_title(Some(0.5), true), "0.5×");
        assert_eq!(aa_title(Some(1.25), true), "1.25×");
    }

    #[test]
    fn aa_is_usable_only_over_a_window() {
        assert!(aa_selectable(Some(StreamSource::Window(3))));
        assert!(!aa_selectable(Some(StreamSource::Desktop)));
        assert!(!aa_selectable(Some(StreamSource::None)));
        assert!(!aa_selectable(None));
    }

    #[test]
    fn gestures_use_the_hosts_names() {
        let names: Vec<_> = GESTURES.iter().map(|(_, g)| *g).collect();
        assert_eq!(
            names,
            [
                "swipeUp",
                "swipeDown",
                "swipeLeft",
                "swipeRight",
                "spread",
                "pinch"
            ]
        );
    }
}
