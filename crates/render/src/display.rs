//! What a model looks like on screen.
//!
//! Finite elements: the faces nobody shares (the skin of the solid elements, and every shell),
//! the element edges on those faces, line elements and nodes. Faces shared by two solid elements
//! are inside the mesh and are left out, so a model of a million elements draws its surface, not
//! its interior. Higher-order elements (tet10, hex20, tri6, quad8) draw through their corner
//! nodes. Every element primitive keeps its node labels, so results (displacements, contours)
//! can be drawn on it ([`DisplayMesh::displaced`]).
//!
//! Geometry: triangulated boundary surfaces (tessellated bodies, imported surfaces) added with
//! [`DisplayMesh::add_surface`], drawn with the edges between their faces.
//!
//! The output is deterministic: elements come out in label order, edges sorted.

use fieldcraft_model::{ElementKind, Model};

use crate::{Aabb3, Vec3};

/// One drawn triangle of an element face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayTri {
    pub p: [Vec3; 3],
    /// The node labels at the corners.
    pub nodes: [u32; 3],
    /// The face's unit normal, pointing out of its element (a shell's own orientation).
    pub normal: Vec3,
    pub kind: ElementKind,
}

/// An element edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayEdge {
    pub p: [Vec3; 2],
    pub nodes: [u32; 2],
}

/// A line element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayLine {
    pub a: Vec3,
    pub b: Vec3,
    pub nodes: [u32; 2],
    pub kind: ElementKind,
}

/// A node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayNode {
    pub label: u32,
    pub p: Vec3,
}

/// A triangle of a geometry surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTri {
    pub p: [Vec3; 3],
    pub normal: Vec3,
}

/// Display data for a model and its geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayMesh {
    /// Visible element faces as triangles (quads are split in two).
    pub tris: Vec<DisplayTri>,
    /// Element edges on the visible faces, each once.
    pub edges: Vec<DisplayEdge>,
    /// Line elements (rods, beams).
    pub lines: Vec<DisplayLine>,
    /// Every node, in label order.
    pub nodes: Vec<DisplayNode>,
    /// Geometry surface triangles.
    pub surface_tris: Vec<SurfaceTri>,
    /// Geometry edges: where two faces meet, and open boundaries.
    pub surface_edges: Vec<[Vec3; 2]>,
    pub bounds: Aabb3,
}

/// The faces of an element as corner indices (0-based into its node list), each wound
/// counter-clockwise seen from outside for a positively oriented element.
fn faces(kind: ElementKind) -> &'static [&'static [usize]] {
    match kind {
        ElementKind::Rod2 | ElementKind::Rod3 | ElementKind::Beam2 => &[],
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
        for (&label, n) in model.nodes() {
            let p = Vec3::from(n.xyz);
            out.nodes.push(DisplayNode { label, p });
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
                if let (Some(&a), Some(&b), Some(&na), Some(&nb)) = (at.first(), at.get(1), e.nodes.first(), e.nodes.get(1)) {
                    out.lines.push(DisplayLine { a, b, nodes: [na, nb], kind: e.kind });
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
                let labels: Vec<u32> = f.iter().filter_map(|&i| e.nodes.get(i).copied()).collect();
                let Some(mut normal) = polygon_normal(&p) else { continue };
                if solid && normal.dot(centre(&p) - element_centre) < 0.0 {
                    normal = -normal;
                }
                for k in 1..p.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c), Some(&la), Some(&lb), Some(&lc)) =
                        (p.first(), p.get(k), p.get(k + 1), labels.first(), labels.get(k), labels.get(k + 1))
                    {
                        out.tris.push(DisplayTri { p: [a, b, c], nodes: [la, lb, lc], normal, kind: e.kind });
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
        out.edges = edges.into_iter().filter_map(|(a, b)| Some(DisplayEdge { p: [pos(a)?, pos(b)?], nodes: [a, b] })).collect();
        out
    }

    /// Add a triangulated geometry surface: `positions`, counter-clockwise `triangles` seen from
    /// outside, and the face each triangle lies on. Edges are drawn where triangles of different
    /// faces meet and along open boundaries; degenerate triangles and out-of-range indices are
    /// skipped.
    pub fn add_surface(&mut self, positions: &[[f64; 3]], triangles: &[[u32; 3]], tri_face: &[u32]) {
        let pos = |i: u32| positions.get(i as usize).map(|p| Vec3::from(*p)).filter(|p| p.is_finite());
        // Tessellations may repeat a point for each triangle or face: copies with identical
        // coordinates are one vertex (the lowest index of the copies).
        let mut order: Vec<u32> = (0..u32::try_from(positions.len()).unwrap_or(u32::MAX)).collect();
        let coords = |i: &u32| positions.get(*i as usize).copied().unwrap_or([f64::NAN; 3]);
        order.sort_by(|a, b| {
            let (p, q) = (coords(a), coords(b));
            p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1])).then(p[2].total_cmp(&q[2])).then(a.cmp(b))
        });
        let mut vertex: Vec<u32> = (0..order.len()).map(|i| i as u32).collect();
        for run in order.chunk_by(|a, b| coords(a).map(f64::to_bits) == coords(b).map(f64::to_bits)) {
            let first = run.first().copied().unwrap_or(0);
            for &i in run {
                if let Some(v) = vertex.get_mut(i as usize) {
                    *v = first;
                }
            }
        }
        let vertex = |i: u32| vertex.get(i as usize).copied().unwrap_or(i);
        // (low vertex, high vertex, face) per triangle side; a side alone, or sides on
        // different faces, make an edge to draw.
        let mut sides: Vec<(u32, u32, u32)> = Vec::with_capacity(triangles.len().saturating_mul(3));
        for (t, &[i0, i1, i2]) in triangles.iter().enumerate() {
            let (Some(a), Some(b), Some(c)) = (pos(i0), pos(i1), pos(i2)) else { continue };
            let Some(normal) = (b - a).cross(c - a).normalized() else { continue };
            for p in [a, b, c] {
                self.bounds.add(p);
            }
            self.surface_tris.push(SurfaceTri { p: [a, b, c], normal });
            let face = tri_face.get(t).copied().unwrap_or(u32::MAX);
            let [v0, v1, v2] = [vertex(i0), vertex(i1), vertex(i2)];
            for (u, v) in [(v0, v1), (v1, v2), (v2, v0)] {
                sides.push((u.min(v), u.max(v), face));
            }
        }
        sides.sort_unstable();
        for run in sides.chunk_by(|a, b| a.0 == b.0 && a.1 == b.1) {
            let Some(&(u, v, face)) = run.first() else { continue };
            if (run.len() == 1 || run.iter().any(|s| s.2 != face))
                && let (Some(a), Some(b)) = (pos(u), pos(v))
            {
                self.surface_edges.push([a, b]);
            }
        }
    }

    /// The node nearest to where a ray (`origin`, `dir`) first hits an element face ahead of
    /// its origin: the hit face's closest corner, and the hit point. Line elements and geometry
    /// are not hit.
    pub fn pick(&self, origin: Vec3, dir: Vec3) -> Option<(u32, Vec3)> {
        let mut best: Option<(f64, &DisplayTri)> = None;
        for t in &self.tris {
            // Möller–Trumbore, both sides.
            let (e1, e2) = (t.p[1] - t.p[0], t.p[2] - t.p[0]);
            let h = dir.cross(e2);
            let det = e1.dot(h);
            if !(det.abs() > 1e-300) {
                continue;
            }
            let s = origin - t.p[0];
            let u = s.dot(h) / det;
            let q = s.cross(e1);
            let v = dir.dot(q) / det;
            let d = e2.dot(q) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && d >= 0.0 && d.is_finite() && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, t));
            }
        }
        let (d, t) = best?;
        let hit = origin + dir * d;
        let (i, _) = t.p.iter().enumerate().min_by(|a, b| a.1.dist(hit).total_cmp(&b.1.dist(hit)))?;
        Some((*t.nodes.get(i)?, hit))
    }

    /// The elements moved by `offset(label)` (no offset for a node it has none for): faces,
    /// edges, line elements and nodes move, face normals follow the deformation. Geometry
    /// surfaces are left out (they do not deform).
    pub fn displaced(&self, offset: impl Fn(u32) -> Option<Vec3>) -> DisplayMesh {
        let at = |p: Vec3, label: u32| p + offset(label).filter(|d| d.is_finite()).unwrap_or(Vec3::ZERO);
        let tris = self
            .tris
            .iter()
            .map(|t| {
                let p = [at(t.p[0], t.nodes[0]), at(t.p[1], t.nodes[1]), at(t.p[2], t.nodes[2])];
                // The triangle's own normal, on the side its face's normal was.
                let before = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]);
                let side = if before.dot(t.normal) < 0.0 { -1.0 } else { 1.0 };
                let normal = ((p[1] - p[0]).cross(p[2] - p[0]) * side).normalized().unwrap_or(t.normal);
                DisplayTri { p, normal, ..*t }
            })
            .collect();
        let mut out = DisplayMesh {
            tris,
            edges: self.edges.iter().map(|e| DisplayEdge { p: [at(e.p[0], e.nodes[0]), at(e.p[1], e.nodes[1])], nodes: e.nodes }).collect(),
            lines: self.lines.iter().map(|l| DisplayLine { a: at(l.a, l.nodes[0]), b: at(l.b, l.nodes[1]), ..*l }).collect(),
            nodes: self.nodes.iter().map(|n| DisplayNode { label: n.label, p: at(n.p, n.label) }).collect(),
            ..Default::default()
        };
        for n in &out.nodes {
            out.bounds.add(n.p);
        }
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
        assert_eq!(d.lines[0].nodes, [4, 5]);
        assert!(d.tris.iter().all(|t| t.nodes.iter().zip(t.p).all(|(&l, p)| Vec3::from(m.node(l).unwrap().xyz) == p)));
    }

    #[test]
    fn surfaces_draw_their_face_boundaries() {
        // A unit square in two faces (one triangle each), plus a degenerate and an out-of-range
        // triangle, which are skipped.
        let p = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
        let mut d = DisplayMesh::default();
        d.add_surface(&p, &[[0, 1, 2], [0, 2, 3], [0, 0, 1], [0, 1, 9]], &[0, 1, 1, 1]);
        assert_eq!(d.surface_tris.len(), 2);
        // Four open sides and the diagonal between the two faces.
        assert_eq!(d.surface_edges.len(), 5);
        let mut one_face = DisplayMesh::default();
        one_face.add_surface(&p, &[[0, 1, 2], [0, 2, 3]], &[0, 0]);
        assert_eq!(one_face.surface_edges.len(), 4);
        assert_eq!(one_face.bounds.max, Vec3::new(1.0, 1.0, 0.0));
        // A triangle soup (each triangle with its own copies of the corners) is welded first.
        let soup = [p[0], p[1], p[2], p[0], p[2], p[3]];
        let mut welded = DisplayMesh::default();
        welded.add_surface(&soup, &[[0, 1, 2], [3, 4, 5]], &[0, 0]);
        assert_eq!(welded.surface_edges.len(), 4);
        let mut hostile = DisplayMesh::default();
        hostile.add_surface(&[[f64::NAN, 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[[0, 1, 2], [u32::MAX, 0, 1]], &[]);
        assert!(hostile.surface_tris.is_empty() && hostile.surface_edges.is_empty());
    }

    #[test]
    fn displacing_moves_elements_by_node_and_keeps_normals_outward() {
        let d = DisplayMesh::from_model(&block(2, 1, 1));
        // Stretch along x by 50 %: node labels carry the offset to every primitive.
        let m = block(2, 1, 1);
        let moved = d.displaced(|label| m.node(label).map(|n| Vec3::new(n.xyz[0] * 0.5, 0.0, 0.0)));
        assert_eq!(moved.tris.len(), d.tris.len());
        assert_eq!(moved.bounds.max, Vec3::new(3.0, 1.0, 1.0));
        assert!(moved.edges.iter().all(|e| e.p[0].x == 1.5 * m.node(e.nodes[0]).unwrap().xyz[0]));
        let c = moved.bounds.center();
        for t in &moved.tris {
            let fc = (t.p[0] + t.p[1] + t.p[2]) * (1.0 / 3.0);
            assert!(t.normal.dot(fc - c) > 0.0, "{t:?}");
        }
        // No offset, or a non-finite one, leaves a node where it is.
        let same = d.displaced(|label| (label == 1).then_some(Vec3::new(f64::NAN, 0.0, 0.0)));
        assert_eq!(same.tris, d.tris);
    }

    #[test]
    fn picking_finds_the_nearest_corner_of_the_face_hit_first() {
        let m = block(2, 1, 1);
        let d = DisplayMesh::from_model(&m);
        // From above, aimed near the corner (2, 1, 1): the top face is hit, not the bottom.
        let (label, hit) = d.pick(Vec3::new(1.9, 0.9, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
        assert_eq!(m.node(label).unwrap().xyz, [2.0, 1.0, 1.0]);
        assert!((hit.z - 1.0).abs() < 1e-12);
        // From below the bottom face wins.
        let (label, _) = d.pick(Vec3::new(0.1, 0.1, -10.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        assert_eq!(m.node(label).unwrap().xyz, [0.0, 0.0, 0.0]);
        assert_eq!(d.pick(Vec3::new(5.0, 5.0, 10.0), Vec3::new(0.0, 0.0, -1.0)), None);
        assert_eq!(d.pick(Vec3::new(f64::NAN, 0.0, 0.0), Vec3::new(0.0, 0.0, -1.0)), None);
    }

    #[test]
    fn empty_and_deterministic() {
        assert_eq!(DisplayMesh::from_model(&Model::default()), DisplayMesh::default());
        let m = block(2, 2, 1);
        assert_eq!(DisplayMesh::from_model(&m), DisplayMesh::from_model(&m.clone()));
    }
}
