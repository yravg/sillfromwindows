# Sill for Windows: agent instructions

This is the first file to read. Then read [`HANDOFF.md`](HANDOFF.md) for the current state and the
next work, and [`docs/windows-client-plan.md`](docs/windows-client-plan.md) for the plan, tickets
and decisions.

## What this repository is

Sill is a Mac app that streams one Mac window, or the desktop, to an iPhone or iPad. **The Mac and
iOS apps, the wire protocol, the design and the documentation are Noah Saffer's work**
([Saffsanity/sill](https://github.com/Saffsanity/sill)); this repository is a fork that adds the
Windows client in [`WindowsClient/`](WindowsClient/). Credit and license are in
[`README.md`](README.md), [`NOTICE`](NOTICE) and [`LICENSE`](LICENSE). Do not present the Mac/iOS
work as the fork's.

The root [`CLAUDE.md`](CLAUDE.md) is upstream's guidance for the Apple sources. This file and
`HANDOFF.md` govern the Windows client. When the two disagree about an Apple source, the upstream
conventions win; when a Windows task requires an upstream change, stop and read
`docs/windows-client/upstream-sync.md` first.

## Where things are

| Path | What it is |
|---|---|
| `WindowsClient/Cargo.toml` | the Rust workspace: `sill-proto`, `sill-net`, `sill-media`, `sill-app` |
| `WindowsClient/crates/sill-proto` | the wire: framing, kinds, payloads, JSON shapes (the oracle is `Sources/StreamProtocol`) |
| `WindowsClient/crates/sill-net` | identity, pairing, TLS, discovery, the session |
| `WindowsClient/crates/sill-media` | FFmpeg decode loaded at run time, audio playout |
| `WindowsClient/crates/sill-app` | the app: eframe/egui on wgpu, the connect screen, the stream screen, Settings |
| `WindowsClient/crates/sill-app/src/ui/widgets.rs` | the shared UI pieces: bar buttons, icons, switches, segmented controls, cards, thumbnails |
| `WindowsClient/crates/sill-app/src/theme.rs` | the palette and the iPad's metrics; **use these tokens, never raw values** |
| `docs/windows-client-plan.md` | PRD, architecture, roadmap, tickets, decisions, progress |
| `docs/windows-client/coverage-matrix.md` | every iOS screen/control mapped to Windows |
| `docs/windows-client/protocol-spec.md` | the wire as this client speaks it, with the upstream file:line |
| `docs/windows-client/upstream-sync.md` | how to merge upstream, and what the new wire means here |
| `docs/windows-client/screenshots/` | mock renders from the test harness (never real Mac content) |
| `Sources/`, `iOSClient/`, `Tests/`, `Scripts/` | upstream Mac/iOS sources; edit only for an upstream sync |

## Build, test, run (Windows)

```powershell
cd WindowsClient
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p sill-app     # target\release\Sill.exe
```

If `target\release\Sill.exe` is running, the final link fails with `Access is denied`; close the app,
or build with `$env:CARGO_TARGET_DIR='...\target-diag'` (gitignored).

`target\release` already carries the FFmpeg 8 shared libraries (`avcodec-62.dll`, `avutil-60.dll`,
`swscale-9.dll`, `swresample-6.dll`). On a fresh machine, copy those four beside `Sill.exe` (the
package script does it on Linux; see `WindowsClient/README.md`). The app writes
`%APPDATA%\Sill\Sill.log`, `identity.key` (DPAPI-wrapped), `macs.json` and `prefs.json`.

Run `Sill.exe`; on the Mac, Sill for Mac 0.6 or later, pair once with the 12-digit code, then pick
a window or the Desktop. Ctrl is ⌘ (Ctrl+C, Ctrl+V); Alt is ⌥; the Windows key can be ⌘ in
Settings.

### Test hooks (TEST ONLY, inert unless set)

| Variable | What it does |
|---|---|
| `SILL_TEST_MOCK=<case>` | sample data, no network; cases live in `ui/connect.rs`, `ui/stream.rs`, `ui/settings.rs` |
| `SILL_TEST_SHOT="a.png@2;b.png@5"` | saves what egui drew at those seconds after launch |
| `SILL_TEST_QUIT_AFTER=6` | quits then |
| `SILL_TEST_WINDOW=1600x1000` | opens the window at that logical size |
| `SILL_TEST_STREAM_OPEN=settings,drawer,menu,aa` | opens those at start (mock cases) |
| `SILL_TEST_CONNECT=<Mac ID>` | connects to a saved Mac at launch |
| `SILL_HOME=<dir>` | uses that settings folder instead of `%APPDATA%\Sill` |

`SILL_TEST_MOCK=stream-desktop`, `SILL_TEST_STREAM_OPEN=settings` and
`SILL_TEST_SHOT=...` are the quickest visual check. The mock screenshots in
`docs/windows-client/screenshots/` were made that way. Linux/headless: `scripts/run-headless.sh`.

## Rules for changes

- **Match the house style.** Comments explain why; cite the iOS source (`StreamScreen.swift:…`,
  `HostSettingsPanel.swift:…`, `Sources/StreamProtocol/…`) where a rule comes from. Pure rules live
  in a module with tests; platform glue stays thin. Keep the `OWNER:` header on files.
- **UI.** Use `theme.rs` tokens and `ui/widgets.rs`. The target is the iPad's look: 86 pt bar
  (78 compact), 66 pt icon-and-label buttons, 104×66 thumbnails with app badges and the active
  halo, 360 pt Settings on 12 pt cards, Segoe UI, 30 pt menu rows. Windows adaptations are mouse
  and keyboard, no on-screen keyboard, a **Mac** button for the gestures (Mission Control, Spaces,
  …), and right-click window actions. One thing open at a time (drawer, Settings, Menus, Aa).
- **Protocol.** `sill-proto` mirrors `Sources/StreamProtocol`; the Swift is the oracle. Unknown host
  kinds are skipped (`MessageKind::Unknown(n)`); unknown JSON fields are ignored. Additive changes
  are safe; a changed shape is not. Update `docs/windows-client/protocol-spec.md` and the framing,
  payload and session tests with every wire change.
- **Upstream sync.** Keep `WindowsClient/` and the Windows docs additive. Fetch with
  `git -c http.version=HTTP/1.1 …` on this network, merge `upstream/main`, run the checks, update
  `HANDOFF.md`. The only upstream file with fork edits is a four-line pointer at the top of
  `CLAUDE.md` (plus the README credit and `NOTICE`). See `docs/windows-client/upstream-sync.md`.
- **Security and privacy.** Never commit `%APPDATA%\Sill`, `identity.key`, `macs.json`, pairing
  codes, keys or real Mac screenshots. Test hooks must stay inert without their environment
  variables. Do not connect to someone's Mac or send input without permission. Do not add
  destructive git operations; tag a backup before rewriting `main`.
- **Definition of done.** `cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace
  --all-targets -- -D warnings` and `cargo build --release -p sill-app` pass; the mock screenshot
  renders; a live session is checked when a Mac is available; `CHANGELOG.md` and `HANDOFF.md` are
  updated; upstream files stay untouched (except the CLAUDE pointer and credit).
