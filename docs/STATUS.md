# Development status

*Update this when pausing work so development can resume cold.*

Last update: 2026-07-06 (pre-first-build)

## Done

- [x] Core: device autodetect, config/state, mode registry, plan resolver
- [x] Radio DSP + RX engine (IQ-hub), NR/NB/auto-notch, filters, squelch,
      AM env+sync, recording (squelch-aware WAV)
- [x] Parsers: multimon POCSAG/APRS, dsd call fields, SBS aircraft store
- [x] Simulator: IQ band compositor (all profiles), POCSAG/AX.25/Baudot
      encoders with decode-back tests, SBS fleet, dsd-style line feeds
- [x] Session layer: lifecycle, retune, scanner brain, stores, persistence
- [x] doctor report + selftest
- [x] sys integration: battery/volume/power

## In progress

- [ ] egui GUI (`src/gui/`): theme, icons, widgets, screens, shot/raster
- [ ] `main.rs` (clap dispatch)

## Next

- [ ] First build + `cargo test` + clippy pass (expect iteration)
- [ ] `deck shot` screenshot set (dark+light, big + 480×480)
- [ ] README (rich, screenshots), ADDING_MODES.md, CHANGELOG
- [ ] CI: build+test, release matrix (x86_64 + aarch64)
- [ ] tag v0.1.0

## Known caveats to document / revisit

- `airspyhf_rx` flag set varies between builds (`-r /dev/stdout`, output
  format); template is config-overridable, doctor shows the resolved cmd.
- dsd-neo frame flags (`-fs/-fy/-fd/-fi/-f1/-fz`) follow DSD-FME
  conventions; verify against the installed build, override in `[decoders]`.
- WFM is mono (no stereo pilot decode yet).
- Scanner hop across band edges restarts the source (~0.3–0.5 s); hops
  within ±1.2 MHz (RTL) / ±384 kHz (Airspy HF+) are instant.
- GUI screenshots: `deck shot` uses the built-in CPU rasterizer, no GPU.
