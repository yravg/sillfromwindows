//! Which Mac a row is and where to dial it (iOSClient/RemoteDialPolicy.swift's order, DiscoveryPolicy's
//! `rowMac`): home rows first, then the saved Mac's own addresses (Tailscale, a VPN, a typed one).
//! A listed service is a saved Mac when its recognition tag was made with that Mac's key.
use std::net::IpAddr;

use sill_net::identity::base64url_decode;
use sill_net::pair_client::SavedMac;
use sill_net::pairing::{recognition_tag, HomeDoor};

use crate::discovery::Found;

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// A saved Mac: dial these `host:port` in order.
    Dial {
        mac: SavedMac,
        targets: Vec<String>,
    },
    /// A listed Mac this PC has not paired with and whose door speaks TLS: ask it to pair.
    Pair {
        name: String,
        target: String,
    },
    /// Listed, but its door is plain TCP (a development build): this client never dials it.
    Plain(String),
    NotFound,
}

/// `host:port` for a listed service's addresses: IPv4 first, IPv6 after; link-local IPv6 is left
/// out (it needs a scope the browser's address does not carry).
pub fn home_targets(f: &Found) -> Vec<String> {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for a in &f.addresses {
        match a {
            IpAddr::V4(a) if !a.is_unspecified() => v4.push(format!("{a}:{}", f.port)),
            IpAddr::V6(a) if !a.is_unspecified() && (a.segments()[0] & 0xffc0) != 0xfe80 => {
                v6.push(format!("[{a}]:{}", f.port))
            }
            _ => {}
        }
    }
    v4.extend(v6);
    v4
}

/// The saved Mac a listed service is: its tag says so, or (no tag at all) its name is.
pub fn saved_for<'a>(f: &Found, saved: &'a [SavedMac]) -> Option<&'a SavedMac> {
    match &f.tag {
        Some(tag) => saved.iter().find(|m| {
            m.recognition_key
                .as_deref()
                .and_then(base64url_decode)
                .is_some_and(|k| recognition_tag::matches(tag, &k))
        }),
        None => saved.iter().find(|m| m.name == f.name),
    }
}

fn push_unique(into: &mut Vec<String>, more: impl IntoIterator<Item = String>) {
    for t in more {
        if !into.contains(&t) {
            into.push(t);
        }
    }
}

/// The plan for a row id ("saved:<macID>" or "bonjour:<service name>"). `remote_only` is "Connect
/// Remotely": the saved addresses only, never the home door.
pub fn plan(row_id: &str, saved: &[SavedMac], found: &[Found], remote_only: bool) -> Plan {
    if let Some(id) = row_id.strip_prefix("saved:") {
        let Some(mac) = saved.iter().find(|m| m.mac_id == id) else {
            return Plan::NotFound;
        };
        let mut targets = Vec::new();
        if !remote_only {
            for f in found
                .iter()
                .filter(|f| saved_for(f, std::slice::from_ref(mac)).is_some())
            {
                push_unique(&mut targets, home_targets(f));
            }
        }
        push_unique(&mut targets, mac.dial_targets());
        return Plan::Dial {
            mac: mac.clone(),
            targets,
        };
    }
    if let Some(name) = row_id.strip_prefix("bonjour:") {
        let Some(f) = found.iter().find(|f| f.name == name) else {
            return Plan::NotFound;
        };
        if let Some(mac) = saved_for(f, saved) {
            let mut targets = if remote_only {
                Vec::new()
            } else {
                home_targets(f)
            };
            push_unique(&mut targets, mac.dial_targets());
            return Plan::Dial {
                mac: mac.clone(),
                targets,
            };
        }
        if f.door == HomeDoor::Plain {
            return Plan::Plain(f.name.clone());
        }
        return match home_targets(f).into_iter().next() {
            Some(target) => Plan::Pair {
                name: f.name.clone(),
                target,
            },
            None => Plan::NotFound,
        };
    }
    Plan::NotFound
}

/// The home targets a reconnect adds in front of a saved Mac's own addresses.
pub fn reconnect_targets(mac: &SavedMac, found: &[Found], remote_allowed: bool) -> Vec<String> {
    let mut targets = Vec::new();
    for f in found
        .iter()
        .filter(|f| saved_for(f, std::slice::from_ref(mac)).is_some())
    {
        push_unique(&mut targets, home_targets(f));
    }
    if remote_allowed {
        push_unique(&mut targets, mac.dial_targets());
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use sill_net::identity::base64url_encode;

    fn key(n: u8) -> Vec<u8> {
        vec![n; 32]
    }

    fn saved(id: &str, name: &str, n: u8, addrs: &[&str]) -> SavedMac {
        SavedMac {
            name: name.into(),
            mac_id: id.into(),
            fingerprint: "Tyl0TmZlUyPNKVJsJ_vePXGZMbcQgFBFZtA-06cZ4ik".into(),
            addresses: addrs.iter().map(|s| s.to_string()).collect(),
            port: 7455,
            recognition_key: Some(base64url_encode(&key(n))),
            method: "code".into(),
        }
    }

    fn found(name: &str, n: Option<u8>, door: HomeDoor, addrs: &[&str]) -> Found {
        Found {
            name: name.into(),
            addresses: addrs.iter().map(|a| a.parse().unwrap()).collect(),
            port: 50123,
            tag: n.map(|n| recognition_tag::make(&key(n), &[1, 2, 3, 4, 5, 6])),
            door,
        }
    }

    #[test]
    fn a_saved_row_dials_home_first_then_its_own_addresses() {
        let s = [saved("A", "Studio", 1, &["100.88.89.4", "[fd7a::1]:9000"])];
        let f = [found(
            "Studio",
            Some(1),
            HomeDoor::PairingRequired,
            &["192.168.1.5", "fe80::1", "2001:db8::2"],
        )];
        let Plan::Dial { targets, .. } = plan("saved:A", &s, &f, false) else {
            panic!()
        };
        assert_eq!(
            targets,
            vec![
                "192.168.1.5:50123",
                "[2001:db8::2]:50123",
                "100.88.89.4:7455",
                "[fd7a::1]:9000"
            ]
        );
    }

    #[test]
    fn connect_remotely_never_dials_the_home_door() {
        let s = [saved("A", "Studio", 1, &["100.88.89.4"])];
        let f = [found(
            "Studio",
            Some(1),
            HomeDoor::PairingRequired,
            &["192.168.1.5"],
        )];
        let Plan::Dial { targets, .. } = plan("saved:A", &s, &f, true) else {
            panic!()
        };
        assert_eq!(targets, vec!["100.88.89.4:7455"]);
    }

    #[test]
    fn a_service_under_a_saved_name_with_another_tag_is_not_that_mac() {
        let s = [saved("A", "Studio", 1, &["100.88.89.4"])];
        // same Bonjour name, a tag made with another key (a look-alike or a Mac set up again)
        let f = [found(
            "Studio",
            Some(9),
            HomeDoor::PairingRequired,
            &["192.168.1.5"],
        )];
        assert_eq!(
            plan("bonjour:Studio", &s, &f, false),
            Plan::Pair {
                name: "Studio".into(),
                target: "192.168.1.5:50123".into()
            }
        );
        // the saved row itself never takes its address from it
        let Plan::Dial { targets, .. } = plan("saved:A", &s, &f, false) else {
            panic!()
        };
        assert_eq!(targets, vec!["100.88.89.4:7455"]);
    }

    #[test]
    fn a_listed_saved_mac_is_dialed_not_asked() {
        let s = [saved("A", "Studio", 1, &["100.88.89.4"])];
        let f = [found(
            "Studio",
            Some(1),
            HomeDoor::PairingRequired,
            &["192.168.1.5"],
        )];
        assert!(matches!(
            plan("bonjour:Studio", &s, &f, false),
            Plan::Dial { .. }
        ));
    }

    #[test]
    fn an_unknown_open_or_pairing_door_asks_and_a_plain_door_is_never_dialed() {
        let f = [
            found("New", Some(3), HomeDoor::Open, &["10.0.0.2"]),
            found("Old", None, HomeDoor::Plain, &["10.0.0.3"]),
        ];
        assert!(matches!(
            plan("bonjour:New", &[], &f, false),
            Plan::Pair { .. }
        ));
        assert_eq!(
            plan("bonjour:Old", &[], &f, false),
            Plan::Plain("Old".into())
        );
        assert_eq!(plan("bonjour:Gone", &[], &f, false), Plan::NotFound);
        assert_eq!(plan("saved:X", &[], &f, false), Plan::NotFound);
    }

    #[test]
    fn a_service_with_no_address_we_can_dial_is_not_found() {
        let f = [found("Only", Some(3), HomeDoor::Open, &["fe80::7"])];
        assert_eq!(plan("bonjour:Only", &[], &f, false), Plan::NotFound);
    }

    #[test]
    fn a_reconnect_prefers_a_listed_door_and_obeys_remote_allowed() {
        let m = saved("A", "Studio", 1, &["100.88.89.4"]);
        let f = [found(
            "Studio",
            Some(1),
            HomeDoor::PairingRequired,
            &["192.168.1.5"],
        )];
        assert_eq!(
            reconnect_targets(&m, &f, true),
            vec!["192.168.1.5:50123", "100.88.89.4:7455"]
        );
        assert_eq!(reconnect_targets(&m, &f, false), vec!["192.168.1.5:50123"]);
        assert_eq!(reconnect_targets(&m, &[], true), vec!["100.88.89.4:7455"]);
    }
}
