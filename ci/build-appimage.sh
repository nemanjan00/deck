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
  libusb-1.0-0-dev libsndfile1-dev libpulse-dev libssl-dev \
  libncurses-dev libairspyhf-dev \
  multimon-ng sox minimodem hackrf
# NOTE: we deliberately do NOT apt-install rtl-sdr / librtlsdr-dev. The apt
# librtlsdr (0.6.0) predates the RTL-SDR Blog V4 (R828D tuner) and cannot
# tune it. We build the rtl-sdr-blog fork below instead — it drives V4, V3
# and clones, and rtl_ais + dump1090 link against it.
# airspyhf tools live in different packages across releases; best-effort.
sudo apt-get install -y --no-install-recommends airspyhf || \
  echo "WARN: 'airspyhf' package unavailable — airspyhf_rx may be missing"
endgroup

group "librtlsdr (rtl-sdr-blog fork — required for RTL-SDR Blog V4)"
git clone --depth 1 https://github.com/rtlsdrblog/rtl-sdr-blog "$STAGE/rtl-sdr-blog"
cmake -S "$STAGE/rtl-sdr-blog" -B "$STAGE/rtl-sdr-blog/build" \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local \
  -DINSTALL_UDEV_RULES=OFF -DDETACH_KERNEL_DRIVER=ON
cmake --build "$STAGE/rtl-sdr-blog/build" -j"$JOBS"
sudo cmake --install "$STAGE/rtl-sdr-blog/build"
sudo ldconfig
endgroup

# Copy an apt-provided binary into the AppDir if present.
take() {
  if p="$(command -v "$1" 2>/dev/null)"; then
    cp -v "$p" "$APPDIR/usr/bin/"
  else
    echo "WARN: $1 not found (mode using it will show 'needs tools')"
  fi
}
group "collect tools"
# rtl_sdr/rtl_fm/rtl_test come from the blog fork in /usr/local/bin (found
# first on PATH); the rest from apt.
for b in rtl_sdr rtl_fm rtl_test multimon-ng sox minimodem hackrf_transfer airspyhf_rx; do
  take "$b"
done
endgroup

group "mbelib-neo (source) — dsd-neo's AMBE/IMBE vocoder dependency"
# repo is "mbelib-neo"; the CMake package it provides is named "mbe-neo".
# dsd-neo needs the 2.x soft-decision API (1.x is not supported).
git clone --depth 1 https://github.com/arancormonk/mbelib-neo "$STAGE/mbelib-neo"
cmake -S "$STAGE/mbelib-neo" -B "$STAGE/mbelib-neo/build" -DCMAKE_BUILD_TYPE=Release
cmake --build "$STAGE/mbelib-neo/build" -j"$JOBS"
sudo cmake --install "$STAGE/mbelib-neo/build"
sudo ldconfig
endgroup

group "dsd-neo (source)"
# deck feeds dsd-neo audio on stdin (-i -) and takes -o pulse, so it needs
# NO radio backend and no terminal UI of its own. Disabling SoapySDR (ON by
# default, and not installed), RTL-SDR and the ncurses UI strips it to its
# REQUIRED deps only: mbe-neo + libsndfile + OpenSSL + PulseAudio.
git clone --depth 1 https://github.com/arancormonk/dsd-neo "$STAGE/dsd-neo"
# BUILD_TESTING=OFF drops the test suite (it fails under -Werror on GCC 11);
# we only need the app binary (lives in apps/dsd-cli).
cmake -S "$STAGE/dsd-neo" -B "$STAGE/dsd-neo/build" \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH=/usr/local \
  -DBUILD_TESTING=OFF \
  -DDSD_ENABLE_SOAPYSDR=OFF -DDSD_ENABLE_RTLSDR=OFF \
  -DDSD_ENABLE_TERMINAL_UI=OFF \
  -DDSD_WARNINGS_AS_ERRORS=OFF
cmake --build "$STAGE/dsd-neo/build" -j"$JOBS"
# the CLI binary name has varied (dsd-neo / dsd-cli / dsd); grab whatever
# built, excluding the test binaries, and install it as `dsd-neo`.
found="$(find "$STAGE/dsd-neo/build" -type f -executable \
  \( -name 'dsd-neo' -o -name 'dsd-cli' -o -name 'dsd' \) \
  -not -path '*/tests/*' -not -name '*.so*' | head -1)"
test -n "$found" || { echo "dsd-neo binary not found in build tree"; \
  find "$STAGE/dsd-neo/build" -type f -executable -not -name '*.so*'; exit 1; }
cp -v "$found" "$APPDIR/usr/bin/dsd-neo"
endgroup

# both link librtlsdr — point them at the blog fork in /usr/local so they
# inherit V4 support (pkg-config for dump1090; -I/-L for rtl-ais).
export PKG_CONFIG_PATH="/usr/local/lib/pkgconfig:${PKG_CONFIG_PATH:-}"

group "rtl-ais (source)"
git clone --depth 1 https://github.com/dgiardini/rtl-ais "$STAGE/rtl-ais"
make -C "$STAGE/rtl-ais" -j"$JOBS" \
  CFLAGS="-O2 -I/usr/local/include" LDFLAGS="-L/usr/local/lib"
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
# NB: linuxdeploy leaves AppRun as a SYMLINK into usr/bin/deck — remove it
# first, or `cat >` follows the symlink and clobbers the deck binary (which
# then makes AppRun's $HERE resolve to usr/bin, doubling the exec path).
rm -f "$APPDIR/AppRun"
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
# $APPDIR is set by the AppImage runtime; fall back to $0's dir otherwise.
HERE="${APPDIR:-$(dirname "$(readlink -f "$0")")}"
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
