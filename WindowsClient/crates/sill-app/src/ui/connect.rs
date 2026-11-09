//! OWNER: worker U1. The connect screen and pairing.
use std::sync::mpsc::Sender;

use crate::model::{AppModel, Command};

#[derive(Default)]
pub struct ConnectUi {}

impl ConnectUi {
    pub fn show(&mut self, ui: &mut egui::Ui, _model: &AppModel, _tx: &Sender<Command>) {
        egui::CentralPanel::default().show(ui, |ui| ui.heading("Connect to a Mac"));
    }
}
