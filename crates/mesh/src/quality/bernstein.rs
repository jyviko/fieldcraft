//! Bernstein–Bézier polynomials on the reference domains of solid elements.
//!
//! A reference domain is a product of simplices: three segments for a hexahedron, a triangle
//! times a segment for a wedge, one tetrahedron for a tetrahedron. On a simplex of dimension `d`
//! a polynomial of degree `n` is written in the Bernstein basis
//! `B_α(λ) = n! / (α_0! … α_d!) · λ_0^α_0 … λ_d^α_d` (`|α| = n`, `λ` barycentric coordinates);
//! on a product the basis is the product of each factor's basis.
//!
//! Three properties make the basis the right tool for certifying an element's Jacobian
//! (Johnen, Remacle, Geuzaine, *Geometrical validity of curvilinear finite elements*,
//! J. Comput. Phys. 233:359–372, 2013):
//! 1. The basis is a partition of unity with non-negative terms, so the polynomial lies between
//!    its smallest and its largest coefficient everywhere on the domain.
//! 2. A coefficient whose multi-index sits at a vertex of the domain is the polynomial's value
//!    there.
//! 3. Subdividing the domain (de Casteljau) gives the coefficients on each piece, and they converge
//!    to the values as the pieces shrink.
//!
//! So a positive smallest coefficient proves positivity, a non-positive vertex coefficient proves
//! the opposite, and subdivision settles the cases in between. The product rule
//! `B^m_α · B^n_β = C(m;α) C(n;β) / C(m+n;α+β) · B^{m+n}_{α+β}` (multinomials `C`) and the
//! derivative rule `∂p/∂λ_k − ∂p/∂λ_0 = n Σ_β (c_{β+e_k} − c_{β+e_0}) B^{n−1}_β` are the standard
//! ones (Farin, *Curves and Surfaces for CAGD*, 5th ed., ch. 17–18).

use crate::{Error, Result};

/// Highest degree per factor. The Jacobian determinant of a 20-node hexahedron has degree 5.
pub(crate) const MAX_DEGREE: usize = 8;

/// Free barycentric components of a multi-index: three for every 3-D reference domain (the
/// component `α_0` of each factor is implied by its degree).
const FREE: usize = 3;
type Multi = [usize; FREE];

/// One simplex factor of a reference domain (`dim` 1 is a segment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Factor {
    pub dim: usize,
    pub deg: usize,
}

/// A polynomial in Bernstein form on a product of simplices.
///
/// Coefficients are stored densely over `(deg + 1)^dim` slots per factor; slots whose free
/// components exceed the factor's degree are unused and stay zero.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bernstein {
    factors: Vec<Factor>,
    coef: Vec<f64>,
}

fn factorial(n: usize) -> f64 {
    (2..=n).map(|k| k as f64).product()
}

impl Bernstein {
    pub fn zero(factors: &[Factor]) -> Result<Self> {
        let free: usize = factors.iter().map(|f| f.dim).sum();
        if free > FREE || factors.iter().any(|f| f.dim == 0 || f.deg > MAX_DEGREE) {
            return Err(Error::Invalid(format!("unsupported Bernstein domain {factors:?}")));
        }
        let len = factors.iter().map(|f| (f.deg + 1).pow(f.dim as u32)).product();
        Ok(Self { factors: factors.to_vec(), coef: vec![0.0; len] })
    }

    pub fn factors(&self) -> &[Factor] {
        &self.factors
    }

    /// Position of factor `f`'s first free component in a multi-index.
    fn offset(&self, f: usize) -> usize {
        self.factors.iter().take(f).map(|x| x.dim).sum()
    }

    fn slot(&self, m: &Multi) -> usize {
        let mut s = 0;
        let mut p = 0;
        for f in &self.factors {
            for _ in 0..f.dim {
                s = s * (f.deg + 1) + m.get(p).copied().unwrap_or(0);
                p += 1;
            }
        }
        s
    }

    fn valid(&self, m: &Multi) -> bool {
        let mut p = 0;
        self.factors.iter().all(|f| {
            let sum: usize = m.iter().skip(p).take(f.dim).sum();
            p += f.dim;
            sum <= f.deg
        })
    }

    /// Every valid multi-index, in storage order.
    fn multis(&self) -> Vec<Multi> {
        let mut out = vec![[0; FREE]];
        let mut p = 0;
        for f in &self.factors {
            for _ in 0..f.dim {
                out = out
                    .into_iter()
                    .flat_map(|m| {
                        (0..=f.deg).map(move |c| {
                            let mut m = m;
                            if let Some(x) = m.get_mut(p) {
                                *x = c;
                            }
                            m
                        })
                    })
                    .collect();
                p += 1;
            }
        }
        out.retain(|m| self.valid(m));
        out
    }

    fn get(&self, m: &Multi) -> f64 {
        self.coef.get(self.slot(m)).copied().unwrap_or(0.0)
    }

    fn add_at(&mut self, m: &Multi, v: f64) {
        let s = self.slot(m);
        if let Some(c) = self.coef.get_mut(s) {
            *c += v;
        }
    }

    /// Set the coefficient of `m` (free components, factor after factor).
    pub fn set(&mut self, m: Multi, v: f64) {
        let s = self.slot(&m);
        if self.valid(&m)
            && let Some(c) = self.coef.get_mut(s)
        {
            *c = v;
        }
    }

    /// Barycentric component `c` (0..=dim) of factor `f` in `m`.
    fn component(&self, m: &Multi, f: usize, c: usize) -> usize {
        let (Some(fac), off) = (self.factors.get(f), self.offset(f)) else { return 0 };
        if c == 0 { fac.deg.saturating_sub(m.iter().skip(off).take(fac.dim).sum()) } else { m.get(off + c - 1).copied().unwrap_or(0) }
    }

    /// The multinomial weight `n! / α!` of `m`, over all factors.
    fn weight(&self, m: &Multi) -> f64 {
        let mut w = 1.0;
        for (f, fac) in self.factors.iter().enumerate() {
            w *= factorial(fac.deg);
            for c in 0..=fac.dim {
                w /= factorial(self.component(m, f, c));
            }
        }
        w
    }

    fn same_domain(&self, o: &Self) -> bool {
        self.factors.len() == o.factors.len() && self.factors.iter().zip(&o.factors).all(|(a, b)| a.dim == b.dim)
    }

    /// The multinomial weight of every slot (zero for unused slots).
    fn weights(&self) -> Vec<f64> {
        let mut w = vec![0.0; self.coef.len()];
        for m in self.multis() {
            let s = self.slot(&m);
            if let Some(x) = w.get_mut(s) {
                *x = self.weight(&m);
            }
        }
        w
    }

    /// The product of two polynomials on the same domain.
    pub fn mul(&self, o: &Self) -> Result<Self> {
        if !self.same_domain(o) {
            return Err(Error::Invalid("Bernstein product of different domains".into()));
        }
        let factors: Vec<Factor> = self.factors.iter().zip(&o.factors).map(|(a, b)| Factor { dim: a.dim, deg: a.deg + b.deg }).collect();
        let mut out = Self::zero(&factors)?;
        let out_w = out.weights();
        let terms =
            |p: &Self| -> Vec<(Multi, f64)> { p.multis().into_iter().map(|m| (m, p.get(&m) * p.weight(&m))).filter(|(_, c)| *c != 0.0).collect() };
        let right = terms(o);
        for (a, ca) in terms(self) {
            for (b, cb) in &right {
                let mut k = a;
                for (x, y) in k.iter_mut().zip(b) {
                    *x += y;
                }
                let s = out.slot(&k);
                if let (Some(c), Some(w)) = (out.coef.get_mut(s), out_w.get(s)) {
                    *c += ca * cb / w;
                }
            }
        }
        Ok(out)
    }

    /// `self + s · o`, both of the same domain and degrees.
    pub fn axpy(&self, s: f64, o: &Self) -> Result<Self> {
        if self.factors != o.factors {
            return Err(Error::Invalid("Bernstein sum of different spaces".into()));
        }
        let coef = self.coef.iter().zip(&o.coef).map(|(a, b)| a + s * b).collect();
        Ok(Self { factors: self.factors.clone(), coef })
    }

    /// The derivative along reference direction `k` (1..=dim) of factor `f`: the direction from
    /// that factor's vertex 0 to its vertex `k`, with the other coordinates held fixed.
    pub fn derivative(&self, f: usize, k: usize) -> Result<Self> {
        let fac = self.factors.get(f).copied().ok_or_else(|| Error::Invalid(format!("no factor {f}")))?;
        if k == 0 || k > fac.dim || fac.deg == 0 {
            return Err(Error::Invalid(format!("no derivative {k} of factor {fac:?}")));
        }
        let mut factors = self.factors.clone();
        if let Some(x) = factors.get_mut(f) {
            x.deg -= 1;
        }
        let mut out = Self::zero(&factors)?;
        let pos = self.offset(f) + k - 1;
        for b in out.multis() {
            let mut hi = b;
            if let Some(x) = hi.get_mut(pos) {
                *x += 1;
            }
            let v = fac.deg as f64 * (self.get(&hi) - self.get(&b));
            out.add_at(&b, v);
        }
        Ok(out)
    }

    /// Split factor `f` at the midpoint of its edge `(i, j)` (vertex numbers 0..=dim) with de
    /// Casteljau's algorithm. The first piece keeps vertex `i` (vertex `j` becomes the midpoint),
    /// the second keeps vertex `j` (vertex `i` becomes the midpoint).
    pub fn bisect(&self, f: usize, i: usize, j: usize) -> Result<(Self, Self)> {
        let fac = self.factors.get(f).copied().ok_or_else(|| Error::Invalid(format!("no factor {f}")))?;
        if i == j || i > fac.dim || j > fac.dim {
            return Err(Error::Invalid(format!("no edge ({i}, {j}) in {fac:?}")));
        }
        let off = self.offset(f);
        let shift = |start: &Multi, r: usize| {
            let mut m = *start;
            if i > 0
                && let Some(x) = m.get_mut(off + i - 1)
            {
                *x -= r;
            }
            if j > 0
                && let Some(x) = m.get_mut(off + j - 1)
            {
                *x += r;
            }
            m
        };
        let (mut a, mut b) = (self.clone(), self.clone());
        // Each line starts where component j is zero; walking it moves units from i to j.
        for start in self.multis() {
            if self.component(&start, f, j) != 0 {
                continue;
            }
            let s = self.component(&start, f, i);
            let line: Vec<Multi> = (0..=s).map(|r| shift(&start, r)).collect();
            let mut tri: Vec<f64> = line.iter().map(|m| self.get(m)).collect();
            let n = tri.len();
            let mut left = vec![0.0; n];
            let mut right = vec![0.0; n];
            for level in 0..n {
                let last = n - 1 - level;
                if let (Some(l), Some(&first)) = (left.get_mut(level), tri.first()) {
                    *l = first;
                }
                if let (Some(r), Some(&end)) = (right.get_mut(last), tri.get(last)) {
                    *r = end;
                }
                for r in 0..last {
                    if let (Some(&x), Some(&y)) = (tri.get(r), tri.get(r + 1))
                        && let Some(t) = tri.get_mut(r)
                    {
                        *t = 0.5 * (x + y);
                    }
                }
            }
            for ((m, l), r) in line.iter().zip(&left).zip(&right) {
                let s = self.slot(m);
                if let Some(c) = a.coef.get_mut(s) {
                    *c = *l;
                }
                if let Some(c) = b.coef.get_mut(s) {
                    *c = *r;
                }
            }
        }
        Ok((a, b))
    }

    /// The coefficients of the used slots, in a fixed order shared by every polynomial on the
    /// same domain (so the coefficients of a vector-valued polynomial's components line up).
    pub fn coefficients(&self) -> Vec<f64> {
        self.multis().iter().map(|m| self.get(m)).collect()
    }

    /// The smallest and largest coefficient: bounds of the polynomial on its domain.
    pub fn bounds(&self) -> (f64, f64) {
        self.multis().iter().map(|m| self.get(m)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| (lo.min(c), hi.max(c)))
    }

    /// The value at a vertex of the domain: one vertex number (0..=dim) per factor.
    pub fn at_vertex(&self, verts: &[usize]) -> f64 {
        let mut m = [0; FREE];
        for (f, (fac, &v)) in self.factors.iter().zip(verts).enumerate() {
            if v > 0
                && v <= fac.dim
                && let Some(x) = m.get_mut(self.offset(f) + v - 1)
            {
                *x = fac.deg;
            }
        }
        self.get(&m)
    }

    /// Every vertex of the domain, as one vertex number per factor.
    pub fn vertices(&self) -> Vec<Vec<usize>> {
        self.factors.iter().fold(vec![Vec::new()], |acc, f| {
            acc.into_iter()
                .flat_map(|v| {
                    (0..=f.dim).map(move |k| {
                        let mut v = v.clone();
                        v.push(k);
                        v
                    })
                })
                .collect()
        })
    }

    /// The values at every vertex of the domain.
    pub fn vertex_values(&self) -> Vec<f64> {
        self.vertices().iter().map(|v| self.at_vertex(v)).collect()
    }

    /// The integral over the reference domain (unit segment, unit right triangle and unit right
    /// tetrahedron factors). Every basis function of a factor has the same integral,
    /// `1 / (dim! · C(deg + dim, dim))`.
    pub fn integral(&self) -> f64 {
        let per: f64 = self.factors.iter().map(|f| factorial(f.deg) / factorial(f.deg + f.dim)).product();
        self.multis().iter().map(|m| self.get(m)).sum::<f64>() * per
    }

    /// The value at a point given by barycentric coordinates per factor (`dim + 1` each).
    pub fn eval(&self, bary: &[&[f64]]) -> f64 {
        self.multis()
            .iter()
            .map(|m| {
                let mut t = self.get(m) * self.weight(m);
                for (f, fac) in self.factors.iter().enumerate() {
                    for c in 0..=fac.dim {
                        let l = bary.get(f).and_then(|b| b.get(c)).copied().unwrap_or(0.0);
                        t *= l.powi(self.component(m, f, c) as i32);
                    }
                }
                t
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(deg: usize) -> Factor {
        Factor { dim: 1, deg }
    }

    /// A pseudo-random polynomial (deterministic).
    fn sample(factors: &[Factor], seed: u64) -> Bernstein {
        let mut p = Bernstein::zero(factors).unwrap();
        let mut s = seed;
        for m in p.multis() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            p.set(m, ((s >> 33) as f64 / (1u64 << 31) as f64) - 0.5);
        }
        p
    }

    fn points(dim: usize) -> Vec<Vec<f64>> {
        match dim {
            1 => vec![vec![0.7, 0.3], vec![0.1, 0.9], vec![1.0, 0.0]],
            2 => vec![vec![0.2, 0.3, 0.5], vec![0.6, 0.1, 0.3], vec![0.0, 0.0, 1.0]],
            _ => vec![vec![0.1, 0.2, 0.3, 0.4], vec![0.25; 4], vec![0.0, 1.0, 0.0, 0.0]],
        }
    }

    fn domains() -> Vec<Vec<Factor>> {
        vec![vec![seg(2), seg(1), seg(3)], vec![Factor { dim: 2, deg: 2 }, seg(1)], vec![Factor { dim: 3, deg: 2 }]]
    }

    /// Every combination of sample points, one per factor.
    fn grid(factors: &[Factor]) -> Vec<Vec<Vec<f64>>> {
        factors.iter().fold(vec![Vec::new()], |acc, f| {
            acc.into_iter().flat_map(|v| points(f.dim).into_iter().map(move |p| [v.clone(), vec![p]].concat())).collect()
        })
    }

    fn eval(p: &Bernstein, x: &[Vec<f64>]) -> f64 {
        let refs: Vec<&[f64]> = x.iter().map(|v| v.as_slice()).collect();
        p.eval(&refs)
    }

    #[test]
    fn product_matches_pointwise_product() {
        for d in domains() {
            let a = sample(&d, 1);
            let b = sample(&d.iter().map(|f| Factor { dim: f.dim, deg: 1 }).collect::<Vec<_>>(), 2);
            let ab = a.mul(&b).unwrap();
            for x in grid(&d) {
                assert!((eval(&ab, &x) - eval(&a, &x) * eval(&b, &x)).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn derivative_matches_finite_difference() {
        for d in domains() {
            let p = sample(&d, 3);
            for (f, fac) in d.iter().enumerate() {
                for k in 1..=fac.dim {
                    let dp = p.derivative(f, k).unwrap();
                    for x in grid(&d) {
                        // Move along direction k: λ_k += h, λ_0 -= h.
                        let h = 1e-6;
                        let mut plus = x.clone();
                        let mut minus = x.clone();
                        plus[f][k] += h;
                        plus[f][0] -= h;
                        minus[f][k] -= h;
                        minus[f][0] += h;
                        let fd = (eval(&p, &plus) - eval(&p, &minus)) / (2.0 * h);
                        assert!((eval(&dp, &x) - fd).abs() < 1e-6, "{fd} vs {}", eval(&dp, &x));
                    }
                }
            }
        }
    }

    #[test]
    fn bisection_reproduces_the_polynomial() {
        for d in domains() {
            let p = sample(&d, 4);
            for (f, fac) in d.iter().enumerate() {
                for (i, j) in [(0, 1), (1, 0), (fac.dim, 0)] {
                    if i == j {
                        continue;
                    }
                    let (a, b) = p.bisect(f, i, j).unwrap();
                    for x in grid(&d) {
                        // Piece a: vertex j is the midpoint, so λ_i += λ_j / 2, λ_j /= 2.
                        let mut xa = x.clone();
                        let lj = xa[f][j];
                        xa[f][i] += lj / 2.0;
                        xa[f][j] = lj / 2.0;
                        assert!((eval(&a, &x) - eval(&p, &xa)).abs() < 1e-12);
                        let mut xb = x.clone();
                        let li = xb[f][i];
                        xb[f][j] += li / 2.0;
                        xb[f][i] = li / 2.0;
                        assert!((eval(&b, &x) - eval(&p, &xb)).abs() < 1e-12);
                    }
                }
            }
        }
    }

    #[test]
    fn vertices_bounds_and_integral() {
        for d in domains() {
            let p = sample(&d, 5);
            let (lo, hi) = p.bounds();
            for v in p.vertices() {
                let x: Vec<Vec<f64>> = d.iter().zip(&v).map(|(f, &k)| (0..=f.dim).map(|c| if c == k { 1.0 } else { 0.0 }).collect()).collect();
                let value = eval(&p, &x);
                assert!((p.at_vertex(&v) - value).abs() < 1e-12);
                assert!(lo <= value && value <= hi);
            }
        }
        // ∫ over the unit tetrahedron of 1 is 1/6; over the triangle × segment of 1 is 1/2.
        let mut one = Bernstein::zero(&[Factor { dim: 3, deg: 2 }]).unwrap();
        for m in one.multis() {
            one.set(m, 1.0);
        }
        assert!((one.integral() - 1.0 / 6.0).abs() < 1e-15);
        let mut one = Bernstein::zero(&[Factor { dim: 2, deg: 1 }, seg(2)]).unwrap();
        for m in one.multis() {
            one.set(m, 1.0);
        }
        assert!((one.integral() - 0.5).abs() < 1e-15);
    }

    #[test]
    fn rejects_unsupported_domains() {
        assert!(Bernstein::zero(&[Factor { dim: 3, deg: 1 }, seg(1)]).is_err());
        assert!(Bernstein::zero(&[seg(MAX_DEGREE + 1)]).is_err());
        let p = Bernstein::zero(&[Factor { dim: 3, deg: 1 }]).unwrap();
        assert!(p.derivative(0, 4).is_err());
        assert!(p.bisect(0, 2, 2).is_err());
        assert!(p.mul(&Bernstein::zero(&[seg(1), seg(1), seg(1)]).unwrap()).is_err());
    }
}
