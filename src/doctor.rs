//! `deck doctor` — environment report and end-to-end selftest.

use crate::config::Config;
use crate::device::detect;
use crate::modes::MODES;
use crate::pipeline::{resolve, Plan, ToolReport};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

pub fn report(cfg: &Config) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "deck v{} — environment report",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(s);

    let devices = detect();
    let _ = writeln!(s, "SDR devices");
    for d in &devices {
        let extra = match (&d.serial, d.usb_path.is_empty()) {
            (Some(sn), false) => format!("  serial={sn}  usb={}", d.usb_path),
            _ => String::new(),
        };
        let _ = writeln!(
            s,
            "  [{}] {} — {}{}",
            d.stable_id(),
            d.kind.label(),
            d.product,
            extra
        );
    }
    let _ = writeln!(s);

    let tools = ToolReport::scan();
    let _ = writeln!(s, "external tools");
    for t in crate::pipeline::KNOWN_TOOLS {
        match tools.found.get(*t).and_then(|p| p.as_ref()) {
            Some(p) => {
                let _ = writeln!(s, "  {t:<12} {}", p.display());
            }
            None => {
                let _ = writeln!(s, "  {t:<12} — not found");
            }
        }
    }
    let _ = writeln!(s);

    // USB permission hints (bundled AppImage still needs host udev setup)
    let have_rules = std::path::Path::new("/etc/udev/rules.d")
        .read_dir()
        .map(|rd| {
            rd.flatten().any(|e| {
                std::fs::read_to_string(e.path())
                    .map(|c| c.contains("2832") || c.contains("rtl") || c.contains("RTL"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    let dvb_loaded = std::fs::read_to_string("/proc/modules")
        .map(|m| m.contains("dvb_usb_rtl28xxu"))
        .unwrap_or(false);
    let _ = writeln!(
        s,
        "USB access\n  udev rules for SDRs: {}\n  DVB kernel driver:  {}",
        if have_rules {
            "present"
        } else {
            "NOT found — see packaging/70-deck-sdr.rules"
        },
        if dvb_loaded {
            "LOADED — blacklist dvb_usb_rtl28xxu (grabs the RTL dongle)"
        } else {
            "ok (not holding the dongle)"
        }
    );
    let _ = writeln!(s);

    match crate::audio::resolve_sink(cfg, &tools) {
        Some(sink) => {
            let _ = writeln!(s, "audio sink: {sink}");
        }
        None => {
            let _ = writeln!(
                s,
                "audio sink: NONE (install pulseaudio-utils / pipewire / alsa-utils)"
            );
        }
    }
    let _ = writeln!(s);

    // capability matrix per detected device
    for d in &devices {
        let _ = writeln!(s, "mode support on {} ({})", d.kind.label(), d.stable_id());
        for m in MODES {
            let line = match resolve(m.id, d, cfg, &tools, m.default_hz, cfg.sdr.gain) {
                None => "unsupported on this device".to_string(),
                Some(r) => {
                    let what = match &r.plan {
                        Plan::Iq {
                            demod, decoder_cmd, ..
                        } => {
                            let dec = decoder_cmd
                                .as_deref()
                                .map(|c| {
                                    format!(
                                        " → {}",
                                        c.split_whitespace().next().unwrap_or("decoder")
                                    )
                                })
                                .unwrap_or_default();
                            format!("iq/{demod:?}{dec}")
                        }
                        Plan::Extern { cmdline, .. } => format!(
                            "extern: {}",
                            cmdline.split_whitespace().next().unwrap_or("?")
                        ),
                    };
                    let range_ok = d.freq_ok(m.default_hz);
                    let mut flags = String::new();
                    if !r.missing.is_empty() {
                        let _ = write!(flags, "  MISSING: {}", r.missing.join(", "));
                    }
                    if !range_ok {
                        let _ = write!(flags, "  (default freq out of range)");
                    }
                    if let Some(n) = &r.note {
                        let _ = write!(flags, "  [{n}]");
                    }
                    format!("{what}{flags}")
                }
            };
            let _ = writeln!(s, "  {:<10} {line}", m.label);
        }
        let _ = writeln!(s);
    }
    s
}

fn run_with_timeout(cmdline: &str, timeout: Duration) -> (String, bool) {
    let Ok(mut sp) = crate::pipeline::spawn_shell(cmdline, true, false) else {
        return (String::new(), false);
    };
    let stdout = sp.child.stdout.take();
    let reader = stdout.map(|so| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let mut so = so;
            let _ = so.read_to_string(&mut s);
            s
        })
    });
    let started = Instant::now();
    let mut timed_out = true;
    while started.elapsed() < timeout {
        match sp.child.try_wait() {
            Ok(Some(_)) => {
                timed_out = false;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    if timed_out {
        crate::pipeline::kill_group(sp.pgid);
    }
    let out = reader.and_then(|h| h.join().ok()).unwrap_or_default();
    (out, !timed_out)
}

/// Pipe the simulator's signals through the real decoders and check the
/// decodes come back. Skips tests whose decoder isn't installed.
pub fn selftest() -> String {
    let mut s = String::new();
    let tools = ToolReport::scan();
    let deck = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "deck".into());

    let mut ran = 0;
    let mut passed = 0;
    let mut case = |name: &str, needs: &[&str], cmd: String, expect: &[&str], s: &mut String| {
        let missing: Vec<&&str> = needs.iter().filter(|b| !tools.has(b)).collect();
        if !missing.is_empty() {
            let _ = writeln!(
                s,
                "  SKIP {name} (missing: {})",
                missing.iter().map(|m| **m).collect::<Vec<_>>().join(", ")
            );
            return;
        }
        ran += 1;
        let (out, finished) = run_with_timeout(&cmd, Duration::from_secs(25));
        let ok = finished && expect.iter().all(|e| out.contains(e));
        if ok {
            passed += 1;
            let _ = writeln!(s, "  PASS {name}");
        } else {
            let _ = writeln!(s, "  FAIL {name}");
            let tail: String = out.chars().rev().take(300).collect::<String>();
            let tail: String = tail.chars().rev().collect();
            let _ = writeln!(s, "       output tail: {tail:?}");
        }
    };

    let _ = writeln!(s, "deck selftest (sim signal → real decoder)");
    case(
        "POCSAG → multimon-ng",
        &["multimon-ng"],
        format!(
            "'{deck}' simgen --mode pocsag --count 1 --fast \
             | multimon-ng -t raw -a POCSAG512 -a POCSAG1200 -a POCSAG2400 -"
        ),
        &["POCSAG1200", "1234567"],
        &mut s,
    );
    case(
        "APRS → multimon-ng",
        &["multimon-ng"],
        format!(
            "'{deck}' simgen --mode aprs --count 2 --fast \
             | multimon-ng -t raw -a AFSK1200 -"
        ),
        &["AFSK1200", "N0DECK"],
        &mut s,
    );
    case(
        "RTTY → minimodem",
        &["sox", "minimodem"],
        format!(
            "'{deck}' simgen --mode rtty --count 1 --fast \
             | sox -t raw -r 22050 -e signed -b 16 -c 1 - -t wav - \
             | minimodem --rx --quiet rtty --file /dev/stdin"
        ),
        &["DECK"],
        &mut s,
    );
    let _ = writeln!(s);
    let _ = writeln!(s, "  {passed}/{ran} ran tests passed");
    if ran == 0 {
        let _ = writeln!(s, "  (no decoders installed — nothing to test against)");
    }
    s
}
