//! The deck GUI: fullscreen egui app. Everything drives through `Session`.
//!
//! Input contract: the whole UI is operable with arrows + Enter/Esc
//! (Flipper-style d-pad). Touch and letter keys are accelerators.

pub mod icons;
pub mod misc;
pub mod modeview;
pub mod theme;
pub mod widgets;

pub mod raster;
pub mod shot;

use crate::config::ModePersist;
use crate::freq::FreqInput;
use crate::modes::{mode_def, ModeId, Section, MODES};
use crate::session::Session;
use crate::sys::SysMon;
use eframe::egui::{self, Align2, Color32, FontId, Key, Sense, Stroke, Vec2};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use theme::Theme;
use widgets::WfTex;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Menu,
    Mode(ModeId),
    Devices,
    Doctor,
}

/// Per-mode UI state (freq editor, focus, pending retunes).
pub struct ModeUi {
    pub freq: FreqInput,
    pub drag_acc: f32,
    pub mp: ModePersist,
    pub focus: usize,
    pub editing_freq: bool,
    pub list_mode: bool,
    pub list_sel: usize,
    pub viz: u8,
    pub retune_at: Option<Instant>,
    pub gain_restart_at: Option<Instant>,
    pub show_log: bool,
    pub preset_open: bool,
    pub preset_sel: usize,
    /// "open this frequency in another mode" picker (waterfall hand-off)
    pub openin_open: bool,
    pub openin_sel: usize,
    pub ctl_drag: f32,
}

impl ModeUi {
    fn new(mp: ModePersist) -> Self {
        Self {
            freq: FreqInput::new(mp.freq),
            drag_acc: 0.0,
            mp,
            focus: 0,
            editing_freq: false,
            list_mode: false,
            list_sel: 0,
            viz: 0,
            retune_at: None,
            gain_restart_at: None,
            show_log: false,
            preset_open: false,
            preset_sel: 0,
            openin_open: false,
            openin_sel: 0,
            ctl_drag: 0.0,
        }
    }
}

/// A menu entry: a mode tile or a tool tile.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Mode(ModeId),
    Devices,
    Doctor,
    Power,
}

pub fn tiles() -> Vec<(Section, Tile)> {
    let mut v: Vec<(Section, Tile)> = MODES
        .iter()
        .map(|m| (m.section, Tile::Mode(m.id)))
        .collect();
    v.push((Section::Tools, Tile::Devices));
    v.push((Section::Tools, Tile::Doctor));
    v.push((Section::Tools, Tile::Power));
    v
}

pub struct DeckApp {
    pub session: Session,
    pub sys: SysMon,
    pub dark: bool,
    pub th: Theme,
    theme_applied: bool,
    pub screen: Screen,
    pub splash_since: Option<Instant>,
    pub power_sel: Option<usize>,
    pub power_err: Option<String>,
    pub menu_focus: usize,
    pub devices_focus: usize,
    pub mode_ui: HashMap<ModeId, ModeUi>,
    pub wf_audio_tex: WfTex,
    pub wf_band_tex: WfTex,
    pub detail: Option<String>,
    pub doctor: misc::DoctorState,
    pub support: HashMap<ModeId, Option<String>>, // None = ok, Some(reason) = blocked
    support_dev: usize,
    /// suppress input handling for the frame after a screen switch
    nav_cooldown: u8,
    /// last-frame menu grid width, so d-pad up/down knows the geometry
    menu_cols: usize,
}

impl DeckApp {
    pub fn new(session: Session, splash: bool) -> Self {
        let dark = session
            .persist
            .theme
            .as_deref()
            .map(|t| t != "light")
            .unwrap_or_else(|| session.cfg.ui.theme != "light");
        let th = if dark { Theme::dark() } else { Theme::light() };
        let mut app = Self {
            sys: SysMon::new(),
            dark,
            th,
            theme_applied: false,
            screen: Screen::Menu,
            splash_since: splash.then(Instant::now),
            power_sel: None,
            power_err: None,
            menu_focus: 0,
            devices_focus: 0,
            mode_ui: HashMap::new(),
            wf_audio_tex: WfTex::default(),
            wf_band_tex: WfTex::default(),
            detail: None,
            doctor: misc::DoctorState::default(),
            support: HashMap::new(),
            support_dev: usize::MAX,
            nav_cooldown: 0,
            menu_cols: 4,
            session,
        };
        app.refresh_support();
        if let Some(e) = app.session.cfg_error.take() {
            app.session.set_status(e);
        }
        app
    }

    pub fn mode_ui(&mut self, mode: ModeId) -> &mut ModeUi {
        let mp = self.session.mode_persist(mode);
        self.mode_ui.entry(mode).or_insert_with(|| ModeUi::new(mp))
    }

    pub fn refresh_support(&mut self) {
        self.support.clear();
        let dev = self.session.device().clone();
        for m in MODES {
            let r = crate::pipeline::resolve(
                m.id,
                &dev,
                &self.session.cfg,
                &self.session.tools,
                m.default_hz,
                self.session.cfg.sdr.gain,
            );
            let verdict = match r {
                None => Some(format!("not available on {}", dev.kind.label())),
                Some(res) if !res.missing.is_empty() => {
                    Some(format!("install {}", res.missing.join(", ")))
                }
                Some(_) => None,
            };
            self.support.insert(m.id, verdict);
        }
        self.support_dev = self.session.active_dev;
    }

    pub fn set_theme(&mut self, dark: bool) {
        self.dark = dark;
        self.th = if dark { Theme::dark() } else { Theme::light() };
        self.theme_applied = false;
        self.session.persist.theme = Some(if dark { "dark" } else { "light" }.into());
    }

    /// Save the current mode's live knob state into persistence.
    pub fn snapshot_mode(&mut self, mode: ModeId) {
        if let Some(ui) = self.mode_ui.get(&mode) {
            let mut mp = ui.mp.clone();
            mp.freq = ui.freq.hz;
            let mp = self.session.knobs_snapshot(&mp);
            self.session.save_mode_persist(mode, mp.clone());
            if let Some(ui) = self.mode_ui.get_mut(&mode) {
                ui.mp = mp;
            }
        }
    }

    pub fn start_rx(&mut self, mode: ModeId) {
        let mp = {
            let ui = self.mode_ui(mode);
            let mut mp = ui.mp.clone();
            mp.freq = ui.freq.hz;
            mp
        };
        match self.session.start(mode, &mp) {
            Ok(()) => {
                self.session.save_mode_persist(mode, mp);
            }
            Err(e) => self.session.set_status(e),
        }
    }

    pub fn stop_rx(&mut self) {
        if let Some(mode) = self.session.running.as_ref().map(|r| r.mode) {
            self.snapshot_mode(mode);
        }
        self.session.stop();
    }

    pub fn running_mode(&self) -> Option<ModeId> {
        self.session.running.as_ref().map(|r| r.mode)
    }

    /// Switch to a mode screen (seeding its UI state) with input cooldown.
    pub fn goto_mode(&mut self, m: ModeId) {
        let _ = self.mode_ui(m);
        self.screen = Screen::Mode(m);
        self.nav_cooldown = 1;
    }
}

impl eframe::App for DeckApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

impl DeckApp {
    /// One full UI frame. Separated from `eframe::App::update` so the
    /// headless screenshot rasterizer can drive the exact same path.
    pub fn frame(&mut self, ctx: &egui::Context) {
        if !self.theme_applied {
            self.th.apply(ctx);
            self.theme_applied = true;
        }
        self.session.tick();
        self.sys.refresh(false);
        if self.support_dev != self.session.active_dev {
            self.refresh_support();
        }

        // deferred work: debounced retune / gain restarts
        let now = Instant::now();
        if let Some(mode) = self.running_mode() {
            let (due_retune, due_gain, hz) = {
                let ui = self.mode_ui(mode);
                (
                    ui.retune_at.map(|t| now >= t).unwrap_or(false),
                    ui.gain_restart_at.map(|t| now >= t).unwrap_or(false),
                    ui.freq.hz,
                )
            };
            if due_retune {
                self.mode_ui(mode).retune_at = None;
                if let Err(e) = self.session.retune(hz) {
                    self.session.set_status(e);
                }
            }
            if due_gain {
                self.mode_ui(mode).gain_restart_at = None;
                self.stop_rx();
                self.start_rx(mode);
                self.session.set_status("restarted with new gain");
            }
        }

        // splash
        if let Some(since) = self.splash_since {
            let done = since.elapsed() > Duration::from_millis(1600)
                || ctx.input(|i| {
                    i.pointer.any_pressed()
                        || i.events
                            .iter()
                            .any(|e| matches!(e, egui::Event::Key { .. }))
                });
            if done {
                self.splash_since = None;
            } else {
                misc::draw_splash(self, ctx, since);
                ctx.request_repaint_after(Duration::from_millis(33));
                return;
            }
        }

        if self.nav_cooldown > 0 {
            self.nav_cooldown -= 1;
        } else {
            self.handle_keys(ctx);
        }

        match self.screen {
            Screen::Menu => self.draw_menu(ctx),
            Screen::Mode(m) => modeview::draw(self, ctx, m),
            Screen::Devices => misc::draw_devices(self, ctx),
            Screen::Doctor => misc::draw_doctor(self, ctx),
        }
        misc::draw_power_popup(self, ctx);
        self.draw_detail_popup(ctx);

        // repaint cadence: fast while running, relaxed otherwise (clock)
        if self.session.running.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl DeckApp {
    fn handle_keys(&mut self, ctx: &egui::Context) {
        let (esc, enter, up, down, left, right) = ctx.input(|i| {
            (
                i.key_pressed(Key::Escape) || i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
                i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::K),
                i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::J),
                i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::H),
                i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::L),
            )
        });
        if ctx.input(|i| i.key_pressed(Key::T)) {
            let dark = !self.dark;
            self.set_theme(dark);
        }
        if ctx.input(|i| i.key_pressed(Key::F11)) {
            let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
        }
        if ctx.input(|i| i.key_pressed(Key::Comma)) {
            self.sys.volume_step(-1);
        }
        if ctx.input(|i| i.key_pressed(Key::Period)) {
            self.sys.volume_step(1);
        }
        if ctx.input(|i| i.key_pressed(Key::M)) {
            self.sys.toggle_mute();
        }

        // modal layers eat navigation first
        if self.power_sel.is_some() {
            misc::power_keys(self, esc, enter, up, down);
            return;
        }
        if self.detail.is_some() {
            if esc || enter {
                self.detail = None;
            }
            return;
        }

        match self.screen {
            Screen::Menu => self.menu_keys(esc, enter, up, down, left, right),
            Screen::Mode(m) => modeview::keys(self, ctx, m, esc, enter, up, down, left, right),
            Screen::Devices => misc::devices_keys(self, esc, enter, up, down),
            Screen::Doctor => {
                if esc {
                    self.screen = Screen::Menu;
                    self.nav_cooldown = 1;
                }
            }
        }
    }

    pub fn open_tile(&mut self, t: Tile) {
        match t {
            Tile::Mode(m) => {
                if let Some(reason) = self.support.get(&m).cloned().flatten() {
                    self.session.set_status(reason);
                } else {
                    let _ = self.mode_ui(m); // seed UI state
                    self.screen = Screen::Mode(m);
                    self.nav_cooldown = 1;
                }
            }
            Tile::Devices => {
                self.devices_focus = self.session.active_dev;
                self.screen = Screen::Devices;
                self.nav_cooldown = 1;
            }
            Tile::Doctor => {
                self.doctor.ensure_report(&self.session.cfg);
                self.screen = Screen::Doctor;
                self.nav_cooldown = 1;
            }
            Tile::Power => self.power_sel = Some(0),
        }
    }

    fn menu_keys(&mut self, esc: bool, enter: bool, up: bool, down: bool, left: bool, right: bool) {
        let n = tiles().len();
        let cols = self.menu_cols;
        if left && self.menu_focus > 0 {
            self.menu_focus -= 1;
        }
        if right && self.menu_focus + 1 < n {
            self.menu_focus += 1;
        }
        if up {
            self.menu_focus = self.menu_focus.saturating_sub(cols);
        }
        if down {
            self.menu_focus = (self.menu_focus + cols).min(n - 1);
        }
        if enter {
            let t = tiles()[self.menu_focus].1;
            self.open_tile(t);
        }
        if esc {
            self.power_sel = Some(0);
        }
    }

    fn draw_detail_popup(&mut self, ctx: &egui::Context) {
        let Some(text) = self.detail.clone() else {
            return;
        };
        let th = self.th.clone();
        egui::Area::new(egui::Id::new("detail"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(th.panel)
                    .stroke(Stroke::new(1.0, th.outline))
                    .rounding(10.0)
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        ui.set_max_width(ctx.screen_rect().width() * 0.85);
                        ui.label(
                            egui::RichText::new(text)
                                .font(FontId::monospace(13.0))
                                .color(th.text),
                        );
                        ui.add_space(8.0);
                        if ui.button("close  [OK]").clicked() {
                            self.detail = None;
                        }
                    });
            });
    }
}

// --------------------------------------------------------------- chrome

impl DeckApp {
    pub fn status_bar(&mut self, ctx: &egui::Context, title: &str, show_back: bool) {
        let th = self.th.clone();
        egui::TopBottomPanel::top("status")
            .frame(egui::Frame::none().fill(th.bg).inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 6.0,
                bottom: 4.0,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if show_back {
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(36.0, 26.0), Sense::click());
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "‹",
                            FontId::proportional(26.0),
                            th.accent,
                        );
                        if resp.clicked() {
                            self.screen = Screen::Menu;
                        }
                    }
                    ui.label(
                        egui::RichText::new(title)
                            .font(FontId::proportional(17.0))
                            .strong()
                            .color(th.text),
                    );
                    if let Some(mode) = self.running_mode() {
                        if self.screen != Screen::Mode(mode) {
                            let label = format!("• {}", mode_def(mode).label);
                            let resp = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(label).color(th.ok).size(13.0),
                                )
                                .sense(Sense::click()),
                            );
                            if resp.clicked() {
                                self.screen = Screen::Mode(mode);
                            }
                        }
                    }
                    if self.session.stores.rec_path.is_some() {
                        ui.label(egui::RichText::new("• REC").color(th.rec).size(13.0));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let clock = chrono::Local::now().format("%H:%M").to_string();
                        ui.label(
                            egui::RichText::new(clock)
                                .font(FontId::monospace(14.0))
                                .color(th.text_dim),
                        );
                        if let Some(b) = self.sys.battery {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(30.0, 16.0), Sense::hover());
                            icons::battery(
                                ui.painter(),
                                rect,
                                f32::from(b.percent) / 100.0,
                                matches!(b.state, crate::sys::BatState::Charging),
                                &th,
                            );
                            ui.label(
                                egui::RichText::new(format!("{}%", b.percent))
                                    .size(12.0)
                                    .color(th.text_dim),
                            );
                        }
                        if let Some(v) = self.sys.volume {
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(20.0, 16.0), Sense::click());
                            let pct = if self.sys.muted { None } else { Some(v) };
                            icons::speaker(ui.painter(), rect, pct, &th);
                            if resp.clicked() {
                                self.sys.toggle_mute();
                            }
                            if !self.sys.muted {
                                ui.label(
                                    egui::RichText::new(format!("{v}%"))
                                        .size(12.0)
                                        .color(th.text_dim),
                                );
                            }
                        }
                    });
                });
            });
    }

    pub fn hint_bar(&mut self, ctx: &egui::Context, hint: &str) {
        let th = self.th.clone();
        egui::TopBottomPanel::bottom("hints")
            .frame(egui::Frame::none().fill(th.bg).inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 3.0,
                bottom: 5.0,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match &self.session.status {
                        Some((msg, _)) => {
                            ui.label(egui::RichText::new(msg).color(th.warn).size(12.0));
                        }
                        None => {
                            ui.label(egui::RichText::new(hint).color(th.text_faint).size(11.0));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dev = self.session.device();
                        ui.label(
                            egui::RichText::new(format!("SDR: {}", dev.kind.label()))
                                .size(11.0)
                                .color(th.text_dim),
                        );
                    });
                });
            });
    }
}

// ------------------------------------------------------------- tile grid

impl DeckApp {
    pub fn draw_menu(&mut self, ctx: &egui::Context) {
        self.status_bar(ctx, "deck", false);
        // the hint line doubles as the focused mode's description
        let hint = match tiles().get(self.menu_focus).map(|(_, t)| *t) {
            Some(Tile::Mode(m)) => mode_def(m).desc.to_string(),
            Some(Tile::Devices) => "select / rescan SDR hardware".to_string(),
            Some(Tile::Doctor) => "environment report & selftest".to_string(),
            Some(Tile::Power) => "suspend · reboot · power off".to_string(),
            None => "arrows move · OK open · BACK power · T theme".to_string(),
        };
        self.hint_bar(ctx, &hint);
        let th = self.th.clone();
        let all = tiles();
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(th.bg).inner_margin(10.0))
            .show(ctx, |ui| {
                let w = ui.available_width();
                let cols = ((w / 132.0) as usize).clamp(2, 8);
                self.menu_cols = cols;
                let tile_w = (w - (cols as f32 - 1.0) * 8.0) / cols as f32;
                let tile_h = (tile_w * 0.80).clamp(92.0, 128.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let mut idx = 0usize;
                        let mut section: Option<Section> = None;
                        while idx < all.len() {
                            let sec = all[idx].0;
                            if section != Some(sec) {
                                section = Some(sec);
                                ui.add_space(if idx == 0 { 0.0 } else { 6.0 });
                                ui.label(
                                    egui::RichText::new(sec.label())
                                        .size(10.5)
                                        .color(th.text_faint)
                                        .strong(),
                                );
                            }
                            ui.horizontal(|ui| {
                                let mut in_row = 0;
                                while idx < all.len() && all[idx].0 == sec && in_row < cols {
                                    let (_, t) = all[idx];
                                    let focused = self.menu_focus == idx;
                                    let resp = self.draw_tile(
                                        ui,
                                        &th,
                                        t,
                                        Vec2::new(tile_w, tile_h),
                                        focused,
                                    );
                                    if resp.clicked() {
                                        self.menu_focus = idx;
                                        self.open_tile(t);
                                    }
                                    idx += 1;
                                    in_row += 1;
                                }
                            });
                        }
                    });
            });
    }

    fn draw_tile(
        &mut self,
        ui: &mut egui::Ui,
        th: &Theme,
        t: Tile,
        size: Vec2,
        focused: bool,
    ) -> egui::Response {
        let (key, label, sub, enabled) = match t {
            Tile::Mode(m) => {
                let def = mode_def(m);
                let mp = self.session.mode_persist(m);
                let hz = if mp.freq == 0 {
                    def.default_hz
                } else {
                    mp.freq
                };
                let blocked = self.support.get(&m).cloned().flatten();
                let running = self.running_mode() == Some(m);
                let sub = if running {
                    "• RX".to_string()
                } else if blocked.is_some() {
                    "needs tools".to_string()
                } else {
                    crate::freq::fmt_short(hz)
                };
                (def.key, def.label, sub, blocked.is_none())
            }
            Tile::Devices => (
                "devices",
                "DEVICES",
                format!("{} found", self.session.devices.len()),
                true,
            ),
            Tile::Doctor => ("doctor", "DOCTOR", "env check".to_string(), true),
            Tile::Power => ("power", "POWER", "off · reboot".to_string(), true),
        };
        let resp = widgets::tile(ui, th, size, key, label, &sub, focused, enabled);
        if focused {
            resp.scroll_to_me(Some(egui::Align::Center));
        }
        resp
    }
}

// -- small helpers shared by screens --------------------------------------

pub fn panel_frame(th: &Theme) -> egui::Frame {
    egui::Frame::none()
        .fill(th.panel)
        .rounding(8.0)
        .inner_margin(10.0)
}

pub fn age_color(th: &Theme, age_s: f32) -> Color32 {
    if age_s < 10.0 {
        th.text
    } else if age_s < 30.0 {
        th.text_dim
    } else {
        th.text_faint
    }
}
