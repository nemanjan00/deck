#!/usr/bin/env bash
# Build a self-contained deck AppImage that BUNDLES the SDR decode tools
# (rtl_sdr, multimon-ng, sox, minimodem, hackrf, airspyhf + dsd-neo, rtl_ais,
# dump1090 from source). Runs on Debian/Ubuntu — GitHub CI or a Pi.
#
#   ci/build-appimage.sh <x86_64|aarch64>
#
# Result: ./deck-<arch>.AppImage next to the repo root.
#
# NOTE: bundling the tools removes the `apt install` step, but RTL-SDR still
# needs host-side USB permissions (see packaging/70-deck-sdr.rules and the
# blacklist note). No AppImage can grant those.
#
# glibc, libGL/EGL, libxkbcommon and the audio daemon libs are deliberately
# left to the host (standard on Pi OS desktop) — linuxdeploy's excludelist
# handles that. We only bundle app-level libs.
set -euo pipefail

ARCH="${1:?usage: build-appimage.sh <x86_64|aarch64>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAGE="$ROOT/stage"
APPDIR="$ROOT/AppDir"
JOBS="$(nproc 2>/dev/null || echo 2)"
rm -rf "$STAGE" "$APPDIR"
mkdir -p "$STAGE" "$APPDIR/usr/bin" "$APPDIR/usr/lib"

group() { echo "::group::$*"; }
endgroup() { echo "::endgroup::"; }

group "apt build + runtime deps"
sudo apt-get update
# build toolchain + -dev libs for the from-source tools, and the
# apt-shippable runtime tools themselves.
sudo apt-get install -y --no-install-recommends \
  build-essential cmake git pkg-config curl ca-certificates \
  librtlsdr-dev libusb-1.0-0-dev libsndfile1-dev libpulse-dev \
  libncurses-dev libairspyhf-dev \
  rtl-sdr multimon-ng sox minimodem hackrf
# airspyhf tools live in different packages across releases; best-effort.
sudo apt-get install -y --no-install-recommends airspyhf || \
  echo "WARN: 'airspyhf' package unavailable — airspyhf_rx may be missing"
endgroup

# Copy an apt-provided binary into the AppDir if present.
take() {
  if p="$(command -v "$1" 2>/dev/null)"; then
    cp -v "$p" "$APPDIR/usr/bin/"
  else
    echo "WARN: $1 not found (mode using it will show 'needs tools')"
  fi
}
group "collect apt tools"
for b in rtl_sdr rtl_fm rtl_test multimon-ng sox minimodem hackrf_transfer airspyhf_rx; do
  take "$b"
done
endgroup

group "dsd-neo (source)"
git clone --depth 1 https://github.com/arancormonk/dsd-neo "$STAGE/dsd-neo"
cmake -S "$STAGE/dsd-neo" -B "$STAGE/dsd-neo/build" -DCMAKE_BUILD_TYPE=Release
cmake --build "$STAGE/dsd-neo/build" -j"$JOBS"
# binary name has varied (dsd-neo / dsd); grab whatever built.
found="$(find "$STAGE/dsd-neo/build" -maxdepth 3 -type f -executable \
  \( -name 'dsd-neo' -o -name 'dsd' \) | head -1)"
cp -v "$found" "$APPDIR/usr/bin/dsd-neo"
endgroup

group "rtl-ais (source)"
git clone --depth 1 https://github.com/dgiardini/rtl-ais "$STAGE/rtl-ais"
make -C "$STAGE/rtl-ais" -j"$JOBS"
cp -v "$STAGE/rtl-ais/rtl_ais" "$APPDIR/usr/bin/"
endgroup

group "dump1090 (source, flightaware)"
git clone --depth 1 https://github.com/flightaware/dump1090 "$STAGE/dump1090"
make -C "$STAGE/dump1090" -j"$JOBS" BLADERF=no
cp -v "$STAGE/dump1090/dump1090" "$APPDIR/usr/bin/"
endgroup

group "deck"
cp -v "$ROOT/target/release/deck" "$APPDIR/usr/bin/deck"
"$ROOT/target/release/deck" shot --icon "$APPDIR/deck.png"
cat > "$APPDIR/deck.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=deck
Comment=Handheld ham-radio RX machine for SDR cyberdecks
Exec=deck
Icon=deck
Categories=HamRadio;Audio;Network;
Terminal=false
EOF
cp "$APPDIR/deck.png" "$APPDIR/.DirIcon"
endgroup

group "bundle libraries (linuxdeploy)"
curl -fsSL -o "$STAGE/linuxdeploy" \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$ARCH.AppImage"
chmod +x "$STAGE/linuxdeploy"
exe_args=()
for b in "$APPDIR"/usr/bin/*; do
  exe_args+=("--executable=$b")
done
# linuxdeploy traces + copies + rpath-patches every executable's non-excluded
# libs. It writes its own AppRun/desktop; we overwrite AppRun below.
NO_STRIP=1 "$STAGE/linuxdeploy" --appimage-extract-and-run \
  --appdir "$APPDIR" "${exe_args[@]}" \
  --desktop-file "$APPDIR/deck.desktop" --icon-file "$APPDIR/deck.png"
endgroup

# Custom AppRun: put bundled tools on PATH so deck's `which()` finds them,
# and bundled libs on LD_LIBRARY_PATH for the tools + deck.
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
export LD_LIBRARY_PATH="$HERE/usr/lib:$HERE/usr/lib/$(uname -m)-linux-gnu:${LD_LIBRARY_PATH:-}"
exec "$HERE/usr/bin/deck" "$@"
EOF
chmod +x "$APPDIR/AppRun"

group "pack"
curl -fsSL -o "$STAGE/appimagetool" \
  "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
chmod +x "$STAGE/appimagetool"
ARCH="$ARCH" "$STAGE/appimagetool" --appimage-extract-and-run "$APPDIR" "$ROOT/deck-$ARCH.AppImage"
endgroup

echo "built deck-$ARCH.AppImage with bundled tools:"
ls -1 "$APPDIR/usr/bin"
