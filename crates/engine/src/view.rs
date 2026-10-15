//! Headless pictures of a session: the model, its geometry and the results display drawn with
//! the CPU rasterizer of [`fieldcraft_render`], with no window or GPU (the MCP server's
//! screenshots without an app, docs/mcp.md).
//!
//! The picture follows the desktop viewport's conventions: geometry draws as faces with their
//! boundaries while the model has no element faces, then as boundaries only; elements in their
//! kind's colour with their edges; a results view ([`crate::results::ResultsView`]) draws the
//! deformed shape (automatic or chosen scale) with its contour bands. Animation is not drawn: a
//! picture shows the full deformation (a mode at its positive shape). The analysis setup's
//! symbols ([`fieldcraft_render::glyphs`]) are drawn over the model as the viewport draws them,
//! muted over a results plot.

use fieldcraft_model::ElementKind;
use fieldcraft_render::contour::{Bands, band_color, split_triangle};
use fieldcraft_render::glyphs::{GlyphColors, GlyphShow, Glyphs};
use fieldcraft_render::{Aabb3, Camera, DisplayMesh, Orientation, Rgb, Scene, SceneLine, SceneTri, Vec3};
use serde_json::Value;

use crate::results::{displacements, field_values, magnitude, range};
use crate::{EngineError, Result, Session};

/// The colours of a picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Background gradient, top to bottom.
    pub background: (Rgb, Rgb),
    pub hex: Rgb,
    pub tet: Rgb,
    pub wedge: Rgb,
    pub shell: Rgb,
    pub line_element: Rgb,
    pub mesh_edge: Rgb,
    pub geometry: Rgb,
    pub geometry_edge: Rgb,
    /// The bodies of a design with several, in turn.
    pub bodies: [Rgb; 8],
    /// Faces of nodes without a value in a contour plot.
    pub no_result: Rgb,
    /// The contour colour scale, low to high.
    pub contour: [Rgb; 5],
    /// The analysis setup's symbols.
    pub glyphs: GlyphColors,
}

impl Palette {
    /// The desktop app's light theme.
    pub const LIGHT: Palette = Palette {
        background: (Rgb(0xf0, 0xf3, 0xf7), Rgb(0xc8, 0xd0, 0xdc)),
        hex: Rgb(0x78, 0x9c, 0xc0),
        tet: Rgb(0x80, 0xb4, 0x8a),
        wedge: Rgb(0xc0, 0xaa, 0x78),
        shell: Rgb(0xc0, 0x8c, 0x78),
        line_element: Rgb(0xc8, 0x8c, 0x1e),
        mesh_edge: Rgb(0x1e, 0x23, 0x2a),
        geometry: Rgb(0xb4, 0xba, 0xc4),
        geometry_edge: Rgb(0x2a, 0x30, 0x3a),
        bodies: [
            Rgb(0x8f, 0xb4, 0xd9),
            Rgb(0xe0, 0xa4, 0x6c),
            Rgb(0x8c, 0xc4, 0x8a),
            Rgb(0xd6, 0x8c, 0x96),
            Rgb(0xb0, 0x9c, 0xd8),
            Rgb(0xd8, 0xc8, 0x70),
            Rgb(0x7c, 0xc4, 0xc0),
            Rgb(0xc0, 0xa0, 0x84),
        ],
        no_result: Rgb(0x9a, 0xa0, 0xaa),
        contour: [Rgb(0x2b, 0x4f, 0xd8), Rgb(0x2f, 0xb8, 0xe0), Rgb(0x3c, 0xc8, 0x64), Rgb(0xf0, 0xd2, 0x3c), Rgb(0xe0, 0x40, 0x2e)],
        glyphs: GlyphColors {
            constraint: Rgb(0x00, 0x8a, 0xa8),
            load: Rgb(0xd0, 0x24, 0x24),
            interaction: Rgb(0x9a, 0x30, 0xc0),
            thermal: Rgb(0xe8, 0x4c, 0x00),
            label: Rgb(0x26, 0x2a, 0x32),
            datum: Rgb(0x1e, 0x8c, 0x6e),
        },
    };

    fn element(&self, kind: ElementKind) -> Rgb {
        match kind {
            ElementKind::Hex8 | ElementKind::Hex20 => self.hex,
            ElementKind::Tet4 | ElementKind::Tet10 => self.tet,
            ElementKind::Wedge6 => self.wedge,
            ElementKind::Tri3 | ElementKind::Tri6 | ElementKind::Quad4 | ElementKind::Quad8 => self.shell,
            ElementKind::Rod2 | ElementKind::Rod3 | ElementKind::Beam2 => self.line_element,
        }
    }
}

/// Mesh edge, geometry edge and line element widths, in output pixels.
const EDGE_WIDTH: f64 = 1.0;
const GEOMETRY_EDGE_WIDTH: f64 = 1.5;
const LINE_ELEMENT_WIDTH: f64 = 3.0;
/// The automatic scale draws the largest displacement this fraction of the model's size.
const AUTO_FRACTION: f64 = 0.1;

/// The chordal tolerance geometry is tessellated with for display: a thousandth of its size.
fn display_tolerance(b: &Aabb3) -> f64 {
    let d = b.diagonal();
    if d.is_finite() { (d * 1e-3).clamp(1e-3, 1.0) } else { 0.1 }
}

/// What a picture shows besides the image.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// The solved step drawn, when the results display is on.
    pub step: Option<String>,
    /// The deformation scale factor drawn (0: undeformed).
    pub scale: f64,
    /// The contour field drawn, with its smallest and largest value and their nodes.
    pub contour: Option<(String, (u32, f64), (u32, f64))>,
    /// Bodies that could not be tessellated for display.
    pub failed: Vec<String>,
    /// Definitions whose symbols are a sample (`Clamp: 400 of 12000 nodes`).
    pub sampled: Vec<String>,
}

/// The model's display data with the open design's bodies and the imported surfaces.
pub fn display(s: &Session) -> (DisplayMesh, Vec<String>) {
    let mut d = DisplayMesh::from_model(&s.model);
    let mut failed = Vec::new();
    for body in s.design.iter().flat_map(|d| &d.bodies) {
        let mesh = body.measure().and_then(|m| body.tessellate(display_tolerance(&Aabb3 { min: m.bbox_min.into(), max: m.bbox_max.into() })));
        match mesh {
            Ok(m) => d.add_surface(&m.positions, &m.triangles, &m.tri_face),
            Err(e) => failed.push(format!("{}: {e}", body.name)),
        }
    }
    for surface in &s.surfaces {
        d.add_surface(&surface.mesh.positions, &surface.mesh.triangles, &surface.mesh.tri_face);
    }
    (d, failed)
}

/// The model drawn as the viewport draws it (shaded with element edges).
fn model_scene(d: &DisplayMesh, p: &Palette) -> Scene {
    let mut s = Scene { background: Some(p.background), ..Default::default() };
    s.tris = d.tris.iter().map(|t| SceneTri { p: t.p, normal: t.normal, color: p.element(t.kind) }).collect();
    let faces_drawn = d.tris.is_empty();
    if faces_drawn {
        s.tris.extend(d.surface_tris.iter().map(|t| SceneTri { p: t.p, normal: t.normal, color: d.body_color(t, p.geometry, &p.bodies) }));
    }
    s.lines = d.edges.iter().map(|e| SceneLine { a: e.p[0], b: e.p[1], color: p.mesh_edge, width: EDGE_WIDTH }).collect();
    if faces_drawn {
        s.lines.extend(d.surface_edges.iter().map(|[a, b]| SceneLine { a: *a, b: *b, color: p.mesh_edge, width: GEOMETRY_EDGE_WIDTH }));
    } else {
        // Over the mesh the geometry's edges are drawn along it (the element edges on them), so
        // they lie on the element faces.
        let edge = p.geometry_edge;
        s.lines.extend(d.edges.iter().filter(|e| e.feature).map(|e| SceneLine { a: e.p[0], b: e.p[1], color: edge, width: GEOMETRY_EDGE_WIDTH }));
    }
    s.lines.extend(d.lines.iter().map(|l| SceneLine { a: l.a, b: l.b, color: p.element(l.kind), width: LINE_ELEMENT_WIDTH }));
    s
}

/// The scene of the session: the model, or the results view's plot, and what it shows.
pub fn scene(s: &Session, p: &Palette) -> (Scene, Aabb3, Shown) {
    let (base, failed) = display(s);
    let glyphs = Glyphs::from_model(&s.model);
    let mut shown = Shown { failed, sampled: glyphs.notes.clone(), ..Shown::default() };
    let plot = s.results_view.as_ref().and_then(|v| s.latest_result(&v.step).map(|(r, _)| (v, r)));
    let Some((view, result)) = plot else {
        let mut bounds = base.bounds;
        let mut scene = model_scene(&base, p);
        bounds.union(glyphs.draw(|label| base.node_position(label), GlyphShow::ALL, &p.glyphs, &mut scene));
        bounds.union(fieldcraft_render::glyphs::datums(&s.model, &base.bounds, &p.glyphs, &mut scene));
        return (scene, bounds, shown);
    };
    shown.step = Some(view.step.clone());
    let pick = view.pick();
    let nodes = result.nodes();
    let u = displacements(result, pick).unwrap_or(&[]);
    let scale = if !view.deformed {
        0.0
    } else if let Some(k) = view.scale {
        k
    } else {
        let largest = u.iter().map(|v| magnitude(*v)).filter(|m| m.is_finite()).fold(0.0, f64::max);
        let size = base.bounds.diagonal();
        if largest > 0.0 && size.is_finite() && size > 0.0 { AUTO_FRACTION * size / largest } else { 0.0 }
    };
    shown.scale = scale;
    let drawn = base.displaced(|label| {
        let i = nodes.binary_search(&label).ok()?;
        u.get(i).map(|d| Vec3::from(*d) * scale)
    });
    let mut scene = model_scene(&drawn, p);
    let contour = view.contour.and_then(|field| {
        let values = field_values(&s.model, result, field, pick).ok()?;
        let (min, max) = range(nodes, &values)?;
        Some((field, min, max, values))
    });
    if let Some((field, min, max, values)) = contour {
        shown.contour = Some((field.id().to_string(), min, max));
        let bands = Bands::new(min.1, max.1, view.bands);
        let at = |label: u32| nodes.binary_search(&label).ok().and_then(|i| values.get(i).copied()).unwrap_or(f64::NAN);
        scene.tris.clear();
        for tri in &drawn.tris {
            split_triangle(tri.p, tri.nodes.map(at), &bands, &mut |piece, band| {
                let color = band.map_or(p.no_result, |k| band_color(&p.contour, k, bands.count));
                for k in 1..piece.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c)) = (piece.first(), piece.get(k), piece.get(k + 1)) {
                        scene.tris.push(SceneTri { p: [a, b, c], normal: tri.normal, color });
                    }
                }
            });
        }
    }
    let symbols = glyphs.draw(|label| drawn.node_position(label), GlyphShow::ALL, &p.glyphs.over_results(p.background.1), &mut scene);
    let mut bounds = base.bounds;
    bounds.union(drawn.bounds);
    bounds.union(symbols);
    (scene, bounds, shown)
}

/// A camera orientation from JSON, `{"yaw": …, "pitch": …, "distance"?: …}`: degrees, and the
/// distance from the target in model units (the control channel's `ui.camera`, the MCP
/// `screenshot` tool's `camera`). Hostile input is refused or clamped
/// ([`Orientation::checked`]).
pub fn orientation(p: &Value) -> Result<Orientation> {
    let bad = |m: &str| EngineError::Params(format!("camera: {m}"));
    let o = p.as_object().ok_or_else(|| bad("an object {yaw, pitch, distance?} in degrees"))?;
    if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "yaw" | "pitch" | "distance")) {
        return Err(bad(&format!("unknown field `{}` (yaw, pitch, distance)", k.chars().take(40).collect::<String>())));
    }
    let number = |k: &str| o.get(k).filter(|v| !v.is_null()).map(|v| v.as_f64().ok_or_else(|| bad(&format!("`{k}` must be a number"))));
    let yaw = number("yaw").ok_or_else(|| bad("`yaw` (degrees) is required"))??;
    let pitch = number("pitch").ok_or_else(|| bad("`pitch` (degrees) is required"))??;
    let distance = number("distance").transpose()?;
    Orientation::checked(yaw, pitch, distance).ok_or_else(|| bad("`yaw` and `pitch` must be finite and `distance` finite and above 0"))
}

/// A PNG of the session seen from `at`, `w` × `h` pixels: framing everything drawn, or from
/// `at`'s distance when it has one.
pub fn render_png(s: &Session, at: &Orientation, w: usize, h: usize, p: &Palette) -> Result<(Vec<u8>, Shown)> {
    let (mut scene, bounds, shown) = scene(s, p);
    let mut cam = Camera::default();
    cam.orient(&Orientation { distance: None, ..*at });
    cam.fit(&bounds);
    cam.orient(at);
    scene.radius = if bounds.is_empty() { 1.0 } else { (cam.target.dist(bounds.center()) + bounds.diagonal() * 0.5).max(1e-3) };
    let png = fieldcraft_render::render_png(&scene, &cam, w, h).map_err(|e| EngineError::Params(format!("render: {e}")))?;
    Ok((png, shown))
}

#[cfg(test)]
mod tests {
    use fieldcraft_render::StandardView;
    use serde_json::json;

    use super::*;

    #[test]
    fn renders_the_model_and_its_results() {
        let iso = Orientation::of(StandardView::Iso);
        let mut s = Session::default();
        let empty = render_png(&s, &iso, 32, 24, &Palette::LIGHT).unwrap();
        assert!(empty.0.starts_with(&[0x89, b'P', b'N', b'G']));
        s.run_script(&crate::sample::cantilever()).unwrap();
        let (model, shown) = render_png(&s, &iso, 64, 48, &Palette::LIGHT).unwrap();
        assert_eq!(shown, Shown::default());
        // The clamp's cones and the push's arrows are drawn in their colours.
        let (scene, ..) = super::scene(&s, &Palette::LIGHT);
        assert!(scene.tris.iter().any(|t| t.color == Palette::LIGHT.glyphs.constraint));
        assert!(scene.lines.iter().any(|l| l.color == Palette::LIGHT.glyphs.load));
        s.execute("solve.static", &json!({})).unwrap();
        let (plot, shown) = render_png(&s, &iso, 64, 48, &Palette::LIGHT).unwrap();
        assert_ne!(model, plot);
        assert!(shown.scale > 0.0 && shown.step.is_some(), "{shown:?}");
        let (field, min, max) = shown.contour.unwrap();
        assert_eq!(field, "von_mises");
        assert!(max.1 > min.1);
        assert!(render_png(&s, &Orientation::of(StandardView::Top), 0, 48, &Palette::LIGHT).is_err());
        // Over the plot the symbols are muted.
        let (scene, ..) = super::scene(&s, &Palette::LIGHT);
        let muted = Palette::LIGHT.glyphs.over_results(Palette::LIGHT.background.1);
        assert!(scene.tris.iter().any(|t| t.color == muted.constraint));
        assert!(!scene.tris.iter().any(|t| t.color == Palette::LIGHT.glyphs.constraint));
    }

    #[test]
    fn camera_orientation_is_checked() {
        let o = orientation(&json!({"yaw": -45, "pitch": 30})).unwrap();
        assert_eq!((o.yaw, o.pitch, o.distance), (-45.0, 30.0, None));
        let o = orientation(&json!({"yaw": 400, "pitch": -400, "distance": 1e300})).unwrap();
        assert_eq!((o.yaw, o.pitch, o.distance), (40.0, -90.0, Some(Orientation::DISTANCE.1)));
        for hostile in [
            json!(null),
            json!([1, 2]),
            json!({}),
            json!({"yaw": 0}),
            json!({"pitch": 0}),
            json!({"yaw": "left", "pitch": 0}),
            json!({"yaw": 0, "pitch": 0, "distance": 0}),
            json!({"yaw": 0, "pitch": 0, "distance": -5}),
            json!({"yaw": 0, "pitch": 0, "distance": "far"}),
            json!({"yaw": 0, "pitch": 0, "roll": 3}),
            json!({"yaw": f64::INFINITY, "pitch": 0}),
        ] {
            assert!(orientation(&hostile).is_err(), "{hostile}");
        }
        // A picture from a chosen orientation and distance differs from the framed iso one.
        let mut s = Session::default();
        s.run_script(&crate::sample::cantilever()).unwrap();
        let iso = render_png(&s, &Orientation::of(StandardView::Iso), 64, 48, &Palette::LIGHT).unwrap().0;
        let side = render_png(&s, &orientation(&json!({"yaw": -80, "pitch": 10, "distance": 2000})).unwrap(), 64, 48, &Palette::LIGHT).unwrap().0;
        assert_ne!(iso, side);
    }
}
