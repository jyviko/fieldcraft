//! The quadrilateral mesh of a swept cross-section: the scheme of every filled region of the
//! arrangement and its interval constraints, then, on the assigned counts, the nodes of every
//! curve (shared by the regions on either side) and each region's quadrilaterals.

use std::collections::HashMap;

use super::arrangement::{Arrangement, Q};
use super::interval::Intervals;
use super::region::{self, Scheme};
use crate::quad::QuadOptions;
use crate::size::SizeField;
use crate::{Error, Result};

/// A meshed cross-section: its nodes and, per region of the arrangement, its quadrilaterals and
/// the scheme that made them (free for regions not filled).
pub(crate) struct Section {
    pub nodes: Vec<Q>,
    pub quads: Vec<Vec<[u32; 4]>>,
    pub schemes: Vec<Scheme>,
}

/// The scheme of each filled region (`used`), its constraints added to `ia` (one variable per
/// curve of the arrangement, by curve index).
pub(crate) fn schemes(arr: &Arrangement, used: &[bool], ia: &mut Intervals) -> Vec<Scheme> {
    let oriented = |c: usize, fwd: bool| -> Vec<Q> {
        let mut v: Vec<Q> =
            arr.curves.get(c).map(|x| x.points.iter().map(|&p| arr.points.get(p).copied().unwrap_or([0.0; 2])).collect()).unwrap_or_default();
        if !fwd {
            v.reverse();
        }
        v
    };
    let mut schemes: Vec<Scheme> = vec![Scheme::Free; arr.regions.len()];
    for (r, region) in arr.regions.iter().enumerate() {
        if !used.get(r).copied().unwrap_or(false) {
            continue;
        }
        let polylines: Vec<Vec<Vec<Q>>> = region.loops.iter().map(|lp| lp.iter().map(|&(c, f)| oriented(c, f)).collect()).collect();
        let scheme = region::classify(&polylines);
        let vars: Vec<Vec<usize>> = region.loops.iter().map(|lp| lp.iter().map(|x| x.0).collect()).collect();
        region::constrain(ia, &vars, scheme);
        if let Some(slot) = schemes.get_mut(r) {
            *slot = scheme;
        }
    }
    schemes
}

/// Mesh the filled regions on the curve counts `counts`: `place(c, n)` gives the `n + 1` points
/// of curve `c` (its interior ones are used; its ends are the arrangement's points, shared).
/// The schemes come from [`schemes`]; a structured one whose grid would fold is meshed free.
pub(crate) fn mesh(
    arr: &Arrangement,
    used: &[bool],
    mut schemes: Vec<Scheme>,
    counts: &[usize],
    place: &dyn Fn(usize, usize) -> Result<Vec<Q>>,
    field: &dyn SizeField,
    opts: &QuadOptions,
) -> Result<Section> {
    // Nodes: curve ends, then curve interiors.
    let mut nodes: Vec<Q> = Vec::new();
    let mut point_node: HashMap<usize, u32> = HashMap::new();
    let mut curve_nodes: Vec<Vec<u32>> = Vec::with_capacity(arr.curves.len());
    for (c, curve) in arr.curves.iter().enumerate() {
        let n = counts.get(c).copied().unwrap_or(1);
        let pts = place(c, n)?;
        let mut ids = Vec::with_capacity(pts.len());
        let last = pts.len().saturating_sub(1);
        for (k, p) in pts.iter().enumerate() {
            let end = if k == 0 {
                curve.points.first()
            } else if k == last {
                curve.points.last()
            } else {
                None
            };
            let id = match end {
                Some(&pid) => *point_node.entry(pid).or_insert_with(|| {
                    nodes.push(arr.points.get(pid).copied().unwrap_or([0.0; 2]));
                    nodes.len() as u32 - 1
                }),
                None => {
                    nodes.push(*p);
                    nodes.len() as u32 - 1
                }
            };
            ids.push(id);
        }
        curve_nodes.push(ids);
    }
    let mut region_quads: Vec<Vec<[u32; 4]>> = vec![Vec::new(); arr.regions.len()];
    for (r, region) in arr.regions.iter().enumerate() {
        if !used.get(r).copied().unwrap_or(false) {
            continue;
        }
        let mut loops_ids: Vec<Vec<u32>> = Vec::with_capacity(region.loops.len());
        let mut starts: Vec<Vec<usize>> = Vec::new();
        for lp in &region.loops {
            let mut ids = Vec::new();
            let mut st = Vec::new();
            for &(c, fwd) in lp {
                st.push(ids.len());
                let mut seq: Vec<u32> = curve_nodes.get(c).cloned().unwrap_or_default();
                if !fwd {
                    seq.reverse();
                }
                seq.pop();
                ids.extend(seq);
            }
            loops_ids.push(ids);
            starts.push(st);
        }
        let at = |i: u32| nodes.get(i as usize).copied().unwrap_or([0.0; 2]);
        let loops: Vec<Vec<Q>> = loops_ids.iter().map(|l| l.iter().map(|&i| at(i)).collect()).collect();
        let scheme = schemes.get(r).copied().unwrap_or(Scheme::Free);
        let (mesh, used_scheme) = region::mesh(&loops, &starts, scheme, field, opts).map_err(|e| region_error(r, e))?;
        if let Some(slot) = schemes.get_mut(r) {
            *slot = used_scheme;
        }
        let mut local: Vec<u32> = loops_ids.concat();
        for p in mesh.nodes.iter().skip(mesh.boundary_nodes) {
            nodes.push(*p);
            local.push(nodes.len() as u32 - 1);
        }
        let quads = region_quads.get_mut(r).ok_or_else(|| Error::Failed("region index".into()))?;
        for q in &mesh.quads {
            quads.push(q.map(|i| local.get(i as usize).copied().unwrap_or(0)));
        }
    }
    Ok(Section { nodes, quads: region_quads, schemes })
}

fn region_error(r: usize, e: Error) -> Error {
    match e {
        Error::Limit(m) => Error::Limit(m),
        other => Error::Failed(format!("cross-section region {r}: {other}")),
    }
}
