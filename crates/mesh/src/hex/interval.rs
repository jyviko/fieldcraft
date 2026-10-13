//! Interval assignment: the number of mesh divisions of every curve, chosen together so that
//! every face mesher downstream can succeed with conforming meshes on shared curves.
//!
//! The problem is the integer program of Tam and Armstrong (*Finite element mesh control by
//! integer programming*, IJNME 36(15):2581–2605, 1993) and Mitchell (*High fidelity interval
//! assignment*, Proc. 6th International Meshing Roundtable, 1997; IJCGA 10(4):399–415, 2000):
//! integer counts `x_i ≥ 1` close to goals `g_i` (the curve length over the target size), subject to
//! - **equalities** `Σ_A x = Σ_B x`: opposite sides of a mapped face or of a sweep's linking
//!   face;
//! - **parity**: `Σ_L x` even on every loop of a face meshed by the free all-quad mesher (a
//!   multiple of 4 around a disk meshed as an O-grid);
//! - **minimum sums** `Σ_L x ≥ m`: a loop needs enough points to be meshed.
//!
//! Mitchell's *incremental interval assignment* (Mitchell, *Incremental interval assignment by
//! integer linear algebra*, Proc. 29th International Meshing Roundtable, 2021) starts from a
//! rounded solution and repairs violated constraints by unit steps chosen by a priority on the
//! change in relative error. This module uses that scheme with increments only: start from the
//! rounded goals, then repeatedly take the first violated constraint and raise the variable whose
//! increase costs least in squared relative error `((x − g)/g)²`, preferring, for parity, variables that
//! no equality couples to other faces. Increments only make the counts never coarser than asked
//! and make termination provable for feasible systems up to the step cap; an infeasible system
//! (one that only grows) hits the cap and is reported, never looped on.

use crate::{Error, Result};

/// The constraint system.
#[derive(Debug, Clone, Default)]
pub(crate) struct Intervals {
    /// Goal count of each variable (curve length over target size).
    pub goals: Vec<f64>,
    /// Lower bound of each variable (at least 1).
    pub mins: Vec<usize>,
    /// `Σ lhs = Σ rhs` (variables may repeat).
    pub equal: Vec<(Vec<usize>, Vec<usize>)>,
    /// `Σ vars` a multiple of the modulus (2: even loops for the free mesher; 4: a disk's
    /// O-grid).
    pub multiple: Vec<(Vec<usize>, usize)>,
    /// `Σ vars ≥ min`.
    pub at_least: Vec<(Vec<usize>, usize)>,
}

/// The largest count a variable may reach.
const MAX_COUNT: usize = 1 << 20;

impl Intervals {
    /// Add a variable with its goal and minimum; returns its index.
    pub(crate) fn var(&mut self, goal: f64, min: usize) -> usize {
        self.goals.push(goal);
        self.mins.push(min.max(1));
        self.goals.len() - 1
    }

    pub(crate) fn solve(&self) -> Result<Vec<usize>> {
        let n = self.goals.len();
        if self.mins.len() != n {
            return Err(Error::Failed("interval assignment: goals and minimums differ in length".into()));
        }
        let mut x = Vec::with_capacity(n);
        for (i, &g) in self.goals.iter().enumerate() {
            if !(g >= 0.0 && g.is_finite()) {
                return Err(Error::Invalid(format!("interval goal {g} is not a finite, non-negative number")));
            }
            if g > MAX_COUNT as f64 {
                return Err(Error::Limit(format!("a curve would get {g:.0} divisions; the limit is {MAX_COUNT}")));
            }
            x.push((g.round() as usize).max(self.mins.get(i).copied().unwrap_or(1)));
        }
        let check = |v: &[usize]| v.iter().all(|&i| i < n);
        if !self.equal.iter().all(|(a, b)| check(a) && check(b))
            || !self.multiple.iter().all(|(l, m)| check(l) && *m >= 2)
            || !self.at_least.iter().all(|(l, _)| check(l))
        {
            return Err(Error::Failed("interval assignment: a constraint names an unknown curve".into()));
        }
        let mut coupled = vec![false; n];
        for (a, b) in &self.equal {
            for &i in a.iter().chain(b) {
                if let Some(c) = coupled.get_mut(i) {
                    *c = true;
                }
            }
        }
        let error = |i: usize, c: usize| -> f64 {
            let g = self.goals.get(i).copied().unwrap_or(1.0).max(1e-3);
            // Squared relative deviation: strictly convex, so repeated increments spread over
            // the candidates instead of piling onto one.
            ((c as f64 - g) / g).powi(2)
        };
        let step_cost = |x: &[usize], i: usize| -> f64 {
            let c = x.get(i).copied().unwrap_or(0);
            error(i, c + 1) - error(i, c)
        };
        let total: usize = x.iter().sum();
        let constraints = self.equal.len() + self.multiple.len() + self.at_least.len();
        let max_steps = total.saturating_mul(8).saturating_add(64 * (n + constraints)).saturating_add(1000);
        for _ in 0..max_steps {
            let sum = |v: &[usize], x: &[usize]| v.iter().map(|&i| x.get(i).copied().unwrap_or(0)).sum::<usize>();
            // The candidates to raise and whether raising them is what the constraint needs.
            let mut raise: Option<Vec<usize>> = None;
            let mut parity = false;
            for (a, b) in &self.equal {
                let (sa, sb) = (sum(a, &x), sum(b, &x));
                if sa == sb {
                    continue;
                }
                // Raise a variable whose net coefficient is on the smaller side.
                let (small, large) = if sa < sb { (a, b) } else { (b, a) };
                let net = |i: usize| small.iter().filter(|&&k| k == i).count() as i64 - large.iter().filter(|&&k| k == i).count() as i64;
                let cands: Vec<usize> = small.iter().copied().filter(|&i| net(i) > 0).collect();
                if cands.is_empty() {
                    return Err(Error::Failed("interval assignment: a side constraint cannot be met (the same curves on both sides)".into()));
                }
                raise = Some(cands);
                break;
            }
            if raise.is_none() {
                for (l, m) in &self.at_least {
                    if sum(l, &x) < *m {
                        raise = Some(l.clone());
                        break;
                    }
                }
            }
            if raise.is_none() {
                for (l, m) in &self.multiple {
                    if !sum(l, &x).is_multiple_of(*m) {
                        // Only a variable listed a number of times that is not a multiple of the
                        // modulus moves the remainder.
                        let moving: Vec<usize> = l.iter().copied().filter(|&i| !l.iter().filter(|&&k| k == i).count().is_multiple_of(*m)).collect();
                        if moving.is_empty() {
                            return Err(Error::Failed("interval assignment: a loop's count cannot reach the multiple it needs".into()));
                        }
                        raise = Some(moving);
                        parity = true;
                        break;
                    }
                }
            }
            let Some(cands) = raise else { return Ok(x) };
            let pick = cands
                .iter()
                .copied()
                .min_by(|&a, &b| {
                    let key = |i: usize| (parity && coupled.get(i).copied().unwrap_or(false), step_cost(&x, i));
                    let (ka, kb) = (key(a), key(b));
                    ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1)).then(a.cmp(&b))
                })
                .ok_or_else(|| Error::Failed("interval assignment: empty constraint".into()))?;
            let v = x.get_mut(pick).ok_or_else(|| Error::Failed("interval index".into()))?;
            *v += 1;
            if *v > MAX_COUNT {
                return Err(Error::Limit(format!("interval assignment drove a curve past {MAX_COUNT} divisions")));
            }
        }
        Err(Error::Failed(format!(
            "interval assignment did not settle in {max_steps} steps: the side and parity constraints of the faces conflict (the region cannot be mapped or swept as decomposed)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_goals_without_constraints() {
        let mut p = Intervals::default();
        p.var(2.4, 1);
        p.var(0.2, 1);
        p.var(7.6, 3);
        assert_eq!(p.solve().unwrap(), vec![2, 1, 8]);
    }

    #[test]
    fn mapped_sides_become_equal_and_loops_even() {
        // A rectangle: sides 0 and 2 opposite, 1 and 3 opposite; side 0 shared with a free face
        // whose loop is 0 + 4 + 5.
        let mut p = Intervals::default();
        for g in [4.2, 1.1, 3.6, 1.4, 2.0, 2.0] {
            p.var(g, 1);
        }
        p.equal.push((vec![0], vec![2]));
        p.equal.push((vec![1], vec![3]));
        p.multiple.push((vec![0, 4, 5], 2));
        let x = p.solve().unwrap();
        assert_eq!(x[0], x[2]);
        assert_eq!(x[1], x[3]);
        assert_eq!((x[0] + x[4] + x[5]) % 2, 0);
        // The parity fix went to an uncoupled curve.
        assert_eq!(x[0], 4);
    }

    #[test]
    fn sums_and_minimums() {
        let mut p = Intervals::default();
        let a = p.var(1.0, 1);
        let b = p.var(1.0, 1);
        let c = p.var(5.0, 1);
        p.equal.push((vec![a, b], vec![c]));
        p.at_least.push((vec![a, b, c], 12));
        p.multiple.push((vec![a, b, c], 2));
        let x = p.solve().unwrap();
        assert_eq!(x[a] + x[b], x[c]);
        assert!(x[a] + x[b] + x[c] >= 12);
        assert_eq!((x[a] + x[b] + x[c]) % 2, 0);
    }

    #[test]
    fn infeasible_systems_are_errors_not_loops() {
        let mut p = Intervals::default();
        let a = p.var(3.0, 1);
        let b = p.var(3.0, 1);
        // a = a + b has no solution with b ≥ 1.
        p.equal.push((vec![a], vec![a, b]));
        assert!(p.solve().is_err());
        let mut q = Intervals::default();
        let a = q.var(3.0, 1);
        q.multiple.push((vec![a, a], 2));
        q.multiple.push((vec![a, a, a], 2));
        q.multiple.push((vec![a, a, a], 4));
        assert!(q.solve().is_ok());
        let mut r = Intervals::default();
        r.var(f64::NAN, 1);
        assert!(r.solve().is_err());
        let mut s = Intervals::default();
        s.var(1e30, 1);
        assert!(s.solve().is_err());
    }
}
