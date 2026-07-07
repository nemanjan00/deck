//! The RX engine: one IQ source process per session (exclusive SDR owner),
//! internal DSP (tune → decimate → demod → clean up), fan-out to the audio
//! sink and/or a decoder's stdin, spectrum/RMS frames for the UI.
//!
//! Tap points, in order:
//!   IQ → [noise blanker] → NCO tune → decimate → channel filter → demod
//!      ├─ decoder feed: RAW demod audio (resampled) — decode integrity first
//!      └─ monitor path: HP/LP filters → auto-notch → NR → squelch → sink
//!
//! In-band retunes are an atomic offset swap (instant — this is what makes
//! the scanner fast); crossing the band edge restarts the source process.

use crate::config::Config;
use crate::dsp::CtcssDet;
use crate::dsp::{
    self, decim_factors, Agc, AutoNotch, DcBlock, Deemphasis, FilterChain, FirDecim, FirDecimF32,
    FmDemod, Nco, NoiseBlanker, Resampler, SamDemod, SpectralNr, SpectrumFft, SsbDemod,
};
use crate::modes::Demod;
use crate::pipeline::{
    attach_line_readers, kill_group, kill_group_wait, spawn_shell, AppEvent, IqFormat, Spawned,
};
use rustfft::num_complex::Complex32;
use std::io::{Read, Write};
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Shared knobs the DSP thread reads every chunk; the UI writes them live.
pub struct Knobs {
    /// NCO offset in Hz: tuned_freq - device_center
    pub offset_hz: AtomicI64,
    /// spectral NR over-subtraction factor bits (0.0 = off)
    pub nr: AtomicU32,
    /// noise blanker threshold factor bits (0.0 = off)
    pub nb: AtomicU32,
    pub notch: AtomicBool,
    pub hp_hz: AtomicU32,
    pub lp_hz: AtomicU32,
    /// squelch RMS threshold bits (0.0 = open)
    pub squelch: AtomicU32,
    /// monitor muted (sink gets silence-free writes skipped)
    pub mute: AtomicBool,
    /// AM detector: envelope (false) or synchronous (true)
    pub sync_det: AtomicBool,
    /// CTCSS tone squelch in centi-Hz (0 = off); gate needs this tone
    pub tone_chz: AtomicU32,
    /// SSB passband (IF) shift in Hz
    pub if_shift: AtomicI32,
    /// Some(path) = record the monitor audio to this WAV; None = stop.
    pub record: Mutex<Option<std::path::PathBuf>>,
    /// RIFF INFO comment embedded in recordings (callsign/TG/mode/freq)
    pub rec_meta: Mutex<String>,
}

pub fn f32_bits(v: f32) -> u32 {
    v.to_bits()
}

pub fn bits_f32(b: u32) -> f32 {
    f32::from_bits(b)
}

impl Knobs {
    pub fn new(offset_hz: i64) -> Arc<Self> {
        Arc::new(Self {
            offset_hz: AtomicI64::new(offset_hz),
            nr: AtomicU32::new(0),
            nb: AtomicU32::new(0),
            notch: AtomicBool::new(false),
            hp_hz: AtomicU32::new(0),
            lp_hz: AtomicU32::new(0),
            squelch: AtomicU32::new(0),
            mute: AtomicBool::new(false),
            sync_det: AtomicBool::new(false),
            tone_chz: AtomicU32::new(0),
            if_shift: AtomicI32::new(0),
            record: Mutex::new(None),
            rec_meta: Mutex::new(String::new()),
        })
    }
}

/// Resolve the audio sink command for 48 kHz s16le mono.
/// PipeWire/Pulse first; ALSA as the last resort.
pub fn resolve_sink(cfg: &Config, tools: &crate::pipeline::ToolReport) -> Option<String> {
    let t = if cfg.audio.sink == "auto" {
        if tools.has("paplay") {
            "paplay --raw --rate={rate} --format=s16le --channels=1".to_string()
        } else if tools.has("pw-cat") {
            "pw-cat -p --rate {rate} --channels 1 --format s16 -".to_string()
        } else if tools.has("aplay") {
            "aplay -q -t raw -f S16_LE -r {rate} -c 1 -".to_string()
        } else {
            return None;
        }
    } else {
        cfg.audio.sink.clone()
    };
    Some(t.replace("{rate}", "48000"))
}

struct StdinSink(Arc<Mutex<Option<ChildStdin>>>);

impl StdinSink {
    fn write(&self, bytes: &[u8]) {
        if let Ok(mut g) = self.0.lock() {
            if let Some(w) = g.as_mut() {
                if w.write_all(bytes).is_err() {
                    *g = None; // consumer died; stop feeding, keep running
                }
            }
        }
    }
}

pub struct EngineParams {
    pub run: u64,
    pub source_cmdline: String,
    pub format: IqFormat,
    pub rate: u32,
    pub demod: Demod,
    pub decoder_cmd: Option<String>,
    pub decoder_rate: u32,
    pub decoder_char_mode: bool,
    pub sink_cmd: Option<String>,
    /// decoder writes decoded audio to stdout (dsd-neo -o -): deck plays +
    /// records it, and the IQ pump does not (it only feeds the decoder).
    pub decoder_audio: bool,
}

pub struct RxEngine {
    run: u64,
    format: IqFormat,
    rate: u32,
    demod: Demod,
    source: Option<Spawned>,
    source_stop: Arc<AtomicBool>,
    decoder: Option<Spawned>,
    decoder_stdin: Option<Arc<Mutex<Option<ChildStdin>>>>,
    decoder_rate: u32,
    sink: Option<Spawned>,
    sink_stdin: Option<Arc<Mutex<Option<ChildStdin>>>>,
    pub knobs: Arc<Knobs>,
    tx: Sender<AppEvent>,
    /// record the IQ monitor path here (false when a Voice decoder owns audio)
    record_in_pump: bool,
}

impl RxEngine {
    pub fn start(
        p: EngineParams,
        knobs: Arc<Knobs>,
        tx: Sender<AppEvent>,
    ) -> std::io::Result<Self> {
        // sink first, so a Voice decoder's decoded-audio stdout can feed it
        let (sink, sink_stdin) = match &p.sink_cmd {
            Some(cmd) => {
                let mut sp = spawn_shell(cmd, false, true)?;
                let stdin = Arc::new(Mutex::new(sp.child.stdin.take()));
                (Some(sp), Some(stdin))
            }
            None => (None, None),
        };
        let (decoder, decoder_stdin) = match &p.decoder_cmd {
            Some(cmd) => {
                let mut sp = spawn_shell(cmd, true, true)?;
                let stdin = Arc::new(Mutex::new(sp.child.stdin.take()));
                if p.decoder_audio {
                    // stdout = decoded 48 kHz s16le audio → deck's sink +
                    // recorder (postprocessing lives here); stderr = call info.
                    if let Some(stderr) = sp.child.stderr.take() {
                        let (tx2, run) = (tx.clone(), p.run);
                        std::thread::spawn(move || {
                            crate::pipeline::read_stream(
                                stderr,
                                run,
                                crate::pipeline::LineSrc::Stderr,
                                tx2,
                                false,
                            )
                        });
                    }
                    if let Some(stdout) = sp.child.stdout.take() {
                        let snk = sink_stdin.clone().map(StdinSink);
                        let (knobs2, tx2, run) = (knobs.clone(), tx.clone(), p.run);
                        std::thread::spawn(move || pump_decoded(stdout, snk, knobs2, tx2, run));
                    }
                } else {
                    attach_line_readers(&mut sp, p.run, &tx, p.decoder_char_mode);
                }
                (Some(sp), Some(stdin))
            }
            None => (None, None),
        };
        let mut eng = Self {
            run: p.run,
            format: p.format,
            rate: p.rate,
            demod: p.demod,
            source: None,
            source_stop: Arc::new(AtomicBool::new(false)),
            decoder,
            decoder_stdin,
            decoder_rate: p.decoder_rate,
            sink,
            sink_stdin,
            knobs,
            tx,
            record_in_pump: !p.decoder_audio,
        };
        eng.spawn_source(&p.source_cmdline)?;
        Ok(eng)
    }

    fn spawn_source(&mut self, cmdline: &str) -> std::io::Result<()> {
        let mut sp = spawn_shell(cmdline, true, false)?;
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(stderr) = sp.child.stderr.take() {
            let tx = self.tx.clone();
            let run = self.run;
            std::thread::spawn(move || {
                crate::pipeline::read_stream(
                    stderr,
                    run,
                    crate::pipeline::LineSrc::Stderr,
                    tx,
                    false,
                )
            });
        }
        if let Some(stdout) = sp.child.stdout.take() {
            let cfg = PumpCfg {
                format: self.format,
                rate: self.rate,
                demod: self.demod,
                decoder_rate: self.decoder_rate,
                record: self.record_in_pump,
            };
            let knobs = self.knobs.clone();
            let tx = self.tx.clone();
            let run = self.run;
            let dec = self.decoder_stdin.clone().map(StdinSink);
            let snk = self.sink_stdin.clone().map(StdinSink);
            let stop2 = stop.clone();
            std::thread::spawn(move || pump(stdout, cfg, knobs, dec, snk, tx, run, stop2));
        }
        self.source_stop = stop;
        self.source = Some(sp);
        Ok(())
    }

    /// Move the device center: kill the source, WAIT for the SDR to be
    /// released, then bring up a new source. Decoder + sink stay warm.
    pub fn retune_center(&mut self, new_source_cmdline: &str) -> std::io::Result<()> {
        self.source_stop.store(true, Ordering::Relaxed);
        if let Some(sp) = self.source.take() {
            kill_group_wait(sp);
        }
        self.spawn_source(new_source_cmdline)
    }

    pub fn stop(mut self) {
        self.source_stop.store(true, Ordering::Relaxed);
        if let Some(sp) = self.source.take() {
            kill_group_wait(sp); // release the SDR before we return
        }
        if let Some(stdin) = &self.decoder_stdin {
            if let Ok(mut g) = stdin.lock() {
                *g = None; // EOF lets the decoder flush and exit
            }
        }
        if let Some(sp) = self.decoder.take() {
            kill_group(sp.pgid);
        }
        if let Some(stdin) = &self.sink_stdin {
            if let Ok(mut g) = stdin.lock() {
                *g = None;
            }
        }
        if let Some(sp) = self.sink.take() {
            kill_group(sp.pgid);
        }
    }
}

struct PumpCfg {
    format: IqFormat,
    rate: u32,
    demod: Demod,
    decoder_rate: u32,
    /// record the monitor path (false when a decoder owns the audio output)
    record: bool,
}

/// Per-demod DSP state.
enum Detector {
    Fm(FmDemod),
    Wfm {
        fm: FmDemod,
        deemph: Deemphasis,
        decim: FirDecimF32,
    },
    Am {
        dc: DcBlock,
        agc: Agc,
        sam: SamDemod,
    },
    Ssb(SsbDemod),
    Raw,
}

/// Read a Voice decoder's DECODED audio (dsd-neo `-o -`, 48 kHz s16le mono
/// on its stdout), play it to the sink, and record it. This is where deck
/// owns the digital-voice audio; the discriminator pump only feeds the
/// decoder. Ends when the decoder's stdout closes.
fn pump_decoded(mut stdout: impl Read, sink: Option<StdinSink>, knobs: Arc<Knobs>, tx: Sender<AppEvent>, run: u64) {
    let mut buf = vec![0u8; 8192];
    let mut recorder: Option<crate::rec::WavWriter> = None;
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let pcm = &buf[..n];
        if let Some(snk) = &sink {
            if !knobs.mute.load(Ordering::Relaxed) {
                snk.write(pcm);
            }
        }
        let want = knobs.record.lock().ok().and_then(|g| g.clone());
        match (&want, recorder.is_some()) {
            (Some(path), false) => {
                if let Ok(w) = crate::rec::WavWriter::create(path, 48_000) {
                    let _ = tx.send(AppEvent::Rec {
                        run,
                        path: Some(w.path.to_string_lossy().into_owned()),
                    });
                    recorder = Some(w);
                }
            }
            (None, true) => {
                if let Some(w) = recorder.take() {
                    let meta = knobs.rec_meta.lock().ok().map(|m| m.clone()).unwrap_or_default();
                    let _ = w.finalize(Some(&meta));
                }
                let _ = tx.send(AppEvent::Rec { run, path: None });
            }
            _ => {}
        }
        if let Some(w) = &mut recorder {
            if w.write_s16(pcm).is_err() {
                recorder = None;
            }
        }
    }
    if let Some(w) = recorder.take() {
        let meta = knobs.rec_meta.lock().ok().map(|m| m.clone()).unwrap_or_default();
        let _ = w.finalize(Some(&meta));
        let _ = tx.send(AppEvent::Rec { run, path: None });
    }
}

#[allow(clippy::too_many_arguments)]
fn pump(
    mut stdout: impl Read,
    cfg: PumpCfg,
    knobs: Arc<Knobs>,
    decoder: Option<StdinSink>,
    sink: Option<StdinSink>,
    tx: Sender<AppEvent>,
    run: u64,
    stop: Arc<AtomicBool>,
) {
    // ---- chain construction -------------------------------------------
    let complex_target: u32 = match cfg.demod {
        Demod::Wfm => {
            if cfg.rate % 240_000 == 0 {
                240_000
            } else {
                192_000
            }
        }
        _ => 48_000,
    };
    let Some(factors) = decim_factors(cfg.rate, complex_target) else {
        let _ = tx.send(AppEvent::Line {
            run,
            src: crate::pipeline::LineSrc::Stderr,
            text: format!("deck: no decimation plan {} -> {complex_target}", cfg.rate),
        });
        return;
    };
    let mut decims: Vec<FirDecim> = factors.into_iter().map(FirDecim::new).collect();
    let mut nco = Nco::new(0.0, cfg.rate as f64);
    let mut cur_offset: i64 = i64::MIN; // force initial set
    let mut nb = NoiseBlanker::new();

    // channel low-pass at demod rate (keeps neighbours out)
    let chan_fc = match cfg.demod {
        Demod::Nfm => Some(8_000.0f32),
        Demod::Am => Some(6_000.0f32),
        _ => None,
    };
    let mut chan_i: Vec<dsp::Biquad> = Vec::new();
    let mut chan_q: Vec<dsp::Biquad> = Vec::new();
    if let Some(fc) = chan_fc {
        let bq = dsp::Biquad::lowpass(complex_target as f32, fc, 0.707);
        chan_i = vec![bq; 2];
        chan_q = vec![bq; 2];
    }

    let mut det = match cfg.demod {
        Demod::Nfm => Detector::Fm(FmDemod::new(48_000.0, 4_000.0)),
        Demod::Wfm => Detector::Wfm {
            fm: FmDemod::new(complex_target as f32, 75_000.0),
            deemph: Deemphasis::new(complex_target as f32, 50e-6),
            decim: FirDecimF32::new((complex_target / 48_000) as usize),
        },
        Demod::Am => Detector::Am {
            dc: DcBlock::new(),
            agc: Agc::new(0.5),
            sam: SamDemod::new(),
        },
        Demod::Usb => Detector::Ssb(SsbDemod::new(48_000.0, true)),
        Demod::Lsb => Detector::Ssb(SsbDemod::new(48_000.0, false)),
        Demod::Raw => Detector::Raw,
    };

    let mut filters = FilterChain::new(48_000);
    let mut notch = AutoNotch::new();
    let mut nr = SpectralNr::new();
    let mut resampler = (cfg.decoder_rate != 0 && cfg.decoder_rate != 48_000)
        .then(|| Resampler::new(48_000, cfg.decoder_rate));

    let audio_fft = SpectrumFft::new(512);
    let iq_fft = SpectrumFft::new(1024);
    let mut audio_acc: Vec<f32> = Vec::new();
    let mut last_audio = Instant::now() - Duration::from_secs(1);
    let mut last_iq = Instant::now() - Duration::from_secs(1);
    let mut gate_open = false;
    let mut recorder: Option<crate::rec::WavWriter> = None;
    let mut ctcss = CtcssDet::new(48_000);
    let mut cur_if_shift: i32 = 0;
    let mut iq_rec: Option<std::fs::File> = None;

    let bytes_per = match cfg.format {
        IqFormat::Cu8 | IqFormat::Cs8 => 2usize,
        IqFormat::Cs16 => 4usize,
        IqFormat::F32 => 8usize,
    };
    let mut raw = vec![0u8; 16384 * bytes_per];
    let mut leftover: Vec<u8> = Vec::new();

    // ---- pump loop ------------------------------------------------------
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let n = match stdout.read(&mut raw) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut bytes: Vec<u8> = Vec::with_capacity(leftover.len() + n);
        bytes.extend_from_slice(&leftover);
        bytes.extend_from_slice(&raw[..n]);
        let usable = bytes.len() - bytes.len() % bytes_per;
        leftover = bytes[usable..].to_vec();

        let mut iq: Vec<Complex32> = match cfg.format {
            IqFormat::Cu8 => bytes[..usable]
                .chunks_exact(2)
                .map(|b| {
                    Complex32::new((b[0] as f32 - 127.5) / 127.5, (b[1] as f32 - 127.5) / 127.5)
                })
                .collect(),
            IqFormat::Cs8 => bytes[..usable]
                .chunks_exact(2)
                .map(|b| {
                    Complex32::new(f32::from(b[0] as i8) / 128.0, f32::from(b[1] as i8) / 128.0)
                })
                .collect(),
            IqFormat::Cs16 => bytes[..usable]
                .chunks_exact(4)
                .map(|b| {
                    Complex32::new(
                        i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                        i16::from_le_bytes([b[2], b[3]]) as f32 / 32768.0,
                    )
                })
                .collect(),
            IqFormat::F32 => bytes[..usable]
                .chunks_exact(8)
                .map(|b| {
                    Complex32::new(
                        f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                        f32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                    )
                })
                .collect(),
        };
        if iq.is_empty() {
            continue;
        }

        // IQ recording (waterfall sessions): raw source bytes to disk
        if matches!(cfg.demod, Demod::Raw) {
            let want = knobs.record.lock().ok().and_then(|g| g.clone());
            match (&want, iq_rec.is_some()) {
                (Some(base), false) => {
                    let ext = match cfg.format {
                        IqFormat::Cu8 => "cu8",
                        IqFormat::Cs8 => "cs8",
                        IqFormat::Cs16 => "cs16",
                        IqFormat::F32 => "cf32",
                    };
                    let path = base.with_extension(ext);
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    match std::fs::File::create(&path) {
                        Ok(f) => {
                            let _ = std::fs::write(
                                base.with_extension("txt"),
                                format!(
                                    "rate={}
format={ext}
",
                                    cfg.rate
                                ),
                            );
                            let _ = tx.send(AppEvent::Rec {
                                run,
                                path: Some(path.to_string_lossy().into_owned()),
                            });
                            iq_rec = Some(f);
                        }
                        Err(_) => {
                            if let Ok(mut g) = knobs.record.lock() {
                                *g = None;
                            }
                        }
                    }
                }
                (None, true) => {
                    iq_rec = None;
                    let _ = tx.send(AppEvent::Rec { run, path: None });
                }
                _ => {}
            }
            if let Some(f) = &mut iq_rec {
                if f.write_all(&bytes[..usable]).is_err() {
                    iq_rec = None;
                    if let Ok(mut g) = knobs.record.lock() {
                        *g = None;
                    }
                    let _ = tx.send(AppEvent::Rec { run, path: None });
                }
            }
        }

        // full-band scope (pre-tune) ~12 fps
        if last_iq.elapsed() > Duration::from_millis(80) && iq.len() >= 1024 {
            last_iq = Instant::now();
            let spec = iq_fft.iq_db(&iq[..1024]);
            if tx.send(AppEvent::Iq { run, spec }).is_err() {
                break;
            }
        }

        let want_shift = knobs.if_shift.load(Ordering::Relaxed);
        if want_shift != cur_if_shift {
            cur_if_shift = want_shift;
            if let Detector::Ssb(ssb) = &mut det {
                let usb = matches!(cfg.demod, Demod::Usb);
                *ssb = SsbDemod::with_shift(48_000.0, usb, want_shift);
            }
        }

        nb.factor = bits_f32(knobs.nb.load(Ordering::Relaxed));
        nb.process(&mut iq);

        let offset = knobs.offset_hz.load(Ordering::Relaxed);
        if offset != cur_offset {
            cur_offset = offset;
            nco.set_freq(-(offset as f64), cfg.rate as f64);
        }
        nco.mix(&mut iq);

        let mut cur = iq;
        for d in &mut decims {
            let mut out = Vec::with_capacity(cur.len() / d.factor + 1);
            d.process(&cur, &mut out);
            cur = out;
        }
        if !chan_i.is_empty() {
            for x in &mut cur {
                let mut i = x.re;
                let mut q = x.im;
                for k in 0..chan_i.len() {
                    i = chan_i[k].tick(i);
                    q = chan_q[k].tick(q);
                }
                *x = Complex32::new(i, q);
            }
        }

        // demodulate → 48 kHz audio
        let mut audio: Vec<f32> = Vec::with_capacity(cur.len());
        match &mut det {
            Detector::Fm(fm) => fm.process(&cur, &mut audio),
            Detector::Wfm { fm, deemph, decim } => {
                let mut wide = Vec::with_capacity(cur.len());
                fm.process(&cur, &mut wide);
                deemph.process(&mut wide);
                decim.process(&wide, &mut audio);
            }
            Detector::Am { dc, agc, sam } => {
                if knobs.sync_det.load(Ordering::Relaxed) {
                    sam.process(&cur, &mut audio);
                } else {
                    audio.extend(cur.iter().map(|c| c.norm()));
                    dc.process(&mut audio);
                    agc.process(&mut audio);
                }
            }
            Detector::Ssb(ssb) => ssb.process(&cur, &mut audio),
            Detector::Raw => continue,
        }
        if audio.is_empty() {
            continue;
        }

        // decoder feed: raw demod audio, resampled to what the decoder wants.
        // Apply headroom so strong signals don't clip the s16 stream — clipped
        // discriminator audio wrecks digital-voice frames (CRC errors, no
        // sync). dsd-neo has its own AGC, so a lower level is safe.
        if let Some(dec) = &decoder {
            const DEC_HEADROOM: f32 = 0.5;
            let feed: Vec<f32> = audio.iter().map(|s| s * DEC_HEADROOM).collect();
            if let Some(rs) = &mut resampler {
                let mut out = Vec::with_capacity(feed.len() / 2 + 16);
                rs.process(&feed, &mut out);
                dec.write(&dsp::f32_to_s16(&out));
            } else {
                dec.write(&dsp::f32_to_s16(&feed));
            }
        }

        // CTCSS rides below 300 Hz on the raw discriminator output
        if matches!(cfg.demod, Demod::Nfm) {
            ctcss.feed(&audio);
        }

        // monitor path
        let rms_now = dsp::rms(&audio);
        let (hp, lp) = (
            knobs.hp_hz.load(Ordering::Relaxed),
            knobs.lp_hz.load(Ordering::Relaxed),
        );
        if hp != filters.hp_hz || lp != filters.lp_hz {
            filters.set(hp, lp);
        }
        filters.process(&mut audio);
        notch.enabled = knobs.notch.load(Ordering::Relaxed);
        notch.process(&mut audio);
        nr.strength = bits_f32(knobs.nr.load(Ordering::Relaxed));
        let mut mon = if nr.strength > 0.0 {
            nr.process(&audio)
        } else {
            std::mem::take(&mut audio)
        };

        // squelch with hysteresis (monitor only; decoders hear everything)
        let sq = bits_f32(knobs.squelch.load(Ordering::Relaxed));
        if sq > 0.0 {
            if gate_open {
                if rms_now < sq * 0.7 {
                    gate_open = false;
                }
            } else if rms_now > sq {
                gate_open = true;
            }
        } else {
            gate_open = true;
        }
        // CTCSS tone squelch: gate additionally requires the selected tone
        let want_chz = knobs.tone_chz.load(Ordering::Relaxed);
        let tone_ok = want_chz == 0
            || ctcss
                .detected
                .map(|d| (d - want_chz as f32 / 100.0).abs() < 0.5)
                .unwrap_or(false);
        let open = gate_open && tone_ok;
        if !open {
            mon.iter_mut().for_each(|x| *x = 0.0);
        }

        // Only play the discriminator monitor when this pump owns the audio.
        // For Voice decoders (cfg.record == false) pump_decoded owns the sink,
        // so the IQ pump must stay off it or the two mix.
        if cfg.record {
            if let Some(snk) = &sink {
                if !knobs.mute.load(Ordering::Relaxed) {
                    snk.write(&dsp::f32_to_s16(&mon));
                }
            }
        }

        // recording (squelch-aware: closed-gate static isn't written).
        // Skipped when a decoder owns the audio — pump_decoded records the
        // DECODED voice instead of this discriminator audio.
        let want_rec = if cfg.record {
            knobs.record.lock().ok().and_then(|g| g.clone())
        } else {
            None
        };
        match (&want_rec, recorder.is_some()) {
            (Some(path), false) => match crate::rec::WavWriter::create(path, 48_000) {
                Ok(w) => {
                    let _ = tx.send(AppEvent::Rec {
                        run,
                        path: Some(w.path.to_string_lossy().into_owned()),
                    });
                    recorder = Some(w);
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Line {
                        run,
                        src: crate::pipeline::LineSrc::Stderr,
                        text: format!("deck: recording failed: {e}"),
                    });
                    if let Ok(mut g) = knobs.record.lock() {
                        *g = None;
                    }
                    let _ = tx.send(AppEvent::Rec { run, path: None });
                }
            },
            (None, true) => {
                if let Some(w) = recorder.take() {
                    let meta = knobs.rec_meta.lock().ok().map(|m| m.clone()).unwrap_or_default();
                    let _ = w.finalize(Some(&meta));
                }
                let _ = tx.send(AppEvent::Rec { run, path: None });
            }
            _ => {}
        }
        if let Some(w) = &mut recorder {
            if open && w.write_s16(&dsp::f32_to_s16(&mon)).is_err() {
                recorder = None;
                if let Ok(mut g) = knobs.record.lock() {
                    *g = None;
                }
                let _ = tx.send(AppEvent::Rec { run, path: None });
            }
        }

        // audio spectrum frames (~15 fps) from the monitor path
        audio_acc.extend_from_slice(&mon);
        if audio_acc.len() >= 512 && last_audio.elapsed() > Duration::from_millis(66) {
            let start = audio_acc.len() - 512;
            let spec = audio_fft.real_db(&audio_acc[start..]);
            audio_acc.clear();
            last_audio = Instant::now();
            if tx
                .send(AppEvent::Audio {
                    run,
                    rms: rms_now,
                    spec,
                    tone: ctcss.detected,
                })
                .is_err()
            {
                break;
            }
        } else if audio_acc.len() > 8192 {
            audio_acc.drain(..audio_acc.len() - 512);
        }
    }
    if let Some(w) = recorder.take() {
        let meta = knobs.rec_meta.lock().ok().map(|m| m.clone()).unwrap_or_default();
        let _ = w.finalize(Some(&meta));
        let _ = tx.send(AppEvent::Rec { run, path: None });
    }
}
