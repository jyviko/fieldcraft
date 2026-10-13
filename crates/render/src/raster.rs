//! A drawable scene (coloured triangles, lines and points) and a CPU rasterizer for headless
//! snapshots: z-buffered, lit triangles and depth-tested lines, rendered at 2× and downsampled
//! for anti-aliasing. The GPU viewport draws the same [`Scene`].
//!
//! Adapted from SolidCraft's `crates/render/src/raster.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use crate::{Camera, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    fn f(self) -> [f64; 3] {
        [self.0 as f64 / 255.0, self.1 as f64 / 255.0, self.2 as f64 / 255.0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneTri {
    pub p: [Vec3; 3],
    /// Unit normal; lighting is two-sided.
    pub normal: Vec3,
    pub color: Rgb,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLine {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Rgb,
    /// Width in output pixels.
    pub width: f64,
}

/// A node marker: a square `size` output pixels across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScenePoint {
    pub p: Vec3,
    pub color: Rgb,
    pub size: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub tris: Vec<SceneTri>,
    pub lines: Vec<SceneLine>,
    pub points: Vec<ScenePoint>,
    /// Background gradient top → bottom.
    pub background: Option<(Rgb, Rgb)>,
    /// Scene radius around the camera target, for the clipping planes.
    pub radius: f64,
}

/// The lit colour of a surface with normal `n` seen along `view_dir` (toward the eye). The GPU
/// shader computes the same.
pub fn shade(n: Vec3, view_dir: Vec3, base: [f64; 3]) -> [f64; 3] {
    let n = if n.dot(view_dir) < 0.0 { -n } else { n };
    let key = (view_dir * 0.8 + Vec3::new(-0.3, 0.2, 0.6)).normalized().unwrap_or(view_dir);
    let diff = n.dot(key).max(0.0);
    let fill = n.dot(view_dir).max(0.0);
    let h = (key + view_dir).normalized().unwrap_or(view_dir);
    let spec = n.dot(h).max(0.0).powi(40) * 0.25;
    let k = 0.42 + 0.38 * diff + 0.25 * fill;
    [(base[0] * k + spec).min(1.0), (base[1] * k + spec).min(1.0), (base[2] * k + spec).min(1.0)]
}

/// RGBA pixels plus depth.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    depth: Vec<f32>,
}

const SS: usize = 2;
/// Largest output side in pixels.
pub const MAX_DIM: usize = 8192;
/// Largest half-width of a line or point, in supersampled pixels.
const MAX_MARK: f64 = 64.0;
/// Most samples drawn along one line segment (a segment from far off screen is bounded).
const MAX_LINE_STEPS: usize = 20_000;

impl Canvas {
    fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, rgba: vec![255; w * h * 4], depth: vec![f32::INFINITY; w * h] }
    }
    fn put(&mut self, x: usize, y: usize, c: [f64; 3], a: f64) {
        let i = (y * self.w + x) * 4;
        if let Some(px) = self.rgba.get_mut(i..i + 3) {
            for (k, v) in px.iter_mut().enumerate() {
                let old = *v as f64 / 255.0;
                *v = ((old * (1.0 - a) + c[k] * a).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
}

/// Render a scene with the camera into an RGBA canvas of `w × h` pixels (each clamped to
/// 1..=[`MAX_DIM`]).
pub fn render(scene: &Scene, cam: &Camera, w: usize, h: usize) -> Canvas {
    let (w, h) = (w.clamp(1, MAX_DIM), h.clamp(1, MAX_DIM));
    let (sw, sh) = (w * SS, h * SS);
    let mut c = Canvas::new(sw, sh);
    let (top, bot) = scene.background.unwrap_or((Rgb(236, 240, 245), Rgb(205, 212, 222)));
    let (a, b) = (top.f(), bot.f());
    for y in 0..sh {
        let t = y as f64 / sh.max(1) as f64;
        let col = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
        for x in 0..sw {
            c.put(x, y, col, 1.0);
        }
    }
    let vp = cam.view_proj(sw as f64 / sh as f64, scene.radius.max(1e-3));
    let project = |p: Vec3| -> Option<[f64; 3]> {
        let q = vp.apply(p);
        if q[3] <= 1e-9 {
            return None;
        }
        Some([(q[0] / q[3] + 1.0) * 0.5 * sw as f64, (1.0 - q[1] / q[3]) * 0.5 * sh as f64, q[2] / q[3]])
    };
    let view_dir = cam.back();
    for t in &scene.tris {
        let (Some(a), Some(b), Some(cc)) = (project(t.p[0]), project(t.p[1]), project(t.p[2])) else { continue };
        let col = shade(t.normal, view_dir, t.color.f());
        let (x0, x1) = (a[0].min(b[0]).min(cc[0]).floor().max(0.0), a[0].max(b[0]).max(cc[0]).ceil().min(sw as f64 - 1.0));
        let (y0, y1) = (a[1].min(b[1]).min(cc[1]).floor().max(0.0), a[1].max(b[1]).max(cc[1]).ceil().min(sh as f64 - 1.0));
        if !(x0 <= x1 && y0 <= y1) {
            continue;
        }
        let area = (b[0] - a[0]) * (cc[1] - a[1]) - (b[1] - a[1]) * (cc[0] - a[0]);
        if !(area.abs() >= 1e-12) {
            continue;
        }
        for y in y0 as usize..=y1 as usize {
            for x in x0 as usize..=x1 as usize {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((b[0] - px) * (cc[1] - py) - (b[1] - py) * (cc[0] - px)) / area;
                let w1 = ((cc[0] - px) * (a[1] - py) - (cc[1] - py) * (a[0] - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let z = (w0 * a[2] + w1 * b[2] + w2 * cc[2]) as f32;
                let i = y * sw + x;
                if let Some(d) = c.depth.get_mut(i)
                    && z < *d
                {
                    *d = z;
                    c.put(x, y, col, 1.0);
                }
            }
        }
    }
    for l in &scene.lines {
        let (Some(a), Some(b)) = (project(l.a), project(l.b)) else { continue };
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let steps = if len.is_finite() { (len.ceil() as usize).clamp(1, MAX_LINE_STEPS) } else { continue };
        for k in 0..=steps {
            let t = k as f64 / steps as f64;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
            dot(&mut c, p, l.width * SS as f64, l.color.f(), true);
        }
    }
    for p in &scene.points {
        if let Some(q) = project(p.p) {
            dot(&mut c, q, p.size * SS as f64, p.color.f(), false);
        }
    }
    downsample(&c, w, h)
}

/// A depth-tested disc (`round`) or square of diameter `size` at screen point `p` (x, y, depth).
fn dot(c: &mut Canvas, p: [f64; 3], size: f64, col: [f64; 3], round: bool) {
    // Widths are capped so a hostile width cannot make the loop below unbounded.
    let half = if size.is_finite() { (size * 0.5).clamp(0.5, MAX_MARK) } else { 0.5 };
    let r = half.ceil() as i64;
    let (sw, sh) = (c.w as i64, c.h as i64);
    if !(p[0].is_finite() && p[1].is_finite()) || p[0] < -half || p[1] < -half || p[0] > sw as f64 + half || p[1] > sh as f64 + half {
        return;
    }
    for dy in -r..=r {
        for dx in -r..=r {
            let (ix, iy) = (p[0].floor() as i64 + dx, p[1].floor() as i64 + dy);
            if ix < 0 || iy < 0 || ix >= sw || iy >= sh {
                continue;
            }
            let (ox, oy) = (ix as f64 + 0.5 - p[0], iy as f64 + 0.5 - p[1]);
            let d = if round { (ox * ox + oy * oy).sqrt() } else { ox.abs().max(oy.abs()) };
            let cov = (half + 0.5 - d).clamp(0.0, 1.0);
            let i = iy as usize * c.w + ix as usize;
            // Lines and points sit on faces: a small depth allowance keeps them in front.
            if cov > 0.0 && c.depth.get(i).is_some_and(|dd| (p[2] as f32) <= *dd + 2e-4) {
                c.put(ix as usize, iy as usize, col, cov);
            }
        }
    }
}

fn downsample(c: &Canvas, w: usize, h: usize) -> Canvas {
    let mut out = Canvas::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let i = ((y * SS + sy) * c.w + x * SS + sx) * 4;
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += c.rgba.get(i + k).copied().unwrap_or(0) as u32;
                    }
                }
            }
            let o = (y * w + x) * 4;
            for (k, a) in acc.iter().enumerate() {
                if let Some(v) = out.rgba.get_mut(o + k) {
                    *v = (*a / (SS * SS) as u32) as u8;
                }
            }
        }
    }
    out
}

/// Render to PNG bytes.
pub fn render_png(scene: &Scene, cam: &Camera, w: usize, h: usize) -> Option<Vec<u8>> {
    let c = render(scene, cam, w, h);
    png(c.w, c.h, &c.rgba)
}

/// RGBA pixels (`w × h`, 8 bits per channel) to PNG bytes; `None` when the sizes disagree.
pub fn png(w: usize, h: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    if w.checked_mul(h)?.checked_mul(4)? != rgba.len() || w == 0 || h == 0 {
        return None;
    }
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, u32::try_from(w).ok()?, u32::try_from(h).ok()?);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
        writer.finish().ok()?;
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StandardView;

    fn red_triangle() -> Scene {
        Scene {
            tris: vec![SceneTri {
                p: [Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, -10.0), Vec3::new(0.0, 0.0, 10.0)],
                normal: Vec3::new(0.0, -1.0, 0.0),
                color: Rgb(200, 30, 30),
            }],
            radius: 20.0,
            ..Default::default()
        }
    }

    #[test]
    fn renders_a_triangle_lines_and_points() {
        let mut scene = red_triangle();
        let mut cam = Camera { distance: 50.0, ..Default::default() };
        cam.set_view(StandardView::Front);
        let c = render(&scene, &cam, 64, 64);
        let px = |c: &Canvas, x: usize, y: usize| c.rgba[(y * c.w + x) * 4..(y * c.w + x) * 4 + 4].to_vec();
        let centre = px(&c, 32, 32);
        assert!(centre[0] > centre[1] + 50, "centre pixel is red: {centre:?}");
        // A black line across the middle, in front of the triangle, and a point.
        scene.lines.push(SceneLine { a: Vec3::new(-10.0, -0.1, 0.0), b: Vec3::new(10.0, -0.1, 0.0), color: Rgb(0, 0, 0), width: 3.0 });
        scene.points.push(ScenePoint { p: Vec3::new(0.0, -0.1, 5.0), color: Rgb(0, 0, 255), size: 5.0 });
        let c = render(&scene, &cam, 64, 64);
        assert!(px(&c, 32, 32)[0] < 40, "{:?}", px(&c, 32, 32));
        let (x, y, _) = cam.to_screen(Vec3::new(0.0, 0.0, 5.0), 64.0, 64.0, 20.0).unwrap();
        let p = px(&c, x as usize, y as usize);
        assert!(p[2] > 200 && p[0] < 60, "{p:?}");
    }

    #[test]
    fn png_bytes_and_hostile_sizes() {
        let scene = red_triangle();
        let cam = Camera::default();
        assert!(render_png(&scene, &cam, 16, 16).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));
        assert_eq!(render(&scene, &cam, 0, usize::MAX).w, 1);
        assert!(png(2, 2, &[0; 3]).is_none());
        assert!(png(usize::MAX, 2, &[]).is_none());
        // Far-away and non-finite geometry is skipped or bounded, never a panic.
        let wild = Scene {
            lines: vec![SceneLine { a: Vec3::new(-1e12, 0.0, 0.0), b: Vec3::new(1e12, 0.0, 0.0), color: Rgb(0, 0, 0), width: 1e6 }],
            points: vec![ScenePoint { p: Vec3::new(f64::NAN, 0.0, 0.0), color: Rgb(0, 0, 0), size: 4.0 }],
            radius: f64::NAN,
            ..scene
        };
        let _ = render(&wild, &cam, 32, 32);
    }
}
