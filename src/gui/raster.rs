//! Headless CPU rasterizer: egui → tessellated meshes → RGBA pixels → PNG.
//! No GPU, no window system — used by `deck shot` for README screenshots
//! and as a full-UI smoke test in CI.

use eframe::egui::{self, epaint, Color32, TextureId};
use std::collections::HashMap;

struct Tex {
    size: [usize; 2],
    pixels: Vec<Color32>, // premultiplied
}

fn apply_delta(store: &mut HashMap<TextureId, Tex>, id: TextureId, delta: &epaint::ImageDelta) {
    let (size, pixels): ([usize; 2], Vec<Color32>) = match &delta.image {
        epaint::ImageData::Color(img) => (img.size, img.pixels.clone()),
        epaint::ImageData::Font(f) => (f.size, f.srgba_pixels(None).collect()),
    };
    match delta.pos {
        None => {
            store.insert(id, Tex { size, pixels });
        }
        Some([px, py]) => {
            if let Some(t) = store.get_mut(&id) {
                for y in 0..size[1] {
                    for x in 0..size[0] {
                        let dx = px + x;
                        let dy = py + y;
                        if dx < t.size[0] && dy < t.size[1] {
                            t.pixels[dy * t.size[0] + dx] = pixels[y * size[0] + x];
                        }
                    }
                }
            }
        }
    }
}

fn sample(t: &Tex, u: f32, v: f32) -> Color32 {
    let x = (u * t.size[0] as f32 - 0.5).clamp(0.0, t.size[0] as f32 - 1.0);
    let y = (v * t.size[1] as f32 - 0.5).clamp(0.0, t.size[1] as f32 - 1.0);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(t.size[0] - 1), (y0 + 1).min(t.size[1] - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: usize, y: usize| t.pixels[y * t.size[0] + x];
    let lerp = |a: Color32, b: Color32, f: f32| {
        let l = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * f) as u8;
        Color32::from_rgba_premultiplied(
            l(a.r(), b.r()),
            l(a.g(), b.g()),
            l(a.b(), b.b()),
            l(a.a(), b.a()),
        )
    };
    let top = lerp(px(x0, y0), px(x1, y0), fx);
    let bot = lerp(px(x0, y1), px(x1, y1), fx);
    lerp(top, bot, fy)
}

fn mul(a: Color32, b: Color32) -> Color32 {
    let m = |x: u8, y: u8| ((u16::from(x) * u16::from(y)) / 255) as u8;
    Color32::from_rgba_premultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

/// Render `frames` UI passes and rasterize the last one.
pub fn render_rgba(
    width: u32,
    height: u32,
    ppp: f32,
    frames: u32,
    mut ui_fn: impl FnMut(&egui::Context),
) -> Vec<u8> {
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(ppp);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::Vec2::new(width as f32 / ppp, height as f32 / ppp),
        )),
        ..Default::default()
    };
    let mut textures: HashMap<TextureId, Tex> = HashMap::new();
    let mut last = None;
    for _ in 0..frames.max(1) {
        let out = ctx.run(raw.clone(), |ctx| ui_fn(ctx));
        for (id, delta) in &out.textures_delta.set {
            apply_delta(&mut textures, *id, delta);
        }
        last = Some(out);
    }
    let out = last.unwrap();
    let clipped = ctx.tessellate(out.shapes, ppp);

    let (w, h) = (width as usize, height as usize);
    let mut fb = vec![Color32::BLACK; w * h];

    for cp in &clipped {
        let epaint::Primitive::Mesh(mesh) = &cp.primitive else {
            continue;
        };
        let tex = textures.get(&mesh.texture_id);
        let clip = cp.clip_rect;
        let (cx0, cy0) = (
            ((clip.min.x * ppp).floor().max(0.0)) as usize,
            ((clip.min.y * ppp).floor().max(0.0)) as usize,
        );
        let (cx1, cy1) = (
            ((clip.max.x * ppp).ceil() as usize).min(w),
            ((clip.max.y * ppp).ceil() as usize).min(h),
        );
        for tri in mesh.indices.chunks_exact(3) {
            let v = [
                &mesh.vertices[tri[0] as usize],
                &mesh.vertices[tri[1] as usize],
                &mesh.vertices[tri[2] as usize],
            ];
            let px = |i: usize| (v[i].pos.x * ppp, v[i].pos.y * ppp);
            let (x0, y0) = px(0);
            let (x1, y1) = px(1);
            let (x2, y2) = px(2);
            let minx = x0.min(x1).min(x2).floor().max(cx0 as f32) as usize;
            let maxx = (x0.max(x1).max(x2).ceil() as usize).min(cx1);
            let miny = y0.min(y1).min(y2).floor().max(cy0 as f32) as usize;
            let maxy = (y0.max(y1).max(y2).ceil() as usize).min(cy1);
            if minx >= maxx || miny >= maxy {
                continue;
            }
            let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
            if area.abs() < 1e-6 {
                continue;
            }
            for py in miny..maxy {
                for pxx in minx..maxx {
                    let (sx, sy) = (pxx as f32 + 0.5, py as f32 + 0.5);
                    let w0 = ((x1 - sx) * (y2 - sy) - (x2 - sx) * (y1 - sy)) / area;
                    let w1 = ((x2 - sx) * (y0 - sy) - (x0 - sx) * (y2 - sy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 {
                        continue;
                    }
                    let ic = |f: fn(&epaint::Vertex) -> u8| {
                        (w0 * f32::from(f(v[0]))
                            + w1 * f32::from(f(v[1]))
                            + w2 * f32::from(f(v[2])))
                        .clamp(0.0, 255.0) as u8
                    };
                    let mut src = Color32::from_rgba_premultiplied(
                        ic(|v| v.color.r()),
                        ic(|v| v.color.g()),
                        ic(|v| v.color.b()),
                        ic(|v| v.color.a()),
                    );
                    if let Some(t) = tex {
                        let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                        let vv = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                        src = mul(src, sample(t, u, vv));
                    }
                    let a = u16::from(src.a());
                    if a == 0 {
                        continue;
                    }
                    let dst = fb[py * w + pxx];
                    let blend = |s: u8, d: u8| {
                        (u16::from(s) + (u16::from(d) * (255 - a)) / 255).min(255) as u8
                    };
                    fb[py * w + pxx] = Color32::from_rgba_premultiplied(
                        blend(src.r(), dst.r()),
                        blend(src.g(), dst.g()),
                        blend(src.b(), dst.b()),
                        blend(src.a(), dst.a()),
                    );
                }
            }
        }
    }

    let mut rgba = Vec::with_capacity(w * h * 4);
    for c in fb {
        rgba.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
    }
    rgba
}

pub fn write_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_a_filled_panel() {
        let rgba = render_rgba(120, 80, 1.0, 2, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::from_rgb(10, 200, 100)))
                .show(ctx, |ui| {
                    ui.label("x");
                });
        });
        assert_eq!(rgba.len(), 120 * 80 * 4);
        // center pixel should be the fill color
        let i = (40 * 120 + 60) * 4;
        assert_eq!(&rgba[i..i + 3], &[10, 200, 100]);
    }

    #[test]
    fn text_marks_pixels() {
        let a = render_rgba(200, 60, 1.0, 2, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::BLACK))
                .show(ctx, |_| {});
        });
        let b = render_rgba(200, 60, 1.0, 2, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::BLACK))
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("DECK 145.500")
                            .size(24.0)
                            .color(Color32::WHITE),
                    );
                });
        });
        assert_ne!(a, b, "text should change the framebuffer");
        let lit = b.chunks(4).filter(|c| c[0] > 100).count();
        assert!(lit > 50, "glyphs should light up pixels, got {lit}");
    }
}
