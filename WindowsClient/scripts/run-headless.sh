#!/bin/bash
# TEST ONLY: runs a command (the Linux build of Sill, usually with SILL_TEST_MOCK and SILL_TEST_SHOT)
# under a virtual X display with software GL, for screenshots on a machine with no screen.
# Fetches the X keyboard and EGL libraries this kind of machine often lacks into target/xlibs
# with `apt-get download` (no root needed). Needs xvfb-run.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)/target/xlibs"
LIB="$ROOT/root/usr/lib/x86_64-linux-gnu"
if [ ! -e "$LIB/libxkbcommon-x11.so" ]; then
  mkdir -p "$ROOT/debs"
  (cd "$ROOT/debs" && for p in libxkbcommon-x11-0 libxcb-xkb1 libegl1 libegl-mesa0 libgbm1 \
      libwayland-server0 libxcb-randr0 libxcb-dri2-0 libxcb-dri3-0 libxcb-present0 libxcb-sync1 \
      libxcb-xfixes0 libxshmfence1 libdrm2 libglapi-mesa libglvnd0; do
      apt-get download "$p" >/dev/null 2>&1 || echo "run-headless: could not fetch $p" >&2; done)
  for f in "$ROOT"/debs/*.deb; do dpkg-deb -x "$f" "$ROOT/root"; done
  ln -sf libxkbcommon-x11.so.0 "$LIB/libxkbcommon-x11.so"
fi
export LD_LIBRARY_PATH="$LIB${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export __EGL_VENDOR_LIBRARY_FILENAMES="$ROOT/root/usr/share/glvnd/egl_vendor.d/50_mesa.json"
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl
exec xvfb-run -a -s "-screen 0 ${SILL_TEST_SCREEN:-1440x900}x24" "$@"
