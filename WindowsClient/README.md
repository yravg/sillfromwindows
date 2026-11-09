# Sill for Windows

A viewer for Sill for Mac on Windows 10 and 11: pair once with the Mac's code, then stream one of its
windows, or the Desktop, and use it with this PC's mouse and keyboard, at home or through
Tailscale or a VPN. The plan, decisions and tickets are in `../docs/windows-client-plan.md`.

## Use it

1. On the Mac, Sill for Mac 0.6 or later. Away from home: Sill › Settings › Remote Access on.
2. Unzip `Sill-windows-x64.zip` and run `Sill.exe` (unsigned for now: SmartScreen › More info ›
   Run anyway).
3. A Mac on the same network is listed; click it. Otherwise **Add a Mac…**, type the Mac's
   address (its Tailscale name or address, shown in Sill › Settings › Remote Access) and the
   12-digit code from Sill › Settings › Devices › Pair iPhone or iPad….

Keys: Ctrl is ⌘ (Ctrl+C, Ctrl+V, Ctrl+Tab…), Alt is ⌥. Your settings and this PC's key live in
`%APPDATA%\Sill`.

## Build

Workspace crates: `sill-proto` (the wire), `sill-net` (identity, pairing, TLS, the session),
`sill-media` (FFmpeg decode loaded at run time, sound playout), `sill-app` (the app: egui on wgpu).

```
cargo test --workspace                       # on Linux, macOS or Windows
cargo build --release -p sill-app            # on Windows: target/release/Sill.exe
scripts/package-windows.sh                   # from Linux: target/dist/Sill-windows-x64.zip
```
`package-windows.sh` cross-compiles with cargo-zigbuild (`pip install --user ziglang`,
`cargo install cargo-zigbuild`) and adds FFmpeg 8.1.3's LGPL shared DLLs (pinned, checksum
checked) beside `Sill.exe`. On Windows itself, copy `avcodec-62.dll`, `avutil-60.dll`,
`swscale-9.dll` and `swresample-6.dll` from an FFmpeg LGPL shared build next to `Sill.exe`.

## Test hooks (TEST ONLY, inert unless set)

- `SILL_TEST_MOCK=<case>`: labelled sample data, no network (cases in `ui/connect.rs`,
  `ui/stream.rs`).
- `SILL_TEST_SHOT="a.png@2;b.png@5"`, `SILL_TEST_QUIT_AFTER=6`, `SILL_TEST_WINDOW=WxH`:
  screenshots of what the app drew.
- `SILL_TEST_CONNECT=<Mac ID>`: connect to a saved Mac at launch. `SILL_HOME=<dir>`: the
  settings folder.
- `scripts/run-headless.sh <cmd>`: runs the Linux build under Xvfb with software GL.
- `crates/sill-net/examples/sill_cli.rs`: pair, open a session, `--dump` a capture.
