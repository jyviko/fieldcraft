//! Shape functions and their derivatives in natural coordinates.
//!
//! Node order is the model's (Abaqus/VTK, [`modelcraft_model::ElementKind`]):
//! - tetrahedra on the reference element with corners (0,0,0), (1,0,0), (0,1,0), (0,0,1) and the
//!   volume coordinates L₁ = 1 − ξ − η − ζ, L₂ = ξ, L₃ = η, L₄ = ζ; tet10 mid-side nodes on edges
//!   1-2, 2-3, 3-1, 1-4, 2-4, 3-4 with N = 4 Lₐ L_b, corners N = L(2L − 1);
//! - hexahedra on [−1, 1]³, corners 1–4 at ζ = −1 counter-clockwise from (−1,−1), 5–8 above
//!   them; hex20 is the serendipity element (corners
//!   N = ⅛(1 + ξξᵢ)(1 + ηηᵢ)(1 + ζζᵢ)(ξξᵢ + ηηᵢ + ζζᵢ − 2), a mid-side node with ξᵢ = 0
//!   N = ¼(1 − ξ²)(1 + ηηᵢ)(1 + ζζᵢ), and likewise), mid-side nodes 9–12 on the bottom edges,
//!   13–16 on the top edges, 17–20 on the vertical edges (mid-side positions come from
//!   [`modelcraft_model::ElementKind::edges`], the one place the node order is written down).
//!
//! The functions are the textbook ones (O. C. Zienkiewicz, R. L. Taylor, J. Z. Zhu, *The Finite
//! Element Method: Its Basis and Fundamentals*, 7th ed., Butterworth-Heinemann 2013, ch. 6;
//! T. J. R. Hughes, *The Finite Element Method*, Dover 2000, ch. 3).

use modelcraft_model::ElementKind;

/// Most nodes an element in this crate has (hex20).
pub const MAX_NODES: usize = 20;

/// Shape function values and natural derivatives at one point.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub count: usize,
    pub n: [f64; MAX_NODES],
    /// ∂Nᵢ/∂ξ, ∂Nᵢ/∂η, ∂Nᵢ/∂ζ.
    pub dn: [[f64; 3]; MAX_NODES],
}

impl Shape {
    fn new(count: usize) -> Shape {
        Shape { count, n: [0.0; MAX_NODES], dn: [[0.0; 3]; MAX_NODES] }
    }

    pub fn values(&self) -> &[f64] {
        self.n.get(..self.count).unwrap_or_default()
    }

    pub fn derivatives(&self) -> &[[f64; 3]] {
        self.dn.get(..self.count).unwrap_or_default()
    }
}

/// Natural coordinates of the hexahedron's corners.
pub const HEX_CORNERS: [[f64; 3]; 8] =
    [[-1., -1., -1.], [1., -1., -1.], [1., 1., -1.], [-1., 1., -1.], [-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]];

/// Natural coordinates of the tetrahedron's corners.
pub const TET_CORNERS: [[f64; 3]; 4] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

/// Four-node tetrahedron.
pub fn tet4(xi: [f64; 3]) -> Shape {
    let mut s = Shape::new(4);
    s.n[0] = 1.0 - xi[0] - xi[1] - xi[2];
    s.n[1] = xi[0];
    s.n[2] = xi[1];
    s.n[3] = xi[2];
    s.dn[0] = [-1.0, -1.0, -1.0];
    s.dn[1] = [1.0, 0.0, 0.0];
    s.dn[2] = [0.0, 1.0, 0.0];
    s.dn[3] = [0.0, 0.0, 1.0];
    s
}

/// Ten-node tetrahedron.
pub fn tet10(xi: [f64; 3]) -> Shape {
    let lin = tet4(xi);
    let l = [lin.n[0], lin.n[1], lin.n[2], lin.n[3]];
    let dl = [lin.dn[0], lin.dn[1], lin.dn[2], lin.dn[3]];
    let mut s = Shape::new(10);
    for i in 0..4 {
        s.n[i] = l[i] * (2.0 * l[i] - 1.0);
        s.dn[i] = dl[i].map(|d| (4.0 * l[i] - 1.0) * d);
    }
    // Mid-side nodes on the model's tet10 edges.
    for edge in ElementKind::Tet10.edges() {
        if let [a, b, m] = **edge {
            s.n[m] = 4.0 * l[a] * l[b];
            s.dn[m] = [0, 1, 2].map(|c| 4.0 * (dl[a][c] * l[b] + l[a] * dl[b][c]));
        }
    }
    s
}

/// Eight-node hexahedron (trilinear).
pub fn hex8(xi: [f64; 3]) -> Shape {
    let mut s = Shape::new(8);
    for (i, c) in HEX_CORNERS.iter().enumerate() {
        let f = [0, 1, 2].map(|k| 1.0 + xi[k] * c[k]);
        s.n[i] = f[0] * f[1] * f[2] / 8.0;
        s.dn[i] = [c[0] * f[1] * f[2] / 8.0, c[1] * f[0] * f[2] / 8.0, c[2] * f[0] * f[1] / 8.0];
    }
    s
}

/// Twenty-node serendipity hexahedron.
pub fn hex20(xi: [f64; 3]) -> Shape {
    let mut s = Shape::new(20);
    let [x, y, z] = xi;
    for (i, c) in HEX_CORNERS.iter().enumerate() {
        let f = [0, 1, 2].map(|k| 1.0 + xi[k] * c[k]);
        let g = x * c[0] + y * c[1] + z * c[2] - 2.0;
        s.n[i] = f[0] * f[1] * f[2] * g / 8.0;
        // ∂/∂ξ_k: c_k f_l f_m g / 8 + f_0 f_1 f_2 c_k / 8.
        s.dn[i] = [c[0] * f[1] * f[2] * (g + f[0]) / 8.0, c[1] * f[0] * f[2] * (g + f[1]) / 8.0, c[2] * f[0] * f[1] * (g + f[2]) / 8.0];
    }
    // Mid-side nodes on the model's hex20 edges, at the midpoints of their corners.
    for edge in ElementKind::Hex20.edges() {
        let [a, b, i] = **edge else { continue };
        let c = [0, 1, 2].map(|k| (HEX_CORNERS[a][k] + HEX_CORNERS[b][k]) / 2.0);
        // The direction along the edge has cᵢ = 0: N = ¼(1 − t²)(1 + a aᵢ)(1 + b bᵢ).
        let along = if c[0] == 0.0 {
            0
        } else if c[1] == 0.0 {
            1
        } else {
            2
        };
        let (p, q) = ((along + 1) % 3, (along + 2) % 3);
        let t = xi[along];
        let fp = 1.0 + xi[p] * c[p];
        let fq = 1.0 + xi[q] * c[q];
        s.n[i] = (1.0 - t * t) * fp * fq / 4.0;
        let mut d = [0.0; 3];
        d[along] = -2.0 * t * fp * fq / 4.0;
        d[p] = (1.0 - t * t) * c[p] * fq / 4.0;
        d[q] = (1.0 - t * t) * fp * c[q] / 4.0;
        s.dn[i] = d;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes_of(kind: &str) -> Vec<[f64; 3]> {
        match kind {
            "tet4" => TET_CORNERS.to_vec(),
            "tet10" => {
                let mut v = TET_CORNERS.to_vec();
                for e in ElementKind::Tet10.edges() {
                    v.push([0, 1, 2].map(|k| (TET_CORNERS[e[0]][k] + TET_CORNERS[e[1]][k]) / 2.0));
                }
                v
            }
            "hex8" => HEX_CORNERS.to_vec(),
            _ => {
                let mut v = HEX_CORNERS.to_vec();
                for e in ElementKind::Hex20.edges() {
                    v.push([0, 1, 2].map(|k| (HEX_CORNERS[e[0]][k] + HEX_CORNERS[e[1]][k]) / 2.0));
                }
                v
            }
        }
    }

    #[test]
    fn kronecker_delta_partition_of_unity_and_derivatives() {
        let kinds: [(&str, fn([f64; 3]) -> Shape); 4] = [("tet4", tet4), ("tet10", tet10), ("hex8", hex8), ("hex20", hex20)];
        for (name, f) in kinds {
            let nodes = nodes_of(name);
            for (i, p) in nodes.iter().enumerate() {
                let s = f(*p);
                for (j, v) in s.values().iter().enumerate() {
                    assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-14, "{name} N{j} at node {i}: {v}");
                }
            }
            // At an arbitrary point: ΣN = 1, ΣdN = 0, Σ N x = x (linear completeness), and the
            // derivatives match central differences.
            let p = [0.21, 0.17, 0.33];
            let s = f(p);
            assert!((s.values().iter().sum::<f64>() - 1.0).abs() < 1e-14);
            for k in 0..3 {
                assert!(s.derivatives().iter().map(|d| d[k]).sum::<f64>().abs() < 1e-13);
                let x: f64 = s.values().iter().zip(&nodes).map(|(n, x)| n * x[k]).sum();
                assert!((x - p[k]).abs() < 1e-14, "{name}");
                let h = 1e-6;
                let (mut a, mut b) = (p, p);
                a[k] += h;
                b[k] -= h;
                let (sa, sb) = (f(a), f(b));
                for j in 0..s.count {
                    let fd = (sa.n[j] - sb.n[j]) / (2.0 * h);
                    assert!((fd - s.dn[j][k]).abs() < 1e-8, "{name} dN{j}/dξ{k}: {fd} vs {}", s.dn[j][k]);
                }
            }
        }
    }
}
