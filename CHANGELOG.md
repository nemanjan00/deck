# Changelog

## Unreleased

- Waterfall signal browser: automatic peak detection (noise-floor tracked,
  DC-spike excluded, persistence-smoothed); tap/OK a peak to tune, RIGHT to
  hand off
- Hand-off: OPEN IN control / double-tap opens the tuned frequency in any
  mode and starts RX
- KC908-inspired: SPAN zoom (full/2/4/8x around the marker), MKR/PK
  measurement readout on the scope, memory channels (saved from any mode,
  starred in preset picker, persisted)
- Scanner priority channel (persisted; revisited every 4 hops; LEFT/P)
- Peak list: LEFT / mem+ saves a peak straight to memory channels
- ADS-B offline radar view: range rings, home QTH ([adsb] lat/lon or traffic
  centroid), track-rotated altitude-colored arrows, position trails
- F12 in-app screenshot to recordings/screens/ (field logging)
- Radar gained real geography: embedded Natural Earth coastlines + borders
  (68 KB, public domain, offline)
- AIS mode: AIVDM decoder (1/2/3/18 positions, 5/24A names, fragments,
  checksums), rtl_ais template, boat-fleet simulator — ships share the
  radar/table with aircraft
- CTCSS tone squelch: 38-tone Goertzel detector, live tone chip, gate +
  recorder require the selected tone
- SSB IF shift (±800 Hz, pitch-preserving), [sdr] cal_db level calibration
- Waterfall: raw IQ recording (.cu8/.cs16 + sidecar), SWEEP
  search-between-limits feeding the peak browser, band-plan overlay
- HackRF One support (cs8, hackrf_transfer template, author-untested)
- rigctl server for dsd-neo trunk following (-U 4532): decoder-driven
  retunes; in-band hops are instant

## 0.1.0 — 2026-07-06

First release. deck is a fullscreen touch GUI that turns an SDR cyberdeck
into a handheld wideband receiver.

### Receiver
- IQ-hub architecture: deck owns the SDR through one source process
  (`rtl_sdr` / `airspyhf_rx` / built-in simulator) and does tuning,
  decimation and demodulation in-process
- Demodulators: NFM, WFM (mono), AM (envelope + synchronous/SAM), USB, LSB
- Instant in-band retunes (atomic NCO offset); device-releasing restarts
  across band edges; DC-spike avoidance via rate/4 offset tuning
- DSP toolkit on the monitor path: spectral noise reduction, IQ noise
  blanker, LMS auto-notch, biquad HP/LP ladders, hysteresis squelch
- Decoders fed raw demod audio on stdin: dsd-neo (DMR/YSF/D-STAR/NXDN/
  P25/M17), multimon-ng (POCSAG/APRS), sox+minimodem (RTTY)
- ADS-B via any SBS server on :30003 (dump1090, readsb, rtl1090)
- Squelch-aware WAV recording (48 kHz mono, timestamped files)

### Modes & views
- 17 modes: NFM WFM AM USB LSB · DMR YSF D-STAR NXDN P25 M17 ·
  POCSAG APRS RTTY ADS-B · Scanner, Waterfall
- Live DMR call card (TG, SRC, slot, color code), call history
- Pager/APRS tables with detail popups, aircraft table with aging,
  RTTY text feed
- Channel scanner: dwell/hold, lockouts (persisted), hit log,
  per-channel activity counters
- Band scope + waterfall with drag-to-tune and click-to-jump

### UI
- egui/eframe fullscreen app; flat dark ("hacker") and light themes
- Portapack-style tile menu with painted vector icons (no image assets)
- Per-digit frequency tuner: tap to select, drag/scroll to spin
- 100% d-pad operable (arrows + OK/Back); touch-first sizing (44 px+)
- Status bar: battery, volume (wpctl/pactl/amixer), clock, REC badge;
  power menu (suspend/reboot/poweroff via logind)
- Splash screen; per-mode persistence of freq/gain/DSP settings

### Simulator & testing
- `deck simgen`: IQ band compositor (NBFM babble, real POCSAG/BCH bursts,
  AX.25/AFSK packets, Baudot RTTY, AM/SSB/WFM, drifting carriers), decoder
  audio generators, decoded-line feeds — deterministic via seed
- `deck doctor [--selftest]`: environment report + sim→real-decoder checks
- `deck shot`: headless CPU rasterizer renders every screen to PNG
  (no GPU) — doubles as UI smoke test and generates the README shots
- 65 unit tests: demod correctness, NR/NB/notch efficacy, encoder
  round-trips (POCSAG BCH, AX.25 CRC/HDLC via Goertzel, Baudot), parsers

### Platform
- Linux x86_64 + aarch64; offline-only; sysfs USB autodetect (no libusb)
- Targets small handhelds: Hackberry Pi CM5, Mecha Comet, Cardputer
  Zero-class, square 480×480 layouts supported
