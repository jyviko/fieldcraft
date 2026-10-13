//! The solver-neutral finite element model.
//!
//! A [`Model`] holds what every solver deck describes: nodes and elements addressed by the
//! positive integer labels the decks use. Materials, sections, sets, boundary conditions, loads
//! and steps join it as their tasks land (plan/architecture.md §4). Solver formats (Abaqus input,
//! Nastran bulk data, LS-DYNA keyword) translate to and from this model; nothing here knows any
//! of them.
//!
//! The model is unit-consistent: it stores numbers in whatever consistent system the user chose
//! (for example mm, tonne, s, N, MPa) and never converts them.
//!
//! Every mutation validates its input, because labels and coordinates come from files, scripts
//! and the control channel: a bad value is an [`Error`], never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The largest node or element label. Nastran's GRID and element ids stop at 99 999 999 (eight
/// digits in a small field), the tightest of the three formats.
pub const MAX_LABEL: u32 = 99_999_999;

/// Caps on model size, so hostile input cannot exhaust memory.
pub const MAX_NODES: usize = 50_000_000;
pub const MAX_ELEMENTS: usize = 50_000_000;

/// Coordinates beyond this magnitude are rejected (1e9 model units).
pub const MAX_COORD: f64 = 1e9;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("limit reached: {0}")]
    Limit(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A node: a point in model space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub xyz: [f64; 3],
}

/// Element topology. Node order is corners first, then mid-side nodes, with positive volume for
/// a right-handed ordering:
/// - tet10: mid-side nodes 5–10 on edges 1-2, 2-3, 3-1, 1-4, 2-4, 3-4;
/// - wedge6: triangle 1-2-3, then 4-5-6 on the side its right-hand normal points to;
/// - hex8/hex20: face 1-2-3-4, then 5-6-7-8 above it; hex20's mid-side nodes 9–12 lie on the
///   edges of 1-2-3-4, 13–16 on those of 5-6-7-8, and 17–20 on edges 1-5, 2-6, 3-7, 4-8.
///
/// This is the Abaqus (and VTK) order. Formats that order nodes differently, such as Nastran's
/// CHEXA with 20 nodes (vertical edges before the top face's edges), are reordered by their deck
/// readers and writers; the model never stores another order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    /// Two-node axial member.
    Rod2,
    /// Two-node beam.
    Beam2,
    /// Three-node triangle (shell or membrane).
    Tri3,
    /// Six-node triangle.
    Tri6,
    /// Four-node quadrilateral.
    Quad4,
    /// Eight-node quadrilateral.
    Quad8,
    /// Four-node tetrahedron.
    Tet4,
    /// Ten-node tetrahedron.
    Tet10,
    /// Six-node wedge (pentahedron).
    Wedge6,
    /// Eight-node hexahedron.
    Hex8,
    /// Twenty-node hexahedron.
    Hex20,
}

impl ElementKind {
    pub const ALL: [ElementKind; 11] = [
        ElementKind::Rod2,
        ElementKind::Beam2,
        ElementKind::Tri3,
        ElementKind::Tri6,
        ElementKind::Quad4,
        ElementKind::Quad8,
        ElementKind::Tet4,
        ElementKind::Tet10,
        ElementKind::Wedge6,
        ElementKind::Hex8,
        ElementKind::Hex20,
    ];

    pub fn node_count(self) -> usize {
        match self {
            ElementKind::Rod2 | ElementKind::Beam2 => 2,
            ElementKind::Tri3 => 3,
            ElementKind::Quad4 | ElementKind::Tet4 => 4,
            ElementKind::Tri6 | ElementKind::Wedge6 => 6,
            ElementKind::Quad8 | ElementKind::Hex8 => 8,
            ElementKind::Tet10 => 10,
            ElementKind::Hex20 => 20,
        }
    }

    /// Topological dimension: 1 for line elements, 2 for shells, 3 for solids.
    pub fn dimension(self) -> u8 {
        match self {
            ElementKind::Rod2 | ElementKind::Beam2 => 1,
            ElementKind::Tri3 | ElementKind::Tri6 | ElementKind::Quad4 | ElementKind::Quad8 => 2,
            ElementKind::Tet4 | ElementKind::Tet10 | ElementKind::Wedge6 | ElementKind::Hex8 | ElementKind::Hex20 => 3,
        }
    }

    /// The name used in commands and `.modelcraft` files (`hex8`, `tet10`, …).
    pub fn name(self) -> &'static str {
        match self {
            ElementKind::Rod2 => "rod2",
            ElementKind::Beam2 => "beam2",
            ElementKind::Tri3 => "tri3",
            ElementKind::Tri6 => "tri6",
            ElementKind::Quad4 => "quad4",
            ElementKind::Quad8 => "quad8",
            ElementKind::Tet4 => "tet4",
            ElementKind::Tet10 => "tet10",
            ElementKind::Wedge6 => "wedge6",
            ElementKind::Hex8 => "hex8",
            ElementKind::Hex20 => "hex20",
        }
    }

    pub fn from_name(name: &str) -> Option<ElementKind> {
        ElementKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// An element: a topology and its node labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub kind: ElementKind,
    pub nodes: Vec<u32>,
}

/// Nodes and elements by label.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    nodes: BTreeMap<u32, Node>,
    elements: BTreeMap<u32, Element>,
}

fn check_label(what: &str, label: u32) -> Result<()> {
    if label == 0 || label > MAX_LABEL {
        return Err(Error::Invalid(format!("{what} label {label} is outside 1..={MAX_LABEL}")));
    }
    Ok(())
}

impl Model {
    pub fn nodes(&self) -> &BTreeMap<u32, Node> {
        &self.nodes
    }

    pub fn elements(&self) -> &BTreeMap<u32, Element> {
        &self.elements
    }

    pub fn node(&self, label: u32) -> Option<&Node> {
        self.nodes.get(&label)
    }

    pub fn element(&self, label: u32) -> Option<&Element> {
        self.elements.get(&label)
    }

    /// Add a node, or move an existing one (decks redefine nodes the same way).
    pub fn set_node(&mut self, label: u32, xyz: [f64; 3]) -> Result<()> {
        check_label("node", label)?;
        if let Some(c) = xyz.iter().find(|c| !(c.abs() <= MAX_COORD)) {
            return Err(Error::Invalid(format!("node {label}: coordinate {c} is not a finite number within ±{MAX_COORD}")));
        }
        if !self.nodes.contains_key(&label) && self.nodes.len() >= MAX_NODES {
            return Err(Error::Limit(format!("a model holds at most {MAX_NODES} nodes")));
        }
        self.nodes.insert(label, Node { xyz });
        Ok(())
    }

    /// Add an element, or replace an existing one. Its nodes must exist and be distinct.
    pub fn set_element(&mut self, label: u32, kind: ElementKind, nodes: Vec<u32>) -> Result<()> {
        check_label("element", label)?;
        if nodes.len() != kind.node_count() {
            return Err(Error::Invalid(format!("element {label}: {} needs {} nodes, got {}", kind.name(), kind.node_count(), nodes.len())));
        }
        for (i, n) in nodes.iter().enumerate() {
            if !self.nodes.contains_key(n) {
                return Err(Error::NotFound(format!("element {label}: node {n}")));
            }
            if nodes.get(..i).is_some_and(|before| before.contains(n)) {
                return Err(Error::Invalid(format!("element {label}: node {n} appears twice")));
            }
        }
        if !self.elements.contains_key(&label) && self.elements.len() >= MAX_ELEMENTS {
            return Err(Error::Limit(format!("a model holds at most {MAX_ELEMENTS} elements")));
        }
        self.elements.insert(label, Element { kind, nodes });
        Ok(())
    }

    /// Remove a node that no element uses.
    pub fn remove_node(&mut self, label: u32) -> Result<()> {
        if let Some((e, _)) = self.elements.iter().find(|(_, e)| e.nodes.contains(&label)) {
            return Err(Error::Invalid(format!("node {label} is used by element {e}")));
        }
        self.nodes.remove(&label).map(|_| ()).ok_or_else(|| Error::NotFound(format!("node {label}")))
    }

    pub fn remove_element(&mut self, label: u32) -> Result<()> {
        self.elements.remove(&label).map(|_| ()).ok_or_else(|| Error::NotFound(format!("element {label}")))
    }

    /// Element counts by kind, in [`ElementKind::ALL`] order, kinds with none left out.
    pub fn element_counts(&self) -> Vec<(ElementKind, usize)> {
        ElementKind::ALL.into_iter().map(|k| (k, self.elements.values().filter(|e| e.kind == k).count())).filter(|(_, n)| *n > 0).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Model {
        let mut m = Model::default();
        let corners = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]];
        for (i, c) in corners.into_iter().enumerate() {
            m.set_node(i as u32 + 1, c).unwrap();
        }
        m.set_element(1, ElementKind::Hex8, (1..=8).collect()).unwrap();
        m
    }

    #[test]
    fn builds_a_hex() {
        let m = cube();
        assert_eq!(m.nodes().len(), 8);
        assert_eq!(m.element_counts(), vec![(ElementKind::Hex8, 1)]);
    }

    #[test]
    fn rejects_bad_labels_and_coordinates() {
        let mut m = Model::default();
        assert!(m.set_node(0, [0.; 3]).is_err());
        assert!(m.set_node(MAX_LABEL + 1, [0.; 3]).is_err());
        assert!(m.set_node(1, [f64::NAN, 0., 0.]).is_err());
        assert!(m.set_node(1, [f64::INFINITY, 0., 0.]).is_err());
        assert!(m.set_node(1, [2e9, 0., 0.]).is_err());
        assert!(m.nodes().is_empty());
    }

    #[test]
    fn rejects_bad_connectivity() {
        let mut m = cube();
        assert!(matches!(m.set_element(2, ElementKind::Tet4, vec![1, 2, 3]), Err(Error::Invalid(_))));
        assert!(matches!(m.set_element(2, ElementKind::Tet4, vec![1, 2, 3, 99]), Err(Error::NotFound(_))));
        assert!(matches!(m.set_element(2, ElementKind::Tet4, vec![1, 2, 3, 3]), Err(Error::Invalid(_))));
        assert_eq!(m.elements().len(), 1);
    }

    #[test]
    fn used_nodes_cannot_be_removed() {
        let mut m = cube();
        assert!(m.remove_node(1).is_err());
        m.remove_element(1).unwrap();
        m.remove_node(1).unwrap();
        assert!(m.remove_node(1).is_err());
    }

    #[test]
    fn kind_names_round_trip() {
        for k in ElementKind::ALL {
            assert_eq!(ElementKind::from_name(k.name()), Some(k));
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(k.name()));
        }
        assert_eq!(ElementKind::from_name("C3D8"), None);
    }
}
