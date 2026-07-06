//! Custom widgets: flat control rows, the digit-wheel frequency tuner,
//! spectrum, waterfall, S-meter, menu tiles. All touch + d-pad friendly,
//! sized for fingers (44 px+ targets).

use super::theme::Theme;
use crate::freq::FreqInput;
use crate::session::WaterfallBuf;
use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Pos2, Rect, Response, Sense, Stroke, TextureHandle,
    TextureOptions, Ui, Vec2,
};

pub fn focus_ring(ui: &Ui, rect: Rect, th: &Theme) {
    ui.painter()
        .rect_stroke(rect.expand(1.5), 6.0, Stroke::new(2.0, th.accent));
}

/// A flat, tappable control row: `label ......... value`.
pub struct ControlRow<'a> {
    pub label: &'a str,
    pub value: String,
    pub active: bool,
    pub focused: bool,
    pub enabled: bool,
}

pub fn control_row(ui: &mut Ui, th: &Theme, c: ControlRow) -> Response {
    let h = 44.0f32.max(ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), h),
        if c.enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let p = ui.painter();
    let bg = if c.active {
        th.sel_bg
    } else if resp.hovered() && c.enabled {
        th.panel_hi
    } else {
        th.panel
    };
    p.rect_filled(rect, 6.0, bg);
    if c.focused {
        focus_ring(ui, rect, th);
    }
    let (fg, val_fg) = if c.active {
        (th.sel_fg, th.sel_fg)
    } else if c.enabled {
        (th.text, th.accent)
    } else {
        (th.text_faint, th.text_faint)
    };
    p.text(
        Pos2::new(rect.min.x + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        c.label,
        FontId::proportional(15.0),
        fg,
    );
    p.text(
        Pos2::new(rect.max.x - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        c.value,
        FontId::monospace(15.0),
        val_fg,
    );
    resp
}

/// Big primary action button (RX start/stop).
pub fn action_button(ui: &mut Ui, th: &Theme, label: &str, hot: bool, focused: bool) -> Response {
    let h = 56.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    let p = ui.painter();
    let bg = if hot { th.rec } else { th.sel_bg };
    let fg = if hot { Color32::WHITE } else { th.sel_fg };
    p.rect_filled(rect, 8.0, bg);
    if focused {
        focus_ring(ui, rect, th);
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(19.0),
        fg,
    );
    resp
}

// ------------------------------------------------------------- freq tuner

pub struct TunerOut {
    pub changed: bool,
    pub tapped: bool,
}

/// Per-digit frequency tuner: tap a digit to select, drag it vertically or
/// scroll to spin it (slot-machine style), d-pad handled by the caller.
#[allow(clippy::needless_range_loop, clippy::too_many_arguments)]
pub fn freq_tuner(
    ui: &mut Ui,
    th: &Theme,
    f: &mut FreqInput,
    editing: bool,
    focused: bool,
    drag_acc: &mut f32,
    compact: bool,
) -> TunerOut {
    let digits = f.digits();
    let dsize = if compact { 30.0 } else { 40.0 };
    let dw = dsize * 0.62;
    let sep_w = dw * 0.35;
    let total_w = 10.0 * dw + 3.0 * sep_w + dsize * 1.2;
    let h = dsize * 1.5;
    let (outer, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(total_w), h),
        Sense::hover(),
    );
    let mut out = TunerOut {
        changed: false,
        tapped: false,
    };
    let p = ui.painter().clone();
    let x0 = outer.center().x - total_w / 2.0;
    let first_sig = digits.iter().position(|d| *d != 0).unwrap_or(9);
    let mut x = x0;
    for i in 0..10usize {
        if i == 1 || i == 4 || i == 7 {
            let sep_c = if i > first_sig || f.cursor as usize >= 10 - i {
                th.text_dim
            } else {
                th.text_faint
            };
            p.text(
                Pos2::new(x + sep_w / 2.0, outer.center().y + dsize * 0.32),
                Align2::CENTER_CENTER,
                ".",
                FontId::monospace(dsize),
                sep_c,
            );
            x += sep_w;
        }
        let rect = Rect::from_min_size(Pos2::new(x, outer.min.y), Vec2::new(dw, h));
        let cursor_here = (9 - f.cursor) as usize == i;
        let id = ui.id().with(("digit", i));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() {
            f.cursor = (9 - i) as u32;
            out.tapped = true;
        }
        if resp.dragged() && cursor_here {
            *drag_acc += resp.drag_delta().y;
            while *drag_acc <= -12.0 {
                f.up();
                *drag_acc += 12.0;
                out.changed = true;
            }
            while *drag_acc >= 12.0 {
                f.down();
                *drag_acc -= 12.0;
                out.changed = true;
            }
        }
        if resp.hovered() {
            let scroll = ui.input(|inp| inp.raw_scroll_delta.y);
            if scroll > 0.5 {
                f.cursor = (9 - i) as u32;
                f.up();
                out.changed = true;
            } else if scroll < -0.5 {
                f.cursor = (9 - i) as u32;
                f.down();
                out.changed = true;
            }
        }
        let significant = i >= first_sig || digits[i] != 0 || cursor_here;
        let color = if cursor_here && (editing || focused) {
            th.sel_fg
        } else if significant {
            th.text
        } else {
            th.text_faint
        };
        if cursor_here && (editing || focused) {
            p.rect_filled(
                rect.shrink2(Vec2::new(0.5, dsize * 0.12)),
                4.0,
                if editing {
                    th.accent
                } else {
                    th.accent.linear_multiply(0.55)
                },
            );
        }
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            digits[i].to_string(),
            FontId::monospace(dsize),
            color,
        );
        x += dw;
    }
    p.text(
        Pos2::new(x + 6.0, outer.center().y + dsize * 0.25),
        Align2::LEFT_CENTER,
        "Hz",
        FontId::proportional(dsize * 0.4),
        th.text_dim,
    );
    out
}

// ------------------------------------------------------------- spectrum

pub fn spectrum(
    ui: &mut Ui,
    th: &Theme,
    height: f32,
    spec: &[f32],
    peak: Option<&[f32]>,
    range: (f32, f32),
    marker: Option<f32>,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click_and_drag(),
    );
    let p = ui.painter();
    p.rect_filled(rect, 6.0, th.panel);
    if spec.is_empty() {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "no signal",
            FontId::proportional(12.0),
            th.text_faint,
        );
        return resp;
    }
    let (lo, hi) = range;
    let mut db = lo.ceil();
    while db < hi {
        if (db as i32) % 20 == 0 {
            let y = rect.max.y - (db - lo) / (hi - lo) * rect.height();
            p.line_segment(
                [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
                Stroke::new(1.0, th.grid),
            );
        }
        db += 10.0;
    }
    let n = spec.len();
    let step = (n as f32 / rect.width()).max(1.0);
    let mut pts: Vec<Pos2> = Vec::with_capacity(rect.width() as usize + 2);
    let mut x = rect.min.x;
    while x < rect.max.x {
        let i0 = (((x - rect.min.x) / rect.width()) * n as f32) as usize;
        let i1 = ((i0 as f32 + step) as usize).min(n);
        let v = spec[i0..i1.max(i0 + 1)]
            .iter()
            .fold(f32::MIN, |a, &b| a.max(b));
        let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
        let y = rect.max.y - t * rect.height();
        p.line_segment(
            [Pos2::new(x, rect.max.y), Pos2::new(x, y)],
            Stroke::new(1.0, th.spectrum_fill),
        );
        pts.push(Pos2::new(x, y));
        x += 1.0;
    }
    p.add(egui::Shape::line(pts, Stroke::new(1.5, th.spectrum_line)));
    if let Some(peak) = peak {
        if peak.len() == n {
            let mut pk: Vec<Pos2> = Vec::with_capacity(rect.width() as usize);
            let mut x = rect.min.x;
            while x < rect.max.x {
                let i = (((x - rect.min.x) / rect.width()) * n as f32) as usize;
                let t = ((peak[i.min(n - 1)] - lo) / (hi - lo)).clamp(0.0, 1.0);
                pk.push(Pos2::new(x, rect.max.y - t * rect.height()));
                x += 3.0;
            }
            p.add(egui::Shape::line(
                pk,
                Stroke::new(1.0, th.accent2.linear_multiply(0.7)),
            ));
        }
    }
    if let Some(m) = marker {
        let x = rect.min.x + m.clamp(0.0, 1.0) * rect.width();
        p.line_segment(
            [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
            Stroke::new(1.5, th.warn),
        );
    }
    resp
}

// ------------------------------------------------------------ waterfall

pub struct WfTex {
    pub tex: Option<TextureHandle>,
    pub rev: u64,
    pub dark: bool,
    pub window: (usize, usize),
}

impl Default for WfTex {
    fn default() -> Self {
        Self {
            tex: None,
            rev: u64::MAX,
            dark: true,
            window: (0, 0),
        }
    }
}

pub fn waterfall(
    ui: &mut Ui,
    th: &Theme,
    wf: &WaterfallBuf,
    slot: &mut WfTex,
    height: f32,
    marker: Option<f32>,
    window: Option<(usize, usize)>,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click_and_drag(),
    );
    let p = ui.painter();
    p.rect_filled(rect, 6.0, th.panel);
    if wf.rows.is_empty() || wf.width == 0 {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "waterfall warming up…",
            FontId::proportional(12.0),
            th.text_faint,
        );
        return resp;
    }
    let (wx, wlen) = window
        .filter(|(s, l)| *l > 0 && s + l <= wf.width)
        .unwrap_or((0, wf.width));
    if slot.rev != wf.rev || slot.dark != th.dark || slot.window != (wx, wlen) || slot.tex.is_none()
    {
        let h = wf.rows.len();
        let mut img = ColorImage::new([wlen, h], Color32::BLACK);
        for (y, row) in wf.rows.iter().enumerate() {
            for (xp, v) in row[wx..wx + wlen].iter().enumerate() {
                img.pixels[y * wlen + xp] = th.wf_color(*v);
            }
        }
        match &mut slot.tex {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => {
                slot.tex = Some(
                    ui.ctx()
                        .load_texture("waterfall", img, TextureOptions::LINEAR),
                )
            }
        }
        slot.rev = wf.rev;
        slot.dark = th.dark;
        slot.window = (wx, wlen);
    }
    if let Some(tex) = &slot.tex {
        p.image(
            tex.id(),
            rect.shrink(1.0),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if let Some(m) = marker {
        let x = rect.min.x + m.clamp(0.0, 1.0) * rect.width();
        p.line_segment(
            [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
            Stroke::new(1.2, th.warn.linear_multiply(0.8)),
        );
    }
    resp
}

// -------------------------------------------------------------- S-meter

pub fn s_meter(ui: &mut Ui, th: &Theme, rms: f32, squelch: f32, gate_hint: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, th.panel);
    let db = 20.0 * (rms.max(1e-5)).log10(); // −100..0
    let t = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let fill = Rect::from_min_max(
        rect.min + Vec2::splat(2.0),
        Pos2::new(
            rect.min.x + 2.0 + (rect.width() - 4.0) * t,
            rect.max.y - 2.0,
        ),
    );
    let color = if gate_hint { th.meter } else { th.text_faint };
    p.rect_filled(fill, 3.0, color);
    if squelch > 0.0 {
        let sq_db = 20.0 * squelch.max(1e-5).log10();
        let st = ((sq_db + 60.0) / 60.0).clamp(0.0, 1.0);
        let x = rect.min.x + 2.0 + (rect.width() - 4.0) * st;
        p.line_segment(
            [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
            Stroke::new(2.0, th.warn),
        );
    }
    p.text(
        Pos2::new(rect.max.x - 6.0, rect.center().y),
        Align2::RIGHT_CENTER,
        format!("{db:>4.0} dB"),
        FontId::monospace(10.0),
        th.text_dim,
    );
}

// ----------------------------------------------------------------- tiles

#[allow(clippy::too_many_arguments)]
pub fn tile(
    ui: &mut Ui,
    th: &Theme,
    size: Vec2,
    icon_key: &str,
    label: &str,
    sub: &str,
    selected: bool,
    enabled: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let bg = if selected {
        th.panel_hi
    } else if resp.hovered() && enabled {
        th.panel_hi.linear_multiply(if th.dark { 0.8 } else { 1.0 })
    } else {
        th.panel
    };
    p.rect_filled(rect, 10.0, bg);
    if selected {
        p.rect_stroke(rect, 10.0, Stroke::new(2.0, th.accent));
    }
    let icon_c = if !enabled {
        th.text_faint
    } else if selected {
        th.accent
    } else {
        th.accent2
    };
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + size.y * 0.34),
        Vec2::splat(size.y * 0.38),
    );
    super::icons::draw(p, icon_rect, icon_key, icon_c, th.text_faint);
    p.text(
        Pos2::new(rect.center().x, rect.max.y - size.y * 0.30),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(14.5),
        if enabled { th.text } else { th.text_faint },
    );
    p.text(
        Pos2::new(rect.center().x, rect.max.y - size.y * 0.13),
        Align2::CENTER_CENTER,
        sub,
        FontId::proportional(11.5),
        th.text_dim,
    );
    resp
}

/// Small labeled value chip: `TG 2311`.
pub fn chip(ui: &mut Ui, th: &Theme, label: &str, value: &str, color: Color32) {
    let font = FontId::monospace(16.0);
    let lfont = FontId::proportional(10.5);
    let text_w = 10.0 * value.len() as f32 + 6.5 * label.len() as f32 + 26.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(text_w.max(72.0), 40.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, th.panel_hi);
    p.text(
        Pos2::new(rect.min.x + 8.0, rect.min.y + 10.0),
        Align2::LEFT_CENTER,
        label,
        lfont,
        th.text_dim,
    );
    p.text(
        Pos2::new(rect.min.x + 8.0, rect.max.y - 11.0),
        Align2::LEFT_CENTER,
        value,
        font,
        color,
    );
}
