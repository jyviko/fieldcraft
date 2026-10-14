//! What the viewport draws: the model's display data in the chosen style and the theme's
//! colours. The GPU viewport and the headless render (`ui.render`) draw the same [`Scene`].
//!
//! Geometry (the open design, imported surfaces) draws as faces with their boundaries while
//! the model has no element faces; once it is meshed, as boundaries only, so the mesh is not
//! hidden behind coincident faces.

use fieldcraft_render::{Aabb3, Camera, DisplayMesh, Scene, SceneLine, ScenePoint, SceneTri};
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
}

impl Show {
    /// Geometry and mesh, without nodes.
    pub const DEFAULT: Show = Show { geometry: true, mesh: true, nodes: false };
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
            s.tris.extend(display.surface_tris.iter().map(|d| SceneTri { p: d.p, normal: d.normal, color: rgb(t.geometry) }));
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

/// The radius around the camera target that holds the bounds (for the clipping planes).
pub fn radius(b: &Aabb3, cam: &Camera) -> f64 {
    if b.is_empty() { 1.0 } else { (cam.target.dist(b.center()) + b.diagonal() * 0.5).max(1e-3) }
}

#[cfg(test)]
mod tests {
    use fieldcraft_model::{ElementKind, Model};

    use super::*;

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
