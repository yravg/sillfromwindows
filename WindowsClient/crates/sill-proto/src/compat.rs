//! Sources/StreamProtocol/Compatibility.swift: kind 23 (`Hello`), `SillProtocol`, `SillVersion`.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

/// `SillProtocol.current` (Compatibility.swift:31-35).
pub const PROTOCOL_CURRENT: i64 = 1;

/// `Hello` (Compatibility.swift:104-124), kind 23: the first message of every session connection.
/// `audio` (codecs, best first) is required for the host to send sound.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<Vec<String>>,
}

/// `SillVersion` (Compatibility.swift:46-100): tags, bundle versions and the wire's versions,
/// compared part by part ("0.10" > "0.9"); trailing zero parts do not count.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SillVersion {
    components: Vec<u64>,
}

impl SillVersion {
    pub fn from_components(mut c: Vec<u64>) -> Self {
        while c.last() == Some(&0) {
            c.pop();
        }
        SillVersion { components: c }
    }

    pub fn zero() -> Self {
        Self::from_components(vec![])
    }

    pub fn components(&self) -> &[u64] {
        &self.components
    }

    /// `init?(_ text:)`: trim whitespace, drop one leading "v"/"V", take the leading run of ASCII
    /// digits and dots, drop trailing dots; at most 8 parts of 1-9 ASCII digits each.
    pub fn parse(text: &str) -> Option<SillVersion> {
        let trimmed = text.trim();
        let rest = trimmed.strip_prefix(['v', 'V']).unwrap_or(trimmed);
        let mut prefix: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        while prefix.ends_with('.') {
            prefix.pop();
        }
        if !prefix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        let parts: Vec<&str> = prefix.split('.').collect();
        if parts.len() > 8 {
            return None;
        }
        let mut numbers = Vec::new();
        for p in parts {
            if !(1..=9).contains(&p.len()) {
                return None;
            }
            numbers.push(p.parse().ok()?);
        }
        Some(Self::from_components(numbers))
    }
}

impl fmt::Display for SillVersion {
    /// At least two parts: "1" is "1.0".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut c = self.components.clone();
        while c.len() < 2 {
            c.push(0);
        }
        let s: Vec<String> = c.iter().map(u64::to_string).collect();
        write!(f, "{}", s.join("."))
    }
}

impl PartialOrd for SillVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SillVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.components.len().max(other.components.len());
        for i in 0..n {
            let x = self.components.get(i).copied().unwrap_or(0);
            let y = other.components.get(i).copied().unwrap_or(0);
            if x != y {
                return x.cmp(&y);
            }
        }
        Ordering::Equal
    }
}
