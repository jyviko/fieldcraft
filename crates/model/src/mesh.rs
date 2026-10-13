//! Nodes and elements.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Error, MAX_COORD, MAX_ELEMENTS, MAX_LABEL, MAX_NODES, Model, Result};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

    /// The faces, numbered from 1 in this order, as positions in the element's node list: the
    /// corners, then (quadratic kinds) the mid-side nodes in the same cyclic order.
    ///
    /// - Solids: each face's corners run counter-clockwise seen from outside, so the right-hand
    ///   normal points out of the element. Hexahedra: 1 is the face of nodes 1-2-3-4, 2 the face
    ///   of 5-6-7-8, then the four sides along edges 1-2, 2-3, 3-4 and 4-1. Tetrahedra: 1 is
    ///   1-2-3, then the faces opposite node 3, node 1 and node 2. Wedges: the two triangles, then
    ///   the sides along edges 1-2, 2-3 and 3-1.
    /// - Shells: face 1 is the side the element's right-hand normal points to, face 2 the other.
    /// - Line elements have none.
    pub fn faces(self) -> &'static [&'static [usize]] {
        match self {
            ElementKind::Rod2 | ElementKind::Beam2 => &[],
            ElementKind::Tri3 => &[&[0, 1, 2], &[0, 2, 1]],
            ElementKind::Tri6 => &[&[0, 1, 2, 3, 4, 5], &[0, 2, 1, 5, 4, 3]],
            ElementKind::Quad4 => &[&[0, 1, 2, 3], &[0, 3, 2, 1]],
            ElementKind::Quad8 => &[&[0, 1, 2, 3, 4, 5, 6, 7], &[0, 3, 2, 1, 7, 6, 5, 4]],
            ElementKind::Tet4 => &[&[0, 2, 1], &[0, 1, 3], &[1, 2, 3], &[0, 3, 2]],
            ElementKind::Tet10 => &[&[0, 2, 1, 6, 5, 4], &[0, 1, 3, 4, 8, 7], &[1, 2, 3, 5, 9, 8], &[0, 3, 2, 7, 9, 6]],
            ElementKind::Wedge6 => &[&[0, 2, 1], &[3, 4, 5], &[0, 1, 4, 3], &[1, 2, 5, 4], &[2, 0, 3, 5]],
            ElementKind::Hex8 => &[&[0, 3, 2, 1], &[4, 5, 6, 7], &[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 7, 6], &[3, 0, 4, 7]],
            ElementKind::Hex20 => &[
                &[0, 3, 2, 1, 11, 10, 9, 8],
                &[4, 5, 6, 7, 12, 13, 14, 15],
                &[0, 1, 5, 4, 8, 17, 12, 16],
                &[1, 2, 6, 5, 9, 18, 13, 17],
                &[2, 3, 7, 6, 10, 19, 14, 18],
                &[3, 0, 4, 7, 11, 16, 15, 19],
            ],
        }
    }

    /// The edges, numbered from 1 in this order, as positions in the element's node list: the two
    /// corners, then the mid-side node for quadratic kinds. The order follows the mid-side nodes
    /// (see [`ElementKind`]); linear kinds use the same order.
    pub fn edges(self) -> &'static [&'static [usize]] {
        match self {
            ElementKind::Rod2 | ElementKind::Beam2 => &[&[0, 1]],
            ElementKind::Tri3 => &[&[0, 1], &[1, 2], &[2, 0]],
            ElementKind::Tri6 => &[&[0, 1, 3], &[1, 2, 4], &[2, 0, 5]],
            ElementKind::Quad4 => &[&[0, 1], &[1, 2], &[2, 3], &[3, 0]],
            ElementKind::Quad8 => &[&[0, 1, 4], &[1, 2, 5], &[2, 3, 6], &[3, 0, 7]],
            ElementKind::Tet4 => &[&[0, 1], &[1, 2], &[2, 0], &[0, 3], &[1, 3], &[2, 3]],
            ElementKind::Tet10 => &[&[0, 1, 4], &[1, 2, 5], &[2, 0, 6], &[0, 3, 7], &[1, 3, 8], &[2, 3, 9]],
            ElementKind::Wedge6 => &[&[0, 1], &[1, 2], &[2, 0], &[3, 4], &[4, 5], &[5, 3], &[0, 3], &[1, 4], &[2, 5]],
            ElementKind::Hex8 => &[&[0, 1], &[1, 2], &[2, 3], &[3, 0], &[4, 5], &[5, 6], &[6, 7], &[7, 4], &[0, 4], &[1, 5], &[2, 6], &[3, 7]],
            ElementKind::Hex20 => &[
                &[0, 1, 8],
                &[1, 2, 9],
                &[2, 3, 10],
                &[3, 0, 11],
                &[4, 5, 12],
                &[5, 6, 13],
                &[6, 7, 14],
                &[7, 4, 15],
                &[0, 4, 16],
                &[1, 5, 17],
                &[2, 6, 18],
                &[3, 7, 19],
            ],
        }
    }
}

/// One face of an element, a member of a surface: the element's label and its face number
/// (1-based, [`ElementKind::faces`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ElementFace {
    pub element: u32,
    pub face: u8,
}

/// An element: a topology and its node labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub kind: ElementKind,
    pub nodes: Vec<u32>,
}

/// Check that every coordinate is a finite number within ±[`MAX_COORD`] (the rule for nodes,
/// shared by everything that takes coordinates from outside).
pub fn check_coordinates(xyz: &[f64]) -> Result<()> {
    match xyz.iter().find(|c| !(c.abs() <= MAX_COORD)) {
        Some(c) => Err(Error::Invalid(format!("coordinate {c} is not a finite number within ±{MAX_COORD}"))),
        None => Ok(()),
    }
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
        check_coordinates(&xyz).map_err(|e| match e {
            Error::Invalid(m) => Error::Invalid(format!("node {label}: {m}")),
            e => e,
        })?;
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
        // A replaced element's face and edge numbers no longer mean what its tags recorded.
        self.classification.forget_element(label);
        self.elements.insert(label, Element { kind, nodes });
        Ok(())
    }

    /// Remove a node that no element uses.
    pub fn remove_node(&mut self, label: u32) -> Result<()> {
        if let Some((e, _)) = self.elements.iter().find(|(_, e)| e.nodes.contains(&label)) {
            return Err(Error::Invalid(format!("node {label} is used by element {e}")));
        }
        self.nodes.remove(&label).ok_or_else(|| Error::NotFound(format!("node {label}")))?;
        self.classification.forget_node(label);
        Ok(())
    }

    pub fn remove_element(&mut self, label: u32) -> Result<()> {
        self.elements.remove(&label).ok_or_else(|| Error::NotFound(format!("element {label}")))?;
        self.classification.forget_element(label);
        Ok(())
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

    /// Reference coordinates with positive volume for each kind (mid-side nodes at edge midpoints).
    fn reference(kind: ElementKind) -> Vec<[f64; 3]> {
        let corners: Vec<[f64; 3]> = match kind {
            ElementKind::Rod2 | ElementKind::Beam2 => vec![[0., 0., 0.], [1., 0., 0.]],
            ElementKind::Tri3 | ElementKind::Tri6 => vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            ElementKind::Quad4 | ElementKind::Quad8 => vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
            ElementKind::Tet4 | ElementKind::Tet10 => vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            ElementKind::Wedge6 => vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [0., 1., 1.]],
            ElementKind::Hex8 | ElementKind::Hex20 => {
                vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]]
            }
        };
        let mut xyz = corners.clone();
        xyz.resize(kind.node_count(), [f64::NAN; 3]);
        for e in kind.edges() {
            if let [a, b, m] = **e {
                xyz[m] = [0, 1, 2].map(|i| (corners[a][i] + corners[b][i]) / 2.0);
            }
        }
        xyz
    }

    #[test]
    fn edges_cover_every_node_and_faces_point_out() {
        for kind in ElementKind::ALL {
            let xyz = reference(kind);
            assert!(xyz.iter().flatten().all(|c| c.is_finite()), "{kind:?}: every mid-side node is on an edge");
            let centroid = [0, 1, 2].map(|i| xyz.iter().map(|p| p[i]).sum::<f64>() / xyz.len() as f64);
            for (f, face) in kind.faces().iter().enumerate() {
                let n = if face.len() > 4 { face.len() / 2 } else { face.len() };
                let c: Vec<[f64; 3]> = face[..n].iter().map(|&i| xyz[i]).collect();
                // Newell normal of the corner polygon, against the direction out of the centroid.
                let mut normal = [0.0; 3];
                for k in 0..n {
                    let (p, q) = (c[k], c[(k + 1) % n]);
                    normal = [normal[0] + p[1] * q[2] - p[2] * q[1], normal[1] + p[2] * q[0] - p[0] * q[2], normal[2] + p[0] * q[1] - p[1] * q[0]];
                }
                let out = [0, 1, 2].map(|i| c[0][i] - centroid[i]);
                if kind.dimension() == 3 {
                    assert!(normal[0] * out[0] + normal[1] * out[1] + normal[2] * out[2] > 0.0, "{kind:?} face {} points in", f + 1);
                }
                // A quadratic face's mid-side nodes sit on its own corner edges, in cyclic order.
                for (k, &m) in face[n..].iter().enumerate() {
                    let (a, b) = (face[k], face[(k + 1) % n]);
                    assert_eq!(xyz[m], [0, 1, 2].map(|i| (xyz[a][i] + xyz[b][i]) / 2.0), "{kind:?} face {} mid-side {}", f + 1, k + 1);
                }
            }
            if kind.dimension() == 2 {
                let [p, q] = kind.faces() else { panic!("{kind:?}: two sides") };
                assert_eq!(p.len(), kind.node_count());
                let mut sorted = q.to_vec();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..kind.node_count()).collect::<Vec<_>>());
            }
        }
        assert_eq!(ElementKind::Hex8.faces().len(), 6);
        assert_eq!(ElementKind::Tet4.faces().len(), 4);
        assert_eq!(ElementKind::Wedge6.faces().len(), 5);
        assert_eq!(ElementKind::Hex20.edges().len(), 12);
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
