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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Sill")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 420.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("Sill", options, Box::new(|cc| Ok(Box::new(app::SillApp::new(cc)))))
}
