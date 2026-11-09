//! Sill for Windows. OWNER: the lead.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod controller;
mod discovery;
mod input;
mod keymap;
mod model;
mod store;
mod theme;
mod ui;

fn main() -> eframe::Result<()> {
    // TEST ONLY: SILL_TEST_WINDOW=WxH opens the window at that size (screenshots at several sizes).
    let size = std::env::var("SILL_TEST_WINDOW")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1280.0, 800.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Sill")
            .with_inner_size(size)
            .with_min_inner_size([640.0, 420.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Sill",
        options,
        Box::new(|cc| Ok(Box::new(app::SillApp::new(cc)))),
    )
}
