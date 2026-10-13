//! What a finite element model looks like on screen: the faces nobody shares (the skin of the
//! solid elements, and every shell), the element edges on those faces, line elements and nodes.
//!
//! Faces shared by two solid elements are inside the mesh and are left out, so a model of a
//! million elements draws its surface, not its interior. Higher-order elements (tet10, hex20,
//! tri6, quad8) draw through their corner nodes. The output is deterministic: faces come out in
//! element label order, edges sorted.

use modelcraft_model::{ElementKind, Model};

use crate::{Aabb3, Vec3};

/// One drawn triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayTri {
    pub p: [Vec3; 3],
    /// The face's unit normal, pointing out of its element (a shell's own orientation).
    pub normal: Vec3,
    pub kind: ElementKind,
}

/// A line element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayLine {
    pub a: Vec3,
    pub b: Vec3,
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

/// Most corners of any element (hex).
const MAX_CORNERS: usize = 8;

/// A face's identity regardless of winding and starting corner: its sorted node labels, a
/// triangle padded with `u32::MAX`.
fn face_key(e: &[u32], face: &[usize]) -> Option<[u32; 4]> {
    let mut k = [u32::MAX; 4];
    for (slot, &i) in k.iter_mut().zip(face) {
        *slot = *e.get(i)?;
    }
    k.sort_unstable();
    Some(k)
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

fn centre(p: &[Vec3]) -> Vec3 {
    p.iter().fold(Vec3::ZERO, |s, q| s + *q) * (1.0 / p.len().max(1) as f64)
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
        // Faces two solid elements share are inside the mesh: sort every solid face's key and
        // keep the ones that occur more than once.
        let mut keys: Vec<[u32; 4]> = model
            .elements()
            .values()
            .filter(|e| e.kind.dimension() == 3)
            .flat_map(|e| faces(e.kind).iter().filter_map(|f| face_key(&e.nodes, f)))
            .collect();
        keys.sort_unstable();
        let mut inner: Vec<[u32; 4]> = keys.windows(2).filter(|w| w.first() == w.get(1)).filter_map(|w| w.first().copied()).collect();
        inner.dedup();
        drop(keys);
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for e in model.elements().values() {
            // Corners: one past the highest face index (line elements: their two ends).
            let corners = faces(e.kind).iter().flat_map(|f| f.iter()).max().map_or(2, |m| m + 1).min(MAX_CORNERS);
            let mut at = [Vec3::ZERO; MAX_CORNERS];
            let mut found = 0;
            for (slot, &n) in at.iter_mut().zip(e.nodes.iter().take(corners)) {
                if let Some(p) = pos(n) {
                    *slot = p;
                    found += 1;
                }
            }
            if found < corners {
                continue;
            }
            let at = at.get(..corners).unwrap_or(&[]);
            if e.kind.dimension() == 1 {
                if let (Some(&a), Some(&b)) = (at.first(), at.get(1)) {
                    out.lines.push(DisplayLine { a, b, kind: e.kind });
                }
                continue;
            }
            let solid = e.kind.dimension() == 3;
            let element_centre = centre(at);
            for f in faces(e.kind) {
                if solid && face_key(&e.nodes, f).is_some_and(|k| inner.binary_search(&k).is_ok()) {
                    continue;
                }
                let p: Vec<Vec3> = f.iter().filter_map(|&i| at.get(i).copied()).collect();
                let Some(mut normal) = polygon_normal(&p) else { continue };
                if solid && normal.dot(centre(&p) - element_centre) < 0.0 {
                    normal = -normal;
                }
                for k in 1..p.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c)) = (p.first(), p.get(k), p.get(k + 1)) {
                        out.tris.push(DisplayTri { p: [a, b, c], normal, kind: e.kind });
                    }
                }
                for (j, &i) in f.iter().enumerate() {
                    let i2 = f.get((j + 1) % f.len()).copied().unwrap_or(i);
                    if let (Some(&a), Some(&b)) = (e.nodes.get(i), e.nodes.get(i2)) {
                        edges.push((a.min(b), a.max(b)));
                    }
                }
            }
        }
        edges.sort_unstable();
        edges.dedup();
        out.edges = edges.into_iter().filter_map(|(a, b)| Some([pos(a)?, pos(b)?])).collect();
        out
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
        // Euler on the skin: V - E + F = 2 (two nodes are inside).
        let skin_nodes = 4 * 3 * 3 - 2;
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
