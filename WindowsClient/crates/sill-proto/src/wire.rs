//! `Wire` (Sources/StreamProtocol/Switcher.swift:180-188): plain `JSONEncoder`/`JSONDecoder`.

use serde::{de::DeserializeOwned, Serialize};

/// `Wire.encode`: the JSON bytes, or empty when the value cannot be encoded (Swift: `try?` then
/// `Data()`).
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

/// `Wire.decode`: `None` when the bytes do not decode as `T` (missing required field, wrong type,
/// not JSON). Unknown keys are ignored.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    serde_json::from_slice(bytes).ok()
}

/// `Wire.decode` for a payload that is a JSON object (every kind but 5, the app list): Swift's
/// decoder refuses an array or any other top-level value for a struct, while serde would read a
/// JSON array as a struct's fields in order, so an all-optional struct such as `Hello` would
/// accept `[]`. Prefer this over [`decode`] for every object payload.
pub fn decode_object<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    let first = bytes.iter().find(|b| !b.is_ascii_whitespace())?;
    if *first != b'{' {
        return None;
    }
    decode(bytes)
}

/// serde adapter for Swift `Data?`: standard base64 with padding (Foundation's `Data` Codable).
/// Decoding is padding-tolerant. Use as `#[serde(with = "crate::wire::b64_opt", default, skip_serializing_if = "Option::is_none")]`.
pub mod b64_opt {
    use base64::{
        alphabet,
        engine::{general_purpose::GeneralPurpose, DecodePaddingMode, GeneralPurposeConfig},
        Engine,
    };
    use serde::{Deserialize, Deserializer, Serializer};

    const LENIENT: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        match text {
            None => Ok(None),
            Some(t) => LENIENT
                .decode(t.as_bytes())
                .map(Some)
                .map_err(serde::de::Error::custom),
        }
    }
}

/// Standard base64 (padded on encode, padding-tolerant on decode), for `SignedMacInfo`'s fields.
pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::{
        alphabet,
        engine::{general_purpose::GeneralPurpose, DecodePaddingMode, GeneralPurposeConfig},
        Engine,
    };
    const LENIENT: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    LENIENT.decode(text.as_bytes()).ok()
}

pub fn b64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
