//! Quadratic tetrahedra (tet10): a node on every edge, at the midpoint inside the domain and on
//! the geometry on the boundary — projected onto the edge's feature curve, or onto the B-rep face
//! of a boundary edge inside one face — so curved boundaries are represented to second order.
//!
//! A curved edge can make an element invalid (a thin element against a strongly curved face).
//! Every element is checked with the certified Jacobian test that the validity gate uses
//! ([`crate::quality::element_validity`], which proves FieldCraft's criterion on the whole
//! element); an element that fails gets its curved edges straightened (the node goes back to the
//! straight midpoint, for every element sharing that edge) and the elements around are checked
//! again. Straightening only ever moves nodes back to the linear mesh, which is valid, so the loop
//! ends; it is also capped by the number of edges.

use fieldcraft_model::ElementKind;

use crate::quality::{Validity, element_validity};
use crate::{Error, Result};

use super::TaggedFace;
use super::complex::{IdMap, NONE, face_toward, sorted2};
use super::geometry::{BoundaryGeometry, Tessellation};
use super::measure;

/// Tet10 mid-side nodes on edges 1-2, 2-3, 3-1, 1-4, 2-4, 3-4 (the model's node order).
use crate::quality::jacobian::TET_EDGES as MID_EDGES;

/// The quadratic mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Quadratic {
    pub points: Vec<[f64; 3]>,
    pub elements: Vec<Vec<u32>>,
    /// Edges left straight to keep their elements valid.
    pub straightened: usize,
}

/// Tet10 elements from tet4 ones (see the module docs).
pub fn tet10(
    points: &[[f64; 3]],
    tets: &[[u32; 4]],
    boundary: &[TaggedFace],
    curves: &IdMap<(u32, u32), u32>,
    curve_faces: &[[u32; 2]],
    geometry: &Tessellation,
) -> Result<Quadratic> {
    // Boundary edges and the B-rep face they lie on.
    let mut edge_face: IdMap<(u32, u32), u32> = IdMap::default();
    for b in boundary {
        let t = tets.get(b.tet as usize).ok_or_else(|| Error::Failed("tet10: boundary face of no tetrahedron".into()))?;
        let f = face_toward(*t, usize::from(b.opposite));
        for i in 0..3 {
            edge_face.entry(sorted2(f[i], f[(i + 1) % 3])).or_insert(b.face);
        }
    }
    let p = |v: u32| points.get(v as usize).copied().ok_or_else(|| Error::Failed("tet10: vertex id".into()));
    let mut out = points.to_vec();
    // Edge -> (node, straight midpoint).
    let mut node: IdMap<(u32, u32), (u32, [f64; 3])> = IdMap::default();
    let mut elements = Vec::with_capacity(tets.len());
    for t in tets {
        let mut e = t.to_vec();
        for &(i, j) in &MID_EDGES {
            let key = sorted2(t[i], t[j]);
            let id = match node.get(&key) {
                Some(&(id, _)) => id,
                None => {
                    let m = measure::mid(p(t[i])?, p(t[j])?);
                    let curved = match curves.get(&key) {
                        Some(&c) => curve_faces.get(c as usize).and_then(|f| geometry.project_curve(*f, m)),
                        None => edge_face.get(&key).and_then(|&f| geometry.project_face(f, m)),
                    };
                    let id = u32::try_from(out.len()).map_err(|_| Error::Limit("too many nodes".into()))?;
                    out.push(curved.filter(|q| q.iter().all(|x| x.is_finite())).unwrap_or(m));
                    node.insert(key, (id, m));
                    id
                }
            };
            e.push(id);
        }
        elements.push(e);
    }
    // Straighten what makes an element invalid.
    let coords = |out: &[[f64; 3]], e: &[u32]| -> Result<Vec<[f64; 3]>> {
        e.iter().map(|&v| out.get(v as usize).copied().ok_or_else(|| Error::Failed("tet10: node id".into()))).collect()
    };
    let mut straightened = 0;
    let mut check: Vec<usize> = (0..elements.len()).collect();
    let mut edge_elements: IdMap<(u32, u32), Vec<usize>> = IdMap::default();
    for (n, t) in tets.iter().enumerate() {
        for &(i, j) in &MID_EDGES {
            edge_elements.entry(sorted2(t[i], t[j])).or_default().push(n);
        }
    }
    for _ in 0..=node.len() {
        let mut next = Vec::new();
        for &n in &check {
            let Some(e) = elements.get(n) else { continue };
            if element_validity(ElementKind::Tet10, &coords(&out, e)?)?.validity == Validity::Valid {
                continue;
            }
            let t = tets.get(n).copied().unwrap_or([NONE; 4]);
            for &(i, j) in &MID_EDGES {
                let key = sorted2(t[i], t[j]);
                let Some(&(id, m)) = node.get(&key) else { continue };
                if out.get(id as usize) != Some(&m) {
                    if let Some(s) = out.get_mut(id as usize) {
                        *s = m;
                    }
                    straightened += 1;
                    next.extend(edge_elements.get(&key).cloned().unwrap_or_default());
                }
            }
        }
        next.sort_unstable();
        next.dedup();
        if next.is_empty() {
            break;
        }
        check = next;
    }
    Ok(Quadratic { points: out, elements, straightened })
}
