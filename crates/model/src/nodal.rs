//! Consistent nodal loads: turning a force, moment or traction on a set into the nodal values a
//! deck or the solver applies, the same way for every consumer.
//!
//! A load over a surface is spread by the element shape functions: node `i` of a face takes
//! `∫ N_i dA` of a unit traction, integrated by Gauss quadrature on the face's own (possibly
//! curved, quadratic) geometry. The weights of a face sum to its area, so a total force split by
//! them keeps its total on any mesh of the same surface. Quadratic faces give the textbook
//! values (corner 0 and mid-side A/3 on a flat six-node triangle; corner −A/12 and mid-side A/3 on
//! a flat eight-node quadrilateral).

use std::collections::BTreeMap;

use crate::{ElementFace, Error, Model, Result};

/// Shape functions and their derivatives at one point of a reference face.
struct Shape {
    n: Vec<f64>,
    dxi: Vec<f64>,
    deta: Vec<f64>,
}

/// Triangle: barycentric L0 = 1 − ξ − η, L1 = ξ, L2 = η.
fn triangle(nodes: usize, xi: f64, eta: f64) -> Shape {
    let l = [1.0 - xi - eta, xi, eta];
    let dl = [(-1.0, -1.0), (1.0, 0.0), (0.0, 1.0)];
    let mut s = Shape { n: Vec::with_capacity(nodes), dxi: Vec::with_capacity(nodes), deta: Vec::with_capacity(nodes) };
    if nodes == 3 {
        for (li, (dx, de)) in l.iter().zip(dl) {
            s.n.push(*li);
            s.dxi.push(dx);
            s.deta.push(de);
        }
        return s;
    }
    for (li, (dx, de)) in l.iter().zip(dl) {
        s.n.push(li * (2.0 * li - 1.0));
        s.dxi.push((4.0 * li - 1.0) * dx);
        s.deta.push((4.0 * li - 1.0) * de);
    }
    for (la, lb, (dxa, dea), (dxb, deb)) in [(l[0], l[1], dl[0], dl[1]), (l[1], l[2], dl[1], dl[2]), (l[2], l[0], dl[2], dl[0])] {
        s.n.push(4.0 * la * lb);
        s.dxi.push(4.0 * (dxa * lb + la * dxb));
        s.deta.push(4.0 * (dea * lb + la * deb));
    }
    s
}

/// Quadrilateral on [−1, 1]², corners (−1,−1), (1,−1), (1,1), (−1,1).
fn quadrilateral(nodes: usize, xi: f64, eta: f64) -> Shape {
    const CORNERS: [(f64, f64); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    let mut s = Shape { n: Vec::with_capacity(nodes), dxi: Vec::with_capacity(nodes), deta: Vec::with_capacity(nodes) };
    for (xi_i, eta_i) in CORNERS {
        let (a, b) = (1.0 + xi * xi_i, 1.0 + eta * eta_i);
        if nodes == 4 {
            s.n.push(a * b / 4.0);
            s.dxi.push(xi_i * b / 4.0);
            s.deta.push(eta_i * a / 4.0);
        } else {
            let c = xi * xi_i + eta * eta_i - 1.0;
            s.n.push(a * b * c / 4.0);
            s.dxi.push(xi_i * b * (2.0 * xi * xi_i + eta * eta_i) / 4.0);
            s.deta.push(eta_i * a * (xi * xi_i + 2.0 * eta * eta_i) / 4.0);
        }
    }
    if nodes == 8 {
        // Mid-sides of edges 1-2 (η = −1), 2-3 (ξ = 1), 3-4 (η = 1), 4-1 (ξ = −1).
        for (xi_i, eta_i) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
            if xi_i == 0.0 {
                s.n.push((1.0 - xi * xi) * (1.0 + eta * eta_i) / 2.0);
                s.dxi.push(-xi * (1.0 + eta * eta_i));
                s.deta.push(eta_i * (1.0 - xi * xi) / 2.0);
            } else {
                s.n.push((1.0 + xi * xi_i) * (1.0 - eta * eta) / 2.0);
                s.dxi.push(xi_i * (1.0 - eta * eta) / 2.0);
                s.deta.push(-eta * (1.0 + xi * xi_i));
            }
        }
    }
    s
}

/// Gauss points `(ξ, η, weight)`: a degree-4 six-point rule on the triangle (Dunavant 1985) and
/// 3 × 3 Gauss–Legendre on the quadrilateral; both integrate the quadratic faces' weights exactly
/// when the face is flat.
fn rule(corners: usize) -> Vec<(f64, f64, f64)> {
    if corners == 3 {
        let (a1, w1) = (0.445_948_490_915_965, 0.223_381_589_678_011 / 2.0);
        let (a2, w2) = (0.091_576_213_509_771, 0.109_951_743_655_322 / 2.0);
        let (b1, b2) = (1.0 - 2.0 * a1, 1.0 - 2.0 * a2);
        vec![(a1, a1, w1), (a1, b1, w1), (b1, a1, w1), (a2, a2, w2), (a2, b2, w2), (b2, a2, w2)]
    } else {
        let g = (0.6f64).sqrt();
        let pts = [(-g, 5.0 / 9.0), (0.0, 8.0 / 9.0), (g, 5.0 / 9.0)];
        pts.iter().flat_map(|&(x, wx)| pts.iter().map(move |&(y, wy)| (x, y, wx * wy))).collect()
    }
}

impl Model {
    /// The nodal weights `∫ N_i dA` of a set of element faces, summed per node. Their sum is the
    /// faces' area.
    pub fn surface_weights(&self, faces: &[ElementFace]) -> Result<BTreeMap<u32, f64>> {
        let mut out: BTreeMap<u32, f64> = BTreeMap::new();
        for &f in faces {
            let labels = self.face_nodes(f).ok_or_else(|| Error::NotFound(format!("element {} face {}", f.element, f.face)))?;
            let xyz: Vec<[f64; 3]> =
                labels.iter().map(|n| self.node(*n).map(|p| p.xyz).ok_or_else(|| Error::NotFound(format!("node {n}")))).collect::<Result<_>>()?;
            let (corners, shape): (usize, fn(usize, f64, f64) -> Shape) = match labels.len() {
                3 | 6 => (3, triangle),
                4 | 8 => (4, quadrilateral),
                n => return Err(Error::Invalid(format!("element {} face {} has {n} nodes, not a surface face", f.element, f.face))),
            };
            for (xi, eta, w) in rule(corners) {
                let s = shape(labels.len(), xi, eta);
                let mut tx = [0.0; 3];
                let mut te = [0.0; 3];
                for ((p, dx), de) in xyz.iter().zip(&s.dxi).zip(&s.deta) {
                    for k in 0..3 {
                        tx[k] += dx * p[k];
                        te[k] += de * p[k];
                    }
                }
                let c = [tx[1] * te[2] - tx[2] * te[1], tx[2] * te[0] - tx[0] * te[2], tx[0] * te[1] - tx[1] * te[0]];
                let da = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() * w;
                for (label, n) in labels.iter().zip(&s.n) {
                    *out.entry(*label).or_default() += n * da;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ElementKind;

    /// A flat 2 × 3 face of a hex8 or hex20 (face 1, z = 0), and its weights.
    fn face_weights(kind: ElementKind) -> Vec<f64> {
        let mut m = Model::default();
        let corners = [[0., 0., 0.], [2., 0., 0.], [2., 3., 0.], [0., 3., 0.], [0., 0., 1.], [2., 0., 1.], [2., 3., 1.], [0., 3., 1.]];
        let mut xyz = corners.to_vec();
        for e in kind.edges() {
            if let [a, b, _] = **e {
                xyz.push([0, 1, 2].map(|i| (corners[a][i] + corners[b][i]) / 2.0));
            }
        }
        for (i, p) in xyz.iter().enumerate() {
            m.set_node(i as u32 + 1, *p).unwrap();
        }
        m.set_element(1, kind, (1..=kind.node_count() as u32).collect()).unwrap();
        let w = m.surface_weights(&[ElementFace { element: 1, face: 1 }]).unwrap();
        m.face_nodes(ElementFace { element: 1, face: 1 }).unwrap().iter().map(|n| w[n]).collect()
    }

    #[test]
    fn weights_are_the_consistent_nodal_loads() {
        let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(close(&face_weights(ElementKind::Hex8), &[1.5; 4]), "{:?}", face_weights(ElementKind::Hex8));
        let quad8 = face_weights(ElementKind::Hex20);
        assert!(close(&quad8, &[-0.5, -0.5, -0.5, -0.5, 2.0, 2.0, 2.0, 2.0]), "{quad8:?}");

        // Triangles: a tet4 and a tet10 face (face 1, the right triangle of legs 1 in z = 0).
        let mut m = Model::default();
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let mut xyz = p.to_vec();
        for e in ElementKind::Tet10.edges() {
            if let [a, b, _] = **e {
                xyz.push([0, 1, 2].map(|i| (p[a][i] + p[b][i]) / 2.0));
            }
        }
        for (i, q) in xyz.iter().enumerate() {
            m.set_node(i as u32 + 1, *q).unwrap();
        }
        m.set_element(1, ElementKind::Tet4, vec![1, 2, 3, 4]).unwrap();
        m.set_element(2, ElementKind::Tet10, (1..=10).collect()).unwrap();
        let w = m.surface_weights(&[ElementFace { element: 1, face: 1 }]).unwrap();
        assert!(close(&w.values().copied().collect::<Vec<_>>(), &[1.0 / 6.0; 3]), "{w:?}");
        let w = m.surface_weights(&[ElementFace { element: 2, face: 1 }]).unwrap();
        let tri6: Vec<f64> = m.face_nodes(ElementFace { element: 2, face: 1 }).unwrap().iter().map(|n| w[n]).collect();
        assert!(close(&tri6, &[0.0, 0.0, 0.0, 1.0 / 6.0, 1.0 / 6.0, 1.0 / 6.0]), "{tri6:?}");
    }
}
