# First-boot field testing checklist

Ordered so each step validates the layer below it. Everything here is the
gap between "proven in CI" and "proven on air".

## 0. Desktop, no hardware (5 min)
- [ ] `deck --windowed` → splash → menu renders, d-pad + mouse both work
- [ ] Devices shows **Simulator**; NFM → START → babble audio + live spectrum
      (validates: engine → DSP → sink → GUI event loop)
- [ ] POCSAG on sim: pages decode into the table (validates the full
      IQ → demod → resample → multimon stdin → parser chain *if* multimon-ng
      is installed; falls back to line-sim otherwise)
- [ ] Waterfall on sim: peaks appear; OPEN IN hands off; SWEEP completes

## 1. Decoder integration (1 min)
- [ ] `deck doctor --selftest` — POCSAG + APRS via multimon-ng, RTTY via
      sox+minimodem. **This is the audio-scaling verdict.** If FAIL: adjust
      the discriminator scale (FmDemod dev in `audio.rs`) or decoder flags.

## 2. RTL-SDR on air
- [ ] NFM on a known-active frequency (PMR/repeater): audio, S-meter,
      squelch feel; adjust `[sdr] gain`, per-mode squelch
- [ ] POCSAG on a live pager channel — the decisive end-to-end test
- [ ] APRS 144.800/144.390: packets in the table
- [ ] Waterfall: drag-to-tune, peak list vs what you see
- [ ] Scanner across PMR446: hold/dwell feel (`[scanner]`), lockout, priority
- [ ] ADS-B: dump1090 spawns, SBS connects, radar populates; set
      `[adsb] lat/lon`
- [ ] AIS (near water): `rtl_ais` template flags correct?
- [ ] Device-busy: start RX twice / with gqrx open → friendly error, no wedge

## 3. dsd-neo (digital voice)
- [ ] DMR hotspot/repeater: sync, call card fields (slot/CC/TG/RID)
- [ ] Verify frame flags against your build: `-fs -fy -fd -fi -f1 -fz`
      (override per mode in `[decoders]` if they differ)
- [ ] Trunking (experimental): add `-U 4532` to the dmr decoder override,
      confirm dsd-neo connects and `F <hz>` retunes land (raw log shows them)

## 4. Airspy HF+ / HackRF (templates are best-effort)
- [ ] `airspyhf_rx` flags/output format — fix `[pipelines.airspyhf]
      iq_source` if needed; Doctor prints the resolved command
- [ ] HackRF: `hackrf_transfer -r -` stdout streaming + gain flags

## 5. Handheld (Hackberry/Comet class)
- [ ] GL/GLES launch fullscreen; theme readable outdoors (T toggles)
- [ ] Touch: digit wheel, waterfall drag, 44px targets honest on the panel
- [ ] CPU % during NFM and Waterfall (if hot: FIR decimators are the
      optimization target), battery drain, volume keys vs mixer
- [ ] Power menu: suspend/resume with RX running (expect: source dies via
      PDEATHSIG semantics? verify restart behavior), poweroff
- [ ] F12 screenshot lands in recordings/screens

## Known-unverified list (mirror of STATUS.md)
airspyhf_rx flags · hackrf template · dsd-neo flags + rigctl client
behavior · rtl_ais `-n` output mode · live GUI loop on GL · CM5 CPU budget
