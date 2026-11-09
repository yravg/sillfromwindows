//! TEST ONLY sample data for SILL_TEST_MOCK (labelled as samples; nothing here is a real Mac).
//! OWNER: the lead. The cases live with their screens: ui::connect::mock_case (worker U1),
//! ui::stream::mock_case (worker U2).
use std::sync::{Arc, Mutex};

use crate::controller::Handles;
use crate::model::{AppModel, Command};

pub fn start(case: &str) -> Handles {
    let mut model = AppModel {
        device_name: "SAMPLE-PC (Windows)".into(),
        ..Default::default()
    };
    let video = crate::model::VideoSlot::default();
    if !crate::ui::connect::mock_case(case, &mut model) {
        crate::ui::stream::mock_case(case, &mut model, &video);
    }
    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    std::thread::spawn(move || {
        for c in rx {
            eprintln!("mock: command {c:?}");
        }
    });
    Handles {
        shared: Arc::new(Mutex::new(model)),
        commands: tx,
        video,
    }
}
