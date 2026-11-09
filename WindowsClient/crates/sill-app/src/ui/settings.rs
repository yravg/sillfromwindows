//! OWNER: worker U3. The Settings panel (iOSClient/HostSettingsPanel.swift): the Mac's stream
//! settings through the ledger (kinds 16/17), this PC's own preferences, Disconnect. Keep the API.
use std::sync::mpsc::Sender;

use crate::model::{AppModel, Command};

#[derive(Default)]
pub struct SettingsUi {}

impl SettingsUi {
    /// Draws the panel's contents into `ui` (the stream screen gives it a right-side area of its
    /// own, 360 pt wide). Returns false when the person closed it (Done or Esc).
    pub fn show(&mut self, ui: &mut egui::Ui, _model: &AppModel, tx: &Sender<Command>) -> bool {
        ui.heading("Settings");
        if ui.button("Disconnect").clicked() {
            let _ = tx.send(Command::Disconnect);
        }
        !ui.button("Done").clicked()
    }
}
