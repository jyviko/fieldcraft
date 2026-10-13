//! Numerical integration rules on the reference elements.
//!
//! - Gauss–Legendre rules on [−1, 1] with 1 to 4 points (exact for polynomials of degree
//!   2n − 1), and their tensor products on the reference hexahedron [−1, 1]³.
//! - On the reference tetrahedron {ξ, η, ζ ≥ 0, ξ + η + ζ ≤ 1} (volume 1/6): the centroid rule
//!   (degree 1), the symmetric four-point rule (degree 2), and the conical product rule of Stroud
//!   (A. H. Stroud, *Approximate Calculation of Multiple Integrals*, Prentice-Hall 1971, §2.7):
//!   the collapsed map ξ = u, η = v(1 − u), ζ = w(1 − u)(1 − v) from the unit cube, with
//!   Jacobian (1 − u)²(1 − v), integrated by Gauss–Legendre in u, v and w. With n points per
//!   direction it is exact for polynomials of degree 2n − 3 on the tetrahedron, with positive
//!   weights only.
//! - On the reference triangle {ξ, η ≥ 0, ξ + η ≤ 1}: the same collapsed product in two
//!   dimensions (Jacobian 1 − u), exact for degree 2n − 2.
//! - On the reference square [−1, 1]²: tensor Gauss–Legendre.

/// A quadrature point: natural coordinates and weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub xi: [f64; 3],
    pub weight: f64,
}

/// Gauss–Legendre abscissae and weights on [−1, 1], `n` in 1..=4 (clamped).
pub fn gauss_legendre(n: usize) -> &'static [(f64, f64)] {
    const G1: [(f64, f64); 1] = [(0.0, 2.0)];
    const G2: [(f64, f64); 2] = [(-0.577_350_269_189_625_8, 1.0), (0.577_350_269_189_625_8, 1.0)];
    const G3: [(f64, f64); 3] = [(-0.774_596_669_241_483_4, 5.0 / 9.0), (0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)];
    const G4: [(f64, f64); 4] = [
        (-0.861_136_311_594_052_6, 0.347_854_845_137_453_9),
        (-0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
        (0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
        (0.861_136_311_594_052_6, 0.347_854_845_137_453_9),
    ];
    match n {
        0 | 1 => &G1,
        2 => &G2,
        3 => &G3,
        _ => &G4,
    }
}

/// An integration rule on a reference element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rule {
    /// n × n × n Gauss–Legendre on the reference hexahedron.
    Hex(u8),
    /// The tetrahedron's centroid, weight 1/6 (degree 1).
    TetCentroid,
    /// Four symmetric points on the tetrahedron, weight 1/24 each (degree 2).
    Tet4,
    /// Stroud's conical product on the tetrahedron, n points per direction.
    TetConical(u8),
    /// n × n Gauss–Legendre on the reference square (z = 0).
    Quad(u8),
    /// The collapsed product on the reference triangle, n points per direction (z = 0).
    TriConical(u8),
}

impl Rule {
    pub fn points(self) -> Vec<Point> {
        match self {
            Rule::Hex(n) => {
                let g = gauss_legendre(usize::from(n));
                let mut out = Vec::with_capacity(g.len().pow(3));
                // ξ fastest, then η, then ζ: the corner order of the reference hexahedron for 2 × 2 × 2.
                for &(z, wz) in g {
                    for &(y, wy) in g {
                        for &(x, wx) in g {
                            out.push(Point { xi: [x, y, z], weight: wx * wy * wz });
                        }
                    }
                }
                out
            }
            Rule::TetCentroid => vec![Point { xi: [0.25; 3], weight: 1.0 / 6.0 }],
            Rule::Tet4 => {
                // (5 ∓ √5)/20: the barycentric coordinates of the degree-2 rule.
                let a = 0.585_410_196_624_968_5;
                let b = 0.138_196_601_125_010_5;
                // Point k sits near corner k (barycentric a there, b elsewhere).
                [[b, b, b], [a, b, b], [b, a, b], [b, b, a]].into_iter().map(|xi| Point { xi, weight: 1.0 / 24.0 }).collect()
            }
            Rule::TetConical(n) => {
                let g: Vec<(f64, f64)> = gauss_legendre(usize::from(n)).iter().map(|&(x, w)| ((x + 1.0) / 2.0, w / 2.0)).collect();
                let mut out = Vec::with_capacity(g.len().pow(3));
                for &(u, wu) in &g {
                    for &(v, wv) in &g {
                        for &(w, ww) in &g {
                            let xi = [u, v * (1.0 - u), w * (1.0 - u) * (1.0 - v)];
                            out.push(Point { xi, weight: wu * wv * ww * (1.0 - u) * (1.0 - u) * (1.0 - v) });
                        }
                    }
                }
                out
            }
            Rule::Quad(n) => {
                let g = gauss_legendre(usize::from(n));
                g.iter().flat_map(|&(y, wy)| g.iter().map(move |&(x, wx)| Point { xi: [x, y, 0.0], weight: wx * wy })).collect()
            }
            Rule::TriConical(n) => {
                let g: Vec<(f64, f64)> = gauss_legendre(usize::from(n)).iter().map(|&(x, w)| ((x + 1.0) / 2.0, w / 2.0)).collect();
                g.iter()
                    .flat_map(|&(u, wu)| g.iter().map(move |&(v, wv)| Point { xi: [u, v * (1.0 - u), 0.0], weight: wu * wv * (1.0 - u) }))
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ∫ ξ^a η^b ζ^c over the reference tetrahedron = a! b! c! / (a + b + c + 3)!.
    fn tet_monomial(a: u32, b: u32, c: u32) -> f64 {
        let f = |n: u32| (1..=n).map(f64::from).product::<f64>();
        f(a) * f(b) * f(c) / f(a + b + c + 3)
    }

    fn integrate(rule: Rule, a: i32, b: i32, c: i32) -> f64 {
        rule.points().iter().map(|p| p.weight * p.xi[0].powi(a) * p.xi[1].powi(b) * p.xi[2].powi(c)).sum()
    }

    #[test]
    fn tetrahedron_rules_are_exact_to_their_degree() {
        for (rule, degree) in [(Rule::TetCentroid, 1), (Rule::Tet4, 2), (Rule::TetConical(3), 3), (Rule::TetConical(4), 5)] {
            for a in 0..=degree {
                for b in 0..=degree - a {
                    for c in 0..=degree - a - b {
                        let exact = tet_monomial(a, b, c);
                        let got = integrate(rule, a as i32, b as i32, c as i32);
                        assert!((got - exact).abs() < 1e-14, "{rule:?} ξ^{a} η^{b} ζ^{c}: {got} vs {exact}");
                    }
                }
            }
        }
    }

    #[test]
    fn hexahedron_and_face_rules() {
        for n in 1..=4u8 {
            let d = 2 * i32::from(n) - 1;
            for a in 0..=d {
                // ∫_{-1}^{1} x^a dx.
                let exact_1d = if a % 2 == 1 { 0.0 } else { 2.0 / f64::from(a + 1) };
                let got = integrate(Rule::Hex(n), a, 0, 0);
                assert!((got - exact_1d * 4.0).abs() < 1e-14, "hex {n} x^{a}");
                let got = integrate(Rule::Quad(n), 0, a, 0);
                assert!((got - exact_1d * 2.0).abs() < 1e-14, "quad {n} y^{a}");
            }
        }
        // Triangle: ∫ ξ^a η^b = a! b! / (a + b + 2)!.
        let f = |n: i32| (1..=n).map(f64::from).product::<f64>();
        for a in 0..=4 {
            for b in 0..=4 - a {
                let got = integrate(Rule::TriConical(3), a, b, 0);
                assert!((got - f(a) * f(b) / f(a + b + 2)).abs() < 1e-14, "tri ξ^{a} η^{b}");
            }
        }
    }
}
