//! Sill wire format. Pure port of `Sources/StreamProtocol` (the Swift is the oracle).
//!
//! * [`framing`]: the 14-byte header, every message kind, the size caps, an incremental reader
//!   (`iOSClient/MessageReader.swift`) and `ParameterSets`.
//! * [`wire`]: Swift `JSONEncoder`/`JSONDecoder` conventions (`Wire.encode/decode`) and Data as base64.
//! * the payloads, one module per Swift file: [`switcher`], [`input`], [`viewport`], [`compat`],
//!   [`remote`], [`settings`], [`menu`], [`small`] (pointer, gesture, clipboard), [`audio`], [`blobs`].
//!
//! JSON shape rules (all payloads): field names as Swift's synthesized `Codable` writes them; an
//! `Optional` that is nil is omitted on encode and accepted as absent or `null` on decode; unknown
//! keys are ignored on decode (an older or newer peer); a Swift enum with associated values is
//! `{"case":{"_0":..,"label":..}}`. No crypto lives here (see `sill-net`).

pub mod audio;
pub mod blobs;
pub mod compat;
pub mod framing;
pub mod input;
pub mod menu;
pub mod remote;
pub mod settings;
pub mod small;
pub mod switcher;
pub mod viewport;
pub mod wire;

pub use framing::{
    Caps, Header, Message, MessageKind, MessageReader, ParameterSets, ReadError, HEADER_LEN,
};
