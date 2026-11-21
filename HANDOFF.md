# Handoff: Sill for Windows

Last updated: 2026-11-21. This file is the current state for whoever picks the work up next; the
durable rules are in [`AGENTS.md`](AGENTS.md), and the plan is
[`docs/windows-client-plan.md`](docs/windows-client-plan.md).

## The repository in one paragraph

This is a GitHub fork of [Saffsanity/sill](https://github.com/Saffsanity/sill), the Mac/iOS app that
streams a Mac window or desktop to an iPhone or iPad. The upstream Mac app, iOS app, wire protocol,
design and documentation are Noah Saffer's work (99% of the repository). The fork adds the Windows
viewer in [`WindowsClient/`](WindowsClient/), a Rust workspace using eframe/egui on wgpu, FFmpeg
(at run time) and Windows DPAPI for the device key. `main` is upstream `main` plus the Windows
folders and the credit/agent docs, so syncing upstream is a normal merge.

- Fork: https://github.com/yravg/sillfromwindows
- Upstream: https://github.com/Saffsanity/sill
- Upstream base at the last sync: `c33d105` (2026-11-20, Sill 0.6.3)
- Last feature commit at the time of writing: `bc1bd5e` (the commit that adds this file comes after)
- Branch: `main`; `upstream` remote is set to the GitHub URL.

## Run it now (on the author's PC)

The release build is already made:

```powershell
E:\dev\SillFromWindows\WindowsClient\target\release\Sill.exe
```

The four FFmpeg DLLs are beside it in `target\release`. If an older window is still open, it may be
`Sill.prev.exe` (renamed so a rebuild could link); close it and launch the new `Sill.exe`. The app
stores its paired Mac, key and log in `%APPDATA%\Sill`; never commit that folder. A paired Mac is
configured on this PC, and `SILL_TEST_CONNECT=<Mac ID>` can connect at launch; the ID lives in
`%APPDATA%\Sill\macs.json` (do not copy it into the repo).

To build and check on any Windows machine:

```powershell
cd E:\dev\SillFromWindows\WindowsClient
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p sill-app
```

## What works and was verified

- **Pairing and streaming** to Sill for Mac over the home/remote doors, TLS pinning, the window
  list, thumbnails/icons, mouse and keyboard input, menus, clipboard, Settings through the ledger,
  the Apps drawer, thumbnails' right-click window actions, sound wiring, and reconnects.
- **Picture:** the Mac's frame is decoded by FFmpeg and drawn at its own gamma. A live session on
  2026-11-21 showed the Mac picture and the Sill bar at matching brightness (the old
  `Bgra8UnormSrgb` texture had darkened the whole picture a second time).
- **Stream bar:** iPad metrics and look — 86 pt (78 compact) height, 66 pt icon-and-label buttons,
  104×66 thumbnails with app badges, active halo, Segoe UI, vector icons, menu rows, one-thing-open
  behaviour. See `docs/windows-client/screenshots/stream-bar.png` (mock data only).
- **Settings panel:** 360 pt floating panel with the original's header, 12 pt group cards, 44 pt
  rows, real switches, segmented Resolution/Frame Rate, Quality menu, footnotes, orange callouts,
  pinned Disconnect and version line. See `docs/windows-client/screenshots/settings.png` and
  `settings-away.png`.
- **Desktop displays (upstream 0.6.3):** the button reads "Desktop N", shows a dot per stop and
  sends kind 47 `nextDisplay` while the Desktop streams. It reads `desktopDisplays`,
  `desktopDisplay` and `desktopDisplayNames`; older hosts leave them out and behave as before.
  The Mac used for live tests has one display, so multi-display stepping is verified by the pure
  tests and the mock, not on real hardware yet.
- **Upstream compatibility:** unknown kinds are skipped, unknown JSON fields ignored; a
  `Goodbye.restarting` (one-click update) already shows the Mac's message and reconnects through the
  unknown-reason path. `Hello.offConsoleView` is not sent, so the host treats this client as an
  older device (locked list without windows).
- **Checks:** `cargo test --workspace` passes (288 tests at the last run), `cargo fmt --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` are clean, and `cargo build --release -p
  sill-app` succeeds. The live check connected to the author's Mac Mini, streamed the Desktop, and
  showed the new bar and Settings.

## Just landed

1. The dark-picture fix: the video texture is `Bgra8Unorm` (egui's shader does the sRGB work);
   `Bgra8UnormSrgb` decoded it twice.
2. The repository synced to upstream `main` `c33d105` (Sill 0.6.3), with the old history preserved
   by tag `backup/pre-upstream-sync-2026-11-21`. The README and `NOTICE` credit Noah/upstream; the
   GitHub description says the fork and the original.
3. The UI parity pass (bar, thumbnails, Settings, Apps drawer, Segoe UI, vector icons).
4. Desktop displays (kind 47) and the window-list fields.
5. The version line in Settings.
6. The handoff pack: `AGENTS.md`, this file, `AGENT.md`, `WindowsClient/AGENTS.md` and mock
   screenshots under `docs/windows-client/screenshots/`.

## Next work, in priority order

1. **Connect screen parity.** The iPad's connect screen is not fully ported: tiles/list switch,
   group sections and naming, the last-frame preview per saved Mac (a stored keyframe), the row/tile
   context menu (Connect, Connect Remotely, group, list/tiles, remove preview, Forget), and the
   paired-state words. The Windows rows, Add a Mac card, pairing and footer already work. Matrix
   rows C03–C08 and the iOS references in `docs/windows-client/coverage-matrix.md`.
2. **Pair This PC… over the stream** (upstream kind 21, iOS `PairingOverlay.swift`, plan W-37). Add
   the Settings row and the overlay; the connect screen's pairing path can be reused. It needs the
   host's code display and the 12-digit entry.
3. **Take the Tour** (plan W-36, `TourPolicy.swift`). The iPad shows a first-run tour and a
   "Take the Tour" row. Decide the Windows steps; the tour must pause input, as on iOS.
4. **Off-console view** (upstream `Hello.offConsoleView`, `LockedMac.viewLine`, PR #316/#340). This
   is privacy-sensitive: it shows the session's desktop while another user is on the Mac. Only add
   it if Enrico explicitly wants it, with a clear opt-in.
5. **Accessibility and Windows conventions.** Narrator/UI Automation names and notifications,
   keyboard focus rings and reachability, high contrast, 200% scaling. The buttons and switches have
   accesskit roles now; the picture and overlays need a pass. Plan W-28.
6. **Ship.** No installer, signing or `sill://` handler yet (stage 4, W-40–W-45). The exe is
   unsigned, so SmartScreen warns. The package script exists in `WindowsClient/scripts/`.
7. **Maintenance.** Sync upstream when Sill for Mac moves (see
   `docs/windows-client/upstream-sync.md`); a monthly check is enough. Add a Windows CI workflow
   (fmt, clippy, tests without FFmpeg fixtures, release build) if the GitHub Actions bill allows.
8. **Quality, if asked.** The live Mac is classified as *away* (Tailscale) and runs Low · 4 Mbps;
   the host says the link cannot carry a steady picture even at Low. The client now shows that
   callout; raising the away preset is a host/link decision, not a client default. Do not silently
   raise it.

## Where to look

| Need | File |
|---|---|
| The rules for any agent/harness | `AGENTS.md` |
| The plan, PRD, tickets, decisions | `docs/windows-client-plan.md` |
| Screen-by-screen iOS → Windows map | `docs/windows-client/coverage-matrix.md` |
| The wire, kinds and payloads | `docs/windows-client/protocol-spec.md` |
| Upstream sync and the new wire | `docs/windows-client/upstream-sync.md` |
| Client architecture | `WindowsClient/README.md`, `WindowsClient/crates/*` |
| Shared UI look | `WindowsClient/crates/sill-app/src/theme.rs`, `src/ui/widgets.rs` |
| Stream screen and bar | `WindowsClient/crates/sill-app/src/ui/stream.rs` |
| Settings panel | `WindowsClient/crates/sill-app/src/ui/settings.rs` |
| Connect/pairing screen | `WindowsClient/crates/sill-app/src/ui/connect.rs` |
| Controller/session/input | `WindowsClient/crates/sill-app/src/controller.rs`, `src/input.rs` |
| Wire types | `WindowsClient/crates/sill-proto/src/*.rs` |
| Mock cases and screenshots | `WindowsClient/crates/sill-app/src/ui/mock.rs`, `docs/windows-client/screenshots/` |

## Verification recipes

Mock screenshots (no Mac, no network; safe to run while another Sill is open if you use the
`target-diag` build):

```powershell
cd WindowsClient
$env:SILL_HOME='E:\orca\tmp\sill-home'
$env:SILL_TEST_MOCK='stream-desktop'
$env:SILL_TEST_WINDOW='1600x1000'
$env:SILL_TEST_STREAM_OPEN='settings'      # or drawer, menu, aa
$env:SILL_TEST_SHOT='E:\orca\tmp\check.png@2'
$env:SILL_TEST_QUIT_AFTER='3.5'
cargo build --release -p sill-app --target-dir target-diag
.\target-diag\release\Sill.exe
```

A normal `cargo build --release -p sill-app` fails with "Access is denied" while
`target\release\Sill.exe` is running. Close the app, or rename the running exe (the previous session
used `Sill.prev.exe`), or build into `target-diag`. Never delete a running exe by force.

For a live check, launch with `SILL_TEST_CONNECT=<Mac ID>` and a screenshot hook. It connects to the
saved Mac and uses the real identity; do not send input, and do not commit the log, screenshots or
`%APPDATA%\Sill`.

## Traps and risks

- **Upstream edits.** Keep Windows work additive. The only upstream files with fork edits are the
  four-line pointer at the top of `CLAUDE.md` and the credit block at the top of `README.md`;
  `NOTICE` and the agent docs are new files. Keep them while merging.
- **GitHub network (China).** Git over HTTPS can be reset. Fetch/push with
  `git -c http.version=HTTP/1.1 -c http.lowspeedlimit=0 -c http.lowspeedtime=300 …`; this worked.
- **FFmpeg.** The app loads the shared libraries at run time; tests that decode need them (or the
  synthetic fixture). Keep the four DLLs beside the exe; never commit them. The package script pins
  the LGPL build.
- **Python oracle tests.** They use `python3` or `python`; Windows usually has `python`. The tests
  fail rather than skip if neither is present.
- **DPI.** Screenshots are egui's own, in physical pixels; 1600×1000 logical at 150% is 2400×1500.
  Compare mock and live renders at the same scaling.
- **Multi-display.** The Desktop stepping is tested in Rust and the mock; the live Mac has one
  display. Test on a Mac with two screens before calling it done.
- **Security.** `identity.key` is DPAPI-wrapped; `macs.json` contains fingerprints and recognition
  keys. Never commit `%APPDATA%\Sill`, pairing codes, real screen captures or logs. Test hooks are
  inert unless their environment variable is set; keep it that way.
- **Unsigned build.** There is no certificate or installer; SmartScreen warns. Do not claim a
  signed release.

## New-agent checklist

1. Read `AGENTS.md`, this file, and `docs/windows-client-plan.md`.
2. `git pull`; make sure `main` is clean and up to date; check `git log -3` and `git status`.
3. Run `cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D
   warnings` before changing anything.
4. Pick one task from "Next work"; read its iOS source and the coverage row first.
5. Make the smallest complete change; use `theme.rs` and `ui/widgets.rs`; add a pure test or a
   screenshot check.
6. Update `WindowsClient/CHANGELOG.md`, the plan/coverage and this handoff.
7. Run the full checks and a mock screenshot; live-test if a Mac is available.
8. Commit with the house prose style, push `main`, and keep `WindowsClient/` additive.
