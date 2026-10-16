//! Geometry check before meshing: the features of a body that are small against the element
//! size an analyst intends to mesh it with, and so will force tiny or distorted elements.
//!
//! Four kinds, each measured on a fine tessellation of the B-rep (its edge polylines and its
//! triangles tagged by face) and compared with a fraction of the target size:
//! - **short edges**: an edge shorter than `short_edge × size`;
//! - **small faces**: a face whose area is below `small_face × size²`;
//! - **sliver faces**: a face that is not small but narrow, its mean width `2 × area / perimeter`
//!   (the width of a long strip, the radius of a disc) below `sliver × size`;
//! - **near faces**: two nearly parallel faces closer than `gap × size` without touching: a gap
//!   between bodies or cells that almost meet, faces that overlap, or a wall thinner than the gap.
//!
//! Every finding names the body, its persistent face and edge names, the measured value, the
//! limit it fell below and a point to look at. The check reads the geometry and changes nothing.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::edges::EDGE_TOLERANCE;
use crate::{Error, Result, Shape};

/// Most triangles the check looks at, over every body.
pub const MAX_TRIANGLES: usize = 1_000_000;

/// Most grid-cell insertions of the near-face search (a triangle goes into every cell its box
/// covers).
const MAX_CELL_ENTRIES: usize = 20_000_000;

/// The thresholds of a check, as fractions of the target element size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckOptions {
    /// The target element size (mm).
    pub size: f64,
    pub short_edge: f64,
    pub small_face: f64,
    pub sliver: f64,
    pub gap: f64,
    /// Faces whose normals differ from parallel (or opposite) by more than this (radians) are
    /// never near faces.
    pub parallel_angle: f64,
}

impl CheckOptions {
    /// The default thresholds for `size`.
    pub fn new(size: f64) -> Self {
        CheckOptions { size, short_edge: 0.3, small_face: 0.1, sliver: 0.3, gap: 0.3, parallel_angle: 10f64.to_radians() }
    }
}

/// What a finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    ShortEdge,
    SmallFace,
    SliverFace,
    NearFaces,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::ShortEdge, Kind::SmallFace, Kind::SliverFace, Kind::NearFaces];

    pub fn name(self) -> &'static str {
        match self {
            Kind::ShortEdge => "short_edge",
            Kind::SmallFace => "small_face",
            Kind::SliverFace => "sliver_face",
            Kind::NearFaces => "near_faces",
        }
    }
}

/// One feature below its threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub kind: Kind,
    pub body: String,
    /// The edge, for a short edge.
    pub edge: Option<String>,
    /// The face (small and sliver faces), the faces of the edge (short edges), or the two faces
    /// (near faces: the first on `body`, the second on `other_body`).
    pub faces: Vec<String>,
    /// The body of the second face, for near faces.
    pub other_body: Option<String>,
    /// The measure: length (mm), area (mm²), width (mm) or distance (mm).
    pub value: f64,
    /// The limit `value` fell below, in the same unit.
    pub limit: f64,
    /// A point on the feature to look at.
    pub at: [f64; 3],
}

/// Checks `bodies` against `options`. Findings come grouped by kind, worst first within a kind.
pub fn check(bodies: &[&Shape], options: &CheckOptions) -> Result<Vec<Finding>> {
    let o = options;
    let fractions = [o.short_edge, o.small_face, o.sliver, o.gap];
    if !(o.size > 0.0 && o.size.is_finite()) || fractions.iter().any(|f| !(*f >= 0.0 && f.is_finite())) {
        return Err(Error::Invalid("the check needs a positive size and thresholds that are finite and not negative".into()));
    }
    if !(o.parallel_angle >= 0.0 && o.parallel_angle < std::f64::consts::FRAC_PI_2) {
        return Err(Error::Invalid("the parallel angle must be at least 0 and below 90°".into()));
    }
    let mut findings = Vec::new();
    let mut triangles = Vec::new();
    let mut adjacent = BTreeSet::new();
    let mut touch = 0.0f64;
    for (b, shape) in bodies.iter().enumerate() {
        let tolerance = (shape.size()? * EDGE_TOLERANCE).max(1e-4);
        touch = touch.max(2.0 * tolerance);
        let mesh = shape.tessellate(tolerance)?;
        if triangles.len() + mesh.triangles.len() > MAX_TRIANGLES {
            return Err(Error::Invalid(format!("the bodies have more than {MAX_TRIANGLES} triangles to check")));
        }
        let edges = shape.edges_of(&mesh);
        // Short edges, and each face's perimeter (a seam, both sides on one face, bounds nothing).
        let mut perimeter: BTreeMap<u32, f64> = BTreeMap::new();
        for e in &edges {
            let length = e.length();
            let distinct: BTreeSet<u32> = e.faces.iter().copied().collect();
            if !(e.faces.len() == 2 && distinct.len() == 1) {
                for &f in &distinct {
                    *perimeter.entry(f).or_default() += length;
                }
            }
            if let [f, g] = e.faces[..] {
                adjacent.insert((b, f.min(g), f.max(g)));
            }
            let limit = o.short_edge * o.size;
            if length < limit {
                findings.push(Finding {
                    kind: Kind::ShortEdge,
                    body: shape.name.clone(),
                    edge: Some(e.name.clone()),
                    faces: distinct.iter().map(|&f| shape.face_label(f)).collect(),
                    other_body: None,
                    value: length,
                    limit,
                    at: e.point_at(0.5).unwrap_or_default(),
                });
            }
        }
        // Face areas, area centroids and the triangle centres to place them by.
        let mut faces: BTreeMap<u32, FaceSum> = BTreeMap::new();
        for (t, &f) in mesh.triangles.iter().zip(&mesh.tri_face) {
            let corner = |i: u32| mesh.positions.get(i as usize).copied();
            let (Some(p), Some(q), Some(r)) = (corner(t[0]), corner(t[1]), corner(t[2])) else { continue };
            let n = cross(sub(q, p), sub(r, p));
            let area = 0.5 * norm(n);
            let centre = [0, 1, 2].map(|k| (p[k] + q[k] + r[k]) / 3.0);
            let sum = faces.entry(f).or_default();
            sum.area += area;
            sum.moment = add(sum.moment, scale(centre, area));
            sum.centres.push(centre);
            if area > 0.0 && area.is_finite() {
                triangles.push(Tri { body: b, face: f, corners: [p, q, r], normal: scale(n, 0.5 / area) });
            }
        }
        for (&f, sum) in &faces {
            let at = sum.location();
            let small = o.small_face * o.size * o.size;
            if sum.area < small {
                findings.push(face_finding(Kind::SmallFace, shape, f, sum.area, small, at));
                continue;
            }
            let Some(p) = perimeter.get(&f).copied().filter(|p| *p > 0.0) else { continue };
            let width = 2.0 * sum.area / p;
            let limit = o.sliver * o.size;
            if width < limit {
                findings.push(face_finding(Kind::SliverFace, shape, f, width, limit, at));
            }
        }
    }
    findings.extend(near_faces(bodies, &triangles, &adjacent, touch, o)?);
    findings.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.value.total_cmp(&b.value)));
    Ok(findings)
}

fn face_finding(kind: Kind, shape: &Shape, f: u32, value: f64, limit: f64, at: [f64; 3]) -> Finding {
    Finding { kind, body: shape.name.clone(), edge: None, faces: vec![shape.face_label(f)], other_body: None, value, limit, at }
}

#[derive(Default)]
struct FaceSum {
    area: f64,
    moment: [f64; 3],
    centres: Vec<[f64; 3]>,
}

impl FaceSum {
    /// The triangle centre nearest the area centroid: a point on the face (the centroid of a
    /// curved or ring-shaped face lies off it).
    fn location(&self) -> [f64; 3] {
        let c = if self.area > 0.0 { scale(self.moment, 1.0 / self.area) } else { self.centres.first().copied().unwrap_or_default() };
        self.centres.iter().copied().min_by(|a, b| norm(sub(*a, c)).total_cmp(&norm(sub(*b, c)))).unwrap_or(c)
    }
}

struct Tri {
    body: usize,
    face: u32,
    corners: [[f64; 3]; 3],
    /// Unit normal.
    normal: [f64; 3],
}

/// Pairs of nearly parallel faces closer than the gap but farther apart than `touch` (faces
/// within it are the same surface tessellated twice, such as the shared face of two cells),
/// measured from points on each triangle to their perpendicular foot on the other face.
fn near_faces(bodies: &[&Shape], tris: &[Tri], adjacent: &BTreeSet<(usize, u32, u32)>, touch: f64, o: &CheckOptions) -> Result<Vec<Finding>> {
    let gap = o.gap * o.size;
    if gap <= touch || tris.is_empty() {
        return Ok(Vec::new());
    }
    // A uniform grid over the triangles' boxes, cells no smaller than the gap.
    let mut extents: Vec<f64> = tris.iter().map(|t| norm(sub(bbox(t).1, bbox(t).0))).collect();
    extents.sort_by(f64::total_cmp);
    let cell = extents.get(extents.len() / 2).copied().unwrap_or(gap).max(gap);
    let key = |p: [f64; 3]| p.map(|x| (x / cell).floor() as i64);
    let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    let mut entries = 0usize;
    for (i, t) in tris.iter().enumerate() {
        let (lo, hi) = bbox(t);
        let (lo, hi) = (key(lo), key(hi));
        let count = (0..3).map(|k| (hi[k] - lo[k] + 1).max(1) as usize).product::<usize>();
        entries = entries.saturating_add(count);
        if entries > MAX_CELL_ENTRIES {
            return Err(Error::Invalid("the near-face search is too large for these bodies; check them one at a time".into()));
        }
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    grid.entry([x, y, z]).or_default().push(i);
                }
            }
        }
    }
    let parallel = o.parallel_angle.cos();
    // Per face pair (the lower first): the smallest distance and where.
    let mut pairs: BTreeMap<((usize, u32), (usize, u32)), (f64, [f64; 3])> = BTreeMap::new();
    for a in tris {
        let [p, q, r] = a.corners;
        let c = [0, 1, 2].map(|k| (p[k] + q[k] + r[k]) / 3.0);
        for s in [c, mid(c, p), mid(c, q), mid(c, r)] {
            let (lo, hi) = (key(s.map(|x| x - gap)), key(s.map(|x| x + gap)));
            let mut seen = BTreeSet::new();
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        for &j in grid.get(&[x, y, z]).into_iter().flatten() {
                            let Some(b) = tris.get(j) else { continue };
                            if !seen.insert(j) || (a.body, a.face) >= (b.body, b.face) {
                                continue;
                            }
                            if a.body == b.body && adjacent.contains(&(a.body, a.face.min(b.face), a.face.max(b.face))) {
                                continue;
                            }
                            if dot(a.normal, b.normal).abs() < parallel {
                                continue;
                            }
                            let foot = closest_on_triangle(s, b.corners);
                            let d = norm(sub(s, foot));
                            // Only a perpendicular foot: a point beside the other face is not over it.
                            if d <= touch || d >= gap || dot(sub(s, foot), b.normal).abs() < 0.999 * d {
                                continue;
                            }
                            let slot = pairs.entry(((a.body, a.face), (b.body, b.face))).or_insert((d, s));
                            if d < slot.0 {
                                *slot = (d, s);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for (((ba, fa), (bb, fb)), (d, at)) in pairs {
        let (Some(sa), Some(sb)) = (bodies.get(ba), bodies.get(bb)) else { continue };
        out.push(Finding {
            kind: Kind::NearFaces,
            body: sa.name.clone(),
            edge: None,
            faces: vec![sa.face_label(fa), sb.face_label(fb)],
            other_body: Some(sb.name.clone()),
            value: d,
            limit: gap,
            at,
        });
    }
    Ok(out)
}

fn bbox(t: &Tri) -> ([f64; 3], [f64; 3]) {
    let [p, q, r] = t.corners;
    ([0, 1, 2].map(|k| p[k].min(q[k]).min(r[k])), [0, 1, 2].map(|k| p[k].max(q[k]).max(r[k])))
}

/// The point of triangle `abc` closest to `p` (by its Voronoi regions: Ericson, *Real-Time
/// Collision Detection*, 2005, §5.1.5).
fn closest_on_triangle(p: [f64; 3], [a, b, c]: [[f64; 3]; 3]) -> [f64; 3] {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add(a, scale(ab, d1 / (d1 - d3)));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add(a, scale(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return add(b, scale(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denom = va + vb + vc;
    if !(denom.abs() > 0.0) {
        return a;
    }
    add(a, add(scale(ab, vb / denom), scale(ac, vc / denom)))
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|x| x * k)
}
fn mid(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    scale(add(a, b), 0.5)
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_point_regions() {
        let t = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let near = |p, q: [f64; 3]| norm(sub(closest_on_triangle(p, t), q)) < 1e-12;
        assert!(near([0.2, 0.2, 3.0], [0.2, 0.2, 0.0]));
        assert!(near([-1.0, -1.0, 0.0], [0.0, 0.0, 0.0]));
        assert!(near([0.5, -2.0, 0.0], [0.5, 0.0, 0.0]));
        assert!(near([2.0, 2.0, 0.0], [0.5, 0.5, 0.0]));
    }

    #[test]
    fn hostile_options_are_errors() {
        let body = crate::block([0.0; 3], [1.0; 3]).unwrap();
        for o in [
            CheckOptions::new(0.0),
            CheckOptions::new(f64::NAN),
            CheckOptions { gap: -1.0, ..CheckOptions::new(1.0) },
            CheckOptions { parallel_angle: 2.0, ..CheckOptions::new(1.0) },
        ] {
            assert!(check(&[&body], &o).is_err(), "{o:?}");
        }
    }

    /// A unit cube at size 1 has nothing small; at size 10 every edge is short and every face small.
    #[test]
    fn a_cube_is_clean_at_its_own_size() {
        let body = crate::block([0.0; 3], [1.0; 3]).unwrap();
        assert!(check(&[&body], &CheckOptions::new(1.0)).unwrap().is_empty());
        let f = check(&[&body], &CheckOptions::new(10.0)).unwrap();
        assert_eq!(f.iter().filter(|f| f.kind == Kind::ShortEdge).count(), 12);
        assert_eq!(f.iter().filter(|f| f.kind == Kind::SmallFace).count(), 6);
    }

    /// Two blocks 0.1 apart face each other across the gap; a block on the other's face touches it.
    #[test]
    fn near_faces_across_a_gap_but_not_a_contact() {
        let a = crate::block([0.0; 3], [10.0; 3]).unwrap();
        let b = crate::block([0.0, 0.0, 10.1], [10.0, 10.0, 20.0]).unwrap();
        let f = check(&[&a, &b], &CheckOptions::new(2.0)).unwrap();
        let near: Vec<&Finding> = f.iter().filter(|f| f.kind == Kind::NearFaces).collect();
        assert_eq!(near.len(), 1, "{f:?}");
        assert!((near[0].value - 0.1).abs() < 1e-9 && (near[0].at[2] - 10.0).abs() < 1e-9, "{:?}", near[0]);
        let c = crate::block([0.0, 0.0, 10.0], [10.0, 10.0, 20.0]).unwrap();
        assert!(check(&[&a, &c], &CheckOptions::new(2.0)).unwrap().is_empty());
    }
}
