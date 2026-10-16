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
//! muted over a results plot (and only those its step used), at their default size on screen
//! once the camera is known.

use fieldcraft_model::ElementKind;
use fieldcraft_render::contour::{bands_color, split_triangle};
use fieldcraft_render::glyphs::{self, GlyphColors, GlyphShow, GlyphView, Glyphs};
use fieldcraft_render::{Aabb3, Camera, DisplayMesh, Orientation, Rgb, Scene, SceneLine, SceneTri, Vec3};
use serde_json::Value;

use crate::results::{auto_scale, displacements, field_values, range};
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
    /// Values above a clamped legend's maximum.
    pub contour_over: Rgb,
    /// The elements `mesh.quality.mark` marked.
    pub marked: Rgb,
    /// The analysis setup's symbols.
    pub glyphs: GlyphColors,
}

/// Values above a clamped legend's maximum (both themes): a deep purple apart from the scale's red.
pub const CONTOUR_OVER: Rgb = Rgb(0x6e, 0x2c, 0x80);

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
        contour_over: CONTOUR_OVER,
        marked: Rgb(0xe0, 0x2e, 0x2e),
        glyphs: GlyphColors {
            constraint: Rgb(0x00, 0x8a, 0xa8),
            load: Rgb(0xd0, 0x24, 0x24),
            interaction: Rgb(0x9a, 0x30, 0xc0),
            thermal: Rgb(0xe8, 0x4c, 0x00),
            label: Rgb(0x26, 0x2a, 0x32),
            datum: Rgb(0x1e, 0x8c, 0x6e),
            contact: Rgb(0x6a, 0x8a, 0x00),
            highlight: Rgb(0xd6, 0x00, 0x7f),
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

/// The chordal tolerance geometry is tessellated with for display: a thousandth of its size.
fn display_tolerance(b: &Aabb3) -> f64 {
    let d = b.diagonal();
    if d.is_finite() { (d * 1e-3).clamp(1e-3, 1.0) } else { 0.1 }
}

/// The findings of a `geometry.check`, to draw over the design it checked.
#[derive(Clone, Debug, Default)]
pub struct GeometryMarks {
    /// The design checked: the marks are stale once another is open.
    pub design: std::sync::Weak<fieldcraft_geometry::Design>,
    pub marks: Vec<GeometryMark>,
}

/// One finding to draw: its faces, its edge and the point to look at.
#[derive(Clone, Debug, PartialEq)]
pub struct GeometryMark {
    /// The faces by body name and face index.
    pub faces: Vec<(String, u32)>,
    /// A short edge's polyline (empty for the other kinds).
    pub path: Vec<[f64; 3]>,
    pub at: [f64; 3],
}

/// The findings of the last `geometry.check`, while the design it checked is still open.
pub fn geometry_marks(s: &Session) -> Option<&GeometryMarks> {
    let marks = s.geometry_marks.as_deref()?;
    let design = s.design.as_ref()?;
    std::sync::Weak::ptr_eq(&marks.design, &std::sync::Arc::downgrade(design)).then_some(marks)
}

/// Marker line width, and the marker dot's diameter, in output pixels.
const MARK_WIDTH: f64 = 4.0;
const MARK_DOT: f64 = 12.0;

/// Draw `marks` over `scene` in `color`: the faces they name recoloured when `faces` (the
/// scene's triangles start with `display`'s surface triangles in order, as they do while no
/// element faces are drawn; `bodies` names the drawn surfaces in order), short edges as thick
/// lines and a dot at each finding's point, both over everything so a feature too small to see
/// at the zoom, or inside the mesh, still shows where it is.
pub fn draw_geometry_marks(scene: &mut Scene, display: &DisplayMesh, bodies: &[&str], marks: &GeometryMarks, faces: bool, color: Rgb) {
    if faces {
        let marked: std::collections::BTreeSet<(u32, u32)> = marks
            .marks
            .iter()
            .flat_map(|m| &m.faces)
            .filter_map(|(body, f)| bodies.iter().position(|b| b == body).and_then(|b| u32::try_from(b).ok()).map(|b| (b, *f)))
            .collect();
        for (t, d) in scene.tris.iter_mut().zip(&display.surface_tris) {
            if marked.contains(&(d.body, d.face)) {
                t.color = color;
            }
        }
    }
    for m in &marks.marks {
        for w in m.path.windows(2) {
            if let [a, b] = w {
                scene.on_top.push(SceneLine { a: Vec3::from(*a), b: Vec3::from(*b), color, width: MARK_WIDTH });
            }
        }
        let at = Vec3::from(m.at);
        scene.on_top.push(SceneLine { a: at, b: at, color, width: MARK_DOT });
    }
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

/// A section through the mesh: the elements whose centroid lies on the camera's side of a plane
/// normal to an axis are not drawn, so the element faces inside show (an element cut: whole elements, not
/// sliced ones). Geometry surfaces are not drawn over a sectioned mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    /// 0, 1, 2: the plane is normal to X, Y, Z.
    pub axis: usize,
    /// Where the plane crosses the axis; `None` at the middle of the mesh.
    pub at: Option<f64>,
}

impl Section {
    /// `x`, `y`, `z` (the middle of the mesh) or `x=12.5`.
    pub fn parse(s: &str) -> Option<Section> {
        let (axis, at) = match s.split_once('=') {
            Some((a, v)) => (a, Some(v.trim().parse::<f64>().ok().filter(|v| v.is_finite())?)),
            None => (s, None),
        };
        let axis = ["x", "y", "z"].iter().position(|n| n.eq_ignore_ascii_case(axis.trim()))?;
        Some(Section { axis, at })
    }

    /// The elements on the side of the plane `toward` points to (the direction toward the eye).
    fn beyond(&self, m: &fieldcraft_model::Model, toward: Vec3) -> std::collections::BTreeSet<u32> {
        let sign = match [toward.x, toward.y, toward.z].get(self.axis) {
            Some(v) if *v < 0.0 => -1.0,
            _ => 1.0,
        };
        let coord = |label: &u32| m.node(*label).and_then(|n| n.xyz.get(self.axis).copied());
        let centroid = |nodes: &[u32]| {
            let v: Vec<f64> = nodes.iter().filter_map(coord).collect();
            (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
        };
        let at = self.at.unwrap_or_else(|| {
            let (lo, hi) = m
                .nodes()
                .values()
                .filter_map(|n| n.xyz.get(self.axis).copied())
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
            if lo <= hi { (lo + hi) * 0.5 } else { 0.0 }
        });
        m.elements().iter().filter(|(_, e)| centroid(&e.nodes).is_some_and(|c| (c - at) * sign > 0.0)).map(|(l, _)| *l).collect()
    }
}

/// The model's display data with the open design's bodies and the imported surfaces.
pub fn display(s: &Session) -> (DisplayMesh, Vec<String>) {
    display_cut(s, None, Vec3::Z)
}

/// [`display`] with a section through the mesh, cut away on the side `toward` points to.
pub fn display_cut(s: &Session, section: Option<Section>, toward: Vec3) -> (DisplayMesh, Vec<String>) {
    let mut hidden = s.hidden_elements();
    let cut = section.filter(|_| !s.model.elements().is_empty());
    if let Some(c) = cut {
        hidden.extend(c.beyond(&s.model, toward));
    }
    let mut d = DisplayMesh::from_model_hiding(&s.model, &hidden);
    let mut failed = Vec::new();
    if cut.is_some() {
        return (d, failed);
    }
    for body in s.design.iter().flat_map(|d| &d.bodies).filter(|b| s.shows(fieldcraft_io::ItemKind::Body, &b.name)) {
        let mesh = body.measure().and_then(|m| body.tessellate(display_tolerance(&Aabb3 { min: m.bbox_min.into(), max: m.bbox_max.into() })));
        match mesh {
            Ok(m) => d.add_surface(&m.positions, &m.triangles, &m.tri_face),
            Err(e) => failed.push(format!("{}: {e}", body.name)),
        }
    }
    for surface in s.surfaces.iter().filter(|x| s.shows(fieldcraft_io::ItemKind::Surface, &x.name)) {
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

/// The elements `mesh.quality.mark` marked (its set), drawn in the marker colour.
pub fn marked(s: &Session) -> std::collections::BTreeSet<u32> {
    s.model.set(crate::cmd::mesh::MARKED_SET).map(|set| set.elements.iter().copied().collect()).unwrap_or_default()
}

/// Colour the faces of the `marked` elements in `scene`, whose triangles start with the element
/// faces of `display` in order (as [`model_scene`] and the app's viewport build them).
pub fn mark(scene: &mut Scene, display: &DisplayMesh, marked: &std::collections::BTreeSet<u32>, color: Rgb) {
    if marked.is_empty() {
        return;
    }
    for (t, d) in scene.tris.iter_mut().zip(&display.tris) {
        if marked.contains(&d.element) {
            t.color = color;
        }
    }
}

/// The model part of a picture: the model or the results view's plot, the shape the symbols sit
/// on (the deformed one of a plot), the box around it and what it shows.
struct Picture {
    scene: Scene,
    drawn: DisplayMesh,
    bounds: Aabb3,
    shown: Shown,
    glyphs: Glyphs,
    colors: GlyphColors,
    /// The box around the datums drawn: inside the clipping radius, never framed (#295).
    datums: Aabb3,
}

impl Picture {
    /// The setup's symbols at symbol unit `unit` ([`glyphs::symbol_unit`]), in a scene of their
    /// own; notes the definitions drawn with a sample. Returns the box around them.
    fn symbols(&mut self, unit: f64, into: &mut Scene) -> Aabb3 {
        let drawn = &self.drawn;
        let view = GlyphView { show: GlyphShow::ALL, colors: &self.colors, unit, highlight: None };
        let d = self.glyphs.draw(|label| drawn.node_position(label), &view, into);
        self.shown.sampled = d.notes;
        d.bounds
    }
}

fn picture(s: &Session, p: &Palette, section: Option<Section>, toward: Vec3) -> Picture {
    let (base, failed) = display_cut(s, section, toward);
    let mut shown = Shown { failed, ..Shown::default() };
    let plot = s.results_view.as_ref().and_then(|v| s.latest_result(&v.step).map(|(r, _)| (v, r)));
    // Over a results plot, the symbols of what its step used.
    let glyphs = plot.map_or_else(|| s.glyphs(), |(v, _)| s.step_glyphs(&v.step));
    let Some((view, result)) = plot else {
        let mut scene = model_scene(&base, p);
        mark(&mut scene, &base, &marked(s), p.marked);
        if let Some(marks) = geometry_marks(s) {
            let bodies: Vec<&str> = s
                .design
                .iter()
                .flat_map(|d| &d.bodies)
                .filter(|b| s.shows(fieldcraft_io::ItemKind::Body, &b.name))
                .map(|b| b.name.as_str())
                .collect();
            draw_geometry_marks(&mut scene, &base, &bodies, marks, base.tris.is_empty(), p.marked);
        }
        let bounds = base.bounds;
        let shows = |n: &str| s.shows(fieldcraft_io::ItemKind::Datum, n);
        let datums = fieldcraft_render::glyphs::datums(&s.model, &base.bounds, &shows, None, &p.glyphs, &mut scene);
        return Picture { scene, drawn: base, bounds, shown, glyphs, colors: p.glyphs, datums };
    };
    shown.step = Some(view.step.clone());
    let pick = view.pick();
    let nodes = result.nodes();
    let u = displacements(result, pick).unwrap_or(&[]);
    let scale = if !view.deformed { 0.0 } else { view.scale.unwrap_or_else(|| auto_scale(result, pick, base.bounds.diagonal())) };
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
        let bands = view.bands(min.1, max.1);
        let at = |label: u32| nodes.binary_search(&label).ok().and_then(|i| values.get(i).copied()).unwrap_or(f64::NAN);
        scene.tris.clear();
        for tri in &drawn.tris {
            split_triangle(tri.p, tri.nodes.map(at), &bands, &mut |piece, band| {
                let color = band.map_or(p.no_result, |k| bands_color(&p.contour, p.contour_over, &bands, k));
                for k in 1..piece.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c)) = (piece.first(), piece.get(k), piece.get(k + 1)) {
                        scene.tris.push(SceneTri { p: [a, b, c], normal: tri.normal, color });
                    }
                }
            });
        }
    }
    let mut bounds = base.bounds;
    bounds.union(drawn.bounds);
    Picture { scene, drawn, bounds, shown, glyphs, colors: p.glyphs.over_results(p.background.1), datums: Aabb3::EMPTY }
}

/// The scene of the session (the model, or the results view's plot) with the setup's symbols
/// at the model's own symbol unit, the box around it, and what it shows.
pub fn scene(s: &Session, p: &Palette) -> (Scene, Aabb3, Shown) {
    let mut pic = picture(s, p, None, Vec3::Z);
    let mut symbols = Scene::default();
    let b = pic.symbols(pic.glyphs.model_unit(), &mut symbols);
    let mut scene = pic.scene;
    scene.append(symbols);
    pic.bounds.union(b);
    (scene, pic.bounds, pic.shown)
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

/// A picture ready to draw: the scene (with its background and clipping radius), the setup's
/// symbols as a layer over it, and the camera.
pub struct Framed {
    pub scene: Scene,
    pub symbols: Scene,
    pub cam: Camera,
    pub shown: Shown,
}

/// The session seen from `at` in a `w` × `h` pixel picture: framing everything drawn but the
/// datums (they are drawn at the model's extent, and framing them would only add margin), or from
/// `at`'s distance when it has one. The symbols are sized for their default size on screen
/// ([`glyphs::DEFAULT_SIZE`] pixels) for that camera. The CPU rasterizer ([`render_png`]) and
/// the GPU (`fieldcraft-cli snapshot`) draw the same [`Framed`]. A `section` draws the mesh cut
/// ([`Section`]).
pub fn frame(s: &Session, at: &Orientation, w: usize, h: usize, p: &Palette, section: Option<Section>) -> Framed {
    let mut cam = Camera::default();
    cam.orient(at);
    let mut pic = picture(s, p, section, cam.back());
    let aspect = w as f64 / h.max(1) as f64;
    let unit = |cam: &Camera| glyphs::unit_for(cam, h as f64, glyphs::DEFAULT_SIZE, 1.0).unwrap_or(f64::NAN);
    // Framed on the model, then again with its symbols at that camera's size, which reach a
    // little beyond it.
    cam.orient(&Orientation { distance: None, ..*at });
    cam.fit_in(&pic.bounds, aspect, fieldcraft_render::FULL_VIEW);
    let mut symbols = Scene::default();
    let mut framed = pic.bounds;
    framed.union(pic.symbols(unit(&cam), &mut symbols));
    cam.fit_in(&framed, aspect, fieldcraft_render::FULL_VIEW);
    cam.orient(at);
    symbols = Scene::default();
    let mut bounds = pic.bounds;
    bounds.union(pic.symbols(unit(&cam), &mut symbols));
    bounds.union(pic.datums);
    pic.scene.radius = if bounds.is_empty() { 1.0 } else { (cam.target.dist(bounds.center()) + bounds.diagonal() * 0.5).max(1e-3) };
    Framed { scene: pic.scene, symbols, cam, shown: pic.shown }
}

/// A PNG of the session seen from `at`, `w` × `h` pixels ([`frame`]), drawn on the CPU.
pub fn render_png(s: &Session, at: &Orientation, w: usize, h: usize, p: &Palette) -> Result<(Vec<u8>, Shown)> {
    let f = frame(s, at, w, h, p, None);
    let png = fieldcraft_render::render_layers_png(&f.scene, &[&f.symbols], &f.cam, w, h).map_err(|e| EngineError::Params(format!("render: {e}")))?;
    Ok((png, f.shown))
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
        // The clamp's triangles and the push's arrows are drawn in their colours, as wireframes.
        let (scene, ..) = super::scene(&s, &Palette::LIGHT);
        assert!(scene.lines.iter().any(|l| l.color == Palette::LIGHT.glyphs.constraint));
        assert!(scene.lines.iter().any(|l| l.color == Palette::LIGHT.glyphs.load));
        assert!(!scene.tris.iter().any(|t| t.color == Palette::LIGHT.glyphs.constraint));
        // And in the picture: pixels of the clamp's colour.
        let (png, _) = render_png(&s, &iso, 320, 240, &Palette::LIGHT).unwrap();
        assert!(count_color(&png, Palette::LIGHT.glyphs.constraint) > 20);
        s.execute("solve.static", &json!({})).unwrap();
        let (plot, shown) = render_png(&s, &iso, 64, 48, &Palette::LIGHT).unwrap();
        assert_ne!(model, plot);
        // A static step first shows its contours on the undeformed shape.
        assert!(shown.scale == 0.0 && shown.step.is_some(), "{shown:?}");
        let (field, min, max) = shown.contour.unwrap();
        assert_eq!(field, "von_mises");
        assert!(max.1 > min.1);
        assert!(render_png(&s, &Orientation::of(StandardView::Top), 0, 48, &Palette::LIGHT).is_err());
        // Over the plot the symbols are muted.
        let (scene, ..) = super::scene(&s, &Palette::LIGHT);
        let muted = Palette::LIGHT.glyphs.over_results(Palette::LIGHT.background.1);
        assert!(scene.lines.iter().any(|l| l.color == muted.constraint));
        assert!(!scene.lines.iter().any(|l| l.color == Palette::LIGHT.glyphs.constraint));
    }

    /// Pixels of colour `c` (within a few levels) in a PNG.
    fn count_color(png: &[u8], c: Rgb) -> usize {
        let d = png::Decoder::new(std::io::Cursor::new(png));
        let mut r = d.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        let close = |a: u8, b: u8| a.abs_diff(b) <= 6;
        buf[..info.buffer_size()].chunks(4).filter(|p| close(p[0], c.0) && close(p[1], c.1) && close(p[2], c.2)).count()
    }

    /// The symbols are as large on screen in a close-up as in a distant view.
    #[test]
    fn symbols_keep_their_size_in_pictures_at_any_distance() {
        let mut s = Session::default();
        s.run_script(&crate::sample::cantilever()).unwrap();
        let at = |d: f64| orientation(&json!({"yaw": -45, "pitch": 30, "distance": d})).unwrap();
        let clamp = |d: f64| count_color(&render_png(&s, &at(d), 400, 300, &Palette::LIGHT).unwrap().0, Palette::LIGHT.glyphs.constraint);
        let (near, far) = (clamp(120.0), clamp(1200.0));
        // Zoomed out ten times, the symbols are as large (a few overlap and thin out).
        assert!(far > 30 && far * 4 > near, "near {near}, far {far}");
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
