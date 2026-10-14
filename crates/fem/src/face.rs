//! Element faces: consistent nodal forces of surface loads.
//!
//! A pressure p on a face gives node i the force fᵢ = −p ∫ Nᵢ n dA, where n dA =
//! (∂x/∂ξ × ∂x/∂η) dξ dη on the face's own, possibly curved, geometry; faces are numbered so this
//! normal points out of the element ([`fieldcraft_model::ElementKind::faces`]), and a positive
//! pressure pushes in. This is the consistent load vector of the virtual work of the surface load
//! (Zienkiewicz, Taylor & Zhu 2013, §6.6), integrated with the model's face rules (exact for a
//! uniform pressure on a flat face).

use crate::{Error, Result};

/// ∫ Nᵢ n dA for every node of a face (node coordinates in the face's node order): the model's
/// face integration ([`fieldcraft_model::nodal::face_normal_weights`]), so pressures and the
/// tractions the model distributes use the same rule.
pub fn normal_weights(xyz: &[[f64; 3]]) -> Result<Vec<[f64; 3]>> {
    fieldcraft_model::nodal::face_normal_weights(xyz).map_err(|e| Error::Invalid(e.to_string()))
}

/// Consistent nodal forces of a uniform pressure (positive pushes into the element).
pub fn pressure(xyz: &[[f64; 3]], magnitude: f64) -> Result<Vec<[f64; 3]>> {
    Ok(normal_weights(xyz)?.into_iter().map(|w| w.map(|c| -magnitude * c)).collect())
}

/// A pressure that follows the deformed face (positive pushes in): the nodal forces
/// fᵢ = −p ∫ Nᵢ (x,ξ × x,η) dξ dη on the current positions x = X + u (reference coordinates `xyz`,
/// displacements `u`, in the face's node order), and the load stiffness K = −∂f/∂u, 3n × 3n
/// row-major, from Δ(x,ξ × x,η) = [x,ξ]× Δx,η − [x,η]× Δx,ξ:
/// K_ab = p ∫ N_a (N_b,η [x,ξ]× − N_b,ξ [x,η]×) dξ dη (K. Schweizerhof, E. Ramm, *Displacement
/// dependent pressure loads in nonlinear finite element analyses*, Comput. Struct.
/// 18(6):1099–1114, 1984). K is not symmetric on its own; over a closed surface, or one whose
/// boundary is held, the assembled skew parts cancel. Same rule as [`pressure`], so at u = 0 the
/// forces are its forces.
pub fn follower_pressure(xyz: &[[f64; 3]], u: &[[f64; 3]], magnitude: f64) -> Result<(Vec<[f64; 3]>, Vec<f64>)> {
    let n = xyz.len();
    if u.len() != n {
        return Err(Error::Invalid(format!("{} displacements for a {n}-node face", u.len())));
    }
    let x: Vec<[f64; 3]> = xyz.iter().zip(u).map(|(a, b)| [a[0] + b[0], a[1] + b[1], a[2] + b[2]]).collect();
    if x.iter().flatten().any(|c| !c.is_finite()) || !magnitude.is_finite() {
        return Err(Error::Invalid("face positions and pressure must be finite".into()));
    }
    let points = fieldcraft_model::nodal::face_points(n).map_err(|e| Error::Invalid(e.to_string()))?;
    let mut f = vec![[0.0; 3]; n];
    let mut k = vec![0.0; 9 * n * n];
    for q in &points {
        let (mut tx, mut te) = ([0.0; 3], [0.0; 3]);
        for ((p, dx), de) in x.iter().zip(&q.dxi).zip(&q.deta) {
            for c in 0..3 {
                tx[c] += dx * p[c];
                te[c] += de * p[c];
            }
        }
        let normal = [tx[1] * te[2] - tx[2] * te[1], tx[2] * te[0] - tx[0] * te[2], tx[0] * te[1] - tx[1] * te[0]];
        let (sx, se) = (crate::rotation::skew(&tx), crate::rotation::skew(&te));
        for (a, na) in q.n.iter().enumerate() {
            if let Some(fa) = f.get_mut(a) {
                for c in 0..3 {
                    fa[c] -= magnitude * q.weight * na * normal[c];
                }
            }
            for (b, (db_xi, db_eta)) in q.dxi.iter().zip(&q.deta).enumerate() {
                for r in 0..3 {
                    for c in 0..3 {
                        let v = magnitude * q.weight * na * (db_eta * sx[r][c] - db_xi * se[r][c]);
                        if let Some(e) = k.get_mut((3 * a + r) * 3 * n + 3 * b + c) {
                            *e += v;
                        }
                    }
                }
            }
        }
    }
    Ok((f, k))
}

/// A pressure that follows a deformed edge of a 2D element (positive pushes into a
/// counter-clockwise element): the nodal forces (x, y) fᵢ = −p ∫ Nᵢ e (t_y, −t_x) ds on the
/// current positions, with t the tangent and e the out-of-plane extent (a thickness, or 2π r at
/// the current radius when revolved), and the load stiffness K = −∂f/∂u, 2n × 2n row-major (the
/// revolved extent depends on the radial motion too).
pub fn follower_edge_pressure(
    xyz: &[[f64; 3]],
    u: &[[f64; 2]],
    extent: fieldcraft_model::Extent,
    magnitude: f64,
) -> Result<(Vec<[f64; 2]>, Vec<f64>)> {
    let n = xyz.len();
    if u.len() != n {
        return Err(Error::Invalid(format!("{} displacements for a {n}-node edge", u.len())));
    }
    let x: Vec<[f64; 2]> = xyz.iter().zip(u).map(|(a, b)| [a[0] + b[0], a[1] + b[1]]).collect();
    if x.iter().flatten().any(|c| !c.is_finite()) || !magnitude.is_finite() {
        return Err(Error::Invalid("edge positions and pressure must be finite".into()));
    }
    let revolved = matches!(extent, fieldcraft_model::Extent::Revolved);
    let points = fieldcraft_model::nodal::edge_points(n).map_err(|e| Error::Invalid(e.to_string()))?;
    let mut f = vec![[0.0; 2]; n];
    let mut k = vec![0.0; 4 * n * n];
    let two_pi = 2.0 * std::f64::consts::PI;
    for q in &points {
        let (mut t, mut pos) = ([0.0; 2], [0.0; 2]);
        for ((p, dn), ni) in x.iter().zip(&q.dxi).zip(&q.n) {
            for c in 0..2 {
                t[c] += dn * p[c];
                pos[c] += ni * p[c];
            }
        }
        let e = extent.at([pos[0], pos[1], 0.0]);
        let w = magnitude * q.weight;
        for (a, na) in q.n.iter().enumerate() {
            if let Some(fa) = f.get_mut(a) {
                fa[0] -= w * na * e * t[1];
                fa[1] += w * na * e * t[0];
            }
            for (b, (nb, db)) in q.n.iter().zip(&q.dxi).enumerate() {
                // ∂e/∂u_b (radial) = 2π N_b when revolved.
                let de = if revolved { two_pi * nb } else { 0.0 };
                // K = −∂f/∂u.
                let entries = [(0, 0, w * na * de * t[1]), (0, 1, w * na * e * db), (1, 0, -w * na * (e * db + de * t[0])), (1, 1, 0.0)];
                for (r, c, v) in entries {
                    if let Some(x) = k.get_mut((2 * a + r) * 2 * n + 2 * b + c) {
                        *x += v;
                    }
                }
            }
        }
    }
    Ok((f, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_on_flat_faces_is_the_textbook_split() {
        // A 2 × 3 quadrilateral in z = 0, normal +z: quad4 shares p A equally, quad8 gives the
        // corners −A/12 and the mid-sides A/3.
        let quad = [[0., 0., 0.], [2., 0., 0.], [2., 3., 0.], [0., 3., 0.]];
        let f = pressure(&quad, 1.0).unwrap();
        assert!(f.iter().all(|v| (v[2] + 1.5).abs() < 1e-14 && v[0] == 0.0 && v[1] == 0.0), "{f:?}");
        let mut q8 = quad.to_vec();
        for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
            q8.push([0, 1, 2].map(|k| (quad[a][k] + quad[b][k]) / 2.0));
        }
        let f = pressure(&q8, 1.0).unwrap();
        for (i, v) in f.iter().enumerate() {
            let want = if i < 4 { 0.5 } else { -2.0 };
            assert!((v[2] - want).abs() < 1e-13, "{i}: {v:?}");
        }
        // Triangles: tri3 A/3 each; tri6 corners 0, mid-sides A/3.
        let tri = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let f = pressure(&tri, 6.0).unwrap();
        assert!(f.iter().all(|v| (v[2] + 1.0).abs() < 1e-14), "{f:?}");
        let mut t6 = tri.to_vec();
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            t6.push([0, 1, 2].map(|k| (tri[a][k] + tri[b][k]) / 2.0));
        }
        let f = pressure(&t6, 6.0).unwrap();
        for (i, v) in f.iter().enumerate() {
            let want = if i < 3 { 0.0 } else { -1.0 };
            assert!((v[2] - want).abs() < 1e-13, "{i}: {v:?}");
        }
        assert!(pressure(&tri[..2], 1.0).is_err());
    }

    /// Central differences of the follower forces against the load stiffness, on a distorted
    /// quadratic face and a revolved quadratic edge; at u = 0 the forces are the dead ones.
    #[test]
    fn follower_pressure_stiffness_is_the_derivative_of_its_forces() {
        let face = [[0., 0., 0.], [2., 0., 0.1], [2.2, 3., 0.], [0., 3., -0.2], [1., -0.1, 0.], [2.1, 1.5, 0.], [1., 3.1, 0.1], [0.1, 1.5, 0.]];
        let u: Vec<[f64; 3]> = (0..8).map(|i| [0.1 * (i as f64).sin(), 0.05 * (i as f64).cos(), 0.2 * ((i * i) as f64).sin()]).collect();
        let (f0, _) = follower_pressure(&face, &[[0.0; 3]; 8], 3.0).unwrap();
        let dead = pressure(&face, 3.0).unwrap();
        assert!(f0.iter().flatten().zip(dead.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-13), "{f0:?}");
        let (_, k) = follower_pressure(&face, &u, 3.0).unwrap();
        let h = 1e-6;
        for j in 0..24 {
            let shifted = |s: f64| {
                let mut v = u.clone();
                v[j / 3][j % 3] += s;
                follower_pressure(&face, &v, 3.0).unwrap().0
            };
            let (p, m) = (shifted(h), shifted(-h));
            for i in 0..24 {
                let numeric = -(p[i / 3][i % 3] - m[i / 3][i % 3]) / (2.0 * h);
                assert!((numeric - k[i * 24 + j]).abs() < 1e-7, "{i} {j}: {numeric} vs {}", k[i * 24 + j]);
            }
        }
        let edge = [[1.0, 0.0, 0.0], [1.5, 2.0, 0.0], [1.3, 1.0, 0.0]];
        let ue = [[0.1, 0.05], [-0.2, 0.1], [0.05, -0.03]];
        for extent in [fieldcraft_model::Extent::Revolved, fieldcraft_model::Extent::Thickness(2.0)] {
            let (f0, _) = follower_edge_pressure(&edge, &[[0.0; 2]; 3], extent, 2.0).unwrap();
            let dead = fieldcraft_model::nodal::edge_pressure(&edge, extent, 2.0).unwrap();
            assert!(f0.iter().zip(&dead).all(|(a, b)| (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12));
            let (_, k) = follower_edge_pressure(&edge, &ue, extent, 2.0).unwrap();
            for j in 0..6 {
                let shifted = |s: f64| {
                    let mut v = ue;
                    v[j / 2][j % 2] += s;
                    follower_edge_pressure(&edge, &v, extent, 2.0).unwrap().0
                };
                let (p, m) = (shifted(h), shifted(-h));
                for i in 0..6 {
                    let numeric = -(p[i / 2][i % 2] - m[i / 2][i % 2]) / (2.0 * h);
                    assert!((numeric - k[i * 6 + j]).abs() < 1e-7, "{extent:?} {i} {j}: {numeric} vs {}", k[i * 6 + j]);
                }
            }
        }
    }

    #[test]
    fn pressure_on_a_curved_face_sums_to_the_projected_area() {
        // A quadratic face on a quarter cylinder r = 2, z in 0..1, normal outward: the total
        // pressure force is −p times the projected areas (2 in x and 2 in y).
        let r: f64 = 2.0;
        let at = |t: f64, z: f64| [r * t.cos(), r * t.sin(), z];
        let h = std::f64::consts::FRAC_PI_2;
        // Corners counter-clockwise seen from outside: (0,0), (0,1)... with the normal outward.
        let xyz = vec![at(0.0, 0.0), at(h, 0.0), at(h, 1.0), at(0.0, 1.0), at(h / 2.0, 0.0), at(h, 0.5), at(h / 2.0, 1.0), at(0.0, 0.5)];
        let w = normal_weights(&xyz).unwrap();
        let total = w.iter().fold([0.0; 3], |a, v| [a[0] + v[0], a[1] + v[1], a[2] + v[2]]);
        // A quadratic arc interpolates the circle closely; the projected area is exact for the
        // interpolated surface (its end points are exact).
        assert!((total[0].abs() - 2.0).abs() < 1e-12 && (total[1].abs() - 2.0).abs() < 1e-12 && total[2].abs() < 1e-12, "{total:?}");
        assert!(total[0] > 0.0 && total[1] > 0.0, "this ordering points outwards: {total:?}");
    }
}
