# Sill for Windows: plan, PRD, roadmap and tickets

Status: plan, nothing built. Branch `windows-client` (from main at 41e8cb8). Owner: Enrico (this fork;
his call, 2026-11-09). The Mac host and the iOS app are unchanged by this work.

Sources this plan rests on, both read from the code with file:line on every row:
- `docs/windows-client/coverage-matrix.md`: every screen, control and feature of the iOS app, 112
  rows: 54 keep, 37 adapt, 20 drop, 1 unclear.
- `docs/windows-client/protocol-spec.md`: the wire as the unchanged Mac host speaks it to a device
  (framing, Bonjour, TLS and pinning, pairing, session order, input, every kind full parity uses), and
  what is still UNCLEAR (its §9).

## 1. Decisions taken (Enrico, 2026-11-09)

| # | Decision | Said |
|---|---|---|
| W1 | Rust + FFmpeg | "Rust + FFmpeg" |
| W2 | Full parity with the iPhone and iPad app, in stages | "Full parity" |
| W3 | Home network (Bonjour) and Remote Access (Tailscale, VPN, typed address) | "Home network + Remote Access" |
| W4 | `WindowsClient/` in this repo, on its own branch | "New folder in this repo, own branch" |
| W5 | Built the way Sill is built (a native client, pure policy modules with checks, a plan file), not the yravgbuilder Tauri + React stack; yravgbuilder supplies the process: PRD, roadmap, tickets, handoff | "Maintain how orignal/latest Sill version how they do it… I need the roadmap, PRD, ticket, etc all are in our workflow" |
| W6 | New dependencies approved on this branch: rustls, mdns-sd, FFmpeg (dynamically linked, LGPL), windows-rs, cpal, the mingw target | "Approve all" |
| W7 | This fork is Enrico's: CLAUDE.md's "Apple frameworks only" governs the Swift code and is not changed; the Windows client lives in its own folder with its own rules (this file) | "My fork, my call" |

## 2. PRD

**Product.** Sill for Windows: a free viewer that streams one Mac window, or the Desktop, from Sill
for Mac to a Windows 10/11 PC, and controls it with the PC's mouse and keyboard. It is a third kind
of device beside the iPhone and the iPad: the Mac sees it as a device, pairs it with a code, lists it
in Settings › Devices, and serves it at both doors.

**Users and jobs.**
- Someone at a Windows PC who wants one Mac app (Xcode, Final Cut, Logic, a Mac-only tool) beside
  their Windows work, at home or away.
- Jobs: find the Mac; pair once with the Mac's code; pick a window or the Desktop; see it sharp and
  live; click, scroll, type and use shortcuts in it; copy and paste both ways; hear it; use its menus
  and window buttons; change the stream's quality; reach it away from home through Tailscale or a VPN.

**Platforms.** Windows 10 22H2 and Windows 11, x86-64 first (ARM64 later). One .exe and FFmpeg's
DLLs in a folder or an installer.

**Non-goals.** Everything the matrix marks drop: AWDL and Direct Wireless, the USB cable's automatic
pairing, the iPhone layouts, the Duo, Pencil, haptics, the on-screen key row and trackpad,
three-finger touch gestures, the iOS Debug harness. Hosting (Windows as the host) is out: Sill
streams Mac windows.

**Requirements** (ids in the third column are the matrix's rows):

| Req | What | Matrix rows | Stage |
|---|---|---|---|
| R1 | Wire: framing, every kind full parity needs, Swift's JSON shapes exactly | spec §1, §5–7 | 1 |
| R2 | Device identity: a P-256 key kept with DPAPI (or Credential Locker), never leaving the PC; saved Macs with their pins | P10 | 1 |
| R3 | TLS 1.3 both ways, the Mac's key pinned by SPKI SHA-256, ALPN `sill/1` and `sill-pair/1` | spec §3 | 1 |
| R4 | Pairing: the Mac's code typed (12 digits, Damm check), PBKDF2 proof, the ask (kind 19 `ask`), `sill://pair` links, Pair This PC… | P01–P08, P11 | 1 |
| R5 | Discovery: `_sill._tcp` on the home network, TXT `r` and `p`, the row words | C01–C08, C10, C12 | 1 |
| R6 | Remote Access: saved Macs dialed by their addresses, the failure words, the reconnect | N01, N02 | 2 |
| R7 | The stream: HEVC decoded on the GPU (FFmpeg D3D11VA, software fallback), shown aspect-fit, viewport sent, keyframe recovery | S01, S02, S11 | 1 |
| R8 | The window bar: thumbnails, Apps drawer and search, Desktop, Settings; a thumbnail's Close, Minimize and Full Screen from a right-click | B02, B06–B11, D01–D06 | 2 |
| R9 | Input: mouse buttons, moves, wheel and precision-touchpad scroll with phases, keys as HID usages, modifiers, IME text | K01, K02, K04–K07, K11 | 2 |
| R10 | The Mac's pointer drawn when the Mac moves it; the PC's own pointer over the picture otherwise | S03 | 2 |
| R11 | Settings panel: quality, resolution, rate, speed, Virtual Display, Send Audio, Mute, Away from home, callouts, Disconnect | T01–T08, T10, T11, T16–T18 | 3 |
| R12 | Clipboard both ways (text), Share Clipboard switch | S08, T13 | 3 |
| R13 | Sound: AAC-ELD to WASAPI, Mute | A01, A02, T08 | 3 |
| R14 | The Mac's menus as a native menu strip | B03, M01 | 3 |
| R15 | Aa text size, zoom, Reverse Scrolling, the five gesture actions as commands | B04, S05, T14, T15 | 3 |
| R16 | Locked Mac cover, link line, hello and update notice, tour | S06, S07, X01, T16 | 2–3 |
| R17 | Accessibility: keyboard reachability, Narrator (UI Automation) names and notifications, high contrast, 200 % scale | X05, B12 | every stage |

**Success measures** (measured, not judged): pairs with an unchanged Sill for Mac 0.6.x on the first
try; frame age on Wi-Fi within 5 ms of the iPad's (the host's `client …` lines); 60 fps held at the
iPad's resolution on a mid-range GPU; no stuck key or button after any disconnect (the host's
`in.keyModifiersLeft` stays 0).

## 3. Architecture (Sill's way, in Rust)

Sill keeps rules in pure modules, each with a check (`Tests/checks/`), and keeps platform glue thin.
The Windows client copies that shape:

```
WindowsClient/
  Cargo.toml            workspace
  crates/sill-proto/    wire: header, kinds, every payload (serde, Swift's JSON shapes); pure; port of Sources/StreamProtocol
  crates/sill-policy/   pure rules ported one by one from iOSClient (DiscoveryPolicy, GoodbyePolicy, RemoteDialPolicy,
                        KeyframeRecovery, HostSettingsLedger, PointerPresence, ConnectionList, ...), each with its
                        Tests/checks cases as Rust tests
  crates/sill-net/      TLS (rustls), Bonjour (mdns-sd), the session, pairing, the reconnect; no UI
  crates/sill-media/    FFmpeg HEVC decode (D3D11VA, software fallback), AAC-ELD decode, WASAPI out (cpal)
  crates/sill-win/      the Windows app: windows-rs, the window, the bar, panels, input capture, clipboard
  tests/                cross-checks against Scripts/sillclient.py, the Python reference client
```

Rule for every port (the repo's own "a change to a checked file updates its check"): a Swift rule
moved to Rust brings its check's cases with it as Rust tests, and cites the Swift file:line it
follows. The Swift stays the oracle: when the two disagree, the Swift wins until Enrico says otherwise.

On this Linux machine every crate but `sill-win` builds and tests natively; `sill-win` cross-compiles
to `x86_64-pc-windows-gnu`. Bytes on the wire are cross-checked against `Scripts/sillclient.py`,
which already pairs and streams against the real host with OpenSSL.

## 4. Roadmap

| Stage | Outcome | Tickets |
|---|---|---|
| 0 | Canvas: every Windows screen drawn and signed off by Enrico (canvas-first playbook) | W-01 to W-03 |
| 1 | Pair and watch: find or type the Mac, pair with its code, pick a window or the Desktop, see it live | W-10 to W-19 |
| 2 | Use it: window bar, Apps, mouse, keyboard, scroll, the Mac's pointer, Remote Access, reconnect | W-20 to W-28 |
| 3 | Parity: settings, clipboard, sound, menus, Aa and zoom, gestures as commands, tour | W-30 to W-37 |
| 4 | Ship: installer, signing, `sill://` handler, update check, site and README words | W-40 to W-45 |

## 5. Tickets

Done means: its tests pass, its matrix rows are marked done, and what was not checked is written
under it. One owner per file.

**Stage 0 (design)**
- W-01 Windows chrome reference for the boards: window, title bar, menu strip, flyout, dialog.
- W-02 Boards: connect screen (rows, tiles, empty, Public-network, notice), pairing (typed, ask,
  link confirmation), stream screen with the bar, Apps drawer, Settings panel, locked cover, link
  line, tour; every state; light, dark, high contrast.
- W-03 Sign-off by Enrico; decisions D1–D8 answered.

**Stage 1 (pair and watch)**
- W-10 Workspace, CI (cargo fmt, clippy -D warnings, test; the Windows target built), CHANGELOG.
- W-11 sill-proto framing: header, kinds 0–44, caps; cases from `Tests/checks/protocol`.
- W-12 sill-proto payloads: WindowList, Hello, Viewport, Goodbye, PairRequest/Result,
  SignedMacInfo, ParameterSets, Switcher blobs; round trips against JSON the Swift encoder wrote
  (fixtures made on a Mac with a small Swift script, or captured from `sillclient.py`).
- W-13 Identity: P-256 key, self-signed device certificate, SPKI fingerprint; DPAPI storage.
- W-14 TLS: rustls client with the pinned verifier and client auth; first live handshake against a
  Mac (risks 1 and 2).
- W-15 Pairing: Damm code, PBKDF2 proof, kinds 19/20, the ask, the link; cases from
  `Tests/checks/home-*` and `remote-rules`.
- W-16 Discovery: mdns-sd browse of `_sill._tcp`, TXT `r` and `p`, recognition tag, row words
  (DiscoveryPolicy port with its `policy` and `home-device` cases).
- W-17 Session: hello first, window-list gate, viewport, ping and liveness (MessageReader's rule),
  goodbye words (GoodbyePolicy).
- W-18 Video: FFmpeg HEVC from length-prefixed NALs and out-of-band parameter sets, D3D11VA,
  software fallback, swap-chain present, keyframe request (KeyframeRecovery port).
- W-19 Connect screen and pairing UI; the first window shown live.

**Stage 2 (use it)**
- W-20 Window bar: thumbnails, order kept per Mac, Desktop, Settings button.
- W-21 Apps drawer and search, launch (kind 7).
- W-22 Thumbnail right-click: Close, Minimize, Full Screen (kind 15).
- W-23 Mouse, wheel and precision-touchpad scroll with phases (kind 8); positions as frame fractions.
- W-24 Keyboard: Windows scan code → HID usage table, modifiers (D2), IME text through TSF, the
  `keyRepeat` rule, every key let go on focus loss and disconnect.
- W-25 The Mac's pointer (kind 26, PointerPresence port).
- W-26 Remote Access dials and the reconnect order (RemoteDialPolicy, RemoteConnector).
- W-27 Locked Mac cover, link line, update notice (kind 22 "update").
- W-28 Narrator and keyboard reachability for everything so far.

**Stage 3 (parity)**
- W-30 Settings panel and its ledger (kinds 16/17, HostSettingsLedger port).
- W-31 Clipboard both ways (kind 32, ClipboardPolicy port).
- W-32 Sound: AAC-ELD decode, WASAPI playout (AudioPlayout port), Mute.
- W-33 The Mac's menus (kinds 24/25/27, MacMenuState port).
- W-34 Aa text size, zoom, Reverse Scrolling.
- W-35 Gesture actions as commands (kind 28).
- W-36 Tour (TourPolicy port, Windows steps).
- W-37 Pair This PC… over the stream (kind 21).

**Stage 4 (ship)**
- W-40 Installer (D6) with FFmpeg's DLLs and their LGPL notice.
- W-41 Code signing (D7).
- W-42 `sill://` protocol handler.
- W-43 Update check against GitHub releases (UpdatePolicy port).
- W-44 README, site download and support words for Windows.
- W-45 How the PC appears in the Mac's Settings › Devices (D8).

## 6. Decisions D1–D8 (Enrico, 2026-11-09: "Yes, go with all your recommendation"; D7 has none, so unsigned until he decides)

- **D1 UI toolkit.** Sill's bar is native-feel. Recommend windows-rs: a Win32 window, the video in
  a D3D11 swap chain through DirectComposition, the bar and panels drawn with Direct2D/DirectWrite in
  the iOS app's palette, native menus and dialogs. Alternative: egui on wgpu (faster to build, not
  native).
- **D2 Which key is the Mac's ⌘.** Recommend Ctrl → ⌘ and the Windows key → ⌃, with a setting to
  swap: Windows keeps Win+L, Win+D and others for itself, so Win → ⌘ breaks common shortcuts. (The
  matrix's K05 assumed Win = ⌘.)
- **D3 Update destination** (C09): recommend the GitHub release page until a Windows listing exists.
- **D4 Local Network wording** (C14): recommend detecting a Public network profile and saying
  "Windows is treating this network as public, so Sill can't find your Mac. Set it to Private in
  Settings › Network & internet."
- **D5 More than one Sill window** (X08): recommend one window in stages 1–3.
- **D6 Installer:** recommend a zip first, MSIX later.
- **D7 Signing:** no certificate exists; SmartScreen warns on an unsigned .exe. Enrico's call.
- **D8 The Mac side:** recommend no host change; the hello's device name is "‹PC name› (Windows)".

## 7. Risks (the spec's §9)

1. rustls and the Mac's self-signed v3 certificate with no extensions, and Apple's
   CertificateRequest (W-14 is the first live test; a custom verifier is planned).
2. Mutual TLS failing after the handshake rather than during it (read the host's Sill.log).
3. HEVC delivery: length-prefixed NALs, out-of-band parameter sets, no PTS.
4. Pairing proofs must be byte-exact; five wrong tries close a pairing window.
5. The AAC-ELD cookie's format (ASC or ES descriptor) is unclear; sound can ship after video.

## 8. Handoff

- Next step: stage 0 (W-01, W-02) is design work for `/tastestack:tastestack-mode`, which only
  Enrico can start. W-10 to W-13 need no Mac and can run in parallel with stage 0 if Enrico says so.
- No Mac is reachable from this Linux machine: every live check (W-14 onward) needs Enrico's Mac with
  Sill for Mac 0.6.x and a Windows PC.

## Progress

- 2026-11-09: D1–D8 taken as recommended. Enrico's MacBook Pro M1 runs Sill on the tailnet (enricos-macbook-pro, 100.88.89.4); its remote door 7455 answers a TCP connect from this machine (~250 ms round trip). Workspace WindowsClient/ made (sill-proto, sill-net); extra crates beside the approved list: serde, serde_json, base64, sha2, hmac, pbkdf2, p256, rcgen, rustls-pemfile (all pure Rust; rustls on ring).

- 2026-11-09: decisions W1–W7; the coverage matrix and the wire spec written by delegates and
  spot-checked against the code (StreamMessage.swift:132-191, Pairing.swift:81,
  ContentView.swift:1405, AddMacCard.swift:138); this plan.
- 2026-11-09: W-11, W-12, W-13, W-14 done; W-15's pure part done. `cargo test --workspace` 123 passed,
  clippy -D warnings and fmt clean, `cargo zigbuild --target x86_64-pc-windows-gnu` builds (Zig's
  bundled mingw; no sudo here). Live: rustls handshake with Enrico's Mac (100.88.89.4:7455), TLS 1.3,
  ALPN sill/1, the Mac's 310-byte certificate parsed and its CertificateVerify verified by rustls; the
  Mac asked for a client certificate (ECDSA P-256 first) and refused the unpaired key with
  CertificateUnknown after the handshake, as the spec said. Mac SPKI SHA-256
  4f29744e66655323cd29526c27fbde3d719931b71080504566d03ed3a719e229. Risks 1 and 2 closed.

