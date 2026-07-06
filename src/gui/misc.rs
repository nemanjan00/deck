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
