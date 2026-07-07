# Development status

*Update this when pausing work so development can resume cold.*

Last update: 2026-07-06 — v0.1.0 + waterfall browser/hand-off/span/readout,
memories, scanner PRIORITY channel (LEFT/P toggles, revisited every 4 hops),
peak->memory (LEFT/mem+), ADS-B offline RADAR view (VIEW control: rings,
track arrows, alt colors, trails, home via [adsb] lat/lon), F12 in-app
screenshot -> recordings/screens/.
Roadmap clearance session (2026-07-07): world-map radar layer, AIS mode,
CTCSS tone squelch, IF shift, cal_db, IQ recording, SWEEP, band-plan
overlay, HackRF (untested), rigctl trunk-following server. All tested where
testable without hardware; 73 tests green.
AppImage CI GREEN (both arches) — ci/build-appimage.sh bundles rtl_sdr,
multimon-ng, sox, minimodem, hackrf, airspyhf (apt) + mbelib-neo, dsd-neo,
rtl_ais, dump1090 (source); smoke test confirms every tool resolves. Bare
deck-<arch> binaries also publish to `continuous` on every main push.
Shakeout that got it there: mbelib-neo repo name (not mbe-neo); dsd-neo
built stdin-only (-DBUILD_TESTING=OFF + SoapySDR/RTL/UI off); linuxdeploy
leaves AppRun a symlink into usr/bin (rm before writing or it clobbers deck).
NOT YET run on real hardware / GL — see FIELD_TESTING.md.

NEXT: cut v0.2.0 (Unreleased changelog is complete). Remaining roadmap:
stereo WFM, DCS, absolute dBµV cal, Airspy R2 (fractional rates).

## Done (v0.1.0)

- [x] Core: sysfs device autodetect, config/state, mode registry, plan resolver
- [x] IQ-hub RX engine: NCO tune / decimate / NFM·WFM·AM(+SAM)·USB·LSB demod,
      NR / NB / auto-notch / HP·LP / squelch, squelch-aware WAV recording
- [x] Decoder integration via stdin audio: dsd-neo, multimon-ng, sox+minimodem;
      ADS-B via SBS :30003 (dump1090/readsb/rtl1090)
- [x] Parsers: multimon POCSAG/APRS, dsd call fields (slot/CC/TG/RID/NAC/RAN),
      SBS aircraft store — all tolerant, all tested
- [x] Simulator: IQ band compositor (BCH-correct POCSAG, AX.25/AFSK, Baudot
      RTTY, AM/SSB/WFM/NBFM babble, carriers), line feeds, standalone IQ gen
- [x] Session layer: lifecycle, instant in-band retune, scanner brain
      (dwell/hold/lockouts persisted), typed stores, run-id event filtering
- [x] egui GUI: tile menu w/ painted vector icons, digit-wheel tuner,
      spectrum/waterfall (drag-to-tune), call card, tables, scanner view,
      dark+light flat themes, splash, battery/volume/clock, power menu,
      full d-pad operation, 44px+ touch targets, square-screen layouts
- [x] `deck shot` headless CPU rasterizer (README screenshots + UI smoke)
- [x] doctor report + sim→decoder selftest
- [x] 65 tests green · clippy clean · fmt clean
- [x] README (screenshot gallery, mermaid), ADDING_MODES, CHANGELOG, CI +
      release workflows (x86_64 + aarch64)

## Field-testing TODO (needs real hardware/decoders)

- [ ] Verify `airspyhf_rx` flags + output format against a real HF+
      (template overridable; Doctor shows resolved command)
- [ ] Verify dsd-neo frame flags on an installed build (`-fs -fy -fd -fi -f1 -fz`)
- [ ] Run `deck doctor --selftest` on a box with multimon-ng/sox/minimodem
- [ ] GUI on target devices (GL/GLES via eframe glow); `--windowed` for desks
- [ ] Scanner dwell/hold tuning against live PMR/2m traffic

## Known caveats

- WFM mono (no stereo pilot decode)
- Scanner band-edge hops restart the source (~0.3–0.5 s); in-band instant
- GUI screenshots use the built-in rasterizer (no GPU path exercised in CI)
- Host-disk-full incident during development truncated 11 files mid-write;
  fully recovered (git + rewrites). Lesson encoded: scripted patches now use
  atomic temp+rename writes; commit cadence increased.

## Roadmap

IQ recording · stereo WFM · CTCSS/DCS · trunk following · priority-channel
scan · dBµV calibration · band plans · more SDRs (HackRF/Airspy R2 via
USB_IDS + iq_source template)

KC908 manual mined for ideas (user-provided); adopted: span zoom, marker/peak
readout, memories. Deliberately skipped for now: TX anything, field-strength
calibration (needs cal tables), IF shift.
