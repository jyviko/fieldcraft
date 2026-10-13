//! Couplings of reference points to the mesh, and the elimination of every linear constraint
//! (couplings and prescribed values) from the system.
//!
//! A coupled reference point is a node with six degrees of freedom (three translations, three
//! rotations) after the mesh's translational ones. Each coupling becomes linear constraint rows
//! Σ cⱼ uⱼ = 0:
//!
//! - **Kinematic** (a rigid link): every node s of the set follows the reference point r rigidly
//!   in the coupled translations, u_s = u_r + θ_r × (x_s − x_r) (small rotations; R. D. Cook,
//!   D. S. Malkus, M. E. Plesha, R. J. Witt, *Concepts and Applications of Finite Element
//!   Analysis*, 4th ed., Wiley 2002, §9.2). The reference point's rotations take part through the
//!   lever arms, so a moment on it reaches the face as a couple even when only translations are
//!   coupled; coupled rotations have nothing to act on at solid nodes, which have none.
//! - **Distributing** (an interpolation constraint): the reference point moves with the rigid
//!   motion that fits the set's nodal translations best in the least-squares sense, with equal
//!   weights (as the model and the deck writers define it). About the nodes' centroid c, with
//!   rᵢ = xᵢ − c, the fit is u_c = Σ uᵢ / N and θ = J⁻¹ Σ rᵢ × uᵢ with
//!   J = Σ (|rᵢ|² I − rᵢ rᵢᵀ); the reference point's motion is θ_r = θ and
//!   u_r = u_c + θ × (x_r − c). Only the coupled degrees of freedom are tied. By virtual work a
//!   force or moment on the point is spread over the nodes as the forces whose resultant and
//!   moment about the point equal it, weighted like the fit, and the region stays free to deform
//!   (G. H. K. Heirman, W. Desmet, *Interface reduction of flexible bodies for efficient modeling
//!   of body flexibility in multibody dynamics*, Multibody Syst. Dyn. 24(2):219–234, 2010, on
//!   interpolation constraints). Nodes on one line cannot carry a rotation about that line: that
//!   is an error when rotations are coupled, and their mean motion is used otherwise.
//!
//! **Elimination** (master–slave by transformation, Cook et al. 2002 §9.2; C. A. Felippa,
//! *Introduction to Finite Element Methods*, ch. 8, University of Colorado lecture notes): every
//! degree of freedom is free, prescribed (a boundary condition), dependent on others, or inactive
//! (a reference point direction no coupling ties). Rows are taken one at a time in a fixed order
//! (boundary conditions first, then couplings by name, nodes by label, directions in order). A
//! row is first rewritten in terms of the current free and prescribed degrees of freedom; one
//! free degree of freedom is then chosen to depend on the rest: the row's intended one (the
//! coupled node, or the distributing coupling's reference point) while it is free, otherwise the
//! largest coefficient, lowest index first (the automatic choice of J. F. Abel, M. S. Shephard,
//! *An algorithm for multipoint constraints in finite element analysis*, IJNME 14(3):464–467,
//! 1979). A row with no free degree of freedom left is redundant when the prescribed values
//! satisfy it and contradictory otherwise. Every dependent degree of freedom ends up as a
//! combination of free and prescribed ones, u = T_f q + T_p u_p, so the reduced system is
//! T_fᵀ K T_f q = T_fᵀ (f − K T_p u_p) and the reactions at prescribed degrees of freedom are
//! T_pᵀ (K u − f): no penalty numbers and no Lagrange multipliers, so the matrix stays symmetric
//! positive definite for the Cholesky factorization. With no coupling, T is the identity on the
//! free degrees of freedom and the result is the plain elimination of prescribed values, bit for
//! bit.

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_fem::dense::inverse3;
use modelcraft_model::{CouplingKind, Model, Resolved};

use crate::sparse::Csr;
use crate::system::Mesh;
use crate::{Error, Result};

/// One linear constraint Σ cⱼ uⱼ = 0 and the degree of freedom meant to be eliminated with it.
#[derive(Debug, Clone)]
pub struct Row {
    pub terms: Vec<(usize, f64)>,
    pub pivot: usize,
    /// The coupling it comes from (for messages).
    pub origin: String,
}

/// A coefficient below this fraction of a row's largest one is round-off, not a constraint.
const NEGLIGIBLE: f64 = 1e-10;

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// [a]×, the matrix of a × ·.
fn cross_matrix(a: [f64; 3]) -> [[f64; 3]; 3] {
    [[0.0, -a[2], a[1]], [a[2], 0.0, -a[0]], [-a[1], a[0], 0.0]]
}

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut c = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            c[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    c
}

/// The constraint rows of every coupling: couplings by name, nodes by label, directions in order.
pub fn coupling_rows(model: &Model, resolved: &Resolved, mesh: &Mesh) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for (name, c) in model.couplings() {
        let point = mesh.point(&c.reference_point).ok_or_else(|| Error::Internal(format!("reference point `{}`", c.reference_point)))?;
        let xr = mesh.points.get(point).map(|p| p.xyz).ok_or_else(|| Error::Internal("reference point index".into()))?;
        let rp = mesh.point_dof(point, 0);
        let set = resolved.sets.get(&c.set).ok_or_else(|| Error::Invalid(format!("coupling `{name}`: set `{}` is not resolved", c.set)))?;
        if set.nodes.is_empty() {
            return Err(Error::Invalid(format!("coupling `{name}`: set `{}` has no nodes", c.set)));
        }
        let nodes: Vec<usize> = set
            .nodes
            .iter()
            .map(|l| mesh.node(*l).ok_or_else(|| Error::Invalid(format!("coupling `{name}`: node {l} belongs to no solid element"))))
            .collect::<Result<_>>()?;
        let xyz = |n: usize| mesh.xyz.get(n).copied().unwrap_or_default();
        let origin = format!("coupling `{name}`");
        match c.kind {
            CouplingKind::Kinematic => {
                let translations: Vec<usize> = (0..3).filter(|k| c.dofs.contains(*k as u8 + 1)).collect();
                if translations.is_empty() {
                    return Err(Error::Invalid(format!(
                        "coupling `{name}` couples only rotations, which the nodes of solid elements do not have; couple translations too"
                    )));
                }
                for &n in &nodes {
                    let r = sub(xyz(n), xr);
                    for &k in &translations {
                        // u_s,k − u_r,k − (θ × r)_k = 0, (θ × r)_k = θ_a r_b − θ_b r_a.
                        let (a, b) = ((k + 1) % 3, (k + 2) % 3);
                        let mut terms = vec![(3 * n + k, 1.0), (rp + k, -1.0)];
                        if r[b] != 0.0 {
                            terms.push((rp + 3 + a, -r[b]));
                        }
                        if r[a] != 0.0 {
                            terms.push((rp + 3 + b, r[a]));
                        }
                        rows.push(Row { terms, pivot: 3 * n + k, origin: origin.clone() });
                    }
                }
            }
            CouplingKind::Distributing => {
                let count = nodes.len() as f64;
                let mut centroid = [0.0; 3];
                for &n in &nodes {
                    let x = xyz(n);
                    for k in 0..3 {
                        centroid[k] += x[k] / count;
                    }
                }
                let arms: Vec<[f64; 3]> = nodes.iter().map(|n| sub(xyz(*n), centroid)).collect();
                let mut j = [[0.0; 3]; 3];
                for r in &arms {
                    let rr = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
                    for a in 0..3 {
                        for b in 0..3 {
                            j[a][b] += if a == b { rr } else { 0.0 } - r[a] * r[b];
                        }
                    }
                }
                let trace = j[0][0] + j[1][1] + j[2][2];
                let inverse = inverse3(&j).filter(|(_, det)| *det > 1e-10 * trace.powi(3) / 27.0).map(|(inv, _)| inv);
                let rotations = (4..=6).any(|d| c.dofs.contains(d));
                if inverse.is_none() && rotations {
                    return Err(Error::Invalid(format!(
                        "coupling `{name}`: the nodes of set `{}` lie on one line (or at one point), so a distributing coupling cannot carry rotations through them; couple translations only or use a kinematic coupling",
                        c.set
                    )));
                }
                // θ = Σ Gᵢ uᵢ with Gᵢ = J⁻¹ [rᵢ]×; u_r = Σ (I/N − [d]× Gᵢ) uᵢ with d = x_r − c.
                let d = cross_matrix(sub(xr, centroid));
                let g: Vec<[[f64; 3]; 3]> = arms.iter().map(|r| inverse.map_or([[0.0; 3]; 3], |inv| mat_mul(&inv, &cross_matrix(*r)))).collect();
                for k in 0..6u8 {
                    if !c.dofs.contains(k + 1) {
                        continue;
                    }
                    let k = usize::from(k);
                    let mut terms = vec![(rp + k, 1.0)];
                    for (&n, gi) in nodes.iter().zip(&g) {
                        for comp in 0..3 {
                            let a = if k < 3 {
                                let dg: f64 = (0..3).map(|m| d[k][m] * gi[m][comp]).sum();
                                if k == comp { 1.0 / count - dg } else { -dg }
                            } else {
                                gi[k - 3][comp]
                            };
                            if a != 0.0 {
                                terms.push((3 * n + comp, -a));
                            }
                        }
                    }
                    rows.push(Row { terms, pivot: rp + k, origin: origin.clone() });
                }
            }
        }
    }
    Ok(rows)
}

/// The degrees of freedom that exist: every mesh translation, and the reference point
/// directions some coupling row involves.
pub fn active_dofs(mesh: &Mesh, rows: &[Row]) -> Vec<bool> {
    let mut active = vec![false; mesh.dofs()];
    active.iter_mut().take(mesh.mesh_dofs()).for_each(|a| *a = true);
    for row in rows {
        for (dof, c) in &row.terms {
            if *c != 0.0
                && let Some(a) = active.get_mut(*dof)
            {
                *a = true;
            }
        }
    }
    active
}

/// A sparse list of (index, coefficient) per row.
#[derive(Debug, Clone, Default)]
struct Terms {
    ptr: Vec<usize>,
    idx: Vec<usize>,
    val: Vec<f64>,
}

impl Terms {
    fn row(&self, r: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        let (a, b) = (self.ptr.get(r).copied().unwrap_or(0), self.ptr.get(r + 1).copied().unwrap_or(0));
        self.idx.get(a..b).unwrap_or_default().iter().copied().zip(self.val.get(a..b).unwrap_or_default().iter().copied())
    }

    fn is_empty_row(&self, r: usize) -> bool {
        self.ptr.get(r) == self.ptr.get(r + 1)
    }

    fn push_row(&mut self, terms: impl Iterator<Item = (usize, f64)>) {
        if self.ptr.is_empty() {
            self.ptr.push(0);
        }
        for (i, v) in terms {
            self.idx.push(i);
            self.val.push(v);
        }
        self.ptr.push(self.idx.len());
    }

    /// The transpose, with `n` rows.
    fn transpose(&self, n: usize) -> Terms {
        let rows = self.ptr.len().saturating_sub(1);
        let mut count = vec![0usize; n + 1];
        for i in &self.idx {
            if let Some(c) = count.get_mut(i + 1) {
                *c += 1;
            }
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let mut next = count.clone();
        let mut idx = vec![0; self.idx.len()];
        let mut val = vec![0.0; self.idx.len()];
        for r in 0..rows {
            for (i, v) in self.row(r) {
                if let Some(slot) = next.get_mut(i)
                    && let (Some(x), Some(y)) = (idx.get_mut(*slot), val.get_mut(*slot))
                {
                    *x = r;
                    *y = v;
                    *slot += 1;
                }
            }
        }
        Terms { ptr: count, idx, val }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Free,
    Fixed,
    Dependent(usize),
    Inactive,
}

/// Add `scale · src` into the sorted `dst`.
fn merge(dst: &mut Vec<(usize, f64)>, src: &[(usize, f64)], scale: f64) {
    let mut out = Vec::with_capacity(dst.len() + src.len());
    let (mut i, mut j) = (0, 0);
    while i < dst.len() || j < src.len() {
        match (dst.get(i), src.get(j)) {
            (Some(a), Some(b)) if a.0 == b.0 => {
                out.push((a.0, a.1 + scale * b.1));
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a.0 < b.0 => {
                out.push(*a);
                i += 1;
            }
            (Some(a), None) => {
                out.push(*a);
                i += 1;
            }
            (_, Some(b)) => {
                out.push((b.0, scale * b.1));
                j += 1;
            }
            (None, None) => break,
        }
    }
    out.retain(|t| t.1 != 0.0);
    *dst = out;
}

/// The map from the free equations and prescribed values to every degree of freedom.
#[derive(Debug, Clone)]
pub struct Reduction {
    /// Free equations.
    pub neq: usize,
    /// The degree of freedom of each equation.
    pub eq_dof: Vec<usize>,
    /// Prescribed value of each degree of freedom (0 where none).
    pub values: Vec<f64>,
    /// Whether a degree of freedom is prescribed.
    pub prescribed: Vec<bool>,
    /// Whether a degree of freedom depends on others through a constraint.
    pub dependent: Vec<bool>,
    /// T_f: per degree of freedom, (equation, coefficient).
    free: Terms,
    /// T_p: per degree of freedom, (prescribed degree of freedom, coefficient).
    fixed: Terms,
}

impl Reduction {
    /// Eliminate the prescribed values and the constraint rows. `order` is the mesh nodes' order
    /// for numbering the free equations; reference point equations follow them.
    pub fn new(mesh: &Mesh, order: &[usize], prescribed: &[Option<f64>], rows: &[Row], active: &[bool]) -> Result<Reduction> {
        let n = mesh.dofs();
        let mut state: Vec<State> = (0..n)
            .map(|d| match (active.get(d).copied().unwrap_or(false), prescribed.get(d).copied().flatten()) {
                (false, _) => State::Inactive,
                (true, Some(_)) => State::Fixed,
                (true, None) => State::Free,
            })
            .collect();
        let values: Vec<f64> = (0..n).map(|d| prescribed.get(d).copied().flatten().unwrap_or(0.0)).collect();
        // Dependent expressions in free and prescribed degrees of freedom, and for each free
        // degree of freedom the expressions that use it.
        let mut exprs: Vec<Vec<(usize, f64)>> = Vec::new();
        let mut users: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for row in rows {
            let scale = row.terms.iter().fold(0.0f64, |m, t| m.max(t.1.abs()));
            if !(scale > 0.0 && scale.is_finite()) {
                return Err(Error::Invalid(format!("{}: a constraint with no finite coefficient", row.origin)));
            }
            // The row in current free and prescribed degrees of freedom.
            let mut terms: Vec<(usize, f64)> = Vec::new();
            let mut sorted = row.terms.clone();
            sorted.sort_by_key(|t| t.0);
            for (dof, c) in sorted {
                match state.get(dof).copied() {
                    Some(State::Free | State::Fixed) => merge(&mut terms, &[(dof, c)], 1.0),
                    Some(State::Dependent(e)) => merge(&mut terms, exprs.get(e).map(Vec::as_slice).unwrap_or_default(), c),
                    Some(State::Inactive) | None => return Err(Error::Internal("a constraint on a missing degree of freedom".into())),
                }
            }
            let candidate = |t: &&(usize, f64)| state.get(t.0) == Some(&State::Free) && t.1.abs() > NEGLIGIBLE * scale;
            let intended = terms.iter().filter(candidate).find(|t| t.0 == row.pivot);
            let largest = terms.iter().filter(candidate).fold(None, |best: Option<&(usize, f64)>, t| match best {
                Some(b) if b.1.abs() >= t.1.abs() => Some(b),
                _ => Some(t),
            });
            let Some(&(pivot, cp)) = intended.or(largest) else {
                // Only prescribed degrees of freedom (or round-off) are left.
                let residual: f64 =
                    terms.iter().filter(|t| state.get(t.0) == Some(&State::Fixed)).map(|t| t.1 * values.get(t.0).copied().unwrap_or(0.0)).sum();
                let size: f64 = terms.iter().map(|t| (t.1 * values.get(t.0).copied().unwrap_or(0.0)).abs()).sum();
                if residual.abs() > 1e-9 * size.max(f64::MIN_POSITIVE) && residual.abs() > 0.0 {
                    return Err(Error::Invalid(format!(
                        "{} contradicts the prescribed values (boundary conditions or other couplings hold its nodes in a way it cannot follow)",
                        row.origin
                    )));
                }
                continue;
            };
            let expr: Vec<(usize, f64)> = terms.iter().filter(|t| t.0 != pivot).map(|t| (t.0, -t.1 / cp)).collect();
            // Substitute the pivot in the expressions that use it.
            if let Some(using) = users.remove(&pivot) {
                for u in using {
                    let Some(e) = exprs.get_mut(u) else { continue };
                    let Ok(at) = e.binary_search_by_key(&pivot, |t| t.0) else { continue };
                    let a = e.remove(at).1;
                    merge(e, &expr, a);
                    for t in e.iter() {
                        if state.get(t.0) == Some(&State::Free) {
                            users.entry(t.0).or_default().insert(u);
                        }
                    }
                }
            }
            let index = exprs.len();
            for t in &expr {
                if state.get(t.0) == Some(&State::Free) {
                    users.entry(t.0).or_default().insert(index);
                }
            }
            exprs.push(expr);
            if let Some(s) = state.get_mut(pivot) {
                *s = State::Dependent(index);
            }
        }
        // Equations: free mesh degrees of freedom in node order, then reference point ones.
        let mut eq = vec![usize::MAX; n];
        let mut eq_dof = Vec::new();
        let mesh_order = order.iter().flat_map(|node| (0..3).map(move |c| 3 * node + c));
        for dof in mesh_order.chain(mesh.mesh_dofs()..n) {
            if state.get(dof) == Some(&State::Free)
                && let Some(e) = eq.get_mut(dof)
                && *e == usize::MAX
            {
                *e = eq_dof.len();
                eq_dof.push(dof);
            }
        }
        if state.iter().enumerate().any(|(d, s)| *s == State::Free && eq.get(d) == Some(&usize::MAX)) {
            return Err(Error::Internal("the node order misses a degree of freedom".into()));
        }
        let (mut free, mut fixed) = (Terms::default(), Terms::default());
        let mut dependent = vec![false; n];
        for dof in 0..n {
            match state.get(dof).copied().unwrap_or(State::Inactive) {
                State::Free => {
                    free.push_row(std::iter::once((eq.get(dof).copied().unwrap_or(0), 1.0)));
                    fixed.push_row(std::iter::empty());
                }
                State::Fixed => {
                    free.push_row(std::iter::empty());
                    fixed.push_row(std::iter::once((dof, 1.0)));
                }
                State::Dependent(e) => {
                    let expr = exprs.get(e).map(Vec::as_slice).unwrap_or_default();
                    let state = &state;
                    let on = |want: State| expr.iter().filter(move |t| state.get(t.0) == Some(&want));
                    free.push_row(on(State::Free).map(|t| (eq.get(t.0).copied().unwrap_or(0), t.1)));
                    fixed.push_row(on(State::Fixed).copied());
                    if let Some(d) = dependent.get_mut(dof) {
                        *d = true;
                    }
                }
                State::Inactive => {
                    free.push_row(std::iter::empty());
                    fixed.push_row(std::iter::empty());
                }
            }
        }
        let prescribed = state.iter().map(|s| *s == State::Fixed).collect();
        Ok(Reduction { neq: eq_dof.len(), eq_dof, values, prescribed, dependent, free, fixed })
    }

    /// Whether any degree of freedom depends on others (T is not a plain selection).
    pub fn has_dependents(&self) -> bool {
        self.dependent.iter().any(|d| *d)
    }

    /// T_fᵀ A T_f for a matrix over the mesh's degrees of freedom (reference points carry no
    /// stiffness or mass of their own). Row by row with a dense accumulator (F. G. Gustavson,
    /// *Two fast algorithms for sparse matrices*, ACM TOMS 4(3):250–269, 1978), in a fixed order;
    /// structural zeros are kept so matrices on one pattern keep one pattern.
    pub fn reduce(&self, a: &Csr) -> Csr {
        let n = self.neq;
        let columns = self.free.transpose(n);
        let mut acc = vec![0.0; n];
        let mut mark = vec![usize::MAX; n];
        let mut touched: Vec<usize> = Vec::new();
        let mut ptr = Vec::with_capacity(n + 1);
        ptr.push(0);
        let (mut idx, mut val) = (Vec::new(), Vec::new());
        for e in 0..n {
            touched.clear();
            for (dof, ca) in columns.row(e) {
                let (cols, vals) = a.row(dof);
                for (b, v) in cols.iter().zip(vals) {
                    for (f, cb) in self.free.row(*b) {
                        let (Some(m), Some(x)) = (mark.get_mut(f), acc.get_mut(f)) else { continue };
                        if *m != e {
                            *m = e;
                            *x = 0.0;
                            touched.push(f);
                        }
                        *x += ca * v * cb;
                    }
                }
            }
            touched.sort_unstable();
            for &f in &touched {
                idx.push(f);
                val.push(acc.get(f).copied().unwrap_or(0.0));
            }
            ptr.push(idx.len());
        }
        let mut out = Csr { n, ptr, idx, val };
        if self.has_dependents() {
            symmetrize(&mut out);
        }
        out
    }

    /// The displacement of the prescribed values alone, T_p u_p, over every degree of freedom.
    fn prescribed_motion(&self) -> Vec<f64> {
        (0..self.values.len()).map(|d| self.fixed.row(d).map(|(p, c)| c * self.values.get(p).copied().unwrap_or(0.0)).sum()).collect()
    }

    /// The reduced right-hand side T_fᵀ (f − K T_p u_p); `k` is over the mesh's degrees of freedom.
    pub fn rhs(&self, f: &[f64], k: &Csr) -> Vec<f64> {
        let mut g = f.to_vec();
        let u0 = self.prescribed_motion();
        if u0.iter().any(|v| *v != 0.0) {
            let mut ku = vec![0.0; k.n];
            k.mul(u0.get(..k.n).unwrap_or_default(), &mut ku);
            for (x, y) in g.iter_mut().zip(&ku) {
                *x -= y;
            }
        }
        let mut rhs = vec![0.0; self.neq];
        for (dof, gv) in g.iter().enumerate() {
            for (e, c) in self.free.row(dof) {
                if let Some(r) = rhs.get_mut(e) {
                    *r += c * gv;
                }
            }
        }
        rhs
    }

    /// Every degree of freedom from the free equations' values `q` (and the prescribed values
    /// when `with_values`).
    pub fn expand(&self, q: &[f64], with_values: bool) -> Vec<f64> {
        let u0 = if with_values { self.prescribed_motion() } else { vec![0.0; self.values.len()] };
        u0.iter()
            .enumerate()
            .map(|(d, base)| {
                let free: f64 = self.free.row(d).map(|(e, c)| c * q.get(e).copied().unwrap_or(0.0)).sum();
                if self.fixed.is_empty_row(d) { free } else { base + free }
            })
            .collect()
    }

    /// The reactions at the prescribed degrees of freedom, T_pᵀ (K u − f), over every degree of
    /// freedom (zero where nothing is prescribed).
    pub fn reactions(&self, k: &Csr, u: &[f64], f: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.values.len()];
        for (dof, fv) in f.iter().enumerate() {
            if self.fixed.is_empty_row(dof) {
                continue;
            }
            let (cols, vals) = k.row(dof);
            let ku: f64 = if dof < k.n { cols.iter().zip(vals).map(|(j, v)| v * u.get(*j).copied().unwrap_or(0.0)).sum() } else { 0.0 };
            let r = ku - fv;
            for (p, c) in self.fixed.row(dof) {
                if let Some(x) = out.get_mut(p) {
                    *x += c * r;
                }
            }
        }
        out
    }
}

/// Make a matrix exactly symmetric by copying the lower triangle (what the factorization reads)
/// onto the upper one.
fn symmetrize(a: &mut Csr) {
    for r in 0..a.n {
        let (start, end) = (a.ptr.get(r).copied().unwrap_or(0), a.ptr.get(r + 1).copied().unwrap_or(0));
        for at in start..end {
            let Some(&c) = a.idx.get(at) else { continue };
            if c <= r {
                continue;
            }
            let (cs, ce) = (a.ptr.get(c).copied().unwrap_or(0), a.ptr.get(c + 1).copied().unwrap_or(0));
            let mirror = a.idx.get(cs..ce).and_then(|cols| cols.binary_search(&r).ok()).and_then(|i| a.val.get(cs + i)).copied();
            if let (Some(v), Some(x)) = (mirror, a.val.get_mut(at)) {
                *x = v;
            }
        }
    }
}
