//! Vector icons, painted with egui primitives — crisp at any DPI, no assets.

use eframe::egui::{epaint::PathShape, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

fn p(r: &Rect, x: f32, y: f32) -> Pos2 {
    // normalized 0..1 coordinates inside the icon rect
    Pos2::new(r.min.x + x * r.width(), r.min.y + y * r.height())
}

fn poly(painter: &Painter, pts: Vec<Pos2>, fill: Color32) {
    painter.add(Shape::Path(PathShape::convex_polygon(
        pts,
        fill,
        Stroke::NONE,
    )));
}

fn line(painter: &Painter, r: &Rect, pts: &[(f32, f32)], stroke: Stroke) {
    let pts: Vec<Pos2> = pts.iter().map(|(x, y)| p(r, *x, *y)).collect();
    painter.add(Shape::line(pts, stroke));
}

fn sine(painter: &Painter, r: &Rect, y0: f32, amp: f32, cycles: f32, stroke: Stroke) {
    let n = 24;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            p(
                r,
                0.08 + 0.84 * t,
                y0 - amp * (t * cycles * std::f32::consts::TAU).sin(),
            )
        })
        .collect();
    painter.add(Shape::line(pts, stroke));
}

/// Draw the icon for a mode key (or "devices"/"doctor"/"power"/"logo").
pub fn draw(painter: &Painter, rect: Rect, key: &str, color: Color32, dim: Color32) {
    let r = &rect;
    let w = (rect.width() * 0.09).clamp(1.5, 3.0);
    let s = Stroke::new(w, color);
    let sd = Stroke::new(w, dim);
    match key {
        "nfm" => {
            // antenna with tight waves
            line(painter, r, &[(0.5, 0.35), (0.5, 0.9)], s);
            painter.circle_filled(p(r, 0.5, 0.3), w * 1.1, color);
            for (i, rad) in [0.16f32, 0.28].iter().enumerate() {
                let st = if i == 0 { s } else { sd };
                let pi = std::f32::consts::PI;
                arc(painter, p(r, 0.5, 0.3), *rad * rect.width(), -2.2, -0.9, st);
                arc(
                    painter,
                    p(r, 0.5, 0.3),
                    *rad * rect.width(),
                    0.9 - pi,
                    2.2 - pi,
                    st,
                );
            }
        }
        "wfm" => {
            // beefy broadcast waves
            painter.circle_filled(p(r, 0.32, 0.5), w * 1.2, color);
            for (i, rad) in [0.18f32, 0.32, 0.46].iter().enumerate() {
                let st = if i < 2 { s } else { sd };
                arc(painter, p(r, 0.32, 0.5), *rad * rect.width(), -0.9, 0.9, st);
            }
        }
        "am" => {
            sine(painter, r, 0.5, 0.16, 2.0, s);
            // envelope
            line(painter, r, &[(0.08, 0.24), (0.5, 0.18), (0.92, 0.24)], sd);
            line(painter, r, &[(0.08, 0.76), (0.5, 0.82), (0.92, 0.76)], sd);
        }
        "usb" => {
            sine(painter, r, 0.62, 0.13, 2.0, sd);
            poly(
                painter,
                vec![
                    p(r, 0.2, 0.42),
                    p(r, 0.8, 0.42),
                    p(r, 0.62, 0.2),
                    p(r, 0.28, 0.2),
                ],
                color,
            );
        }
        "lsb" => {
            sine(painter, r, 0.38, 0.13, 2.0, sd);
            poly(
                painter,
                vec![
                    p(r, 0.2, 0.58),
                    p(r, 0.8, 0.58),
                    p(r, 0.62, 0.8),
                    p(r, 0.28, 0.8),
                ],
                color,
            );
        }
        "dmr" => {
            // two timeslots
            painter.circle_filled(p(r, 0.32, 0.5), rect.width() * 0.16, color);
            painter.circle_stroke(p(r, 0.68, 0.5), rect.width() * 0.16, s);
        }
        "ysf" => {
            poly(
                painter,
                vec![
                    p(r, 0.5, 0.15),
                    p(r, 0.85, 0.5),
                    p(r, 0.5, 0.85),
                    p(r, 0.15, 0.5),
                ],
                color,
            );
            painter.circle_filled(p(r, 0.5, 0.5), w * 1.2, dim);
        }
        "dstar" => {
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
                let c = p(r, 0.5, 0.5);
                let tip = c + Vec2::angled(a) * rect.width() * 0.38;
                let l = c + Vec2::angled(a + 0.5) * rect.width() * 0.1;
                let rr = c + Vec2::angled(a - 0.5) * rect.width() * 0.1;
                poly(painter, vec![c, l, tip, rr], color);
            }
        }
        "nxdn" => {
            painter.rect_stroke(
                Rect::from_center_size(p(r, 0.5, 0.5), rect.size() * 0.55),
                2.0,
                s,
            );
            painter.rect_filled(
                Rect::from_center_size(p(r, 0.5, 0.5), rect.size() * 0.24),
                1.0,
                color,
            );
        }
        "p25" => {
            poly(
                painter,
                vec![p(r, 0.5, 0.16), p(r, 0.86, 0.8), p(r, 0.14, 0.8)],
                color,
            );
        }
        "m17" => {
            line(
                painter,
                r,
                &[
                    (0.16, 0.8),
                    (0.16, 0.25),
                    (0.5, 0.62),
                    (0.84, 0.25),
                    (0.84, 0.8),
                ],
                s,
            );
        }
        "pocsag" => {
            // pager
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.14, 0.25), p(r, 0.86, 0.75)),
                3.0,
                s,
            );
            line(painter, r, &[(0.26, 0.45), (0.62, 0.45)], sd);
            line(painter, r, &[(0.26, 0.58), (0.74, 0.58)], sd);
            painter.circle_filled(p(r, 0.74, 0.42), w * 0.9, color);
        }
        "aprs" => {
            painter.circle_stroke(p(r, 0.5, 0.5), rect.width() * 0.26, s);
            painter.circle_filled(p(r, 0.5, 0.5), w * 1.1, color);
            line(painter, r, &[(0.5, 0.08), (0.5, 0.3)], s);
            line(painter, r, &[(0.5, 0.7), (0.5, 0.92)], s);
            line(painter, r, &[(0.08, 0.5), (0.3, 0.5)], s);
            line(painter, r, &[(0.7, 0.5), (0.92, 0.5)], s);
        }
        "rtty" => {
            for (i, y) in [0.3f32, 0.5, 0.7].iter().enumerate() {
                let x0 = 0.16 + 0.05 * i as f32;
                let x1 = 0.84 - 0.05 * i as f32;
                line(
                    painter,
                    r,
                    &[(x0, *y), (x1, *y)],
                    if i == 1 { s } else { sd },
                );
            }
            // FSK step
            line(
                painter,
                r,
                &[(0.3, 0.3), (0.3, 0.5), (0.55, 0.5), (0.55, 0.7)],
                s,
            );
        }
        "adsb" => {
            // plane silhouette
            poly(
                painter,
                vec![
                    p(r, 0.5, 0.1),
                    p(r, 0.58, 0.42),
                    p(r, 0.5, 0.62),
                    p(r, 0.42, 0.42),
                ],
                color,
            );
            poly(
                painter,
                vec![
                    p(r, 0.5, 0.32),
                    p(r, 0.92, 0.55),
                    p(r, 0.92, 0.64),
                    p(r, 0.5, 0.5),
                ],
                color,
            );
            poly(
                painter,
                vec![
                    p(r, 0.5, 0.32),
                    p(r, 0.08, 0.55),
                    p(r, 0.08, 0.64),
                    p(r, 0.5, 0.5),
                ],
                color,
            );
            poly(
                painter,
                vec![
                    p(r, 0.5, 0.62),
                    p(r, 0.68, 0.82),
                    p(r, 0.68, 0.88),
                    p(r, 0.5, 0.78),
                    p(r, 0.32, 0.88),
                    p(r, 0.32, 0.82),
                ],
                color,
            );
        }
        "scanner" => {
            arc(painter, p(r, 0.5, 0.5), rect.width() * 0.3, -2.6, 0.4, s);
            arc(painter, p(r, 0.5, 0.5), rect.width() * 0.3, 0.55, 3.55, sd);
            // arrowhead
            poly(
                painter,
                vec![p(r, 0.86, 0.36), p(r, 0.74, 0.28), p(r, 0.76, 0.46)],
                color,
            );
        }
        "waterfall" => {
            for (i, yy) in [0.22f32, 0.42, 0.62, 0.82].iter().enumerate() {
                let widths = [0.7, 0.5, 0.62, 0.34];
                let c = if i % 2 == 0 { color } else { dim };
                painter.rect_filled(
                    Rect::from_min_max(p(r, 0.15, yy - 0.07), p(r, 0.15 + widths[i], yy + 0.07)),
                    1.0,
                    c,
                );
            }
        }
        "devices" => {
            // USB trident
            line(painter, r, &[(0.5, 0.1), (0.5, 0.85)], s);
            poly(
                painter,
                vec![p(r, 0.5, 0.05), p(r, 0.56, 0.18), p(r, 0.44, 0.18)],
                color,
            );
            line(painter, r, &[(0.5, 0.45), (0.22, 0.3)], sd);
            painter.circle_filled(p(r, 0.2, 0.28), w, dim);
            line(painter, r, &[(0.5, 0.6), (0.78, 0.45)], sd);
            painter.rect_filled(
                Rect::from_center_size(p(r, 0.8, 0.43), Vec2::splat(w * 2.0)),
                0.5,
                dim,
            );
            painter.circle_filled(p(r, 0.5, 0.85), w * 1.3, color);
        }
        "doctor" => {
            // pulse line
            line(
                painter,
                r,
                &[
                    (0.08, 0.55),
                    (0.32, 0.55),
                    (0.42, 0.25),
                    (0.54, 0.8),
                    (0.64, 0.55),
                    (0.92, 0.55),
                ],
                s,
            );
        }
        "power" => {
            arc(painter, p(r, 0.5, 0.55), rect.width() * 0.3, -0.6, 3.74, s);
            line(painter, r, &[(0.5, 0.12), (0.5, 0.5)], s);
        }
        "rec" => {
            painter.circle_filled(p(r, 0.5, 0.5), rect.width() * 0.3, color);
        }
        _ => {
            painter.circle_stroke(p(r, 0.5, 0.5), rect.width() * 0.3, s);
        }
    }
}

/// Stroke an arc (radians, clockwise from +x).
fn arc(painter: &Painter, center: Pos2, radius: f32, a0: f32, a1: f32, stroke: Stroke) {
    let n = 18;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            center + Vec2::angled(a) * radius
        })
        .collect();
    painter.add(Shape::line(pts, stroke));
}

/// The deck logotype mark: antenna over a ground plane, radiating.
pub fn logo(painter: &Painter, rect: Rect, color: Color32, dim: Color32) {
    let r = &rect;
    let w = (rect.width() * 0.045).clamp(2.0, 5.0);
    let s = Stroke::new(w, color);
    let sd = Stroke::new(w * 0.8, dim);
    line(painter, r, &[(0.5, 0.18), (0.5, 0.78)], s);
    painter.circle_filled(p(r, 0.5, 0.14), w * 1.4, color);
    for (i, rad) in [0.2f32, 0.34, 0.48].iter().enumerate() {
        let st = if i == 0 { s } else { sd };
        arc(
            painter,
            p(r, 0.5, 0.14),
            *rad * rect.width(),
            -1.1,
            0.25,
            st,
        );
        arc(
            painter,
            p(r, 0.5, 0.14),
            *rad * rect.width(),
            std::f32::consts::PI - 0.25,
            std::f32::consts::PI + 1.1,
            st,
        );
    }
    // ground plane
    line(painter, r, &[(0.26, 0.78), (0.74, 0.78)], s);
    line(painter, r, &[(0.34, 0.86), (0.66, 0.86)], sd);
    line(painter, r, &[(0.42, 0.94), (0.58, 0.94)], sd);
}

/// Battery glyph with fill level (0..1); `charging` overlays a bolt.
pub fn battery(
    painter: &Painter,
    rect: Rect,
    level: f32,
    charging: bool,
    th: &super::theme::Theme,
) {
    let r = rect.shrink(rect.height() * 0.12);
    let body = Rect::from_min_max(r.min, Pos2::new(r.max.x - r.width() * 0.12, r.max.y));
    painter.rect_stroke(body, 2.0, Stroke::new(1.4, th.text_dim));
    let nub = Rect::from_min_max(
        Pos2::new(body.max.x + 1.0, r.min.y + r.height() * 0.28),
        Pos2::new(r.max.x, r.max.y - r.height() * 0.28),
    );
    painter.rect_filled(nub, 1.0, th.text_dim);
    let color = if level < 0.2 {
        th.err
    } else if level < 0.4 {
        th.warn
    } else {
        th.ok
    };
    let fill = body.shrink(2.5);
    let fill = Rect::from_min_max(
        fill.min,
        Pos2::new(
            fill.min.x + fill.width() * level.clamp(0.02, 1.0),
            fill.max.y,
        ),
    );
    painter.rect_filled(fill, 1.0, color);
    if charging {
        let b = body;
        poly(
            painter,
            vec![
                Pos2::new(b.center().x + b.width() * 0.08, b.min.y + 1.0),
                Pos2::new(b.center().x - b.width() * 0.16, b.center().y + 1.0),
                Pos2::new(b.center().x + b.width() * 0.02, b.center().y + 1.0),
                Pos2::new(b.center().x - b.width() * 0.08, b.max.y - 1.0),
                Pos2::new(b.center().x + b.width() * 0.16, b.center().y - 1.0),
                Pos2::new(b.center().x - b.width() * 0.02, b.center().y - 1.0),
            ],
            th.text,
        );
    }
}

/// Speaker glyph; `pct` None = muted.
pub fn speaker(painter: &Painter, rect: Rect, pct: Option<u8>, th: &super::theme::Theme) {
    let r = &rect;
    let c = if pct.is_some() {
        th.text_dim
    } else {
        th.text_faint
    };
    poly(
        painter,
        vec![
            p(r, 0.12, 0.4),
            p(r, 0.34, 0.4),
            p(r, 0.52, 0.2),
            p(r, 0.52, 0.8),
            p(r, 0.34, 0.6),
            p(r, 0.12, 0.6),
        ],
        c,
    );
    let s = Stroke::new((rect.width() * 0.08).clamp(1.2, 2.2), c);
    match pct {
        Some(v) => {
            if v > 5 {
                arc(painter, p(r, 0.55, 0.5), rect.width() * 0.2, -0.9, 0.9, s);
            }
            if v > 55 {
                arc(painter, p(r, 0.55, 0.5), rect.width() * 0.34, -0.9, 0.9, s);
            }
        }
        None => {
            line(painter, r, &[(0.6, 0.32), (0.9, 0.68)], s);
            line(painter, r, &[(0.9, 0.32), (0.6, 0.68)], s);
        }
    }
}
