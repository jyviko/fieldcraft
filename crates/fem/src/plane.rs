//! 2D continuum elements: plane stress, plane strain and axisymmetric.
//!
//! The elements lie in the XY plane and have two displacement components per node, u (along X)
//! and v (along Y). The strain is the 4-vector (εxx, εyy, εzz, γxy), where the third component is
//! the out-of-plane one:
//! - **plane strain** (a slice of a long body): εzz = 0, and σzz = D₃₁εxx + D₃₂εyy + D₃₄γxy is
//!   reported;
//! - **plane stress** (a thin plate loaded in its plane): σzz = 0, the elasticity is condensed to
//!   D̃ᵢⱼ = Dᵢⱼ − Dᵢ₃D₃ⱼ/D₃₃ and εzz = −(D₃₁εxx + D₃₂εyy + D₃₄γxy)/D₃₃ is reported;
//! - **axisymmetric** (a body of revolution about the Y axis): x is the radius r, y the axial
//!   coordinate, and εzz is the hoop strain εθθ = u/r; volumes are 2πr dA, so forces, reactions
//!   and masses are totals over the full circumference.
//!
//! D is the leading 4 × 4 block (xx, yy, zz, xy) of the 3D elasticity, so orthotropic materials
//! with axes along the global axes work as they do in solids; stresses and strains are reported
//! as the solid elements' 6-vectors (xx, yy, zz, xy, xz, yz) with xz = yz = 0 (axisymmetric: rr,
//! axial, hoop, r-axial). Plane elements integrate over their thickness t, so stiffness, mass and
//! loads are those of the whole thickness. (O. C. Zienkiewicz, R. L. Taylor, J. Z. Zhu, *The
//! Finite Element Method: Its Basis and Fundamentals*, 7th ed., Butterworth-Heinemann 2013,
//! ch. 6 (plane stress and strain) and §6.6 (axisymmetric); T. J. R. Hughes, *The Finite Element
//! Method*, Dover 2000, §2.7 and §3.5.)
//!
//! Elements and rules: tri3 (one point), tri6 (three points, degree 2), quad4 (2 × 2) and quad8
//! (3 × 3, or 2 × 2 reduced). Reduced quad4 needs hourglass control and is not offered. Quad4 has
//! an **incompatible modes** formulation (QM6): the internal modes (1 − ξ²) and (1 − η²) in u
//! and v (E. L. Wilson, R. L. Taylor, W. P. Doherty, J. Ghaboussi, *Incompatible displacement
//! models*, 1973), their natural derivatives mapped with the centre Jacobian and scaled by
//! det J₀/det J (R. L. Taylor, P. J. Beresford, E. L. Wilson, *A non-conforming element for
//! stress analysis*, IJNME 10(6):1211–1219, 1976), condensed out of the element. In an
//! axisymmetric element the modes also strain the hoop direction (P/r), and Taylor's scaling no
//! longer makes their strains integrate to zero over the element's volume; their strain
//! operator is therefore shifted by its volume mean, the patch-test correction of E. L. Wilson
//! and A. Ibrahimbegovic (*Use of incompatible displacement modes for the calculation of element
//! stiffnesses or stresses*, Finite Elements in Analysis and Design 7(3):229–241, 1990), which
//! leaves the plane case unchanged (its mean is zero) and makes the axisymmetric element pass the
//! patch test.
//!
//! On the axis (r = 0) the hoop strain u/r is evaluated only at integration points, which lie
//! inside the element (r > 0) for every element with some node off the axis; the solver holds
//! the radial displacement of nodes on the axis at zero, as symmetry requires.

use modelcraft_model::{ElementKind, Extent, SectionKind};

use crate::basis::Basis;
use crate::dense::{Cholesky, Matrix};
use crate::dof::{DofSet, ElementDofs, Internal};
use crate::element::{ElementType, Family, Formulation, PointStress, extrapolation, lump, symmetrize};
use crate::material::Elasticity;
use crate::quadrature::{Point, Rule};
use crate::shape::{MAX_NODES, Shape};
use crate::{Error, Result};

/// Which 2D idealisation an element follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Theory {
    PlaneStress {
        thickness: f64,
    },
    PlaneStrain {
        thickness: f64,
    },
    /// About the Y axis; x is the radius.
    Axisymmetric,
}

/// The 4 × 4 elasticity of a 2D element, in the order (xx, yy, zz, xy).
pub type Elasticity2 = [[f64; 4]; 4];

impl Theory {
    /// The theory of a 2D section (`None` for solid, shell, beam and rod sections).
    pub fn from_section(kind: &SectionKind) -> Option<Theory> {
        match *kind {
            SectionKind::PlaneStress { thickness } => Some(Theory::PlaneStress { thickness }),
            SectionKind::PlaneStrain { thickness } => Some(Theory::PlaneStrain { thickness }),
            SectionKind::Axisymmetric {} => Some(Theory::Axisymmetric),
            _ => None,
        }
    }

    /// How the element extends out of its plane (thickness, or revolved).
    pub fn extent(&self) -> Extent {
        match *self {
            Theory::PlaneStress { thickness } | Theory::PlaneStrain { thickness } => Extent::Thickness(thickness),
            Theory::Axisymmetric => Extent::Revolved,
        }
    }

    /// The 4 × 4 elasticity of this theory from the 3D one.
    pub fn elasticity(&self, d: &Elasticity) -> Result<Elasticity2> {
        let mut out = [[0.0; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                out[i][j] = d[i][j];
            }
        }
        if let Theory::PlaneStress { .. } = self {
            let d33 = d[2][2];
            if !(d33 > 0.0) {
                return Err(Error::Invalid("the material has no stiffness through the thickness".into()));
            }
            for i in 0..4 {
                for j in 0..4 {
                    out[i][j] = if i == 2 || j == 2 { 0.0 } else { d[i][j] - d[i][2] * d[2][j] / d33 };
                }
            }
        }
        Ok(out)
    }
}

/// The 2D element table: chosen by the section kind (a 2D section on tri3, tri6, quad4 or quad8),
/// separately from shells on the same element kinds.
pub static PLANE_TYPES: [ElementType; 4] = [
    ElementType {
        kind: ElementKind::Tri3,
        family: Family::Triangle,
        nodes: 3,
        corners: 3,
        shape: |xi| Basis::Tri3.eval(xi),
        full: Rule::TriCentroid,
        reduced: None,
        mass: Rule::TriConical(3),
        incompatible_modes: false,
    },
    ElementType {
        kind: ElementKind::Tri6,
        family: Family::Triangle,
        nodes: 6,
        corners: 3,
        shape: |xi| Basis::Tri6.eval(xi),
        full: Rule::Tri3,
        reduced: None,
        mass: Rule::TriConical(4),
        incompatible_modes: false,
    },
    ElementType {
        kind: ElementKind::Quad4,
        family: Family::Quadrilateral,
        nodes: 4,
        corners: 4,
        shape: |xi| Basis::Quad4.eval(xi),
        full: Rule::Quad(2),
        reduced: None,
        mass: Rule::Quad(2),
        incompatible_modes: true,
    },
    ElementType {
        kind: ElementKind::Quad8,
        family: Family::Quadrilateral,
        nodes: 8,
        corners: 4,
        shape: |xi| Basis::Quad8.eval(xi),
        full: Rule::Quad(3),
        reduced: Some(Rule::Quad(2)),
        mass: Rule::Quad(3),
        incompatible_modes: false,
    },
];

/// The 2D table row of an element kind.
pub fn plane_type(kind: ElementKind) -> Result<&'static ElementType> {
    PLANE_TYPES
        .iter()
        .find(|t| t.kind == kind)
        .ok_or_else(|| Error::NotSupported(format!("{} elements in a 2D section (2D sections take tri3, tri6, quad4 and quad8)", kind.name())))
}

/// The formulation a 2D section asks for, if this element type has it.
pub fn formulation(kind: ElementKind, f: &modelcraft_model::Formulation) -> Result<Formulation> {
    let ty = plane_type(kind)?;
    if f.hybrid {
        return Err(Error::NotSupported("hybrid (mixed displacement–pressure) 2D elements".into()));
    }
    if f.incompatible_modes {
        return if ty.incompatible_modes {
            Ok(Formulation::IncompatibleModes)
        } else {
            Err(Error::Invalid(format!("incompatible modes apply to quad4 elements, not {}", kind.name())))
        };
    }
    match f.integration {
        modelcraft_model::Integration::Full => Ok(Formulation::Full),
        modelcraft_model::Integration::Reduced if ty.reduced.is_some() => Ok(Formulation::Reduced),
        modelcraft_model::Integration::Reduced => Err(Error::NotSupported(format!(
            "reduced integration of 2D {} elements (quad4 needs hourglass control; use full integration or incompatible modes)",
            kind.name()
        ))),
    }
}

/// One column of the strain operator: (∂N/∂x, ∂N/∂y, hoop N/r), for a node or an internal mode.
type Column = [f64; 3];

/// What the kernels need at one integration point.
struct At {
    shape: Shape,
    /// Strain columns of the nodes.
    g: [Column; MAX_NODES],
    det: f64,
    x: [f64; 3],
    /// det J times the point's weight times the out-of-plane extent: the point's share of the
    /// volume.
    dv: f64,
}

/// The incompatible modes' count of internal degrees of freedom: two modes, two directions.
const INCOMPATIBLE_DOFS: usize = 4;

/// A 2D element: its type, node coordinates (z = 0), formulation and theory.
#[derive(Debug, Clone, Copy)]
pub struct Plane<'a> {
    pub ty: &'static ElementType,
    xyz: &'a [[f64; 3]],
    formulation: Formulation,
    theory: Theory,
}

/// out[2i + r][2j + c] += w (Bᵢᵀ D Bⱼ)[r][c] over every pair of columns (row-major, `cols` wide),
/// computed for j ≥ i and mirrored, so the result is exactly symmetric.
fn accumulate(out: &mut [f64], columns: &[Column], d: &Elasticity2, w: f64) {
    let cols = 2 * columns.len();
    let block = |g: &Column| -> [[f64; 2]; 4] { [[g[0], 0.0], [0.0, g[1]], [g[2], 0.0], [g[1], g[0]]] };
    for (j, gj) in columns.iter().enumerate() {
        let bj = block(gj);
        let mut db = [[0.0; 2]; 4];
        for s in 0..4 {
            for c in 0..2 {
                db[s][c] = (0..4).map(|t| d[s][t] * bj[t][c]).sum();
            }
        }
        for (i, gi) in columns.iter().enumerate().take(j + 1) {
            let bi = block(gi);
            for r in 0..2 {
                for c in if i == j { r } else { 0 }..2 {
                    let v = w * (0..4).map(|s| bi[s][r] * db[s][c]).sum::<f64>();
                    let (row, col) = (2 * i + r, 2 * j + c);
                    if let Some(x) = out.get_mut(row * cols + col) {
                        *x += v;
                    }
                    if row != col
                        && let Some(x) = out.get_mut(col * cols + row)
                    {
                        *x += v;
                    }
                }
            }
        }
    }
}

impl<'a> Plane<'a> {
    pub fn new(kind: ElementKind, xyz: &'a [[f64; 3]], formulation: Formulation, theory: Theory) -> Result<Plane<'a>> {
        let ty = plane_type(kind)?;
        if xyz.len() != ty.nodes {
            return Err(Error::Invalid(format!("a {} element has {} nodes, got {}", kind.name(), ty.nodes, xyz.len())));
        }
        modelcraft_model::check_coordinates(xyz.as_flattened()).map_err(|e| Error::Invalid(e.to_string()))?;
        if xyz.iter().any(|p| p[2] != 0.0) {
            return Err(Error::Invalid(format!("a 2D {} element must lie in the XY plane (z = 0)", kind.name())));
        }
        if theory == Theory::Axisymmetric && xyz.iter().any(|p| p[0] < 0.0) {
            return Err(Error::Invalid("an axisymmetric element must lie in x ≥ 0 (x is the radius)".into()));
        }
        if let Theory::PlaneStress { thickness } | Theory::PlaneStrain { thickness } = theory
            && !(thickness > 0.0 && thickness.is_finite())
        {
            return Err(Error::Invalid(format!("the thickness must be a positive number, got {thickness}")));
        }
        let ok = match formulation {
            Formulation::Full => true,
            Formulation::Reduced => ty.reduced.is_some(),
            Formulation::IncompatibleModes => ty.incompatible_modes,
        };
        if !ok {
            return Err(Error::NotSupported(format!("{formulation:?} formulation of 2D {} elements", kind.name())));
        }
        Ok(Plane { ty, xyz, formulation, theory })
    }

    /// The element's unknowns: u and v at every node ([`DofSet::PLANE`]), and the incompatible
    /// modes it condenses out.
    pub fn dofs(&self) -> ElementDofs {
        let incompatible_modes = if self.formulation == Formulation::IncompatibleModes { INCOMPATIBLE_DOFS } else { 0 };
        ElementDofs { node: DofSet::PLANE, internal: Internal { incompatible_modes, pressure: 0 } }
    }

    /// The stiffness integration rule.
    pub fn rule(&self) -> Rule {
        match (self.formulation, self.ty.reduced) {
            (Formulation::Reduced, Some(r)) => r,
            _ => self.ty.full,
        }
    }

    fn degenerate(&self) -> Error {
        Error::Degenerate(format!("a 2D {} element is inverted, clockwise or degenerate (its Jacobian is not positive)", self.ty.kind.name()))
    }

    /// The 2 × 2 Jacobian J[a][b] = ∂x_b/∂ξ_a and its determinant.
    fn jacobian(&self, s: &Shape) -> ([[f64; 2]; 2], f64) {
        let mut j = [[0.0; 2]; 2];
        for (d, x) in s.derivatives().iter().zip(self.xyz) {
            for a in 0..2 {
                for b in 0..2 {
                    j[a][b] += d[a] * x[b];
                }
            }
        }
        (j, j[0][0] * j[1][1] - j[0][1] * j[1][0])
    }

    fn at(&self, p: &Point) -> Result<At> {
        let basis = Basis::of(self.ty.kind);
        let shape = basis.eval(p.xi);
        let grads = basis.gradients(&shape, self.xyz).map_err(|_| self.degenerate())?;
        let det = grads.det;
        let mut x = [0.0; 3];
        for (n, xi) in shape.values().iter().zip(self.xyz) {
            for k in 0..2 {
                x[k] += n * xi[k];
            }
        }
        let axisymmetric = self.theory == Theory::Axisymmetric;
        if axisymmetric && !(x[0] > 0.0) {
            return Err(Error::Degenerate(format!("an axisymmetric {} element has an integration point on the axis", self.ty.kind.name())));
        }
        let mut g = [[0.0; 3]; MAX_NODES];
        for ((gi, d), n) in g.iter_mut().zip(grads.gradients()).zip(shape.values()) {
            *gi = [d[0], d[1], if axisymmetric { n / x[0] } else { 0.0 }];
        }
        let dv = det * p.weight * self.theory.extent().at(x);
        Ok(At { shape, g, det, x, dv })
    }

    /// The element's volume (area times thickness, or the volume of the ring).
    pub fn volume(&self) -> Result<f64> {
        let mut v = 0.0;
        for p in self.ty.mass.points() {
            v += self.at(&p)?.dv;
        }
        Ok(v)
    }

    /// The incompatible modes' strain columns at every stiffness point: Taylor's mapped
    /// derivatives (and the hoop value P/r when axisymmetric), shifted by their volume mean.
    fn incompatible_columns(&self, points: &[(Point, At)]) -> Result<Vec<[Column; 2]>> {
        let s0 = (self.ty.shape)([0.0; 3]);
        let (j0, det0) = self.jacobian(&s0);
        if !(det0 > 0.0 && det0.is_finite()) {
            return Err(self.degenerate());
        }
        let axisymmetric = self.theory == Theory::Axisymmetric;
        let mut out = Vec::with_capacity(points.len());
        let mut mean = [[0.0; 3]; 2];
        let mut volume = 0.0;
        for (p, a) in points {
            let mut cols = [[0.0; 3]; 2];
            for (k, col) in cols.iter_mut().enumerate() {
                // P_k = 1 − ξ_k²: ∂P/∂ξ_k = −2ξ_k, the other derivative zero.
                let mut dp = [0.0; 2];
                dp[k] = -2.0 * p.xi[k];
                // (det J₀ / det J) J₀⁻¹ ∂P/∂ξ, with J₀⁻¹ = adj(J₀) / det J₀.
                col[0] = (j0[1][1] * dp[0] - j0[0][1] * dp[1]) / a.det;
                col[1] = (-j0[1][0] * dp[0] + j0[0][0] * dp[1]) / a.det;
                col[2] = if axisymmetric { (1.0 - p.xi[k] * p.xi[k]) / a.x[0] } else { 0.0 };
                for c in 0..3 {
                    mean[k][c] += col[c] * a.dv;
                }
            }
            volume += a.dv;
            out.push(cols);
        }
        if !(volume > 0.0) {
            return Err(self.degenerate());
        }
        for cols in &mut out {
            for k in 0..2 {
                for c in 0..3 {
                    cols[k][c] -= mean[k][c] / volume;
                }
            }
        }
        Ok(out)
    }

    /// The integration points with their data.
    fn points(&self) -> Result<Vec<(Point, At)>> {
        self.rule().points().into_iter().map(|p| self.at(&p).map(|a| (p, a))).collect()
    }

    /// The stiffness over nodes and, with incompatible modes, the internal modes after them:
    /// (2n + 4) × (2n + 4).
    fn full_stiffness(&self, d: &Elasticity2) -> Result<(Matrix, Vec<(Point, At)>, Option<Vec<[Column; 2]>>)> {
        let n = self.ty.nodes;
        let points = self.points()?;
        let modes = if self.formulation == Formulation::IncompatibleModes { Some(self.incompatible_columns(&points)?) } else { None };
        let width = 2 * n + modes.as_ref().map_or(0, |_| INCOMPATIBLE_DOFS);
        let mut k = Matrix::zeros(width);
        let mut columns: Vec<Column> = Vec::with_capacity(n + 2);
        for (i, (_, a)) in points.iter().enumerate() {
            columns.clear();
            columns.extend_from_slice(a.g.get(..n).unwrap_or_default());
            if let Some(m) = modes.as_ref().and_then(|m| m.get(i)) {
                columns.extend_from_slice(m);
            }
            accumulate(&mut k.data, &columns, d, a.dv);
        }
        Ok((k, points, modes))
    }

    /// The stiffness matrix, 2n × 2n, degrees of freedom ordered node by node (u, v).
    pub fn stiffness(&self, d: &Elasticity) -> Result<Matrix> {
        let d2 = self.theory.elasticity(d)?;
        let (full, _, modes) = self.full_stiffness(&d2)?;
        if modes.is_none() {
            return Ok(full);
        }
        // K = K_uu − K_ua K_aa⁻¹ K_au.
        let n = 2 * self.ty.nodes;
        let kaa = sub_block(&full, n, n, INCOMPATIBLE_DOFS);
        let chol = Cholesky::new(&kaa)?;
        let mut k = sub_block(&full, 0, 0, n);
        for i in 0..n {
            let mut col: Vec<f64> = (0..INCOMPATIBLE_DOFS).map(|a| full.get(n + a, i)).collect();
            chol.solve(&mut col);
            for j in 0..n {
                let v: f64 = (0..INCOMPATIBLE_DOFS).map(|a| full.get(j, n + a) * col.get(a).copied().unwrap_or(0.0)).sum();
                k.add(j, i, -v);
            }
        }
        symmetrize(&mut k);
        Ok(k)
    }

    /// The consistent mass matrix per displacement component, n × n: m_ij = ∫ ρ Nᵢ Nⱼ dV.
    pub fn mass(&self, density: f64) -> Result<Matrix> {
        let n = self.ty.nodes;
        let mut m = Matrix::zeros(n);
        for p in self.ty.mass.points() {
            let a = self.at(&p)?;
            let w = density * a.dv;
            let v = a.shape.values();
            for (i, x) in v.iter().enumerate() {
                for (j, y) in v.iter().enumerate() {
                    m.add(i, j, w * x * y);
                }
            }
        }
        Ok(m)
    }

    /// The lumped (diagonal) mass per node: the consistent diagonal scaled to the element's mass
    /// (Hinton, Rock and Zienkiewicz 1976).
    pub fn lumped_mass(&self, density: f64) -> Result<Vec<f64>> {
        lump(&self.mass(density)?).ok_or_else(|| self.degenerate())
    }

    /// Consistent nodal forces of a force per volume b(x) in the plane: fᵢ = ∫ Nᵢ b(x) dV (the Z
    /// component of `b` is ignored).
    pub fn body_force_field(&self, b: impl Fn([f64; 3]) -> [f64; 3]) -> Result<Vec<[f64; 2]>> {
        let mut f = vec![[0.0; 2]; self.ty.nodes];
        for p in self.ty.mass.points() {
            let a = self.at(&p)?;
            let b = b(a.x);
            for (fi, n) in f.iter_mut().zip(a.shape.values()) {
                for k in 0..2 {
                    fi[k] += n * b[k] * a.dv;
                }
            }
        }
        Ok(f)
    }

    /// Strain and stress at every integration point for the nodal displacements `u` (u, v).
    pub fn stresses(&self, d: &Elasticity, u: &[[f64; 2]]) -> Result<Vec<PointStress>> {
        if u.len() != self.ty.nodes {
            return Err(Error::Invalid(format!("{} displacements for a {}-node element", u.len(), self.ty.nodes)));
        }
        let d2 = self.theory.elasticity(d)?;
        let (full, points, modes) =
            if self.formulation == Formulation::IncompatibleModes { self.full_stiffness(&d2)? } else { (Matrix::zeros(0), self.points()?, None) };
        // Incompatible modes: α = −K_aa⁻¹ K_au u.
        let alpha = if modes.is_some() {
            let n = 2 * self.ty.nodes;
            let flat: Vec<f64> = u.iter().flatten().copied().collect();
            let mut rhs: Vec<f64> =
                (0..INCOMPATIBLE_DOFS).map(|a| -flat.iter().enumerate().map(|(i, x)| full.get(n + a, i) * x).sum::<f64>()).collect();
            Cholesky::new(&sub_block(&full, n, n, INCOMPATIBLE_DOFS))?.solve(&mut rhs);
            rhs
        } else {
            Vec::new()
        };
        let mut out = Vec::with_capacity(points.len());
        for (i, (_, a)) in points.iter().enumerate() {
            let mut e = [0.0; 4];
            let mut add = |g: &Column, uv: [f64; 2]| {
                e[0] += g[0] * uv[0];
                e[1] += g[1] * uv[1];
                e[2] += g[2] * uv[0];
                e[3] += g[1] * uv[0] + g[0] * uv[1];
            };
            for (g, uv) in a.g.iter().zip(u) {
                add(g, *uv);
            }
            if let Some(cols) = modes.as_ref().and_then(|m| m.get(i)) {
                for (k, g) in cols.iter().enumerate() {
                    add(g, [alpha.get(2 * k).copied().unwrap_or(0.0), alpha.get(2 * k + 1).copied().unwrap_or(0.0)]);
                }
            }
            if let Theory::PlaneStress { .. } = self.theory {
                e[2] = -(d[2][0] * e[0] + d[2][1] * e[1] + d[2][3] * e[3]) / d[2][2];
            }
            // Plane stress: D̃ has no zz row or column, so the εzz just recovered adds nothing.
            let s: [f64; 4] = std::array::from_fn(|r| (0..4).map(|c| d2[r][c] * e[c]).sum());
            out.push(PointStress { xyz: a.x, strain: [e[0], e[1], e[2], e[3], 0.0, 0.0], stress: [s[0], s[1], s[2], s[3], 0.0, 0.0] });
        }
        Ok(out)
    }

    /// Extrapolate values at the integration points to the element's nodes.
    pub fn extrapolate(&self, values: &[[f64; 6]]) -> Result<Vec<[f64; 6]>> {
        let e = extrapolation(self.ty, self.rule())?;
        if values.len() != e.points {
            return Err(Error::Invalid(format!("{} values for {} integration points", values.len(), e.points)));
        }
        Ok(e.apply(values))
    }
}

/// The `size` × `size` block of `m` starting at (`row`, `col`).
fn sub_block(m: &Matrix, row: usize, col: usize, size: usize) -> Matrix {
    let mut out = Matrix::zeros(size);
    for i in 0..size {
        for j in 0..size {
            out.add(i, j, m.get(row + i, col + j));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::isotropic;

    /// A distorted element of `kind` with corners in x ∈ [1, 3] (so axisymmetric elements stay off
    /// the axis); mid-side nodes off the chord midpoints by a little, so quadratic elements are
    /// curved.
    fn element(kind: ElementKind) -> Vec<[f64; 3]> {
        let corners: Vec<[f64; 3]> = match kind {
            ElementKind::Tri3 | ElementKind::Tri6 => vec![[1.0, 0.2, 0.0], [3.0, 0.0, 0.0], [1.6, 1.9, 0.0]],
            _ => vec![[1.0, 0.1, 0.0], [3.0, 0.0, 0.0], [2.7, 2.2, 0.0], [1.2, 1.6, 0.0]],
        };
        let mut xyz = corners.clone();
        for (k, e) in kind.edges().iter().enumerate() {
            if let [a, b, _] = **e {
                let bump = 0.05 * (k as f64 + 1.0);
                xyz.push([(corners[a][0] + corners[b][0]) / 2.0 + bump, (corners[a][1] + corners[b][1]) / 2.0 - bump, 0.0]);
            }
        }
        xyz
    }

    fn variants() -> Vec<(ElementKind, Formulation)> {
        vec![
            (ElementKind::Tri3, Formulation::Full),
            (ElementKind::Tri6, Formulation::Full),
            (ElementKind::Quad4, Formulation::Full),
            (ElementKind::Quad4, Formulation::IncompatibleModes),
            (ElementKind::Quad8, Formulation::Full),
            (ElementKind::Quad8, Formulation::Reduced),
        ]
    }

    const THEORIES: [Theory; 3] = [Theory::PlaneStress { thickness: 0.7 }, Theory::PlaneStrain { thickness: 1.0 }, Theory::Axisymmetric];

    #[test]
    fn rigid_motion_strains_nothing_and_matrices_are_symmetric() {
        let d = isotropic(1000.0, 0.3).unwrap();
        for (kind, f) in variants() {
            let xyz = element(kind);
            for theory in THEORIES {
                let e = Plane::new(kind, &xyz, f, theory).unwrap();
                let k = e.stiffness(&d).unwrap();
                let n = k.n;
                for i in 0..n {
                    assert!(k.get(i, i) > 0.0);
                    for j in 0..n {
                        assert_eq!(k.get(i, j), k.get(j, i), "{kind:?} {f:?} {theory:?}");
                    }
                }
                // Translation along Y always; along X and the rotation only in plane models.
                let mut motions: Vec<Vec<f64>> = vec![xyz.iter().flat_map(|_| [0.0, 1.0]).collect()];
                if theory != Theory::Axisymmetric {
                    motions.push(xyz.iter().flat_map(|_| [1.0, 0.0]).collect());
                    motions.push(xyz.iter().flat_map(|p| [-p[1], p[0]]).collect());
                }
                for u in motions {
                    let f = k.mul_vec(&u);
                    let scale = k.data.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                    assert!(f.iter().all(|v| v.abs() < 1e-12 * scale), "{kind:?} {theory:?}: {f:?}");
                }
            }
        }
    }

    #[test]
    fn linear_fields_give_exact_constant_stress() {
        let (young, nu) = (1000.0, 0.3);
        let d = isotropic(young, nu).unwrap();
        for (kind, f) in variants() {
            let xyz = element(kind);
            // Plane: u = 0.01 x + 0.002 y, v = −0.003 x + 0.004 y.
            for theory in [Theory::PlaneStress { thickness: 0.7 }, Theory::PlaneStrain { thickness: 1.0 }] {
                let e = Plane::new(kind, &xyz, f, theory).unwrap();
                let u: Vec<[f64; 2]> = xyz.iter().map(|p| [0.01 * p[0] + 0.002 * p[1], -0.003 * p[0] + 0.004 * p[1]]).collect();
                let (exx, eyy, gxy) = (0.01, 0.004, -0.001);
                let (sxx, syy, szz, ezz) = match theory {
                    Theory::PlaneStress { .. } => {
                        let c = young / (1.0 - nu * nu);
                        (c * (exx + nu * eyy), c * (eyy + nu * exx), 0.0, -nu / (1.0 - nu) * (exx + eyy))
                    }
                    _ => {
                        let l = young * nu / ((1.0 + nu) * (1.0 - 2.0 * nu));
                        let m = young / (2.0 * (1.0 + nu));
                        (l * (exx + eyy) + 2.0 * m * exx, l * (exx + eyy) + 2.0 * m * eyy, l * (exx + eyy), 0.0)
                    }
                };
                let txy = young / (2.0 * (1.0 + nu)) * gxy;
                for p in e.stresses(&d, &u).unwrap() {
                    let want = [sxx, syy, szz, txy, 0.0, 0.0];
                    assert!(p.stress.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-10), "{kind:?} {f:?} {theory:?}: {:?} vs {want:?}", p.stress);
                    assert!((p.strain[2] - ezz).abs() < 1e-14, "{kind:?} {theory:?}");
                }
                // The extrapolated nodal values are the constant too.
                let values: Vec<[f64; 6]> = e.stresses(&d, &u).unwrap().iter().map(|p| p.stress).collect();
                for v in e.extrapolate(&values).unwrap() {
                    assert!((v[0] - sxx).abs() < 1e-10 && (v[3] - txy).abs() < 1e-10);
                }
            }
            // Axisymmetric: u_r = 0.01 r, u_z = 0.004 z gives εrr = εθθ = 0.01, εzz = 0.004.
            let e = Plane::new(kind, &xyz, f, Theory::Axisymmetric).unwrap();
            let u: Vec<[f64; 2]> = xyz.iter().map(|p| [0.01 * p[0], 0.004 * p[1]]).collect();
            let l = young * nu / ((1.0 + nu) * (1.0 - 2.0 * nu));
            let m = young / (2.0 * (1.0 + nu));
            let tr = 0.024;
            let want = [l * tr + 2.0 * m * 0.01, l * tr + 2.0 * m * 0.004, l * tr + 2.0 * m * 0.01, 0.0, 0.0, 0.0];
            for p in e.stresses(&d, &u).unwrap() {
                assert!(p.stress.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-10), "{kind:?} {f:?} axisymmetric: {:?}", p.stress);
            }
            // And its stiffness has no out-of-balance force for it inside the element: K u = f
            // where f are the consistent forces of the boundary tractions; their total along Y
            // vanishes (no Y body force).
            let k = e.stiffness(&d).unwrap();
            let flat: Vec<f64> = u.iter().flatten().copied().collect();
            let fy: f64 = k.mul_vec(&flat).iter().skip(1).step_by(2).sum();
            assert!(fy.abs() < 1e-10, "{kind:?} {f:?}: {fy}");
        }
    }

    #[test]
    fn volumes_masses_and_loads() {
        let pi = std::f64::consts::PI;
        // A unit square at x ∈ [1, 2]: area 1, ring volume 2π · 1.5.
        let sq = [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
        let e = Plane::new(ElementKind::Quad4, &sq, Formulation::Full, Theory::PlaneStress { thickness: 3.0 }).unwrap();
        assert!((e.volume().unwrap() - 3.0).abs() < 1e-14);
        assert!((e.mass(2.0).unwrap().data.iter().sum::<f64>() - 6.0).abs() < 1e-13);
        let e = Plane::new(ElementKind::Quad4, &sq, Formulation::Full, Theory::Axisymmetric).unwrap();
        assert!((e.volume().unwrap() - 3.0 * pi).abs() < 1e-13);
        assert!((e.lumped_mass(1.0).unwrap().iter().sum::<f64>() - 3.0 * pi).abs() < 1e-12);
        // A centrifugal field ω² r: total ∫ ρ ω² r dV = 2π ∫∫ r² dr dz = 2π · 7/3.
        let f = e.body_force_field(|x| [x[0], 0.0, 0.0]).unwrap();
        assert!((f.iter().map(|v| v[0]).sum::<f64>() - 2.0 * pi * 7.0 / 3.0).abs() < 1e-12);
        // Pressure on the outer edge (x = 2) of the ring pushes inward with p · 2π · 2 · 1.
        let fp = modelcraft_model::nodal::edge_pressure(&[sq[1], sq[2]], Theory::Axisymmetric.extent(), 1.0).unwrap();
        assert!((fp.iter().map(|v| v[0]).sum::<f64>() + 4.0 * pi).abs() < 1e-12, "{fp:?}");
    }

    #[test]
    fn bad_input_is_an_error() {
        let d = isotropic(1000.0, 0.3).unwrap();
        let sq = [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
        let cw = [sq[0], sq[3], sq[2], sq[1]];
        let e = Plane::new(ElementKind::Quad4, &cw, Formulation::Full, Theory::PlaneStrain { thickness: 1.0 }).unwrap();
        assert!(matches!(e.stiffness(&d), Err(Error::Degenerate(_))));
        assert!(Plane::new(ElementKind::Quad4, &sq, Formulation::Reduced, Theory::Axisymmetric).is_err());
        assert!(Plane::new(ElementKind::Tri3, &sq[..3], Formulation::IncompatibleModes, Theory::Axisymmetric).is_err());
        assert!(Plane::new(ElementKind::Hex8, &sq, Formulation::Full, Theory::Axisymmetric).is_err());
        let off = [[1.0, 0.0, 0.5], sq[1], sq[2], sq[3]];
        assert!(Plane::new(ElementKind::Quad4, &off, Formulation::Full, Theory::Axisymmetric).is_err());
        let neg = sq.map(|p| [p[0] - 1.5, p[1], 0.0]);
        assert!(Plane::new(ElementKind::Quad4, &neg, Formulation::Full, Theory::Axisymmetric).is_err());
        assert!(Plane::new(ElementKind::Quad4, &sq, Formulation::Full, Theory::PlaneStress { thickness: f64::NAN }).is_err());
        // A ring element touching the axis is fine (its integration points are off it).
        let on_axis = sq.map(|p| [p[0] - 1.0, p[1], 0.0]);
        let e = Plane::new(ElementKind::Quad4, &on_axis, Formulation::IncompatibleModes, Theory::Axisymmetric).unwrap();
        assert!(e.stiffness(&d).is_ok());
        assert!(formulation(ElementKind::Quad4, &modelcraft_model::Formulation { hybrid: true, ..Default::default() }).is_err());
    }
}
