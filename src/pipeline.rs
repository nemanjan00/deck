//! Pipeline resolution and process supervision.
//!
//! Architecture: deck owns the SDR through exactly ONE source process per
//! session (`rtl_sdr` / `airspyhf_rx` / the simulator) emitting raw IQ on
//! stdout. deck tunes, decimates and demodulates internally, then feeds
//! demodulated audio to decoders' stdin (multimon-ng, dsd-neo, minimodem)
//! and/or the audio sink. Only ADS-B runs as a fully external pipeline
//! (dump1090 owns the device there).

use crate::config::Config;
use crate::device::{SdrDevice, SdrKind};
use crate::modes::{mode_def, Demod, ModeId, PipeKind};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IqFormat {
    Cu8,
    Cs16,
}

/// Per-device IQ front-end: (source template, format, sample rate).
pub fn iq_source(dev: SdrKind) -> (&'static str, IqFormat, u32) {
    match dev {
        SdrKind::RtlSdr => (
            "rtl_sdr -d {device} -f {center_hz} -g {gain} -p {ppm} -s 2400000 -",
            IqFormat::Cu8,
            2_400_000,
        ),
        SdrKind::AirspyHf => (
            "airspyhf_rx -f {center_mhz} -a 768000 -r /dev/stdout",
            IqFormat::Cs16,
            768_000,
        ),
        SdrKind::Sim => (
            "'{deck}' simgen --mode iq-band --profile {profile} --center {center_hz} \
             --channel {freq_hz} --rate 2400000 --format cu8",
            IqFormat::Cu8,
            2_400_000,
        ),
    }
}

/// External (non-IQ) pipelines: ADS-B, and the sim's decoded-line feeds.
fn extern_template(dev: SdrKind, mode: ModeId) -> Option<String> {
    match (dev, mode) {
        (SdrKind::RtlSdr, ModeId::Adsb) => Some(
            "dump1090 --device-index {device} --gain {gain} --quiet --net \
             --net-sbs-port {sbs_port}"
                .into(),
        ),
        (SdrKind::AirspyHf, ModeId::Adsb) => None, // 1090 MHz out of range
        (SdrKind::RtlSdr, ModeId::Ais) => Some("rtl_ais -d {device} -n -g {gain} -p {ppm}".into()),
        (SdrKind::AirspyHf, ModeId::Ais) => None, // rtl_ais is RTL-only
        (SdrKind::Sim, m) => Some(format!(
            "'{{deck}}' simgen --mode {} --lines",
            mode_def(m).key
        )),
        _ => None,
    }
}

/// Tune offset: the device centers rate/4 away from the wanted signal so the
/// DC spike never sits on it; the NCO shifts it back to baseband.
pub fn tune_offset(rate: u32) -> i64 {
    (rate / 4) as i64
}

/// Pick a device center for `freq`, staying inside the device's range.
pub fn center_for(dev: &SdrDevice, freq: u64, rate: u32) -> u64 {
    let off = tune_offset(rate);
    let up = freq.saturating_add(off as u64);
    if dev.freq_ok(up) {
        up
    } else {
        freq.saturating_sub(off as u64)
    }
}

/// Can `freq` be reached from `center` without retuning the device?
pub fn in_band(center: u64, rate: u32, freq: u64) -> bool {
    let guard = (rate / 16) as i64;
    (freq as i64 - center as i64).abs() < (rate as i64) / 2 - guard
}

pub struct TemplateVars {
    pub device: u32,
    pub freq_hz: u64,
    pub center_hz: u64,
    pub gain: f32,
    pub ppm: i32,
    pub rate: u32,
    pub sbs_port: u16,
    pub profile: String,
}

pub fn render_template(t: &str, v: &TemplateVars) -> String {
    let deck = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "deck".into());
    t.replace("{device}", &v.device.to_string())
        .replace("{freq_hz}", &v.freq_hz.to_string())
        .replace("{freq_mhz}", &crate::freq::fmt_mhz(v.freq_hz))
        .replace("{freq_khz}", &format!("{}", v.freq_hz as f64 / 1000.0))
        .replace("{center_hz}", &v.center_hz.to_string())
        .replace("{center_mhz}", &crate::freq::fmt_mhz(v.center_hz))
        .replace("{gain_int}", &format!("{}", v.gain.round() as i64))
        .replace("{gain}", &format!("{:.1}", v.gain))
        .replace("{ppm}", &v.ppm.to_string())
        .replace("{rate}", &v.rate.to_string())
        .replace("{sbs_port}", &v.sbs_port.to_string())
        .replace("{profile}", &v.profile)
        .replace("{deck}", &deck)
}

/// Binaries a template needs: the head of every pipe segment.
pub fn requires_of(template: &str) -> Vec<String> {
    template
        .split('|')
        .filter_map(|seg| {
            let head = seg.split_whitespace().next()?.trim_matches('\'');
            if head.contains("{deck}") || head.is_empty() {
                None
            } else {
                Some(head.to_string())
            }
        })
        .collect()
}

pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(bin);
        if let Ok(md) = std::fs::metadata(&p) {
            use std::os::unix::fs::PermissionsExt;
            if md.is_file() && md.permissions().mode() & 0o111 != 0 {
                return Some(p);
            }
        }
    }
    None
}

#[derive(Clone, Default)]
pub struct ToolReport {
    pub found: HashMap<String, Option<PathBuf>>,
}

pub const KNOWN_TOOLS: &[&str] = &[
    "rtl_sdr",
    "rtl_test",
    "airspyhf_rx",
    "dsd-neo",
    "multimon-ng",
    "dump1090",
    "readsb",
    "rtl_ais",
    "AIS-catcher",
    "sox",
    "minimodem",
    "paplay",
    "pw-cat",
    "aplay",
    "wpctl",
    "pactl",
    "amixer",
];

impl ToolReport {
    pub fn scan() -> Self {
        let mut found = HashMap::new();
        for t in KNOWN_TOOLS {
            found.insert((*t).to_string(), which(t));
        }
        Self { found }
    }

    pub fn has(&self, bin: &str) -> bool {
        match self.found.get(bin) {
            Some(v) => v.is_some(),
            None => which(bin).is_some(),
        }
    }

    pub fn missing_for(&self, template: &str) -> Vec<String> {
        requires_of(template)
            .into_iter()
            .filter(|b| !self.has(b))
            .collect()
    }
}

/// A fully resolved, ready-to-run plan.
pub enum Plan {
    Extern {
        cmdline: String,
        /// read stdout as raw chunks (char feeds) instead of lines
        char_mode: bool,
        /// also connect the SBS TCP client
        sbs: bool,
    },
    Iq {
        source_cmdline: String,
        format: IqFormat,
        rate: u32,
        center_hz: u64,
        demod: Demod,
        decoder_cmd: Option<String>,
        decoder_rate: u32,
        decoder_char_mode: bool,
        audio_out: bool,
    },
}

pub struct Resolved {
    pub plan: Plan,
    pub missing: Vec<String>,
    pub note: Option<String>,
}

pub fn resolve(
    mode: ModeId,
    dev: &SdrDevice,
    cfg: &Config,
    tools: &ToolReport,
    freq_hz: u64,
    gain: f32,
) -> Option<Resolved> {
    let m = mode_def(mode);
    let dev_over = cfg.pipelines.get(dev.kind.key());
    let mut note = None;

    // Full external override for this mode? Run it as Extern.
    let extern_override = dev_over.and_then(|o| o.get(m.key)).cloned();

    let make_vars = |center: u64, rate: u32| TemplateVars {
        device: dev.index,
        freq_hz,
        center_hz: center,
        gain,
        ppm: cfg.sdr.ppm,
        rate,
        sbs_port: cfg.adsb.sbs_port,
        profile: m.key.to_string(),
    };

    let extern_plan = |template: String, note: Option<String>, tools: &ToolReport| {
        let missing = tools.missing_for(&template);
        Resolved {
            plan: Plan::Extern {
                cmdline: render_template(&template, &make_vars(freq_hz, 0)),
                char_mode: mode == ModeId::Rtty,
                sbs: mode == ModeId::Adsb && dev.kind != SdrKind::Sim,
            },
            missing,
            note,
        }
    };

    if let Some(t) = extern_override {
        return Some(extern_plan(
            t,
            Some("external pipeline from config".into()),
            tools,
        ));
    }

    // The simulator can't synthesize real C4FM/4FSK — digital voice on the
    // sim device always runs the decoded-line feed.
    if dev.kind == SdrKind::Sim && matches!(mode_def(mode).view, crate::modes::ViewKind::Voice) {
        let t = extern_template(dev.kind, mode)?;
        return Some(extern_plan(
            t,
            Some("voice sim runs as a decoder feed".into()),
            tools,
        ));
    }

    match m.pipe {
        PipeKind::Extern => {
            let t = extern_template(dev.kind, mode)?;
            Some(extern_plan(t, note, tools))
        }
        PipeKind::Iq(demod) => {
            let (src_t, format, rate) = iq_source(dev.kind);
            let src_t = dev_over
                .and_then(|o| o.get("iq_source"))
                .cloned()
                .unwrap_or_else(|| src_t.to_string());
            let decoder_t = cfg
                .decoders
                .get(m.key)
                .cloned()
                .or_else(|| m.decoder.map(String::from));

            let mut missing = tools.missing_for(&src_t);
            if let Some(d) = &decoder_t {
                missing.extend(tools.missing_for(d));
            }

            // Sim device degrades gracefully: decoder missing → line feed sim.
            if dev.kind == SdrKind::Sim && !missing.is_empty() {
                let t = format!("'{{deck}}' simgen --mode {} --lines", m.key);
                return Some(extern_plan(
                    t,
                    Some(format!(
                        "simulating decoder output ({} not installed)",
                        missing.join(", ")
                    )),
                    tools,
                ));
            }

            let center = center_for(dev, freq_hz, rate);
            if !in_band(center, rate, freq_hz) {
                note = Some("tuned frequency outside device band".into());
            }
            Some(Resolved {
                plan: Plan::Iq {
                    source_cmdline: render_template(&src_t, &make_vars(center, rate)),
                    format,
                    rate,
                    center_hz: center,
                    demod,
                    decoder_cmd: decoder_t,
                    decoder_rate: m.decoder_rate,
                    decoder_char_mode: mode == ModeId::Rtty,
                    audio_out: m.audio_out,
                },
                missing,
                note,
            })
        }
    }
}

// ------------------------------------------------------------- supervision

pub struct Spawned {
    pub child: Child,
    pub pgid: i32,
}

/// Spawn `sh -c cmdline` in its own process group with PDEATHSIG so the
/// whole pipeline can be torn down as a unit (and dies with us).
pub fn spawn_shell(cmdline: &str, want_stdout: bool, want_stdin: bool) -> std::io::Result<Spawned> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(cmdline)
        .stdin(if want_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if want_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        .process_group(0);
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    let pgid = child.id() as i32;
    Ok(Spawned { child, pgid })
}

/// SIGTERM the group now; SIGKILL shortly after (fire-and-forget).
pub fn kill_group(pgid: i32) {
    unsafe {
        libc::killpg(pgid, libc::SIGTERM);
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    });
}

/// Kill and *wait* for the child to be reaped (bounded). Used before
/// re-opening the same SDR so the USB interface is actually released.
pub fn kill_group_wait(mut sp: Spawned) {
    unsafe {
        libc::killpg(sp.pgid, libc::SIGTERM);
    }
    for _ in 0..30 {
        match sp.child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return,
        }
    }
    unsafe {
        libc::killpg(sp.pgid, libc::SIGKILL);
    }
    let _ = sp.child.wait();
}

/// Does this stderr line look like "another process owns the SDR"?
pub fn looks_like_device_busy(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("usb_claim_interface")
        || l.contains("resource busy")
        || l.contains("device or resource busy")
        || l.contains("failed to open")
        || l.contains("no supported devices")
        || l.contains("device not found")
        || l.contains("acquire") && l.contains("fail")
}

// ------------------------------------------------------------------ events

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineSrc {
    Stdout,
    Stderr,
    Sbs,
}

/// Everything background threads report to the UI loop.
pub enum AppEvent {
    Line {
        run: u64,
        src: LineSrc,
        text: String,
    },
    Audio {
        run: u64,
        rms: f32,
        spec: Vec<f32>,
        /// CTCSS tone detected on the demod audio (NFM), if any
        tone: Option<f32>,
    },
    Iq {
        run: u64,
        spec: Vec<f32>,
    },
    SbsStatus {
        run: u64,
        note: String,
    },
    /// recording started (Some(path)) or stopped/failed (None)
    Rec {
        run: u64,
        path: Option<String>,
    },
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' && (c == '\t' || !c.is_control()) {
            out.push(c);
        }
    }
    out
}

/// Attach line readers to a spawned pipeline's stdout/stderr.
pub fn attach_line_readers(sp: &mut Spawned, run: u64, tx: &Sender<AppEvent>, char_mode: bool) {
    if let Some(stdout) = sp.child.stdout.take() {
        let tx = tx.clone();
        std::thread::spawn(move || read_stream(stdout, run, LineSrc::Stdout, tx, char_mode));
    }
    if let Some(stderr) = sp.child.stderr.take() {
        let tx = tx.clone();
        std::thread::spawn(move || read_stream(stderr, run, LineSrc::Stderr, tx, false));
    }
}

pub fn read_stream(
    stream: impl Read,
    run: u64,
    src: LineSrc,
    tx: Sender<AppEvent>,
    char_mode: bool,
) {
    if char_mode {
        let mut rd = stream;
        let mut buf = [0u8; 256];
        while let Ok(n) = rd.read(&mut buf) {
            if n == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
            if tx.send(AppEvent::Line { run, src, text }).is_err() {
                break;
            }
        }
    } else {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if tx
                .send(AppEvent::Line {
                    run,
                    src,
                    text: strip_ansi(&line),
                })
                .is_err()
            {
                break;
            }
        }
    }
}

/// Background SBS (BaseStation, port 30003) TCP client with retry.
pub fn spawn_sbs_client(
    host: String,
    port: u16,
    run: u64,
    tx: Sender<AppEvent>,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        use std::net::TcpStream;
        let addr = format!("{host}:{port}");
        let mut attempts = 0u32;
        let stream = loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match TcpStream::connect(&addr) {
                Ok(s) => break s,
                Err(_) if attempts < 20 => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::SbsStatus {
                        run,
                        note: format!("SBS connect failed ({addr}): {e}"),
                    });
                    return;
                }
            }
        };
        let _ = tx.send(AppEvent::SbsStatus {
            run,
            note: format!("SBS connected ({addr})"),
        });
        let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
        let mut rd = BufReader::new(stream);
        let mut line = String::new();
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            line.clear();
            match rd.read_line(&mut line) {
                Ok(0) => {
                    let _ = tx.send(AppEvent::SbsStatus {
                        run,
                        note: "SBS stream closed".into(),
                    });
                    return;
                }
                Ok(_) => {
                    let _ = tx.send(AppEvent::Line {
                        run,
                        src: LineSrc::Sbs,
                        text: line.trim_end().to_string(),
                    });
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(_) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::SdrDevice;

    fn rtl() -> SdrDevice {
        SdrDevice {
            kind: SdrKind::RtlSdr,
            index: 0,
            product: "test".into(),
            serial: None,
            usb_path: String::new(),
        }
    }

    #[test]
    fn template_render() {
        let v = TemplateVars {
            device: 0,
            freq_hz: 145_800_000,
            center_hz: 146_400_000,
            gain: 32.8,
            ppm: 1,
            rate: 2_400_000,
            sbs_port: 30003,
            profile: "nfm".into(),
        };
        let s = render_template(
            "rtl_sdr -d {device} -f {center_hz} -g {gain} ({freq_mhz})",
            &v,
        );
        assert_eq!(s, "rtl_sdr -d 0 -f 146400000 -g 32.8 (145.800000)");
    }

    #[test]
    fn center_and_in_band() {
        let d = rtl();
        let c = center_for(&d, 145_500_000, 2_400_000);
        assert_eq!(c, 146_100_000);
        assert!(in_band(c, 2_400_000, 145_500_000));
        assert!(in_band(c, 2_400_000, 146_800_000));
        assert!(!in_band(c, 2_400_000, 148_000_000));
        // near top of range the center flips below
        let c2 = center_for(&d, 1_766_000_000, 2_400_000);
        assert_eq!(c2, 1_765_400_000);
    }

    #[test]
    fn resolve_iq_and_extern() {
        let cfg = Config::default();
        let tools = ToolReport::default(); // nothing found
        let d = rtl();
        let r = resolve(ModeId::Nfm, &d, &cfg, &tools, 145_500_000, 30.0).unwrap();
        match r.plan {
            Plan::Iq {
                rate,
                demod,
                audio_out,
                decoder_cmd,
                ..
            } => {
                assert_eq!(rate, 2_400_000);
                assert_eq!(demod, Demod::Nfm);
                assert!(audio_out);
                assert!(decoder_cmd.is_none());
            }
            _ => panic!("expected iq plan"),
        }
        assert!(r.missing.contains(&"rtl_sdr".to_string()));

        let r = resolve(ModeId::Adsb, &d, &cfg, &tools, 1_090_000_000, 40.0).unwrap();
        match r.plan {
            Plan::Extern { cmdline, sbs, .. } => {
                assert!(cmdline.contains("dump1090"));
                assert!(cmdline.contains("--net-sbs-port 30003"));
                assert!(sbs);
            }
            _ => panic!("expected extern plan"),
        }
    }

    #[test]
    fn sim_falls_back_to_lines_without_decoder() {
        let cfg = Config::default();
        let tools = ToolReport::default();
        let d = SdrDevice::sim();
        let r = resolve(ModeId::Pocsag, &d, &cfg, &tools, 169_650_000, 0.0).unwrap();
        match r.plan {
            Plan::Extern { cmdline, .. } => {
                assert!(
                    cmdline.contains("simgen --mode pocsag --lines"),
                    "{cmdline}"
                );
            }
            _ => panic!("expected fallback extern plan"),
        }
        assert!(r.missing.is_empty());
        assert!(r.note.unwrap().contains("multimon-ng"));
    }

    #[test]
    fn decoder_override_wins() {
        let mut cfg = Config::default();
        cfg.decoders
            .insert("pocsag".into(), "my-decoder --flag -".into());
        let mut tools = ToolReport::default();
        tools.found.insert("rtl_sdr".into(), Some("/bin/sh".into()));
        tools
            .found
            .insert("my-decoder".into(), Some("/bin/sh".into()));
        let r = resolve(ModeId::Pocsag, &rtl(), &cfg, &tools, 169_650_000, 30.0).unwrap();
        match r.plan {
            Plan::Iq { decoder_cmd, .. } => {
                assert_eq!(decoder_cmd.unwrap(), "my-decoder --flag -");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn requires_and_busy() {
        let r = requires_of("rtl_sdr -f 1 - | multimon-ng -a AFSK1200 -");
        assert_eq!(r, vec!["rtl_sdr", "multimon-ng"]);
        assert!(looks_like_device_busy("usb_claim_interface error -6"));
        assert!(looks_like_device_busy("rtlsdr: Device or resource busy"));
        assert!(!looks_like_device_busy("Tuned to 146400000 Hz."));
    }

    #[test]
    fn ansi_strip() {
        assert_eq!(strip_ansi("\u{1b}[1;32mok\u{1b}[0m\r"), "ok");
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
