//! The eframe app: owns the controller's handles and hands each frame to the screen that shows.
//! OWNER: the lead.
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::model::{Command, Screen, Shared, VideoSlot};
use crate::{controller, theme, ui};

pub struct SillApp {
    shared: Shared,
    commands: Sender<Command>,
    video: VideoSlot,
    connect: ui::connect::ConnectUi,
    stream: ui::stream::StreamUi,
    harness: Harness,
}

/// TEST ONLY: `SILL_TEST_SHOT="a.png@2;b.png@5"` saves what the app drew at those seconds after
/// launch; `SILL_TEST_QUIT_AFTER=6` quits then. For headless checks (Xvfb on Linux); inert unset.
struct Harness {
    started: std::time::Instant,
    shots: Vec<(String, f64)>,
    quit_after: Option<f64>,
}

impl Harness {
    fn from_env() -> Self {
        let shots = std::env::var("SILL_TEST_SHOT")
            .unwrap_or_default()
            .split(';')
            .filter_map(|e| {
                let (p, t) = e.rsplit_once('@')?;
                Some((p.to_string(), t.parse().ok()?))
            })
            .collect();
        let quit_after = std::env::var("SILL_TEST_QUIT_AFTER")
            .ok()
            .and_then(|v| v.parse().ok());
        Harness {
            started: std::time::Instant::now(),
            shots,
            quit_after,
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if self.shots.is_empty() && self.quit_after.is_none() {
            return;
        }
        let now = self.started.elapsed().as_secs_f64();
        let (due, later): (Vec<_>, Vec<_>) = self.shots.drain(..).partition(|(_, t)| *t <= now);
        self.shots = later;
        for (path, _) in due {
            let cb = egui::ScreenshotCallback::new(move |img: std::sync::Arc<egui::ColorImage>| {
                let [w, h] = img.size;
                let mut buf = Vec::with_capacity(w * h * 4);
                for c in &img.pixels {
                    buf.extend_from_slice(&c.to_srgba_unmultiplied());
                }
                match image::RgbaImage::from_raw(w as u32, h as u32, buf).map(|i| i.save(&path)) {
                    Some(Ok(())) => eprintln!("harness: saved {path} ({w}x{h})"),
                    other => eprintln!("harness: could not save {path}: {other:?}"),
                }
            });
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(cb));
        }
        if self.quit_after.is_some_and(|q| now >= q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl SillApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let waker = Arc::new(move || ctx.request_repaint());
        // TEST ONLY: SILL_TEST_MOCK=<case> draws labelled sample data (ui/mock.rs) with no
        // controller and no network; every command is printed instead of sent.
        let handles = match std::env::var("SILL_TEST_MOCK") {
            Ok(case) => ui::mock::start(&case),
            Err(_) => controller::start(waker),
        };
        // TEST ONLY: SILL_TEST_CONNECT=<Mac ID> connects to that saved Mac at launch, as a click
        // on its row does (headless end-to-end runs).
        if let Ok(id) = std::env::var("SILL_TEST_CONNECT") {
            let _ = handles.commands.send(Command::Connect(format!("saved:{id}")));
        }
        SillApp {
            shared: handles.shared,
            commands: handles.commands,
            video: handles.video,
            connect: ui::connect::ConnectUi::default(),
            stream: ui::stream::StreamUi::new(cc),
            harness: Harness::from_env(),
        }
    }
}

impl eframe::App for SillApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let model = self.shared.lock().map(|m| m.clone()).unwrap_or_default();
        match model.screen {
            Screen::Connect => self.connect.show(ui, &model, &self.commands),
            Screen::Stream => self
                .stream
                .show(ui, frame, &model, &self.commands, &self.video),
        }
        self.harness.tick(ui.ctx());
    }

    fn on_exit(&mut self) {
        let _ = self.commands.send(Command::Quit);
    }
}
