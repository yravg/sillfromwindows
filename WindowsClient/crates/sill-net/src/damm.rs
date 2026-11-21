//! The 12-digit typed pairing code and its Damm check digit.
//!
//! Port of `PairingCode` (Sources/StreamProtocol/Pairing.swift:14-60). Eleven random digits, then one
//! Damm check digit, so a typo is caught on the device and never costs one of the Mac's five tries.
//! The code is shown as "4829 1355 7208".

/// Damm's quasigroup of order 10 (Pairing.swift:19-25; the same table is in Scripts/sillclient.py
/// `DAMM`). `interim = TABLE[interim][digit]`, starting at 0.
pub const TABLE: [[u8; 10]; 10] = [
    [0, 3, 1, 7, 5, 9, 8, 6, 4, 2],
    [7, 0, 9, 2, 1, 5, 4, 8, 6, 3],
    [4, 2, 0, 6, 8, 7, 1, 3, 5, 9],
    [1, 7, 5, 0, 9, 8, 3, 4, 2, 6],
    [6, 1, 2, 3, 0, 4, 5, 9, 7, 8],
    [3, 6, 7, 4, 2, 0, 9, 5, 8, 1],
    [5, 8, 6, 9, 7, 2, 0, 1, 3, 4],
    [8, 9, 4, 5, 3, 6, 2, 0, 1, 7],
    [9, 4, 3, 8, 6, 1, 7, 2, 0, 5],
    [2, 5, 8, 1, 4, 3, 6, 7, 9, 0],
];

/// Digits in a code (Pairing.swift:27).
pub const LENGTH: usize = 12;

/// Why typed input is not a code yet (Pairing.swift:46-51). The device says so locally and sends
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// "A code has 12 digits."
    Length,
    /// "That code has a typo. Check it against your Mac."
    Typo,
}

/// The check digit for `digits` (each 0-9): the final interim (Pairing.swift:30-32). A digit above 9
/// is a caller bug and panics.
pub fn check_digit(digits: &[u8]) -> u8 {
    digits
        .iter()
        .fold(0u8, |interim, &d| TABLE[interim as usize][d as usize])
}

/// A fresh code from a source of random bytes (Pairing.swift:37-44): 11 digits by rejection
/// sampling (a byte of 250 or more is drawn again, so every digit is equally likely), then the check
/// digit. `fill` must fill the slice with random bytes (the caller supplies the OS source; this
/// module is pure).
pub fn generate_with(mut fill: impl FnMut(&mut [u8])) -> String {
    let mut digits: Vec<u8> = Vec::with_capacity(LENGTH);
    while digits.len() < LENGTH - 1 {
        let mut bytes = [0u8; 16];
        fill(&mut bytes);
        for &b in &bytes {
            if b < 250 && digits.len() < LENGTH - 1 {
                digits.push(b % 10);
            }
        }
    }
    digits.push(check_digit(&digits));
    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// `input` as a code: spaces, hyphens and U+00A0 removed, exactly 12 ASCII digits, the check digit
/// valid (Pairing.swift:53-58). Returns the 12 digits.
pub fn check(input: &str) -> Result<String, Problem> {
    let kept: Vec<char> = input
        .chars()
        .filter(|&c| c != ' ' && c != '-' && c != '\u{00A0}')
        .collect();
    if kept.len() != LENGTH || !kept.iter().all(|c| c.is_ascii_digit()) {
        return Err(Problem::Length);
    }
    let digits: Vec<u8> = kept.iter().map(|c| *c as u8 - b'0').collect();
    if check_digit(&digits) != 0 {
        return Err(Problem::Typo);
    }
    Ok(kept.into_iter().collect())
}

/// "4829 1355 7208" (Pairing.swift:60-65): groups of four.
pub fn grouped(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}
