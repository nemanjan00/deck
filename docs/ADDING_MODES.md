# Adding a mode to deck

deck's modes live in one registry; the plumbing (device handling, DSP, UI
scaffolding, doctor, persistence) is shared. A new mode is usually four small
steps. Worked example: adding **CW** (morse listening on a USB demod with a
narrow filter) or a decoder-backed mode like **FLEX** pagers — the steps are
the same shape.

## 1. Register it — `src/modes.rs`

Add a `ModeId` variant and a `MODES` entry:

```rust
ModeDef {
    id: ModeId::Flex,
    key: "flex",                 // stable key for config files
    label: "FLEX",
    section: Section::Data,
    desc: "FLEX pager messages (via multimon-ng)",
    icon: "▤ ", icon_ascii: "=", // spare metadata (GUI paints by `key`)
    view: ViewKind::Pager,       // reuse an existing view…
    pipe: PipeKind::Iq(Demod::Nfm),
    decoder: Some("multimon-ng -t raw -a FLEX -"),
    decoder_rate: 22050,         // rate the decoder wants on stdin
    audio_out: false,            // monitor off by default (toggleable)
    default_hz: 929_612_500,
    presets: &[("US FLEX", 929_612_500)],
},
```

Pick the demod (`Nfm/Wfm/Am/Usb/Lsb/Raw`) and, if an external decoder is
involved, the command that reads s16le mono audio on stdin. That's the whole
contract. `PipeKind::Extern` is the escape hatch for tools that must own the
device themselves (see ADS-B).

**Windowed decoders** (decode a whole cycle at once, not a stream — e.g. FT8)
are a third shape: the mode is still `PipeKind::Iq`, but the plan sets
`windowed: true` (currently derived from `mode == ModeId::Ft8` in
`pipeline::resolve`). Instead of streaming stdin, deck buffers the demod audio
and, on a time boundary, writes a WAV and runs the decoder with `{wav}`
substituted (`decoder_rate` is the WAV rate). See `audio::ft8_window_thread`;
the decode is a one-shot `sh -c` whose stdout/stderr lines are fed back through
the normal `AppEvent::Line` → `apply_line` path.

**Icon:** add an arm in `src/gui/icons.rs::draw` for your `key` (a few painted
strokes); until you do, a placeholder circle renders.

## 2. Parse its output — `src/parse/` (only for new formats)

If the decoder emits a format deck doesn't know, write a parser (see
`multimon.rs` for line-shaped, `sbs.rs` for stateful/tabular) and route it in
`Session::apply_line` (`src/session.rs`) by matching your `ViewKind`.

Reusing an existing `ViewKind` (like `Pager` above) means zero parser work if
the output shape matches — multimon's FLEX lines parse like POCSAG's.

## 3. Give it a sim story — `src/sim/`

- **IQ-level** (best): extend `band.rs::build_profile` with a component so the
  sim device produces a real decodable signal for your mode.
- **Line-level** (fallback): add an arm in `sim/mod.rs::run_lines` emitting
  decoder-shaped lines. This is what digital voice uses.

If you skip this, the sim device falls back to a generic note.

## 4. (Only for new views) render it — `src/gui/modeview.rs`

A new `ViewKind` needs a `draw_*` function and an arm in `draw_right`. Reused
views need nothing.

## Checklist

- [ ] `cargo test` — `modes::tests::registry_consistent` guards the entry
- [ ] `deck doctor` — the mode appears in the support matrix automatically
- [ ] `deck shot` still renders (UI smoke)
- [ ] docs/STATUS.md — note anything unverified (e.g. decoder flag guesses)

## Teaching deck a new SDR

Different job, similar size: add the USB VID:PID to
`device.rs::USB_IDS`, tuning ranges in `SdrKind::ranges`, and an IQ source
template in `pipeline.rs::iq_source` (a command that streams cu8 or cs16 IQ
on stdout at a fixed rate). Everything else — retuning, DC-spike avoidance,
decimation planning — adapts automatically.
