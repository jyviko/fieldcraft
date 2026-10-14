//! Solid and 2D elements of nonlinear steps: total Lagrangian (finite strain) or with small
//! strain and an inelastic material; internal forces and the consistent tangent stiffness for
//! nodal displacements u, and the Cauchy stress and the strain at the integration points.
//!
//! **Formulation.** Total Lagrangian: the integrals run over the reference (undeformed) element
//! with its own quadrature, F = I + Σ u_a ⊗ ∇₀N_a at each point, and the material returns the
//! Kirchhoff stress τ and its spatial tangent c ([`crate::constitutive`]). With the shape
//! function gradients of the current configuration ∇ₓN_a = F⁻ᵀ ∇₀N_a,
//! - internal forces f_a = ∫ Bₓ,aᵀ τ dV₀,
//! - tangent K_ab = ∫ (Bₓ,aᵀ c Bₓ,b + (∇ₓN_a · τ ∇ₓN_b) I) dV₀ (material plus geometric, or
//!   initial-stress, stiffness),
//!
//! with Bₓ the small-strain operator of the current gradients. This is the push-forward of the
//! textbook ∫ B₀ᵀ S dV₀ and ∫ (B₀ᵀ C B₀ + ∇₀N·S∇₀N I) dV₀, identical term by term (Bonet & Wood
//! 2008, §9.2–9.4; K.-J. Bathe, *Finite Element Procedures*, 2nd ed. 2014, §6.3). A rigid motion
//! leaves τ at zero exactly, whatever its size.
//!
//! **Hybrid (mixed u–p) solids** with a decoupled material W = W̄(C̄) + U(J): the three-field
//! formulation of J. C. Simo, R. L. Taylor, K. S. Pister, *Variational and projection methods
//! for the volume constraint in finite deformation elasto-plasticity*, CMAME 51:177–208, 1985,
//! with the element's discontinuous pressure functions N_p (the linear hybrid elements' fields,
//! [`crate::hybrid`]): the dilatation θ = N_p·θ̂ is the projection of J, M θ̂ = ∫ N_p J dV₀ with
//! M = ∫ N_p N_pᵀ dV₀; the pressure p = N_p·p̂ the projection of U′(θ), M p̂ = ∫ N_p U′(θ) dV₀.
//! Then τ = τ_iso(F) + J p I, and with G = ∫ N_p J (∇ₓN)ᵀ dV₀ and H = ∫ N_p U″(θ) N_pᵀ dV₀,
//! K = ∫ (Bₓᵀ c* Bₓ + ∇ₓN·τ∇ₓN I) dV₀ + Gᵀ M⁻¹ H M⁻¹ G with c* = c_iso + J p (I ⊗ I − 2𝕀),
//! symmetric; at u = 0 it is the linear hybrid stiffness K_dev + κ G M⁻¹ Gᵀ. The constant
//! pressure case is the mean dilatation method of J. C. Nagtegaal, D. M. Parks, J. R. Rice
//! (CMAME 4:153–177, 1974) at finite strain. St Venant–Kirchhoff has no such split, so hybrid
//! elements of an elastic material are refused in nonlinear steps; J2 plasticity has it exactly
//! (the return is deviatoric, the volume Hencky's K/2 (ln J)², [`crate::elastoplastic`]).
//!
//! **Small strain** ([`Kinematics::Small`], a step without nonlinear geometry with an inelastic
//! material): the same loops on the linear kinematics, ε = sym ∇u with the reference gradients,
//! f = ∫ Bᵀ σ dV₀ and K = ∫ Bᵀ 𝔻 B dV₀ with the material's consistent tangent 𝔻, and no
//! geometric stiffness. Mixed elements project the dilatation 1 + tr ε and the pressure K tr ε
//! the same way, which is the linear hybrid element of [`crate::hybrid`] while the material is
//! elastic.
//!
//! **2D elements**: plane strain (F₃₃ = 1) and axisymmetric (F₃₃ = r/R, the hoop stretch, with
//! the hoop column N/r of the current radius in Bₓ and a geometric term τ_θθ N_a N_b / r² on the
//! radial motion; Bonet & Wood 2008, §9.5). Plane stress (whose thickness change needs a local
//! iteration for σ_zz = 0) is refused.
//!
//! **Incompatible modes** (hex8, quad4) are refused in nonlinear steps: their finite-strain form
//! (enhanced assumed strain, J. C. Simo, F. Armero, IJNME 33:1413–1449, 1992) needs the internal
//! modes as element unknowns carried between iterations, and it develops spurious hourglass
//! modes in large compression (P. Wriggers, S. Reese, CMAME 135:201–209, 1996), which want a
//! stabilised variant of their own. With small strain they are refused for inelastic materials
//! too: their internal modes would carry the plastic history between iterations.

use crate::constitutive::{MaterialModel, Response, add_pressure, logarithmic_strain};
use crate::dense::{Cholesky, Matrix, inverse3};
use crate::element::{Formulation, PointStress, Solid, accumulate_symmetric, b_block};
use crate::hybrid::pressure_basis;
use crate::plane::{Plane, Theory};
use crate::rotation::{IDENTITY, Mat3};
use crate::{Error, Result};

/// Internal forces (element degrees of freedom, node by node) and, when asked for, the tangent.
#[derive(Debug, Clone, PartialEq)]
pub struct Finite {
    pub force: Vec<f64>,
    pub tangent: Option<Matrix>,
}

/// The kinematics of an evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kinematics {
    /// Total Lagrangian, large displacements, rotations and strains.
    Finite,
    /// Small displacements and strains (ε = sym ∇u, the reference gradients, no geometric
    /// stiffness): only the material is nonlinear.
    Small,
}

/// What one integration point gives the element.
struct PointKinematics {
    weight: f64,
    f: Mat3,
    /// det F, or the dilatation 1 + tr ε of small strain (what a mixed element projects).
    j: f64,
    /// The small strain ε = sym ∇u (engineering shears); zero at finite strain.
    strain: [f64; 6],
    /// ∇ₓN of every node (∇₀N at small strain).
    gradients: Vec<[f64; 3]>,
    /// Reference position.
    x0: [f64; 3],
}

/// The checked history slice of one integration point.
fn point_history(history: &[f64], len: usize, point: usize) -> &[f64] {
    history.get(point * len..(point + 1) * len).unwrap_or_default()
}

/// The small strain of a displacement gradient F − I (engineering shears).
fn small_strain(f: &Mat3) -> [f64; 6] {
    [f[0][0] - 1.0, f[1][1] - 1.0, f[2][2] - 1.0, f[0][1] + f[1][0], f[0][2] + f[2][0], f[1][2] + f[2][1]]
}

/// The response of one point's material with its history slices.
fn point_response(m: &MaterialModel, kin: Kinematics, p: &PointKinematics, committed: &[f64], updated: &mut [f64]) -> Result<Response> {
    match kin {
        Kinematics::Finite => m.response(&p.f, None, committed, updated),
        Kinematics::Small => m.small_strain(&p.strain, None, committed, updated, true),
    }
}

/// Add f_a += w Bₓ,aᵀ τ and, with `k`, the material tangent and (with `geometric`) the
/// geometric one.
fn add_point(force: &mut [f64], k: Option<&mut Matrix>, gradients: &[[f64; 3]], r: &Response, w: f64, geometric: bool) {
    for (a, g) in gradients.iter().enumerate() {
        let b = b_block(g);
        for c in 0..3 {
            if let Some(x) = force.get_mut(3 * a + c) {
                *x += w * (0..6).map(|s| b[s][c] * r.stress[s]).sum::<f64>();
            }
        }
    }
    let Some(k) = k else { return };
    accumulate_symmetric(&mut k.data, gradients, &r.tangent, w);
    if !geometric {
        return;
    }
    let tau = crate::rotation::tensor(&r.stress);
    let n = k.n;
    for (a, ga) in gradients.iter().enumerate() {
        let tg: [f64; 3] = std::array::from_fn(|i| (0..3).map(|jj| tau[i][jj] * ga[jj]).sum());
        for (b, gb) in gradients.iter().enumerate() {
            let s = w * (tg[0] * gb[0] + tg[1] * gb[1] + tg[2] * gb[2]);
            for c in 0..3 {
                if let Some(x) = k.data.get_mut((3 * a + c) * n + 3 * b + c) {
                    *x += s;
                }
            }
        }
    }
}

fn inverted(kind: &str, j: f64) -> Error {
    Error::Degenerate(format!("a {kind} element is inverted at an integration point (J = det F = {j:.3e})"))
}

/// One displacement vector per node, all finite.
fn check_displacements<const N: usize>(u: &[[f64; N]], nodes: usize) -> Result<()> {
    if u.len() != nodes {
        return Err(Error::Invalid(format!("{} displacements for a {nodes}-node element", u.len())));
    }
    if u.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("displacements must be finite".into()));
    }
    Ok(())
}

/// Check the element's history slices against the material and its points.
fn check_history(m: &MaterialModel, points: usize, committed: &[f64], updated: &[f64]) -> Result<usize> {
    let len = m.history_len();
    if committed.len() != points * len || updated.len() != points * len {
        return Err(Error::Invalid(format!("{} history values for {points} points of {len}", committed.len())));
    }
    Ok(len)
}

impl Solid<'_> {
    /// Refuse what has no form here.
    fn check_nonlinear(&self, m: &MaterialModel, u: &[[f64; 3]], kin: Kinematics) -> Result<()> {
        check_displacements(u, self.ty.nodes)?;
        match self.formulation {
            Formulation::IncompatibleModes => {
                let why = match kin {
                    Kinematics::Finite => "in nonlinear steps (their finite-strain form, enhanced assumed strain, is not implemented",
                    Kinematics::Small => "with an inelastic material (their internal modes would carry the plastic history",
                };
                Err(Error::NotSupported(format!("{} incompatible modes {why}; use full integration, hybrid or hex20)", self.ty.kind.name())))
            }
            Formulation::Hybrid { .. } if !m.is_decoupled() => Err(Error::NotSupported(format!(
                "hybrid elements of a {} material in nonlinear steps (the mixed formulation needs a volumetric–isochoric split of the strain energy; give the material the neo-Hookean option)",
                m.name()
            ))),
            _ => Ok(()),
        }
    }

    /// F, J and the gradients at every stiffness point: the current ones at finite strain, the
    /// reference ones at small strain.
    fn kinematics(&self, u: &[[f64; 3]], kin: Kinematics) -> Result<Vec<PointKinematics>> {
        let n = self.ty.nodes;
        let mut out = Vec::new();
        for p in self.rule().points() {
            let g = self.gradients(p.xi)?;
            let reference = g.dx.get(..n).unwrap_or_default();
            let f = crate::constitutive::deformation_gradient(u, reference);
            let (weight, x0) = (g.det * p.weight, self.position_of(g.shape.values()));
            if kin == Kinematics::Small {
                let strain = small_strain(&f);
                let j = 1.0 + strain[0] + strain[1] + strain[2];
                out.push(PointKinematics { weight, f, j, strain, gradients: reference.to_vec(), x0 });
                continue;
            }
            let (inv, j) = inverse3(&f).ok_or_else(|| inverted(self.ty.kind.name(), 0.0))?;
            if !(j > 0.0 && j.is_finite()) {
                return Err(inverted(self.ty.kind.name(), j));
            }
            let gradients = reference.iter().map(|d| std::array::from_fn(|i| (0..3).map(|jj| d[jj] * inv[jj][i]).sum())).collect();
            out.push(PointKinematics { weight, f, j, strain: [0.0; 6], gradients, x0 });
        }
        Ok(out)
    }

    /// The effective response (Kirchhoff stress and tangent, or the stress of small strain) at
    /// every point, the forces, and the tangent when `tangent`.
    fn nonlinear_points(
        &self,
        m: &MaterialModel,
        u: &[[f64; 3]],
        kin: Kinematics,
        committed: &[f64],
        updated: &mut [f64],
        tangent: bool,
    ) -> Result<(Finite, Vec<(PointKinematics, Response)>)> {
        self.check_nonlinear(m, u, kin)?;
        let points = self.kinematics(u, kin)?;
        let len = check_history(m, points.len(), committed, updated)?;
        let ndof = 3 * self.ty.nodes;
        let mut force = vec![0.0; ndof];
        let mut k = tangent.then(|| Matrix::zeros(ndof));
        let responses: Vec<Response> = if let Formulation::Hybrid { .. } = self.formulation {
            self.mixed_responses(m, &points, kin, committed, updated, k.as_mut())?
        } else {
            let mut out = Vec::with_capacity(points.len());
            for (i, p) in points.iter().enumerate() {
                let updated_point = updated.get_mut(i * len..(i + 1) * len).unwrap_or_default();
                out.push(point_response(m, kin, p, point_history(committed, len, i), updated_point)?);
            }
            out
        };
        for (p, r) in points.iter().zip(&responses) {
            add_point(&mut force, k.as_mut(), &p.gradients, r, p.weight, kin == Kinematics::Finite);
        }
        Ok((Finite { force, tangent: k }, points.into_iter().zip(responses).collect()))
    }

    /// The three-field mixed responses τ_iso + J p I with their tangents (small strain:
    /// s + p I), and the dilatation stiffness Gᵀ M⁻¹ H M⁻¹ G added to `k`.
    fn mixed_responses(
        &self,
        m: &MaterialModel,
        points: &[PointKinematics],
        kin: Kinematics,
        committed: &[f64],
        updated: &mut [f64],
        k: Option<&mut Matrix>,
    ) -> Result<Vec<Response>> {
        let small = kin == Kinematics::Small;
        let len = m.history_len();
        let (np, frame) = (self.np(), self.frame());
        let basis: Vec<[f64; 4]> = points.iter().map(|p| pressure_basis(np, p.x0, &frame)).collect();
        let mut mm = Matrix::zeros(np);
        let mut theta = vec![0.0; np];
        for (p, nb) in points.iter().zip(&basis) {
            for a in 0..np {
                theta[a] += p.weight * nb[a] * p.j;
                for b in 0..np {
                    mm.add(a, b, p.weight * nb[a] * nb[b]);
                }
            }
        }
        let chol = Cholesky::new(&mm)?;
        chol.solve(&mut theta);
        let mut pressure = vec![0.0; np];
        let mut h = Matrix::zeros(np);
        for (p, nb) in points.iter().zip(&basis) {
            let th: f64 = (0..np).map(|a| nb[a] * theta[a]).sum();
            if !(th > 0.0 && th.is_finite()) {
                return Err(inverted(self.ty.kind.name(), th));
            }
            let (dp, ddp) = m.volumetric(th, small).ok_or_else(|| Error::Invalid("volumetric response".into()))?;
            for a in 0..np {
                pressure[a] += p.weight * nb[a] * dp;
                for b in 0..np {
                    h.add(a, b, p.weight * ddp * nb[a] * nb[b]);
                }
            }
        }
        chol.solve(&mut pressure);
        let mut out = Vec::with_capacity(points.len());
        for (i, (p, nb)) in points.iter().zip(&basis).enumerate() {
            let pr: f64 = (0..np).map(|a| nb[a] * pressure[a]).sum();
            let (c, u) = (point_history(committed, len, i), updated.get_mut(i * len..(i + 1) * len).unwrap_or_default());
            out.push(if small {
                let mut r = m.small_strain(&p.strain, None, c, u, false)?;
                r.stress.iter_mut().take(3).for_each(|s| *s += pr);
                r
            } else {
                add_pressure(&m.isochoric(&p.f, None, c, u)?, p.j, pr, 0.0)
            });
        }
        if let Some(k) = k {
            // A = M⁻¹ G, np × ndof; K += Aᵀ H A. At small strain G = ∫ N_p ∇₀N dV₀.
            let ndof = k.n;
            let mut a_cols = vec![vec![0.0; np]; ndof];
            for (p, nb) in points.iter().zip(&basis) {
                let jw = p.weight * if small { 1.0 } else { p.j };
                for (node, g) in p.gradients.iter().enumerate() {
                    for c in 0..3 {
                        if let Some(col) = a_cols.get_mut(3 * node + c) {
                            for (x, n) in col.iter_mut().zip(nb) {
                                *x += jw * n * g[c];
                            }
                        }
                    }
                }
            }
            for col in &mut a_cols {
                chol.solve(col);
            }
            let ha: Vec<Vec<f64>> = a_cols.iter().map(|col| h.mul_vec(col)).collect();
            for i in 0..ndof {
                for jj in i..ndof {
                    let v: f64 = a_cols.get(i).zip(ha.get(jj)).map_or(0.0, |(x, y)| x.iter().zip(y).map(|(p, q)| p * q).sum());
                    k.add(i, jj, v);
                    if i != jj {
                        k.add(jj, i, v);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Internal forces and (with `tangent`) the tangent stiffness at nodal displacements `u`,
    /// with the material history `committed` at the start of the increment (points × the
    /// material's history length) written, updated, to `updated`.
    pub fn finite_strain(&self, m: &MaterialModel, u: &[[f64; 3]], committed: &[f64], updated: &mut [f64], tangent: bool) -> Result<Finite> {
        Ok(self.nonlinear_points(m, u, Kinematics::Finite, committed, updated, tangent)?.0)
    }

    /// As [`Self::finite_strain`] with either kinematics; with small strain an inelastic
    /// material on the linear kinematics, f = ∫ Bᵀ σ dV and K = ∫ Bᵀ 𝔻 B dV with the
    /// material's consistent tangent 𝔻.
    pub fn nonlinear(
        &self,
        m: &MaterialModel,
        u: &[[f64; 3]],
        kin: Kinematics,
        committed: &[f64],
        updated: &mut [f64],
        tangent: bool,
    ) -> Result<Finite> {
        Ok(self.nonlinear_points(m, u, kin, committed, updated, tangent)?.0)
    }

    /// The Cauchy stress and the strain (logarithmic at finite strain, ε at small strain) at
    /// every integration point (at its reference position).
    pub fn nonlinear_stresses(&self, m: &MaterialModel, u: &[[f64; 3]], kin: Kinematics, committed: &[f64]) -> Result<Vec<PointStress>> {
        let mut scratch = committed.to_vec();
        let (_, points) = self.nonlinear_points(m, u, kin, committed, &mut scratch, false)?;
        points
            .iter()
            .map(|(p, r)| {
                if kin == Kinematics::Small {
                    return Ok(PointStress { xyz: p.x0, strain: p.strain, stress: r.stress });
                }
                let strain = logarithmic_strain(&p.f).ok_or_else(|| inverted(self.ty.kind.name(), p.j))?;
                Ok(PointStress { xyz: p.x0, strain, stress: r.stress.map(|v| v / p.j) })
            })
            .collect()
    }
}

/// One integration point of a 2D element in a nonlinear step.
struct PlanePoint {
    f: Mat3,
    j: f64,
    x: [f64; 3],
    strain: [f64; 6],
    response: Response,
}

impl Plane<'_> {
    fn check_nonlinear(&self, u: &[[f64; 2]], kin: Kinematics) -> Result<()> {
        check_displacements(u, self.ty.nodes)?;
        if let Theory::PlaneStress { .. } = self.theory {
            let why = match kin {
                Kinematics::Finite => "the thickness change needs a local iteration",
                Kinematics::Small => "σ_zz = 0 needs a local iteration of the inelastic material",
            };
            return Err(Error::NotSupported(format!(
                "plane stress elements in nonlinear steps ({why}; use plane strain, axisymmetric or solid elements)"
            )));
        }
        if self.formulation == Formulation::IncompatibleModes {
            return Err(Error::NotSupported(format!(
                "2D {} incompatible modes in nonlinear steps (their finite-strain form, enhanced assumed strain, is not implemented; use full integration or quad8)",
                self.ty.kind.name()
            )));
        }
        Ok(())
    }

    /// Per point: F, J, the reference position, the small strain and the response; the forces
    /// and the tangent.
    fn nonlinear_points(
        &self,
        m: &MaterialModel,
        u: &[[f64; 2]],
        kin: Kinematics,
        committed: &[f64],
        updated: &mut [f64],
        tangent: bool,
    ) -> Result<(Finite, Vec<PlanePoint>)> {
        self.check_nonlinear(u, kin)?;
        let n = self.ty.nodes;
        let rule = self.rule().points();
        let len = check_history(m, rule.len(), committed, updated)?;
        let ndof = 2 * n;
        let mut force = vec![0.0; ndof];
        let mut k = tangent.then(|| Matrix::zeros(ndof));
        let axisymmetric = self.theory == Theory::Axisymmetric;
        let small = kin == Kinematics::Small;
        let mut out = Vec::with_capacity(rule.len());
        for (i, p) in rule.iter().enumerate() {
            let at = self.at(p)?;
            let cols = at.g.get(..n).unwrap_or_default();
            let mut f = IDENTITY;
            for (ua, g) in u.iter().zip(cols) {
                for r in 0..2 {
                    for c in 0..2 {
                        f[r][c] += ua[r] * g[c];
                    }
                }
                if axisymmetric {
                    f[2][2] += ua[0] * g[2];
                }
            }
            let updated_point = updated.get_mut(i * len..(i + 1) * len).unwrap_or_default();
            let (j, strain, current, r) = if small {
                let strain = small_strain(&f);
                let r = m.small_strain(&strain, None, point_history(committed, len, i), updated_point, true)?;
                (1.0 + strain[0] + strain[1] + strain[2], strain, cols.to_vec(), r)
            } else {
                let det2 = f[0][0] * f[1][1] - f[0][1] * f[1][0];
                let j = det2 * f[2][2];
                if !(j > 0.0 && j.is_finite() && f[2][2] > 0.0) {
                    return Err(inverted(&format!("2D {}", self.ty.kind.name()), j));
                }
                let inv = [[f[1][1] / det2, -f[0][1] / det2], [-f[1][0] / det2, f[0][0] / det2]];
                let current: Vec<[f64; 3]> =
                    cols.iter().map(|g| [g[0] * inv[0][0] + g[1] * inv[1][0], g[0] * inv[0][1] + g[1] * inv[1][1], g[2] / f[2][2]]).collect();
                let r = m.response(&f, None, point_history(committed, len, i), updated_point)?;
                (j, [0.0; 6], current, r)
            };
            let tau = [r.stress[0], r.stress[1], r.stress[2], r.stress[3]];
            for (a, g) in current.iter().enumerate() {
                if let Some(x) = force.get_mut(2 * a) {
                    *x += at.dv * (g[0] * tau[0] + g[2] * tau[2] + g[1] * tau[3]);
                }
                if let Some(x) = force.get_mut(2 * a + 1) {
                    *x += at.dv * (g[1] * tau[1] + g[0] * tau[3]);
                }
            }
            if let Some(k) = k.as_mut() {
                let c4: [[f64; 4]; 4] = std::array::from_fn(|a| std::array::from_fn(|b| r.tangent[a][b]));
                crate::plane::accumulate(&mut k.data, &current, &c4, at.dv);
                if !small {
                    for (a, ga) in current.iter().enumerate() {
                        for (b, gb) in current.iter().enumerate() {
                            let s = ga[0] * gb[0] * tau[0] + ga[1] * gb[1] * tau[1] + (ga[0] * gb[1] + ga[1] * gb[0]) * tau[3];
                            k.add(2 * a, 2 * b, at.dv * (s + tau[2] * ga[2] * gb[2]));
                            k.add(2 * a + 1, 2 * b + 1, at.dv * s);
                        }
                    }
                }
            }
            out.push(PlanePoint { f, j, x: at.x, strain, response: r });
        }
        Ok((Finite { force, tangent: k }, out))
    }

    /// Internal forces (u, v per node) and (with `tangent`) the tangent stiffness at nodal
    /// displacements `u`, as for [`Solid::finite_strain`].
    pub fn finite_strain(&self, m: &MaterialModel, u: &[[f64; 2]], committed: &[f64], updated: &mut [f64], tangent: bool) -> Result<Finite> {
        Ok(self.nonlinear_points(m, u, Kinematics::Finite, committed, updated, tangent)?.0)
    }

    /// As [`Solid::nonlinear`], on the 2D element.
    pub fn nonlinear(
        &self,
        m: &MaterialModel,
        u: &[[f64; 2]],
        kin: Kinematics,
        committed: &[f64],
        updated: &mut [f64],
        tangent: bool,
    ) -> Result<Finite> {
        Ok(self.nonlinear_points(m, u, kin, committed, updated, tangent)?.0)
    }

    /// The Cauchy stress and the strain at every integration point, as the solid elements'
    /// 6-vectors (axisymmetric: rr, axial, hoop, r-axial).
    pub fn nonlinear_stresses(&self, m: &MaterialModel, u: &[[f64; 2]], kin: Kinematics, committed: &[f64]) -> Result<Vec<PointStress>> {
        let mut scratch = committed.to_vec();
        let (_, points) = self.nonlinear_points(m, u, kin, committed, &mut scratch, false)?;
        points
            .iter()
            .map(|p| {
                if kin == Kinematics::Small {
                    return Ok(PointStress { xyz: p.x, strain: p.strain, stress: p.response.stress });
                }
                let strain = logarithmic_strain(&p.f).ok_or_else(|| inverted(&format!("2D {}", self.ty.kind.name()), p.j))?;
                Ok(PointStress { xyz: p.x, strain, stress: p.response.stress.map(|v| v / p.j) })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::isotropic;
    use fieldcraft_model::ElementKind;

    fn hex20() -> Vec<[f64; 3]> {
        let mut x: Vec<[f64; 3]> = crate::shape::HEX_CORNERS.iter().map(|c| [c[0] + 1.0, 0.5 * c[1] + 1.0, 0.8 * c[2]]).collect();
        let edges = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)];
        for (a, b) in edges {
            x.push(std::array::from_fn(|k| 0.5 * (x[a][k] + x[b][k])));
        }
        // Distort a corner and a mid-side node.
        x[6][0] += 0.2;
        x[9][2] += 0.05;
        x
    }

    fn tet10() -> Vec<[f64; 3]> {
        let c = [[0.0, 0.0, 0.0], [1.2, 0.1, 0.0], [0.2, 1.0, 0.1], [0.1, 0.2, 0.9]];
        let mut x = c.to_vec();
        for (a, b) in [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)] {
            x.push(std::array::from_fn(|k| 0.5 * (c[a][k] + c[b][k])));
        }
        x
    }

    fn displacement(n: usize, scale: f64) -> Vec<[f64; 3]> {
        (0..n).map(|i| [scale * (i as f64 * 1.3).sin(), scale * (i as f64 * 0.7).cos(), scale * (i as f64 * 2.1).sin()]).collect()
    }

    /// Central differences of the internal forces against the tangent.
    fn check_tangent(solid: &Solid, m: &MaterialModel, u: &[[f64; 3]]) {
        check_tangent_from(solid, m, u, Kinematics::Finite, &[], 1e-6);
    }

    /// The same from a committed history, with either kinematics.
    fn check_tangent_from(solid: &Solid, m: &MaterialModel, u: &[[f64; 3]], kin: Kinematics, committed: &[f64], tol: f64) {
        let mut scratch = committed.to_vec();
        let k = solid.nonlinear_points(m, u, kin, committed, &mut scratch, true).unwrap().0.tangent.unwrap();
        let scale = k.data.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        let n = 3 * u.len();
        let h = if kin == Kinematics::Small { 1e-9 } else { 1e-6 };
        for j in 0..n {
            let shifted = |s: f64| {
                let mut v = u.to_vec();
                v[j / 3][j % 3] += s;
                solid.nonlinear_points(m, &v, kin, committed, &mut committed.to_vec(), false).unwrap().0.force
            };
            let (p, q) = (shifted(h), shifted(-h));
            for i in 0..n {
                let numeric = (p[i] - q[i]) / (2.0 * h);
                assert!((numeric - k.get(i, j)).abs() < tol * scale, "{:?} {kin:?} {i} {j}: {numeric} vs {}", solid.ty.kind, k.get(i, j));
            }
        }
        for i in 0..n {
            for j in 0..n {
                assert!((k.get(i, j) - k.get(j, i)).abs() <= 1e-12 * scale, "symmetric");
            }
        }
    }

    #[test]
    fn solid_tangents_are_the_derivative_of_the_forces() {
        let d = isotropic(1000.0, 0.3).unwrap();
        let neo = MaterialModel::NeoHookean { shear: 300.0, bulk: 3000.0 };
        let svk = MaterialModel::StVenantKirchhoff(Box::new(d));
        let (x20, x10) = (hex20(), tet10());
        let x8: Vec<[f64; 3]> = x20[..8].to_vec();
        let x6 = vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.1], [0., 0., 1.], [1.1, 0., 1.], [0., 1., 1.2]];
        let x4 = vec![x10[0], x10[1], x10[2], x10[3]];
        for (kind, x) in
            [(ElementKind::Tet4, &x4), (ElementKind::Tet10, &x10), (ElementKind::Wedge6, &x6), (ElementKind::Hex8, &x8), (ElementKind::Hex20, &x20)]
        {
            for m in [&svk, &neo] {
                let solid = Solid::new(kind, x, Formulation::Full).unwrap();
                check_tangent(&solid, m, &displacement(x.len(), 0.08));
            }
        }
        let reduced = Solid::new(ElementKind::Hex20, &x20, Formulation::Reduced).unwrap();
        check_tangent(&reduced, &neo, &displacement(20, 0.08));
        for (kind, x, np) in [(ElementKind::Hex8, &x8, 1), (ElementKind::Hex20, &x20, 4), (ElementKind::Tet10, &x10, 1)] {
            let hybrid = Solid::new(kind, x, Formulation::Hybrid { pressures: np }).unwrap();
            check_tangent(&hybrid, &neo, &displacement(x.len(), 0.08));
            assert!(hybrid.finite_strain(&svk, &displacement(x.len(), 0.0), &[], &mut [], false).is_err(), "SVK has no split");
        }
        let im = Solid::new(ElementKind::Hex8, &x8, Formulation::IncompatibleModes).unwrap();
        assert!(matches!(im.finite_strain(&svk, &displacement(8, 0.0), &[], &mut [], true), Err(Error::NotSupported(_))));
    }

    #[test]
    fn at_zero_displacement_the_tangent_is_the_linear_stiffness() {
        let (e, nu) = (1000.0, 0.45);
        let d = isotropic(e, nu).unwrap();
        let neo = MaterialModel::NeoHookean { shear: e / (2.0 * (1.0 + nu)), bulk: e / (3.0 * (1.0 - 2.0 * nu)) };
        let x20 = hex20();
        for (f, m) in [
            (Formulation::Full, MaterialModel::StVenantKirchhoff(Box::new(d))),
            (Formulation::Full, neo.clone()),
            (Formulation::Hybrid { pressures: 4 }, neo),
        ] {
            let solid = Solid::new(ElementKind::Hex20, &x20, f).unwrap();
            let r = solid.finite_strain(&m, &vec![[0.0; 3]; 20], &[], &mut [], true).unwrap();
            let linear = solid.stiffness(&d).unwrap();
            let k = r.tangent.unwrap();
            for (a, b) in k.data.iter().zip(&linear.data) {
                assert!((a - b).abs() < 1e-9 * e, "{f:?}: {a} vs {b}");
            }
            assert!(r.force.iter().all(|v| v.abs() < 1e-12 * e));
        }
    }

    /// J2 plasticity in full and hybrid elements, at finite and small strain: the tangent is
    /// the derivative of the forces from a plastic history too, and the plastic points are
    /// really plastic.
    #[test]
    fn plastic_tangents_are_the_derivative_of_the_forces() {
        use crate::elastoplastic::{HISTORY, PLASTIC_STRAIN};
        use fieldcraft_model::{Elastic, Hardening, Plastic};
        let j2 = crate::plasticity::J2::from_model(
            &Elastic::Isotropic { young: 1000.0, poisson: 0.3 },
            Some(&Plastic::Bilinear { yield_stress: 5.0, hardening_modulus: 50.0, hardening: Hardening::Isotropic }),
            None,
        )
        .unwrap();
        let m = MaterialModel::Plastic(std::sync::Arc::new(j2));
        let (x20, x10) = (hex20(), tet10());
        let x8: Vec<[f64; 3]> = x20[..8].to_vec();
        for (kind, x, f) in [
            (ElementKind::Hex8, &x8, Formulation::Full),
            (ElementKind::Tet10, &x10, Formulation::Full),
            (ElementKind::Hex20, &x20, Formulation::Reduced),
            (ElementKind::Hex8, &x8, Formulation::Hybrid { pressures: 1 }),
            (ElementKind::Hex20, &x20, Formulation::Hybrid { pressures: 4 }),
        ] {
            let solid = Solid::new(kind, x, f).unwrap();
            let points = solid.rule().points().len();
            for (kin, scale) in [(Kinematics::Finite, 0.05), (Kinematics::Small, 0.05)] {
                // A first increment from the virgin state, committed, then a second from it.
                let mut committed = vec![0.0; points * HISTORY];
                let first = displacement(x.len(), 0.5 * scale);
                solid.nonlinear_points(&m, &first, kin, &vec![0.0; points * HISTORY], &mut committed, false).unwrap();
                assert!((0..points).all(|p| committed[p * HISTORY + PLASTIC_STRAIN] > 0.0), "{kind:?} {f:?} {kin:?} yields everywhere");
                check_tangent_from(&solid, &m, &displacement(x.len(), scale), kin, &committed, 1e-6);
            }
        }
        // Incompatible modes have no inelastic form.
        let im = Solid::new(ElementKind::Hex8, &x8, Formulation::IncompatibleModes).unwrap();
        assert!(matches!(
            im.nonlinear(&m, &displacement(8, 0.0), Kinematics::Small, &[0.0; 8 * HISTORY], &mut [0.0; 8 * HISTORY], true),
            Err(Error::NotSupported(_))
        ));
    }

    #[test]
    fn plastic_plane_strain_and_axisymmetric_tangents_are_the_derivative_of_the_forces() {
        use crate::elastoplastic::{HISTORY, PLASTIC_STRAIN};
        use fieldcraft_model::{Elastic, Hardening, Plastic};
        let j2 = crate::plasticity::J2::from_model(
            &Elastic::Isotropic { young: 1000.0, poisson: 0.3 },
            Some(&Plastic::Bilinear { yield_stress: 5.0, hardening_modulus: 50.0, hardening: Hardening::Isotropic }),
            None,
        )
        .unwrap();
        let m = MaterialModel::Plastic(std::sync::Arc::new(j2));
        let quad8 = [
            [1.0, 0.0, 0.0],
            [2.0, 0.1, 0.0],
            [2.2, 1.0, 0.0],
            [0.9, 1.1, 0.0],
            [1.5, 0.0, 0.0],
            [2.1, 0.55, 0.0],
            [1.5, 1.1, 0.0],
            [0.95, 0.5, 0.0],
        ];
        for theory in [Theory::PlaneStrain { thickness: 2.0 }, Theory::Axisymmetric] {
            for (kind, x) in [(ElementKind::Quad8, &quad8[..]), (ElementKind::Tri3, &quad8[..3])] {
                let plane = Plane::new(kind, x, Formulation::Full, theory).unwrap();
                let points = plane.rule().points().len();
                for kin in [Kinematics::Finite, Kinematics::Small] {
                    let u = |scale: f64| -> Vec<[f64; 2]> { displacement(x.len(), scale).iter().map(|v| [v[0], v[1]]).collect() };
                    let mut committed = vec![0.0; points * HISTORY];
                    plane.nonlinear_points(&m, &u(0.03), kin, &vec![0.0; points * HISTORY], &mut committed, false).unwrap();
                    assert!((0..points).all(|p| committed[p * HISTORY + PLASTIC_STRAIN] > 0.0), "{theory:?} {kind:?} {kin:?}");
                    let at = u(0.06);
                    let k = plane.nonlinear_points(&m, &at, kin, &committed, &mut committed.clone(), true).unwrap().0.tangent.unwrap();
                    let scale = k.data.iter().fold(0.0f64, |a, v| a.max(v.abs()));
                    let h = 1e-8;
                    for j in 0..2 * x.len() {
                        let shifted = |s: f64| {
                            let mut v = at.clone();
                            v[j / 2][j % 2] += s;
                            plane.nonlinear_points(&m, &v, kin, &committed, &mut committed.clone(), false).unwrap().0.force
                        };
                        let (p, q) = (shifted(h), shifted(-h));
                        for i in 0..2 * x.len() {
                            let numeric = (p[i] - q[i]) / (2.0 * h);
                            assert!(
                                (numeric - k.get(i, j)).abs() < 1e-6 * scale,
                                "{theory:?} {kind:?} {kin:?} {i} {j}: {numeric} vs {}",
                                k.get(i, j)
                            );
                        }
                    }
                }
            }
        }
        let stress = Plane::new(ElementKind::Quad4, &quad8[..4], Formulation::Full, Theory::PlaneStress { thickness: 1.0 }).unwrap();
        assert!(matches!(
            stress.nonlinear(&m, &[[0.0; 2]; 4], Kinematics::Small, &[0.0; 4 * HISTORY], &mut [0.0; 4 * HISTORY], true),
            Err(Error::NotSupported(_))
        ));
    }

    #[test]
    fn rigid_rotation_strains_and_stresses_nothing() {
        let x20 = hex20();
        let rot = [[0.0, -0.6, 0.8], [0.6, 0.64, 0.48], [-0.8, 0.48, 0.36]];
        let u: Vec<[f64; 3]> = x20.iter().map(|p| std::array::from_fn(|i| (0..3).map(|jj| rot[i][jj] * p[jj]).sum::<f64>() + 5.0 - p[i])).collect();
        let neo = MaterialModel::NeoHookean { shear: 300.0, bulk: 3000.0 };
        for f in [Formulation::Full, Formulation::Hybrid { pressures: 4 }] {
            let solid = Solid::new(ElementKind::Hex20, &x20, f).unwrap();
            let s = solid.nonlinear_stresses(&neo, &u, Kinematics::Finite, &[]).unwrap();
            assert!(s.iter().all(|p| p.stress.iter().chain(&p.strain).all(|v| v.abs() < 1e-12 * 3000.0)), "{f:?}: {:?}", s[0]);
            let r = solid.finite_strain(&neo, &u, &[], &mut [], false).unwrap();
            assert!(r.force.iter().all(|v| v.abs() < 1e-11 * 3000.0));
        }
        // A rotation that inverts the element is refused.
        let flip: Vec<[f64; 3]> = x20.iter().map(|p| [-2.0 * p[0], 0.0, 0.0]).collect();
        let solid = Solid::new(ElementKind::Hex20, &x20, Formulation::Full).unwrap();
        assert!(matches!(solid.finite_strain(&neo, &flip, &[], &mut [], false), Err(Error::Degenerate(_))));
    }

    #[test]
    fn plane_strain_and_axisymmetric_tangents_are_the_derivative_of_the_forces() {
        let quad8 = [
            [1.0, 0.0, 0.0],
            [2.0, 0.1, 0.0],
            [2.2, 1.0, 0.0],
            [0.9, 1.1, 0.0],
            [1.5, 0.0, 0.0],
            [2.1, 0.55, 0.0],
            [1.5, 1.1, 0.0],
            [0.95, 0.5, 0.0],
        ];
        let neo = MaterialModel::NeoHookean { shear: 300.0, bulk: 3000.0 };
        let svk = MaterialModel::StVenantKirchhoff(Box::new(isotropic(1000.0, 0.3).unwrap()));
        for theory in [Theory::PlaneStrain { thickness: 2.0 }, Theory::Axisymmetric] {
            for (kind, x) in [(ElementKind::Quad8, &quad8[..]), (ElementKind::Quad4, &quad8[..4]), (ElementKind::Tri3, &quad8[..3])] {
                let plane = Plane::new(kind, x, Formulation::Full, theory).unwrap();
                for m in [&neo, &svk] {
                    let u: Vec<[f64; 2]> = displacement(x.len(), 0.06).iter().map(|v| [v[0], v[1]]).collect();
                    let k = plane.finite_strain(m, &u, &[], &mut [], true).unwrap().tangent.unwrap();
                    let scale = k.data.iter().fold(0.0f64, |a, v| a.max(v.abs()));
                    let h = 1e-6;
                    for j in 0..2 * x.len() {
                        let shifted = |s: f64| {
                            let mut v = u.clone();
                            v[j / 2][j % 2] += s;
                            plane.finite_strain(m, &v, &[], &mut [], false).unwrap().force
                        };
                        let (p, q) = (shifted(h), shifted(-h));
                        for i in 0..2 * x.len() {
                            let numeric = (p[i] - q[i]) / (2.0 * h);
                            assert!((numeric - k.get(i, j)).abs() < 1e-6 * scale, "{theory:?} {kind:?} {i} {j}: {numeric} vs {}", k.get(i, j));
                        }
                    }
                }
                // At rest the tangent is the linear stiffness.
                let d = isotropic(1000.0, 0.3).unwrap();
                let k = plane.finite_strain(&svk, &vec![[0.0; 2]; x.len()], &[], &mut [], true).unwrap().tangent.unwrap();
                for (a, b) in k.data.iter().zip(&plane.stiffness(&d).unwrap().data) {
                    assert!((a - b).abs() < 1e-9 * 1000.0);
                }
            }
        }
        let stress = Plane::new(ElementKind::Quad4, &quad8[..4], Formulation::Full, Theory::PlaneStress { thickness: 1.0 }).unwrap();
        assert!(matches!(stress.finite_strain(&svk, &[[0.0; 2]; 4], &[], &mut [], true), Err(Error::NotSupported(_))));
    }
}
