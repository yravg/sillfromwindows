//! Pairing proofs, recognition tag, TXT `p`, SafeText, the address parser, PairLink and kind 18.
//! Cases ported from Tests/checks/protocol/main.swift (address parser, PairingCode, PairingProof,
//! SafeText, PairLink, kind 18), Tests/checks/home-txt/main.swift (HomeDoorTXT), the plan's vectors
//! (docs/remote-access-plan.md 3.5-3.6) and Python/OpenSSL (tests/fixtures/python_vectors.json).
use sha2::{Digest, Sha256};
use sill_net::identity::*;
use sill_net::pairing::proof;
use sill_net::pairing::*;
use std::collections::BTreeMap;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/python_vectors.json")).unwrap()
}
fn sha(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}

// ---------------------------------------------------------------------------------------------
// PairingProof: the plan's vectors (Tests/checks/protocol:160-199)

#[test]
fn plan_vectors_code_path() {
    let fp_mac = sha("mac");
    let fp_dev = sha("device");
    assert_eq!(
        hex(&fp_mac),
        "348a629f5ceed032c3e8706ec47d9bfafb00fb4250b018dd965435ca50cb836e"
    );
    assert_eq!(
        hex(&fp_dev),
        "263a4dbe41488fb87214b0032339dbb9f0c8da14c16dfcf13084bf3c2552eca5"
    );
    let k = proof::code_key("482913557208", &fp_mac).unwrap();
    assert_eq!(
        hex(&k),
        "e9baf879a1cc4ac8b21554df480b6c1621671319107c91e7cb05d154f649d31d"
    );
    let d = proof::device_proof(&k, &fp_dev, &fp_mac);
    let m = proof::mac_proof(&k, &fp_mac, &fp_dev);
    assert_eq!(
        base64url_encode(&d),
        "dk7PZj4vHsAxj--1u9Vg7ehSn8a1tWc0RCG46YpFVO8"
    );
    assert_eq!(
        base64url_encode(&m),
        "6BmGWC9XT8J8OY8phhq9bASgoSnoWpRxBeqZtvRrWmk"
    );
}

#[test]
fn plan_vectors_qr_path() {
    let fp_mac = sha("mac");
    let fp_dev = sha("device");
    let k = proof::qr_key(&[0x22u8; 16]);
    let d = proof::device_proof(&k, &fp_dev, &fp_mac);
    let m = proof::mac_proof(&k, &fp_mac, &fp_dev);
    assert_eq!(
        base64url_encode(&d),
        "AyZAJqNTU5nSKEaHx6RZOf0K5dOeygR_8JSrCV_gNOo"
    );
    assert_eq!(
        base64url_encode(&m),
        "XAX-uZPsq_Ax8fvI_sV7Ex0a-ROi19rI-FW-ykPh9TE"
    );
}

#[test]
fn message_layout_is_label_zero_then_both_fingerprints() {
    // Hand-built: "sill-pair-v1 device" || 00 || fp_d || fp_m.
    let m = proof::device_message(&[1u8; 32], &[2u8; 32]);
    assert_eq!(&m[..19], b"sill-pair-v1 device");
    assert_eq!(m[19], 0);
    assert_eq!(&m[20..52], &[1u8; 32]);
    assert_eq!(&m[52..], &[2u8; 32]);
    let m = proof::mac_message(&[2u8; 32], &[1u8; 32]);
    assert_eq!(&m[..16], b"sill-pair-v1 mac");
    assert_eq!(m[16], 0);
    assert_eq!(&m[17..49], &[2u8; 32]);
    assert_eq!(&m[49..], &[1u8; 32]);
}

#[test]
fn proof_properties() {
    let fp_mac = sha("mac");
    let fp_dev = sha("device");
    let qr = proof::qr_key(&[0x22u8; 16]);
    let d = proof::device_proof(&qr, &fp_dev, &fp_mac);
    let m = proof::mac_proof(&qr, &fp_mac, &fp_dev);
    assert_ne!(d, m); // the labels differ
    assert_ne!(proof::device_proof(&qr, &fp_mac, &fp_dev), d); // swapped fingerprints
    assert_ne!(proof::mac_proof(&qr, &fp_dev, &fp_mac), m);
    assert_ne!(proof::device_proof(&qr, &fp_dev, &sha("relay")), d); // a relay's key in the Mac's place
    assert!(proof::is_valid_device_proof(&d, &qr, &fp_dev, &fp_mac));
    let mut flipped = d;
    flipped[5] ^= 1;
    assert!(!proof::is_valid_device_proof(
        &flipped, &qr, &fp_dev, &fp_mac
    ));
    assert!(!proof::is_valid_device_proof(&m, &qr, &fp_dev, &fp_mac)); // proof_M in proof_D's place
    assert!(proof::is_valid_mac_proof(&m, &qr, &fp_mac, &fp_dev));
    assert!(!proof::is_valid_mac_proof(&d, &qr, &fp_mac, &fp_dev));
    assert!(!proof::is_valid_mac_proof(&m[..31], &qr, &fp_mac, &fp_dev)); // truncated
    assert!(!proof::is_valid_mac_proof(&[], &qr, &fp_mac, &fp_dev));
    let other = proof::code_key("482913557217", &fp_mac).unwrap();
    let k = proof::code_key("482913557208", &fp_mac).unwrap();
    let mp = proof::mac_proof(&k, &fp_mac, &fp_dev);
    assert!(!proof::is_valid_mac_proof(&mp, &other, &fp_mac, &fp_dev));
}

#[test]
fn code_key_refuses_anything_but_twelve_ascii_digits() {
    let fp = sha("mac");
    assert!(proof::code_key("48291355720", &fp).is_none());
    assert!(proof::code_key("4829135572080", &fp).is_none());
    assert!(proof::code_key("48291355720a", &fp).is_none());
    assert!(proof::code_key("4829 1355 7208", &fp).is_none()); // callers pass damm::check's output
    assert!(proof::code_key("٤٨٢٩١٣٥٥٧٢٠٨", &fp).is_none());
    assert!(proof::code_key("", &fp).is_none());
}

#[test]
fn code_key_depends_on_the_mac_fingerprint() {
    let a = proof::code_key("482913557208", &sha("mac")).unwrap();
    let b = proof::code_key("482913557208", &sha("relay")).unwrap();
    assert_ne!(a, b);
}

#[test]
fn proofs_match_sillclient_py_byte_for_byte() {
    let f = fixtures();
    let mut n = 0;
    for p in f["proofs"].as_array().unwrap() {
        let fm = unhex(p["fp_mac"].as_str().unwrap());
        let fd = unhex(p["fp_dev"].as_str().unwrap());
        let key = unhex(p["key"].as_str().unwrap());
        if let Some(code) = p["code"].as_str() {
            assert_eq!(
                proof::code_key(code, &fm).unwrap().to_vec(),
                key,
                "{}",
                p["name"]
            );
        }
        assert_eq!(
            base64url_encode(&proof::device_proof(&key, &fd, &fm)),
            p["proof_d"].as_str().unwrap()
        );
        assert_eq!(
            base64url_encode(&proof::mac_proof(&key, &fm, &fd)),
            p["proof_m"].as_str().unwrap()
        );
        n += 1;
    }
    assert!(n >= 8);
}

// ---------------------------------------------------------------------------------------------
// RecognitionTag (plan 3.5: key 0x11*32, p = a1a2a3a4a5a6 gives oaKjpKWmL4V47Ctm)

#[test]
fn recognition_tag_plan_vector() {
    let key = [0x11u8; 32];
    let nonce = [0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6];
    assert_eq!(recognition_tag::make(&key, &nonce), "oaKjpKWmL4V47Ctm");
    assert!(recognition_tag::matches("oaKjpKWmL4V47Ctm", &key));
    assert!(!recognition_tag::matches("oaKjpKWmL4V47Ctm", &[0x12u8; 32]));
}

#[test]
fn recognition_tag_refusals() {
    let key = [0x11u8; 32];
    assert!(!recognition_tag::matches("", &key));
    assert!(!recognition_tag::matches("oaKjpKWmL4V47Ct", &key)); // 15 chars
    assert!(!recognition_tag::matches("oaKjpKWmL4V47Ctm=", &key)); // padding
    assert!(!recognition_tag::matches("oaKjpKWmL4V47Ctn", &key)); // last 6 bytes changed
    assert!(!recognition_tag::matches("oaKjpKWmL4V47Ctm/", &key));
    // 10,000 wrong keys never match (Swift check, plan H3).
    for i in 0u32..10_000 {
        let k = Sha256::digest(i.to_be_bytes());
        assert!(!recognition_tag::matches("oaKjpKWmL4V47Ctm", &k));
    }
    assert_eq!(recognition_tag::TXT_KEY, "r");
}

#[test]
fn recognition_tags_match_python() {
    for t in fixtures()["tags"].as_array().unwrap() {
        let key = unhex(t["key"].as_str().unwrap());
        let nonce: [u8; 6] = unhex(t["nonce"].as_str().unwrap()).try_into().unwrap();
        assert_eq!(
            recognition_tag::make(&key, &nonce),
            t["tag"].as_str().unwrap()
        );
        assert!(recognition_tag::matches(t["tag"].as_str().unwrap(), &key));
    }
}

// ---------------------------------------------------------------------------------------------
// HomeDoorTXT (Tests/checks/home-txt:30-60)

fn txt(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn home_door_dictionary_rules() {
    assert_eq!(HOME_DOOR_KEY, "p");
    assert_eq!(home_door_value(true), "1");
    assert_eq!(home_door_value(false), "0");
    assert_eq!(home_door(&txt(&[])), HomeDoor::Plain);
    assert_eq!(
        home_door(&txt(&[("r", "oaKjpKWmL4V47Ctm")])),
        HomeDoor::Plain
    );
    assert_eq!(
        home_door(&txt(&[("r", "t"), ("p", "1")])),
        HomeDoor::PairingRequired
    );
    assert_eq!(home_door(&txt(&[("r", "t"), ("p", "0")])), HomeDoor::Open);
    for v in [
        "", "2", "yes", "true", "no", "false", " 0", "0 ", "00", "O", "-0", "0x0", "\u{0660}",
    ] {
        assert_eq!(
            home_door(&txt(&[("p", v)])),
            HomeDoor::PairingRequired,
            "{v:?}"
        );
    }
    assert_eq!(home_door(&txt(&[("P", "0")])), HomeDoor::Open);
    assert_eq!(home_door(&txt(&[("P", "1")])), HomeDoor::PairingRequired);
    assert_eq!(
        home_door(&txt(&[("p", "1"), ("P", "0")])),
        HomeDoor::PairingRequired
    ); // p before P
    assert_eq!(home_door(&txt(&[("p", "0"), ("P", "1")])), HomeDoor::Open);
    assert_eq!(
        home_door(&txt(&[("q", "0"), ("pp", "0"), ("r", "0")])),
        HomeDoor::Plain
    );
    for rp in [true, false] {
        let want = if rp {
            HomeDoor::PairingRequired
        } else {
            HomeDoor::Open
        };
        assert_eq!(home_door(&txt(&[("p", home_door_value(rp))])), want);
    }
}

#[test]
fn home_door_entry_rules() {
    assert_eq!(home_door_entry(None), HomeDoor::Plain);
    assert_eq!(home_door_entry(Some(None)), HomeDoor::PairingRequired); // bare `p`: reads as 1
    assert_eq!(home_door_entry(Some(Some(b""))), HomeDoor::PairingRequired); // `p=`
    assert_eq!(home_door_entry(Some(Some(b"0"))), HomeDoor::Open);
    assert_eq!(home_door_entry(Some(Some(b"1"))), HomeDoor::PairingRequired);
    assert_eq!(home_door_entry(Some(Some(b"2"))), HomeDoor::PairingRequired);
    assert_eq!(
        home_door_entry(Some(Some(&[0xff, 0xfe]))),
        HomeDoor::PairingRequired
    );
}

// ---------------------------------------------------------------------------------------------
// SafeText (Tests/checks/protocol:254-271)

#[test]
fn safe_label_cases() {
    let l = |s: &str| safe_label(s, LABEL_LIMIT);
    assert_eq!(l("iPad (iPad14,1)"), "iPad (iPad14,1)");
    assert_eq!(l("iPad\n\tPro"), "iPad Pro");
    assert_eq!(l("iPad\nPro"), "iPad Pro");
    assert_eq!(l("iPad\u{2028}mini"), "iPad mini");
    assert!(!l("iPad\nClient connected: 1.2.3.4").contains('\n'));
    assert_eq!(l("iPad\u{202E}dáPi"), "iPaddáPi");
    assert_eq!(
        l("a\u{2066}b\u{2067}c\u{2068}d\u{2069}e\u{202A}f\u{202B}g\u{202C}h\u{202D}i"),
        "abcdefghi"
    );
    assert_eq!(l("i\u{200B}Pa\u{FEFF}d"), "iPad");
    assert_eq!(l("i\u{0007}P\u{001B}[31ma\u{0085}d\u{009B}"), "iP[31ma d");
    assert_eq!(l("  iPad \u{00A0}\u{3000}  mini  "), "iPad mini");
    assert_eq!(l(&"M".repeat(500)).chars().count(), 64);
    assert_eq!(l(&"é".repeat(70)).chars().count(), 64);
    assert_eq!(l("Noah’s iPad 🍎"), "Noah’s iPad 🍎");
    assert_eq!(l("\u{202E}\u{0000}\n "), "");
    assert_eq!(l("i\u{E000}Pad"), "iPad");
    // Truncation never leaves a trailing space, and keeps a letter with its combining mark whole.
    assert_eq!(safe_label("abc def", 4), "abc");
    assert_eq!(safe_label("e\u{0301}e\u{0301}x", 2), "e\u{0301}e\u{0301}");
}

// ---------------------------------------------------------------------------------------------
// Address parser (Tests/checks/protocol:23-125)

fn ok(input: &str, host: &str, port: Option<u16>, kind: AddressKind) {
    let got = parse_address(input).unwrap_or_else(|e| panic!("{input:?} refused {e:?}"));
    assert_eq!(
        (got.host.as_str(), got.port, got.kind),
        (host, port, kind),
        "{input:?}"
    );
}
fn bad(input: &str, f: AddressFailure) {
    assert_eq!(parse_address(input), Err(f), "{input:?}");
}

#[test]
fn address_trimming_and_whitespace() {
    use AddressKind::*;
    ok(" 100.1.1.1 ", "100.1.1.1", None, Ipv4);
    ok("\t100.101.102.103\n", "100.101.102.103", None, Ipv4);
    bad("100.1 .1.1", AddressFailure::Malformed);
    bad("mac mini.local", AddressFailure::Malformed);
    bad("mac\tmini", AddressFailure::Malformed);
    bad("", AddressFailure::Empty);
    bad("   ", AddressFailure::Empty);
}

#[test]
fn address_ipv6() {
    use AddressKind::*;
    ok("[fd7a:115c:a1e0::1234]", "fd7a:115c:a1e0::1234", None, Ipv6);
    ok(
        "[fd7a:115c:a1e0::1234]:7455",
        "fd7a:115c:a1e0::1234",
        Some(7455),
        Ipv6,
    );
    ok(
        "[FD7A:115C:A1E0:0:0:0:0:1234]:17455",
        "fd7a:115c:a1e0::1234",
        Some(17455),
        Ipv6,
    );
    ok("[::1]:51234", "::1", Some(51234), Ipv6);
    bad("[1.2.3.4]", AddressFailure::Ipv6);
    bad("[::1", AddressFailure::Ipv6);
    bad("[::1]x", AddressFailure::Malformed);
    bad("[::1]:", AddressFailure::Port);
    bad("[::1]:99999", AddressFailure::Port);
    ok("fd7a::1:7455", "fd7a::1:7455", None, Ipv6); // never address + port
    ok("::1", "::1", None, Ipv6);
    ok("2001:db8::20", "2001:db8::20", None, Ipv6);
    bad("2001:db8:::20", AddressFailure::Ipv6);
    bad("gg::1", AddressFailure::Ipv6);
}

#[test]
fn address_zones_refused() {
    bad("fe80::1%en0", AddressFailure::Zone);
    bad("[fe80::1%en0]:7455", AddressFailure::Zone);
    bad("fe80::1%25en0", AddressFailure::Zone);
}

#[test]
fn address_ipv4_mapped_is_ipv4() {
    use AddressKind::*;
    ok("::ffff:1.2.3.4", "1.2.3.4", None, Ipv4);
    ok(
        "[::ffff:100.101.102.103]:7455",
        "100.101.102.103",
        Some(7455),
        Ipv4,
    );
    ok("::ffff:0102:0304", "1.2.3.4", None, Ipv4);
}

#[test]
fn address_ipv4_strictness() {
    use AddressKind::*;
    ok("100.101.102.103", "100.101.102.103", None, Ipv4);
    ok("100.101.102.103:7455", "100.101.102.103", Some(7455), Ipv4);
    ok("0.0.0.0", "0.0.0.0", None, Ipv4);
    ok("255.255.255.255", "255.255.255.255", None, Ipv4);
    for s in [
        "1.2.3",
        "100.1",
        "0x64.0x65.1.2",
        "0x7f000001",
        "4294967295",
        "256.1.1.1",
        "01.2.3.4",
        "017.0.0.1",
        "1.2.3.4.5",
        "1.2.3.4.",
    ] {
        bad(s, AddressFailure::Ipv4);
    }
    bad("1.2.3.4:0", AddressFailure::Port);
}

#[test]
fn address_names() {
    use AddressKind::*;
    ok(
        "mac-mini.tail1234.ts.net",
        "mac-mini.tail1234.ts.net",
        None,
        Name,
    );
    ok(
        "home.example.net:17455",
        "home.example.net",
        Some(17455),
        Name,
    );
    ok("localhost", "localhost", None, Name);
    ok("mac-mini", "mac-mini", None, Name);
    ok("home.example.net.", "home.example.net.", None, Name);
    ok("1e100.net", "1e100.net", None, Name);
    ok("123.example", "123.example", None, Name);
    for s in [
        "-bad.example",
        "bad-.example",
        "a..b",
        "under_score.example",
        "exämple.com",
    ] {
        bad(s, AddressFailure::Name);
    }
    let l63 = format!("{}.net", "a".repeat(63));
    ok(&l63, &l63, None, Name);
    bad(&format!("{}.net", "a".repeat(64)), AddressFailure::Name);
    let long253 = format!("{}.d", vec!["abc"; 63].join("."));
    assert_eq!(long253.len(), 253);
    ok(&long253, &long253, None, Name);
    bad(&format!("{long253}e"), AddressFailure::Name);
}

#[test]
fn address_ports() {
    use AddressKind::*;
    ok("host:1", "host", Some(1), Name);
    ok("host:65535", "host", Some(65535), Name);
    for s in ["host:0", "host:65536", "host:+80", "host:80a", "host:"] {
        bad(s, AddressFailure::Port);
    }
    bad(":7455", AddressFailure::Malformed);
    bad("http://1.2.3.4", AddressFailure::Port);
}

#[test]
fn address_pairing_links_are_handed_to_pairlink() {
    bad("sill://pair?v=1", AddressFailure::PairingLink);
    bad("SILL://PAIR?v=1", AddressFailure::PairingLink);
}

#[test]
fn address_text_round_trips() {
    for s in [
        "[fd7a:115c:a1e0::1234]:7455",
        "100.101.102.103",
        "home.example.net:17455",
        "[::1]",
    ] {
        assert_eq!(parse_address(s).unwrap().text(), s);
    }
    let v6 = ParsedAddress {
        host: "fd7a::1".into(),
        port: Some(7455),
        kind: AddressKind::Ipv6,
    };
    assert_eq!(v6.text(), "[fd7a::1]:7455");
    let v4 = ParsedAddress {
        host: "100.1.2.3".into(),
        port: None,
        kind: AddressKind::Ipv4,
    };
    assert_eq!(v4.text(), "100.1.2.3");
}

// ---------------------------------------------------------------------------------------------
// PairLink (Tests/checks/protocol:273-331)

fn mac_fp() -> Vec<u8> {
    unhex(fixtures()["identities"][0]["fp"].as_str().unwrap())
}
fn secret() -> Vec<u8> {
    (0u8..16).collect()
}
fn pct(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || "-._~:".contains(c) {
            o.push(c);
        } else {
            let mut b = [0u8; 4];
            for x in c.encode_utf8(&mut b).bytes() {
                o.push_str(&format!("%{x:02X}"));
            }
        }
    }
    o
}
fn link_with(f: impl FnOnce(&mut Vec<(String, String)>)) -> String {
    let fp = mac_fp();
    let mut items: Vec<(String, String)> = [
        ("v", "1".to_string()),
        ("m", mac_id(&fp)),
        ("k", base64url_encode(&fp)),
        ("s", base64url_encode(&secret())),
        ("n", "Mac mini".to_string()),
        ("p", "7455".to_string()),
        ("a", "100.101.102.103".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    f(&mut items);
    let q: Vec<String> = items
        .iter()
        .map(|(k, v)| format!("{k}={}", pct(v)))
        .collect();
    format!("sill://pair?{}", q.join("&"))
}

#[test]
fn link_round_trip() {
    let v6 = ParsedAddress {
        host: "fd7a:115c:a1e0::1234".into(),
        port: None,
        kind: AddressKind::Ipv6,
    };
    let addrs = vec![
        ParsedAddress {
            host: "mac-mini.tail1234.ts.net".into(),
            port: None,
            kind: AddressKind::Name,
        },
        ParsedAddress {
            host: "100.101.102.103".into(),
            port: None,
            kind: AddressKind::Ipv4,
        },
        v6.clone(),
        ParsedAddress {
            host: "192.168.1.20".into(),
            port: None,
            kind: AddressKind::Ipv4,
        },
        ParsedAddress {
            host: "home.example.net".into(),
            port: Some(17455),
            kind: AddressKind::Name,
        },
    ];
    let link = PairLink::new(&mac_fp(), &secret(), "Tom & Jerry’s Mac=mini?", 7455, addrs);
    let url = link.url();
    assert!(url.len() < 300, "{} bytes", url.len());
    let back = PairLink::parse(&url).unwrap();
    assert_eq!(back, link);
    assert_eq!(back.addresses[2], v6);
    assert_eq!(back.mac_id, mac_id(&mac_fp()));
}

#[test]
fn link_url_escapes_like_swift() {
    let l = PairLink::new(&mac_fp(), &secret(), "A&B=C d", 7455, vec![]);
    let u = l.url();
    assert!(u.contains("&n=A%26B%3DC%20d&"), "{u}");
    let l6 = PairLink::new(
        &mac_fp(),
        &secret(),
        "x",
        7455,
        vec![ParsedAddress {
            host: "fd7a::1".into(),
            port: Some(7455),
            kind: AddressKind::Ipv6,
        }],
    );
    assert!(l6.url().ends_with("&a=%5Bfd7a::1%5D:7455"), "{}", l6.url());
}

#[test]
fn link_baseline_parses() {
    assert!(PairLink::parse(&link_with(|_| {})).is_ok());
    assert!(PairLink::parse(&format!("  {}\n", link_with(|_| {}))).is_ok()); // trimmed
    assert!(PairLink::parse(&link_with(|_| {}).replace("sill://pair", "SILL://Pair")).is_ok());
}

fn refused(name: &str, text: &str, want: LinkFailure) {
    assert_eq!(PairLink::parse(text), Err(want), "{name}");
}

#[test]
fn link_refusals() {
    use LinkFailure::*;
    let fp = mac_fp();
    refused("v=2", &link_with(|i| i[0].1 = "2".into()), Version);
    refused(
        "no v",
        &link_with(|i| {
            i.remove(0);
        }),
        Version,
    );
    refused(
        "no m",
        &link_with(|i| {
            i.remove(1);
        }),
        Field("m".into()),
    );
    refused(
        "m of another key",
        &link_with(|i| i[1].1 = "A3C5HR4RBV67YR21".into()),
        MacIdMismatch,
    );
    refused(
        "m lower case",
        &link_with(|i| i[1].1 = i[1].1.to_lowercase()),
        Field("m".into()),
    );
    refused(
        "no k",
        &link_with(|i| {
            i.remove(2);
        }),
        Field("k".into()),
    );
    let k31 = base64url_encode(&fp[..31]);
    refused(
        "k of 31 bytes",
        &link_with(|i| i[2].1 = k31),
        Field("k".into()),
    );
    let k = base64url_encode(&fp);
    refused(
        "two k",
        &link_with(|i| i.push(("k".into(), k))),
        Field("k".into()),
    );
    refused(
        "no s",
        &link_with(|i| {
            i.remove(3);
        }),
        Field("s".into()),
    );
    let s15 = base64url_encode(&secret()[..15]);
    refused(
        "s of 15 bytes",
        &link_with(|i| i[3].1 = s15),
        Field("s".into()),
    );
    refused(
        "no p",
        &link_with(|i| {
            i.remove(5);
        }),
        Field("p".into()),
    );
    refused(
        "p=0",
        &link_with(|i| i[5].1 = "0".into()),
        Field("p".into()),
    );
    refused(
        "p=65536",
        &link_with(|i| i[5].1 = "65536".into()),
        Field("p".into()),
    );
    refused(
        "p=007455 (six digits)",
        &link_with(|i| i[5].1 = "007455".into()),
        Field("p".into()),
    );
    refused(
        "an address the parser refuses (1.2.3)",
        &link_with(|i| i.push(("a".into(), "1.2.3".into()))),
        Address("1.2.3".into()),
    );
    refused(
        "a zone",
        &link_with(|i| i.push(("a".into(), "fe80::1%en0".into()))),
        Address("fe80::1%en0".into()),
    );
    refused("http scheme", "http://pair?v=1", NotALink);
    refused(
        "another host",
        &link_with(|_| {}).replace("sill://pair", "sill://evil"),
        NotALink,
    );
    refused(
        "a path",
        &link_with(|_| {}).replace("sill://pair?", "sill://pair/x?"),
        NotALink,
    );
    refused("no query", "sill://pair", NotALink);
    refused("bad percent escape", "sill://pair?v=%zz", NotALink);
    refused("empty v", "sill://pair?v=", Version);
}

#[test]
fn link_unknown_params_and_limits() {
    let l = PairLink::parse(&link_with(|i| {
        i.push(("x".into(), "ignored".into()));
        i.push(("zz".into(), "".into()));
    }))
    .unwrap();
    assert_eq!(l.addresses.len(), 1);
    let l = PairLink::parse(&link_with(|i| {
        for n in 1..=7 {
            i.push(("a".into(), format!("10.0.0.{n}")));
        }
    }))
    .unwrap();
    assert_eq!(l.addresses.len(), 5);
    assert_eq!(l.addresses[0].host, "100.101.102.103");
    let l = PairLink::parse(&link_with(|i| {
        i[4].1 = format!("{}\u{202E}", "M".repeat(100))
    }))
    .unwrap();
    assert_eq!(l.name.chars().count(), 64);
    assert!(!l.name.contains('\u{202E}'));
    let l = PairLink::parse(&link_with(|i| {
        i.remove(4);
    }))
    .unwrap();
    assert_eq!(l.name, "Mac");
    let l = PairLink::parse(&link_with(|i| i[4].1 = "\u{202E}\n".into())).unwrap();
    assert_eq!(l.name, "Mac");
}

#[test]
fn links_match_sillclient_py_parse_link() {
    for l in fixtures()["links"].as_array().unwrap() {
        let got = PairLink::parse(l["url"].as_str().unwrap()).unwrap();
        assert_eq!(hex(&got.fingerprint), l["fp"].as_str().unwrap());
        assert_eq!(hex(&got.secret), l["secret"].as_str().unwrap());
        assert_eq!(got.port as u64, l["port"].as_u64().unwrap());
        assert_eq!(got.mac_id, l["macID"].as_str().unwrap());
        assert_eq!(got.name, safe_label(l["name"].as_str().unwrap(), 64));
        let want: Vec<String> = l["addresses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        let have: Vec<String> = got.addresses.iter().map(|a| a.text()).collect();
        assert_eq!(have, want);
        // And the URL Rust writes is one Python's parser accepts with the same fields (same escape rule).
        assert_eq!(got.url(), l["url"].as_str().unwrap());
    }
}

// ---------------------------------------------------------------------------------------------
// kind 18 (Tests/checks/protocol:333-350; sillclient.py verify_macinfo)

fn info_json(id: &str) -> Vec<u8> {
    format!(
        r#"{{"v":1,"macID":"{id}","name":"Mac mini","issuedAt":1790265600.5,"remoteAccess":true,"remotePort":7455,"internet":false,"addresses":[{{"host":"100.101.102.103","kind":"vpn","via":"Tailscale"}}]}}"#
    )
    .into_bytes()
}

#[test]
fn kind18_round_trip_and_tampering() {
    let key = DeviceIdentity::generate();
    let json = info_json(&key.mac_id());
    let (info, k, sig) = sign_mac_info(&json, &key);
    let v = verify_signed_mac_info(&info, &k, &sig).unwrap();
    assert_eq!(v.info_json, json);
    assert_eq!(v.fingerprint, key.fingerprint());
    assert_eq!(v.mac_id, key.mac_id());

    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut ib = STANDARD.decode(&info).unwrap();
    ib[10] ^= 1;
    assert_eq!(
        verify_signed_mac_info(&STANDARD.encode(&ib), &k, &sig),
        Err(MacInfoFailure::BadSignature)
    );
    let mut sb = STANDARD.decode(&sig).unwrap();
    let n = sb.len();
    sb[n - 3] ^= 1;
    assert!(verify_signed_mac_info(&info, &k, &STANDARD.encode(&sb)).is_err());
    let other = DeviceIdentity::generate();
    assert_eq!(
        verify_signed_mac_info(&info, &STANDARD.encode(other.public_point()), &sig),
        Err(MacInfoFailure::BadSignature)
    );
    // Signed by another key, with its own key field: the signature checks but the macID binding fails.
    let (i2, k2, s2) = sign_mac_info(&json, &other);
    assert_eq!(
        verify_signed_mac_info(&i2, &k2, &s2),
        Err(MacInfoFailure::MacIdMismatch)
    );
}

#[test]
fn kind18_malformed_fields() {
    let key = DeviceIdentity::generate();
    let (info, k, sig) = sign_mac_info(&info_json(&key.mac_id()), &key);
    use base64::{engine::general_purpose::STANDARD, Engine};
    assert_eq!(
        verify_signed_mac_info("!!!", &k, &sig),
        Err(MacInfoFailure::Malformed)
    );
    assert_eq!(
        verify_signed_mac_info(&info, &STANDARD.encode([4u8; 64]), &sig),
        Err(MacInfoFailure::Malformed)
    );
    let mut compressed = key.public_point()[..33].to_vec();
    compressed[0] = 2;
    assert_eq!(
        verify_signed_mac_info(&info, &STANDARD.encode(&compressed), &sig),
        Err(MacInfoFailure::Malformed)
    );
    assert_eq!(
        verify_signed_mac_info(&info, &k, "AAAA"),
        Err(MacInfoFailure::Malformed)
    ); // not DER
       // Unpadded base64 is not standard-padded base64 (Swift's Data(base64Encoded:) refuses it).
    assert_eq!(
        verify_signed_mac_info(info.trim_end_matches('='), &k, &sig).is_err(),
        info.ends_with('=')
    );
    // Signed bytes that are not JSON, or JSON without a macID.
    let (i3, k3, s3) = sign_mac_info(b"not json", &key);
    assert_eq!(
        verify_signed_mac_info(&i3, &k3, &s3),
        Err(MacInfoFailure::BadInfo)
    );
    let (i4, k4, s4) = sign_mac_info(br#"{"v":1}"#, &key);
    assert_eq!(
        verify_signed_mac_info(&i4, &k4, &s4),
        Err(MacInfoFailure::BadInfo)
    );
}

#[test]
fn kind18_signed_by_openssl_verifies_in_rust() {
    for k in fixtures()["kind18"].as_array().unwrap() {
        let v = verify_signed_mac_info(
            k["info"].as_str().unwrap(),
            k["key"].as_str().unwrap(),
            k["sig"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(v.info_json, unhex(k["info_hex"].as_str().unwrap()));
        assert_eq!(hex(&v.fingerprint), k["fp"].as_str().unwrap());
        assert_eq!(v.mac_id, k["macid"].as_str().unwrap());
        // Any flipped bit of the signed bytes fails.
        let mut bad = unhex(k["info_hex"].as_str().unwrap());
        bad[3] ^= 0x01;
        use base64::{engine::general_purpose::STANDARD, Engine};
        assert!(verify_signed_mac_info(
            &STANDARD.encode(bad),
            k["key"].as_str().unwrap(),
            k["sig"].as_str().unwrap()
        )
        .is_err());
    }
}

#[test]
fn rust_signatures_verify_with_openssl_semantics() {
    // A signature Rust makes is DER ECDSA-SHA256 over the exact bytes, as `openssl dgst -verify` reads
    // it: tag 0x30, two INTEGERs.
    let key = DeviceIdentity::generate();
    let sig = key.sign_der(b"abc");
    assert_eq!(sig[0], 0x30);
    assert_eq!(sig[2], 0x02);
}
