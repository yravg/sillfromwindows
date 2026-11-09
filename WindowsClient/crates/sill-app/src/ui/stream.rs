//! OWNER: worker U2. The stream screen: video, bar, thumbnails, Apps drawer, the Mac's pointer.
use std::sync::mpsc::Sender;

use crate::model::{AppModel, Command, VideoSlot};

pub struct StreamUi {}

impl StreamUi {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        StreamUi {}
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        _frame: &mut eframe::Frame,
        _model: &AppModel,
        _tx: &Sender<Command>,
        _video: &VideoSlot,
    ) {
        egui::CentralPanel::default().show(ui, |_ui| {});
    }
}

/// TEST ONLY: fill `model` (and `video` with a sample frame) for a stream-screen SILL_TEST_MOCK case.
pub fn mock_case(_case: &str, _model: &mut AppModel, _video: &VideoSlot) {}
