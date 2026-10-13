//! Fill-reducing ordering: nested dissection by coordinate bisection.
//!
//! Nested dissection (A. George, *Nested dissection of a regular finite element mesh*, SIAM J.
//! Numer. Anal. 10(2):345–363, 1973) orders a separator after the two parts it splits, so the
//! Cholesky factor of a 3D mesh with n nodes fills O(n^{4/3}) instead of the O(n^{5/3}) of a band
//! or envelope ordering. The separator here is geometric: the nodes are split at the median of
//! the longest axis of their bounding box, and the nodes of the smaller side that touch the other
//! side form the separator (the coordinate nested dissection of H. D. Simon, *Partitioning of
//! unstructured problems for parallel processing*, Computing Systems in Engineering 2:135–148,
//! 1991, with vertex separators). Ties are broken by node index, so the ordering is
//! deterministic.

/// Parts this small are ordered as they come.
const LEAF: usize = 16;

/// A permutation of the nodes: `order[k]` is the node eliminated k-th.
pub fn nested_dissection(xyz: &[[f64; 3]], adjacency: &[Vec<usize>]) -> Vec<usize> {
    let n = xyz.len();
    let mut order = Vec::with_capacity(n);
    // 0 = not in the current part, 1 = left, 2 = right.
    let mut side = vec![0u8; n];
    // An explicit stack instead of recursion: (part, separator to emit after it).
    enum Task {
        Split(Vec<usize>),
        Emit(Vec<usize>),
    }
    let mut stack = vec![Task::Split((0..n).collect())];
    while let Some(task) = stack.pop() {
        let mut part = match task {
            Task::Emit(sep) => {
                order.extend(sep);
                continue;
            }
            Task::Split(p) => p,
        };
        if part.len() <= LEAF {
            order.extend(part);
            continue;
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &v in &part {
            let p = xyz.get(v).copied().unwrap_or_default();
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let axis = (0..3).fold(0, |best, k| if hi[k] - lo[k] > hi[best] - lo[best] { k } else { best });
        let key = |v: usize| xyz.get(v).map_or(0.0, |p| p[axis]);
        part.sort_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)));
        let half = part.len() / 2;
        for (i, &v) in part.iter().enumerate() {
            if let Some(s) = side.get_mut(v) {
                *s = if i < half { 1 } else { 2 };
            }
        }
        let touches = |v: usize, other: u8| adjacency.get(v).is_some_and(|row| row.iter().any(|w| side.get(*w) == Some(&other)));
        let (left, right) = part.split_at(half);
        let left_boundary: Vec<usize> = left.iter().copied().filter(|&v| touches(v, 2)).collect();
        let right_boundary: Vec<usize> = right.iter().copied().filter(|&v| touches(v, 1)).collect();
        let separator = if left_boundary.len() <= right_boundary.len() { left_boundary } else { right_boundary };
        for &v in &part {
            if let Some(s) = side.get_mut(v) {
                *s = 0;
            }
        }
        // Mark the separator so the parts leave it out.
        for &v in &separator {
            if let Some(s) = side.get_mut(v) {
                *s = 3;
            }
        }
        let a: Vec<usize> = left.iter().copied().filter(|v| side.get(*v) != Some(&3)).collect();
        let b: Vec<usize> = right.iter().copied().filter(|v| side.get(*v) != Some(&3)).collect();
        for &v in &separator {
            if let Some(s) = side.get_mut(v) {
                *s = 0;
            }
        }
        // Order: a, then b, then the separator (the stack runs in reverse).
        stack.push(Task::Emit(separator));
        stack.push(Task::Split(b));
        stack.push(Task::Split(a));
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permutation_with_separators_last() {
        // A 12 × 12 grid graph.
        let m = 12;
        let id = |i: usize, j: usize| i * m + j;
        let xyz: Vec<[f64; 3]> = (0..m * m).map(|v| [(v / m) as f64, (v % m) as f64, 0.0]).collect();
        let mut adj = vec![Vec::new(); m * m];
        for i in 0..m {
            for j in 0..m {
                for (di, dj) in [(0i64, 1i64), (1, 0), (0, -1), (-1, 0), (0, 0)] {
                    let (a, b) = (i as i64 + di, j as i64 + dj);
                    if (0..m as i64).contains(&a) && (0..m as i64).contains(&b) {
                        adj[id(i, j)].push(id(a as usize, b as usize));
                    }
                }
            }
        }
        let order = nested_dissection(&xyz, &adj);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..m * m).collect::<Vec<_>>());
        // The top separator (last 12 entries) is one grid line.
        let last: Vec<usize> = order[m * m - m..].to_vec();
        assert!(last.iter().all(|v| v / m == last[0] / m) || last.iter().all(|v| v % m == last[0] % m), "{last:?}");
        assert_eq!(order, nested_dissection(&xyz, &adj), "deterministic");
    }
}
