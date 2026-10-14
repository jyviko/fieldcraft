//! `cargo xtask app-icon`: draw FieldCraft's application icon family with VectorCraft.
//!
//! The mark is an L-shaped bracket with a bolt hole, drawn in isometric view and covered by a
//! structured hexahedral mesh (an O-grid around the hole). The faces are separated by
//! transparent gaps instead of painted edge lines, so the backdrop shows through them in every
//! appearance. All geometry is computed here, turned into VectorCraft commands and run headlessly
//! with `vectorcraft-cli run` (`VECTORCRAFT_CLI`, else `vectorcraft-cli` on the PATH), which
//! saves each document's editable `.vectorcraft` source and exports its SVG (and PNG).
//!
//! Outputs, all under `assets/app-icon/` (see its README.md):
//! - `icon.vectorcraft`, `icon.svg`, `icon.png`: the full icon on its backdrop, 1024 × 1024.
//! - `icon-small.vectorcraft`, `icon-small.svg`, `icon-small.png`: simplified artwork (no mesh,
//!   no floor grid, faces meeting edge to edge so the tones alone separate them) for 16–32 px.
//! - `icon.iconset/` (sips), `icon.icns` (iconutil) and `icon.ico` (ImageMagick).
//! - `FieldCraft.icon/`: an Icon Composer document with separate floor-grid (Default and Dark),
//!   part and mesh layers; their editable sources are `layers/*.vectorcraft`.
//! - `Assets.car`: Apple's actool compilation of `FieldCraft.icon`, when Xcode 26 or later is
//!   installed (otherwise the step is skipped with a note and the checked-in catalog stays).
//!
//! macOS only (sips, iconutil); ImageMagick (`magick`) writes the Windows icon.

use std::f64::consts::{FRAC_PI_2, TAU};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

type P2 = [f64; 2];
type P3 = [f64; 3];

/// Canvas side in pixels; every document is one square artboard.
const CANVAS: f64 = 1024.0;
/// Pixels per model unit, and the screen point the model origin's bounding box centres on.
const SCALE: f64 = 371.0;
const CENTRE: P2 = [512.0, 512.0];
const COS30: f64 = 0.866_025_403_784_438_6;

/// The bracket, in model units (x to the right-front, y to the left-front, z up): a base plate
/// `L × D × T` with an upright wall `L × T × H` along its back edge.
const L: f64 = 1.0;
const D: f64 = 1.0;
const H: f64 = 1.0;
const T: f64 = 0.32;
/// The bolt hole through the base plate, and the half-side of the O-grid block around it (the
/// block spans the plate's depth exactly).
const HOLE: P2 = [0.5, 0.66];
const HOLE_R: f64 = 0.19;
const BLOCK: f64 = 0.34;

/// The macOS icon grid: the backdrop's square, inset 100 px on a 1024 canvas.
const BACKDROP: [f64; 2] = [100.0, 924.0];

/// Part palette: lit top faces, the left (+y) faces and the right (+x) faces, each a vertical
/// gradient, and the bolt hole's inner wall.
const TOP: [&str; 2] = ["#86e3d3", "#4cc4b1"];
const LEFT: [&str; 2] = ["#20b39f", "#118c7c"];
const RIGHT: [&str; 2] = ["#0f786d", "#0a544e"];
const BORE: [&str; 2] = ["#0b665e", "#04302c"];

#[derive(Clone, Copy, PartialEq)]
enum Appearance {
    Light,
    Dark,
}

/// How much of the mark is drawn: the full mesh and floor grid, or the small-size artwork.
#[derive(Clone, Copy, PartialEq)]
enum Detail {
    Full,
    Small,
}

impl Detail {
    /// Width of the transparent gap between neighbouring faces, in pixels on the 1024 canvas.
    fn gap(self) -> f64 {
        match self {
            Detail::Full => 16.0,
            Detail::Small => 0.0,
        }
    }
}

struct Stroke {
    paint: Value,
    width: f64,
    cap: &'static str,
}

struct Shape {
    name: String,
    d: String,
    fill: Value,
    stroke: Option<Stroke>,
    opacity: f64,
    shadow: Option<Value>,
}

impl Shape {
    fn filled(name: &str, d: String, fill: Value) -> Shape {
        Shape { name: name.into(), d, fill, stroke: None, opacity: 1.0, shadow: None }
    }

    fn stroked(name: &str, d: String, stroke: Stroke, opacity: f64) -> Shape {
        Shape { name: name.into(), d, fill: json!({"none": true}), stroke: Some(stroke), opacity, shadow: None }
    }
}

struct Layer {
    name: &'static str,
    shapes: Vec<Shape>,
    /// A path that clips the layer's contents (VectorCraft's layer clipping mask).
    clip: Option<String>,
}

struct Doc {
    title: &'static str,
    layers: Vec<Layer>,
}

// ---------------------------------------------------------------- geometry

fn project(p: P3) -> P2 {
    [CENTRE[0] + SCALE * (p[0] - p[1]) * COS30, CENTRE[1] + SCALE * ((p[0] + p[1]) * 0.5 - p[2])]
}

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn signed_area(poly: &[P2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| cross(poly[i], poly[(i + 1) % n])).sum::<f64>() * 0.5
}

/// Number for SVG path data: two decimals, trailing zeros dropped.
fn num(x: f64) -> String {
    let s = format!("{x:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn polyline(pts: &[P2], closed: bool) -> String {
    let mut d = String::new();
    for (i, p) in pts.iter().enumerate() {
        d.push_str(if i == 0 { "M" } else { " L" });
        d.push_str(&format!("{} {}", num(p[0]), num(p[1])));
    }
    if closed {
        d.push_str(" Z");
    }
    d
}

/// Move the polygon's flagged edges inward by `by` pixels (the others stay), recomputing each
/// corner as the meeting point of its two edge lines.
fn inset(poly: &[P2], flagged: &[bool], by: f64) -> Result<Vec<P2>, String> {
    let n = poly.len();
    if n < 3 || flagged.len() != n {
        return Err("a face needs at least three corners and one flag per edge".into());
    }
    let inward = signed_area(poly).signum();
    // Each edge as (point on the moved line, unit direction).
    let mut lines = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let v = sub(b, a);
        let len = v[0].hypot(v[1]);
        if len < 1e-9 {
            return Err("a face has a zero-length edge".into());
        }
        let dir = [v[0] / len, v[1] / len];
        let off = if flagged[i] { by } else { 0.0 };
        let normal = [-dir[1] * inward * off, dir[0] * inward * off];
        lines.push(([a[0] + normal[0], a[1] + normal[1]], dir));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (p, r) = lines[(i + n - 1) % n];
        let (q, s) = lines[i];
        let denom = cross(r, s);
        if denom.abs() < 1e-9 {
            return Err("a face has two collinear edges in a row".into());
        }
        let t = cross(sub(q, p), s) / denom;
        out.push([p[0] + r[0] * t, p[1] + r[1] * t]);
    }
    Ok(out)
}

/// A circle of radius `r` around `c` on the horizontal plane at height `z`, projected: four
/// cubic arcs (projection is affine, so the projected Béziers are exact). `ccw` picks the
/// screen winding (positive shoelace area when true).
fn circle_d(c: P2, r: f64, z: f64, ccw: bool) -> String {
    let arcs = |sign: f64| -> Vec<[P2; 3]> {
        let k = 0.552_284_749_830_793_4 * r;
        let at = |t: f64| [c[0] + r * t.cos(), c[1] + sign * r * t.sin()];
        let tangent = |t: f64| [-t.sin(), sign * t.cos()];
        (0..4)
            .map(|i| {
                let (t0, t1) = (i as f64 * FRAC_PI_2, (i + 1) as f64 * FRAC_PI_2);
                let (p0, p1, d0, d1) = (at(t0), at(t1), tangent(t0), tangent(t1));
                let c1 = [p0[0] + k * d0[0], p0[1] + k * d0[1]];
                let c2 = [p1[0] - k * d1[0], p1[1] - k * d1[1]];
                [project([c1[0], c1[1], z]), project([c2[0], c2[1], z]), project([p1[0], p1[1], z])]
            })
            .collect()
    };
    let mut segs = arcs(1.0);
    let anchors: Vec<P2> = segs.iter().map(|s| s[2]).collect();
    if (signed_area(&anchors) > 0.0) != ccw {
        segs = arcs(-1.0);
    }
    let start = segs[3][2];
    let mut d = format!("M{} {}", num(start[0]), num(start[1]));
    for [c1, c2, p] in &segs {
        d.push_str(&format!(" C{} {} {} {} {} {}", num(c1[0]), num(c1[1]), num(c2[0]), num(c2[1]), num(p[0]), num(p[1])));
    }
    d.push_str(" Z");
    d
}

#[derive(Clone, Copy)]
enum Tone {
    Top,
    Left,
    Right,
}

struct Face {
    name: &'static str,
    /// Corners in model space and, per edge (corner i to i + 1), whether another face of the
    /// part meets it there (those edges get the gap).
    corners: Vec<P3>,
    shared: Vec<bool>,
    tone: Tone,
}

fn faces() -> Vec<Face> {
    vec![
        Face {
            name: "wall top",
            corners: vec![[0.0, 0.0, H], [L, 0.0, H], [L, T, H], [0.0, T, H]],
            shared: vec![false, true, true, false],
            tone: Tone::Top,
        },
        Face {
            name: "base top",
            corners: vec![[0.0, T, T], [L, T, T], [L, D, T], [0.0, D, T]],
            shared: vec![true, true, true, false],
            tone: Tone::Top,
        },
        Face {
            name: "wall front",
            corners: vec![[0.0, T, T], [L, T, T], [L, T, H], [0.0, T, H]],
            shared: vec![true, true, true, false],
            tone: Tone::Left,
        },
        Face {
            name: "base front",
            corners: vec![[0.0, D, 0.0], [L, D, 0.0], [L, D, T], [0.0, D, T]],
            shared: vec![false, true, true, false],
            tone: Tone::Left,
        },
        Face {
            name: "end",
            corners: vec![[L, 0.0, 0.0], [L, D, 0.0], [L, D, T], [L, T, T], [L, T, H], [L, 0.0, H]],
            shared: vec![false, true, true, true, true, false],
            tone: Tone::Right,
        },
    ]
}

fn ccw(poly: Vec<P2>) -> Vec<P2> {
    if signed_area(&poly) < 0.0 { poly.into_iter().rev().collect() } else { poly }
}

/// A face's outline as path data, with its tone and its vertical extent on screen.
struct Outline {
    name: &'static str,
    tone: Tone,
    d: String,
    top: f64,
    bottom: f64,
}

/// Each face's outline on screen, inset at the shared edges by half the gap, wound
/// counter-clockwise (positive area); the base top carries the bolt hole wound the other way.
fn face_outlines(detail: Detail) -> Result<Vec<Outline>, String> {
    let mut out = Vec::new();
    for f in faces() {
        let screen: Vec<P2> = f.corners.iter().map(|&p| project(p)).collect();
        let poly = ccw(inset(&screen, &f.shared, detail.gap() / 2.0)?);
        let (top, bottom) = poly.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
        let mut d = polyline(&poly, true);
        if f.name == "base top" {
            d.push(' ');
            d.push_str(&circle_d(HOLE, HOLE_R, T, false));
        }
        out.push(Outline { name: f.name, tone: f.tone, d, top, bottom });
    }
    Ok(out)
}

/// Clip a segment to the axis-aligned square [lo, hi]² (Liang–Barsky).
fn clip_to_square(a: P2, b: P2, lo: f64, hi: f64) -> Option<(P2, P2)> {
    let d = sub(b, a);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [(-d[0], a[0] - lo), (d[0], hi - a[0]), (-d[1], a[1] - lo), (d[1], hi - a[1])] {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t0 < t1).then(|| ([a[0] + d[0] * t0, a[1] + d[1] * t0], [a[0] + d[0] * t1, a[1] + d[1] * t1]))
}

/// The hexahedral mesh's edges on the visible faces, as path data. The interior nodes are
/// shared, so lines run on across the face boundaries as a real structured mesh would.
fn mesh_d() -> String {
    let (x0, x1) = (HOLE[0] - BLOCK, HOLE[0] + BLOCK);
    let xs: Vec<f64> = (0..=4).map(|i| x0 + (x1 - x0) * i as f64 / 4.0).collect();
    let ys: Vec<f64> = (1..4).map(|i| T + (D - T) * i as f64 / 4.0).collect();
    let zs: Vec<f64> = (1..4).map(|i| T + (H - T) * i as f64 / 4.0).collect();
    let mid = T / 2.0;
    let mut segs: Vec<(P3, P3)> = Vec::new();
    // Base top: the O-grid block's sides and the strips either side of it.
    for x in [x0, x1] {
        segs.push(([x, T, T], [x, D, T]));
    }
    for &y in &ys {
        segs.push(([0.0, y, T], [x0, y, T]));
        segs.push(([x1, y, T], [L, y, T]));
    }
    // Wall front and wall top.
    for &x in &xs {
        segs.push(([x, T, T], [x, T, H]));
        segs.push(([x, 0.0, H], [x, T, H]));
    }
    for &z in &zs {
        segs.push(([0.0, T, z], [L, T, z]));
    }
    segs.push(([0.0, mid, H], [L, mid, H]));
    // Base front.
    for &x in &xs {
        segs.push(([x, D, 0.0], [x, D, T]));
    }
    segs.push(([0.0, D, mid], [L, D, mid]));
    // End face (x = L), an L shape.
    segs.push(([L, mid, 0.0], [L, mid, H]));
    for &y in std::iter::once(&T).chain(&ys) {
        segs.push(([L, y, 0.0], [L, y, T]));
    }
    segs.push(([L, 0.0, mid], [L, D, mid]));
    segs.push(([L, 0.0, T], [L, T, T]));
    for &z in &zs {
        segs.push(([L, 0.0, z], [L, T, z]));
    }
    let mut parts: Vec<String> = segs.iter().map(|&(a, b)| polyline(&[project(a), project(b)], false)).collect();
    // O-grid around the hole: spokes from the block's boundary nodes to the hole, and one ring
    // halfway between the hole and the block's square.
    for i in -2_i32..=2 {
        for j in -2_i32..=2 {
            if i.abs() != 2 && j.abs() != 2 {
                continue;
            }
            let node = [HOLE[0] + BLOCK * i as f64 / 2.0, HOLE[1] + BLOCK * j as f64 / 2.0];
            let t = (j as f64).atan2(i as f64);
            let rim = [HOLE[0] + HOLE_R * t.cos(), HOLE[1] + HOLE_R * t.sin()];
            parts.push(polyline(&[project([rim[0], rim[1], T]), project([node[0], node[1], T])], false));
        }
    }
    let ring: Vec<P2> = (0..96)
        .map(|k| {
            let t = k as f64 * TAU / 96.0;
            let square = BLOCK / t.cos().abs().max(t.sin().abs());
            let r = HOLE_R + 0.5 * (square - HOLE_R);
            project([HOLE[0] + r * t.cos(), HOLE[1] + r * t.sin(), T])
        })
        .collect();
    parts.push(polyline(&ring, true));
    parts.join(" ")
}

/// The floor grid on the ground plane under the part, clipped to the backdrop's square.
fn floor_d() -> String {
    let step = 1.0 / 3.0;
    let mut parts = Vec::new();
    for k in -12..=15 {
        let c = k as f64 * step;
        for (a, b) in [([c, -6.0, 0.0], [c, 6.0, 0.0]), ([-6.0, c, 0.0], [6.0, c, 0.0])] {
            if let Some((p, q)) = clip_to_square(project(a), project(b), BACKDROP[0], BACKDROP[1]) {
                parts.push(polyline(&[p, q], false));
            }
        }
    }
    parts.join(" ")
}

/// The backdrop's rounded square: straight sides with soft (continuous-looking) corners.
fn backdrop_d() -> String {
    let [lo, hi] = BACKDROP;
    let (r, h) = (220.0, 150.0); // corner span and handle length
    let p = |x: f64, y: f64| format!("{} {}", num(x), num(y));
    format!(
        "M{} L{} C{} {} {} L{} C{} {} {} L{} C{} {} {} L{} C{} {} {} Z",
        p(lo + r, lo),
        p(hi - r, lo),
        p(hi - r + h, lo),
        p(hi, lo + r - h),
        p(hi, lo + r),
        p(hi, hi - r),
        p(hi, hi - r + h),
        p(hi - r + h, hi),
        p(hi - r, hi),
        p(lo + r, hi),
        p(lo + r - h, hi),
        p(lo, hi - r + h),
        p(lo, hi - r),
        p(lo, lo + r),
        p(lo, lo + r - h),
        p(lo + r - h, lo),
        p(lo + r, lo),
    )
}

// ---------------------------------------------------------------- paints and layers

fn linear(stops: [&str; 2], top: f64, bottom: f64) -> Value {
    json!({"gradient": {"kind": "linear", "stops": [{"offset": 0, "color": stops[0]}, {"offset": 1, "color": stops[1]}],
        "start": [CANVAS / 2.0, top], "end": [CANVAS / 2.0, bottom]}})
}

fn part_layer(detail: Detail) -> Result<Layer, String> {
    let mut shapes = Vec::new();
    for Outline { name, tone, d, top, bottom } in face_outlines(detail)? {
        let colours = match tone {
            Tone::Top => TOP,
            Tone::Left => LEFT,
            Tone::Right => RIGHT,
        };
        shapes.push(Shape::filled(name, d, linear(colours, top, bottom)));
    }
    let centre = project([HOLE[0], HOLE[1], T]);
    let half = HOLE_R * SCALE * 0.5_f64.sqrt();
    shapes.push(Shape::filled("bolt hole", circle_d(HOLE, HOLE_R, T, true), linear(BORE, centre[1] - half, centre[1] + half)));
    Ok(Layer { name: "Part", shapes, clip: None })
}

fn mesh_layer() -> Result<Layer, String> {
    let clip = face_outlines(Detail::Full)?.into_iter().map(|o| o.d).collect::<Vec<_>>().join(" ");
    let stroke = Stroke { paint: json!({"color": "#ffffff"}), width: 4.5, cap: "butt" };
    Ok(Layer { name: "Mesh", shapes: vec![Shape::stroked("hexahedral mesh", mesh_d(), stroke, 0.5)], clip: Some(clip) })
}

fn floor_layer(appearance: Appearance) -> Layer {
    let colour = match appearance {
        Appearance::Light => "#7fa8a2",
        Appearance::Dark => "#4f8a82",
    };
    let fade = [(0.0, 1.0), (0.3, 0.9), (0.65, 0.35), (1.0, 0.0)];
    let stops: Vec<Value> = fade.iter().map(|(o, a)| json!({"offset": o, "color": colour, "opacity": a})).collect();
    let paint = json!({"gradient": {"kind": "radial", "stops": stops, "start": [512.0, 720.0], "end": [1090.0, 720.0], "aspect": 50}});
    let stroke = Stroke { paint, width: 3.0, cap: "butt" };
    // Clipped to the backdrop, so the flattened icon's grid stops at its rounded corners (Icon
    // Composer masks its own layers the same way).
    Layer { name: "Floor grid", shapes: vec![Shape::stroked("floor grid", floor_d(), stroke, 1.0)], clip: Some(backdrop_d()) }
}

fn backdrop_layer(appearance: Appearance) -> Layer {
    let (colours, shadow) = match appearance {
        Appearance::Light => (["#fbfdfd", "#dbe5e4"], "#1d3b38"),
        Appearance::Dark => (["#2b3433", "#141a1a"], "#000000"),
    };
    let mut shape = Shape::filled("backdrop", backdrop_d(), linear(colours, BACKDROP[0], BACKDROP[1]));
    shape.shadow = Some(json!({"mode": "multiply", "opacity": 22, "x": 0, "y": 10, "blur": 14, "color": shadow}));
    Layer { name: "Backdrop", shapes: vec![shape], clip: None }
}

fn composite(detail: Detail, appearance: Appearance) -> Result<Doc, String> {
    let mut layers = vec![backdrop_layer(appearance)];
    if detail == Detail::Full {
        layers.push(floor_layer(appearance));
    }
    layers.push(part_layer(detail)?);
    if detail == Detail::Full {
        layers.push(mesh_layer()?);
    }
    let title = match (detail, appearance) {
        (Detail::Full, Appearance::Light) => "FieldCraft application icon",
        (Detail::Full, Appearance::Dark) => "FieldCraft application icon (dark preview)",
        (Detail::Small, Appearance::Light) => "FieldCraft application icon, small sizes",
        (Detail::Small, Appearance::Dark) => "FieldCraft application icon, small sizes (dark preview)",
    };
    Ok(Doc { title, layers })
}

// ---------------------------------------------------------------- VectorCraft

/// The VectorCraft commands that draw `doc` and save its source to `source`.
fn commands(doc: &Doc, source: &Path) -> Vec<(&'static str, Value)> {
    let mut out = vec![("file.new", json!({"width": CANVAS, "height": CANVAS, "units": "Pixels", "name": doc.title, "created": null}))];
    for (i, layer) in doc.layers.iter().enumerate() {
        if i == 0 {
            out.push(("layer.setProps", json!({"name": layer.name})));
        } else {
            out.push(("layer.new", json!({"name": layer.name})));
        }
        for s in &layer.shapes {
            out.push(("path.create", json!({"d": s.d})));
            out.push(("paint.setFill", s.fill.clone()));
            match &s.stroke {
                Some(st) => {
                    out.push(("paint.setStroke", st.paint.clone()));
                    out.push(("stroke.set", json!({"weight": st.width, "cap": st.cap, "join": "round", "dash": null})));
                }
                None => out.push(("paint.setStroke", json!({"none": true}))),
            }
            let mut props = json!({"name": s.name});
            if s.opacity < 1.0 {
                props["opacity"] = json!(s.opacity * 100.0);
            }
            out.push(("object.setProps", props));
            if let Some(shadow) = &s.shadow {
                out.push(("effect.apply", json!({"effect": "stylize.dropShadow", "params": shadow})));
            }
        }
        if let Some(clip) = &layer.clip {
            out.push(("path.create", json!({"d": clip})));
            out.push(("object.setProps", json!({"name": "faces"})));
            out.push(("select.none", json!({})));
            out.push(("layer.clippingMask.toggle", json!({})));
        }
    }
    out.push(("select.none", json!({})));
    out.push(("document.save", json!({"path": source, "compress": true, "modified": null})));
    out
}

fn vectorcraft(doc: &Doc, source: &Path, exports: &[PathBuf]) -> Result<(), String> {
    let cli = std::env::var("VECTORCRAFT_CLI").unwrap_or_else(|_| "vectorcraft-cli".into());
    let mut c = Command::new(&cli);
    c.arg("run");
    for (cmd, params) in commands(doc, source) {
        c.args(["--cmd", cmd, "--params", &params.to_string()]);
    }
    for e in exports {
        c.arg("--export").arg(e);
    }
    let out = c.output().map_err(|e| format!("{cli}: {e} (set VECTORCRAFT_CLI to the vectorcraft-cli binary)"))?;
    if !out.status.success() {
        return Err(format!(
            "{}: vectorcraft-cli failed:\n{}{}",
            doc.title,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn tool(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{program} {}: {}{}", args.join(" "), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
}

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| format!("{}: not valid UTF-8", p.display()))
}

/// Pixel sizes at and below this use the small-size artwork.
const SMALL_MAX: u32 = 32;

/// Downsample the 1024 px export that suits `px` into `dest`.
fn resample(dir: &Path, px: u32, dest: &Path) -> Result<(), String> {
    let src = dir.join(if px <= SMALL_MAX { "icon-small.png" } else { "icon.png" });
    let size = px.to_string();
    tool("sips", &["-z", &size, &size, path_str(&src)?, "--out", path_str(dest)?])
}

fn icon_json() -> Value {
    let group = |name: &str, layer: Value, specular: bool| {
        json!({"layers": [layer], "name": name, "shadow": {"kind": "none", "opacity": 0}, "specular": specular,
            "translucency": {"enabled": false, "value": 0.3}})
    };
    let position = json!({"scale": 0.9, "translation-in-points": [0, 0]});
    json!({
        "fill": "system-light",
        "groups": [
            group("03 Mesh", json!({"image-name": "03-mesh.svg", "name": "03-mesh", "position": position,
                "glass-specializations": [{"appearance": "tinted", "value": true}]}), false),
            group("02 Part", json!({"image-name": "02-part.svg", "name": "02-part", "position": position,
                "glass-specializations": [{"appearance": "tinted", "value": true}]}), true),
            group("01 Floor", json!({"image-name-specializations": [{"value": "01-floor.svg"}, {"appearance": "dark", "value": "01-floor-dark.svg"}],
                "name": "01-floor", "fill": "none", "position": position,
                "glass-specializations": [{"value": false}, {"appearance": "tinted", "value": true}]}), false),
        ],
        "supported-platforms": {"circles": ["watchOS"], "squares": "shared"}
    })
}

pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("assets/app-icon");
    let layers = dir.join("layers");
    let bundle = dir.join("FieldCraft.icon");
    let assets = bundle.join("Assets");
    let iconset = dir.join("icon.iconset");
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let work = target.join("app-icon");
    for d in [&layers, &assets, &iconset, &work] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }

    // The flattened icons (backdrop included) and the dark previews.
    vectorcraft(&composite(Detail::Full, Appearance::Light)?, &dir.join("icon.vectorcraft"), &[dir.join("icon.svg"), dir.join("icon.png")])?;
    vectorcraft(
        &composite(Detail::Small, Appearance::Light)?,
        &dir.join("icon-small.vectorcraft"),
        &[dir.join("icon-small.svg"), dir.join("icon-small.png")],
    )?;
    vectorcraft(&composite(Detail::Full, Appearance::Dark)?, &work.join("icon-dark.vectorcraft"), &[work.join("icon-dark.png")])?;
    vectorcraft(&composite(Detail::Small, Appearance::Dark)?, &work.join("icon-small-dark.vectorcraft"), &[work.join("icon-small-dark.png")])?;

    // Icon Composer layers: the system supplies the backdrop.
    let single = |title, layer| Doc { title, layers: vec![layer] };
    let layer_docs = [
        ("01-floor", single("FieldCraft icon floor grid", floor_layer(Appearance::Light))),
        ("01-floor-dark", single("FieldCraft icon floor grid (Dark)", floor_layer(Appearance::Dark))),
        ("02-part", single("FieldCraft icon part", part_layer(Detail::Full)?)),
        ("03-mesh", single("FieldCraft icon mesh", mesh_layer()?)),
    ];
    for (name, doc) in &layer_docs {
        vectorcraft(doc, &layers.join(format!("{name}.vectorcraft")), &[assets.join(format!("{name}.svg"))])?;
    }
    let json = serde_json::to_string_pretty(&icon_json()).map_err(|e| e.to_string())?;
    std::fs::write(bundle.join("icon.json"), json + "\n").map_err(|e| format!("icon.json: {e}"))?;

    // macOS iconset and icns.
    for px in [16_u32, 32, 128, 256, 512] {
        resample(&dir, px, &iconset.join(format!("icon_{px}x{px}.png")))?;
        resample(&dir, px * 2, &iconset.join(format!("icon_{px}x{px}@2x.png")))?;
    }
    tool("iconutil", &["-c", "icns", path_str(&iconset)?, "-o", path_str(&dir.join("icon.icns"))?])?;

    // Windows icon.
    let mut ico_args = Vec::new();
    for px in [16_u32, 24, 32, 48, 64, 128, 256] {
        let p = work.join(format!("ico-{px}.png"));
        resample(&dir, px, &p)?;
        ico_args.push(path_str(&p)?.to_string());
    }
    ico_args.push(path_str(&dir.join("icon.ico"))?.to_string());
    let args: Vec<&str> = ico_args.iter().map(String::as_str).collect();
    tool("magick", &args)?;

    // The layered macOS 26 icon, when an Xcode with Icon Composer support is installed.
    let partial = work.join("icon-info.plist");
    let compiled = tool(
        "xcrun",
        &[
            "actool",
            path_str(&bundle)?,
            "--compile",
            path_str(&work)?,
            "--platform",
            "macosx",
            "--minimum-deployment-target",
            "11.0",
            "--app-icon",
            "FieldCraft",
            "--output-partial-info-plist",
            path_str(&partial)?,
            "--output-format",
            "human-readable-text",
        ],
    );
    match compiled {
        Ok(()) => {
            std::fs::copy(work.join("Assets.car"), dir.join("Assets.car")).map_err(|e| format!("Assets.car: {e}"))?;
            println!("app-icon: compiled FieldCraft.icon into assets/app-icon/Assets.car");
        }
        Err(e) => println!("app-icon: note: actool could not compile FieldCraft.icon, Assets.car left as it was ({e})"),
    }
    println!("app-icon: wrote assets/app-icon (dark previews in {})", work.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insetting_moves_only_the_flagged_edges() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let out = inset(&square, &[true, false, false, false], 2.0).unwrap();
        assert_eq!(out, vec![[0.0, 2.0], [10.0, 2.0], [10.0, 10.0], [0.0, 10.0]]);
        // The same result whichever way the polygon winds.
        let rev: Vec<P2> = square.iter().rev().copied().collect();
        let out = inset(&rev, &[false, false, true, false], 2.0).unwrap();
        assert!(out.contains(&[0.0, 2.0]) && out.contains(&[10.0, 2.0]));
        assert!(inset(&square[..2], &[true, true], 1.0).is_err());
    }

    #[test]
    fn the_hole_winds_against_its_face() {
        for o in face_outlines(Detail::Full).unwrap() {
            assert!(o.d.starts_with('M') && o.d.contains('Z'), "{}: {}", o.name, o.d);
        }
        let anchors = |d: &str| -> Vec<P2> {
            d.split('C')
                .skip(1)
                .map(|s| {
                    let n: Vec<f64> = s.split_whitespace().filter_map(|x| x.trim_end_matches('Z').parse().ok()).collect();
                    [n[4], n[5]]
                })
                .collect()
        };
        assert!(signed_area(&anchors(&circle_d(HOLE, HOLE_R, T, true))) > 0.0);
        assert!(signed_area(&anchors(&circle_d(HOLE, HOLE_R, T, false))) < 0.0);
    }

    #[test]
    fn segments_clip_to_the_square() {
        assert_eq!(clip_to_square([-5.0, 5.0], [15.0, 5.0], 0.0, 10.0), Some(([0.0, 5.0], [10.0, 5.0])));
        assert_eq!(clip_to_square([-5.0, -5.0], [-1.0, 20.0], 0.0, 10.0), None);
    }

    #[test]
    fn every_document_starts_new_and_ends_saved() {
        let doc = composite(Detail::Full, Appearance::Light).unwrap();
        let steps = commands(&doc, Path::new("/tmp/icon.vectorcraft"));
        assert_eq!(steps.first().map(|s| s.0), Some("file.new"));
        assert_eq!(steps.last().map(|s| s.0), Some("document.save"));
        assert_eq!(steps.iter().filter(|s| s.0 == "layer.new").count(), 3);
        // The floor grid is clipped to the backdrop and the mesh to the faces.
        assert_eq!(steps.iter().filter(|s| s.0 == "layer.clippingMask.toggle").count(), 2);
    }
}
