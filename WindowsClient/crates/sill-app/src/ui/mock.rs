//! TEST ONLY sample data for SILL_TEST_MOCK (labelled as samples; nothing here is a real Mac).
//! OWNER: the lead; `connect_case` belongs to worker U1, `stream_case` to worker U2.
use std::sync::{Arc, Mutex};

use crate::controller::Handles;
use crate::model::{AppModel, Command};

pub fn start(case: &str) -> Handles {
    let mut model = AppModel { device_name: "SAMPLE-PC (Windows)".into(), ..Default::default() };
    if !connect_case(case, &mut model) {
        stream_case(case, &mut model);
    }
    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    std::thread::spawn(move || {
        for c in rx {
            eprintln!("mock: command {c:?}");
        }
    });
    Handles { shared: Arc::new(Mutex::new(model)), commands: tx, video: Default::default() }
}

/// OWNER: worker U1. Fill `model` for a connect-screen case; false when the case is not one.
fn connect_case(_case: &str, _model: &mut AppModel) -> bool {
    false
}

/// OWNER: worker U2. Fill `model` for a stream-screen case (and its sample video, if any).
fn stream_case(_case: &str, _model: &mut AppModel) {}
