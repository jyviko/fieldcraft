//! Independent checks of a triangulation: they re-derive every property from the arrays with
//! the exact predicates and share no code with the insertion.

use super::{DEAD, GHOST, NONE, Triangulation, volume_sign};
use crate::predicates::{Point3, Sign, insphere, insphere_sos};
use crate::{Error, Result};

fn failed<T>(what: String) -> Result<T> {
    Err(Error::Failed(format!("Delaunay verification: {what}")))
}

/// `det[b − a, c − a, d − a] / 6` in floating point (a measure for the volume balance only).
pub(super) fn volume(p: [&Point3; 4]) -> f64 {
    let [a, b, c, d] = p.map(Point3::coords);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let w = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
    (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0]) + u[2] * (v[0] * w[1] - v[1] * w[0])) / 6.0
}

/// The face of `vs` opposite slot `k`, sorted.
fn face(vs: [u32; 4], k: usize) -> [u32; 3] {
    let mut f = [NONE; 3];
    for (o, (_, &v)) in f.iter_mut().zip(vs.iter().enumerate().filter(|&(i, _)| i != k)) {
        *o = v;
    }
    f.sort_unstable();
    f
}

impl Triangulation {
    fn point_of(&self, v: u32) -> Result<&Point3> {
        match self.points.get(v as usize) {
            Some(p) => Ok(p),
            None => failed(format!("vertex {v} does not exist")),
        }
    }

    fn points_with<'a>(&'a self, vs: [u32; 4], k: usize, p: &'a Point3) -> Result<[&'a Point3; 4]> {
        let mut out = [p; 4];
        for (i, o) in out.iter_mut().enumerate() {
            if i != k {
                *o = self.point_of(vs.get(i).copied().unwrap_or(NONE))?;
            }
        }
        Ok(out)
    }

    /// The vertex of `o` across its shared face with `t`, and the slot of that face in `o`.
    fn across(&self, t: u32, o: u32) -> Result<([u32; 4], usize)> {
        let Some(ws) = self.tet(o) else { return failed(format!("tetrahedron {t} has a deleted neighbour {o}")) };
        let adj = self.adj.get(o as usize).copied().unwrap_or([NONE; 4]);
        let back: Vec<usize> = (0..4).filter(|&k| adj.get(k) == Some(&t)).collect();
        match back.as_slice() {
            [k] => Ok((ws, *k)),
            _ => failed(format!("tetrahedra {t} and {o} are not mutual neighbours exactly once")),
        }
    }

    /// Check the triangulation: adjacency is mutual and matches shared faces, every finite
    /// tetrahedron has positive volume, the hull is convex, every interior face is locally
    /// Delaunay (the opposite vertex is not inside the circumsphere, exactly, and is outside
    /// with the perturbation), every vertex is in a tetrahedron, and the finite volume equals the
    /// volume the hull faces enclose. By Delaunay's lemma, local Delaunay faces in a
    /// triangulation of the hull make every circumsphere empty. `O(tetrahedra)`.
    pub fn verify(&self) -> Result<()> {
        let n = self.points.len();
        let mut seen = vec![false; n];
        let (mut finite_volume, mut hull_volume, mut scale) = (0.0f64, 0.0f64, 0.0f64);
        let reference = self.tets.iter().flatten().copied().find(|&v| v < DEAD);
        for (t, &vs) in self.tets.iter().enumerate() {
            if vs[0] == DEAD {
                continue;
            }
            let t = t as u32;
            let adj = self.adj.get(t as usize).copied().unwrap_or([NONE; 4]);
            for (i, &v) in vs.iter().enumerate() {
                if vs.iter().skip(i + 1).any(|&w| w == v) {
                    return failed(format!("tetrahedron {t} repeats vertex {v}"));
                }
                if v == GHOST {
                    continue;
                }
                if self.duplicate_of(v).is_some() {
                    return failed(format!("duplicate point {v} is a vertex"));
                }
                match seen.get_mut(v as usize) {
                    Some(s) => *s = true,
                    None => return failed(format!("tetrahedron {t} has vertex {v}, which does not exist")),
                }
            }
            for (j, &o) in adj.iter().enumerate() {
                if o == t {
                    return failed(format!("tetrahedron {t} is its own neighbour"));
                }
                let (ws, k) = self.across(t, o)?;
                if face(vs, j) != face(ws, k) {
                    return failed(format!("tetrahedra {t} and {o} are neighbours without sharing a face"));
                }
            }
            match vs.iter().position(|&v| v == GHOST) {
                None => {
                    let p = self.points_with(vs, 4, self.point_of(vs[0])?)?;
                    if volume_sign(p) != Sign::Positive {
                        return failed(format!("tetrahedron {t} is flat or inverted"));
                    }
                    let vol = volume(p);
                    finite_volume += vol;
                    scale += vol.abs();
                    let [a, b, c, d] = p;
                    for (j, &o) in adj.iter().enumerate() {
                        let (ws, k) = self.across(t, o)?;
                        let apex = ws.get(k).copied().unwrap_or(NONE);
                        if apex == GHOST {
                            continue;
                        }
                        let e = self.point_of(apex)?;
                        if insphere(b, a, c, d, e) == Sign::Positive {
                            return failed(format!("vertex {apex} is inside the circumsphere of tetrahedron {t} (face {j})"));
                        }
                        if insphere_sos(b, a, c, d, e) != Sign::Negative {
                            return failed(format!("vertex {apex} is inside the perturbed circumsphere of tetrahedron {t}"));
                        }
                    }
                }
                Some(g) => {
                    if vs.iter().filter(|&&v| v == GHOST).count() != 1 {
                        return failed(format!("tetrahedron {t} has more than one ghost vertex"));
                    }
                    let finite = adj.get(g).copied().unwrap_or(NONE);
                    let (fs, fk) = self.across(t, finite)?;
                    if fs.contains(&GHOST) {
                        return failed(format!("hull face of {t} borders another ghost"));
                    }
                    let inner = self.point_of(fs.get(fk).copied().unwrap_or(NONE))?;
                    if volume_sign(self.points_with(vs, g, inner)?) != Sign::Negative {
                        return failed(format!("ghost tetrahedron {t} faces into the hull"));
                    }
                    for (j, &o) in adj.iter().enumerate() {
                        if j == g {
                            continue;
                        }
                        let (ws, k) = self.across(t, o)?;
                        let apex = self.point_of(ws.get(k).copied().unwrap_or(NONE))?;
                        match volume_sign(self.points_with(vs, g, apex)?) {
                            Sign::Positive => return failed(format!("the hull is not convex at ghost tetrahedron {t}")),
                            Sign::Negative => {}
                            Sign::Zero => {
                                // Coplanar hull faces: the perturbed decision of the finite
                                // tetrahedron under this face.
                                let [a, b, c, d] = self.points_with(fs, 4, inner)?;
                                if insphere_sos(b, a, c, d, apex) != Sign::Negative {
                                    return failed(format!("coplanar hull faces at ghost tetrahedron {t} are not perturbed-Delaunay"));
                                }
                            }
                        }
                    }
                    if let Some(r) = reference {
                        hull_volume -= volume(self.points_with(vs, g, self.point_of(r)?)?);
                    }
                }
            }
        }
        if self.dimension() == 3 {
            for (v, s) in seen.iter().enumerate() {
                let v = v as u32;
                let triangulated = self.duplicate_of(v).is_none();
                if *s != triangulated {
                    return failed(format!("vertex {v} is {}in a tetrahedron", if *s { "" } else { "not " }));
                }
                if triangulated && !self.incident_tet(v).and_then(|t| self.tet(t)).is_some_and(|vs| vs.contains(&v)) {
                    return failed(format!("vertex {v} has no valid incident tetrahedron"));
                }
            }
        } else if self.tets.iter().any(|vs| vs[0] != DEAD) {
            return failed("a lower-dimensional triangulation has tetrahedra".into());
        }
        if (finite_volume - hull_volume).abs() > 1e-9 * scale {
            return failed(format!("the tetrahedra fill {finite_volume}, the hull encloses {hull_volume}"));
        }
        Ok(())
    }

    /// Brute force: no triangulated vertex is inside any circumsphere (exactly) or the perturbed
    /// circumsphere, and none is beyond a hull face. `O(points × tetrahedra)`; for tests.
    pub fn verify_empty_spheres(&self) -> Result<()> {
        let vertices: Vec<u32> = (0..self.points.len() as u32).filter(|&v| self.duplicate_of(v).is_none()).collect();
        for (t, &vs) in self.tets.iter().enumerate() {
            if vs[0] == DEAD {
                continue;
            }
            for &u in &vertices {
                if vs.contains(&u) {
                    continue;
                }
                let e = self.point_of(u)?;
                match vs.iter().position(|&v| v == GHOST) {
                    None => {
                        let [a, b, c, d] = self.points_with(vs, 4, e)?;
                        if insphere(b, a, c, d, e) == Sign::Positive || insphere_sos(b, a, c, d, e) != Sign::Negative {
                            return failed(format!("vertex {u} is inside the circumsphere of tetrahedron {t}"));
                        }
                    }
                    Some(g) => {
                        if volume_sign(self.points_with(vs, g, e)?) == Sign::Positive {
                            return failed(format!("vertex {u} is beyond the hull face of ghost tetrahedron {t}"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
