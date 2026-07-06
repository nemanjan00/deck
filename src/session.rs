//! UI-agnostic application state: session lifecycle (start/stop/retune),
//! event application (decoder lines → typed stores), the scanner brain,
//! and persistence. The GUI is a thin skin over this.

use crate::audio::{bits_f32, f32_bits, Knobs, RxEngine};
use crate::config::{Chan, Config, ModePersist, PersistState};
use crate::device::{SdrDevice, SdrKind};
use crate::modes::{mode_def, ModeId, ViewKind};
use crate::parse::dsd::CallFields;
use crate::parse::multimon::{AprsMsg, AprsParser, PagerMsg};
use crate::parse::sbs::AircraftStore;
use crate::pipeline::{
    self, in_band, resolve, AppEvent, LineSrc, Plan, Spawned, ToolReport,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Timed<T> {
    pub at: chrono::DateTime<chrono::Local>,
    pub msg: T,
}

impl<T> Timed<T> {
    fn now(msg: T) -> Self {
        Self {
            at: chrono::Local::now(),
            msg,
        }
    }
}

pub struct WaterfallBuf {
    pub rows: VecDeque<Vec<u8>>,
    pub width: usize,
    pub cap: usize,
    /// bumped on every push — cheap change detection for texture uploads
    pub rev: u64,
}

impl WaterfallBuf {
    pub fn new(cap: usize) -> Self {
        Self {
            rows: VecDeque::new(),
            width: 0,
            cap,
            rev: 0,
        }
    }

    pub fn push(&mut self, spec: &[f32], min_db: f32, max_db: f32) {
        self.width = spec.len();
        self.rev += 1;
        let row: Vec<u8> = spec
            .iter()
            .map(|v| {
                let t = ((v - min_db) / (max_db - min_db)).clamp(0.0, 1.0);
                (t * 255.0) as u8
            })
            .collect();
        self.rows.push_front(row);
        while self.rows.len() > self.cap {
            self.rows.pop_back();
        }
    }
}

pub struct LiveCall {
    pub fields: CallFields,
    pub started: Instant,
    pub last: Instant,
}

pub struct CallRecord {
    pub at: chrono::DateTime<chrono::Local>,
    pub fields: CallFields,
    pub dur_s: f32,
}

/// Everything decoded/measured during the current session.
pub struct Stores {
    pub raw: VecDeque<(LineSrc, String)>,
    pub pagers: VecDeque<Timed<PagerMsg>>,
    pub aprs: VecDeque<Timed<AprsMsg>>,
    aprs_parser: AprsParser,
    pub textfeed: String,
    pub aircraft: AircraftStore,
    pub call: Option<LiveCall>,
    pub call_history: VecDeque<CallRecord>,
    pub audio_rms: f32,
    pub audio_spec: Vec<f32>,
    pub audio_peak: Vec<f32>,
    pub band_spec: Vec<f32>,
    pub wf_audio: WaterfallBuf,
    pub wf_band: WaterfallBuf,
    pub decoded: u64,
    pub lines: u64,
    pub device_busy: bool,
    pub rec_path: Option<String>,
    pub rec_since: Option<Instant>,
    pub sbs_note: Option<String>,
    pub last_rms_at: Instant,
}

impl Stores {
    fn new() -> Self {
        Self {
            raw: VecDeque::new(),
            pagers: VecDeque::new(),
            aprs: VecDeque::new(),
            aprs_parser: AprsParser::new(),
            textfeed: String::new(),
            aircraft: AircraftStore::new(),
            call: None,
            call_history: VecDeque::new(),
            audio_rms: 0.0,
            audio_spec: Vec::new(),
            audio_peak: Vec::new(),
            band_spec: Vec::new(),
            wf_audio: WaterfallBuf::new(512),
            wf_band: WaterfallBuf::new(512),
            decoded: 0,
            lines: 0,
            device_busy: false,
            rec_path: None,
            rec_since: None,
            sbs_note: None,
            last_rms_at: Instant::now(),
        }
    }

    fn reset_for_run(&mut self) {
        let history_keep = std::mem::take(&mut self.call_history);
        *self = Self::new();
        self.call_history = history_keep;
    }

    fn push_raw(&mut self, src: LineSrc, text: String) {
        self.lines += 1;
        self.raw.push_front((src, text));
        while self.raw.len() > 1500 {
            self.raw.pop_back();
        }
    }
}

pub enum Backend {
    Iq(RxEngine),
    Extern {
        child: Spawned,
        sbs_stop: Option<Arc<AtomicBool>>,
    },
}

pub struct Running {
    pub run: u64,
    pub mode: ModeId,
    pub backend: Backend,
    pub knobs: Arc<Knobs>,
    pub started: Instant,
    pub center_hz: u64,
    pub rate: u32,
    pub freq_hz: u64,
    pub note: Option<String>,
    pub audio_capable: bool,
    pub monitorable: bool,
}

#[derive(Clone)]
pub struct ScanChan {
    pub label: String,
    pub hz: u64,
    pub locked: bool,
    pub hits: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Paused,
    Sampling,
    Hold,
}

pub struct ScanState {
    pub channels: Vec<ScanChan>,
    pub cur: usize,
    pub phase: ScanPhase,
    pub phase_since: Instant,
    pub last_signal: Instant,
    pub hits: VecDeque<Timed<String>>,
}

impl ScanState {
    fn from_cfg(cfg: &Config, lockouts: &[u64]) -> Self {
        Self {
            channels: cfg
                .scanner
                .channels
                .iter()
                .map(|c| ScanChan {
                    label: c.label.clone(),
                    hz: c.hz,
                    locked: lockouts.contains(&c.hz),
                    hits: 0,
                })
                .collect(),
            cur: 0,
            phase: ScanPhase::Sampling,
            phase_since: Instant::now(),
            last_signal: Instant::now(),
            hits: VecDeque::new(),
        }
    }
}

pub struct Session {
    pub cfg: Config,
    pub cfg_error: Option<String>,
    pub persist: PersistState,
    pub tools: ToolReport,
    pub devices: Vec<SdrDevice>,
    pub active_dev: usize,
    pub running: Option<Running>,
    pub stores: Stores,
    pub scan: ScanState,
    pub status: Option<(String, Instant)>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    run_counter: u64,
}

impl Session {
    pub fn new(config_path: Option<&std::path::Path>) -> Self {
        let (cfg, cfg_error) = crate::config::load_config(config_path);
        let persist = crate::config::load_state();
        let devices = crate::device::detect();
        let tools = ToolReport::scan();
        let active_dev = persist
            .device
            .as_ref()
            .and_then(|id| devices.iter().position(|d| &d.stable_id() == id))
            .unwrap_or_else(|| {
                devices
                    .iter()
                    .position(|d| d.kind != SdrKind::Sim)
                    .unwrap_or(devices.len().saturating_sub(1))
            });
        let (tx, rx) = std::sync::mpsc::channel();
        let scan = ScanState::from_cfg(&cfg, &persist.lockouts);
        Self {
            cfg,
            cfg_error,
            persist,
            tools,
            devices,
            active_dev,
            running: None,
            stores: Stores::new(),
            scan,
            status: None,
            tx,
            rx,
            run_counter: 0,
        }
    }

    pub fn device(&self) -> &SdrDevice {
        &self.devices[self.active_dev.min(self.devices.len() - 1)]
    }

    pub fn rescan_devices(&mut self) {
        let cur_id = self.device().stable_id();
        self.devices = crate::device::detect();
        self.tools = ToolReport::scan();
        self.active_dev = self
            .devices
            .iter()
            .position(|d| d.stable_id() == cur_id)
            .unwrap_or(self.devices.len() - 1);
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    pub fn mode_persist(&self, mode: ModeId) -> ModePersist {
        let m = mode_def(mode);
        self.persist
            .modes
            .get(m.key)
            .cloned()
            .unwrap_or_else(|| ModePersist {
                freq: m.default_hz,
                gain: self.cfg.sdr.gain,
                squelch: if mode == ModeId::Scanner { 0.05 } else { 0.0 },
                monitor: m.audio_out,
                ..Default::default()
            })
    }

    pub fn save_mode_persist(&mut self, mode: ModeId, mp: ModePersist) {
        self.persist
            .modes
            .insert(mode_def(mode).key.to_string(), mp);
    }

    /// All presets for a mode: built-ins + config extras.
    pub fn presets(&self, mode: ModeId) -> Vec<Chan> {
        let m = mode_def(mode);
        let mut v: Vec<Chan> = m
            .presets
            .iter()
            .map(|(label, hz)| Chan {
                label: (*label).to_string(),
                hz: *hz,
            })
            .collect();
        if let Some(extra) = self.cfg.presets.get(m.key) {
            v.extend(extra.iter().cloned());
        }
        v
    }

    // ------------------------------------------------------------ lifecycle

    pub fn start(&mut self, mode: ModeId, mp: &ModePersist) -> Result<(), String> {
        self.stop();
        let dev = self.device().clone();
        let freq = if mp.freq == 0 {
            mode_def(mode).default_hz
        } else {
            mp.freq
        };
        let Some(resolved) = resolve(mode, &dev, &self.cfg, &self.tools, freq, mp.gain) else {
            return Err(format!(
                "{} is not supported on {}",
                mode_def(mode).label,
                dev.kind.label()
            ));
        };
        if !resolved.missing.is_empty() {
            return Err(format!("missing tools: {}", resolved.missing.join(", ")));
        }
        if !dev.freq_ok(freq) && !matches!(resolved.plan, Plan::Extern { .. }) {
            return Err(format!(
                "{} is outside {}'s tuning range",
                crate::freq::fmt_short(freq),
                dev.kind.label()
            ));
        }

        self.run_counter += 1;
        let run = self.run_counter;
        self.stores.reset_for_run();
        if mode == ModeId::Scanner {
            self.scan.cur = self
                .scan
                .channels
                .iter()
                .position(|c| !c.locked)
                .unwrap_or(0);
            self.scan.phase = ScanPhase::Sampling;
            self.scan.phase_since = Instant::now();
        }

        match resolved.plan {
            Plan::Extern {
                cmdline,
                char_mode,
                sbs,
            } => {
                let mut sp = pipeline::spawn_shell(&cmdline, true, false)
                    .map_err(|e| format!("spawn failed: {e}"))?;
                pipeline::attach_line_readers(&mut sp, run, &self.tx, char_mode);
                let sbs_stop = sbs.then(|| {
                    let stop = Arc::new(AtomicBool::new(false));
                    pipeline::spawn_sbs_client(
                        self.cfg.adsb.sbs_host.clone(),
                        self.cfg.adsb.sbs_port,
                        run,
                        self.tx.clone(),
                        stop.clone(),
                    );
                    stop
                });
                self.running = Some(Running {
                    run,
                    mode,
                    backend: Backend::Extern {
                        child: sp,
                        sbs_stop,
                    },
                    knobs: Knobs::new(0),
                    started: Instant::now(),
                    center_hz: freq,
                    rate: 0,
                    freq_hz: freq,
                    note: resolved.note,
                    audio_capable: false,
                    monitorable: false,
                });
            }
            Plan::Iq {
                source_cmdline,
                format,
                rate,
                center_hz,
                demod,
                decoder_cmd,
                decoder_rate,
                decoder_char_mode,
                audio_out,
            } => {
                let knobs = Knobs::new(freq as i64 - center_hz as i64);
                knobs.nr.store(f32_bits(nr_level(mp.nr)), Ordering::Relaxed);
                knobs.nb.store(f32_bits(nb_level(mp.nb)), Ordering::Relaxed);
                knobs.notch.store(mp.notch, Ordering::Relaxed);
                knobs.hp_hz.store(mp.hp, Ordering::Relaxed);
                knobs.lp_hz.store(mp.lp, Ordering::Relaxed);
                knobs
                    .squelch
                    .store(f32_bits(mp.squelch), Ordering::Relaxed);
                knobs.sync_det.store(mp.det == 1, Ordering::Relaxed);
                let monitorable = audio_out || decoder_cmd.is_some();
                knobs
                    .mute
                    .store(!(audio_out || mp.monitor), Ordering::Relaxed);

                let wants_audio = demod != crate::modes::Demod::Raw
                    && (audio_out || decoder_cmd.is_some() || mode == ModeId::Scanner);
                let sink_cmd = if wants_audio {
                    crate::audio::resolve_sink(&self.cfg, &self.tools)
                } else {
                    None
                };
                let audio_capable = sink_cmd.is_some();
                let engine = RxEngine::start(
                    crate::audio::EngineParams {
                        run,
                        source_cmdline,
                        format,
                        rate,
                        demod,
                        decoder_cmd,
                        decoder_rate,
                        decoder_char_mode,
                        sink_cmd,
                    },
                    knobs.clone(),
                    self.tx.clone(),
                )
                .map_err(|e| format!("engine start failed: {e}"))?;
                self.running = Some(Running {
                    run,
                    mode,
                    backend: Backend::Iq(engine),
                    knobs,
                    started: Instant::now(),
                    center_hz,
                    rate,
                    freq_hz: freq,
                    note: resolved.note,
                    audio_capable,
                    monitorable,
                });
            }
        }
        if let Some(note) = self.running.as_ref().and_then(|r| r.note.clone()) {
            self.set_status(note);
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            match r.backend {
                Backend::Iq(engine) => engine.stop(),
                Backend::Extern { child, sbs_stop } => {
                    if let Some(s) = sbs_stop {
                        s.store(true, Ordering::Relaxed);
                    }
                    pipeline::kill_group_wait(child);
                }
            }
            self.stores.rec_path = None;
            self.stores.rec_since = None;
        }
    }

    /// Retune the running session. In-band = instant NCO swap; out-of-band =
    /// source restart on a new center. Extern backends don't retune.
    pub fn retune(&mut self, freq: u64) -> Result<(), String> {
        let dev = self.device().clone();
        let cfg_gain = self.cfg.sdr.gain;
        let Some(r) = self.running.as_mut() else {
            return Ok(());
        };
        match &mut r.backend {
            Backend::Extern { .. } => Err("stop and restart to change ADS-B tuning".into()),
            Backend::Iq(engine) => {
                if !dev.freq_ok(freq) {
                    return Err(format!(
                        "{} is outside {}'s range",
                        crate::freq::fmt_short(freq),
                        dev.kind.label()
                    ));
                }
                r.freq_hz = freq;
                if in_band(r.center_hz, r.rate, freq) {
                    r.knobs
                        .offset_hz
                        .store(freq as i64 - r.center_hz as i64, Ordering::Relaxed);
                    Ok(())
                } else {
                    let mp = ModePersist {
                        freq,
                        gain: cfg_gain,
                        ..Default::default()
                    };
                    let resolved = resolve(r.mode, &dev, &self.cfg, &self.tools, freq, mp.gain)
                        .ok_or("mode unsupported")?;
                    let Plan::Iq {
                        source_cmdline,
                        center_hz,
                        ..
                    } = resolved.plan
                    else {
                        return Err("plan changed shape".into());
                    };
                    r.center_hz = center_hz;
                    r.knobs
                        .offset_hz
                        .store(freq as i64 - center_hz as i64, Ordering::Relaxed);
                    engine
                        .retune_center(&source_cmdline)
                        .map_err(|e| format!("retune failed: {e}"))
                }
            }
        }
    }

    pub fn toggle_record(&mut self, mode: ModeId) {
        let Some(r) = &self.running else {
            return;
        };
        if !matches!(r.backend, Backend::Iq(_)) {
            self.set_status("recording works in IQ modes only");
            return;
        }
        let recording = self.stores.rec_path.is_some();
        if let Ok(mut g) = r.knobs.record.lock() {
            if recording {
                *g = None;
            } else {
                let dir = crate::rec::recordings_dir(&self.cfg.audio.record_dir);
                let name = crate::rec::recording_filename(mode_def(mode).key, r.freq_hz);
                *g = Some(dir.join(name));
            }
        }
    }

    // -------------------------------------------------------------- events

    pub fn drain_events(&mut self) {
        let now = Instant::now();
        while let Ok(ev) = self.rx.try_recv() {
            let Some(r) = &self.running else {
                continue;
            };
            let (run, mode) = (r.run, r.mode);
            match ev {
                AppEvent::Line { run: er, src, text } if er == run => {
                    self.apply_line(mode, src, text);
                }
                AppEvent::Audio { run: er, rms, spec } if er == run => {
                    self.stores.audio_rms = rms;
                    self.stores.last_rms_at = now;
                    if self.stores.audio_peak.len() != spec.len() {
                        self.stores.audio_peak = spec.clone();
                    } else {
                        for (p, v) in self.stores.audio_peak.iter_mut().zip(&spec) {
                            *p = (*p - 0.35).max(*v);
                        }
                    }
                    self.stores.wf_audio.push(&spec, -90.0, -10.0);
                    self.stores.audio_spec = spec;
                }
                AppEvent::Iq { run: er, spec } if er == run => {
                    self.stores.wf_band.push(&spec, -80.0, 0.0);
                    self.stores.band_spec = spec;
                }
                AppEvent::SbsStatus { run: er, note } if er == run => {
                    self.stores.sbs_note = Some(note);
                }
                AppEvent::Rec { run: er, path } if er == run => {
                    self.stores.rec_since = path.as_ref().map(|_| now);
                    self.stores.rec_path = path;
                }
                _ => {} // stale run
            }
        }
    }

    fn apply_line(&mut self, mode: ModeId, src: LineSrc, text: String) {
        if src == LineSrc::Stderr && pipeline::looks_like_device_busy(&text) {
            self.stores.device_busy = true;
        }
        let view = mode_def(mode).view;
        match view {
            ViewKind::Pager => {
                if src != LineSrc::Stderr {
                    if let Some(m) = crate::parse::multimon::parse_pocsag(&text) {
                        self.stores.decoded += 1;
                        self.stores.pagers.push_front(Timed::now(m));
                        while self.stores.pagers.len() > 400 {
                            self.stores.pagers.pop_back();
                        }
                    }
                }
            }
            ViewKind::Aprs => {
                if src != LineSrc::Stderr {
                    if let Some(m) = self.stores.aprs_parser.push(&text) {
                        self.stores.decoded += 1;
                        self.stores.aprs.push_front(Timed::now(m));
                        while self.stores.aprs.len() > 400 {
                            self.stores.aprs.pop_back();
                        }
                    }
                }
            }
            ViewKind::Adsb => {
                if src != LineSrc::Stderr
                    && self.stores.aircraft.push_line(&text, Instant::now())
                {
                    self.stores.decoded += 1;
                }
            }
            ViewKind::TextFeed => {
                if src == LineSrc::Stdout {
                    self.stores.decoded += 1;
                    self.stores.textfeed.push_str(&text);
                    if !text.ends_with('\n') && text.len() > 40 {
                        self.stores.textfeed.push('\n');
                    }
                    let overflow = self.stores.textfeed.len().saturating_sub(16_000);
                    if overflow > 0 {
                        let cut = self
                            .stores
                            .textfeed
                            .char_indices()
                            .nth(overflow)
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.stores.textfeed.drain(..cut);
                    }
                }
            }
            ViewKind::Voice => {
                if src != LineSrc::Stderr {
                    let fields = crate::parse::dsd::parse_line(&text);
                    if !fields.is_empty() {
                        self.stores.decoded += 1;
                        let now = Instant::now();
                        match &mut self.stores.call {
                            Some(c) => {
                                c.fields.merge(fields);
                                c.last = now;
                            }
                            None => {
                                self.stores.call = Some(LiveCall {
                                    fields,
                                    started: now,
                                    last: now,
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        self.stores.push_raw(src, text);
    }

    // ---------------------------------------------------------------- tick

    /// Periodic work; call from the UI at ~10 Hz or faster.
    pub fn tick(&mut self) {
        self.drain_events();
        let now = Instant::now();

        // voice call expiry → history
        if let Some(c) = &self.stores.call {
            if now.duration_since(c.last) > Duration::from_millis(2500) {
                let c = self.stores.call.take().unwrap();
                self.stores.call_history.push_front(CallRecord {
                    at: chrono::Local::now(),
                    fields: c.fields,
                    dur_s: c.last.duration_since(c.started).as_secs_f32().max(0.1),
                });
                while self.stores.call_history.len() > 200 {
                    self.stores.call_history.pop_back();
                }
            }
        }

        if self
            .running
            .as_ref()
            .map(|r| mode_def(r.mode).view == ViewKind::Adsb)
            .unwrap_or(false)
        {
            self.stores.aircraft.purge(now, 90);
        }

        if let Some((_, at)) = &self.status {
            if now.duration_since(*at) > Duration::from_secs(5) {
                self.status = None;
            }
        }

        self.tick_scanner(now);
    }

    fn tick_scanner(&mut self, now: Instant) {
        let running_scan = self
            .running
            .as_ref()
            .map(|r| r.mode == ModeId::Scanner)
            .unwrap_or(false);
        if !running_scan || self.scan.channels.is_empty() {
            return;
        }
        let sq = self
            .running
            .as_ref()
            .map(|r| bits_f32(r.knobs.squelch.load(Ordering::Relaxed)))
            .unwrap_or(0.05)
            .max(0.01);
        let rms_fresh = now.duration_since(self.stores.last_rms_at) < Duration::from_millis(400);
        let signal = rms_fresh && self.stores.audio_rms > sq;
        let dwell = Duration::from_millis(self.cfg.scanner.dwell_ms.max(150));
        let hold = Duration::from_millis(self.cfg.scanner.hold_ms.max(300));

        match self.scan.phase {
            ScanPhase::Paused => {}
            ScanPhase::Sampling => {
                if signal {
                    self.scan.phase = ScanPhase::Hold;
                    self.scan.phase_since = now;
                    self.scan.last_signal = now;
                    let ch = &mut self.scan.channels[self.scan.cur];
                    ch.hits += 1;
                    let label = format!(
                        "{} ({})",
                        ch.label.clone(),
                        crate::freq::fmt_short(ch.hz)
                    );
                    self.scan.hits.push_front(Timed::now(label));
                    while self.scan.hits.len() > 100 {
                        self.scan.hits.pop_back();
                    }
                } else if now.duration_since(self.scan.phase_since) > dwell {
                    self.scan_step(1);
                }
            }
            ScanPhase::Hold => {
                if signal {
                    self.scan.last_signal = now;
                } else if now.duration_since(self.scan.last_signal) > hold {
                    self.scan.phase = ScanPhase::Sampling;
                    self.scan_step(1);
                }
            }
        }
    }

    /// Advance the scanner by `dir` channels (skipping lockouts) and retune.
    pub fn scan_step(&mut self, dir: i32) {
        let n = self.scan.channels.len();
        if n == 0 {
            return;
        }
        let mut idx = self.scan.cur;
        for _ in 0..n {
            idx = (idx as i64 + dir as i64).rem_euclid(n as i64) as usize;
            if !self.scan.channels[idx].locked {
                break;
            }
        }
        self.scan.cur = idx;
        self.scan.phase_since = Instant::now();
        let hz = self.scan.channels[idx].hz;
        if let Err(e) = self.retune(hz) {
            self.set_status(e);
        }
    }

    pub fn toggle_lockout(&mut self, idx: usize) {
        if let Some(c) = self.scan.channels.get_mut(idx) {
            c.locked = !c.locked;
            let hz = c.hz;
            if c.locked {
                if !self.persist.lockouts.contains(&hz) {
                    self.persist.lockouts.push(hz);
                }
            } else {
                self.persist.lockouts.retain(|&l| l != hz);
            }
        }
    }

    // ---------------------------------------------------------- persistence

    /// Snapshot live knob values into a ModePersist (called before saving).
    pub fn knobs_snapshot(&self, base: &ModePersist) -> ModePersist {
        let mut mp = base.clone();
        if let Some(r) = &self.running {
            let k = &r.knobs;
            mp.freq = r.freq_hz;
            mp.nr = level_from_nr(bits_f32(k.nr.load(Ordering::Relaxed)));
            mp.nb = level_from_nb(bits_f32(k.nb.load(Ordering::Relaxed)));
            mp.notch = k.notch.load(Ordering::Relaxed);
            mp.hp = k.hp_hz.load(Ordering::Relaxed);
            mp.lp = k.lp_hz.load(Ordering::Relaxed);
            mp.squelch = bits_f32(k.squelch.load(Ordering::Relaxed));
            mp.det = u8::from(k.sync_det.load(Ordering::Relaxed));
            if r.monitorable {
                mp.monitor = !k.mute.load(Ordering::Relaxed);
            }
        }
        mp
    }

    pub fn save(&mut self) {
        self.persist.device = Some(self.device().stable_id());
        crate::config::save_state(&self.persist);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        self.save();
    }
}

// NR/NB ladders: level index ↔ DSP factor
pub const NR_LEVELS: &[f32] = &[0.0, 1.2, 2.0, 3.0];
pub const NB_LEVELS: &[f32] = &[0.0, 4.0, 2.5];

pub fn nr_level(idx: u8) -> f32 {
    NR_LEVELS[(idx as usize).min(NR_LEVELS.len() - 1)]
}

pub fn nb_level(idx: u8) -> f32 {
    NB_LEVELS[(idx as usize).min(NB_LEVELS.len() - 1)]
}

fn level_from_nr(v: f32) -> u8 {
    NR_LEVELS
        .iter()
        .position(|x| (x - v).abs() < 0.01)
        .unwrap_or(0) as u8
}

fn level_from_nb(v: f32) -> u8 {
    NB_LEVELS
        .iter()
        .position(|x| (x - v).abs() < 0.01)
        .unwrap_or(0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waterfall_quantizes_and_caps() {
        let mut wf = WaterfallBuf::new(3);
        for i in 0..5 {
            wf.push(&[-80.0 + i as f32 * 10.0, -10.0], -80.0, -10.0);
        }
        assert_eq!(wf.rows.len(), 3);
        assert_eq!(wf.rows[0][1], 255);
        assert!(wf.rows[2][0] < 128);
    }

    #[test]
    fn nr_ladder_roundtrip() {
        for i in 0..NR_LEVELS.len() as u8 {
            assert_eq!(level_from_nr(nr_level(i)), i);
        }
        for i in 0..NB_LEVELS.len() as u8 {
            assert_eq!(level_from_nb(nb_level(i)), i);
        }
    }
}
