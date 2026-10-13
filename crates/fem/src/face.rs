//! Element faces: consistent nodal forces of surface loads.
//!
//! A pressure p on a face gives node i the force fᵢ = −p ∫ Nᵢ n dA, where n dA =
//! (∂x/∂ξ × ∂x/∂η) dξ dη on the face's own, possibly curved, geometry; faces are numbered so this
//! normal points out of the element ([`modelcraft_model::ElementKind::faces`]), and a positive
//! pressure pushes in. This is the consistent load vector of the virtual work of the surface load
//! (Zienkiewicz, Taylor & Zhu 2013, §6.6), exact for a uniform pressure on a flat or quadratic
//! face with the rules used here.

use crate::quadrature::Rule;
use crate::shape;
use crate::{Error, Result};

fn rule(nodes: usize) -> Rule {
    if nodes == 3 || nodes == 6 { Rule::TriConical(3) } else { Rule::Quad(3) }
}

/// ∫ Nᵢ n dA for every node of a face (node coordinates in the face's node order).
pub fn normal_weights(xyz: &[[f64; 3]]) -> Result<Vec<[f64; 3]>> {
    if xyz.iter().flatten().any(|c| !c.is_finite()) {
        return Err(Error::Invalid("face coordinates must be finite".into()));
    }
    let mut out = vec![[0.0; 3]; xyz.len()];
    for p in rule(xyz.len()).points() {
        let s =
            shape::face(xyz.len(), p.xi[0], p.xi[1]).ok_or_else(|| Error::Invalid(format!("a face has 3, 4, 6 or 8 nodes, not {}", xyz.len())))?;
        let mut tx = [0.0; 3];
        let mut te = [0.0; 3];
        for (d, x) in s.derivatives().iter().zip(xyz) {
            for k in 0..3 {
                tx[k] += d[0] * x[k];
                te[k] += d[1] * x[k];
            }
        }
        let n = [tx[1] * te[2] - tx[2] * te[1], tx[2] * te[0] - tx[0] * te[2], tx[0] * te[1] - tx[1] * te[0]];
        for (o, ni) in out.iter_mut().zip(s.values()) {
            for k in 0..3 {
                o[k] += ni * n[k] * p.weight;
            }
        }
    }
    Ok(out)
}

/// Consistent nodal forces of a uniform pressure (positive pushes into the element).
pub fn pressure(xyz: &[[f64; 3]], magnitude: f64) -> Result<Vec<[f64; 3]>> {
    Ok(normal_weights(xyz)?.into_iter().map(|w| w.map(|c| -magnitude * c)).collect())
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
