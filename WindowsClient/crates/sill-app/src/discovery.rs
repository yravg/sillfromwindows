//! Bonjour browse of `_sill._tcp` (mdns-sd) and the connect screen's rows (iOSClient/
//! DiscoveryPolicy.swift's rules, ported pure and tested).
//!
//! Decisions for Windows: there is no AWDL (mdns-sd's Apple P2P stays off, the Swift
//! `includePeerToPeer` has no counterpart). mdns-sd does not say which interface a service was seen
//! on, so a saved Mac on the network always reads "Wi-Fi" (the word is a hint, not a promise; "Wired"
//! and "Direct" are never produced). `SavedMac` carries no revoked/newKey/homeTLS flags, so a saved
//! Mac is taken as not removed, not re-keyed, and seen over TLS.
use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use sill_net::identity::base64url_decode;
use sill_net::pair_client::SavedMac;
use sill_net::pairing::{home_door_entry, recognition_tag, HomeDoor};

use crate::model::{MacRow, Waker};

/// The Bonjour type Sill for Mac advertises.
pub const SERVICE_TYPE: &str = "_sill._tcp.local.";

/// One `_sill._tcp` service as the browser last resolved it.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The Bonjour instance name ("Enrico's MacBook Pro").
    pub name: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    /// TXT `r`: the recognition tag, when the Mac has an identity (empty reads as none).
    pub tag: Option<String>,
    /// TXT `p`: what the home door is.
    pub door: HomeDoor,
}

pub struct Discovery {
    started: Instant,
    list: Arc<Mutex<BTreeMap<String, Found>>>,
    stop: Arc<AtomicBool>,
}

impl Discovery {
    /// Starts browsing; `waker` is called whenever the list changes.
    pub fn start(waker: Waker) -> Discovery {
        Self::start_type(SERVICE_TYPE, waker)
    }

    fn start_type(ty: &'static str, waker: Waker) -> Discovery {
        let list = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (l, s) = (list.clone(), stop.clone());
        std::thread::Builder::new()
            .name("sill-discovery".into())
            .spawn(move || browse_loop(ty, l, s, waker))
            .expect("spawn discovery thread");
        Discovery {
            started: Instant::now(),
            list,
            stop,
        }
    }

    /// The services listed now, by Bonjour name.
    pub fn found(&self) -> Vec<Found> {
        let mut v: Vec<Found> = self.list.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn started_at(&self) -> Instant {
        self.started
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------------------------
// The browser (StreamClient.rebuildBrowser, StreamClient.swift:1336)

/// iOSClient/DiscoveryPolicy.swift:678 `browserRetry`.
const BROWSER_RETRY: [u64; 6] = [1, 2, 4, 8, 16, 30];
/// DiscoveryPolicy.swift:680 `browserStable`: a browser that ran this long counts as the first failure.
const BROWSER_STABLE: Duration = Duration::from_secs(30);

/// DiscoveryPolicy.swift:691 `browserRetryDelay(afterFailures:)`.
fn browser_retry_delay(failures: usize) -> Duration {
    Duration::from_secs(BROWSER_RETRY[failures.clamp(1, BROWSER_RETRY.len()) - 1])
}

/// DiscoveryPolicy.swift:697 `browserFailures`.
fn browser_failures(previous: usize, ran_for: Duration) -> usize {
    if ran_for >= BROWSER_STABLE {
        1
    } else {
        previous + 1
    }
}

fn instance_name(fullname: &str, ty: &str) -> String {
    let suffix = format!(".{ty}");
    fullname
        .strip_suffix(&suffix)
        .unwrap_or(fullname)
        .to_string()
}

fn browse_loop(
    ty: &'static str,
    list: Arc<Mutex<BTreeMap<String, Found>>>,
    stop: Arc<AtomicBool>,
    waker: Waker,
) {
    let mut failures = 0usize;
    while !stop.load(Ordering::Relaxed) {
        let began = Instant::now();
        let ended_by_failure = run_browser(ty, &list, &stop, &waker);
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if ended_by_failure {
            failures = browser_failures(failures, began.elapsed());
            // What a dead browser listed is stale; a browser made again lists afresh.
            if !list.lock().unwrap().is_empty() {
                list.lock().unwrap().clear();
                waker();
            }
            let until = Instant::now() + browser_retry_delay(failures);
            while Instant::now() < until && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// One browser's life. True when it failed (the daemon or the channel went away).
fn run_browser(
    ty: &str,
    list: &Arc<Mutex<BTreeMap<String, Found>>>,
    stop: &AtomicBool,
    waker: &Waker,
) -> bool {
    let Ok(daemon) = ServiceDaemon::new() else {
        return true;
    };
    let Ok(rx) = daemon.browse(ty) else {
        let _ = daemon.shutdown();
        return true;
    };
    let mut failed = false;
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(ServiceEvent::ServiceResolved(r)) => {
                let name = instance_name(r.get_fullname(), ty);
                let mut addresses: Vec<IpAddr> =
                    r.get_addresses().iter().map(|a| a.to_ip_addr()).collect();
                addresses.sort();
                addresses.dedup();
                let tag = r
                    .get_property_val_str("r")
                    .filter(|t| !t.is_empty())
                    .map(str::to_string);
                let door = home_door_entry(r.get_property_val("p"));
                let found = Found {
                    name,
                    addresses,
                    port: r.get_port(),
                    tag,
                    door,
                };
                let mut l = list.lock().unwrap();
                if l.get(r.get_fullname()) != Some(&found) {
                    l.insert(r.get_fullname().to_string(), found);
                    drop(l);
                    waker();
                }
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                if list.lock().unwrap().remove(&fullname).is_some() {
                    waker();
                }
            }
            Ok(_) => {}
            Err(mdns_sd::RecvTimeoutError::Timeout) => {}
            Err(mdns_sd::RecvTimeoutError::Disconnected) => {
                failed = true;
                break;
            }
        }
    }
    let _ = daemon.shutdown();
    failed
}

// ---------------------------------------------------------------------------------------------
// The rows (DiscoveryPolicy.swift §7.3, StreamClient.recomputeMacs StreamClient.swift:1450-1500)

/// A saved Mac by the Bonjour name it was reached under (here: its saved name), as
/// `rowMac`'s `saved` argument.
type ByName<'a> = [(&'a str, &'a str)];

/// DiscoveryPolicy.swift:958 `rowMac`: the saved Mac a row is: the one its tag names, else, for a row
/// that carries no tag at all, the saved Mac under the row's name (taken by name alone). Returns
/// (macID, tagNamed).
fn row_mac(
    tagged: Option<&str>,
    carries_tag: bool,
    name: &str,
    saved: &ByName,
) -> Option<(String, bool)> {
    if let Some(t) = tagged {
        return Some((t.to_string(), true));
    }
    if carries_tag {
        return None;
    }
    saved
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(id, _)| (id.to_string(), false))
}

/// DiscoveryPolicy.swift:968 `otherTagUnderSavedName`: a tag that names no saved Mac under a saved
/// Mac's name (a Mac set up again, or a look-alike).
fn other_tag_under_saved_name(
    tagged: Option<&str>,
    carries_tag: bool,
    name: &str,
    saved: &ByName,
) -> bool {
    tagged.is_none() && carries_tag && saved.iter().any(|(_, n)| *n == name)
}

/// DiscoveryPolicy.swift:977 `rowDoor`: such a row on an open door is dialed as one that requires
/// pairing.
fn row_door(door: HomeDoor, other_tag_under_saved_name: bool) -> HomeDoor {
    if other_tag_under_saved_name && door == HomeDoor::Open {
        HomeDoor::PairingRequired
    } else {
        door
    }
}

/// DiscoveryPolicy.swift:1046 `rowWord`, Release (`debug` false, a plain door is never dialed),
/// the cable case dropped (no wired reading on Windows), `revoked`/`newKey` false.
fn row_word(door: HomeDoor, saved: bool) -> &'static str {
    match door {
        HomeDoor::Plain => "Update Sill",
        HomeDoor::PairingRequired | HomeDoor::Open => {
            if saved {
                "Wi-Fi"
            } else {
                "Not paired"
            }
        }
    }
}

/// SavedMacs.swift:236 `remoteOrder`. The saved list is kept in pairing order (oldest first), so a
/// name's records go together, newest first, in the order names first appear.
fn remote_order(list: &[SavedMac]) -> Vec<&SavedMac> {
    let mut out: Vec<&SavedMac> = Vec::new();
    let mut done: HashSet<&str> = HashSet::new();
    for mac in list {
        if done.insert(mac.name.as_str()) {
            out.extend(list.iter().filter(|m| m.name == mac.name).rev());
        }
    }
    out
}

/// SavedMacs.swift:257 `recognize`: the saved Mac whose recognition key made `tag`.
fn recognize<'a>(tag: Option<&str>, saved: &'a [SavedMac]) -> Option<&'a SavedMac> {
    let tag = tag.filter(|t| !t.is_empty())?;
    saved.iter().find(|m| {
        m.recognition_key
            .as_deref()
            .and_then(base64url_decode)
            .is_some_and(|key| recognition_tag::matches(tag, &key))
    })
}

/// The rows, pure: network services (named by their saved Mac when the tag says so), by name, then
/// each saved Mac not listed on the network as a "Remote" row (oldest-first save order reversed per
/// name). Ids: "saved:<macID>" for a saved Mac, "bonjour:<instance name>" for the rest.
pub fn rows(found: &[Found], saved: &[SavedMac]) -> Vec<MacRow> {
    let by_name: Vec<(&str, &str)> = saved
        .iter()
        .rev() // most recently saved first, as the Swift list is by last use
        .map(|m| (m.mac_id.as_str(), m.name.as_str()))
        .collect();
    let mut sorted: Vec<&Found> = found.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = Vec::new();
    let mut listed: HashSet<String> = HashSet::new();
    for f in sorted {
        let tagged = recognize(f.tag.as_deref(), saved).map(|m| m.mac_id.as_str());
        let carries = f.tag.as_deref().is_some_and(|t| !t.is_empty());
        let named = row_mac(tagged, carries, &f.name, &by_name);
        let saved_mac = named
            .as_ref()
            .and_then(|(id, _)| saved.iter().find(|m| &m.mac_id == id));
        let door = row_door(
            f.door,
            other_tag_under_saved_name(tagged, carries, &f.name, &by_name),
        );
        let word = row_word(door, saved_mac.is_some());
        let (id, mac_id) = match saved_mac {
            Some(m) => {
                if !listed.insert(m.mac_id.clone()) {
                    continue; // a second service of the same Mac: one row
                }
                (format!("saved:{}", m.mac_id), Some(m.mac_id.clone()))
            }
            None => (format!("bonjour:{}", f.name), None),
        };
        out.push(MacRow {
            id,
            name: f.name.clone(),
            word: Some(word.to_string()),
            mac_id,
            busy: false,
        });
    }
    for m in remote_order(saved) {
        if listed.contains(&m.mac_id) {
            continue;
        }
        out.push(MacRow {
            id: format!("saved:{}", m.mac_id),
            name: m.name.clone(),
            word: Some("Remote".into()),
            mac_id: Some(m.mac_id.clone()),
            busy: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sill_net::identity::base64url_encode;

    fn saved(name: &str, id: &str, key: &[u8]) -> SavedMac {
        SavedMac {
            name: name.into(),
            mac_id: id.into(),
            fingerprint: String::new(),
            addresses: vec![],
            port: 7455,
            recognition_key: Some(base64url_encode(key)),
            method: "code".into(),
        }
    }
    fn found(name: &str, tag: Option<String>, door: HomeDoor) -> Found {
        Found {
            name: name.into(),
            addresses: vec!["10.0.0.2".parse().unwrap()],
            port: 47000,
            tag,
            door,
        }
    }
    fn tag(key: &[u8]) -> String {
        recognition_tag::make(key, &[1, 2, 3, 4, 5, 6])
    }

    // ---- ported from Tests/checks/home-device/main.swift:165-173 (rowMac) ----
    #[test]
    fn row_mac_cases() {
        let by: Vec<(&str, &str)> = vec![
            ("RECENT", "Mac mini"),
            ("OLD", "Mac mini"),
            ("STUDIO", "Studio"),
        ];
        assert_eq!(
            row_mac(Some("OLD"), true, "Studio", &by),
            Some(("OLD".into(), true))
        );
        assert_eq!(
            row_mac(None, false, "Studio", &by),
            Some(("STUDIO".into(), false))
        );
        assert_eq!(
            row_mac(None, false, "Mac mini", &by),
            Some(("RECENT".into(), false)),
            "two saved of one name: the first given"
        );
        assert_eq!(row_mac(None, false, "Office", &by), None);
        assert_eq!(row_mac(None, false, "Mac mini (2)", &by), None);
        assert_eq!(row_mac(None, false, "mac mini", &by), None);
        assert_eq!(row_mac(None, false, "Mac mini", &[]), None);
        // a tag that names nobody is never the name rule's
        assert_eq!(row_mac(None, true, "Studio", &by), None);
    }

    // ---- home-device: otherTagUnderSavedName / rowDoor ----
    #[test]
    fn other_tag_and_row_door() {
        let by = vec![("A", "Studio")];
        assert!(other_tag_under_saved_name(None, true, "Studio", &by));
        assert!(!other_tag_under_saved_name(None, true, "Office", &by));
        assert!(!other_tag_under_saved_name(None, false, "Studio", &by));
        assert!(!other_tag_under_saved_name(Some("A"), true, "Studio", &by));
        assert_eq!(row_door(HomeDoor::Open, true), HomeDoor::PairingRequired);
        assert_eq!(row_door(HomeDoor::Open, false), HomeDoor::Open);
        assert_eq!(row_door(HomeDoor::Plain, true), HomeDoor::Plain);
    }

    // ---- ported from home-device rowWord table (Release, no cable) ----
    #[test]
    fn row_word_table() {
        for door in [HomeDoor::PairingRequired, HomeDoor::Open] {
            assert_eq!(row_word(door, true), "Wi-Fi");
            assert_eq!(row_word(door, false), "Not paired");
        }
        // no downgrade: a plain door is "Update Sill", saved or not
        assert_eq!(row_word(HomeDoor::Plain, true), "Update Sill");
        assert_eq!(row_word(HomeDoor::Plain, false), "Update Sill");
    }

    // ---- ported: browserRetryDelay / browserFailures (policy) ----
    #[test]
    fn browser_backoff() {
        let d: Vec<u64> = (1..=8).map(|n| browser_retry_delay(n).as_secs()).collect();
        assert_eq!(d, vec![1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(browser_retry_delay(0).as_secs(), 1);
        assert_eq!(browser_failures(3, Duration::from_secs(29)), 4);
        assert_eq!(browser_failures(3, Duration::from_secs(30)), 1);
    }

    // ---- ported: remoteOrder (policy / remote-rules): one name's records newest first ----
    #[test]
    fn remote_order_groups_names_newest_first() {
        let list = vec![
            saved("Mac", "OLD", b"k1"),
            saved("Other", "O", b"k2"),
            saved("Mac", "NEW", b"k3"),
        ];
        let ids: Vec<&str> = remote_order(&list)
            .iter()
            .map(|m| m.mac_id.as_str())
            .collect();
        assert_eq!(ids, vec!["NEW", "OLD", "O"]);
    }

    // ---- new: rows end to end ----
    #[test]
    fn network_rows_by_name_then_remote() {
        let k = b"recognition-key-aaaaaaaaaaaaaaaa";
        let s = vec![saved("Studio", "STUDIO", k), saved("Away", "AWAY", b"zz")];
        let f = vec![
            found("Zed", None, HomeDoor::Open),
            found("Studio", Some(tag(k)), HomeDoor::PairingRequired),
            found("Old Mac", None, HomeDoor::Plain),
        ];
        let r = rows(&f, &s);
        let view: Vec<(&str, &str)> = r
            .iter()
            .map(|r| (r.id.as_str(), r.word.as_deref().unwrap()))
            .collect();
        assert_eq!(
            view,
            vec![
                ("bonjour:Old Mac", "Update Sill"),
                ("saved:STUDIO", "Wi-Fi"),
                ("bonjour:Zed", "Not paired"),
                ("saved:AWAY", "Remote"),
            ]
        );
        assert_eq!(r[1].mac_id.as_deref(), Some("STUDIO"));
    }

    #[test]
    fn tag_by_key_wins_over_a_renamed_mac() {
        // the saved Mac's Bonjour name changed: its tag still names it
        let k = b"key-key-key-key-key-key-key-key!";
        let r = rows(
            &[found("Studio (2)", Some(tag(k)), HomeDoor::PairingRequired)],
            &[saved("Studio", "S", k)],
        );
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].id.as_str(), r[0].word.as_deref()),
            ("saved:S", Some("Wi-Fi"))
        );
    }

    #[test]
    fn name_rule_only_for_a_tagless_row() {
        let s = vec![saved("Studio", "S", b"real-key")];
        // tagless TLS row under the saved name: the saved Mac, by name alone
        let r = rows(&[found("Studio", None, HomeDoor::PairingRequired)], &s);
        assert_eq!(r[0].id, "saved:S");
        // a tag no saved Mac resolves under the saved name: not that Mac, not paired, Remote beside it
        let r = rows(
            &[found("Studio", Some(tag(b"other-key")), HomeDoor::Open)],
            &s,
        );
        assert_eq!(r[0].id, "bonjour:Studio");
        assert_eq!(r[0].word.as_deref(), Some("Not paired"));
        assert_eq!(r[1].id, "saved:S");
        assert_eq!(r[1].word.as_deref(), Some("Remote"));
    }

    #[test]
    fn empty_tag_counts_as_none_and_duplicates_collapse() {
        let k = b"dupkey";
        let s = vec![saved("Studio", "S", k)];
        let r = rows(
            &[
                found("Studio", Some(tag(k)), HomeDoor::PairingRequired),
                found("Studio ", Some(tag(k)), HomeDoor::PairingRequired),
            ],
            &s,
        );
        assert_eq!(r.len(), 1);
        assert!(rows(&[], &[]).is_empty());
    }

    // ---- live: mDNS on this machine ----
    #[test]
    fn live_browse_finds_and_removes_a_service() {
        use mdns_sd::ServiceInfo;
        let hits = Arc::new(AtomicBool::new(false));
        let h = hits.clone();
        let d = Discovery::start(Arc::new(move || h.store(true, Ordering::Relaxed)));
        let reg = ServiceDaemon::new().expect("daemon");
        let name = format!("SillTest{}", std::process::id());
        let k = b"live-key";
        let txt = [("r", tag(k)), ("p", "1".to_string())];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &name,
            "sill-test-host.local.",
            "",
            47999,
            &txt[..],
        )
        .expect("info")
        .enable_addr_auto();
        reg.register(info).expect("register");
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut got = None;
        while Instant::now() < deadline {
            if let Some(f) = d.found().into_iter().find(|f| f.name == name) {
                got = Some(f);
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let f = got.expect("service found over mDNS");
        assert_eq!(f.port, 47999);
        assert_eq!(f.door, HomeDoor::PairingRequired);
        assert_eq!(f.tag.as_deref(), Some(tag(k).as_str()));
        assert!(hits.load(Ordering::Relaxed), "waker called");
        let r = rows(&[f], &[saved("Mine", "M", k)]);
        assert_eq!(r[0].id, "saved:M");
        reg.unregister(&format!("{name}.{SERVICE_TYPE}")).unwrap();
        let _ = reg.shutdown();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && d.found().iter().any(|f| f.name == name) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(!d.found().iter().any(|f| f.name == name), "removed");
    }
}
