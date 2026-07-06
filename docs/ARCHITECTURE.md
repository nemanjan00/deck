# deck — architecture

*Keep this file current: it is the map for whoever (or whatever) continues
development.*

## What deck is

A fullscreen touch GUI (egui/eframe) that turns an SDR cyberdeck into a
handheld ham-radio RX machine. deck **owns the SDR**: exactly one source
process per session (`rtl_sdr`, `airspyhf_rx`, or the built-in simulator)
streams raw IQ into deck, which tunes/decimates/demodulates internally and
fans audio out to the speaker and/or external decoders' stdin (`multimon-ng`,
`dsd-neo`, `minimodem`). Only ADS-B runs as a fully external pipeline
(`dump1090` owns the device; deck consumes its SBS TCP feed — which also
makes readsb/rtl1090 drop-in compatible).

```
                       ┌────────────────────────── deck ──────────────────────────┐
 rtl_sdr / airspyhf_rx │  IQ → NB → NCO tune → decimate → chan LP → demod          │
 / deck simgen ───────►│        │                                │                │
   (one source proc,   │        └── band FFT (waterfall/scope)   ├─ raw audio ────┼─► decoder stdin
    owns the SDR)      │                                         │  (resampled)   │   multimon-ng / dsd-neo /
                       │              monitor path: HP/LP → auto-notch → NR       │   sox|minimodem
                       │                  → squelch → sink (paplay/pw-cat/aplay)  │        │
                       │                  → WAV recorder (squelch-aware)          │   stdout lines
                       │              audio FFT (spectrum view)                   │        ▼
                       └──────────────────────────────────────────────────────────┘   parsers → stores → GUI
```

## Module map (src/)

| module | role | key types |
|---|---|---|
| `main.rs` | clap dispatch: GUI (default), `simgen`, `doctor`, `config`, `shot` | |
| `session.rs` | UI-agnostic app state: start/stop/retune, event application, scanner brain, persistence | `Session`, `Running`, `Stores`, `ScanState` |
| `modes.rs` | THE mode registry — add modes here | `ModeId`, `ModeDef`, `MODES`, `Demod`, `PipeKind`, `ViewKind` |
| `pipeline.rs` | plan resolution (mode×device→plan), templates, process supervision (pgroups, PDEATHSIG, busy detection), SBS client | `Plan`, `resolve()`, `Spawned`, `AppEvent` |
| `audio.rs` | the RX engine: pump thread does all live DSP; knobs are atomics the GUI writes | `RxEngine`, `Knobs`, `EngineParams` |
| `dsp.rs` | DSP primitives: NCO, FIR decimators, FM/SAM/SSB demod, biquads, filter chain, spectral NR, noise blanker, LMS auto-notch, FFT, RNG | |
| `device.rs` | sysfs USB autodetect (no libusb), tuning ranges, always-present sim device | `SdrDevice`, `SdrKind` |
| `freq.rs` | frequency digit editor + formatting | `FreqInput` |
| `config.rs` | `~/.config/deck/config.toml` + persisted state (`state.toml`) | `Config`, `PersistState`, `ModePersist` |
| `parse/` | decoder-output parsers: `multimon` (POCSAG/APRS), `dsd` (call fields), `sbs` (aircraft) | |
| `sim/` | `deck simgen`: IQ band compositor (`band.rs`), POCSAG/AX.25/Baudot encoders, SBS fleet, dsd-style line feeds | |
| `rec.rs` | WAV writer + recordings dir | `WavWriter` |
| `sys.rs` | battery (sysfs), volume (wpctl→pactl→amixer), power actions (systemctl) | `SysMon` |
| `doctor.rs` | environment report + sim→decoder selftest | |
| `gui/` | egui front-end (see below) | |

## Invariants / design decisions

- **One SDR owner.** A session spawns exactly one source process; stop/retune
  uses `kill_group_wait` so the USB interface is truly released before the
  next open. stderr lines matching `looks_like_device_busy` set a friendly
  "device in use" hint.
- **Run ids.** Every background thread tags events with its `run`; stale
  events from a previous session are dropped in `Session::drain_events`.
- **Decoders get raw demod audio** (pre user-filters/NR) — decode integrity
  beats listening comfort. The monitor path is where NR/notch/filters live.
- **In-band retune is an atomic offset write** (`Knobs::offset_hz`); the
  device only re-opens when the new frequency leaves the captured band.
  The device centers `rate/4` away from the tuned freq (DC spike avoidance).
- **Everything works without hardware**: the sim device feeds the *real* RX
  chain and *real* decoders (`simgen --mode iq-band`), and degrades to
  decoded-line feeds when decoders aren't installed. Digital voice sim is
  always a line feed (no real C4FM synthesis).
- **Offline.** No network use except localhost SBS. All assets compiled in.
- **Input contract:** everything operable with arrows + Enter/Esc (Flipper
  d-pad style); touch and letter keys are accelerators, never requirements.
- **arm64 + x86_64 Linux** are the release targets.

## Adding a mode (checklist)

1. `modes.rs`: add `ModeId` variant + `MODES` entry (demod, decoder template,
   `decoder_rate`, view kind, presets, icon).
2. If it needs new decoder-output parsing: add a parser in `parse/`, route it
   in `Session::apply_line` (match on `ViewKind`).
3. Sim story: extend `sim/band.rs::build_profile` (IQ-level) and/or
   `sim/mod.rs::run_lines` (line feed).
4. If the view is new: add a screen renderer in `gui/`.
5. `doctor.rs` picks it up automatically via the registry.

See `docs/ADDING_MODES.md` for a worked example.

## GUI (egui/eframe)

- `gui/mod.rs` — `DeckApp: eframe::App`: screen routing, d-pad focus model,
  key handling, event pump (`Session::tick` per frame + repaint scheduling).
- `gui/theme.rs` — flat dark ("hacker") + light themes; all colors here.
- `gui/icons.rs` — vector icons painted with egui primitives (no assets).
- `gui/widgets.rs` — tile, digit-wheel freq tuner (drag/scroll per digit),
  spectrum, waterfall (texture), S-meter, flat buttons, status bar.
- `gui/screens/` — menu (tile grid), mode screens per `ViewKind`, devices,
  doctor, power menu, splash.
- `gui/shot.rs` + `gui/raster.rs` — headless screenshots: egui → tessellate →
  own CPU rasterizer → PNG. No GPU needed; also serves as a UI smoke test.

## Testing

- `cargo test` covers: DSP correctness (demods recover tones, filters shape
  spectra, NR/NB/notch measurably work), POCSAG/AX.25/Baudot encoders decode
  back through test-side receivers, parsers against real-world-shaped lines,
  plan resolution, config round-trips.
- `deck doctor --selftest` pipes sim signals through the *installed real
  decoders* and checks the decodes (POCSAG/APRS via multimon-ng, RTTY via
  sox+minimodem).
- `deck shot` renders every screen headlessly (CI-able UI smoke test).
