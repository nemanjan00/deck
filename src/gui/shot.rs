//! `deck shot` — render every screen headlessly (CPU rasterizer) into PNGs
//! for the README, with deterministic demo data. Doubles as a UI smoke test.

use super::{DeckApp, Screen};
use crate::dsp::Rng;
use crate::modes::ModeId;
use crate::parse::dsd;
use crate::parse::multimon::{AprsMsg, PagerContent, PagerMsg};
use crate::session::{LiveCall, Running, Session, Timed};
use crate::sys::{BatState, Battery};
use anyhow::Result;
use std::path::Path;
use std::time::{Duration, Instant};

fn demo_spectrum(rng: &mut Rng, n: usize, bumps: &[(f32, f32, f32)]) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = i as f32 / n as f32;
            let mut v = -84.0 + rng.gauss() as f32 * 2.0;
            for (pos, width, height) in bumps {
                let d = (x - pos) / width;
                v += height * (-d * d).exp();
            }
            v.min(-12.0)
        })
        .collect()
}

fn fill_band(app: &mut DeckApp, seed: u64) {
    let mut rng = Rng::new(seed);
    for k in 0..170 {
        let drift = (k as f32) * 0.0004;
        let spec = demo_spectrum(
            &mut rng,
            1024,
            &[
                (0.5, 0.004, 52.0),
                (0.62 + drift, 0.003, if k % 11 < 6 { 40.0 } else { 6.0 }),
                (0.30, 0.006, if k % 17 < 9 { 46.0 } else { 0.0 }),
                (0.75, 0.002, 30.0),
            ],
        );
        app.session.stores.wf_band.push(&spec, -80.0, 0.0);
        if k == 169 {
            app.session.stores.band_spec = spec;
        }
    }
}

fn fill_audio(app: &mut DeckApp, seed: u64) {
    let mut rng = Rng::new(seed);
    for k in 0..170 {
        let voice = if k % 23 < 13 { 30.0 } else { 4.0 };
        let spec = demo_spectrum(
            &mut rng,
            256,
            &[
                (0.06, 0.05, voice),
                (0.14, 0.08, voice * 0.8),
                (0.3, 0.1, voice * 0.35),
                (0.62, 0.02, 18.0), // a heterodyne the notch would kill
            ],
        );
        app.session.stores.wf_audio.push(&spec, -90.0, -10.0);
        if k == 169 {
            app.session.stores.audio_peak = spec.iter().map(|v| v + 4.0).collect();
            app.session.stores.audio_spec = spec;
        }
    }
    app.session.stores.audio_rms = 0.09;
    app.session.stores.last_rms_at = Instant::now();
}

/// A fake running session (backend = a sleeping child) so screenshots show
/// the on-air state without hardware.
fn fake_running(app: &mut DeckApp, mode: ModeId, freq: u64) {
    if let Ok(sp) = crate::pipeline::spawn_shell("sleep 30", false, false) {
        let center = freq + 600_000;
        app.session.running = Some(Running {
            run: 999,
            mode,
            backend: crate::session::Backend::Extern {
                child: sp,
                sbs_stop: None,
            },
            knobs: crate::audio::Knobs::new(freq as i64 - center as i64),
            started: Instant::now() - Duration::from_secs(154),
            center_hz: center,
            rate: 2_400_000,
            freq_hz: freq,
            note: None,
            audio_capable: true,
            monitorable: true,
        });
    }
}

fn demo_app(scene: &str, dark: bool) -> DeckApp {
    let mut session = Session::new(None);
    // deterministic environment for screenshots
    session.persist.theme = Some(if dark { "dark" } else { "light" }.into());
    let sim_idx = session.devices.len() - 1;
    session.active_dev = sim_idx;
    let mut app = DeckApp::new(session, false);
    app.sys.battery = Some(Battery {
        percent: 87,
        state: BatState::Discharging,
    });
    app.sys.volume = Some(65);
    app.sys.muted = false;

    match scene {
        "menu" => {}
        "nfm" => {
            let ui = app.mode_ui(ModeId::Nfm);
            ui.freq.hz = 145_500_000;
            ui.mp.nr = 2;
            ui.mp.squelch = 0.03;
            ui.mp.lp = 3600;
            ui.viz = 0;
            fake_running(&mut app, ModeId::Nfm, 145_500_000);
            fill_audio(&mut app, 5);
        }
        "waterfall" => {
            let ui = app.mode_ui(ModeId::Waterfall);
            ui.freq.hz = 433_920_000;
            fake_running(&mut app, ModeId::Waterfall, 433_920_000);
            fill_band(&mut app, 9);
            for (hz, db) in [
                (433_920_000u64, -28.0f32),
                (434_213_000, -39.0),
                (433_437_000, -44.0),
                (434_775_000, -52.0),
            ] {
                app.session.stores.peaks.push(crate::session::Peak {
                    hz,
                    db,
                    last: Instant::now(),
                });
            }
        }
        "dmr" => {
            let ui = app.mode_ui(ModeId::Dmr);
            ui.freq.hz = 438_800_000;
            fake_running(&mut app, ModeId::Dmr, 438_800_000);
            fill_audio(&mut app, 7);
            let mut fields = dsd::parse_line(
                "Slot 2 TLC DMR | Color Code=01 | Group Call | TG: 2311 | RID: 2621234",
            );
            fields.merge(dsd::parse_line("Slot 2 VC3 DMR | TG: 2311 | RID: 2621234"));
            app.session.stores.call = Some(LiveCall {
                fields,
                started: Instant::now() - Duration::from_secs(7),
                last: Instant::now(),
            });
            for (i, (tg, src, dur)) in [
                ("2311", "2621234", 12.4f32),
                ("91", "2620015", 33.0),
                ("2311", "2624477", 5.2),
                ("9", "2629001", 61.8),
            ]
            .iter()
            .enumerate()
            {
                let f = dsd::CallFields {
                    tg: Some((*tg).into()),
                    src: Some((*src).into()),
                    slot: Some(if i % 2 == 0 { 2 } else { 1 }),
                    ..Default::default()
                };
                app.session
                    .stores
                    .call_history
                    .push_back(crate::session::CallRecord {
                        at: chrono::Local::now() - chrono::Duration::seconds(40 * (i as i64 + 1)),
                        fields: f,
                        dur_s: *dur,
                    });
            }
        }
        "pocsag" => {
            let ui = app.mode_ui(ModeId::Pocsag);
            ui.freq.hz = 169_650_000;
            fake_running(&mut app, ModeId::Pocsag, 169_650_000);
            fill_audio(&mut app, 11);
            let msgs = [
                (
                    1_234_567u32,
                    "A2 Ambulancepost Aalsmeer Straatweg 21 lift vast",
                ),
                (
                    2_002_412,
                    "P 1 BDH-01 Liftopsluiting Marktplein 4 Zandvoort",
                ),
                (1_990_007, "Testoproep regio 12 — geen actie vereist"),
                (1_234_569, "GRIP1 oefening MELDKAMER conform draaiboek"),
            ];
            for (i, (addr, text)) in msgs.iter().enumerate() {
                app.session.stores.pagers.push_back(Timed {
                    at: chrono::Local::now() - chrono::Duration::seconds(31 * i as i64),
                    msg: PagerMsg {
                        baud: 1200,
                        address: addr.to_string(),
                        function: "0".into(),
                        content: PagerContent::Alpha((*text).into()),
                    },
                });
            }
            app.session.stores.pagers.push_back(Timed {
                at: chrono::Local::now() - chrono::Duration::seconds(150),
                msg: PagerMsg {
                    baud: 512,
                    address: "200042".into(),
                    function: "3".into(),
                    content: PagerContent::Numeric("0612345678".into()),
                },
            });
            app.session.stores.decoded = 5;
        }
        "aprs" => {
            let ui = app.mode_ui(ModeId::Aprs);
            ui.freq.hz = 144_800_000;
            fake_running(&mut app, ModeId::Aprs, 144_800_000);
            fill_audio(&mut app, 13);
            for (i, (from, info)) in [
                ("YU1ABC-9", "!4447.40N/02027.60E>073/036 mobile via deck"),
                ("YT2XYZ-7", ">deck handheld online"),
                ("S52DK-1", "T#123,123,045,678,010,090,00000000"),
                ("9A3W", "!4550.12N/01559.55E#PHG5360 digi Sljeme"),
            ]
            .iter()
            .enumerate()
            {
                app.session.stores.aprs.push_back(Timed {
                    at: chrono::Local::now() - chrono::Duration::seconds(45 * i as i64),
                    msg: AprsMsg {
                        from: (*from).into(),
                        to: "APDECK".into(),
                        path: "WIDE1-1,WIDE2-2".into(),
                        info: (*info).into(),
                    },
                });
            }
        }
        "adsb" => {
            fake_running(&mut app, ModeId::Adsb, 1_090_000_000);
            let mut fleet = crate::sim::adsb::Fleet::new(42, 7, 44.82, 20.29);
            let t0 = Instant::now();
            for _ in 0..40 {
                for line in fleet.step(1.0) {
                    app.session.stores.aircraft.push_line(&line, t0);
                }
            }
            app.session.stores.sbs_note = Some("SBS connected (127.0.0.1:30003)".into());
        }
        "scanner" => {
            let ui = app.mode_ui(ModeId::Scanner);
            ui.mp.squelch = 0.045;
            fake_running(&mut app, ModeId::Scanner, 446_006_250);
            fill_audio(&mut app, 17);
            app.session.scan.cur = 2;
            app.session.scan.phase = crate::session::ScanPhase::Hold;
            app.session.scan.channels[2].hits = 4;
            app.session.scan.channels[5].hits = 1;
            app.session.scan.channels[7].locked = true;
            for (i, label) in ["PMR446 ch3 (446.0313 MHz)", "2m calling (145.5 MHz)"]
                .iter()
                .enumerate()
            {
                app.session.scan.hits.push_back(Timed {
                    at: chrono::Local::now() - chrono::Duration::seconds(60 * i as i64),
                    msg: (*label).to_string(),
                });
            }
            app.session.stores.audio_rms = 0.11;
        }
        _ => {}
    }

    app.screen = match scene {
        "menu" => Screen::Menu,
        "adsb" => Screen::Mode(ModeId::Adsb),
        "nfm" => Screen::Mode(ModeId::Nfm),
        "waterfall" => Screen::Mode(ModeId::Waterfall),
        "dmr" => Screen::Mode(ModeId::Dmr),
        "pocsag" => Screen::Mode(ModeId::Pocsag),
        "aprs" => Screen::Mode(ModeId::Aprs),
        "scanner" => Screen::Mode(ModeId::Scanner),
        _ => Screen::Menu,
    };
    if !dark {
        app.set_theme(false);
    }
    app
}

pub struct ShotSpec {
    pub name: &'static str,
    pub scene: &'static str,
    pub dark: bool,
    pub w: u32,
    pub h: u32,
}

pub fn default_shots() -> Vec<ShotSpec> {
    let mut v = Vec::new();
    for scene in [
        "menu",
        "nfm",
        "waterfall",
        "dmr",
        "pocsag",
        "aprs",
        "adsb",
        "scanner",
    ] {
        v.push(ShotSpec {
            name: scene,
            scene,
            dark: true,
            w: 880,
            h: 520,
        });
    }
    for scene in ["menu", "nfm", "pocsag"] {
        v.push(ShotSpec {
            name: scene,
            scene,
            dark: false,
            w: 880,
            h: 520,
        });
    }
    // square handheld form factor (Hackberry/Mecha class)
    for scene in ["menu", "nfm"] {
        v.push(ShotSpec {
            name: scene,
            scene,
            dark: true,
            w: 480,
            h: 480,
        });
    }
    v
}

pub fn run(out_dir: &Path) -> Result<()> {
    let shots = default_shots();
    let n = shots.len();
    for s in shots {
        let mut app = demo_app(s.scene, s.dark);
        let rgba = super::raster::render_rgba(s.w, s.h, 1.0, 3, |ctx| {
            app.frame(ctx);
        });
        // fake_running spawned a sleeper; kill it
        app.session.stop();
        let suffix = format!(
            "{}{}",
            if s.dark { "" } else { "-light" },
            if s.w == s.h { "-sq" } else { "" }
        );
        let path = out_dir.join(format!("{}{}.png", s.name, suffix));
        super::raster::write_png(&path, s.w, s.h, &rgba)?;
        println!("wrote {}", path.display());
    }
    println!("{n} screenshots in {}", out_dir.display());
    Ok(())
}
