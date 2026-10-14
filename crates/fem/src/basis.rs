//! The shape-function library: every element family's interpolation with its first and second
//! derivatives in natural coordinates, and their mapping to physical gradients, Hessians and
//! Laplacians through the isoparametric map.
//!
//! Families ([`Basis`]) and their reference elements (natural coordinates ξ, η, ζ; unused ones
//! are zero):
//! - **lines** on ξ ∈ [−1, 1]: line2 (ends at ξ = −1, 1), line3 (then the middle node at 0);
//! - **triangles** on {ξ, η ≥ 0, ξ + η ≤ 1}: tri3 at (0,0), (1,0), (0,1), area coordinates
//!   L₁ = 1 − ξ − η, L₂ = ξ, L₃ = η; tri6 adds mid-side nodes on edges 1-2, 2-3, 3-1 with
//!   N = 4 Lₐ L_b and corners N = L(2L − 1);
//! - **quadrilaterals** on [−1, 1]²: corners counter-clockwise from (−1, −1); quad4 bilinear,
//!   quad8 serendipity (mid-side nodes on edges 1-2, 2-3, 3-4, 4-1), quad9 biquadratic Lagrange
//!   (quad8 plus the centre);
//! - **tetrahedra**: tet4, tet10 (see [`crate::shape`]);
//! - **wedges**: the triangle times ζ ∈ [−1, 1]; wedge6, and wedge15 with mid-side nodes on the
//!   bottom triangle's edges, the top's, then the vertical edges 1-4, 2-5, 3-6 (the order of
//!   [`ElementKind::Wedge6`]'s edges): corners N = ½ L(2L − 1)(1 + ζζᵢ) − ½ L(1 − ζ²), triangle
//!   mid-sides N = 2 Lₐ L_b (1 + ζζᵢ), vertical mid-sides N = L(1 − ζ²);
//! - **pyramid5**: square base [−1, 1]² at ζ = 0 (corners as quad4), apex (0, 0, 1); the rational
//!   functions N = ¼[(1 − ζ) + ξξᵢ + ηηᵢ + ξᵢηᵢ ξη / (1 − ζ)] and N₅ = ζ, linearly complete and
//!   conforming with neighbouring tetrahedra and hexahedra (L. Bedrosian, *Shape functions and
//!   integration formulas for three-dimensional finite element analysis*, IJNME 35(1):95–108,
//!   1992). At the apex itself the rational term and its derivatives take their limit along the
//!   axis, zero; quadrature points never lie there;
//! - **hexahedra** on [−1, 1]³: hex8 trilinear, hex20 serendipity (see [`crate::shape`]), hex27
//!   triquadratic Lagrange (hex20, then the face centres in the order of [`ElementKind::Hex8`]'s
//!   faces, then the centre).
//!
//! Node orders are corners first, then mid-side nodes in the model's edge order (the model is the
//! one place they are written down), then face and body nodes, the Abaqus/VTK convention of
//! [`fieldcraft_model::ElementKind`]. The functions are the textbook ones (O. C. Zienkiewicz,
//! R. L. Taylor, J. Z. Zhu, *The Finite Element Method: Its Basis and Fundamentals*, 7th ed.,
//! 2013, ch. 6; T. J. R. Hughes, *The Finite Element Method*, Dover 2000, ch. 3).
//!
//! **Derivatives.** [`Basis::eval`] gives values and first derivatives; the families the solid
//! kernels used before the library existed (tet4, tet10, wedge6, hex8, hex20) keep their
//! hand-written functions there, so element results do not change by a bit. [`Basis::eval2`]
//! adds the Hessian ∂²N/∂ξₐ∂ξ_b, computed from the same formulas by second-order automatic
//! differentiation ([`crate::jet`]): exact to round-off for every family.
//!
//! **Physical derivatives** (Hughes 2000, §3.9 and §4.2 for the second derivatives): with
//! J_ab = ∂x_b/∂ξₐ, the gradient is ∇ₓN = J⁻¹ ∇_ξN and the Hessian
//! Hₓ = J⁻¹ (H_ξ − Σ_b (∂N/∂x_b) X_b″) J⁻ᵀ, where X_b″ = Σᵢ x_{i,b} H_ξ(Nᵢ) is the second
//! derivative of the map. On a curved (non-affine) element that last term is what makes a field
//! linear in x have zero Hessian; dropping it is a classic error in residual-based stabilization.
//! A family of dimension d maps onto the first d physical coordinates (plane elements in x–y,
//! lines along x); elements embedded in a higher dimension (shells, beams, trusses in 3D) pass
//! coordinates in their own local frame.

use std::sync::LazyLock;

use fieldcraft_model::ElementKind;

use crate::dense::positive_inverse;
use crate::jet::Jet;
use crate::shape::{self, HEX_CORNERS, MAX_NODES, Shape, TET_CORNERS, WEDGE_CORNERS};
use crate::{Error, Result};

/// An element family's interpolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Basis {
    Line2,
    Line3,
    Tri3,
    Tri6,
    Quad4,
    Quad8,
    Quad9,
    Tet4,
    Tet10,
    Wedge6,
    Wedge15,
    Pyramid5,
    Hex8,
    Hex20,
    Hex27,
}

/// Values, first and second natural derivatives at one point.
#[derive(Debug, Clone, Copy)]
pub struct Shape2 {
    pub shape: Shape,
    /// ∂²Nᵢ/∂ξₐ∂ξ_b.
    pub d2n: [[[f64; 3]; 3]; MAX_NODES],
}

impl Shape2 {
    pub fn hessians(&self) -> &[[[f64; 3]; 3]] {
        self.d2n.get(..self.shape.count).unwrap_or_default()
    }
}

/// Physical gradients at one point.
#[derive(Debug, Clone, Copy)]
pub struct Gradients {
    pub count: usize,
    /// ∂Nᵢ/∂x_b.
    pub dx: [[f64; 3]; MAX_NODES],
    /// det J (a length, area or volume ratio for dimension 1, 2, 3).
    pub det: f64,
}

impl Gradients {
    pub fn gradients(&self) -> &[[f64; 3]] {
        self.dx.get(..self.count).unwrap_or_default()
    }
}

/// Physical gradients and Hessians at one point.
#[derive(Debug, Clone, Copy)]
pub struct Hessians {
    pub gradients: Gradients,
    /// ∂²Nᵢ/∂x_a∂x_b.
    pub d2x: [[[f64; 3]; 3]; MAX_NODES],
}

impl Hessians {
    pub fn hessians(&self) -> &[[[f64; 3]; 3]] {
        self.d2x.get(..self.gradients.count).unwrap_or_default()
    }

    /// ∇²Nᵢ, the trace of the Hessian.
    pub fn laplacian(&self, i: usize) -> f64 {
        self.d2x.get(i).map_or(0.0, |h| h[0][0] + h[1][1] + h[2][2])
    }
}

const LINE_ENDS: [[f64; 3]; 2] = [[-1., 0., 0.], [1., 0., 0.]];
const TRI_CORNERS: [[f64; 3]; 3] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
const QUAD_CORNERS: [[f64; 3]; 4] = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]];
const PYRAMID_CORNERS: [[f64; 3]; 5] = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.], [0., 0., 1.]];

fn midpoint(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|k| (a[k] + b[k]) / 2.0)
}

/// Corners, then the midpoints of `edges` (pairs of corner positions).
fn with_midsides(corners: &[[f64; 3]], edges: &[&[usize]]) -> Vec<[f64; 3]> {
    let mut v = corners.to_vec();
    for e in edges {
        if let (Some(a), Some(b)) = (e.first().and_then(|a| corners.get(*a)), e.get(1).and_then(|b| corners.get(*b))) {
            v.push(midpoint(*a, *b));
        }
    }
    v
}

/// One-dimensional quadratic Lagrange function of the node at t = c ∈ {−1, 0, 1}.
fn lagrange2(t: Jet, c: f64) -> Jet {
    if c < 0.0 {
        t * (t - 1.0) * 0.5
    } else if c > 0.0 {
        t * (t + 1.0) * 0.5
    } else {
        1.0 - t * t
    }
}

impl Basis {
    pub const ALL: [Basis; 15] = [
        Basis::Line2,
        Basis::Line3,
        Basis::Tri3,
        Basis::Tri6,
        Basis::Quad4,
        Basis::Quad8,
        Basis::Quad9,
        Basis::Tet4,
        Basis::Tet10,
        Basis::Wedge6,
        Basis::Wedge15,
        Basis::Pyramid5,
        Basis::Hex8,
        Basis::Hex20,
        Basis::Hex27,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Basis::Line2 => "line2",
            Basis::Line3 => "line3",
            Basis::Tri3 => "tri3",
            Basis::Tri6 => "tri6",
            Basis::Quad4 => "quad4",
            Basis::Quad8 => "quad8",
            Basis::Quad9 => "quad9",
            Basis::Tet4 => "tet4",
            Basis::Tet10 => "tet10",
            Basis::Wedge6 => "wedge6",
            Basis::Wedge15 => "wedge15",
            Basis::Pyramid5 => "pyramid5",
            Basis::Hex8 => "hex8",
            Basis::Hex20 => "hex20",
            Basis::Hex27 => "hex27",
        }
    }

    /// The interpolation of a model element kind (line elements: along their axis).
    pub fn of(kind: ElementKind) -> Basis {
        match kind {
            ElementKind::Rod2 | ElementKind::Beam2 => Basis::Line2,
            ElementKind::Rod3 => Basis::Line3,
            ElementKind::Tri3 => Basis::Tri3,
            ElementKind::Tri6 => Basis::Tri6,
            ElementKind::Quad4 => Basis::Quad4,
            ElementKind::Quad8 => Basis::Quad8,
            ElementKind::Tet4 => Basis::Tet4,
            ElementKind::Tet10 => Basis::Tet10,
            ElementKind::Wedge6 => Basis::Wedge6,
            ElementKind::Hex8 => Basis::Hex8,
            ElementKind::Hex20 => Basis::Hex20,
        }
    }

    /// Dimension of the reference element.
    pub fn dim(self) -> usize {
        match self {
            Basis::Line2 | Basis::Line3 => 1,
            Basis::Tri3 | Basis::Tri6 | Basis::Quad4 | Basis::Quad8 | Basis::Quad9 => 2,
            _ => 3,
        }
    }

    /// Number of nodes (and functions).
    pub fn count(self) -> usize {
        match self {
            Basis::Line2 => 2,
            Basis::Line3 | Basis::Tri3 => 3,
            Basis::Quad4 | Basis::Tet4 => 4,
            Basis::Pyramid5 => 5,
            Basis::Tri6 | Basis::Wedge6 => 6,
            Basis::Quad8 | Basis::Hex8 => 8,
            Basis::Quad9 => 9,
            Basis::Tet10 => 10,
            Basis::Wedge15 => 15,
            Basis::Hex20 => 20,
            Basis::Hex27 => 27,
        }
    }

    /// The corner (vertex) nodes, first in the node list.
    pub fn corners(self) -> usize {
        match self {
            Basis::Line2 | Basis::Line3 => 2,
            Basis::Tri3 | Basis::Tri6 => 3,
            Basis::Quad4 | Basis::Quad8 | Basis::Quad9 | Basis::Tet4 | Basis::Tet10 => 4,
            Basis::Pyramid5 => 5,
            Basis::Wedge6 | Basis::Wedge15 => 6,
            Basis::Hex8 | Basis::Hex20 | Basis::Hex27 => 8,
        }
    }

    /// The degree of the complete polynomials the family reproduces exactly (in natural
    /// coordinates, and in physical ones on an affine map).
    pub fn degree(self) -> usize {
        match self {
            Basis::Line2 | Basis::Tri3 | Basis::Quad4 | Basis::Tet4 | Basis::Wedge6 | Basis::Pyramid5 | Basis::Hex8 => 1,
            _ => 2,
        }
    }

    /// The natural coordinates of the nodes, in node order.
    pub fn nodes(self) -> &'static [[f64; 3]] {
        static TABLES: LazyLock<Vec<Vec<[f64; 3]>>> = LazyLock::new(|| Basis::ALL.iter().map(|b| b.build_nodes()).collect());
        TABLES.get(self as usize).map(Vec::as_slice).unwrap_or_default()
    }

    fn build_nodes(self) -> Vec<[f64; 3]> {
        match self {
            Basis::Line2 => LINE_ENDS.to_vec(),
            Basis::Line3 => with_midsides(&LINE_ENDS, ElementKind::Rod2.edges()),
            Basis::Tri3 => TRI_CORNERS.to_vec(),
            Basis::Tri6 => with_midsides(&TRI_CORNERS, ElementKind::Tri6.edges()),
            Basis::Quad4 => QUAD_CORNERS.to_vec(),
            Basis::Quad8 => with_midsides(&QUAD_CORNERS, ElementKind::Quad8.edges()),
            Basis::Quad9 => {
                let mut v = with_midsides(&QUAD_CORNERS, ElementKind::Quad8.edges());
                v.push([0.0; 3]);
                v
            }
            Basis::Tet4 => TET_CORNERS.to_vec(),
            Basis::Tet10 => with_midsides(&TET_CORNERS, ElementKind::Tet10.edges()),
            Basis::Wedge6 => WEDGE_CORNERS.to_vec(),
            Basis::Wedge15 => with_midsides(&WEDGE_CORNERS, ElementKind::Wedge6.edges()),
            Basis::Pyramid5 => PYRAMID_CORNERS.to_vec(),
            Basis::Hex8 => HEX_CORNERS.to_vec(),
            Basis::Hex20 => with_midsides(&HEX_CORNERS, ElementKind::Hex20.edges()),
            Basis::Hex27 => {
                let mut v = with_midsides(&HEX_CORNERS, ElementKind::Hex20.edges());
                for face in ElementKind::Hex8.faces() {
                    let c = face.iter().filter_map(|i| HEX_CORNERS.get(*i)).fold([0.0; 3], |s, x| [0, 1, 2].map(|k| s[k] + x[k] / 4.0));
                    v.push(c);
                }
                v.push([0.0; 3]);
                v
            }
        }
    }

    /// Values and first natural derivatives at `xi`.
    pub fn eval(self, xi: [f64; 3]) -> Shape {
        match self {
            Basis::Tet4 => shape::tet4(xi),
            Basis::Tet10 => shape::tet10(xi),
            Basis::Wedge6 => shape::wedge6(xi),
            Basis::Hex8 => shape::hex8(xi),
            Basis::Hex20 => shape::hex20(xi),
            _ => self.eval2(xi).shape,
        }
    }

    /// Values, first and second natural derivatives at `xi`.
    pub fn eval2(self, xi: [f64; 3]) -> Shape2 {
        let jets = self.jets(xi);
        let count = self.count();
        let mut out = Shape2 { shape: Shape { count, n: [0.0; MAX_NODES], dn: [[0.0; 3]; MAX_NODES] }, d2n: [[[0.0; 3]; 3]; MAX_NODES] };
        for (i, j) in jets.iter().enumerate().take(count) {
            out.shape.n[i] = j.v;
            out.shape.dn[i] = j.g;
            out.d2n[i] = j.h;
        }
        out
    }

    /// The functions as jets in the natural coordinates (only the first `count` are set).
    fn jets(self, xi: [f64; 3]) -> [Jet; MAX_NODES] {
        let x = Jet::point(xi);
        let dim = self.dim();
        let mut n = [Jet::constant(0.0); MAX_NODES];
        let nodes = self.nodes();
        // Area or volume coordinates of the simplex families.
        let tri = [1.0 - x[0] - x[1], x[0], x[1]];
        let tet = [1.0 - x[0] - x[1] - x[2], x[0], x[1], x[2]];
        let set = |n: &mut [Jet; MAX_NODES], i: usize, v: Jet| {
            if let Some(slot) = n.get_mut(i) {
                *slot = v;
            }
        };
        match self {
            Basis::Line2 | Basis::Quad4 | Basis::Hex8 => {
                for (i, c) in nodes.iter().enumerate() {
                    let v = (0..dim).fold(Jet::constant(1.0), |p, k| p * (1.0 + x[k] * c[k]) * 0.5);
                    set(&mut n, i, v);
                }
            }
            Basis::Line3 | Basis::Quad9 | Basis::Hex27 => {
                for (i, c) in nodes.iter().enumerate() {
                    let v = (0..dim).fold(Jet::constant(1.0), |p, k| p * lagrange2(x[k], c[k]));
                    set(&mut n, i, v);
                }
            }
            Basis::Quad8 | Basis::Hex20 => {
                // Corners ∏(1 + ξc)/2ᵈ (Σ ξc − (d − 1)); a mid-side node with c_k = 0:
                // (1 − ξ_k²) ∏_{l≠k}(1 + ξ_l c_l)/2ᵈ⁻¹.
                for (i, c) in nodes.iter().enumerate() {
                    let along = (0..dim).find(|k| c[*k] == 0.0);
                    let v = match along {
                        None => {
                            let p = (0..dim).fold(Jet::constant(1.0), |p, k| p * (1.0 + x[k] * c[k]) * 0.5);
                            let s = (0..dim).fold(Jet::constant(-(dim as f64 - 1.0)), |s, k| s + x[k] * c[k]);
                            p * s
                        }
                        Some(a) => (0..dim).filter(|k| *k != a).fold(1.0 - x[a] * x[a], |p, k| p * (1.0 + x[k] * c[k]) * 0.5),
                    };
                    set(&mut n, i, v);
                }
            }
            Basis::Tri3 => {
                for (i, l) in tri.iter().enumerate() {
                    set(&mut n, i, *l);
                }
            }
            Basis::Tet4 => {
                for (i, l) in tet.iter().enumerate() {
                    set(&mut n, i, *l);
                }
            }
            Basis::Tri6 | Basis::Tet10 => {
                let (l, edges): (&[Jet], _) =
                    if self == Basis::Tri6 { (&tri, ElementKind::Tri6.edges()) } else { (&tet, ElementKind::Tet10.edges()) };
                for (i, li) in l.iter().enumerate() {
                    set(&mut n, i, *li * (*li * 2.0 - 1.0));
                }
                for e in edges {
                    if let [a, b, m] = **e
                        && let (Some(la), Some(lb)) = (l.get(a), l.get(b))
                    {
                        set(&mut n, m, *la * *lb * 4.0);
                    }
                }
            }
            Basis::Wedge6 => {
                for i in 0..3 {
                    set(&mut n, i, tri[i] * (1.0 - x[2]) * 0.5);
                    set(&mut n, i + 3, tri[i] * (1.0 + x[2]) * 0.5);
                }
            }
            Basis::Wedge15 => {
                let bubble = 1.0 - x[2] * x[2];
                for i in 0..6 {
                    let (l, z) = (tri[i % 3], if i < 3 { -1.0 } else { 1.0 });
                    set(&mut n, i, l * (l * 2.0 - 1.0) * (1.0 + x[2] * z) * 0.5 - l * bubble * 0.5);
                }
                for (e, edge) in ElementKind::Wedge6.edges().iter().enumerate() {
                    let [a, b] = **edge else { continue };
                    let v = if e < 6 {
                        let z = if a < 3 { -1.0 } else { 1.0 };
                        tri[a % 3] * tri[b % 3] * (1.0 + x[2] * z) * 2.0
                    } else {
                        tri[a % 3] * bubble
                    };
                    set(&mut n, 6 + e, v);
                }
            }
            Basis::Pyramid5 => {
                let d = 1.0 - x[2];
                let r = if d.v.abs() > 0.0 { x[0] * x[1] / d } else { Jet::constant(0.0) };
                for (i, c) in PYRAMID_CORNERS.iter().take(4).enumerate() {
                    set(&mut n, i, (d + x[0] * c[0] + x[1] * c[1] + r * (c[0] * c[1])) * 0.25);
                }
                set(&mut n, 4, x[2]);
            }
        }
        n
    }

    /// The Jacobian J_ab = ∂x_b/∂ξₐ over the family's dimensions, the identity in the others.
    fn jacobian(self, s: &Shape, xyz: &[[f64; 3]]) -> Result<[[f64; 3]; 3]> {
        if xyz.len() < s.count {
            return Err(Error::Invalid(format!("a {} element has {} nodes, got {} coordinates", self.name(), s.count, xyz.len())));
        }
        let dim = self.dim();
        let mut j = [[0.0; 3]; 3];
        for (d, x) in s.derivatives().iter().zip(xyz) {
            for a in 0..dim {
                for b in 0..dim {
                    j[a][b] += d[a] * x[b];
                }
            }
        }
        for a in dim..3 {
            j[a][a] = 1.0;
        }
        Ok(j)
    }

    fn degenerate(self) -> Error {
        Error::Degenerate(format!("a {} element is inverted or degenerate (its Jacobian is not positive)", self.name()))
    }

    /// Physical gradients at a point from its natural shape (`eval`) and the node coordinates.
    pub fn gradients(self, s: &Shape, xyz: &[[f64; 3]]) -> Result<Gradients> {
        self.mapped(s, xyz).map(|m| m.0)
    }

    /// The physical gradients and J⁻¹.
    fn mapped(self, s: &Shape, xyz: &[[f64; 3]]) -> Result<(Gradients, [[f64; 3]; 3])> {
        let (inv, det) = positive_inverse(&self.jacobian(s, xyz)?).ok_or_else(|| self.degenerate())?;
        let mut dx = [[0.0; 3]; MAX_NODES];
        for (g, d) in dx.iter_mut().zip(s.derivatives()) {
            for b in 0..3 {
                g[b] = inv[b][0] * d[0] + inv[b][1] * d[1] + inv[b][2] * d[2];
            }
        }
        Ok((Gradients { count: s.count, dx, det }, inv))
    }

    /// Physical gradients and Hessians from a point's `eval2` and the node coordinates, with the
    /// second-derivative term of the map.
    pub fn hessians(self, s: &Shape2, xyz: &[[f64; 3]]) -> Result<Hessians> {
        let (gradients, inv) = self.mapped(&s.shape, xyz)?;
        let dim = self.dim();
        // X_b″ = Σᵢ x_{i,b} H_ξ(Nᵢ) for the mapped coordinates b < dim.
        let mut map2 = [[[0.0; 3]; 3]; 3];
        for (h, x) in s.hessians().iter().zip(xyz) {
            for b in 0..dim {
                for p in 0..3 {
                    for q in 0..3 {
                        map2[b][p][q] += x[b] * h[p][q];
                    }
                }
            }
        }
        let mut d2x = [[[0.0; 3]; 3]; MAX_NODES];
        for ((out, h), g) in d2x.iter_mut().zip(s.hessians()).zip(gradients.gradients()) {
            // A = H_ξ − Σ_b g_b X_b″, then Hₓ = J⁻¹ A J⁻ᵀ.
            let mut a = *h;
            for b in 0..dim {
                for p in 0..3 {
                    for q in 0..3 {
                        a[p][q] -= g[b] * map2[b][p][q];
                    }
                }
            }
            for r in 0..3 {
                for c in 0..3 {
                    let mut v = 0.0;
                    for p in 0..3 {
                        for q in 0..3 {
                            v += inv[r][p] * a[p][q] * inv[c][q];
                        }
                    }
                    out[r][c] = v;
                }
            }
        }
        Ok(Hessians { gradients, d2x })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A point inside every family's reference element.
    const P: [f64; 3] = [0.21, 0.17, 0.33];

    fn point(b: Basis) -> [f64; 3] {
        match b.dim() {
            1 => [P[0], 0.0, 0.0],
            2 => [P[0], P[1], 0.0],
            _ => P,
        }
    }

    /// Monomials ξᵃ ηᵇ ζᶜ of total degree ≤ `degree` in the family's dimensions, with their exact
    /// gradient and Hessian.
    fn monomials(dim: usize, degree: usize) -> Vec<[usize; 3]> {
        let mut out = Vec::new();
        for a in 0..=degree {
            for b in 0..=degree {
                for c in 0..=degree {
                    let e = [a, b, c];
                    if a + b + c <= degree && (dim..3).all(|k| e[k] == 0) {
                        out.push(e);
                    }
                }
            }
        }
        out
    }

    fn monomial(e: [usize; 3], x: [f64; 3]) -> (f64, [f64; 3], [[f64; 3]; 3]) {
        let p = |k: usize, d: usize| -> f64 {
            // d-th derivative of x_k^{e_k}.
            let n = e[k];
            if d > n {
                return 0.0;
            }
            let coef: f64 = (0..d).map(|i| (n - i) as f64).product();
            coef * x[k].powi((n - d) as i32)
        };
        let v = p(0, 0) * p(1, 0) * p(2, 0);
        let mut g = [0.0; 3];
        let mut h = [[0.0; 3]; 3];
        for a in 0..3 {
            let mut d = [0usize; 3];
            d[a] += 1;
            g[a] = p(0, d[0]) * p(1, d[1]) * p(2, d[2]);
            for b in 0..3 {
                let mut dd = d;
                dd[b] += 1;
                h[a][b] = p(0, dd[0]) * p(1, dd[1]) * p(2, dd[2]);
            }
        }
        (v, g, h)
    }

    #[test]
    fn kronecker_delta_and_partition_of_unity() {
        for (i, b) in Basis::ALL.into_iter().enumerate() {
            assert_eq!(b as usize, i, "Basis::ALL is in declaration order (node tables)");
            let nodes = b.nodes();
            assert_eq!(nodes.len(), b.count(), "{}", b.name());
            for (i, p) in nodes.iter().enumerate() {
                for s in [b.eval(*p), b.eval2(*p).shape] {
                    for (j, v) in s.values().iter().enumerate() {
                        assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-14, "{} N{j} at node {i}: {v}", b.name());
                    }
                }
            }
            let s = b.eval2(point(b));
            assert!((s.shape.values().iter().sum::<f64>() - 1.0).abs() < 1e-14, "{}", b.name());
            for a in 0..3 {
                assert!(s.shape.derivatives().iter().map(|d| d[a]).sum::<f64>().abs() < 1e-13, "{}", b.name());
                for c in 0..3 {
                    assert!(s.hessians().iter().map(|h| h[a][c]).sum::<f64>().abs() < 1e-12, "{}", b.name());
                }
            }
        }
    }

    #[test]
    fn complete_polynomials_and_their_derivatives_are_reproduced() {
        for b in Basis::ALL {
            let nodes = b.nodes();
            let xi = point(b);
            let s = b.eval2(xi);
            for e in monomials(b.dim(), b.degree()) {
                let (v, g, h) = monomial(e, xi);
                let at = |i: usize| monomial(e, nodes[i]).0;
                let iv: f64 = (0..b.count()).map(|i| s.shape.n[i] * at(i)).sum();
                assert!((iv - v).abs() < 1e-13, "{} {e:?}: {iv} vs {v}", b.name());
                for a in 0..3 {
                    let ig: f64 = (0..b.count()).map(|i| s.shape.dn[i][a] * at(i)).sum();
                    assert!((ig - g[a]).abs() < 1e-12, "{} {e:?} d{a}: {ig} vs {}", b.name(), g[a]);
                    for c in 0..3 {
                        let ih: f64 = (0..b.count()).map(|i| s.d2n[i][a][c] * at(i)).sum();
                        assert!((ih - h[a][c]).abs() < 1e-11, "{} {e:?} d{a}{c}: {ih} vs {}", b.name(), h[a][c]);
                    }
                }
            }
        }
    }

    #[test]
    fn derivatives_match_finite_differences() {
        let step = 1e-5;
        for b in Basis::ALL {
            let xi = point(b);
            let s = b.eval2(xi);
            let fast = b.eval(xi);
            for i in 0..b.count() {
                for a in 0..3 {
                    assert!((fast.dn[i][a] - s.shape.dn[i][a]).abs() < 1e-14, "{} eval vs eval2", b.name());
                }
                assert!((fast.n[i] - s.shape.n[i]).abs() < 1e-15);
            }
            for a in 0..b.dim() {
                let (mut p, mut m) = (xi, xi);
                p[a] += step;
                m[a] -= step;
                let (sp, sm) = (b.eval2(p), b.eval2(m));
                for i in 0..b.count() {
                    let fd = (sp.shape.n[i] - sm.shape.n[i]) / (2.0 * step);
                    assert!((fd - s.shape.dn[i][a]).abs() < 1e-8, "{} dN{i}/dξ{a}", b.name());
                    for c in 0..3 {
                        let fd2 = (sp.shape.dn[i][c] - sm.shape.dn[i][c]) / (2.0 * step);
                        assert!((fd2 - s.d2n[i][a][c]).abs() < 1e-7, "{} d²N{i}/dξ{a}dξ{c}: {fd2} vs {}", b.name(), s.d2n[i][a][c]);
                    }
                }
            }
        }
    }

    /// An affine map and a curved one (the affine map plus a smooth bend), applied to the
    /// reference nodes.
    fn affine(x: [f64; 3]) -> [f64; 3] {
        [2.0 * x[0] + 0.3 * x[1] - 0.2 * x[2] + 1.0, 0.4 * x[0] + 1.5 * x[1] + 0.1 * x[2] - 2.0, -0.3 * x[0] + 0.2 * x[1] + 1.2 * x[2] + 0.5]
    }

    fn curved(x: [f64; 3]) -> [f64; 3] {
        let a = affine(x);
        [a[0] + 0.15 * x[1] * x[1], a[1] + 0.1 * x[0] * x[0], a[2] + 0.12 * x[0] * x[1]]
    }

    /// A quadratic field in physical coordinates (in the family's dimensions) with its gradient
    /// and Hessian.
    fn field(dim: usize, x: [f64; 3], degree: usize) -> (f64, [f64; 3], [[f64; 3]; 3]) {
        let c = [0.7, -1.3, 0.4];
        let mut v = 0.5;
        let mut g = [0.0; 3];
        let mut h = [[0.0; 3]; 3];
        for k in 0..dim {
            v += c[k] * x[k];
            g[k] = c[k];
        }
        if degree >= 2 {
            let q = [[1.1, 0.3, -0.4], [0.3, -0.6, 0.25], [-0.4, 0.25, 0.9]];
            for a in 0..dim {
                for b in 0..dim {
                    v += 0.5 * q[a][b] * x[a] * x[b];
                    g[a] += q[a][b] * x[b];
                    h[a][b] = q[a][b];
                }
            }
        }
        (v, g, h)
    }

    fn mapped(b: Basis, map: fn([f64; 3]) -> [f64; 3]) -> Vec<[f64; 3]> {
        b.nodes()
            .iter()
            .map(|p| {
                let x = map(*p);
                [0, 1, 2].map(|k| if k < b.dim() { x[k] } else { 0.0 })
            })
            .collect()
    }

    #[test]
    fn physical_gradients_hessians_and_laplacians() {
        for b in Basis::ALL {
            let xi = point(b);
            let s = b.eval2(xi);
            for (map, degree) in [(affine as fn([f64; 3]) -> [f64; 3], b.degree()), (curved, 1)] {
                let xyz = mapped(b, map);
                let h = b.hessians(&s, &xyz).unwrap();
                let x: [f64; 3] = (0..b.count()).fold([0.0; 3], |acc, i| [0, 1, 2].map(|k| acc[k] + s.shape.n[i] * xyz[i][k]));
                let (_, g, hh) = field(b.dim(), x, degree);
                let nodal: Vec<f64> = xyz.iter().map(|p| field(b.dim(), *p, degree).0).collect();
                let grad: [f64; 3] = [0, 1, 2].map(|a| (0..b.count()).map(|i| h.gradients.dx[i][a] * nodal[i]).sum());
                for a in 0..3 {
                    assert!((grad[a] - g[a]).abs() < 1e-11, "{} grad {a}: {} vs {}", b.name(), grad[a], g[a]);
                    for c in 0..3 {
                        let v: f64 = (0..b.count()).map(|i| h.d2x[i][a][c] * nodal[i]).sum();
                        assert!((v - hh[a][c]).abs() < 1e-10, "{} hess {a}{c}: {v} vs {}", b.name(), hh[a][c]);
                    }
                }
                let lap: f64 = (0..b.count()).map(|i| h.laplacian(i) * nodal[i]).sum();
                assert!((lap - (hh[0][0] + hh[1][1] + hh[2][2])).abs() < 1e-10, "{} laplacian", b.name());
            }
        }
    }

    #[test]
    fn curved_map_hessians_match_finite_differences_in_physical_space() {
        // On a curved element a quadratic field is not reproduced, but the physical Hessian of
        // the interpolant must still be the derivative of its physical gradient. Differentiate
        // the gradient along the natural directions and map: ∂(∇ₓu)/∂ξₐ = Σ_b J_ab Hₓ[b].
        for b in Basis::ALL.into_iter().filter(|b| b.degree() == 2) {
            let xyz = mapped(b, curved);
            let nodal: Vec<f64> = xyz.iter().map(|p| field(b.dim(), *p, 2).0 + 0.3 * p[0] * p[0] * p[0]).collect();
            let grad_at = |xi: [f64; 3]| -> [f64; 3] {
                let g = b.gradients(&b.eval(xi), &xyz).unwrap();
                [0, 1, 2].map(|a| (0..b.count()).map(|i| g.dx[i][a] * nodal[i]).sum())
            };
            let xi = point(b);
            let s = b.eval2(xi);
            let h = b.hessians(&s, &xyz).unwrap();
            let hu: [[f64; 3]; 3] = [0, 1, 2].map(|r| [0, 1, 2].map(|c| (0..b.count()).map(|i| h.d2x[i][r][c] * nodal[i]).sum()));
            let j = b.jacobian(&s.shape, &xyz).unwrap();
            let step = 1e-5;
            for a in 0..b.dim() {
                let (mut p, mut m) = (xi, xi);
                p[a] += step;
                m[a] -= step;
                let (gp, gm) = (grad_at(p), grad_at(m));
                for c in 0..b.dim() {
                    let fd = (gp[c] - gm[c]) / (2.0 * step);
                    let exact: f64 = (0..3).map(|k| j[a][k] * hu[k][c]).sum();
                    assert!((fd - exact).abs() < 1e-6, "{} ∂(∂u/∂x{c})/∂ξ{a}: {fd} vs {exact}", b.name());
                }
            }
        }
    }

    #[test]
    fn inverted_and_short_inputs_are_errors() {
        let xyz = mapped(Basis::Quad4, affine);
        let s = Basis::Quad4.eval([0.0; 3]);
        assert!(Basis::Quad4.gradients(&s, &xyz[..3]).is_err());
        let flipped: Vec<[f64; 3]> = xyz.iter().map(|p| [p[0], -p[1], 0.0]).collect();
        assert!(matches!(Basis::Quad4.gradients(&s, &flipped), Err(Error::Degenerate(_))));
        let collapsed = vec![[0.0; 3]; 4];
        assert!(Basis::Quad4.gradients(&s, &collapsed).is_err());
        // The pyramid's apex is finite.
        let apex = Basis::Pyramid5.eval2([0.0, 0.0, 1.0]);
        assert!(apex.shape.values().iter().chain(apex.shape.derivatives().iter().flatten()).all(|v| v.is_finite()));
        assert_eq!(Basis::of(ElementKind::Hex20), Basis::Hex20);
        assert_eq!(Basis::of(ElementKind::Rod2).dim(), 1);
    }
}
