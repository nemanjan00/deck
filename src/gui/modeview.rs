//! The per-mode screen: freq tuner, control stack, viz (spectrum/waterfall/
//! band scope) and the view-specific content (call card, pager/APRS tables,
//! aircraft table, text feed, scanner).

use super::theme::Theme;
use super::{panel_frame, widgets, DeckApp, Screen};
use crate::audio::f32_bits;
use crate::dsp::{HP_LADDER, LP_LADDER};
use crate::modes::{mode_def, Demod, ModeId, PipeKind, ViewKind};
use crate::parse::multimon::PagerContent;
use crate::session::{nb_level, nr_level, ScanPhase, NB_LEVELS, NR_LEVELS};
use eframe::egui::{self, Align2, FontId, RichText, Sense, Stroke, Vec2};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ctl {
    Rx,
    Preset,
    Gain,
    Sql,
    Nr,
    Nb,
    Notch,
    Hp,
    Lp,
    Det,
    Mon,
    Rec,
    Viz,
    Pause,
    Log,
}

fn controls_for(mode: ModeId) -> Vec<Ctl> {
    let def = mode_def(mode);
    let mut v = vec![Ctl::Rx];
    if !def.presets.is_empty() {
        v.push(Ctl::Preset);
    }
    if matches!(def.pipe, PipeKind::Iq(_)) {
        v.push(Ctl::Gain);
        if def.view != ViewKind::Waterfall {
            v.push(Ctl::Sql);
            v.push(Ctl::Nr);
            v.push(Ctl::Nb);
            v.push(Ctl::Notch);
            v.push(Ctl::Hp);
            v.push(Ctl::Lp);
            if matches!(def.pipe, PipeKind::Iq(Demod::Am)) {
                v.push(Ctl::Det);
            }
            if def.decoder.is_some() {
                v.push(Ctl::Mon);
            }
            v.push(Ctl::Rec);
            v.push(Ctl::Viz);
        }
        if mode == ModeId::Scanner {
            v.push(Ctl::Pause);
        }
    }
    v.push(Ctl::Log);
    v
}

fn has_list(view: ViewKind) -> bool {
    matches!(
        view,
        ViewKind::Pager | ViewKind::Aprs | ViewKind::Adsb | ViewKind::Scanner | ViewKind::Voice
    )
}

fn ctl_label(c: Ctl) -> &'static str {
    match c {
        Ctl::Rx => "RX",
        Ctl::Preset => "PRESETS",
        Ctl::Gain => "GAIN",
        Ctl::Sql => "SQUELCH",
        Ctl::Nr => "NOISE REDUCE",
        Ctl::Nb => "NOISE BLANK",
        Ctl::Notch => "AUTO NOTCH",
        Ctl::Hp => "HP FILTER",
        Ctl::Lp => "LP FILTER",
        Ctl::Det => "AM DETECTOR",
        Ctl::Mon => "MONITOR",
        Ctl::Rec => "RECORD",
        Ctl::Viz => "VIZ",
        Ctl::Pause => "SCAN",
        Ctl::Log => "LOG",
    }
}

fn viz_label(v: u8) -> &'static str {
    match v {
        0 => "spectrum",
        1 => "audio fall",
        2 => "band scope",
        _ => "band fall",
    }
}

fn ctl_value(app: &DeckApp, mode: ModeId, c: Ctl) -> String {
    let ui = app.mode_ui.get(&mode);
    let mp = ui.map(|u| u.mp.clone()).unwrap_or_default();
    match c {
        Ctl::Rx => {
            if app.running_mode() == Some(mode) {
                "on air".into()
            } else {
                "stopped".into()
            }
        }
        Ctl::Preset => "…".into(),
        Ctl::Gain => {
            if mp.gain <= 0.0 {
                "auto".into()
            } else {
                format!("{:.1} dB", mp.gain)
            }
        }
        Ctl::Sql => {
            if mp.squelch <= 0.0 {
                "open".into()
            } else {
                format!("{:.0} dB", 20.0 * mp.squelch.max(1e-5).log10())
            }
        }
        Ctl::Nr => ["off", "low", "med", "high"][(mp.nr as usize).min(3)].into(),
        Ctl::Nb => ["off", "soft", "hard"][(mp.nb as usize).min(2)].into(),
        Ctl::Notch => if mp.notch { "on" } else { "off" }.into(),
        Ctl::Hp => {
            if mp.hp == 0 {
                "off".into()
            } else {
                format!("{} Hz", mp.hp)
            }
        }
        Ctl::Lp => {
            if mp.lp == 0 {
                "off".into()
            } else {
                format!("{:.1} kHz", mp.lp as f32 / 1000.0)
            }
        }
        Ctl::Det => if mp.det == 1 { "sync" } else { "envelope" }.into(),
        Ctl::Mon => if mp.monitor { "on" } else { "muted" }.into(),
        Ctl::Rec => match app.session.stores.rec_since {
            Some(t) => {
                let s = t.elapsed().as_secs();
                format!("• {:02}:{:02}", s / 60, s % 60)
            }
            None => "off".into(),
        },
        Ctl::Viz => viz_label(ui.map(|u| u.viz).unwrap_or(0)).into(),
        Ctl::Pause => {
            if app.session.scan.phase == ScanPhase::Paused {
                "paused".into()
            } else {
                "running".into()
            }
        }
        Ctl::Log => if ui.map(|u| u.show_log).unwrap_or(false) {
            "shown"
        } else {
            "hidden"
        }
        .into(),
    }
}

/// Push the current mp values into live engine knobs (when running).
fn apply_knobs(app: &mut DeckApp, mode: ModeId) {
    let Some(r) = &app.session.running else {
        return;
    };
    if r.mode != mode {
        return;
    }
    let mp = app
        .mode_ui
        .get(&mode)
        .map(|u| u.mp.clone())
        .unwrap_or_default();
    let k = &r.knobs;
    k.nr.store(f32_bits(nr_level(mp.nr)), Ordering::Relaxed);
    k.nb.store(f32_bits(nb_level(mp.nb)), Ordering::Relaxed);
    k.notch.store(mp.notch, Ordering::Relaxed);
    k.hp_hz.store(mp.hp, Ordering::Relaxed);
    k.lp_hz.store(mp.lp, Ordering::Relaxed);
    k.squelch.store(f32_bits(mp.squelch), Ordering::Relaxed);
    k.sync_det.store(mp.det == 1, Ordering::Relaxed);
    if r.monitorable {
        k.mute.store(!mp.monitor, Ordering::Relaxed);
    }
}

fn adjust(app: &mut DeckApp, mode: ModeId, c: Ctl, dir: i32) {
    {
        let ui = app.mode_ui(mode);
        let mp = &mut ui.mp;
        match c {
            Ctl::Gain => {
                mp.gain = (mp.gain + dir as f32).clamp(0.0, 49.6);
                if app.running_mode() == Some(mode) {
                    app.mode_ui(mode).gain_restart_at =
                        Some(Instant::now() + Duration::from_millis(700));
                }
                return;
            }
            Ctl::Sql => mp.squelch = (mp.squelch + dir as f32 * 0.005).clamp(0.0, 0.4),
            Ctl::Nr => mp.nr = (mp.nr as i32 + dir).rem_euclid(NR_LEVELS.len() as i32) as u8,
            Ctl::Nb => mp.nb = (mp.nb as i32 + dir).rem_euclid(NB_LEVELS.len() as i32) as u8,
            Ctl::Notch => mp.notch = !mp.notch,
            Ctl::Hp => {
                let i = HP_LADDER.iter().position(|h| *h == mp.hp).unwrap_or(0);
                let i = (i as i32 + dir).rem_euclid(HP_LADDER.len() as i32) as usize;
                mp.hp = HP_LADDER[i];
            }
            Ctl::Lp => {
                let i = LP_LADDER.iter().position(|h| *h == mp.lp).unwrap_or(0);
                let i = (i as i32 + dir).rem_euclid(LP_LADDER.len() as i32) as usize;
                mp.lp = LP_LADDER[i];
            }
            Ctl::Det => mp.det = if mp.det == 1 { 0 } else { 1 },
            Ctl::Mon => mp.monitor = !mp.monitor,
            Ctl::Viz => {
                let n = 4;
                ui.viz = ((ui.viz as i32 + dir).rem_euclid(n)) as u8;
                return;
            }
            _ => return,
        }
    }
    apply_knobs(app, mode);
}

fn activate(app: &mut DeckApp, mode: ModeId, c: Ctl) {
    match c {
        Ctl::Rx => {
            if app.running_mode() == Some(mode) {
                app.stop_rx();
            } else {
                app.start_rx(mode);
            }
        }
        Ctl::Preset => {
            let ui = app.mode_ui(mode);
            ui.preset_open = !ui.preset_open;
            ui.preset_sel = 0;
        }
        Ctl::Rec => app.session.toggle_record(mode),
        Ctl::Pause => {
            let s = &mut app.session.scan;
            s.phase = if s.phase == ScanPhase::Paused {
                ScanPhase::Sampling
            } else {
                ScanPhase::Paused
            };
            s.phase_since = Instant::now();
        }
        Ctl::Log => {
            let ui = app.mode_ui(mode);
            ui.show_log = !ui.show_log;
        }
        Ctl::Notch | Ctl::Mon | Ctl::Det => adjust(app, mode, c, 1),
        _ => adjust(app, mode, c, 1),
    }
}

fn schedule_retune(app: &mut DeckApp, mode: ModeId) {
    let hz = app.mode_ui(mode).freq.hz;
    let ui = app.mode_ui(mode);
    ui.mp.freq = hz;
    if app.running_mode() == Some(mode) {
        app.mode_ui(mode).retune_at = Some(Instant::now() + Duration::from_millis(350));
    }
}

// ------------------------------------------------------------------- keys

#[allow(clippy::too_many_arguments)]
pub fn keys(
    app: &mut DeckApp,
    ctx: &egui::Context,
    mode: ModeId,
    esc: bool,
    enter: bool,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
) {
    let view = mode_def(mode).view;
    let ctls = controls_for(mode);
    let n_focus = 1 + ctls.len() + usize::from(has_list(view));

    // digit typing goes straight into the tuner when it has focus
    let digit = ctx.input(|i| {
        for (k, d) in [
            (egui::Key::Num0, 0u8),
            (egui::Key::Num1, 1),
            (egui::Key::Num2, 2),
            (egui::Key::Num3, 3),
            (egui::Key::Num4, 4),
            (egui::Key::Num5, 5),
            (egui::Key::Num6, 6),
            (egui::Key::Num7, 7),
            (egui::Key::Num8, 8),
            (egui::Key::Num9, 9),
        ] {
            if i.key_pressed(k) {
                return Some(d);
            }
        }
        None
    });

    // preset popup layer
    if app.mode_ui(mode).preset_open {
        let presets = app.session.presets(mode);
        let ui = app.mode_ui(mode);
        if up {
            ui.preset_sel = ui.preset_sel.saturating_sub(1);
        }
        if down {
            ui.preset_sel = (ui.preset_sel + 1).min(presets.len().saturating_sub(1));
        }
        if esc {
            ui.preset_open = false;
        }
        if enter {
            if let Some(p) = presets.get(ui.preset_sel) {
                ui.freq.hz = p.hz;
                ui.preset_open = false;
                schedule_retune(app, mode);
            }
        }
        return;
    }

    let focus = app.mode_ui(mode).focus;
    // FREQ focused (index 0)
    if focus == 0 {
        let editing = app.mode_ui(mode).editing_freq;
        if let Some(d) = digit {
            app.mode_ui(mode).freq.type_digit(d);
            schedule_retune(app, mode);
        }
        if editing {
            let ui = app.mode_ui(mode);
            if left {
                ui.freq.left();
            }
            if right {
                ui.freq.right();
            }
            let mut changed = false;
            if up {
                ui.freq.up();
                changed = true;
            }
            if down {
                ui.freq.down();
                changed = true;
            }
            if changed {
                schedule_retune(app, mode);
            }
            if enter || esc {
                app.mode_ui(mode).editing_freq = false;
            }
            return;
        }
        if enter {
            app.mode_ui(mode).editing_freq = true;
            return;
        }
        if left || right {
            let ui = app.mode_ui(mode);
            if left {
                ui.freq.down();
            } else {
                ui.freq.up();
            }
            schedule_retune(app, mode);
        }
    } else if focus <= ctls.len() {
        let c = ctls[focus - 1];
        if enter {
            activate(app, mode, c);
        }
        if left {
            adjust(app, mode, c, -1);
        }
        if right {
            adjust(app, mode, c, 1);
        }
    } else {
        // LIST focus
        let ui_list = app.mode_ui(mode).list_mode;
        if !ui_list && enter {
            app.mode_ui(mode).list_mode = true;
            return;
        }
        if ui_list {
            list_keys(app, mode, view, esc, enter, up, down, left, right);
            return;
        }
    }

    // focus movement + back
    if up {
        let ui = app.mode_ui(mode);
        ui.focus = ui.focus.saturating_sub(1);
    }
    if down {
        let ui = app.mode_ui(mode);
        ui.focus = (ui.focus + 1).min(n_focus - 1);
    }
    if esc {
        app.snapshot_mode(mode);
        app.session.save();
        app.screen = Screen::Menu;
    }
}

#[allow(clippy::too_many_arguments)]
fn list_keys(
    app: &mut DeckApp,
    mode: ModeId,
    view: ViewKind,
    esc: bool,
    enter: bool,
    up: bool,
    down: bool,
    _left: bool,
    right: bool,
) {
    let len = match view {
        ViewKind::Pager => app.session.stores.pagers.len(),
        ViewKind::Aprs => app.session.stores.aprs.len(),
        ViewKind::Adsb => app.session.stores.aircraft.len(),
        ViewKind::Scanner => app.session.scan.channels.len(),
        ViewKind::Voice => app.session.stores.call_history.len(),
        _ => 0,
    };
    let ui = app.mode_ui(mode);
    if up {
        ui.list_sel = ui.list_sel.saturating_sub(1);
    }
    if down && len > 0 {
        ui.list_sel = (ui.list_sel + 1).min(len - 1);
    }
    if esc {
        ui.list_mode = false;
        return;
    }
    let sel = ui.list_sel;
    match view {
        ViewKind::Pager => {
            if enter {
                if let Some(m) = app.session.stores.pagers.get(sel) {
                    app.detail = Some(format!(
                        "{}  POCSAG{}  addr {}  fn {}\n\n{}",
                        m.at.format("%H:%M:%S"),
                        m.msg.baud,
                        m.msg.address,
                        m.msg.function,
                        match &m.msg.content {
                            PagerContent::Alpha(t) => t.clone(),
                            PagerContent::Numeric(t) => format!("numeric: {t}"),
                            PagerContent::ToneOnly => "(tone only)".into(),
                        }
                    ));
                }
            }
        }
        ViewKind::Aprs => {
            if enter {
                if let Some(m) = app.session.stores.aprs.get(sel) {
                    app.detail = Some(format!(
                        "{}  {} → {} via {}\n\n{}",
                        m.at.format("%H:%M:%S"),
                        m.msg.from,
                        m.msg.to,
                        m.msg.path,
                        m.msg.info
                    ));
                }
            }
        }
        ViewKind::Scanner => {
            if enter {
                app.session.scan.cur = sel;
                app.session.scan.phase = ScanPhase::Sampling;
                app.session.scan.phase_since = Instant::now();
                let hz = app.session.scan.channels[sel].hz;
                if let Err(e) = app.session.retune(hz) {
                    app.session.set_status(e);
                }
            }
            if right {
                app.session.toggle_lockout(sel);
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------- draw

pub fn draw(app: &mut DeckApp, ctx: &egui::Context, mode: ModeId) {
    let def = mode_def(mode);
    let th = app.th.clone();
    let running = app.running_mode() == Some(mode);
    let elapsed = app
        .session
        .running
        .as_ref()
        .filter(|r| r.mode == mode)
        .map(|r| {
            let s = r.started.elapsed().as_secs();
            format!("  • RX {:02}:{:02}", s / 60, s % 60)
        })
        .unwrap_or_default();
    let title = format!("{}{}", def.label, elapsed);
    let _ = running;
    app.status_bar(ctx, &title, true);
    let hint = if app.mode_ui(mode).editing_freq {
        "left/right digit · up/down value · OK done"
    } else if app.mode_ui(mode).list_mode {
        "up/down select · OK open/jump · RIGHT lockout · BACK controls"
    } else {
        "up/down focus · left/right adjust · OK toggle · BACK menu"
    };
    app.hint_bar(ctx, hint);

    let screen_w = ctx.screen_rect().width();
    let wide = screen_w >= 660.0;

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(th.bg).inner_margin(8.0))
        .show(ctx, |ui| {
            if wide {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(290.0, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| draw_left(app, ui, mode, &th, false),
                    );
                    ui.add_space(6.0);
                    ui.vertical(|ui| draw_right(app, ui, mode, &th));
                });
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        draw_left(app, ui, mode, &th, true);
                        ui.add_space(6.0);
                        draw_right(app, ui, mode, &th);
                    });
            }
        });

    draw_preset_popup(app, ctx, mode, &th);
    if app.mode_ui(mode).show_log {
        draw_log(app, ctx, &th);
    }
}

fn draw_left(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme, compact: bool) {
    let running = app.running_mode() == Some(mode);
    let ctls = controls_for(mode);
    let focus = app.mode_ui(mode).focus;

    // freq tuner
    panel_frame(th).show(ui, |ui| {
        let (editing, focused) = {
            let u = app.mode_ui(mode);
            (u.editing_freq, focus == 0)
        };
        let mut drag_acc = std::mem::take(&mut app.mode_ui(mode).drag_acc);
        let mut f = app.mode_ui(mode).freq;
        let out = widgets::freq_tuner(ui, th, &mut f, editing, focused, &mut drag_acc, compact);
        {
            let u = app.mode_ui(mode);
            u.freq = f;
            u.drag_acc = drag_acc;
            if out.tapped {
                u.focus = 0;
                u.editing_freq = true;
            }
        }
        if out.changed {
            schedule_retune(app, mode);
        }
        // band fit indicator
        let dev = app.session.device();
        let ok = dev.freq_ok(app.mode_ui.get(&mode).map(|u| u.freq.hz).unwrap_or(0));
        let msg = if ok {
            format!(
                "{} · in band",
                crate::freq::fmt_short(app.mode_ui(mode).freq.hz)
            )
        } else {
            format!("outside {} range", dev.kind.label())
        };
        ui.label(
            RichText::new(msg)
                .size(10.5)
                .color(if ok { th.text_dim } else { th.warn }),
        );
    });
    ui.add_space(4.0);

    // RX button (focus index 1 == ctls[0] == Rx)
    let rx_focused = focus == 1;
    let label = if running {
        "■  STOP"
    } else {
        "START RX"
    };
    if widgets::action_button(ui, th, label, running, rx_focused).clicked() {
        activate(app, mode, Ctl::Rx);
    }
    ui.add_space(4.0);

    // control rows
    for (i, c) in ctls.iter().enumerate().skip(1) {
        let focused = focus == 1 + i;
        let value = ctl_value(app, mode, *c);
        let active = matches!(c, Ctl::Rec) && app.session.stores.rec_path.is_some();
        let resp = widgets::control_row(
            ui,
            th,
            widgets::ControlRow {
                label: ctl_label(*c),
                value,
                active,
                focused,
                enabled: true,
            },
        );
        if resp.clicked() {
            app.mode_ui(mode).focus = 1 + i;
            activate(app, mode, *c);
        }
        if resp.dragged() {
            let mut acc = std::mem::take(&mut app.mode_ui(mode).ctl_drag);
            acc += resp.drag_delta().x;
            while acc >= 22.0 {
                adjust(app, mode, *c, 1);
                acc -= 22.0;
            }
            while acc <= -22.0 {
                adjust(app, mode, *c, -1);
                acc += 22.0;
            }
            app.mode_ui(mode).ctl_drag = acc;
        }
        if focused {
            resp.scroll_to_me(None);
        }
    }

    // LIST pseudo-row
    let view = mode_def(mode).view;
    if has_list(view) {
        let focused = focus == 1 + ctls.len();
        let in_list = app.mode_ui(mode).list_mode;
        let count = match view {
            ViewKind::Pager => app.session.stores.pagers.len(),
            ViewKind::Aprs => app.session.stores.aprs.len(),
            ViewKind::Adsb => app.session.stores.aircraft.len(),
            ViewKind::Scanner => app.session.scan.channels.len(),
            ViewKind::Voice => app.session.stores.call_history.len(),
            _ => 0,
        };
        let resp = widgets::control_row(
            ui,
            th,
            widgets::ControlRow {
                label: "LIST",
                value: format!("{count} >"),
                active: in_list,
                focused,
                enabled: count > 0,
            },
        );
        if resp.clicked() {
            let u = app.mode_ui(mode);
            u.focus = 1 + ctls.len();
            u.list_mode = !u.list_mode;
        }
    }
}

fn draw_right(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    let def = mode_def(mode);
    let view = def.view;
    let avail_h = ui.available_height();

    // ---- viz ----
    let viz_h = match view {
        ViewKind::Audio => (avail_h * 0.52).clamp(120.0, 420.0),
        ViewKind::Waterfall => avail_h - 8.0,
        ViewKind::Adsb => 0.0,
        _ => 92.0,
    };
    if viz_h > 0.0 {
        draw_viz(app, ui, mode, th, viz_h);
        ui.add_space(4.0);
    }

    match view {
        ViewKind::Audio => draw_audio_info(app, ui, mode, th),
        ViewKind::Voice => draw_voice(app, ui, mode, th),
        ViewKind::Pager => draw_pager(app, ui, mode, th),
        ViewKind::Aprs => draw_aprs(app, ui, mode, th),
        ViewKind::Adsb => draw_adsb(app, ui, th),
        ViewKind::TextFeed => draw_textfeed(app, ui, th),
        ViewKind::Scanner => draw_scanner(app, ui, mode, th),
        ViewKind::Waterfall => {}
    }
}

fn draw_viz(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme, h: f32) {
    let view = mode_def(mode).view;
    let viz = if view == ViewKind::Waterfall {
        3
    } else if view == ViewKind::Audio || view == ViewKind::Scanner {
        app.mode_ui(mode).viz
    } else {
        0
    };
    let running = app.running_mode() == Some(mode);

    // marker position for band displays
    let band_marker = app.session.running.as_ref().and_then(|r| {
        if r.rate == 0 {
            return None;
        }
        let freq = app
            .mode_ui
            .get(&mode)
            .map(|u| u.freq.hz)
            .unwrap_or(r.freq_hz);
        Some(0.5 + (freq as f64 - r.center_hz as f64) as f32 / r.rate as f32)
    });

    let resp = match viz {
        0 => {
            let spec = app.session.stores.audio_spec.clone();
            let peak = app.session.stores.audio_peak.clone();
            widgets::spectrum(ui, th, h, &spec, Some(&peak), (-90.0, -10.0), None)
        }
        1 => {
            let wf = &app.session.stores.wf_audio;
            let mut slot = std::mem::take(&mut app.wf_audio_tex);
            let r = widgets::waterfall(ui, th, wf, &mut slot, h, None);
            app.wf_audio_tex = slot;
            r
        }
        2 => {
            let spec = app.session.stores.band_spec.clone();
            widgets::spectrum(ui, th, h, &spec, None, (-80.0, 0.0), band_marker)
        }
        _ => {
            // band scope strip + waterfall
            let scope_h = (h * 0.3).clamp(50.0, 130.0);
            let spec = app.session.stores.band_spec.clone();
            let r1 = widgets::spectrum(ui, th, scope_h, &spec, None, (-80.0, 0.0), band_marker);
            let wf = &app.session.stores.wf_band;
            let mut slot = std::mem::take(&mut app.wf_band_tex);
            let r2 = widgets::waterfall(ui, th, wf, &mut slot, h - scope_h - 6.0, band_marker);
            app.wf_band_tex = slot;
            handle_band_drag(app, mode, &r1, running);
            r2
        }
    };
    if viz >= 2 {
        handle_band_drag(app, mode, &resp, running);
    }
}

/// Drag-to-tune + click-to-tune on band displays.
fn handle_band_drag(app: &mut DeckApp, mode: ModeId, resp: &egui::Response, running: bool) {
    let Some(r) = &app.session.running else {
        return;
    };
    if !running || r.rate == 0 {
        return;
    }
    let rate = r.rate as f64;
    let center = r.center_hz as f64;
    let w = resp.rect.width() as f64;
    if resp.dragged() {
        let dhz = -resp.drag_delta().x as f64 / w * rate;
        let cur = app.mode_ui(mode).freq.hz as f64;
        let new = ((cur + dhz) / 100.0).round() * 100.0;
        app.mode_ui(mode).freq.hz = (new.max(0.0) as u64).min(crate::freq::MAX_HZ);
        schedule_retune(app, mode);
    } else if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let rel = ((pos.x - resp.rect.min.x) as f64 / w).clamp(0.0, 1.0);
            let hz = center + (rel - 0.5) * rate;
            let hz = ((hz / 1000.0).round() * 1000.0).max(0.0) as u64;
            app.mode_ui(mode).freq.hz = hz.min(crate::freq::MAX_HZ);
            schedule_retune(app, mode);
        }
    }
}

fn draw_audio_info(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    let mp = app.mode_ui(mode).mp.clone();
    widgets::s_meter(
        ui,
        th,
        app.session.stores.audio_rms,
        mp.squelch,
        app.session.stores.audio_rms > mp.squelch,
    );
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let def = mode_def(mode);
        widgets::chip(ui, th, "MODE", def.label, th.accent2);
        if mp.nr > 0 {
            widgets::chip(
                ui,
                th,
                "NR",
                ["", "low", "med", "high"][(mp.nr as usize).min(3)],
                th.accent,
            );
        }
        if mp.nb > 0 {
            widgets::chip(
                ui,
                th,
                "NB",
                ["", "soft", "hard"][(mp.nb as usize).min(2)],
                th.accent,
            );
        }
        if mp.notch {
            widgets::chip(ui, th, "NOTCH", "auto", th.accent);
        }
        if mp.hp > 0 || mp.lp > 0 {
            widgets::chip(
                ui,
                th,
                "FILTER",
                &format!(
                    "{}–{}",
                    if mp.hp > 0 {
                        format!("{}", mp.hp)
                    } else {
                        "0".into()
                    },
                    if mp.lp > 0 {
                        format!("{:.1}k", mp.lp as f32 / 1000.0)
                    } else {
                        "∞".into()
                    }
                ),
                th.accent2,
            );
        }
        if let Some(r) = &app.session.running {
            if r.mode == mode && !r.audio_capable && mode_def(mode).audio_out {
                widgets::chip(ui, th, "SINK", "missing!", th.err);
            }
        }
        if let Some(p) = &app.session.stores.rec_path {
            let name = p.rsplit('/').next().unwrap_or(p);
            widgets::chip(ui, th, "REC", name, th.rec);
        }
    });
    if app.session.stores.device_busy {
        ui.label(
            RichText::new("!! SDR busy — another program may hold the device")
                .color(th.err)
                .size(12.0),
        );
    }
}

fn draw_voice(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    let def = mode_def(mode);
    // live call card
    panel_frame(th).show(ui, |ui| match &app.session.stores.call {
        Some(c) => {
            ui.horizontal_wrapped(|ui| {
                widgets::chip(ui, th, "PROTO", def.label, th.accent2);
                if let Some(tg) = &c.fields.tg {
                    widgets::chip(ui, th, "TG", tg, th.accent);
                }
                if let Some(src) = &c.fields.src {
                    widgets::chip(ui, th, "SRC", src, th.text);
                }
                if let Some(dst) = &c.fields.dst {
                    widgets::chip(ui, th, "DST", dst, th.text);
                }
                if let Some(slot) = c.fields.slot {
                    widgets::chip(ui, th, "SLOT", &slot.to_string(), th.accent2);
                }
                if let Some(cc) = &c.fields.cc {
                    widgets::chip(ui, th, "CC", cc, th.accent2);
                }
                for (k, v) in &c.fields.extra {
                    widgets::chip(ui, th, k, v, th.text_dim);
                }
                let dur = c.started.elapsed().as_secs();
                widgets::chip(
                    ui,
                    th,
                    "DUR",
                    &format!("{:02}:{:02}", dur / 60, dur % 60),
                    th.ok,
                );
            });
            if let Some(kind) = &c.fields.kind {
                ui.label(RichText::new(kind).color(th.ok).size(13.0));
            }
        }
        None => {
            ui.label(
                RichText::new(if app.running_mode() == Some(mode) {
                    "monitoring — no call"
                } else {
                    "stopped"
                })
                .color(th.text_faint)
                .size(14.0),
            );
        }
    });
    ui.add_space(4.0);
    // history
    let sel = app.mode_ui(mode).list_sel;
    let in_list = app.mode_ui(mode).list_mode;
    egui::ScrollArea::vertical()
        .id_salt("voice-hist")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, rec) in app.session.stores.call_history.iter().enumerate() {
                let f = &rec.fields;
                let line = format!(
                    "{}  {:>5.1}s  TG {}  SRC {}{}",
                    rec.at.format("%H:%M:%S"),
                    rec.dur_s,
                    f.tg.as_deref().unwrap_or("—"),
                    f.src.as_deref().unwrap_or("—"),
                    f.slot.map(|s| format!("  S{s}")).unwrap_or_default(),
                );
                let selected = in_list && i == sel;
                let mut text = RichText::new(line).font(FontId::monospace(12.0));
                text = if selected {
                    text.color(th.sel_fg).background_color(th.sel_bg)
                } else {
                    text.color(th.text_dim)
                };
                ui.label(text);
            }
        });
}

fn draw_pager(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    let sel = app.mode_ui(mode).list_sel;
    let in_list = app.mode_ui(mode).list_mode;
    let mut open_detail: Option<usize> = None;
    egui::ScrollArea::vertical()
        .id_salt("pager")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.session.stores.pagers.is_empty() {
                ui.label(RichText::new("no pages yet").color(th.text_faint));
            }
            for (i, m) in app.session.stores.pagers.iter().enumerate() {
                let (kind, body) = match &m.msg.content {
                    PagerContent::Alpha(t) => ("A", t.clone()),
                    PagerContent::Numeric(t) => ("N", t.clone()),
                    PagerContent::ToneOnly => ("T", "(tone only)".into()),
                };
                let line = format!(
                    "{} {:>7} {} {}",
                    m.at.format("%H:%M:%S"),
                    m.msg.address,
                    kind,
                    body
                );
                let selected = in_list && i == sel;
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
                    open_detail = Some(i);
                }
            }
        });
    if let Some(i) = open_detail {
        let u = app.mode_ui(mode);
        u.list_sel = i;
        u.list_mode = true;
        list_keys(
            app,
            mode,
            ViewKind::Pager,
            false,
            true,
            false,
            false,
            false,
            false,
        );
    }
}

fn draw_aprs(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    let sel = app.mode_ui(mode).list_sel;
    let in_list = app.mode_ui(mode).list_mode;
    let mut open_detail: Option<usize> = None;
    egui::ScrollArea::vertical()
        .id_salt("aprs")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.session.stores.aprs.is_empty() {
                ui.label(RichText::new("no packets yet").color(th.text_faint));
            }
            for (i, m) in app.session.stores.aprs.iter().enumerate() {
                let line = format!(
                    "{} {:>9} → {:<7} {}",
                    m.at.format("%H:%M:%S"),
                    m.msg.from,
                    m.msg.to,
                    m.msg.info
                );
                let selected = in_list && i == sel;
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
                    open_detail = Some(i);
                }
            }
        });
    if let Some(i) = open_detail {
        let u = app.mode_ui(mode);
        u.list_sel = i;
        u.list_mode = true;
        list_keys(
            app,
            mode,
            ViewKind::Aprs,
            false,
            true,
            false,
            false,
            false,
            false,
        );
    }
}

fn draw_adsb(app: &mut DeckApp, ui: &mut egui::Ui, th: &Theme) {
    let now = Instant::now();
    let stores = &app.session.stores;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} aircraft · {} msgs",
                stores.aircraft.len(),
                stores.aircraft.total_msgs
            ))
            .color(th.text_dim)
            .size(12.0),
        );
        if let Some(n) = &stores.sbs_note {
            ui.label(RichText::new(n).color(th.text_faint).size(11.0));
        }
    });
    ui.add_space(2.0);
    let header = format!(
        "{:<6} {:<8} {:>6} {:>4} {:>4} {:>9} {:>10} {:>5} {:>4}",
        "ICAO", "CALL", "ALT", "GS", "TRK", "LAT", "LON", "MSGS", "AGE"
    );
    ui.label(
        RichText::new(header)
            .font(FontId::monospace(12.0))
            .color(th.text_faint),
    );
    egui::ScrollArea::vertical()
        .id_salt("adsb")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (ac, age) in stores.aircraft.rows(now) {
                let line = format!(
                    "{:<6} {:<8} {:>6} {:>4} {:>4} {:>9} {:>10} {:>5} {:>3.0}s",
                    ac.icao,
                    if ac.callsign.is_empty() {
                        "—"
                    } else {
                        &ac.callsign
                    },
                    ac.alt.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                    ac.gs
                        .map(|v| format!("{v:.0}"))
                        .unwrap_or_else(|| "—".into()),
                    ac.trk
                        .map(|v| format!("{v:.0}"))
                        .unwrap_or_else(|| "—".into()),
                    ac.lat
                        .map(|v| format!("{v:.4}"))
                        .unwrap_or_else(|| "—".into()),
                    ac.lon
                        .map(|v| format!("{v:.4}"))
                        .unwrap_or_else(|| "—".into()),
                    ac.msgs,
                    age,
                );
                ui.label(
                    RichText::new(line)
                        .font(FontId::monospace(12.0))
                        .color(super::age_color(th, age)),
                );
            }
        });
}

fn draw_textfeed(app: &mut DeckApp, ui: &mut egui::Ui, th: &Theme) {
    panel_frame(th).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("textfeed")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let text = if app.session.stores.textfeed.is_empty() {
                    "waiting for characters…"
                } else {
                    &app.session.stores.textfeed
                };
                ui.label(
                    RichText::new(text)
                        .font(FontId::monospace(14.0))
                        .color(th.accent),
                );
            });
    });
}

fn draw_scanner(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    let scan = &app.session.scan;
    let cur = scan.channels.get(scan.cur);
    // current channel card
    panel_frame(th).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (phase, color) = match scan.phase {
                ScanPhase::Paused => ("paused", th.text_faint),
                ScanPhase::Sampling => ("scanning", th.accent2),
                ScanPhase::Hold => ("signal!", th.ok),
            };
            ui.label(RichText::new(phase).color(color).strong().size(15.0));
            if let Some(c) = cur {
                ui.label(
                    RichText::new(format!("{}  ·  {}", c.label, crate::freq::fmt_short(c.hz)))
                        .font(FontId::monospace(15.0))
                        .color(th.text),
                );
            }
        });
        widgets::s_meter(
            ui,
            th,
            app.session.stores.audio_rms,
            app.mode_ui.get(&mode).map(|u| u.mp.squelch).unwrap_or(0.05),
            scan.phase == ScanPhase::Hold,
        );
    });
    ui.add_space(4.0);

    let sel = app.mode_ui(mode).list_sel;
    let in_list = app.mode_ui(mode).list_mode;
    let mut jump: Option<usize> = None;
    let mut lock: Option<usize> = None;
    egui::ScrollArea::vertical()
        .id_salt("chans")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, c) in app.session.scan.channels.iter().enumerate() {
                ui.horizontal(|ui| {
                    let active = i == app.session.scan.cur;
                    let selected = in_list && i == sel;
                    let mark = if active { ">" } else { " " };
                    let line = format!(
                        "{mark} {:<14} {:>12}  hits {:>3}",
                        c.label,
                        crate::freq::fmt_short(c.hz),
                        c.hits
                    );
                    let color = if c.locked {
                        th.text_faint
                    } else if active {
                        th.accent
                    } else {
                        th.text
                    };
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(line)
                                .font(FontId::monospace(12.5))
                                .color(if selected { th.sel_fg } else { color })
                                .background_color(if selected {
                                    th.sel_bg
                                } else {
                                    egui::Color32::TRANSPARENT
                                }),
                        )
                        .sense(Sense::click()),
                    );
                    if resp.clicked() {
                        jump = Some(i);
                    }
                    let lock_resp =
                        ui.add(
                            egui::Button::new(
                                RichText::new(if c.locked { "x" } else { "•" })
                                    .color(if c.locked { th.err } else { th.text_faint }),
                            )
                            .small()
                            .frame(false),
                        );
                    if lock_resp.clicked() {
                        lock = Some(i);
                    }
                });
            }
            if !app.session.scan.hits.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("recent hits").size(10.5).color(th.text_faint));
                for h in app.session.scan.hits.iter().take(6) {
                    ui.label(
                        RichText::new(format!("{}  {}", h.at.format("%H:%M:%S"), h.msg))
                            .font(FontId::monospace(11.5))
                            .color(th.text_dim),
                    );
                }
            }
        });
    if let Some(i) = jump {
        app.session.scan.cur = i;
        app.session.scan.phase = ScanPhase::Sampling;
        app.session.scan.phase_since = Instant::now();
        let hz = app.session.scan.channels[i].hz;
        if let Err(e) = app.session.retune(hz) {
            app.session.set_status(e);
        }
    }
    if let Some(i) = lock {
        app.session.toggle_lockout(i);
    }
}

fn draw_preset_popup(app: &mut DeckApp, ctx: &egui::Context, mode: ModeId, th: &Theme) {
    if !app.mode_ui(mode).preset_open {
        return;
    }
    let presets = app.session.presets(mode);
    let sel = app.mode_ui(mode).preset_sel;
    let mut chosen: Option<u64> = None;
    let mut close = false;
    egui::Area::new(egui::Id::new("presets"))
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(th.panel)
                .stroke(Stroke::new(1.0, th.outline))
                .rounding(10.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("PRESETS").size(11.0).color(th.text_faint));
                    for (i, p) in presets.iter().enumerate() {
                        let selected = i == sel;
                        let line = format!("{:<14} {:>12}", p.label, crate::freq::fmt_short(p.hz));
                        let resp = ui.add(
                            egui::Label::new(
                                RichText::new(line)
                                    .font(FontId::monospace(14.0))
                                    .color(if selected { th.sel_fg } else { th.text })
                                    .background_color(if selected {
                                        th.sel_bg
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    }),
                            )
                            .sense(Sense::click()),
                        );
                        if resp.clicked() {
                            chosen = Some(p.hz);
                        }
                    }
                    ui.add_space(6.0);
                    if ui.button("close").clicked() {
                        close = true;
                    }
                });
        });
    if let Some(hz) = chosen {
        let u = app.mode_ui(mode);
        u.freq.hz = hz;
        u.preset_open = false;
        schedule_retune(app, mode);
    }
    if close {
        app.mode_ui(mode).preset_open = false;
    }
}

fn draw_log(app: &mut DeckApp, ctx: &egui::Context, th: &Theme) {
    egui::TopBottomPanel::bottom("rawlog")
        .frame(
            egui::Frame::none()
                .fill(th.panel)
                .inner_margin(8.0)
                .stroke(Stroke::new(1.0, th.outline)),
        )
        .exact_height(ctx.screen_rect().height() * 0.34)
        .show(ctx, |ui| {
            ui.label(RichText::new("RAW LOG").size(10.5).color(th.text_faint));
            egui::ScrollArea::vertical()
                .id_salt("rawlog")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (src, line) in app.session.stores.raw.iter().take(300) {
                        let color = match src {
                            crate::pipeline::LineSrc::Stderr => th.warn.linear_multiply(0.8),
                            crate::pipeline::LineSrc::Sbs => th.accent2.linear_multiply(0.8),
                            _ => th.text_dim,
                        };
                        ui.label(
                            RichText::new(line)
                                .font(FontId::monospace(11.0))
                                .color(color),
                        );
                    }
                });
        });
}
