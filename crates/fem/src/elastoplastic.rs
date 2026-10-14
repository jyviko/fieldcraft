//! Von Mises (J2) elastoplasticity at a material point of implicit (Newton) analysis: the
//! stress after an increment and its consistent tangent, at finite strain and at small strain.
//! Both kinematics call the radial return of [`crate::plasticity`], the update explicit
//! dynamics uses, so the yield surface, the hardening (bilinear isotropic or linear kinematic,
//! tabulated, Johnson–Cook) and the plastic strain increment of a trial state are one
//! implementation for both solvers.
//!
//! **Finite strain** (J. C. Simo, *Algorithms for static and dynamic multiplicative plasticity
//! that preserve the classical return mapping schemes of the infinitesimal theory*, CMAME
//! 99:61–112, 1992; Simo & Hughes 1998, ch. 9): the multiplicative split F = Fₑ Fₚ with the
//! elastic left Cauchy–Green tensor bₑ = F Cₚ⁻¹ Fᵀ and a Hencky (quadratic in the logarithmic
//! strain) elastic energy, so the Kirchhoff stress is τ = K ln J I + 2G dev εₑ with
//! εₑ = ½ ln bₑ. The history is Cₚ⁻¹ (stored as Cₚ⁻¹ − I, so the virgin state is zeros) and the
//! equivalent plastic strain. An increment freezes Cₚ⁻¹ at its committed value: the trial
//! bₑᵗʳ = F Cₚ⁻¹ Fᵀ, its principal stretches and directions, the trial logarithmic strains
//! ε_A = ½ ln λ_A², and then the **return map of the infinitesimal theory on the principal
//! values** (the exponential map integrates the flow rule exactly in the principal frame, which
//! the isotropy of the model keeps fixed during the return). The update writes back
//! bₑ = Σ exp(2 εₑ,A) n_A ⊗ n_A and Cₚ⁻¹ = F⁻¹ bₑ F⁻ᵀ, which keeps det Cₚ = 1 (plastic flow
//! is isochoric).
//!
//! **Its tangent.** With Cₚ⁻¹ frozen, the Lie derivative of bₑᵗʳ vanishes, so for a rate of
//! deformation d the trial strain changes by dεᵗʳ = ½ (∂ ln b/∂b) : (d b + b d), and
//! L_v τ = 𝔻ᵉᵖ : dεᵗʳ − (d τ + τ d), with 𝔻ᵉᵖ the algorithmic tangent of the return map
//! ([`crate::plasticity::J2::tangent`]). In the principal frame of b the derivative of the
//! logarithm is the Daleckii–Krein divided difference, so dεᵗʳ_AB = θ_AB d_AB with θ_AA = 1 and
//! θ_AB = ½ (λ²_A + λ²_B) ln(λ²_A/λ²_B)/(λ²_A − λ²_B) = artanh(r)/r, r = (λ²_A − λ²_B)/(λ²_A + λ²_B),
//! smooth through equal stretches (its series 1 + r²/3 + r⁴/5 + … near r = 0), so uniaxial and
//! undeformed states need no special case (Simo 1992, §3.3, gives the same tangent in spectral
//! form; R. Bhatia, *Matrix Analysis*, Springer 1997, §V.3, for the divided differences). The
//! tangent is symmetric for isotropic hardening: 𝔻ᵉᵖ and the θ scaling share the principal
//! frame.
//!
//! **Kinematic hardening at finite strain** is refused: a back stress is not coaxial with bₑ, so
//! the principal-frame return no longer holds, and it needs a formulation of its own (for
//! example the Lagrangian logarithmic strain space of C. Miehe, N. Apel, M. Lambrecht, CMAME
//! 191:5383–5425, 2002). It works with small strain.
//!
//! **Small strain** (Simo & Hughes 1998, §3.3): ε = εₑ + εₚ additively, the history the plastic
//! strain εₚ, the back stress α and the equivalent plastic strain; the trial εₑ = ε − εₚ,ₙ goes
//! through the same return, and εₚ advances by √(3/2) Δε̄ₚ n.
//!
//! **Mixed (hybrid) elements** take the isochoric part (the deviatoric return, without K) and the
//! volumetric energy U(J) = K/2 (ln J)², U′ = K ln J / J (finite strain), or U = K/2 (J − 1)² with
//! J the 1 + tr ε of small strain: J2 plasticity is pressure insensitive and isochoric, so the
//! split is exact.
//!
//! **History layout** per integration point ([`HISTORY`] values): 0–5 Cₚ⁻¹ − I (finite strain,
//! tensor components) or εₚ (small strain, engineering shears), 6–11 the back stress α, 12 the
//! equivalent plastic strain.

use crate::constitutive::{Response, Tangent};
use crate::dense::{inverse3, symmetric_eigen};
use crate::plasticity::{J2, PointState, Return};
use crate::rotation::{IDENTITY, Mat3, mul, tensor, transpose, voigt};
use crate::{Error, Result};

/// History values per integration point.
pub const HISTORY: usize = 13;

/// Where the equivalent plastic strain is in a point's history.
pub const PLASTIC_STRAIN: usize = 12;

/// History value `i` (0 when the slice is short).
pub(crate) fn value(h: &[f64], i: usize) -> f64 {
    h.get(i).copied().unwrap_or(0.0)
}

/// Cₚ⁻¹ of a finite-strain history (stored as Cₚ⁻¹ − I).
fn plastic_stretch(h: &[f64]) -> Mat3 {
    let stored = tensor(&std::array::from_fn(|k| value(h, k)));
    std::array::from_fn(|i| std::array::from_fn(|j| IDENTITY[i][j] + stored[i][j]))
}

/// Store Cₚ⁻¹ as Cₚ⁻¹ − I in the first six history values.
fn store_plastic_stretch(cp: &Mat3, h: &mut [f64]) {
    let v = voigt(cp);
    for (k, slot) in h.iter_mut().take(6).enumerate() {
        *slot = v[k] - if k < 3 { 1.0 } else { 0.0 };
    }
}

/// artanh(r)/r, smooth through r = 0.
fn log_divided_difference(r: f64) -> f64 {
    if r.abs() < 1e-4 {
        let r2 = r * r;
        1.0 + r2 * (1.0 / 3.0 + r2 * (1.0 / 5.0 + r2 / 7.0))
    } else {
        r.atanh() / r
    }
}

/// The state a return starts from: the committed back stress and plastic strain, at the point's
/// temperature (Johnson–Cook softening; room temperature without one).
fn start(j2: &J2, committed: &[f64], temperature: Option<f64>) -> PointState {
    let mut s = PointState::new(temperature.unwrap_or_else(|| j2.initial_temperature()));
    s.back = std::array::from_fn(|k| value(committed, 6 + k));
    s.plastic_strain = value(committed, PLASTIC_STRAIN);
    s
}

fn singular() -> Error {
    Error::Degenerate("the plastic material point has no tangent: its hardening falls faster than −3G (a softening material localises)".into())
}

/// The deviatoric trial stress 2G dev ε of a strain 6-vector (engineering shears), as tensor
/// components.
fn trial_deviator(j2: &J2, e: &[f64; 6]) -> [f64; 6] {
    let mean = (e[0] + e[1] + e[2]) / 3.0;
    let g2 = 2.0 * j2.shear;
    [g2 * (e[0] - mean), g2 * (e[1] - mean), g2 * (e[2] - mean), j2.shear * e[3], j2.shear * e[4], j2.shear * e[5]]
}

/// Finite strain: the isochoric (deviatoric) Kirchhoff stress and its spatial tangent at
/// deformation gradient `f`, from the history `committed` (written, updated, to `updated`). Add
/// the volumetric part with [`volumetric`] (or the mixed element's pressure) for the whole
/// response.
pub fn finite_isochoric(j2: &J2, f: &Mat3, temperature: Option<f64>, committed: &[f64], updated: &mut [f64]) -> Result<Response> {
    if let Some(crate::plasticity::Flow::Linear { kinematic: true, .. }) = j2.flow {
        return Err(Error::NotSupported(
            "kinematic hardening at finite strain (its back stress is not coaxial with the elastic stretch; use nonlinear_geometry false or isotropic hardening)"
                .into(),
        ));
    }
    let b = mul(&mul(f, &plastic_stretch(committed)), &transpose(f));
    let (l, q) = symmetric_eigen(&b)
        .filter(|(l, _)| l.iter().all(|x| *x > 0.0))
        .ok_or_else(|| Error::Degenerate("the elastic stretch of a plastic material point is not positive (the material is inverted)".into()))?;
    let eps = l.map(|x| 0.5 * x.ln());
    let mut state = start(j2, committed, temperature);
    let mut trial = trial_deviator(j2, &[eps[0], eps[1], eps[2], 0.0, 0.0, 0.0]);
    let ret: Option<Return> = j2.radial_return(&mut state, &mut trial, 0.0);
    // Back to the global frame: τ = Σ τ_A q_A ⊗ q_A, n likewise.
    let spectral = |v: [f64; 3]| -> Mat3 { std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|a| v[a] * q[i][a] * q[j][a]).sum())) };
    let tau = voigt(&spectral([trial[0], trial[1], trial[2]]));
    let global = ret.map(|r| Return { normal: voigt(&spectral([r.normal[0], r.normal[1], r.normal[2]])), ..r });
    let d = j2.tangent(global.as_ref(), false).ok_or_else(singular)?;
    // θ_AB of the logarithm's derivative.
    let theta: Mat3 =
        std::array::from_fn(|a| std::array::from_fn(|bb| if a == bb { 1.0 } else { log_divided_difference((l[a] - l[bb]) / (l[a] + l[bb])) }));
    let tau_m = tensor(&tau);
    let qt = transpose(&q);
    let mut c: Tangent = [[0.0; 6]; 6];
    for (col, &(i, j)) in crate::constitutive::PAIRS.iter().enumerate() {
        // The unit rate of deformation of this column (engineering shear 1: d_ij = d_ji = ½).
        let mut dm = [[0.0; 3]; 3];
        let half = if i == j { 1.0 } else { 0.5 };
        dm[i][j] = half;
        dm[j][i] = half;
        let mut dp = mul(&mul(&qt, &dm), &q);
        for a in 0..3 {
            for bb in 0..3 {
                dp[a][bb] *= theta[a][bb];
            }
        }
        let de = voigt(&mul(&mul(&q, &dp), &qt));
        let de = [de[0], de[1], de[2], 2.0 * de[3], 2.0 * de[4], 2.0 * de[5]];
        let spin = voigt(&{
            let (x, y) = (mul(&dm, &tau_m), mul(&tau_m, &dm));
            std::array::from_fn(|r| std::array::from_fn(|s| x[r][s] + y[r][s]))
        });
        for row in 0..6 {
            c[row][col] = (0..6).map(|k| d[row][k] * de[k]).sum::<f64>() - spin[row];
        }
    }
    if let Some(r) = ret {
        // bₑ = Σ exp(2 εₑ,A) q_A ⊗ q_A with εₑ = ε − √(3/2) Δε̄ₚ n, and Cₚ⁻¹ = F⁻¹ bₑ F⁻ᵀ.
        let dg = (1.5f64).sqrt() * r.plastic_strain;
        let be = spectral(std::array::from_fn(|a| (2.0 * (eps[a] - dg * r.normal[a])).exp()));
        let (inv, _) = inverse3(f).ok_or_else(|| Error::Degenerate("the deformation gradient is singular".into()))?;
        store_plastic_stretch(&mul(&mul(&inv, &be), &transpose(&inv)), updated);
        if let Some(slot) = updated.get_mut(PLASTIC_STRAIN) {
            *slot = state.plastic_strain;
        }
    }
    Ok(Response { stress: tau, tangent: c })
}

/// U′(J) and U″(J) of the Hencky volumetric energy K/2 (ln J)² (finite strain), or of
/// K/2 (J − 1)² for the dilatation J = 1 + tr ε of small strain.
pub fn volumetric(j2: &J2, j: f64, small: bool) -> Option<(f64, f64)> {
    if !(j > 0.0 && j.is_finite()) {
        return None;
    }
    let k = j2.bulk;
    Some(if small { (k * (j - 1.0), k) } else { (k * j.ln() / j, k * (1.0 - j.ln()) / (j * j)) })
}

/// Small strain: the stress and the consistent tangent at total (mechanical) strain `strain`
/// (engineering shears), from the history `committed` (written, updated, to `updated`); with
/// `bulk` false only the deviatoric part (what a mixed element takes).
pub fn small(j2: &J2, strain: &[f64; 6], temperature: Option<f64>, committed: &[f64], updated: &mut [f64], bulk: bool) -> Result<Response> {
    let elastic: [f64; 6] = std::array::from_fn(|k| strain[k] - value(committed, k));
    let mut state = start(j2, committed, temperature);
    let mut stress = trial_deviator(j2, &elastic);
    let ret = j2.radial_return(&mut state, &mut stress, 0.0);
    if bulk {
        let p = j2.bulk * (elastic[0] + elastic[1] + elastic[2]);
        for s in stress.iter_mut().take(3) {
            *s += p;
        }
    }
    let tangent = j2.tangent(ret.as_ref(), bulk).ok_or_else(singular)?;
    if let Some(r) = ret {
        let dg = (1.5f64).sqrt() * r.plastic_strain;
        for (k, slot) in updated.iter_mut().take(6).enumerate() {
            *slot = value(committed, k) + dg * r.normal[k] * if k < 3 { 1.0 } else { 2.0 };
        }
        for (k, slot) in updated.iter_mut().skip(6).take(6).enumerate() {
            *slot = state.back[k];
        }
        if let Some(slot) = updated.get_mut(PLASTIC_STRAIN) {
            *slot = state.plastic_strain;
        }
    }
    Ok(Response { stress, tangent })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constitutive::{add_pressure, push_forward};
    use fieldcraft_model::{Elastic, Hardening, HardeningPoint, Plastic};

    const E: f64 = 200_000.0;
    const NU: f64 = 0.3;

    fn j2(plastic: Plastic) -> J2 {
        J2::from_model(&Elastic::Isotropic { young: E, poisson: NU }, Some(&plastic), None).unwrap()
    }

    fn bilinear(h: f64, hardening: Hardening) -> J2 {
        j2(Plastic::Bilinear { yield_stress: 250.0, hardening_modulus: h, hardening })
    }

    /// The whole finite-strain response, isochoric plus the Hencky volume.
    fn finite(m: &J2, f: &Mat3, committed: &[f64], updated: &mut [f64]) -> Response {
        let iso = finite_isochoric(m, f, None, committed, updated).unwrap();
        let j = crate::dense::det3(f);
        let (p, dp) = volumetric(m, j, false).unwrap();
        add_pressure(&iso, j, p, j * dp)
    }

    /// The pulled-back tangent C (dS = C : dE) against central differences of S along several
    /// directions of F, as `constitutive::tests::check_tangent` does, from fixed history.
    fn check_finite_tangent(m: &J2, f: &Mat3, committed: &[f64]) -> f64 {
        let mut scratch = vec![0.0; HISTORY];
        let r = finite(m, f, committed, &mut scratch);
        let (inv, _) = inverse3(f).unwrap();
        let t = push_forward(&inv);
        let back: Tangent =
            std::array::from_fn(|a| std::array::from_fn(|b| (0..6).map(|k| (0..6).map(|l| t[a][k] * r.tangent[k][l] * t[b][l]).sum::<f64>()).sum()));
        let scale = back.iter().flatten().fold(0.0f64, |a, v| a.max(v.abs()));
        let h = 1e-7;
        let mut worst: f64 = 0.0;
        for k in 0..9 {
            let mut df = [[0.0; 3]; 3];
            df[k / 3][k % 3] = 1.0;
            df[(k + 1) % 3][(k + 2) % 3] += 0.3;
            let at = |s: f64| {
                let fp: Mat3 = std::array::from_fn(|i| std::array::from_fn(|j| f[i][j] + s * df[i][j]));
                let mut scratch = vec![0.0; HISTORY];
                let tau = finite(m, &fp, committed, &mut scratch).stress;
                crate::rotation::rotate(&inverse3(&fp).unwrap().0, &tau)
            };
            let (sp, sm) = (at(h), at(-h));
            let ftdf = mul(&transpose(f), &df);
            let de = [ftdf[0][0], ftdf[1][1], ftdf[2][2], ftdf[0][1] + ftdf[1][0], ftdf[0][2] + ftdf[2][0], ftdf[1][2] + ftdf[2][1]];
            for a in 0..6 {
                let numeric = (sp[a] - sm[a]) / (2.0 * h);
                let analytic: f64 = (0..6).map(|b| back[a][b] * de[b]).sum();
                worst = worst.max((numeric - analytic).abs() / scale);
            }
        }
        worst
    }

    #[test]
    fn finite_strain_tangent_is_consistent_elastic_and_plastic() {
        let deformations: [Mat3; 3] = [
            [[1.02, 0.015, -0.005], [-0.01, 0.99, 0.02], [0.007, -0.012, 1.005]],
            // Large: well into plasticity.
            [[1.3, 0.15, -0.05], [-0.1, 0.85, 0.2], [0.07, -0.12, 1.05]],
            // Uniaxial (two equal principal stretches).
            [[1.2, 0.0, 0.0], [0.0, 0.92, 0.0], [0.0, 0.0, 0.92]],
        ];
        let tables = Plastic::Table { points: vec![HardeningPoint { strain: 0.0, stress: 250.0 }, HardeningPoint { strain: 0.5, stress: 600.0 }] };
        let jc = Plastic::JohnsonCook {
            a: 350.0,
            b: 275.0,
            n: 0.36,
            c: 0.0,
            m: 1.0,
            reference_strain_rate: 1.0,
            room_temperature: 293.0,
            melt_temperature: 1356.0,
            specific_heat: None,
            taylor_quinney: 0.9,
        };
        // A plastic history to start from: a prior stretch committed.
        let mut prior = vec![0.0; HISTORY];
        finite(&bilinear(2000.0, Hardening::Isotropic), &[[1.1, 0.02, 0.0], [0.0, 0.95, 0.03], [0.0, 0.0, 0.96]], &[0.0; HISTORY], &mut prior);
        assert!(prior[PLASTIC_STRAIN] > 0.0);
        for m in [bilinear(2000.0, Hardening::Isotropic), bilinear(0.0, Hardening::Isotropic), j2(tables), j2(jc)] {
            for f in &deformations {
                for committed in [&[0.0; HISTORY][..], &prior[..]] {
                    let worst = check_finite_tangent(&m, f, committed);
                    assert!(worst < 1e-7, "{:?} at {f:?}: {worst}", m.flow);
                }
            }
        }
    }

    #[test]
    fn finite_strain_tangent_is_symmetric_and_elastic_at_rest() {
        let m = bilinear(2000.0, Hardening::Isotropic);
        let mut scratch = vec![0.0; HISTORY];
        let r = finite(&m, &[[1.3, 0.15, -0.05], [-0.1, 0.85, 0.2], [0.07, -0.12, 1.05]], &[0.0; HISTORY], &mut scratch);
        for a in 0..6 {
            for b in 0..6 {
                assert!((r.tangent[a][b] - r.tangent[b][a]).abs() < 1e-9 * E, "{a} {b}");
            }
        }
        let rest = finite(&m, &IDENTITY, &[0.0; HISTORY], &mut scratch);
        let d = crate::material::isotropic(E, NU).unwrap();
        for a in 0..6 {
            for b in 0..6 {
                assert!((rest.tangent[a][b] - d[a][b]).abs() < 1e-9 * E);
            }
        }
        assert!(rest.stress.iter().all(|v| *v == 0.0));
    }

    /// Uniaxial stress along x at finite strain, from a virgin point to a logarithmic strain ε
    /// in one increment (the lateral stretch found so σ_yy = 0): the Kirchhoff stress is the
    /// small-strain curve in the logarithmic strain, τ = σ_y + E_t (ε − σ_y/E), because the
    /// exponential map integrates a proportional path exactly.
    #[test]
    fn finite_uniaxial_matches_the_logarithmic_closed_form() {
        let (y0, h) = (250.0, 2000.0);
        let m = bilinear(h, Hardening::Isotropic);
        let strain: f64 = 0.2;
        let lam = strain.exp();
        let sigma = |l: f64| {
            let f = [[lam, 0.0, 0.0], [0.0, l, 0.0], [0.0, 0.0, l]];
            let mut u = vec![0.0; HISTORY];
            let r = finite(&m, &f, &[0.0; HISTORY], &mut u);
            let j = lam * l * l;
            (r.stress[0] / j, r.stress[1] / j, u)
        };
        // Secant on the lateral stretch.
        let (mut a, mut b) = (1.0, 0.9);
        for _ in 0..60 {
            let (fa, fb) = (sigma(a).1, sigma(b).1);
            if fb.abs() < 1e-11 || fb == fa {
                break;
            }
            (a, b) = (b, b - fb * (b - a) / (fb - fa));
        }
        let (sxx, _, u) = sigma(b);
        let j = lam * b * b;
        // In Kirchhoff stress τ = J σ: τ = y0 + E_t (ε − y0/E) along the log strain path.
        let et = E * h / (E + h);
        let exact = y0 + et * (strain - y0 / E);
        assert!((sxx * j - exact).abs() < 1e-8 * exact, "{} vs {exact}", sxx * j);
        assert!((u[PLASTIC_STRAIN] - (exact - y0) / h).abs() < 1e-10);
        // The plastic deformation is isochoric: det Cₚ⁻¹ = 1.
        let cp = plastic_stretch(&u);
        assert!((crate::dense::det3(&cp) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn small_strain_tangent_is_consistent_and_cyclic_kinematic_shows_bauschinger() {
        let m = bilinear(10_000.0, Hardening::Kinematic);
        let strain = [3e-3, -1e-3, 5e-4, 2e-3, -1e-3, 7e-4];
        let mut prior = vec![0.0; HISTORY];
        small(&m, &[2e-3, -1e-3, -1e-3, 0.0, 0.0, 0.0], None, &[0.0; HISTORY], &mut prior, true).unwrap();
        for committed in [&[0.0; HISTORY][..], &prior[..]] {
            let mut scratch = vec![0.0; HISTORY];
            let r = small(&m, &strain, None, committed, &mut scratch, true).unwrap();
            let h = 1e-9;
            for b in 0..6 {
                let at = |s: f64| {
                    let mut e = strain;
                    e[b] += s;
                    small(&m, &e, None, committed, &mut [0.0; HISTORY], true).unwrap().stress
                };
                let (p, q) = (at(h), at(-h));
                for a in 0..6 {
                    let numeric = (p[a] - q[a]) / (2.0 * h);
                    assert!((numeric - r.tangent[a][b]).abs() < 1e-5 * E, "{a} {b}: {numeric} vs {}", r.tangent[a][b]);
                }
            }
        }
        // Kinematic hardening is refused at finite strain, named.
        assert!(matches!(finite_isochoric(&m, &IDENTITY, None, &[0.0; HISTORY], &mut [0.0; HISTORY]), Err(Error::NotSupported(_))));
    }

    #[test]
    fn softening_steeper_than_3g_has_no_tangent() {
        let g = E / (2.0 * (1.0 + NU));
        let points = vec![HardeningPoint { strain: 0.0, stress: 250.0 }, HardeningPoint { strain: 1e-4, stress: 250.0 - 4.0 * g * 1e-4 }];
        let m = j2(Plastic::Table { points });
        let r = small(&m, &[5e-3, -2.5e-3, -2.5e-3, 0.0, 0.0, 0.0], None, &[0.0; HISTORY], &mut [0.0; HISTORY], true);
        assert!(matches!(r, Err(Error::Degenerate(_))), "{r:?}");
    }
}
