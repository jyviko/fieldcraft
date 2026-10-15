//! Node tools on a model's mesh: merging coincident nodes ([`merge_plan`]), finding what was
//! not merged or does not join ([`duplicates`]), and a node order of small bandwidth for
//! renumbering ([`reverse_cuthill_mckee`]).
//!
//! The bandwidth order is reverse Cuthill–McKee: E. Cuthill, J. McKee, *Reducing the bandwidth
//! of sparse symmetric matrices*, Proc. 24th ACM National Conference, 1969, pp. 157–172, with
//! the reversal of A. George (1971) and each component started from a pseudo-peripheral node
//! found by repeated breadth-first level structures (A. George, J. W. H. Liu, *An implementation
//! of a pseudoperipheral node finder*, ACM TOMS 5(3):284–295, 1979).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use fieldcraft_model::{ElementFace, ElementKind, Model};

use crate::Result;
use crate::join::coincident_nodes;
use crate::vec3::dist;

/// Which node of a coincident group survives a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Keep {
    #[default]
    Lowest,
    Highest,
}

/// A merge that was not made: it would put two nodes of `element` on one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    pub node: u32,
    pub survivor: u32,
    pub element: u32,
}

/// The nodes to merge, each into its survivor, and the merges left out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergePlan {
    pub map: BTreeMap<u32, u32>,
    pub refused: Vec<Refused>,
}

/// Which nodes among `candidates` (every node when `None`) merge within `tol`, the survivor of
/// each group chosen by `keep`. A merge that would put two nodes of one element together (the
/// element would collapse) is left out and listed, so the rest still merge; the plan is then
/// safe for [`Model::merge_nodes`].
pub fn merge_plan(model: &Model, candidates: Option<&BTreeSet<u32>>, tol: f64, keep: Keep) -> Result<MergePlan> {
    let mut map = crate::join::coincident_nodes_from(model, candidates, tol, keep == Keep::Highest)?;
    let mut refused = Vec::new();
    // Drop merges that would collapse an element until none does (each pass removes at least
    // one, so this ends).
    loop {
        let mut drop: Vec<Refused> = Vec::new();
        for (&label, e) in model.elements() {
            let mut seen: BTreeMap<u32, u32> = BTreeMap::new();
            for &n in &e.nodes {
                let to = map.get(&n).copied().unwrap_or(n);
                if let Some(&first) = seen.get(&to) {
                    // `n` and `first` would both become `to`: keep whichever of them is `to`.
                    let node = if map.contains_key(&n) { n } else { first };
                    drop.push(Refused { node, survivor: to, element: label });
                } else {
                    seen.insert(to, n);
                }
            }
        }
        if drop.is_empty() {
            break;
        }
        for r in drop {
            if map.remove(&r.node).is_some() {
                refused.push(r);
            }
        }
    }
    refused.sort_by_key(|r| (r.node, r.element));
    Ok(MergePlan { map, refused })
}

/// What [`duplicates`] found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Duplicates {
    /// Nodes within the distance of an earlier one: (node, the one it lies on, distance).
    pub coincident: Vec<(u32, u32, f64)>,
    /// Groups of elements on the same nodes.
    pub elements: Vec<Vec<u32>>,
    /// Faces of solid elements that no other element shares (the skin, cracks included).
    pub free_faces: usize,
    /// Free faces lying on another free face within the distance: cracks between parts that
    /// touch without sharing nodes.
    pub cracks: Vec<ElementFace>,
    /// Edges of the skin (free faces of solids, and 2D and shell elements) on only one of its
    /// facets: the border of an open surface or a crack.
    pub free_edges: Vec<[u32; 2]>,
}

/// Coincident nodes not merged, duplicate elements, and free faces and edges.
pub fn duplicates(model: &Model, tol: f64) -> Result<Duplicates> {
    let xyz = |l: u32| model.node(l).map(|n| n.xyz);
    let coincident: Vec<(u32, u32, f64)> =
        coincident_nodes(model, None, tol)?.into_iter().map(|(a, b)| (a, b, xyz(a).zip(xyz(b)).map_or(0.0, |(p, q)| dist(p, q)))).collect();
    // Elements on the same node set.
    let mut by_nodes: BTreeMap<Vec<u32>, Vec<u32>> = BTreeMap::new();
    for (&label, e) in model.elements() {
        let mut k = e.nodes.clone();
        k.sort_unstable();
        by_nodes.entry(k).or_default().push(label);
    }
    let elements: Vec<Vec<u32>> = by_nodes.into_values().filter(|g| g.len() > 1).collect();
    // Free faces of solids, by sorted corners.
    let mut faces: BTreeMap<Vec<u32>, (usize, ElementFace)> = BTreeMap::new();
    let mut facets: Vec<Vec<u32>> = Vec::new();
    for (&label, e) in model.elements() {
        match e.kind.dimension() {
            3 => {
                for (k, local) in e.kind.faces().iter().enumerate() {
                    let corners = corner_cycle(e.kind, &e.nodes, local);
                    let mut key = corners.clone();
                    key.sort_unstable();
                    let face = ElementFace { element: label, face: u8::try_from(k + 1).unwrap_or(u8::MAX) };
                    faces.entry(key).and_modify(|x| x.0 += 1).or_insert((1, face));
                }
            }
            2 => facets.push(corner_cycle(e.kind, &e.nodes, &(0..e.kind.corner_count()).collect::<Vec<_>>())),
            _ => {}
        }
    }
    let free: Vec<(Vec<u32>, ElementFace)> = faces.into_iter().filter(|(_, (n, _))| *n == 1).map(|(k, (_, f))| (k, f)).collect();
    // Cracks: a free face whose corners lie on another free face's corners.
    let mut cracks = Vec::new();
    if !coincident.is_empty() {
        let survivor: BTreeMap<u32, u32> = coincident.iter().map(|&(a, b, _)| (a, b)).collect();
        let mut by_position: BTreeMap<Vec<u32>, Vec<ElementFace>> = BTreeMap::new();
        for (k, f) in &free {
            let mut key: Vec<u32> = k.iter().map(|n| survivor.get(n).copied().unwrap_or(*n)).collect();
            key.sort_unstable();
            by_position.entry(key).or_default().push(*f);
        }
        cracks = by_position.into_values().filter(|g| g.len() > 1).flatten().collect();
        cracks.sort_unstable();
    }
    for (_, f) in &free {
        if let Some(e) = model.element(f.element)
            && let Some(local) = e.kind.faces().get(usize::from(f.face) - 1)
        {
            facets.push(corner_cycle(e.kind, &e.nodes, local));
        }
    }
    let mut edge_count: BTreeMap<[u32; 2], usize> = BTreeMap::new();
    for cycle in &facets {
        for (i, &a) in cycle.iter().enumerate() {
            let b = cycle.get((i + 1) % cycle.len()).copied().unwrap_or(a);
            *edge_count.entry([a.min(b), a.max(b)]).or_default() += 1;
        }
    }
    let free_edges = edge_count.into_iter().filter(|(_, n)| *n == 1).map(|(e, _)| e).collect();
    Ok(Duplicates { coincident, elements, free_faces: free.len(), cracks, free_edges })
}

/// The corner nodes of a face (local node numbers `local`, corners first) in order round it.
fn corner_cycle(kind: ElementKind, nodes: &[u32], local: &[usize]) -> Vec<u32> {
    let corners = kind.corner_count();
    local.iter().filter(|&&i| i < corners).filter_map(|&i| nodes.get(i).copied()).collect()
}

/// The nodes in reverse Cuthill–McKee order: each connected part of the mesh from a
/// pseudo-peripheral node, breadth first with neighbours by increasing degree (then label), the
/// whole order reversed. Nodes no element uses come last, by label.
pub fn reverse_cuthill_mckee(model: &Model) -> Vec<u32> {
    let labels: Vec<u32> = model.nodes().keys().copied().collect();
    let index: BTreeMap<u32, usize> = labels.iter().enumerate().map(|(i, &l)| (l, i)).collect();
    let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); labels.len()];
    for e in model.elements().values() {
        let ids: Vec<usize> = e.nodes.iter().filter_map(|n| index.get(n).copied()).collect();
        for &a in &ids {
            for &b in &ids {
                if a != b
                    && let Some(s) = adj.get_mut(a)
                {
                    s.insert(b);
                }
            }
        }
    }
    let degree = |v: usize| adj.get(v).map_or(0, BTreeSet::len);
    let mut placed = vec![false; labels.len()];
    let mut order: Vec<usize> = Vec::with_capacity(labels.len());
    for start in 0..labels.len() {
        if placed.get(start).copied().unwrap_or(true) || degree(start) == 0 {
            continue;
        }
        let root = pseudo_peripheral(&adj, start);
        let mut queue = VecDeque::from([root]);
        if let Some(p) = placed.get_mut(root) {
            *p = true;
        }
        while let Some(v) = queue.pop_front() {
            order.push(v);
            let mut next: Vec<usize> =
                adj.get(v).map(|s| s.iter().copied().filter(|&w| !placed.get(w).copied().unwrap_or(true)).collect()).unwrap_or_default();
            next.sort_by_key(|&w| (degree(w), w));
            for w in next {
                if let Some(p) = placed.get_mut(w) {
                    *p = true;
                }
                queue.push_back(w);
            }
        }
    }
    order.reverse();
    let mut out: Vec<u32> = order.into_iter().filter_map(|i| labels.get(i).copied()).collect();
    out.extend(labels.iter().enumerate().filter(|(i, _)| !placed.get(*i).copied().unwrap_or(false)).map(|(_, &l)| l));
    out
}

/// A node of `start`'s component far from the others: repeatedly the lowest-degree node of the
/// last level of a breadth-first level structure, while the structure gets deeper.
fn pseudo_peripheral(adj: &[BTreeSet<usize>], start: usize) -> usize {
    let levels = |root: usize| -> Vec<Vec<usize>> {
        let mut seen = BTreeSet::from([root]);
        let mut out = vec![vec![root]];
        while let Some(last) = out.last() {
            let mut next: Vec<usize> = Vec::new();
            for &v in last {
                for &w in adj.get(v).into_iter().flatten() {
                    if seen.insert(w) {
                        next.push(w);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            out.push(next);
        }
        out
    };
    let degree = |v: usize| adj.get(v).map_or(0, BTreeSet::len);
    let mut root = start;
    let mut depth = levels(root).len();
    // Each round strictly deepens the structure, so at most as many rounds as nodes.
    for _ in 0..adj.len() {
        let Some(far) = levels(root).last().and_then(|l| l.iter().copied().min_by_key(|&v| (degree(v), v))) else { break };
        let d = levels(far).len();
        if d <= depth {
            break;
        }
        (root, depth) = (far, d);
    }
    root
}

/// The mesh's bandwidth with node labels `label(old)`: the largest label difference between two
/// nodes of one element.
pub fn bandwidth(model: &Model, label: &dyn Fn(u32) -> u32) -> u32 {
    model
        .elements()
        .values()
        .map(|e| {
            let ls = e.nodes.iter().map(|&n| label(n));
            let (lo, hi) = ls.fold((u32::MAX, 0), |(lo, hi), l| (lo.min(l), hi.max(l)));
            hi.saturating_sub(lo)
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hex8 unit cubes along x, each with its own nodes (cube `k` from x = k to k + 1), so the
    /// four nodes of each joint coincide.
    fn cubes(n: u32) -> Model {
        let mut m = Model::default();
        let mut label = 0;
        for e in 0..n {
            let first = label + 1;
            for [x, y, z] in [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]] {
                label += 1;
                m.set_node(label, [f64::from(e) + x, y, z]).unwrap();
            }
            m.set_element(e + 1, ElementKind::Hex8, (first..first + 8).collect()).unwrap();
        }
        m
    }

    #[test]
    fn a_merge_keeps_the_lowest_or_highest_and_leaves_out_collapses() {
        let m = cubes(2);
        let low = merge_plan(&m, None, 1e-9, Keep::Lowest).unwrap();
        assert_eq!(low.map, [(9, 2), (12, 3), (13, 6), (16, 7)].into());
        let high = merge_plan(&m, None, 1e-9, Keep::Highest).unwrap();
        assert_eq!(high.map, [(2, 9), (3, 12), (6, 13), (7, 16)].into());
        assert!(low.refused.is_empty() && high.refused.is_empty());
        // A third element on nodes 2 and 9 together: their merge would collapse it.
        let mut odd = m.clone();
        odd.set_node(17, [5.0, 5.0, 5.0]).unwrap();
        odd.set_element(3, ElementKind::Rod2, vec![2, 9]).unwrap();
        let plan = merge_plan(&odd, None, 1e-9, Keep::Lowest).unwrap();
        assert_eq!(plan.refused, vec![Refused { node: 9, survivor: 2, element: 3 }]);
        assert_eq!(plan.map.len(), 3);
        odd.merge_nodes(&plan.map).unwrap();
    }

    #[test]
    fn duplicates_find_unmerged_nodes_repeated_elements_and_cracks() {
        let mut m = cubes(2);
        m.set_element(3, ElementKind::Hex8, (1..=8).collect()).unwrap();
        let d = duplicates(&m, 1e-9).unwrap();
        assert_eq!(d.coincident.len(), 4);
        assert_eq!(d.elements, vec![vec![1, 3]]);
        // Cubes 1 and 3 share every face; cube 2's six are free, and its face on x = 1 lies on
        // cube 1's (now shared with cube 3, so not free): no crack. Closed skin: no free edge.
        assert_eq!(d.free_faces, 6);
        assert!(d.cracks.is_empty() && d.free_edges.is_empty(), "{d:?}");
        let d = duplicates(&cubes(2), 1e-9).unwrap();
        assert_eq!(d.free_faces, 12);
        assert_eq!(d.cracks, vec![ElementFace { element: 1, face: 4 }, ElementFace { element: 2, face: 6 }]);
        // A lone quad: its four sides are free edges.
        let mut q = Model::default();
        for (l, p) in [(1, [0., 0., 0.]), (2, [1., 0., 0.]), (3, [1., 1., 0.]), (4, [0., 1., 0.])] {
            q.set_node(l, p).unwrap();
        }
        q.set_element(1, ElementKind::Quad4, vec![1, 2, 3, 4]).unwrap();
        assert_eq!(duplicates(&q, 1e-9).unwrap().free_edges.len(), 4);
    }

    /// Reverse Cuthill–McKee brings the bandwidth of a strip whose labels were scattered back
    /// down to what its connectivity needs.
    #[test]
    fn reverse_cuthill_mckee_narrows_the_bandwidth() {
        let mut m = cubes(6);
        let merged = merge_plan(&m, None, 1e-9, Keep::Lowest).unwrap();
        m.merge_nodes(&merged.map).unwrap();
        // Scatter the labels: node l becomes (l · 7919) mod 1009 + 1.
        let scatter: BTreeMap<u32, u32> = m.nodes().keys().map(|&l| (l, (l * 7919) % 1009 + 1)).collect();
        m.renumber(&scatter, &BTreeMap::new()).unwrap();
        let before = bandwidth(&m, &|l| l);
        let order = reverse_cuthill_mckee(&m);
        assert_eq!(order.len(), m.nodes().len());
        let new: BTreeMap<u32, u32> = order.iter().enumerate().map(|(i, &l)| (l, i as u32 + 1)).collect();
        let after = bandwidth(&m, &|l| new.get(&l).copied().unwrap_or(l));
        // A strip of cubes: each element spans two layers of four nodes.
        assert!(after <= 7 && after < before, "{before} → {after}");
        // An unused node goes last.
        m.set_node(5000, [9.0, 9.0, 9.0]).unwrap();
        assert_eq!(reverse_cuthill_mckee(&m).last(), Some(&5000));
    }
}
