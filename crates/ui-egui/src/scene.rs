//! What the viewport draws: the model's display data in the chosen style and the theme's
//! colours. The GPU viewport and the headless render (`ui.render`) draw the same [`Scene`].
//!
//! Geometry (the open design, imported surfaces) draws as faces with their boundaries while
//! the model has no element faces; once it is meshed, as boundaries only, so the mesh is not
//! hidden behind coincident faces. The analysis setup's symbols ([`symbols`]) go over the model,
//! muted over a results plot.

use fieldcraft_render::glyphs::{GlyphShow, Glyphs};
use fieldcraft_render::{Aabb3, Camera, DisplayMesh, Scene, SceneLine, ScenePoint, SceneTri, Vec3};
use serde::{Deserialize, Serialize};

use crate::theme::{Tokens, rgb};

/// How the model is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Lit faces with the element edges on them.
    #[default]
    ShadedEdges,
    /// Lit faces only.
    Shaded,
    /// Element edges only.
    Wireframe,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::ShadedEdges, Style::Shaded, Style::Wireframe];
    pub fn label(self) -> &'static str {
        match self {
            Style::ShadedEdges => "Shaded with edges",
            Style::Shaded => "Shaded",
            Style::Wireframe => "Wireframe",
        }
    }
}

/// Mesh edge width, geometry edge width, line element width and node size, in points.
const EDGE_WIDTH: f64 = 1.0;
const GEOMETRY_EDGE_WIDTH: f64 = 1.5;
const LINE_ELEMENT_WIDTH: f64 = 3.0;
const NODE_SIZE: f64 = 5.0;

/// What is shown (the browser's eye toggles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Show {
    /// The open design and imported surfaces.
    pub geometry: bool,
    /// Elements.
    pub mesh: bool,
    pub nodes: bool,
    /// Boundary conditions and loads.
    pub loads: bool,
    /// Reference points, couplings and rigid walls.
    pub interactions: bool,
}

impl Show {
    /// Geometry, mesh and the analysis setup, without nodes.
    pub const DEFAULT: Show = Show { geometry: true, mesh: true, nodes: false, loads: true, interactions: true };
}

/// Draw the analysis setup's symbols into `s` at the nodes' positions `pos`, muted over a
/// results plot; returns the box around them.
pub fn symbols(s: &mut Scene, glyphs: &Glyphs, pos: impl Fn(u32) -> Option<Vec3>, show: Show, results: bool, t: &Tokens) -> Aabb3 {
    let colors = t.glyph_colors();
    let colors = if results { colors.over_results(rgb(t.viewport_bottom)) } else { colors };
    glyphs.draw(pos, GlyphShow { loads: show.loads, interactions: show.interactions }, &colors, s)
}

/// Draw the model's datum planes and axes around `bounds` into `s`; returns the box around them.
pub fn datums(s: &mut Scene, model: &fieldcraft_model::Model, bounds: &Aabb3, t: &Tokens) -> Aabb3 {
    fieldcraft_render::glyphs::datums(model, bounds, &t.glyph_colors(), s)
}

/// The scene for `display` drawn in `style`.
pub fn build(display: &DisplayMesh, style: Style, show: Show, t: &Tokens) -> Scene {
    let mut s = Scene { background: Some((rgb(t.viewport_top), rgb(t.viewport_bottom))), ..Default::default() };
    // Geometry faces draw where no element faces cover them.
    let mesh_faces = show.mesh && !display.tris.is_empty();
    if style != Style::Wireframe {
        if show.mesh {
            s.tris = display.tris.iter().map(|d| SceneTri { p: d.p, normal: d.normal, color: rgb(t.element(d.kind)) }).collect();
        }
        if !mesh_faces && show.geometry {
            s.tris.extend(display.surface_tris.iter().map(|d| SceneTri {
                p: d.p,
                normal: d.normal,
                color: rgb(display.body_color(d, t.geometry, &t.bodies)),
            }));
        }
    }
    if style != Style::Shaded && show.mesh {
        // Without faces behind them the edges take the element colour, so they read on the
        // background.
        let color = rgb(if style == Style::Wireframe { t.hex } else { t.mesh_edge });
        s.lines = display.edges.iter().map(|e| SceneLine { a: e.p[0], b: e.p[1], color, width: EDGE_WIDTH }).collect();
    }
    // Face boundaries in every style: they are the geometry's outline.
    if show.geometry {
        s.lines.extend(display.surface_edges.iter().map(|[a, b]| SceneLine {
            a: *a,
            b: *b,
            color: rgb(t.geometry_edge),
            width: GEOMETRY_EDGE_WIDTH,
        }));
    }
    if show.mesh {
        s.lines.extend(display.lines.iter().map(|l| SceneLine { a: l.a, b: l.b, color: rgb(t.element(l.kind)), width: LINE_ELEMENT_WIDTH }));
    }
    if show.nodes {
        s.points = display.nodes.iter().map(|n| ScenePoint { p: n.p, color: rgb(t.node), size: NODE_SIZE }).collect();
    }
    s
}

/// Size of a seed's tick (points).
const SEED_TICK: f64 = 6.0;

/// Draw the model's edge seeds as ticks at the nodes they put along their edges (the drawn
/// bodies' B-rep edges, [`DisplayMesh::curves`], named by `names`): the seeded count, spaced by
/// the bias, so a grading shows as ticks crowding toward an end.
pub fn seeds(s: &mut Scene, model: &fieldcraft_model::Model, names: &[crate::selection::BodyNames], display: &DisplayMesh, t: &Tokens) {
    if model.seeds().is_empty() {
        return;
    }
    let color = rgb(t.warm);
    for run in display.curves.chunk_by(|a, b| a.body == b.body && a.edge == b.edge) {
        let Some(first) = run.first() else { continue };
        let Some(edge) = names.get(first.body as usize).and_then(|b| b.edge(first.edge)) else { continue };
        let Some(seed) = model.seeds().iter().find(|seed| seed.edge.selects(&edge)) else { continue };
        let lengths: Vec<f64> = run.iter().map(|c| c.a.dist(c.b)).collect();
        let total: f64 = lengths.iter().sum();
        if !(total > 0.0 && total.is_finite()) {
            continue;
        }
        // Walk the polyline to each node's arc length.
        let (mut k, mut walked) = (0, 0.0);
        for f in seed.spacing(seed.count(total)) {
            let target = f * total;
            while k + 1 < run.len() && walked + lengths.get(k).copied().unwrap_or(0.0) < target {
                walked += lengths.get(k).copied().unwrap_or(0.0);
                k += 1;
            }
            let (Some(c), Some(l)) = (run.get(k), lengths.get(k)) else { continue };
            let u = if *l > 0.0 { ((target - walked) / l).clamp(0.0, 1.0) } else { 0.0 };
            s.points.push(ScenePoint { p: c.a + (c.b - c.a) * u, color, size: SEED_TICK });
        }
    }
}

/// The radius around the camera target that holds the bounds (for the clipping planes).
pub fn radius(b: &Aabb3, cam: &Camera) -> f64 {
    if b.is_empty() { 1.0 } else { (cam.target.dist(b.center()) + b.diagonal() * 0.5).max(1e-3) }
}

#[cfg(test)]
mod tests {
    use fieldcraft_model::{ElementKind, Model};

    use super::*;

    #[test]
    fn seeds_draw_ticks_at_their_nodes() {
        use fieldcraft_model::{Bias, BiasToward, Divisions, Entity, GeometryRef, Seed, SeedConstraint};
        let mut d = DisplayMesh::default();
        // Two edges of 10 mm; the first in two segments.
        d.add_curves(&[vec![[0.0; 3], [4.0, 0.0, 0.0], [10.0, 0.0, 0.0]], vec![[0.0; 3], [0.0, 10.0, 0.0]]]);
        let names = [crate::selection::BodyNames { name: "B".into(), faces: vec![], edges: vec!["a|b".into(), "a|c".into()] }];
        let mut m = Model::default();
        let edge = GeometryRef::new("B", Entity::Edge("a|b".into())).unwrap();
        m.set_seed(Seed::new(edge, Divisions::Count(4), Some(Bias { ratio: 8.0, toward: BiasToward::Start }), SeedConstraint::Exact).unwrap())
            .unwrap();
        let mut s = Scene::default();
        seeds(&mut s, &m, &names, &d, &Tokens::dark());
        let xs: Vec<f64> = s.points.iter().map(|p| p.p.x).collect();
        let want = [0.0, 10.0 / 15.0, 30.0 / 15.0, 70.0 / 15.0, 10.0];
        assert_eq!(xs.len(), 5);
        assert!(xs.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "{xs:?}");
        assert!(s.points.iter().all(|p| p.p.y == 0.0), "only the seeded edge has ticks");
    }

    #[test]
    fn styles_choose_faces_edges_and_nodes() {
        let mut m = Model::default();
        for (i, p) in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].into_iter().enumerate() {
            m.set_node(i as u32 + 1, p).unwrap();
        }
        m.set_element(1, ElementKind::Tet4, vec![1, 2, 3, 4]).unwrap();
        let d = DisplayMesh::from_model(&m);
        let t = Tokens::dark();
        let s = build(&d, Style::ShadedEdges, Show::DEFAULT, &t);
        assert_eq!((s.tris.len(), s.lines.len(), s.points.len()), (4, 6, 0));
        let s = build(&d, Style::Shaded, Show { nodes: true, ..Show::DEFAULT }, &t);
        assert_eq!((s.tris.len(), s.lines.len(), s.points.len()), (4, 0, 4));
        let s = build(&d, Style::ShadedEdges, Show { mesh: false, ..Show::DEFAULT }, &t);
        assert_eq!((s.tris.len(), s.lines.len()), (0, 0), "a hidden mesh draws nothing");
        let s = build(&d, Style::Wireframe, Show::DEFAULT, &t);
        assert_eq!((s.tris.len(), s.lines.len()), (0, 6));
        assert_eq!(s.tris.first().map(|t| t.color), None);
        assert!(radius(&d.bounds, &Camera::default()) > 0.5);
    }
}
