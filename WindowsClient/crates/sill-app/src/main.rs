//! Sill for Windows. OWNER: the lead.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod clipboard;
mod controller;
mod discovery;
mod input;
mod keymap;
mod model;
mod store;
mod theme;
mod ui;

fn main() -> eframe::Result<()> {
    log_to_file();
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
            .with_min_inner_size([640.0, 420.0])
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Sill",
        options,
        Box::new(|cc| Ok(Box::new(app::SillApp::new(cc)))),
    )
}

/// Sill's icon (iOSClient's AppIcon.png, scaled to 256 px) for the window and the taskbar.
fn app_icon() -> egui::IconData {
    let png = include_bytes!("../assets/AppIcon-256.png");
    match image::load_from_memory(png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            }
        }
        Err(_) => egui::IconData::default(),
    }
}

/// On Windows the app has no console, so what it prints (sessions, closes, problems) goes to
/// `%APPDATA%\Sill\Sill.log`, started afresh at each launch. Elsewhere it stays on stderr.
#[cfg(windows)]
fn log_to_file() {
    use std::os::windows::io::IntoRawHandle;
    let Some(dir) = dirs::config_dir().map(|d| d.join("Sill")) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(file) = std::fs::File::create(dir.join("Sill.log")) else {
        return;
    };
    let handle = windows::Win32::Foundation::HANDLE(file.into_raw_handle());
    // Rust's stderr asks for the handle at every write, so this sends eprintln! to the file.
    unsafe {
        let _ = windows::Win32::System::Console::SetStdHandle(
            windows::Win32::System::Console::STD_ERROR_HANDLE,
            handle,
        );
    }
    eprintln!("Sill for Windows {} started", env!("CARGO_PKG_VERSION"));
}

#[cfg(not(windows))]
fn log_to_file() {}
