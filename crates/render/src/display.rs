//! What a finite element model looks like on screen: the faces nobody shares (the skin of the
//! solid elements, and every shell), the element edges on those faces, line elements and nodes.
//!
//! Faces shared by two solid elements are inside the mesh and are left out, so a model of a
//! million elements draws its surface, not its interior. Higher-order elements (tet10, hex20,
//! tri6, quad8) draw through their corner nodes. The output is deterministic: faces and edges
//! come out in element label order.

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_model::{ElementKind, Model};

use crate::{Aabb3, Vec3};

/// One drawn triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayTri {
    pub p: [Vec3; 3],
    /// The face's unit normal, pointing out of its element (a shell's own orientation).
    pub normal: Vec3,
    /// The element the face belongs to, and its kind (for colour by kind, picking later).
    pub element: u32,
    pub kind: ElementKind,
}

/// A line element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayLine {
    pub a: Vec3,
    pub b: Vec3,
    pub element: u32,
    pub kind: ElementKind,
}

/// Display data for a model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayMesh {
    /// Visible faces as triangles (quads are split in two).
    pub tris: Vec<DisplayTri>,
    /// Element edges on the visible faces, each once.
    pub edges: Vec<[Vec3; 2]>,
    /// Line elements (rods, beams).
    pub lines: Vec<DisplayLine>,
    /// Every node.
    pub nodes: Vec<Vec3>,
    pub bounds: Aabb3,
}

/// The faces of an element as corner indices (0-based into its node list), each wound
/// counter-clockwise seen from outside for a positively oriented element.
fn faces(kind: ElementKind) -> &'static [&'static [usize]] {
    match kind {
        ElementKind::Rod2 | ElementKind::Beam2 => &[],
        ElementKind::Tri3 | ElementKind::Tri6 => &[&[0, 1, 2]],
        ElementKind::Quad4 | ElementKind::Quad8 => &[&[0, 1, 2, 3]],
        ElementKind::Tet4 | ElementKind::Tet10 => &[&[0, 2, 1], &[0, 1, 3], &[1, 2, 3], &[2, 0, 3]],
        ElementKind::Wedge6 => &[&[0, 2, 1], &[3, 4, 5], &[0, 1, 4, 3], &[1, 2, 5, 4], &[2, 0, 3, 5]],
        ElementKind::Hex8 | ElementKind::Hex20 => &[&[0, 3, 2, 1], &[4, 5, 6, 7], &[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 7, 6], &[3, 0, 4, 7]],
    }
}

/// A face's identity regardless of winding and starting corner: its sorted node labels.
fn face_key(labels: &[u32]) -> Vec<u32> {
    let mut k = labels.to_vec();
    k.sort_unstable();
    k
}

/// Newell's normal of a polygon (robust for non-planar quads); `None` when degenerate.
fn polygon_normal(p: &[Vec3]) -> Option<Vec3> {
    let mut n = Vec3::ZERO;
    for (i, a) in p.iter().enumerate() {
        let b = p.get((i + 1) % p.len()).copied().unwrap_or(*a);
        n = n + Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    n.normalized()
}

impl DisplayMesh {
    pub fn from_model(model: &Model) -> DisplayMesh {
        let mut out = DisplayMesh::default();
        let pos = |label: u32| model.node(label).map(|n| Vec3::from(n.xyz));
        for n in model.nodes().values() {
            let p = Vec3::from(n.xyz);
            out.nodes.push(p);
            out.bounds.add(p);
        }
        // How many solid elements use each face: a face used twice is inside the mesh.
        let mut uses: BTreeMap<Vec<u32>, u32> = BTreeMap::new();
        for e in model.elements().values().filter(|e| e.kind.dimension() == 3) {
            for f in faces(e.kind) {
                let labels: Option<Vec<u32>> = f.iter().map(|&i| e.nodes.get(i).copied()).collect();
                if let Some(l) = labels {
                    *uses.entry(face_key(&l)).or_default() += 1;
                }
            }
        }
        let mut edges: BTreeSet<(u32, u32)> = BTreeSet::new();
        for (&label, e) in model.elements() {
            if e.kind.dimension() == 1 {
                if let (Some(a), Some(b)) = (e.nodes.first().and_then(|&n| pos(n)), e.nodes.get(1).and_then(|&n| pos(n))) {
                    out.lines.push(DisplayLine { a, b, element: label, kind: e.kind });
                }
                continue;
            }
            let solid = e.kind.dimension() == 3;
            // The element's centre, to point each face's normal out of it.
            let corners: Vec<Vec3> = e.nodes.iter().take(corner_count(e.kind)).filter_map(|&n| pos(n)).collect();
            let centre = corners.iter().fold(Vec3::ZERO, |s, p| s + *p) * (1.0 / corners.len().max(1) as f64);
            for f in faces(e.kind) {
                let Some(labels) = f.iter().map(|&i| e.nodes.get(i).copied()).collect::<Option<Vec<u32>>>() else { continue };
                if solid && uses.get(&face_key(&labels)).is_some_and(|&n| n > 1) {
                    continue;
                }
                let Some(p) = labels.iter().map(|&l| pos(l)).collect::<Option<Vec<Vec3>>>() else { continue };
                let Some(mut normal) = polygon_normal(&p) else { continue };
                let face_centre = p.iter().fold(Vec3::ZERO, |s, q| s + *q) * (1.0 / p.len() as f64);
                if solid && normal.dot(face_centre - centre) < 0.0 {
                    normal = -normal;
                }
                for k in 1..p.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c)) = (p.first(), p.get(k), p.get(k + 1)) {
                        out.tris.push(DisplayTri { p: [a, b, c], normal, element: label, kind: e.kind });
                    }
                }
                for (i, &a) in labels.iter().enumerate() {
                    let b = labels.get((i + 1) % labels.len()).copied().unwrap_or(a);
                    edges.insert((a.min(b), a.max(b)));
                }
            }
        }
        out.edges = edges.into_iter().filter_map(|(a, b)| Some([pos(a)?, pos(b)?])).collect();
        out
    }
}

fn corner_count(kind: ElementKind) -> usize {
    match kind {
        ElementKind::Rod2 | ElementKind::Beam2 => 2,
        ElementKind::Tri3 | ElementKind::Tri6 => 3,
        ElementKind::Quad4 | ElementKind::Quad8 | ElementKind::Tet4 | ElementKind::Tet10 => 4,
        ElementKind::Wedge6 => 6,
        ElementKind::Hex8 | ElementKind::Hex20 => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `nx × ny × nz` unit hex8 cells.
    fn block(nx: u32, ny: u32, nz: u32) -> Model {
        let mut m = Model::default();
        let id = |i: u32, j: u32, k: u32| 1 + i + j * (nx + 1) + k * (nx + 1) * (ny + 1);
        for k in 0..=nz {
            for j in 0..=ny {
                for i in 0..=nx {
                    m.set_node(id(i, j, k), [i as f64, j as f64, k as f64]).unwrap();
                }
            }
        }
        let mut label = 1;
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let n = vec![
                        id(i, j, k),
                        id(i + 1, j, k),
                        id(i + 1, j + 1, k),
                        id(i, j + 1, k),
                        id(i, j, k + 1),
                        id(i + 1, j, k + 1),
                        id(i + 1, j + 1, k + 1),
                        id(i, j + 1, k + 1),
                    ];
                    m.set_element(label, ElementKind::Hex8, n).unwrap();
                    label += 1;
                }
            }
        }
        m
    }

    #[test]
    fn a_block_shows_its_skin_only() {
        let d = DisplayMesh::from_model(&block(3, 2, 2));
        // Skin quads: 2·(3·2 + 3·2 + 2·2) = 32, two triangles each.
        assert_eq!(d.tris.len(), 64);
        assert_eq!(d.nodes.len(), 4 * 3 * 3);
        assert_eq!(d.bounds.max, Vec3::new(3.0, 2.0, 2.0));
        // Every normal points out of the block.
        let c = d.bounds.center();
        for t in &d.tris {
            let fc = (t.p[0] + t.p[1] + t.p[2]) * (1.0 / 3.0);
            assert!(t.normal.dot(fc - c) > 0.0, "{t:?}");
        }
        // Edges of the skin grid: each surface edge once. Counted per face: x-lines on the
        // two z-faces and two y-faces, and so on. Euler: V - E + F = 2 on the skin.
        let skin_nodes = 4 * 3 * 3 - 2; // the two interior nodes (i=1,2; j=1; k=1)
        assert_eq!(skin_nodes as i64 - d.edges.len() as i64 + 32, 2);
    }

    #[test]
    fn shells_lines_and_tets_draw() {
        let mut m = Model::default();
        for (i, p) in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 1., 0.]].into_iter().enumerate() {
            m.set_node(i as u32 + 1, p).unwrap();
        }
        m.set_element(1, ElementKind::Tet4, vec![1, 2, 3, 4]).unwrap();
        m.set_element(2, ElementKind::Tri3, vec![2, 5, 3]).unwrap();
        m.set_element(3, ElementKind::Beam2, vec![4, 5]).unwrap();
        let d = DisplayMesh::from_model(&m);
        assert_eq!(d.tris.len(), 5);
        assert_eq!(d.lines.len(), 1);
        assert_eq!(d.edges.len(), 6 + 2);
        let tet_normals: Vec<Vec3> = d.tris.iter().filter(|t| t.kind == ElementKind::Tet4).map(|t| t.normal).collect();
        assert!(tet_normals.contains(&Vec3::new(0.0, 0.0, -1.0)));
    }

    #[test]
    fn empty_and_deterministic() {
        assert_eq!(DisplayMesh::from_model(&Model::default()), DisplayMesh::default());
        let m = block(2, 2, 1);
        assert_eq!(DisplayMesh::from_model(&m), DisplayMesh::from_model(&m.clone()));
    }
}
