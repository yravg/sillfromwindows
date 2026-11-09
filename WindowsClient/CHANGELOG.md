# Changelog: Sill for Windows

## Unreleased
- Workspace: sill-proto (wire) and sill-net (identity, pairing, TLS). Plan: docs/windows-client-plan.md.
- sill-proto: framing, kinds 0–28/32/43/44, every payload; 53 tests incl. cross-checks against Scripts/sillclient.py (W-11, W-12).
- sill-net: Damm code, device identity and SPKI fingerprint, Mac ID, pairing proofs, recognition tag, PairLink, kind 18 verification; 62 tests, crypto byte-equal to sillclient.py (W-13, W-15 pure parts).
- sill-net: TLS 1.3 client with the pinned verifier; live handshake with Sill for Mac over Tailscale completed (W-14).
