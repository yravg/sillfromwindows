//! OWNER: worker C1. The controller thread: commands in, model and video out.
use std::sync::mpsc::Sender;

use crate::model::{Command, Shared, VideoSlot, Waker};

pub struct Handles {
    pub shared: Shared,
    pub commands: Sender<Command>,
    pub video: VideoSlot,
}

pub fn start(_waker: Waker) -> Handles {
    let (tx, _rx) = std::sync::mpsc::channel();
    Handles { shared: Default::default(), commands: tx, video: Default::default() }
}
