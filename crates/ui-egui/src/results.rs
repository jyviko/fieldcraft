//! The results display in the viewport: what the engine's [`ResultsView`] asks for, drawn on the
//! model's display data. The deformed shape moves every element by its nodes' displacements
//! (or a mode shape) times the scale; contours split the faces into flat-coloured bands
//! ([`modelcraft_render::contour`]); animation steps the deformation through a fixed number of
//! frames per cycle. A plot of a stale result (the model changed since the solve) is drawn the
//! same way and marked.

use std::sync::Arc;

use modelcraft_engine::Session;
use modelcraft_engine::results::{Field, ResultsView, displacements, field_values, magnitude, range};
use modelcraft_render::contour::{Bands, band_color, split_triangle};
use modelcraft_render::{DisplayMesh, Rgb, Scene, SceneLine, ScenePoint, SceneTri, Vec3};
use modelcraft_solver::StepResult;

use crate::Style;
use crate::theme::{Tokens, rgb};

/// Animation frames per cycle, and the cycle's length in seconds.
pub const FRAMES: u64 = 24;
pub const PERIOD: f64 = 2.0;
/// The automatic scale draws the largest displacement this fraction of the model's size.
const AUTO_FRACTION: f64 = 0.1;
const EDGE_WIDTH: f64 = 1.0;
const LINE_ELEMENT_WIDTH: f64 = 3.0;
const NODE_SIZE: f64 = 5.0;

/// The legend of a contour plot.
#[derive(Clone, Debug, PartialEq)]
pub struct Legend {
    pub field: Field,
    pub bands: Bands,
    /// The smallest and largest value and their nodes.
    pub min: (u32, f64),
    pub max: (u32, f64),
}

/// A results plot ready to draw.
#[derive(Clone, Debug)]
pub struct Plot {
    pub view: ResultsView,
    pub result: Arc<StepResult>,
    /// Whether the model changed since the step was solved.
    pub stale: bool,
    /// The scale factor the deformation is drawn with (0: undeformed), before animation.
    pub scale: f64,
    /// The elements as drawn (deformed at the current animation frame).
    pub display: DisplayMesh,
    pub legend: Option<Legend>,
    /// The contour value at every node, in the result's node order.
    pub values: Option<Vec<f64>>,
}

impl Plot {
    /// A frequency step's mode shown, and its frequency.
    pub fn mode(&self) -> Option<(usize, f64)> {
        match self.result.as_ref() {
            StepResult::Frequency(f) => f.modes.get(self.view.mode.checked_sub(1)?).map(|m| (self.view.mode, m.frequency)),
            StepResult::Static(_) => None,
        }
    }

    /// The node's position in the plot, if it is drawn.
    pub fn node_position(&self, label: u32) -> Option<Vec3> {
        self.display.nodes.binary_search_by_key(&label, |n| n.label).ok().and_then(|i| self.display.nodes.get(i)).map(|n| n.p)
    }
}

/// The result the session's view shows, and whether it is current.
pub fn shown(session: &Session) -> Option<(&ResultsView, &Arc<StepResult>, bool)> {
    let view = session.results_view.as_ref()?;
    let (result, current) = session.latest_result(&view.step)?;
    Some((view, result, current))
}

/// The animation frame shown at time `now` (seconds), when animating.
pub fn frame(view: &ResultsView, now: f64) -> Option<u64> {
    (view.animate && view.deformed && now.is_finite()).then(|| ((now.max(0.0) / PERIOD * FRAMES as f64).floor() as u64) % FRAMES)
}

/// The deformation multiplier of an animation frame: a mode swings through ±1, a static step
/// grows from 0 to 1 and back.
pub fn swing(result: &StepResult, frame: Option<u64>) -> f64 {
    let Some(k) = frame else { return 1.0 };
    let a = std::f64::consts::TAU * k as f64 / FRAMES as f64;
    match result {
        StepResult::Frequency(_) => a.sin(),
        StepResult::Static(_) => 0.5 * (1.0 - a.cos()),
    }
}

/// The deformation scale factor: the view's, or automatic (the largest displacement a tenth of
/// the model's size); 0 when the view is undeformed or nothing moves.
pub fn scale(view: &ResultsView, result: &StepResult, base: &DisplayMesh) -> f64 {
    if !view.deformed {
        return 0.0;
    }
    if let Some(s) = view.scale {
        return s;
    }
    let largest = displacements(result, view.mode).map_or(0.0, |u| u.iter().map(|v| magnitude(*v)).filter(|m| m.is_finite()).fold(0.0, f64::max));
    let size = base.bounds.diagonal();
    if largest > 0.0 && size.is_finite() && size > 0.0 { AUTO_FRACTION * size / largest } else { 0.0 }
}

/// The plot of the session's results view on the model's display data, at animation `frame`.
pub fn plot(session: &Session, base: &DisplayMesh, frame: Option<u64>) -> Option<Plot> {
    let (view, result, current) = shown(session)?;
    let nodes = result.nodes();
    let scale = scale(view, result, base);
    let factor = scale * swing(result, frame);
    let u = displacements(result, view.mode).unwrap_or(&[]);
    let display = if factor != 0.0 {
        base.displaced(|label| {
            let i = nodes.binary_search(&label).ok()?;
            u.get(i).map(|d| Vec3::from(*d) * factor)
        })
    } else {
        base.displaced(|_| None)
    };
    let values = view.contour.and_then(|f| field_values(result, f, view.mode).ok().map(|v| (f, v)));
    let legend = values.as_ref().and_then(|(f, v)| {
        let (min, max) = range(nodes, v)?;
        Some(Legend { field: *f, bands: Bands::new(min.1, max.1, view.bands), min, max })
    });
    Some(Plot { view: view.clone(), result: result.clone(), stale: !current, scale, display, legend, values: values.map(|(_, v)| v) })
}

/// The scene of a plot: faces in contour bands (or element colours), the element edges on the
/// deformed shape, line elements and nodes.
pub fn scene(plot: &Plot, style: Style, show_nodes: bool, t: &Tokens) -> Scene {
    let d = &plot.display;
    let mut s = Scene { background: Some((rgb(t.viewport_top), rgb(t.viewport_bottom))), ..Default::default() };
    let nodes = plot.result.nodes();
    let value = |label: u32| -> f64 {
        let v = plot.values.as_deref().zip(nodes.binary_search(&label).ok()).and_then(|(v, i)| v.get(i).copied());
        v.unwrap_or(f64::NAN)
    };
    if style != Style::Wireframe {
        match (&plot.legend, &plot.values) {
            (Some(legend), Some(_)) => {
                let stops: Vec<Rgb> = t.contour.iter().map(|c| rgb(*c)).collect();
                for tri in &d.tris {
                    let v = tri.nodes.map(value);
                    split_triangle(tri.p, v, &legend.bands, &mut |piece, band| {
                        let color = band.map_or(rgb(t.no_result), |k| band_color(&stops, k, legend.bands.count));
                        for k in 1..piece.len().saturating_sub(1) {
                            if let (Some(&a), Some(&b), Some(&c)) = (piece.first(), piece.get(k), piece.get(k + 1)) {
                                s.tris.push(SceneTri { p: [a, b, c], normal: tri.normal, color });
                            }
                        }
                    });
                }
            }
            _ => s.tris = d.tris.iter().map(|tri| SceneTri { p: tri.p, normal: tri.normal, color: rgb(t.element(tri.kind)) }).collect(),
        }
    }
    if style != Style::Shaded {
        let color = rgb(if style == Style::Wireframe { t.hex } else { t.mesh_edge });
        s.lines = d.edges.iter().map(|e| SceneLine { a: e.p[0], b: e.p[1], color, width: EDGE_WIDTH }).collect();
    }
    s.lines.extend(d.lines.iter().map(|l| SceneLine { a: l.a, b: l.b, color: rgb(t.element(l.kind)), width: LINE_ELEMENT_WIDTH }));
    if show_nodes {
        s.points = d.nodes.iter().map(|n| ScenePoint { p: n.p, color: rgb(t.node), size: NODE_SIZE }).collect();
    }
    s
}

/// A number for the legend and probe: four significant digits, scientific when very large or
/// small.
pub fn number(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let a = v.abs();
    if a == 0.0 {
        "0".into()
    } else if !(1e-3..1e5).contains(&a) {
        format!("{v:.3e}")
    } else {
        let decimals = (3 - a.log10().floor() as i32).clamp(0, 6) as usize;
        format!("{v:.decimals$}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The sample cantilever, solved (static step `Load`, frequency step `Modes`).
    pub(crate) fn solved_cantilever() -> Session {
        let mut s = Session::default();
        s.run_script(&modelcraft_engine::sample::cantilever()).unwrap();
        s.execute("solve.static", &json!({})).unwrap();
        s.execute("solve.frequency", &json!({})).unwrap();
        s
    }

    /// The label of the node at `xyz`.
    fn node_at(s: &Session, xyz: [f64; 3]) -> u32 {
        *s.model.nodes().iter().find(|(_, n)| n.xyz == xyz).unwrap().0
    }

    #[test]
    fn the_deformed_shape_moves_by_the_scaled_displacement() {
        let mut s = solved_cantilever();
        let base = DisplayMesh::from_model(&s.model);
        assert!(plot(&s, &base, None).is_none(), "nothing shown before a results command");
        s.execute("results.deformed", &json!({"scale": 2.0})).unwrap();
        let p = plot(&s, &base, None).unwrap();
        assert_eq!(p.scale, 2.0);
        // A tip corner moves twice its displacement.
        let tip = node_at(&s, [100.0, 0.0, 0.0]);
        let StepResult::Static(r) = p.result.as_ref() else { panic!("a static result") };
        let u = r.displacement[r.nodes.binary_search(&tip).unwrap()];
        let at = p.node_position(tip).unwrap();
        assert!((at.z - 2.0 * u[2]).abs() < 1e-12 && u[2] < -0.2, "{at:?} {u:?}");
        // Automatic: the largest displacement is a tenth of the model's size.
        s.execute("results.deformed", &json!({"scale": "auto"})).unwrap();
        let p = plot(&s, &base, None).unwrap();
        let largest = r.displacement.iter().map(|v| magnitude(*v)).fold(0.0, f64::max);
        assert!((p.scale * largest / base.bounds.diagonal() - AUTO_FRACTION).abs() < 1e-12, "{}", p.scale);
        s.execute("results.deformed", &json!({"show": false})).unwrap();
        assert_eq!(plot(&s, &base, None).unwrap().display.tris, base.tris);
    }

    #[test]
    fn contours_colour_known_nodes_with_their_band() {
        let mut s = solved_cantilever();
        s.execute("results.contour", &json!({"field": "displacement_z", "bands": 4})).unwrap();
        s.execute("results.deformed", &json!({"show": false})).unwrap();
        let base = DisplayMesh::from_model(&s.model);
        let p = plot(&s, &base, None).unwrap();
        let legend = p.legend.clone().unwrap();
        // The tip moves most (lowest band), the clamped root not at all (highest band).
        assert!(legend.min.1 < -0.2 && legend.max.1.abs() < 1e-12, "{legend:?}");
        let t = Tokens::dark();
        let scene = scene(&p, Style::Shaded, false, &t);
        let stops: Vec<Rgb> = t.contour.iter().map(|c| rgb(*c)).collect();
        let (low, high) = (band_color(&stops, 0, 4), band_color(&stops, 3, 4));
        // Faces whose corners are all near the root are in the top band; near the tip, the bottom.
        let root_end = scene.tris.iter().filter(|tri| tri.p.iter().all(|q| q.x < 0.5)).map(|tri| tri.color).collect::<Vec<_>>();
        assert!(!root_end.is_empty() && root_end.iter().all(|c| *c == high), "{root_end:?}");
        let tip_end = scene.tris.iter().filter(|tri| tri.p.iter().all(|q| q.x > 99.5)).map(|tri| tri.color).collect::<Vec<_>>();
        assert!(!tip_end.is_empty() && tip_end.iter().all(|c| *c == low), "{tip_end:?}");
        // Every band appears, and the pieces still cover the faces (more triangles than before).
        for k in 0..4 {
            assert!(scene.tris.iter().any(|tri| tri.color == band_color(&stops, k, 4)), "band {k}");
        }
        assert!(scene.tris.len() > base.tris.len());
    }

    #[test]
    fn modes_swing_and_static_steps_grow_when_animated() {
        let mut s = solved_cantilever();
        s.execute("results.modes", &json!({"mode": 1})).unwrap();
        s.execute("results.animate", &json!({})).unwrap();
        let base = DisplayMesh::from_model(&s.model);
        let view = s.results_view.clone().unwrap();
        assert_eq!(frame(&view, 0.0), Some(0));
        assert_eq!(frame(&view, PERIOD * 0.25), Some(FRAMES / 4));
        assert_eq!(frame(&view, PERIOD * 1.25), Some(FRAMES / 4));
        let (_, result, _) = shown(&s).unwrap();
        assert!((swing(result, Some(FRAMES / 4)) - 1.0).abs() < 1e-12);
        assert!((swing(result, Some(3 * FRAMES / 4)) + 1.0).abs() < 1e-12);
        // At frame 0 a mode is undeformed; a quarter cycle later fully deformed.
        let p0 = plot(&s, &base, Some(0)).unwrap();
        let p1 = plot(&s, &base, Some(FRAMES / 4)).unwrap();
        assert_eq!(p0.display.tris, base.tris);
        assert_ne!(p1.display.tris, base.tris);
        assert_eq!(p1.mode().map(|(m, _)| m), Some(1));
        s.execute("results.deformed", &json!({"step": "Load"})).unwrap();
        let (_, bend, _) = shown(&s).unwrap();
        assert_eq!(swing(bend, Some(0)), 0.0);
        assert!((swing(bend, Some(FRAMES / 2)) - 1.0).abs() < 1e-12);
        assert_eq!(swing(bend, None), 1.0);
    }

    #[test]
    fn numbers_read_well() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(1.23456), "1.235");
        assert_eq!(number(-4.024), "-4.024");
        assert_eq!(number(123.456), "123.5");
        assert_eq!(number(1.5e-6), "1.500e-6");
        assert_eq!(number(2.5e7), "2.500e7");
        assert_eq!(number(f64::NAN), "—");
    }
}
