//! `cargo xtask app-icon`: draw FieldCraft's application icon family with VectorCraft.
//!
//! The mark is a hexahedral-meshed cube in isometric view whose faces carry a banded colour
//! fringe (blue to red, hottest at the cube's front corner) as a stress or temperature result
//! would. The faces are separated by transparent gaps instead of painted edge lines, so the
//! backdrop shows through them in every appearance. All geometry is computed here, turned into VectorCraft commands and
//! run headlessly with `vectorcraft-cli run` (`VECTORCRAFT_CLI`, else `vectorcraft-cli` on the
//! PATH), which saves each document's editable `.vectorcraft` source and exports its SVG (and PNG).
//!
//! Outputs, all under `assets/app-icon/` (see its README.md):
//! - `icon.vectorcraft`, `icon.svg`, `icon.png`: the full icon on its backdrop, 1024 × 1024.
//! - `icon-small.vectorcraft`, `icon-small.svg`, `icon-small.png`: simplified artwork for 16–32 px
//!   (no mesh or floor grid, faces edge to edge).
//! - `icon.iconset/` (sips), `icon.icns` (iconutil) and `icon.ico` (ImageMagick).
//! - `FieldCraft.icon/`: an Icon Composer document with separate floor-grid (Default and Dark),
//!   and cube layers (the mesh shares the cube's); their editable sources are `layers/*.vectorcraft`.
//! - `Assets.car`: Apple's actool compilation of `FieldCraft.icon`, when Xcode 26 or later is
//!   installed (otherwise the step is skipped with a note and the checked-in catalog stays).
//!
//! macOS only (sips, iconutil); ImageMagick (`magick`) writes the Windows icon.

use std::f64::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

type P2 = [f64; 2];
type P3 = [f64; 3];

/// Canvas side in pixels; every document is one square artboard.
const CANVAS: f64 = 1024.0;
/// Pixels per model unit, and the screen point the model origin projects to.
const SCALE: f64 = 300.0;
const CENTRE: P2 = [512.0, 500.0];
const COS30: f64 = 0.866_025_403_784_438_6;

/// The cube's half side, in model units (x to the right-front, y to the left-front, z up); the
/// cube is centred on the origin.
const A: f64 = 0.5;
/// Mesh cells along each edge.
const CELLS: usize = 4;

/// The fringe: a field that grows towards the hot point (the cube's front corner), shown as
/// bands of constant colour. `BAND_RADII[k]` is the distance from the hot point inside which
/// band `k + 1` of `FRINGE` starts; the coldest band fills the rest of the face.
const HOT: P3 = [A, A, A];
const FRINGE: [&str; 7] = ["#2c4fbf", "#2f86e0", "#22bcd8", "#3fc584", "#f0d13a", "#f28a2b", "#de3b2f"];
const BAND_RADII: [f64; 6] = [1.28, 1.07, 0.87, 0.67, 0.47, 0.27];

/// The macOS icon grid: the backdrop's square, inset 100 px on a 1024 canvas.
const BACKDROP: [f64; 2] = [100.0, 924.0];

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
            Detail::Full => 14.0,
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

fn add3(a: P3, b: P3, k: f64) -> P3 {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}

fn dot3(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A circle of radius `r` around `c` in the plane spanned by the unit vectors `u` and `v`,
/// projected: four cubic arcs (projection is affine, so the projected Béziers are exact).
fn circle_d(c: P3, u: P3, v: P3, r: f64) -> String {
    let k = 0.552_284_749_830_793_4 * r;
    let at = |t: f64| add3(add3(c, u, r * t.cos()), v, r * t.sin());
    let dir = |t: f64| add3(add3([0.0; 3], u, -t.sin()), v, t.cos());
    let start = project(at(0.0));
    let mut d = format!("M{} {}", num(start[0]), num(start[1]));
    for i in 0..4 {
        let (t0, t1) = (i as f64 * FRAC_PI_2, (i + 1) as f64 * FRAC_PI_2);
        let c1 = project(add3(at(t0), dir(t0), k));
        let c2 = project(add3(at(t1), dir(t1), -k));
        let p = project(at(t1));
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
    /// cube meets it there (those edges get the gap).
    corners: [P3; 4],
    shared: [bool; 4],
    /// The face's outward normal and two in-plane unit axes.
    normal: P3,
    axes: [P3; 2],
    tone: Tone,
}

fn faces() -> [Face; 3] {
    let shared = [false, true, true, false];
    [
        Face {
            name: "top",
            corners: [[-A, -A, A], [A, -A, A], [A, A, A], [-A, A, A]],
            shared,
            normal: [0.0, 0.0, 1.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            tone: Tone::Top,
        },
        Face {
            name: "left",
            corners: [[-A, A, -A], [A, A, -A], [A, A, A], [-A, A, A]],
            shared,
            normal: [0.0, 1.0, 0.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            tone: Tone::Left,
        },
        Face {
            name: "right",
            corners: [[A, -A, -A], [A, A, -A], [A, A, A], [A, -A, A]],
            shared,
            normal: [1.0, 0.0, 0.0],
            axes: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            tone: Tone::Right,
        },
    ]
}

fn ccw(poly: Vec<P2>) -> Vec<P2> {
    if signed_area(&poly) < 0.0 { poly.into_iter().rev().collect() } else { poly }
}

/// A face's outline on screen, inset at the shared edges by half the gap, wound
/// counter-clockwise (positive area).
fn outline_d(face: &Face, detail: Detail) -> Result<String, String> {
    let screen: Vec<P2> = face.corners.iter().map(|&p| project(p)).collect();
    Ok(polyline(&ccw(inset(&screen, &face.shared, detail.gap() / 2.0)?), true))
}

/// The fringe's band boundaries on a face: for each band radius that reaches the face's plane,
/// the band's colour and the circle (centre, radius) where the sphere around the hot point cuts
/// the plane, largest first.
fn bands(face: &Face) -> Vec<(&'static str, P3, f64)> {
    let dist = dot3(add3(HOT, face.corners[0], -1.0), face.normal);
    let centre = add3(HOT, face.normal, -dist);
    BAND_RADII.iter().zip(&FRINGE[1..]).filter(|(r, _)| **r > dist.abs()).map(|(r, colour)| (*colour, centre, (r * r - dist * dist).sqrt())).collect()
}

/// The hexahedral mesh's edges on the visible faces, as path data. The interior nodes are
/// shared, so lines run on across the face boundaries as a real structured mesh would.
fn mesh_d() -> String {
    let cs: Vec<f64> = (1..CELLS).map(|i| -A + 2.0 * A * i as f64 / CELLS as f64).collect();
    let mut segs: Vec<(P3, P3)> = Vec::new();
    for &c in &cs {
        // Top.
        segs.push(([c, -A, A], [c, A, A]));
        segs.push(([-A, c, A], [A, c, A]));
        // Left (y = A).
        segs.push(([c, A, -A], [c, A, A]));
        segs.push(([-A, A, c], [A, A, c]));
        // Right (x = A).
        segs.push(([A, c, -A], [A, c, A]));
        segs.push(([A, -A, c], [A, A, c]));
    }
    segs.iter().map(|&(a, b)| polyline(&[project(a), project(b)], false)).collect::<Vec<_>>().join(" ")
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

/// The floor grid on the ground plane under the cube, clipped to the backdrop's square.
fn floor_d() -> String {
    let step = 1.0 / 4.0;
    let mut parts = Vec::new();
    for k in -24..=24 {
        let c = A + k as f64 * step;
        for (a, b) in [([c, -8.0, -A], [c, 8.0, -A]), ([-8.0, c, -A], [8.0, c, -A])] {
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

fn colour(c: &str) -> Value {
    json!({"color": c})
}

/// One face of the cube: the coldest band filling it, the warmer bands as nested discs, and a
/// translucent shade that sets the face's tone, all clipped to the face.
fn face_layer(face: &Face, detail: Detail) -> Result<Layer, String> {
    let outline = outline_d(face, detail)?;
    let mut shapes = vec![Shape::filled(face.name, outline.clone(), colour(FRINGE[0]))];
    for (i, (c, centre, r)) in bands(face).into_iter().enumerate() {
        shapes.push(Shape::filled(&format!("{} band {}", face.name, i + 1), circle_d(centre, face.axes[0], face.axes[1], r), colour(c)));
    }
    let shade = match face.tone {
        Tone::Top => Some(("#ffffff", 0.1)),
        Tone::Left => Some(("#000000", 0.12)),
        Tone::Right => Some(("#000000", 0.3)),
    };
    if let Some((c, opacity)) = shade {
        let mut s = Shape::filled(&format!("{} shade", face.name), outline.clone(), colour(c));
        s.opacity = opacity;
        shapes.push(s);
    }
    let name = match face.tone {
        Tone::Top => "Top face",
        Tone::Left => "Left face",
        Tone::Right => "Right face",
    };
    Ok(Layer { name, shapes, clip: Some(outline) })
}

fn part_layers(detail: Detail) -> Result<Vec<Layer>, String> {
    faces().iter().map(|f| face_layer(f, detail)).collect()
}

fn mesh_layer() -> Result<Layer, String> {
    let clip = faces().iter().map(|f| outline_d(f, Detail::Full)).collect::<Result<Vec<_>, _>>()?.join(" ");
    let stroke = Stroke { paint: colour("#0b2236"), width: 3.5, cap: "butt" };
    Ok(Layer { name: "Mesh", shapes: vec![Shape::stroked("hexahedral mesh", mesh_d(), stroke, 0.45)], clip: Some(clip) })
}

fn floor_layer(appearance: Appearance) -> Layer {
    let colour = match appearance {
        Appearance::Light => "#8aa0b4",
        Appearance::Dark => "#4f6a86",
    };
    let fade = [(0.0, 1.0), (0.3, 0.9), (0.65, 0.35), (1.0, 0.0)];
    let stops: Vec<Value> = fade.iter().map(|(o, a)| json!({"offset": o, "color": colour, "opacity": a})).collect();
    let floor = project([0.0, 0.0, -A]);
    let paint = json!({"gradient": {"kind": "radial", "stops": stops, "start": floor, "end": [floor[0] + 560.0, floor[1]], "aspect": 50}});
    let stroke = Stroke { paint, width: 3.0, cap: "butt" };
    // Clipped to the backdrop, so the flattened icon's grid stops at its rounded corners (Icon
    // Composer masks its own layers the same way).
    Layer { name: "Floor grid", shapes: vec![Shape::stroked("floor grid", floor_d(), stroke, 1.0)], clip: Some(backdrop_d()) }
}

fn backdrop_layer(appearance: Appearance) -> Layer {
    let (colours, shadow) = match appearance {
        Appearance::Light => (["#fbfcfe", "#dce3ec"], "#1d2b3b"),
        Appearance::Dark => (["#2a303a", "#13171d"], "#000000"),
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
    layers.extend(part_layers(detail)?);
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
    let plain = |file: &str| {
        json!({"image-name": format!("{file}.svg"), "name": file, "position": position,
            "glass-specializations": [{"appearance": "tinted", "value": true}]})
    };
    json!({
        "fill": "system-light",
        "groups": [
            group("02 Part", plain("02-part"), true),
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
    // The mesh shares the cube's Icon Composer group.
    let single = |title, layer| Doc { title, layers: vec![layer] };
    let mut part_with_mesh = part_layers(Detail::Full)?;
    part_with_mesh.push(mesh_layer()?);
    let layer_docs = [
        ("01-floor", single("FieldCraft icon floor grid", floor_layer(Appearance::Light))),
        ("01-floor-dark", single("FieldCraft icon floor grid (Dark)", floor_layer(Appearance::Dark))),
        ("02-part", Doc { title: "FieldCraft icon cube", layers: part_with_mesh }),
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
    fn every_face_shows_every_band_hottest_last() {
        for f in faces() {
            let b = bands(&f);
            assert_eq!(b.len(), BAND_RADII.len(), "{}", f.name);
            assert!(b.windows(2).all(|w| w[0].2 > w[1].2), "{}", f.name);
            assert_eq!(b.last().map(|x| x.0), Some(FRINGE[FRINGE.len() - 1]));
        }
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
        // Floor, three faces, mesh.
        assert_eq!(steps.iter().filter(|s| s.0 == "layer.new").count(), 5);
        // The floor grid is clipped to the backdrop, each face's fringe and the mesh to the faces.
        assert_eq!(steps.iter().filter(|s| s.0 == "layer.clippingMask.toggle").count(), 5);
    }
}
