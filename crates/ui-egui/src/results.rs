//! The results display in the viewport: what the engine's [`ResultsView`] asks for, drawn on the
//! model's display data. The deformed shape moves every element by its nodes' displacements
//! (or a mode shape) times the scale; contours split the faces into flat-coloured bands
//! ([`modelcraft_render::contour`]); animation steps the deformation through a fixed number of
//! frames per cycle (an explicit step's animation runs through its field frames, contours
//! following). A plot of a stale result (the model changed since the solve) is drawn the
//! same way and marked.

use std::sync::Arc;

use modelcraft_engine::results::{Field, Kind, Pick, ResultsView, displacements, field_values, magnitude, range};
use modelcraft_engine::{Session, StepResult};
use modelcraft_render::contour::{Bands, band_color, split_triangle};
use modelcraft_render::{DisplayMesh, Scene, SceneTri, Vec3};

use crate::Style;
use crate::theme::{Tokens, rgb};

/// Animation frames per cycle, and the cycle's length in seconds.
pub const FRAMES: u64 = 24;
pub const PERIOD: f64 = 2.0;
/// The automatic scale draws the largest displacement this fraction of the model's size.
const AUTO_FRACTION: f64 = 0.1;

/// A contour plot: its field, bands, extremes and the value at every node.
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    pub field: Field,
    pub bands: Bands,
    /// The smallest and largest value and their nodes.
    pub min: (u32, f64),
    pub max: (u32, f64),
    /// The value at every node, in the result's node order.
    pub values: Vec<f64>,
}

impl Contour {
    /// The value at a node (NaN when it has none).
    pub fn at(&self, nodes: &[u32], label: u32) -> f64 {
        nodes.binary_search(&label).ok().and_then(|i| self.values.get(i).copied()).unwrap_or(f64::NAN)
    }
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
    pub contour: Option<Arc<Contour>>,
    /// The mode and field frame drawn.
    pub pick: Pick,
}

/// The part of a plot that does not change with the animation frame.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    pub scale: f64,
    pub contour: Option<Arc<Contour>>,
}

impl Plot {
    /// A frequency step's mode shown, and its frequency.
    pub fn mode(&self) -> Option<(usize, f64)> {
        match self.result.kind() {
            Kind::Frequency(f) => f.modes.get(self.pick.mode.checked_sub(1)?).map(|m| (self.pick.mode, m.frequency)),
            Kind::Static(_) | Kind::Explicit(_) | Kind::Fluid(_) | Kind::Heat(_) => None,
        }
    }

    /// An explicit or transient heat step's frame shown, its time and the number of frames.
    pub fn frame(&self) -> Option<(usize, f64, usize)> {
        let (n, time): (usize, &dyn Fn(usize) -> Option<f64>) = match self.result.kind() {
            Kind::Explicit(e) => (e.frames.len(), &|k| e.frames.get(k).map(|f| f.time)),
            Kind::Heat(h) if h.times.len() > 1 => (h.times.len(), &|k| h.times.get(k).copied()),
            _ => return None,
        };
        let k = self.pick.frame.unwrap_or(n.checked_sub(1)?);
        Some((k, time(k)?, n))
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
/// grows from 0 to 1 and back; an explicit step's frames are drawn as they are.
pub fn swing(result: &StepResult, frame: Option<u64>) -> f64 {
    let Some(k) = frame else { return 1.0 };
    let a = std::f64::consts::TAU * k as f64 / FRAMES as f64;
    match result.kind() {
        Kind::Frequency(_) => a.sin(),
        Kind::Static(_) => 0.5 * (1.0 - a.cos()),
        Kind::Explicit(_) | Kind::Fluid(_) | Kind::Heat(_) => 1.0,
    }
}

/// What is drawn at animation `frame`: the view's mode and field frame, an animated explicit
/// step running through its field frames once per cycle.
pub fn pick(view: &ResultsView, result: &StepResult, frame: Option<u64>) -> Pick {
    let mut p = view.pick();
    if let (Some(k), Kind::Explicit(e)) = (frame, result.kind())
        && let Some(last) = e.frames.len().checked_sub(1)
    {
        p.frame = Some((k as usize * e.frames.len() / FRAMES as usize).min(last));
    }
    p
}

/// The deformation scale factor: the view's, or automatic (the largest displacement a tenth of
/// the model's size); 0 when the view is undeformed or nothing moves.
pub fn scale(view: &ResultsView, result: &StepResult, pick: Pick, base: &DisplayMesh) -> f64 {
    if !view.deformed {
        return 0.0;
    }
    if let Some(s) = view.scale {
        return s;
    }
    let largest = displacements(result, pick).map_or(0.0, |u| u.iter().map(|v| magnitude(*v)).filter(|m| m.is_finite()).fold(0.0, f64::max));
    let size = base.bounds.diagonal();
    if largest > 0.0 && size.is_finite() && size > 0.0 { AUTO_FRACTION * size / largest } else { 0.0 }
}

/// The part of the session's results plot that depends on what is drawn (`pick`), not on the
/// animation's deformation multiplier: the deformation scale and the contour values.
pub fn prepare(session: &Session, base: &DisplayMesh, pick: Pick) -> Option<Prepared> {
    let (view, result, _) = shown(session)?;
    let contour = view.contour.and_then(|field| {
        let values = field_values(&session.model, result, field, pick).ok()?;
        let (min, max) = range(result.nodes(), &values)?;
        Some(Arc::new(Contour { field, bands: Bands::new(min.1, max.1, view.bands), min, max, values }))
    });
    Some(Prepared { scale: scale(view, result, pick, base), contour })
}

/// The plot of the session's results view on the model's display data at animation `frame`,
/// with its [`prepare`]d part.
pub fn plot(session: &Session, base: &DisplayMesh, frame: Option<u64>, prepared: &Prepared) -> Option<Plot> {
    let (view, result, current) = shown(session)?;
    let nodes = result.nodes();
    let pick = pick(view, result, frame);
    let factor = prepared.scale * swing(result, frame);
    let u = displacements(result, pick).unwrap_or(&[]);
    let display = base.displaced(|label| {
        let i = nodes.binary_search(&label).ok()?;
        u.get(i).map(|d| Vec3::from(*d) * factor)
    });
    Some(Plot {
        view: view.clone(),
        result: result.clone(),
        stale: !current,
        scale: prepared.scale,
        display,
        contour: prepared.contour.clone(),
        pick,
    })
}

/// The scene of a plot: the deformed elements drawn as the model is ([`scene::build`]), with
/// their faces split into contour bands when a field is shown.
pub fn scene(plot: &Plot, style: Style, show: crate::scene::Show, t: &Tokens) -> Scene {
    let mut s = crate::scene::build(&plot.display, style, show, t);
    let Some(c) = plot.contour.as_ref().filter(|_| style != Style::Wireframe && show.mesh) else { return s };
    let nodes = plot.result.nodes();
    let stops = t.contour_stops();
    s.tris.clear();
    for tri in &plot.display.tris {
        split_triangle(tri.p, tri.nodes.map(|l| c.at(nodes, l)), &c.bands, &mut |piece, band| {
            let color = band.map_or(rgb(t.no_result), |k| band_color(&stops, k, c.bands.count));
            for k in 1..piece.len().saturating_sub(1) {
                if let (Some(&a), Some(&b), Some(&cc)) = (piece.first(), piece.get(k), piece.get(k + 1)) {
                    s.tris.push(SceneTri { p: [a, b, cc], normal: tri.normal, color });
                }
            }
        });
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
pub(crate) mod tests {
    use serde_json::json;

    use super::*;

    fn plot_at(s: &Session, base: &DisplayMesh, frame: Option<u64>) -> Option<Plot> {
        let (view, result, _) = shown(s)?;
        plot(s, base, frame, &prepare(s, base, pick(view, result, frame))?)
    }

    /// The sample cantilever, solved (static step `Load`, frequency step `Modes`).
    pub(crate) fn solved_cantilever() -> Session {
        let mut s = Session::default();
        s.run_script(&modelcraft_engine::sample::cantilever()).unwrap();
        s.execute("solve.static", &json!({})).unwrap();
        s.execute("solve.frequency", &json!({})).unwrap();
        s
    }

    /// The label of the node at `xyz`.
    pub(crate) fn node_at(s: &Session, xyz: [f64; 3]) -> u32 {
        *s.model.nodes().iter().find(|(_, n)| n.xyz == xyz).unwrap().0
    }

    #[test]
    fn the_deformed_shape_moves_by_the_scaled_displacement() {
        let mut s = solved_cantilever();
        let base = DisplayMesh::from_model(&s.model);
        s.execute("results.clear", &json!({})).unwrap();
        assert!(plot_at(&s, &base, None).is_none(), "nothing shown after results.clear");
        s.execute("results.deformed", &json!({"scale": 2.0})).unwrap();
        let p = plot_at(&s, &base, None).unwrap();
        assert_eq!(p.scale, 2.0);
        // A tip corner moves twice its displacement.
        let tip = node_at(&s, [100.0, 0.0, 0.0]);
        let Kind::Static(r) = p.result.kind() else { panic!("a static result") };
        let u = r.displacement[r.nodes.binary_search(&tip).unwrap()];
        let at = p.node_position(tip).unwrap();
        assert!((at.z - 2.0 * u[2]).abs() < 1e-12 && u[2] < -0.2, "{at:?} {u:?}");
        // Automatic: the largest displacement is a tenth of the model's size.
        s.execute("results.deformed", &json!({"scale": "auto"})).unwrap();
        let p = plot_at(&s, &base, None).unwrap();
        let largest = r.displacement.iter().map(|v| magnitude(*v)).fold(0.0, f64::max);
        assert!((p.scale * largest / base.bounds.diagonal() - AUTO_FRACTION).abs() < 1e-12, "{}", p.scale);
        s.execute("results.deformed", &json!({"show": false})).unwrap();
        assert_eq!(plot_at(&s, &base, None).unwrap().display.tris, base.tris);
    }

    #[test]
    fn contours_colour_known_nodes_with_their_band() {
        let mut s = solved_cantilever();
        s.execute("results.contour", &json!({"field": "displacement_z", "bands": 4})).unwrap();
        s.execute("results.deformed", &json!({"show": false})).unwrap();
        let base = DisplayMesh::from_model(&s.model);
        let p = plot_at(&s, &base, None).unwrap();
        let legend = p.contour.clone().unwrap();
        // The tip moves most (lowest band), the clamped root not at all (highest band).
        assert!(legend.min.1 < -0.2 && legend.max.1.abs() < 1e-12, "{legend:?}");
        let t = Tokens::dark();
        let scene = scene(&p, Style::Shaded, crate::scene::Show::DEFAULT, &t);
        let stops = t.contour_stops();
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
        let p0 = plot_at(&s, &base, Some(0)).unwrap();
        let p1 = plot_at(&s, &base, Some(FRAMES / 4)).unwrap();
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
