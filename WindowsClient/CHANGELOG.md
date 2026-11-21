# Changelog: Sill for Windows

## Unreleased
- Repository: synced to upstream Sill `main` `c33d105` (2026-11-20, Sill 0.6.3). See
  `docs/windows-client/upstream-sync.md`.
- UI: the stream screen follows the iPad's metrics and look — an 86 pt (78 compact) bar with 66 pt
  icon-and-label buttons, 104×66 window thumbnails with app badges and the active halo, Segoe UI,
  and a 360 pt Settings panel on the original's 12 pt group cards, switches, segmented controls,
  footnotes and pinned Disconnect.
- Desktop: the Desktop button reads "Desktop N" with a dot per stop and steps through the Mac's
  displays (kind 47) when the Desktop streams; the window list's `desktopDisplays`,
  `desktopDisplay` and `desktopDisplayNames` are read.
- Stream: the Mac's picture is no longer drawn too dark. The video texture is gamma-space, as
  egui-wgpu samples every texture (an sRGB view decoded it a second time: the bar looked right, the
  picture was dim).
- Workspace: sill-proto (wire) and sill-net (identity, pairing, TLS). Plan: docs/windows-client-plan.md.
- sill-proto: framing, kinds 0–28/32/43/44, every payload; 53 tests incl. cross-checks against Scripts/sillclient.py (W-11, W-12).
- sill-net: Damm code, device identity and SPKI fingerprint, Mac ID, pairing proofs, recognition tag, PairLink, kind 18 verification; 62 tests, crypto byte-equal to sillclient.py (W-13, W-15 pure parts).
- sill-net: TLS 1.3 client with the pinned verifier; live handshake with Sill for Mac over Tailscale completed (W-14).
