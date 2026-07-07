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
    /// waterfall hand-off: open the tuned frequency in another mode
    OpenIn,
    /// waterfall display zoom (span)
    Span,
    /// save the tuned frequency as a memory channel
    MemSave,
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
        if mode == ModeId::Waterfall {
            v.push(Ctl::Span);
            v.push(Ctl::OpenIn);
        }
        v.push(Ctl::MemSave);
    }
    if matches!(mode, ModeId::Adsb | ModeId::Ais) {
        v.push(Ctl::Viz);
    }
    v.push(Ctl::Log);
    v
}

/// A row in the preset picker: built-in/config presets + starred memories.
pub struct PresetItem {
    pub label: String,
    pub hz: u64,
    /// Some(index into persist.memories) when this row is a saved memory
    pub mem_idx: Option<usize>,
}

pub fn preset_items(app: &DeckApp, mode: ModeId) -> Vec<PresetItem> {
    let mut v: Vec<PresetItem> = app
        .session
        .presets(mode)
        .into_iter()
        .map(|c| PresetItem {
            label: c.label,
            hz: c.hz,
            mem_idx: None,
        })
        .collect();
    let key = mode_def(mode).key;
    for (i, m) in app.session.persist.memories.iter().enumerate() {
        if m.mode == key {
            v.push(PresetItem {
                label: format!("* {}", m.label),
                hz: m.hz,
                mem_idx: Some(i),
            });
        }
    }
    v
}

/// Modes a waterfall frequency can be handed off to.
pub fn openin_candidates() -> Vec<ModeId> {
    crate::modes::MODES
        .iter()
        .filter(|m| !matches!(m.id, ModeId::Waterfall | ModeId::Scanner | ModeId::Adsb))
        .map(|m| m.id)
        .collect()
}

fn has_list(view: ViewKind) -> bool {
    matches!(
        view,
        ViewKind::Pager
            | ViewKind::Aprs
            | ViewKind::Adsb
            | ViewKind::Ais
            | ViewKind::Scanner
            | ViewKind::Voice
            | ViewKind::Waterfall
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
        Ctl::OpenIn => "OPEN IN",
        Ctl::Span => "SPAN",
        Ctl::MemSave => "MEMORY",
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
        Ctl::Viz => {
            let v = ui.map(|u| u.viz).unwrap_or(0);
            if matches!(mode, ModeId::Adsb | ModeId::Ais) {
                ["table", "radar"][(v as usize) % 2].into()
            } else {
                viz_label(v).into()
            }
        }
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
        Ctl::OpenIn => ui
            .map(|u| format!("{} >", crate::freq::fmt_short(u.freq.hz)))
            .unwrap_or_else(|| ">".into()),
        Ctl::Span => {
            let rate = app
                .session
                .running
                .as_ref()
                .map(|r| r.rate)
                .filter(|r| *r > 0)
                .unwrap_or(2_400_000);
            let span = rate >> ui.map(|u| u.span).unwrap_or(0).min(3);
            crate::freq::fmt_short(u64::from(span))
        }
        Ctl::MemSave => format!("{} saved", app.session.persist.memories.len()),
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
                let n = if matches!(mode, ModeId::Adsb | ModeId::Ais) {
                    2
                } else {
                    4
                };
                ui.viz = ((ui.viz as i32 + dir).rem_euclid(n)) as u8;
                return;
            }
            Ctl::Span => {
                ui.span = ((ui.span as i32 + dir).rem_euclid(4)) as u8;
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
        Ctl::OpenIn => {
            let ui = app.mode_ui(mode);
            ui.openin_open = true;
            ui.openin_sel = 0;
        }
        Ctl::Span => adjust(app, mode, Ctl::Span, 1),
        Ctl::MemSave => {
            let hz = app.mode_ui(mode).freq.hz;
            let key = mode_def(mode).key.to_string();
            let n = app.session.persist.memories.len() + 1;
            let label = format!("M{n} {}", crate::freq::fmt_short(hz));
            app.session.persist.memories.push(crate::config::Memory {
                label: label.clone(),
                hz,
                mode: key,
            });
            app.session.save();
            app.session.set_status(format!("saved {label}"));
        }
        Ctl::Notch | Ctl::Mon | Ctl::Det => adjust(app, mode, c, 1),
        _ => adjust(app, mode, c, 1),
    }
}

/// Stop the current session and re-open `hz` in `target` (auto-starts RX).
fn open_in(app: &mut DeckApp, target: ModeId, hz: u64) {
    app.stop_rx();
    {
        let u = app.mode_ui(target);
        u.freq.hz = hz.min(crate::freq::MAX_HZ);
        u.mp.freq = u.freq.hz;
        u.focus = 0;
    }
    app.goto_mode(target);
    app.start_rx(target);
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

    // open-in popup layer (waterfall hand-off)
    if app.mode_ui(mode).openin_open {
        let candidates = openin_candidates();
        let hz = app.mode_ui(mode).freq.hz;
        let ui = app.mode_ui(mode);
        if up {
            ui.openin_sel = ui.openin_sel.saturating_sub(1);
        }
        if down {
            ui.openin_sel = (ui.openin_sel + 1).min(candidates.len().saturating_sub(1));
        }
        if esc {
            ui.openin_open = false;
        }
        if enter {
            let sel = ui.openin_sel;
            ui.openin_open = false;
            if let Some(target) = candidates.get(sel) {
                open_in(app, *target, hz);
            }
        }
        return;
    }

    // preset popup layer (built-ins + config extras + starred memories)
    if app.mode_ui(mode).preset_open {
        let items = preset_items(app, mode);
        {
            let ui = app.mode_ui(mode);
            if up {
                ui.preset_sel = ui.preset_sel.saturating_sub(1);
            }
            if down {
                ui.preset_sel = (ui.preset_sel + 1).min(items.len().saturating_sub(1));
            }
            if esc {
                ui.preset_open = false;
            }
        }
        let sel = app.mode_ui(mode).preset_sel;
        if enter {
            if let Some(it) = items.get(sel) {
                let hz = it.hz;
                let ui = app.mode_ui(mode);
                ui.freq.hz = hz;
                ui.preset_open = false;
                schedule_retune(app, mode);
            }
        }
        if right {
            // delete a memory entry (d-pad path)
            if let Some(mi) = items.get(sel).and_then(|it| it.mem_idx) {
                app.session.persist.memories.remove(mi);
                app.session.save();
                let u = app.mode_ui(mode);
                u.preset_sel = u.preset_sel.saturating_sub(1);
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
    left: bool,
    right: bool,
) {
    let len = match view {
        ViewKind::Pager => app.session.stores.pagers.len(),
        ViewKind::Aprs => app.session.stores.aprs.len(),
        ViewKind::Adsb | ViewKind::Ais => app.session.stores.aircraft.len(),
        ViewKind::Scanner => app.session.scan.channels.len(),
        ViewKind::Voice => app.session.stores.call_history.len(),
        ViewKind::Waterfall => app.session.stores.peaks.len(),
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
        ViewKind::Waterfall => {
            let hz = app.session.stores.peaks.get(sel).map(|p| p.hz);
            if let Some(hz) = hz {
                if enter {
                    app.mode_ui(mode).freq.hz = hz;
                    schedule_retune(app, mode);
                }
                if right {
                    let u = app.mode_ui(mode);
                    u.freq.hz = hz;
                    u.openin_open = true;
                    u.openin_sel = 0;
                    u.list_mode = false;
                }
                if left {
                    let label = app.session.save_memory(mode, hz);
                    app.session.set_status(format!("saved {label}"));
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
            if left {
                app.session.toggle_priority(sel);
                app.session.save();
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
    } else if mode == ModeId::Waterfall {
        "drag to tune · double-tap a signal (or OPEN IN) to hand off"
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
    draw_openin_popup(app, ctx, mode, &th);
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
    let label = if running { "■  STOP" } else { "START RX" };
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
            ViewKind::Adsb | ViewKind::Ais => app.session.stores.aircraft.len(),
            ViewKind::Scanner => app.session.scan.channels.len(),
            ViewKind::Voice => app.session.stores.call_history.len(),
            ViewKind::Waterfall => app.session.stores.peaks.len(),
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
        ViewKind::Waterfall => (avail_h * 0.62).max(160.0),
        ViewKind::Adsb | ViewKind::Ais => 0.0,
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
        ViewKind::Adsb | ViewKind::Ais => draw_adsb(app, ui, mode, th),
        ViewKind::TextFeed => draw_textfeed(app, ui, th),
        ViewKind::Scanner => draw_scanner(app, ui, mode, th),
        ViewKind::Waterfall => draw_peaks(app, ui, mode, th),
    }
}

/// The waterfall's signal browser: detected peaks, strongest first.
/// OK/tap = tune to it · RIGHT = open in another mode.
fn draw_peaks(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    let sel = app.mode_ui(mode).list_sel;
    let in_list = app.mode_ui(mode).list_mode;
    let tuned = app.mode_ui(mode).freq.hz;
    let mut tune: Option<u64> = None;
    let mut handoff: Option<u64> = None;
    let mut remember: Option<u64> = None;
    ui.label(
        RichText::new("PEAKS · OK tune · RIGHT open in mode · LEFT save")
            .size(10.5)
            .color(th.text_faint),
    );
    egui::ScrollArea::vertical()
        .id_salt("peaks")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.session.stores.peaks.is_empty() {
                ui.label(RichText::new("listening for signals…").color(th.text_faint));
            }
            let peaks: Vec<(u64, f32, f32)> = app
                .session
                .stores
                .peaks
                .iter()
                .map(|p| (p.hz, p.db, p.last.elapsed().as_secs_f32()))
                .collect();
            for (i, (hz, db, age)) in peaks.iter().enumerate() {
                ui.horizontal(|ui| {
                    let selected = in_list && i == sel;
                    let near_tuned = hz.abs_diff(tuned) < 6_000;
                    let mark = if near_tuned { ">" } else { " " };
                    let line =
                        format!("{mark} {:>13}  {:>5.0} dB", crate::freq::fmt_short(*hz), db);
                    let color = if *age < 1.5 { th.text } else { th.text_dim };
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(line)
                                .font(FontId::monospace(14.0))
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
                        tune = Some(*hz);
                    }
                    let open = ui.add(
                        egui::Button::new(RichText::new("open >").size(11.5).color(th.accent2))
                            .small()
                            .frame(false),
                    );
                    if open.clicked() {
                        handoff = Some(*hz);
                    }
                    let mem = ui.add(
                        egui::Button::new(RichText::new("mem+").size(11.5).color(th.accent))
                            .small()
                            .frame(false),
                    );
                    if mem.clicked() {
                        remember = Some(*hz);
                    }
                });
            }
        });
    if let Some(hz) = tune {
        app.mode_ui(mode).freq.hz = hz;
        schedule_retune(app, mode);
    }
    if let Some(hz) = handoff {
        let u = app.mode_ui(mode);
        u.freq.hz = hz;
        u.openin_open = true;
        u.openin_sel = 0;
    }
    if let Some(hz) = remember {
        let label = app.session.save_memory(mode, hz);
        app.session.set_status(format!("saved {label}"));
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

    // visible window of the band (span zoom) + marker position within it
    let win = band_window(app, mode);
    let band_marker = win.map(|w| {
        let freq = app.mode_ui.get(&mode).map(|u| u.freq.hz).unwrap_or(0) as f64;
        (((freq - w.low_hz) / w.span_hz) as f32).clamp(0.0, 1.0)
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
            let r = widgets::waterfall(ui, th, wf, &mut slot, h, None, None);
            app.wf_audio_tex = slot;
            r
        }
        2 => {
            let spec = windowed_spec(app, win);
            let r = widgets::spectrum(ui, th, h, &spec, None, (-80.0, 0.0), band_marker);
            draw_readout(app, ui, mode, th, &r, win);
            r
        }
        _ => {
            // band scope strip + waterfall
            let scope_h = (h * 0.3).clamp(50.0, 130.0);
            let spec = windowed_spec(app, win);
            let r1 = widgets::spectrum(ui, th, scope_h, &spec, None, (-80.0, 0.0), band_marker);
            draw_readout(app, ui, mode, th, &r1, win);
            let wf = &app.session.stores.wf_band;
            let mut slot = std::mem::take(&mut app.wf_band_tex);
            let r2 = widgets::waterfall(
                ui,
                th,
                wf,
                &mut slot,
                h - scope_h - 6.0,
                band_marker,
                win.map(|w| (w.start, w.len)),
            );
            app.wf_band_tex = slot;
            if let Some(w) = win {
                handle_band_drag(app, mode, &r1, w.low_hz, w.span_hz);
            }
            r2
        }
    };
    let _ = running;
    if viz >= 2 {
        if let Some(w) = win {
            handle_band_drag(app, mode, &resp, w.low_hz, w.span_hz);
        }
    }
}

#[derive(Clone, Copy)]
struct BandWindow {
    start: usize,
    len: usize,
    low_hz: f64,
    span_hz: f64,
}

/// Which slice of the band spectrum is visible (span zoom, marker-centered).
fn band_window(app: &DeckApp, mode: ModeId) -> Option<BandWindow> {
    let r = app.session.running.as_ref()?;
    if r.rate == 0 {
        return None;
    }
    let n = app.session.stores.band_spec.len();
    if n < 16 {
        return None;
    }
    let span_pow = app.mode_ui.get(&mode).map(|u| u.span).unwrap_or(0).min(3) as usize;
    let len = (n >> span_pow).max(16);
    let bin_hz = f64::from(r.rate) / n as f64;
    let band_low = r.center_hz as f64 - f64::from(r.rate) / 2.0;
    let freq = app
        .mode_ui
        .get(&mode)
        .map(|u| u.freq.hz)
        .unwrap_or(r.freq_hz) as f64;
    let marker_bin = ((freq - band_low) / bin_hz) as i64;
    let start = (marker_bin - len as i64 / 2).clamp(0, (n - len) as i64) as usize;
    Some(BandWindow {
        start,
        len,
        low_hz: band_low + start as f64 * bin_hz,
        span_hz: len as f64 * bin_hz,
    })
}

fn windowed_spec(app: &DeckApp, win: Option<BandWindow>) -> Vec<f32> {
    let spec = &app.session.stores.band_spec;
    match win {
        Some(w) if w.start + w.len <= spec.len() => spec[w.start..w.start + w.len].to_vec(),
        _ => spec.clone(),
    }
}

/// KC908-style measurement overlay: marker frequency + level, strongest peak.
fn draw_readout(
    app: &DeckApp,
    ui: &egui::Ui,
    mode: ModeId,
    th: &Theme,
    resp: &egui::Response,
    win: Option<BandWindow>,
) {
    let Some(w) = win else { return };
    let spec = &app.session.stores.band_spec;
    if spec.is_empty() {
        return;
    }
    let freq = app.mode_ui.get(&mode).map(|u| u.freq.hz).unwrap_or(0);
    let n = spec.len();
    let band_low = w.low_hz - w.start as f64 * (w.span_hz / w.len as f64);
    let bin = (((freq as f64 - band_low) / (w.span_hz / w.len as f64)) as usize).min(n - 1);
    let level = spec
        .get(bin.saturating_sub(1)..(bin + 2).min(n))
        .map(|s| s.iter().fold(f32::MIN, |a, &b| a.max(b)))
        .unwrap_or(-80.0);
    let p = ui.painter();
    p.text(
        resp.rect.left_top() + egui::vec2(8.0, 6.0),
        Align2::LEFT_TOP,
        format!("MKR {}  {level:>4.0} dB", crate::freq::fmt_short(freq)),
        FontId::monospace(11.5),
        th.warn,
    );
    if let Some(pk) = app.session.stores.peaks.first() {
        p.text(
            resp.rect.right_top() + egui::vec2(-8.0, 6.0),
            Align2::RIGHT_TOP,
            format!("PK {}  {:>4.0} dB", crate::freq::fmt_short(pk.hz), pk.db),
            FontId::monospace(11.5),
            th.accent2,
        );
    }
}

/// Drag-to-tune + click-to-tune on band displays (window/zoom aware).
fn handle_band_drag(
    app: &mut DeckApp,
    mode: ModeId,
    resp: &egui::Response,
    low_hz: f64,
    span_hz: f64,
) {
    if span_hz <= 0.0 {
        return;
    }
    let w = resp.rect.width() as f64;
    if resp.double_clicked() && mode == ModeId::Waterfall {
        let u = app.mode_ui(mode);
        u.openin_open = true;
        u.openin_sel = 0;
    }
    if resp.dragged() {
        let dhz = -resp.drag_delta().x as f64 / w * span_hz;
        let cur = app.mode_ui(mode).freq.hz as f64;
        let new = ((cur + dhz) / 100.0).round() * 100.0;
        app.mode_ui(mode).freq.hz = (new.max(0.0) as u64).min(crate::freq::MAX_HZ);
        schedule_retune(app, mode);
    } else if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let rel = ((pos.x - resp.rect.min.x) as f64 / w).clamp(0.0, 1.0);
            let hz = low_hz + rel * span_hz;
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

fn draw_adsb(app: &mut DeckApp, ui: &mut egui::Ui, mode: ModeId, th: &Theme) {
    if app.mode_ui(mode).viz % 2 == 1 {
        draw_adsb_map(app, ui, th);
        return;
    }
    draw_adsb_table(app, ui, th);
}

/// Offline radar map: range rings around home (or the traffic centroid),
/// aircraft as track-rotated arrows with altitude coloring and trails.
fn draw_adsb_map(app: &mut DeckApp, ui: &mut egui::Ui, th: &Theme) {
    use eframe::egui::{Pos2, Vec2 as EVec2};
    let now = Instant::now();
    let rows = app.session.stores.aircraft.rows(now);
    let (rect, _) = ui.allocate_exact_size(
        EVec2::new(ui.available_width(), ui.available_height().max(120.0)),
        Sense::hover(),
    );
    let p = ui.painter();
    p.rect_filled(rect, 6.0, th.panel);

    // center: configured home, else centroid of known positions
    let cfg = &app.session.cfg.adsb;
    let known: Vec<(f64, f64)> = rows.iter().filter_map(|(a, _)| a.lat.zip(a.lon)).collect();
    let (clat, clon) = if cfg.lat.abs() > 0.01 || cfg.lon.abs() > 0.01 {
        (cfg.lat, cfg.lon)
    } else if !known.is_empty() {
        (
            known.iter().map(|k| k.0).sum::<f64>() / known.len() as f64,
            known.iter().map(|k| k.1).sum::<f64>() / known.len() as f64,
        )
    } else {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "no positions yet",
            FontId::proportional(12.0),
            th.text_faint,
        );
        return;
    };
    let km = |lat: f64, lon: f64| -> (f64, f64) {
        (
            (lon - clon) * 111.32 * clat.to_radians().cos(),
            (lat - clat) * 110.57,
        )
    };
    // scale: furthest aircraft, clamped
    let max_km = known
        .iter()
        .map(|(la, lo)| {
            let (x, y) = km(*la, *lo);
            (x * x + y * y).sqrt()
        })
        .fold(15.0f64, f64::max)
        .clamp(15.0, 500.0);
    let radius = rect.width().min(rect.height()) * 0.47;
    let px_per_km = f64::from(radius) / max_km;
    let center = rect.center();
    let to_px = |lat: f64, lon: f64| -> Pos2 {
        let (x, y) = km(lat, lon);
        Pos2::new(
            center.x + (x * px_per_km) as f32,
            center.y - (y * px_per_km) as f32,
        )
    };

    // world geography (coastlines + borders), clipped to the panel
    for seg in super::world::segments() {
        let mut pts: Vec<Pos2> = Vec::with_capacity(seg.len());
        let mut any_inside = false;
        for (la, lo) in seg {
            let pos = to_px(*la, *lo);
            if rect.expand(60.0).contains(pos) {
                any_inside = true;
            }
            pts.push(pos);
        }
        if any_inside {
            p.add(egui::Shape::line(
                pts,
                Stroke::new(1.0, th.grid.linear_multiply(1.6)),
            ));
        }
    }

    // range rings at nice steps
    let step = [10.0, 25.0, 50.0, 100.0, 200.0]
        .into_iter()
        .find(|s| max_km / s <= 3.2)
        .unwrap_or(200.0);
    let mut r = step;
    while r <= max_km * 1.02 {
        p.circle_stroke(center, (r * px_per_km) as f32, Stroke::new(1.0, th.grid));
        p.text(
            center + EVec2::new(2.0, -(r * px_per_km) as f32),
            Align2::LEFT_BOTTOM,
            format!("{r:.0} km"),
            FontId::proportional(9.5),
            th.text_faint,
        );
        r += step;
    }
    p.circle_filled(center, 2.5, th.warn); // home
    p.text(
        Pos2::new(center.x, rect.min.y + 4.0),
        Align2::CENTER_TOP,
        "N",
        FontId::proportional(11.0),
        th.text_dim,
    );

    for (ac, age) in &rows {
        let (Some(la), Some(lo)) = (ac.lat, ac.lon) else {
            continue;
        };
        let pos = to_px(la, lo);
        if !rect.expand(-2.0).contains(pos) {
            continue;
        }
        let color = match ac.alt.unwrap_or(0) {
            a if a < 10_000 => th.warn,
            a if a < 25_000 => th.accent,
            _ => th.accent2,
        };
        let color = if *age > 20.0 {
            color.linear_multiply(0.4)
        } else {
            color
        };
        // trail
        if ac.trail.len() > 1 {
            let pts: Vec<Pos2> = ac.trail.iter().map(|(a, b)| to_px(*a, *b)).collect();
            p.add(egui::Shape::line(
                pts,
                Stroke::new(1.0, color.linear_multiply(0.35)),
            ));
        }
        // track-rotated arrow
        let ang = ac.trk.unwrap_or(0.0).to_radians() - std::f32::consts::FRAC_PI_2;
        let dir = EVec2::angled(ang);
        let side = EVec2::angled(ang + std::f32::consts::FRAC_PI_2);
        p.add(egui::Shape::convex_polygon(
            vec![
                pos + dir * 7.0,
                pos - dir * 4.0 + side * 4.0,
                pos - dir * 4.0 - side * 4.0,
            ],
            color,
            Stroke::NONE,
        ));
        let label = if ac.callsign.is_empty() {
            ac.icao.clone()
        } else {
            ac.callsign.clone()
        };
        p.text(
            pos + EVec2::new(8.0, -4.0),
            Align2::LEFT_CENTER,
            format!(
                "{label} {}",
                ac.alt.map(|a| format!("{}", a / 100)).unwrap_or_default()
            ),
            FontId::monospace(10.5),
            th.text_dim,
        );
    }
    p.text(
        rect.left_bottom() + EVec2::new(8.0, -6.0),
        Align2::LEFT_BOTTOM,
        format!("{} aircraft · alt color: <100 <250 FL+", rows.len()),
        FontId::proportional(10.0),
        th.text_faint,
    );
}

fn draw_adsb_table(app: &mut DeckApp, ui: &mut egui::Ui, th: &Theme) {
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
    let mut prio: Option<usize> = None;
    egui::ScrollArea::vertical()
        .id_salt("chans")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, c) in app.session.scan.channels.iter().enumerate() {
                ui.horizontal(|ui| {
                    let active = i == app.session.scan.cur;
                    let selected = in_list && i == sel;
                    let mark = if active { ">" } else { " " };
                    let pri = if c.priority { "P" } else { " " };
                    let line = format!(
                        "{mark}{pri} {:<14} {:>12}  hits {:>3}",
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
                    let pri_resp = ui.add(
                        egui::Button::new(RichText::new("P").size(11.0).color(if c.priority {
                            th.accent2
                        } else {
                            th.text_faint
                        }))
                        .small()
                        .frame(false),
                    );
                    if pri_resp.clicked() {
                        prio = Some(i);
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
    if let Some(i) = prio {
        app.session.toggle_priority(i);
        app.session.save();
    }
}

fn draw_preset_popup(app: &mut DeckApp, ctx: &egui::Context, mode: ModeId, th: &Theme) {
    if !app.mode_ui(mode).preset_open {
        return;
    }
    let items = preset_items(app, mode);
    let sel = app.mode_ui(mode).preset_sel;
    let mut chosen: Option<u64> = None;
    let mut delete: Option<usize> = None;
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
                    ui.label(
                        RichText::new("PRESETS · * = memory (RIGHT/x deletes)")
                            .size(11.0)
                            .color(th.text_faint),
                    );
                    for (i, it) in items.iter().enumerate() {
                        let selected = i == sel;
                        let line =
                            format!("{:<16} {:>12}", it.label, crate::freq::fmt_short(it.hz));
                        ui.horizontal(|ui| {
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
                                chosen = Some(it.hz);
                            }
                            if let Some(mi) = it.mem_idx {
                                let del = ui.add(
                                    egui::Button::new(RichText::new("x").size(12.0).color(th.err))
                                        .small()
                                        .frame(false),
                                );
                                if del.clicked() {
                                    delete = Some(mi);
                                }
                            }
                        });
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
    if let Some(mi) = delete {
        if mi < app.session.persist.memories.len() {
            app.session.persist.memories.remove(mi);
            app.session.save();
        }
    }
    if close {
        app.mode_ui(mode).preset_open = false;
    }
}

fn draw_openin_popup(app: &mut DeckApp, ctx: &egui::Context, mode: ModeId, th: &Theme) {
    if !app.mode_ui(mode).openin_open {
        return;
    }
    let hz = app.mode_ui(mode).freq.hz;
    let candidates = openin_candidates();
    let sel = app.mode_ui(mode).openin_sel;
    let mut chosen: Option<ModeId> = None;
    let mut close = false;
    egui::Area::new(egui::Id::new("openin"))
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(th.panel)
                .stroke(Stroke::new(1.0, th.outline))
                .rounding(10.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("OPEN {} IN", crate::freq::fmt_short(hz)))
                            .size(11.0)
                            .color(th.text_faint),
                    );
                    for (i, id) in candidates.iter().enumerate() {
                        let def = mode_def(*id);
                        let selected = i == sel;
                        let blocked = app.support.get(id).cloned().flatten().is_some();
                        let line = format!("{:<8} {}", def.label, def.desc);
                        let color = if selected {
                            th.sel_fg
                        } else if blocked {
                            th.text_faint
                        } else {
                            th.text
                        };
                        let resp = ui.add(
                            egui::Label::new(
                                RichText::new(line)
                                    .font(FontId::monospace(13.5))
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
                            chosen = Some(*id);
                        }
                    }
                    ui.add_space(6.0);
                    if ui.button("close").clicked() {
                        close = true;
                    }
                });
        });
    if let Some(target) = chosen {
        app.mode_ui(mode).openin_open = false;
        open_in(app, target, hz);
    }
    if close {
        app.mode_ui(mode).openin_open = false;
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
