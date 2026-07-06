//! Flat, modern themes. Dark = phosphor-on-carbon "hacker" look;
//! light = paper + ink. Every color the GUI uses lives here.

use eframe::egui::{self, Color32};

#[derive(Clone)]
pub struct Theme {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub panel_hi: Color32,
    pub outline: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub rec: Color32,
    pub sel_bg: Color32,
    pub sel_fg: Color32,
    pub meter: Color32,
    pub spectrum_fill: Color32,
    pub spectrum_line: Color32,
    pub grid: Color32,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(0x0a, 0x0e, 0x12),
            panel: Color32::from_rgb(0x11, 0x17, 0x1d),
            panel_hi: Color32::from_rgb(0x18, 0x20, 0x28),
            outline: Color32::from_rgb(0x22, 0x2c, 0x36),
            text: Color32::from_rgb(0xd8, 0xe4, 0xea),
            text_dim: Color32::from_rgb(0x8a, 0x9a, 0xa6),
            text_faint: Color32::from_rgb(0x4a, 0x58, 0x64),
            accent: Color32::from_rgb(0x2d, 0xe6, 0xa8), // phosphor green
            accent2: Color32::from_rgb(0x3b, 0xc2, 0xff), // signal cyan
            ok: Color32::from_rgb(0x2d, 0xe6, 0xa8),
            warn: Color32::from_rgb(0xff, 0xc4, 0x4d),
            err: Color32::from_rgb(0xff, 0x5d, 0x6c),
            rec: Color32::from_rgb(0xff, 0x45, 0x58),
            sel_bg: Color32::from_rgb(0x2d, 0xe6, 0xa8),
            sel_fg: Color32::from_rgb(0x06, 0x12, 0x0d),
            meter: Color32::from_rgb(0x2d, 0xe6, 0xa8),
            spectrum_fill: Color32::from_rgba_premultiplied(0x12, 0x5c, 0x46, 0xa0),
            spectrum_line: Color32::from_rgb(0x2d, 0xe6, 0xa8),
            grid: Color32::from_rgb(0x1a, 0x24, 0x2e),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::from_rgb(0xf4, 0xf6, 0xf8),
            panel: Color32::from_rgb(0xff, 0xff, 0xff),
            panel_hi: Color32::from_rgb(0xe9, 0xee, 0xf2),
            outline: Color32::from_rgb(0xd4, 0xdc, 0xe2),
            text: Color32::from_rgb(0x18, 0x22, 0x2a),
            text_dim: Color32::from_rgb(0x5a, 0x6a, 0x76),
            text_faint: Color32::from_rgb(0x9a, 0xa8, 0xb2),
            accent: Color32::from_rgb(0x0b, 0x84, 0x62),
            accent2: Color32::from_rgb(0x06, 0x6d, 0xb2),
            ok: Color32::from_rgb(0x0b, 0x84, 0x62),
            warn: Color32::from_rgb(0xa8, 0x6b, 0x00),
            err: Color32::from_rgb(0xc6, 0x23, 0x41),
            rec: Color32::from_rgb(0xd6, 0x2c, 0x3e),
            sel_bg: Color32::from_rgb(0x0b, 0x84, 0x62),
            sel_fg: Color32::WHITE,
            meter: Color32::from_rgb(0x0b, 0x84, 0x62),
            spectrum_fill: Color32::from_rgba_premultiplied(0x9c, 0xd6, 0xc4, 0xa0),
            spectrum_line: Color32::from_rgb(0x0b, 0x84, 0x62),
            grid: Color32::from_rgb(0xdd, 0xe5, 0xea),
        }
    }

    /// Waterfall intensity 0..255 → color (per-theme ramp).
    pub fn wf_color(&self, v: u8) -> Color32 {
        let t = f32::from(v) / 255.0;
        if self.dark {
            // carbon → deep blue → cyan → green → yellow → white
            ramp(
                t,
                &[
                    (0.00, (0x0a, 0x0e, 0x12)),
                    (0.25, (0x0b, 0x2a, 0x5e)),
                    (0.50, (0x0e, 0x86, 0x9a)),
                    (0.70, (0x2d, 0xe6, 0xa8)),
                    (0.88, (0xf2, 0xe6, 0x4e)),
                    (1.00, (0xff, 0xff, 0xff)),
                ],
            )
        } else {
            // paper → pale blue → blue → violet → crimson → near-black
            ramp(
                t,
                &[
                    (0.00, (0xf4, 0xf6, 0xf8)),
                    (0.30, (0xbc, 0xd8, 0xee)),
                    (0.55, (0x4d, 0x8e, 0xd4)),
                    (0.75, (0x7a, 0x4f, 0xb6)),
                    (0.90, (0xc8, 0x35, 0x52)),
                    (1.00, (0x20, 0x10, 0x18)),
                ],
            )
        }
    }

    /// Apply base visuals so stock egui widgets (scrollbars, popups)
    /// harmonize with the theme.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = self.bg;
        v.window_fill = self.panel;
        v.window_stroke = egui::Stroke::new(1.0, self.outline);
        v.override_text_color = Some(self.text);
        v.widgets.noninteractive.bg_fill = self.panel;
        v.widgets.inactive.bg_fill = self.panel_hi;
        v.widgets.hovered.bg_fill = self.panel_hi;
        v.widgets.active.bg_fill = self.sel_bg;
        v.selection.bg_fill = self.sel_bg.linear_multiply(0.35);
        let mut style = (*ctx.style()).clone();
        style.visuals = v;
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        ctx.set_style(style);
    }
}

fn ramp(t: f32, stops: &[(f32, (u8, u8, u8))]) -> Color32 {
    let mut prev = stops[0];
    for &s in stops {
        if t <= s.0 {
            let span = (s.0 - prev.0).max(1e-6);
            let k = ((t - prev.0) / span).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * k) as u8;
            return Color32::from_rgb(
                mix(prev.1 .0, s.1 .0),
                mix(prev.1 .1, s.1 .1),
                mix(prev.1 .2, s.1 .2),
            );
        }
        prev = s;
    }
    let last = stops.last().unwrap().1;
    Color32::from_rgb(last.0, last.1, last.2)
}
