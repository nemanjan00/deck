//! Splash, Devices, Doctor and the Power menu.

use super::{icons, panel_frame, DeckApp, Screen};
use crate::sys::PowerAction;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::sync::{Arc, Mutex};
use std::time::Instant;

// ------------------------------------------------------------------ splash

pub fn draw_splash(app: &mut DeckApp, ctx: &egui::Context, since: Instant) {
    let th = app.th.clone();
    let t = since.elapsed().as_secs_f32();
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(th.bg))
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            let p = ui.painter();
            let cx = rect.center().x;
            let cy = rect.center().y;
            let logo_size = rect.height().min(rect.width()) * 0.34;
            let logo_rect = Rect::from_center_size(
                Pos2::new(cx, cy - logo_size * 0.42),
                Vec2::splat(logo_size),
            );
            icons::logo(p, logo_rect, th.accent, th.text_faint);
            p.text(
                Pos2::new(cx, cy + logo_size * 0.32),
                Align2::CENTER_CENTER,
                "DECK",
                FontId::monospace(logo_size * 0.42),
                th.text,
            );
            p.text(
                Pos2::new(cx, cy + logo_size * 0.62),
                Align2::CENTER_CENTER,
                "handheld RX machine",
                FontId::proportional(logo_size * 0.11),
                th.text_dim,
            );
            p.text(
                Pos2::new(cx, rect.max.y - 18.0),
                Align2::CENTER_CENTER,
                format!("v{}", env!("CARGO_PKG_VERSION")),
                FontId::monospace(11.0),
                th.text_faint,
            );
            // sweep line
            let sweep_w = rect.width() * 0.4;
            let x0 = rect.min.x + (rect.width() + sweep_w) * ((t * 0.8) % 1.0) - sweep_w;
            let y = cy + logo_size * 0.82;
            for i in 0..24 {
                let x = x0 + sweep_w * i as f32 / 24.0;
                if x < rect.min.x || x > rect.max.x {
                    continue;
                }
                let a = (i as f32 / 24.0 * 255.0) as u8;
                p.line_segment(
                    [Pos2::new(x, y - 1.0), Pos2::new(x, y + 1.0)],
                    Stroke::new(
                        2.0,
                        egui::Color32::from_rgba_unmultiplied(
                            th.accent.r(),
                            th.accent.g(),
                            th.accent.b(),
                            a,
                        ),
                    ),
                );
            }
        });
}

// ----------------------------------------------------------------- devices

pub fn draw_devices(app: &mut DeckApp, ctx: &egui::Context) {
    let th = app.th.clone();
    app.status_bar(ctx, "Devices", true);
    app.hint_bar(ctx, "up/down select · OK activate · BACK menu");
    let devices = app.session.devices.clone();
    let active = app.session.active_dev;
    let focus = app.devices_focus;
    let mut pick: Option<usize> = None;
    let mut rescan = false;

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(th.bg).inner_margin(10.0))
        .show(ctx, |ui| {
            for (i, d) in devices.iter().enumerate() {
                let selected = i == focus;
                let is_active = i == active;
                let frame = panel_frame(&th);
                let resp =
                    frame
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                                icons::draw(
                                    ui.painter(),
                                    rect,
                                    if d.kind == crate::device::SdrKind::Sim {
                                        "waterfall"
                                    } else {
                                        "devices"
                                    },
                                    if is_active { th.accent } else { th.accent2 },
                                    th.text_faint,
                                );
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{}{}",
                                            d.kind.label(),
                                            if is_active { "  • active" } else { "" }
                                        ))
                                        .strong()
                                        .color(if is_active { th.accent } else { th.text }),
                                    );
                                    let ranges = d
                                        .kind
                                        .ranges()
                                        .iter()
                                        .map(|r| {
                                            format!(
                                                "{}–{}",
                                                crate::freq::fmt_short(*r.start()),
                                                crate::freq::fmt_short(*r.end())
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    ui.label(
                                        RichText::new(format!(
                                            "{}· {}{}",
                                            if d.usb_path.is_empty() {
                                                String::new()
                                            } else {
                                                format!("usb {} · ", d.usb_path)
                                            },
                                            ranges,
                                            d.serial
                                                .as_ref()
                                                .map(|s| format!(" · sn {s}"))
                                                .unwrap_or_default()
                                        ))
                                        .size(11.0)
                                        .color(th.text_dim),
                                    );
                                });
                            });
                        })
                        .response;
                let resp = resp.interact(Sense::click());
                if selected {
                    super::widgets::focus_ring(ui, resp.rect, &th);
                }
                if resp.clicked() {
                    pick = Some(i);
                }
                ui.add_space(4.0);
            }
            ui.add_space(6.0);
            if ui
                .button(RichText::new("rescan USB").color(th.accent2))
                .clicked()
            {
                rescan = true;
            }
            ui.add_space(10.0);
            // tools summary
            ui.label(
                RichText::new("DECODERS / TOOLS")
                    .size(10.5)
                    .color(th.text_faint),
            );
            ui.horizontal_wrapped(|ui| {
                for t in crate::pipeline::KNOWN_TOOLS {
                    let ok = app.session.tools.has(t);
                    ui.label(
                        RichText::new(format!("{} {t}", if ok { "+" } else { "-" }))
                            .font(FontId::monospace(11.5))
                            .color(if ok { th.ok } else { th.text_faint }),
                    );
                }
            });
        });

    if rescan {
        app.session.rescan_devices();
        app.refresh_support();
        app.session.set_status("devices rescanned");
    }
    if let Some(i) = pick {
        select_device(app, i);
    }
}

fn select_device(app: &mut DeckApp, i: usize) {
    if i >= app.session.devices.len() {
        return;
    }
    app.stop_rx();
    app.session.active_dev = i;
    app.devices_focus = i;
    app.refresh_support();
    app.session.save();
    let label = app.session.device().kind.label().to_string();
    app.session.set_status(format!("active SDR: {label}"));
}

pub fn devices_keys(app: &mut DeckApp, esc: bool, enter: bool, up: bool, down: bool) {
    let n = app.session.devices.len();
    if up {
        app.devices_focus = app.devices_focus.saturating_sub(1);
    }
    if down && n > 0 {
        app.devices_focus = (app.devices_focus + 1).min(n - 1);
    }
    if enter {
        select_device(app, app.devices_focus);
    }
    if esc {
        app.screen = Screen::Menu;
    }
}

// ---------------------------------------------------------------- recordings

#[derive(Default)]
pub struct RecordingsState {
    pub entries: Vec<crate::rec::RecEntry>,
    pub sel: usize,
    pub dir: std::path::PathBuf,
    pub player: Option<crate::rec::WavPlayer>,
}

impl RecordingsState {
    pub fn refresh(&mut self, cfg: &crate::config::Config) {
        self.dir = crate::rec::recordings_dir(&cfg.audio.record_dir);
        self.entries = crate::rec::list_recordings(&self.dir);
        if self.sel >= self.entries.len() {
            self.sel = self.entries.len().saturating_sub(1);
        }
    }

    fn stop_player(&mut self) {
        self.player = None; // Drop pauses + tears down the sink
    }
}

pub fn draw_recordings(app: &mut DeckApp, ctx: &egui::Context) {
    let th = app.th.clone();
    app.status_bar(ctx, "Recordings", true);
    app.hint_bar(
        ctx,
        "up/down select · OK play/stop · RIGHT delete · BACK menu",
    );
    let sel = app.recordings.sel;
    let mut play: Option<usize> = None;
    let mut delete: Option<usize> = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(th.bg).inner_margin(10.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new(app.recordings.dir.to_string_lossy())
                    .size(11.0)
                    .color(th.text_dim),
            );
            ui.add_space(4.0);
            if app.recordings.entries.is_empty() {
                ui.label(
                    RichText::new("no recordings yet — hit RECORD while receiving")
                        .color(th.text_faint),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, e) in app.recordings.entries.iter().enumerate() {
                        let selected = i == sel;
                        let dur = if e.is_wav {
                            format!("{:02}:{:02}", (e.secs as u32) / 60, (e.secs as u32) % 60)
                        } else {
                            "IQ".to_string()
                        };
                        let line = format!(
                            "{:<42} {:>8} {:>6}",
                            e.name,
                            crate::rec::fmt_size(e.bytes),
                            dur
                        );
                        let resp = ui.add(
                            egui::Label::new(
                                RichText::new(line)
                                    .font(FontId::monospace(12.5))
                                    .color(if selected { th.sel_fg } else { th.text })
                                    .background_color(if selected {
                                        th.sel_bg
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    }),
                            )
                            .sense(Sense::click())
                            .truncate(),
                        );
                        if resp.clicked() {
                            play = Some(i);
                        }
                        let del = ui.add(
                            egui::Button::new(RichText::new("x").size(13.0).color(th.err))
                                .small()
                                .frame(false),
                        );
                        if del.clicked() {
                            delete = Some(i);
                        }
                    }
                });
        });
    // embedded metadata of the selected recording
    if let Some(c) = app
        .recordings
        .entries
        .get(app.recordings.sel)
        .and_then(|e| e.comment.as_deref())
    {
        let th = app.th.clone();
        egui::TopBottomPanel::bottom("recmeta")
            .frame(
                egui::Frame::none()
                    .fill(th.panel)
                    .inner_margin(8.0)
                    .stroke(Stroke::new(1.0, th.outline)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("METADATA").size(10.0).color(th.text_faint));
                ui.label(
                    RichText::new(c)
                        .font(FontId::monospace(12.0))
                        .color(th.accent),
                );
            });
    }
    // transport bar for the loaded player
    let mut seek_to: Option<f32> = None;
    let mut toggle = false;
    let mut nudge: Option<f32> = None;
    if let Some(p) = &app.recordings.player {
        let th = app.th.clone();
        let (pos, dur, playing) = (p.position(), p.duration().max(0.001), p.is_playing());
        let name = p
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        egui::TopBottomPanel::bottom("transport")
            .frame(
                egui::Frame::none()
                    .fill(th.panel_hi)
                    .inner_margin(10.0)
                    .stroke(Stroke::new(1.0, th.outline)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(RichText::new("|◀").size(16.0)).frame(false))
                        .clicked()
                    {
                        nudge = Some(-5.0);
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if playing { "⏸" } else { "▶" })
                                    .size(18.0)
                                    .color(th.accent),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        toggle = true;
                    }
                    if ui
                        .add(egui::Button::new(RichText::new("▶|").size(16.0)).frame(false))
                        .clicked()
                    {
                        nudge = Some(5.0);
                    }
                    let fmt = |s: f32| format!("{:02}:{:02}", (s as u32) / 60, (s as u32) % 60);
                    ui.label(
                        RichText::new(format!("{} / {}", fmt(pos), fmt(dur)))
                            .font(FontId::monospace(13.0))
                            .color(th.text),
                    );
                });
                // clickable/scrubbable progress bar
                let (bar, resp) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 16.0),
                    Sense::click_and_drag(),
                );
                let p2 = ui.painter();
                p2.rect_filled(bar, 4.0, th.panel);
                let t = (pos / dur).clamp(0.0, 1.0);
                let mut fill = bar;
                fill.set_width(bar.width() * t);
                p2.rect_filled(fill, 4.0, th.accent.linear_multiply(0.7));
                if let Some(m) = resp.interact_pointer_pos() {
                    if resp.clicked() || resp.dragged() {
                        let f = ((m.x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
                        seek_to = Some(f * dur);
                    }
                }
                ui.label(RichText::new(name).size(10.5).color(th.text_dim));
            });
    }
    if let Some(p) = &app.recordings.player {
        if toggle {
            p.toggle();
        }
        if let Some(s) = seek_to {
            p.seek(s);
        }
        if let Some(d) = nudge {
            p.seek_by(d);
        }
    }

    if let Some(i) = play {
        app.recordings.sel = i;
        open_or_toggle(app, i);
    }
    if let Some(i) = delete {
        delete_recording(app, i);
    }
}

/// Open the selected recording in the player (or toggle play/pause if it's
/// already loaded). Raw IQ captures aren't playable as audio.
fn open_or_toggle(app: &mut DeckApp, i: usize) {
    let Some(e) = app.recordings.entries.get(i) else {
        return;
    };
    if !e.is_wav {
        app.session
            .set_status("raw IQ capture — not playable as audio");
        return;
    }
    let already = app
        .recordings
        .player
        .as_ref()
        .map(|p| p.path == e.path)
        .unwrap_or(false);
    if already {
        if let Some(p) = &app.recordings.player {
            p.toggle();
        }
        return;
    }
    let path = e.path.clone();
    let name = e.name.clone();
    match crate::rec::WavPlayer::open(&path) {
        Some(p) => {
            app.recordings.player = Some(p);
            app.session.set_status(format!("playing {name}"));
        }
        None => app
            .session
            .set_status("cannot play (no paplay/aplay, or bad file)"),
    }
}

fn delete_recording(app: &mut DeckApp, i: usize) {
    if let Some(e) = app.recordings.entries.get(i) {
        let _ = std::fs::remove_file(&e.path);
    }
    let cfg = app.session.cfg.clone();
    app.recordings.refresh(&cfg);
    app.session.set_status("deleted");
}

#[allow(clippy::too_many_arguments)]
pub fn recordings_keys(
    app: &mut DeckApp,
    esc: bool,
    enter: bool,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
) {
    let n = app.recordings.entries.len();
    if up {
        app.recordings.sel = app.recordings.sel.saturating_sub(1);
    }
    if down && n > 0 {
        app.recordings.sel = (app.recordings.sel + 1).min(n - 1);
    }
    if enter {
        open_or_toggle(app, app.recordings.sel);
    }
    // with a player loaded, ←/→ scrub ∓5 s; otherwise → deletes the row
    match &app.recordings.player {
        Some(p) => {
            if left {
                p.seek_by(-5.0);
            }
            if right {
                p.seek_by(5.0);
            }
        }
        None => {
            if right {
                delete_recording(app, app.recordings.sel);
            }
        }
    }
    if esc {
        if app.recordings.player.is_some() {
            app.recordings.stop_player(); // first BACK stops playback
        } else {
            app.screen = Screen::Menu;
        }
    }
}

// ------------------------------------------------------------------ doctor

#[derive(Default)]
pub struct DoctorState {
    pub report: Option<String>,
    pub selftest: Option<Arc<Mutex<Option<String>>>>,
}

impl DoctorState {
    pub fn ensure_report(&mut self, cfg: &crate::config::Config) {
        if self.report.is_none() {
            self.report = Some(crate::doctor::report(cfg));
        }
    }
}

pub fn draw_doctor(app: &mut DeckApp, ctx: &egui::Context) {
    let th = app.th.clone();
    app.status_bar(ctx, "Doctor", true);
    app.hint_bar(ctx, "BACK menu");
    let mut run_selftest = false;
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(th.bg).inner_margin(10.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("refresh").clicked() {
                    app.doctor.report = Some(crate::doctor::report(&app.session.cfg));
                }
                let testing = app
                    .doctor
                    .selftest
                    .as_ref()
                    .map(|s| s.lock().map(|g| g.is_none()).unwrap_or(false))
                    .unwrap_or(false);
                if testing {
                    ui.spinner();
                    ui.label(RichText::new("selftest running…").color(th.text_dim));
                } else if ui.button("run selftest (sim -> real decoders)").clicked() {
                    run_selftest = true;
                }
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(st) = &app.doctor.selftest {
                        if let Ok(g) = st.lock() {
                            if let Some(out) = g.as_ref() {
                                ui.label(
                                    RichText::new(out)
                                        .font(FontId::monospace(12.0))
                                        .color(th.accent),
                                );
                                ui.add_space(8.0);
                            }
                        }
                    }
                    if let Some(rep) = &app.doctor.report {
                        ui.label(
                            RichText::new(rep)
                                .font(FontId::monospace(12.0))
                                .color(th.text),
                        );
                    }
                });
        });
    if run_selftest {
        let slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        app.doctor.selftest = Some(slot.clone());
        std::thread::spawn(move || {
            let out = crate::doctor::selftest();
            if let Ok(mut g) = slot.lock() {
                *g = Some(out);
            }
        });
    }
}

// ------------------------------------------------------------------- power

const POWER_ITEMS: &[&str] = &["Suspend", "Reboot", "Power off", "Quit deck", "Cancel"];

pub fn power_keys(app: &mut DeckApp, esc: bool, enter: bool, up: bool, down: bool) {
    let Some(sel) = app.power_sel else {
        return;
    };
    if esc {
        app.power_sel = None;
        return;
    }
    let mut sel = sel;
    if up {
        sel = sel.saturating_sub(1);
    }
    if down {
        sel = (sel + 1).min(POWER_ITEMS.len() - 1);
    }
    app.power_sel = Some(sel);
    if enter {
        power_execute(app, sel);
    }
}

fn power_execute(app: &mut DeckApp, sel: usize) {
    app.power_sel = None;
    let act = match sel {
        0 => Some(PowerAction::Suspend),
        1 => Some(PowerAction::Reboot),
        2 => Some(PowerAction::PowerOff),
        3 => {
            app.stop_rx();
            app.session.save();
            std::process::exit(0);
        }
        _ => None,
    };
    if let Some(a) = act {
        app.stop_rx();
        app.session.save();
        if let Err(e) = a.execute() {
            app.power_err = Some(e.clone());
            app.session.set_status(e);
        }
    }
}

pub fn draw_power_popup(app: &mut DeckApp, ctx: &egui::Context) {
    let Some(sel) = app.power_sel else {
        return;
    };
    let th = app.th.clone();
    let mut click: Option<usize> = None;
    egui::Area::new(egui::Id::new("power"))
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(th.panel)
                .stroke(Stroke::new(1.0, th.outline))
                .rounding(12.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_min_width(220.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        icons::draw(ui.painter(), rect, "power", th.err, th.text_faint);
                        ui.label(RichText::new("POWER").strong().color(th.text));
                    });
                    ui.add_space(6.0);
                    for (i, item) in POWER_ITEMS.iter().enumerate() {
                        let selected = i == sel;
                        let danger = i == 1 || i == 2;
                        let color = if selected {
                            th.sel_fg
                        } else if danger {
                            th.err
                        } else {
                            th.text
                        };
                        let resp = ui.add(
                            egui::Label::new(
                                RichText::new(format!("  {item}  "))
                                    .font(FontId::proportional(15.0))
                                    .color(color)
                                    .background_color(if selected {
                                        th.sel_bg
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    }),
                            )
                            .sense(Sense::click()),
                        );
                        if resp.clicked() {
                            click = Some(i);
                        }
                        if resp.hovered() {
                            app.power_sel = Some(i);
                        }
                    }
                    if let Some(e) = &app.power_err {
                        ui.add_space(4.0);
                        ui.label(RichText::new(e).size(11.0).color(th.err));
                    }
                });
        });
    if let Some(i) = click {
        power_execute(app, i);
    }
}
