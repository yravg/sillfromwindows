//! OWNER: worker D1. Bonjour browse of `_sill._tcp` (mdns-sd) and the connect screen's rows
//! (iOSClient/DiscoveryPolicy.swift's rules, ported pure and tested). Keep the API below.
use std::net::IpAddr;
use std::time::Instant;

use sill_net::pair_client::SavedMac;
use sill_net::pairing::HomeDoor;

use crate::model::{MacRow, Waker};

/// One `_sill._tcp` service as the browser last resolved it.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The Bonjour instance name ("Enrico's MacBook Pro").
    pub name: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    /// TXT `r`: the recognition tag, when the Mac has an identity.
    pub tag: Option<String>,
    /// TXT `p`: what the home door is.
    pub door: HomeDoor,
}

pub struct Discovery {
    started: Instant,
}

impl Discovery {
    /// Starts browsing; `waker` is called whenever the list changes.
    pub fn start(_waker: Waker) -> Discovery {
        Discovery { started: Instant::now() }
    }
    /// The services listed now.
    pub fn found(&self) -> Vec<Found> {
        Vec::new()
    }
    pub fn started_at(&self) -> Instant {
        self.started
    }
}

/// The rows, pure: network services (named by their saved Mac when the tag says so) and then each
/// saved Mac not listed on the network as a "Remote" row.
pub fn rows(_found: &[Found], _saved: &[SavedMac]) -> Vec<MacRow> {
    Vec::new()
}
