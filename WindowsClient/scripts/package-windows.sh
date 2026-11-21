#!/bin/bash
# Builds Sill for Windows as a folder and a zip: target/dist/Sill-windows-x64[.zip].
#   Sill.exe, FFmpeg's LGPL shared libraries beside it (loaded at run time, never linked), their
#   licence, and a short read-me. Cross-compiles from Linux or macOS with cargo-zigbuild (Zig's
#   bundled mingw-w64); on Windows itself run `cargo build --release -p sill-app` instead.
# FFmpeg is pinned: BtbN's FFmpeg-Builds autobuild-2026-11-08-12-35, n8.1.3, win64 LGPL shared.
set -euo pipefail
cd "$(dirname "$0")/.."
FF_URL=https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-11-08-12-35/ffmpeg-n8.1.3-34-ga35157abb9-win64-lgpl-shared-8.1.zip
FF_SHA=f2ff0b5756af156ca291da018c3cc1985fc43041388e09a5b0bd9901faa511b3
FF_ZIP=target/ffmpeg-win64-lgpl-shared-8.1.zip
OUT=target/dist/Sill-windows-x64
TARGET=x86_64-pc-windows-gnu

export PATH="$HOME/.local/bin:$PATH"
echo "== Sill.exe (release, $TARGET)"
cargo zigbuild --release -p sill-app --target "$TARGET"

if ! echo "$FF_SHA  $FF_ZIP" | sha256sum -c --status 2>/dev/null; then
  echo "== FFmpeg 8.1.3 LGPL shared (pinned)"
  mkdir -p target
  curl -fsSL -o "$FF_ZIP" "$FF_URL"
  echo "$FF_SHA  $FF_ZIP" | sha256sum -c -
fi

rm -rf "$OUT" "$OUT.zip"
mkdir -p "$OUT/licenses"
cp "target/$TARGET/release/Sill.exe" "$OUT/"
python3 -I - "$FF_ZIP" "$OUT" <<'PY'
import sys, zipfile, os, re
z = zipfile.ZipFile(sys.argv[1]); out = sys.argv[2]
for n in z.namelist():
    base = os.path.basename(n)
    if re.fullmatch(r"(avcodec|avutil|swscale|swresample)-\d+\.dll", base):
        open(os.path.join(out, base), "wb").write(z.read(n)); print("  ", base)
    elif base == "LICENSE.txt":
        open(os.path.join(out, "licenses", "FFmpeg-LICENSE.txt"), "wb").write(z.read(n))
PY
cp ../LICENSE "$OUT/licenses/Sill-LICENSE.txt"
cat > "$OUT/README.txt" <<'TXT'
Sill for Windows (development build)

Streams a window, or the whole Desktop, of a Mac running Sill for Mac 0.6 or later.

1. On the Mac: Sill > Settings > Remote Access on (for Tailscale or a VPN), or the same network.
2. Run Sill.exe. Windows may warn that the app is unrecognized (it is not signed yet):
   More info > Run anyway.
3. Pair: Add a Mac..., type the Mac's address (its Tailscale name or address, shown in
   Sill > Settings > Remote Access) and the 12-digit code from Settings > Devices >
   Pair iPhone or iPad... on the Mac.

FFmpeg (avcodec, avutil, swscale, swresample) is used under the LGPL 2.1 or later and is loaded
at run time from this folder; its licence is in licenses/. Sill is Apache-2.0.
Your settings and this PC's key are in %APPDATA%\Sill.
TXT
(cd "$(dirname "$OUT")" && python3 -I -m zipfile -c "$(basename "$OUT").zip" "$(basename "$OUT")")
ls -la "$OUT" "$OUT.zip"
