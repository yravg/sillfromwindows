//! Sill's device side. Each module ports a Swift file and cites it.
pub mod damm; // owner: worker B (PairingCode's check digit)
pub mod identity; // owner: worker B (device key, certificate, SPKI fingerprint)
pub mod pair_client;
pub mod pairing; // owner: worker B (PairingProof, PairLink, RecognitionTag)
pub mod session;
pub mod tls; // owner: worker C (rustls client, pinned verifier, ALPN)
