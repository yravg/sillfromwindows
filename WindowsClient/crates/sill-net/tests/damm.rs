//! Cases from Tests/checks/protocol/main.swift ("PairingCode (Damm)") and the plan's vectors
//! (docs/remote-access-plan.md section 3.6).
use p256::elliptic_curve::rand_core::{OsRng, RngCore};
use sill_net::damm::{self, Problem};

fn digits(s: &str) -> Vec<u8> {
    s.bytes().map(|b| b - b'0').collect()
}

#[test]
fn table_is_a_quasigroup_with_zero_diagonal() {
    // Damm's table must be a Latin square and weakly totally anti-symmetric; check the Latin square
    // and the zero diagonal (hand-checked rows from Pairing.swift:19-25).
    for i in 0..10 {
        assert_eq!(damm::TABLE[i][i], 0, "diagonal at {i}");
        let mut row: Vec<u8> = damm::TABLE[i].to_vec();
        row.sort();
        assert_eq!(row, (0..10).collect::<Vec<u8>>());
        let mut col: Vec<u8> = (0..10).map(|r| damm::TABLE[r][i]).collect();
        col.sort();
        assert_eq!(col, (0..10).collect::<Vec<u8>>());
    }
}

#[test]
fn plan_example_gets_check_digit_8() {
    // The plan's example (docs/remote-access-plan.md 3.6): 4829 1355 720 gets check digit 8.
    assert_eq!(damm::check_digit(&digits("48291355720")), 8);
    assert_eq!(damm::check_digit(&digits("482913557208")), 0);
}

#[test]
fn hand_computed_tiny_cases() {
    assert_eq!(damm::check_digit(&[]), 0);
    assert_eq!(damm::check_digit(&[0]), 0);
    assert_eq!(damm::check_digit(&[1]), 3); // T[0][1]
    assert_eq!(damm::check_digit(&[5, 7]), 7); // T[0][5]=9, T[9][7]=7
    assert_eq!(damm::check_digit(&[1, 3]), 0); // T[0][1]=3, T[3][3]=0: a valid 2-digit code
}

#[test]
fn every_substitution_and_adjacent_transposition_is_caught() {
    let mut codes = 0;
    for _ in 0..2_000 {
        let mut d: Vec<u8> = (0..11).map(|_| (OsRng.next_u32() % 10) as u8).collect();
        let c = damm::check_digit(&d);
        d.push(c);
        codes += 1;
        assert_eq!(damm::check_digit(&d), 0);
        for p in 0..12 {
            for v in 0..10u8 {
                if v != d[p] {
                    let mut e = d.clone();
                    e[p] = v;
                    assert_ne!(damm::check_digit(&e), 0);
                }
            }
        }
        for p in 0..11 {
            if d[p] != d[p + 1] {
                let mut e = d.clone();
                e.swap(p, p + 1);
                assert_ne!(damm::check_digit(&e), 0);
            }
        }
    }
    assert_eq!(codes, 2_000);
}

#[test]
fn generate_makes_valid_uniform_codes() {
    let mut counts = [0f64; 10];
    for _ in 0..5_000 {
        let c = damm::generate_with(|b| OsRng.fill_bytes(b));
        assert_eq!(c.len(), 12);
        assert!(damm::check(&c).is_ok(), "{c}");
        for ch in c.chars().take(11) {
            counts[(ch as u8 - b'0') as usize] += 1.0;
        }
    }
    let expected = 5_000.0 * 11.0 / 10.0;
    let chi: f64 = counts
        .iter()
        .map(|c| (c - expected).powi(2) / expected)
        .sum();
    assert!(chi < 27.9, "chi2 {chi}"); // 9 df, p 0.001 (as the Swift check)
}

#[test]
fn rejection_sampling_skips_bytes_of_250_or_more() {
    // 255 would bias; a stream of 250..=255 then 0..=9 must yield only the 0..=9 ones.
    let mut calls = 0;
    let code = damm::generate_with(|b| {
        calls += 1;
        for (i, x) in b.iter_mut().enumerate() {
            *x = if i < 6 { 250 + i as u8 } else { (i - 6) as u8 };
        }
    });
    assert!(calls >= 2);
    assert!(damm::check(&code).is_ok());
    assert_eq!(&code[..10], "0123456789");
}

#[test]
fn check_accepts_separators() {
    assert_eq!(damm::check("4829 1355 7208").as_deref(), Ok("482913557208"));
    assert_eq!(damm::check("4829-1355-7208").as_deref(), Ok("482913557208"));
    assert_eq!(
        damm::check("4829\u{00A0}1355\u{00A0}7208").as_deref(),
        Ok("482913557208")
    );
}

#[test]
fn check_rejects_with_the_right_reason() {
    assert_eq!(damm::check("4829 1355 720"), Err(Problem::Length));
    assert_eq!(damm::check("4829 1355 72080"), Err(Problem::Length));
    assert_eq!(damm::check("4829 1355 72O8"), Err(Problem::Length));
    assert_eq!(damm::check("٤٨٢٩١٣٥٥٧٢٠٨"), Err(Problem::Length));
    assert_eq!(damm::check(""), Err(Problem::Length));
    assert_eq!(damm::check("4829 1355 7209"), Err(Problem::Typo));
    assert_eq!(damm::check("4829 1355 7280"), Err(Problem::Typo));
}

#[test]
fn grouped_in_fours() {
    assert_eq!(damm::grouped("482913557208"), "4829 1355 7208");
    assert_eq!(damm::grouped("12345"), "1234 5");
    assert_eq!(damm::grouped(""), "");
}

/// Rust == Python (Scripts/sillclient.py `damm`), fixtures from tests/fixtures/gen_fixtures.py.
#[test]
fn matches_sillclient_py() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/python_vectors.json")).unwrap();
    let cases = v["damm"].as_array().unwrap();
    assert!(cases.len() >= 9);
    for c in cases {
        let d = digits(c["digits"].as_str().unwrap());
        assert_eq!(
            damm::check_digit(&d) as u64,
            c["check"].as_u64().unwrap(),
            "{:?}",
            c
        );
    }
}
