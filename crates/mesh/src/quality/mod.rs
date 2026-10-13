//! Element quality: certified validity, exact volume and shape metrics, and the validity gate
//! every mesher runs before its mesh reaches the model.
//!
//! Sources (see `docs/meshing/literature.md`):
//! - Validity is a proof that `det J > 0` on the whole element, from the Jacobian determinant in
//!   Bernstein form (Johnen, Remacle, Geuzaine, J. Comput. Phys. 233:359–372, 2013). Corner
//!   Jacobians are not enough for a trilinear hexahedron (Marschner, Palmer, Zhang, Solomon,
//!   CGF 39(5):133–147, 2020); see [`jacobian`].
//! - Tetrahedron shape: Shewchuk, *What is a good linear finite element?*, 2002; Klingner and
//!   Shewchuk, *Aggressive tetrahedral mesh improvement*, IMR 2007.
//! - Hexahedron, wedge and quadrilateral shape: Knupp, *Algebraic mesh quality metrics*, SIAM J.
//!   Sci. Comput. 23(1):193–218, 2001.
//!
//! Inputs are hostile: coordinates must be finite and within [`modelcraft_model::MAX_COORD`],
//! and every metric has a defined value on degenerate and inverted elements.
//!
//! Linear tetrahedra take their validity from the exact `orient3d` ([`crate::predicates`]); the
//! other kinds from the floating-point Bernstein certificate, which counts a Jacobian within
//! [`jacobian::RELATIVE_TOLERANCE`] of zero as degenerate.

mod bernstein;
pub mod jacobian;
mod report;
pub(crate) mod shape;

use modelcraft_model::{ElementKind, check_coordinates};

pub use jacobian::{JacobianCheck, Validity};
pub use report::{GateOptions, GateReport, MetricStats, QualityReport, enclosed_volume, quality_report, validity_gate, validity_gate_elements};

use crate::predicates::{Point2, Point3, Sign, orient2d, snap_to_domain, tet_volume_sign};
use crate::{Error, Result};

/// A quality metric. Angles are in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Metric {
    /// Smallest normalized corner Jacobian, in `[-1, 1]`; 1 for a cube, a regular tetrahedron
    /// and a square. Hexahedra also measure the centre.
    ScaledJacobian,
    /// Certified lower bound of `min det J` over an upper bound of `max |det J|`, in `[-1, 1]`;
    /// 1 for affine elements. See [`JacobianCheck::ratio`].
    JacobianRatio,
    /// Knupp's shape metric of the hexahedron corners (and centre), in `[0, 1]`; 1 for a cube.
    Shape,
    /// Smallest dihedral angle of a tetrahedron (70.53° for the regular one).
    MinDihedral,
    /// Largest dihedral angle of a tetrahedron.
    MaxDihedral,
    /// Three times the inradius over the circumradius, signed by orientation; 1 for the regular
    /// tetrahedron.
    RadiusRatio,
    /// Longest edge over smallest altitude, normalized so the regular tetrahedron and the
    /// equilateral triangle score 1; `+∞` when degenerate.
    AspectRatio,
    /// `6√2 V / ℓ_rms³`, signed; 1 for the regular tetrahedron.
    VolumeLength,
    /// Longest over shortest edge; `+∞` when an edge has zero length.
    EdgeRatio,
    /// Smallest corner angle of a triangle or quadrilateral.
    MinAngle,
    /// Largest corner angle of a triangle or quadrilateral.
    MaxAngle,
    /// Skew of a quadrilateral: `|cos θ|` of the angle between its principal axes, in `[0, 1]`;
    /// 0 for a rectangle, 1 when degenerate.
    Skew,
}

impl Metric {
    pub fn name(self) -> &'static str {
        match self {
            Metric::ScaledJacobian => "scaled_jacobian",
            Metric::JacobianRatio => "jacobian_ratio",
            Metric::Shape => "shape",
            Metric::MinDihedral => "min_dihedral",
            Metric::MaxDihedral => "max_dihedral",
            Metric::RadiusRatio => "radius_ratio",
            Metric::AspectRatio => "aspect_ratio",
            Metric::VolumeLength => "volume_length",
            Metric::EdgeRatio => "edge_ratio",
            Metric::MinAngle => "min_angle",
            Metric::MaxAngle => "max_angle",
            Metric::Skew => "skew",
        }
    }

    /// Whether a larger value is a better element.
    pub fn higher_is_better(self) -> bool {
        !matches!(self, Metric::MaxDihedral | Metric::AspectRatio | Metric::EdgeRatio | Metric::MaxAngle | Metric::Skew)
    }

    /// The fixed histogram range; values outside it fall in the first or last bin.
    pub fn range(self) -> (f64, f64) {
        match self {
            Metric::ScaledJacobian | Metric::JacobianRatio | Metric::RadiusRatio | Metric::VolumeLength => (-1.0, 1.0),
            Metric::Shape | Metric::Skew => (0.0, 1.0),
            Metric::MinDihedral | Metric::MaxDihedral | Metric::MinAngle | Metric::MaxAngle => (0.0, 180.0),
            Metric::AspectRatio | Metric::EdgeRatio => (1.0, 11.0),
        }
    }
}

/// The quality of one element.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementQuality {
    pub kind: ElementKind,
    /// Signed volume of a solid (exact for curved elements), area of a shell.
    pub size: f64,
    /// The certified Jacobian check, for solids.
    pub jacobian: Option<JacobianCheck>,
    pub metrics: Vec<(Metric, f64)>,
}

impl ElementQuality {
    pub fn get(&self, metric: Metric) -> Option<f64> {
        self.metrics.iter().find(|(m, _)| *m == metric).map(|(_, v)| *v)
    }

    pub fn validity(&self) -> Option<Validity> {
        self.jacobian.map(|j| j.validity)
    }
}

/// Whether [`element_quality`] measures this kind (solids and shells; not rods or beams).
pub fn is_measured(kind: ElementKind) -> bool {
    kind.dimension() >= 2
}

/// Check the node count and coordinates, and translate the element so its first node is at the
/// origin (less cancellation far from the origin).
fn local(kind: ElementKind, xyz: &[[f64; 3]]) -> Result<Vec<[f64; 3]>> {
    if xyz.len() != kind.node_count() {
        return Err(Error::Invalid(format!("{} needs {} nodes, got {}", kind.name(), kind.node_count(), xyz.len())));
    }
    check_coordinates(xyz.as_flattened())?;
    let o = xyz.first().copied().unwrap_or([0.0; 3]);
    Ok(xyz.iter().map(|p| shape::sub(*p, o)).collect())
}

/// The first `N` points (corner nodes).
fn corners<const N: usize>(p: &[[f64; 3]]) -> Result<[[f64; 3]; N]> {
    p.get(..N).and_then(|s| <[[f64; 3]; N]>::try_from(s).ok()).ok_or_else(|| Error::Invalid(format!("need {N} corner nodes")))
}

/// The Jacobian matrix's columns as polynomials (each three coordinates).
type Columns = [[bernstein::Bernstein; 3]; 3];

/// The certified check of a solid, and its Jacobian columns when they were built. A linear
/// tetrahedron's Jacobian is constant: its sign is the exact `orient3d` of the input
/// coordinates, its volume the determinant.
fn checked(kind: ElementKind, xyz: &[[f64; 3]], p: &[[f64; 3]]) -> Result<(Option<Columns>, JacobianCheck)> {
    if kind == ElementKind::Tet4
        && let Some(sign) = exact_tet_sign(xyz)
    {
        let [a, b, c, d]: [[f64; 3]; 4] = corners(p)?;
        let volume = shape::det(shape::sub(b, a), shape::sub(c, a), shape::sub(d, a)) / 6.0;
        let (validity, ratio) = match sign {
            Sign::Positive => (Validity::Valid, 1.0),
            Sign::Negative => (Validity::Invalid, -1.0),
            Sign::Zero => (Validity::Invalid, 0.0),
        };
        return Ok((None, JacobianCheck { validity, ratio, volume }));
    }
    let jacobian::Jacobian { columns, det } = jacobian::jacobian(kind, p)?;
    Ok((Some(columns), jacobian::certify(det)?))
}

/// The exact orientation of a tetrahedron (positive for node 4 on the side of face 1-2-3's
/// right-hand normal), or `None` when a coordinate is outside the predicates' domain.
fn exact_tet_sign(xyz: &[[f64; 3]]) -> Option<Sign> {
    let p = |i: usize| xyz.get(i).and_then(|c| Point3::try_from(*c).ok());
    Some(tet_volume_sign(&p(0)?, &p(1)?, &p(2)?, &p(3)?))
}

/// The certified Jacobian check and exact volume of a solid element (tet4, tet10, wedge6, hex8,
/// hex20) from its node coordinates in node order.
pub fn element_validity(kind: ElementKind, xyz: &[[f64; 3]]) -> Result<JacobianCheck> {
    let p = local(kind, xyz)?;
    checked(kind, xyz, &p).map(|(_, check)| check)
}

/// Knupp's scaled Jacobian and shape of a hexahedron: the Jacobian matrix at the eight corners
/// and the centre (hex20 uses its curved map).
fn hex_metrics(columns: &Columns) -> (f64, f64) {
    let at = |value: &dyn Fn(&bernstein::Bernstein) -> f64| {
        let [a, b, c] = columns.each_ref().map(|col| col.each_ref().map(value));
        shape::jacobian_metrics(a, b, c)
    };
    let half: &[f64] = &[0.5, 0.5];
    let centre = at(&|q| q.eval(&[half, half, half]));
    jacobian::HEX_CORNERS.iter().map(|corner| at(&|q| q.at_vertex(corner))).fold(centre, |(s, k), (s2, k2)| (s.min(s2), k.min(k2)))
}

/// Every metric of one element from its node coordinates in node order. Quadratic elements are
/// certified on their curved geometry; tet10 and wedge shape metrics are those of their corners.
/// Rods and beams are an error ([`is_measured`]).
pub fn element_quality(kind: ElementKind, xyz: &[[f64; 3]]) -> Result<ElementQuality> {
    let p = local(kind, xyz)?;
    let mut metrics = match kind {
        ElementKind::Tri3 | ElementKind::Tri6 => {
            let (area, metrics) = shape::tri(corners(&p)?);
            return Ok(ElementQuality { kind, size: area, jacobian: None, metrics });
        }
        ElementKind::Quad4 | ElementKind::Quad8 => {
            let (area, metrics) = shape::quad(corners(&p)?);
            return Ok(ElementQuality { kind, size: area, jacobian: None, metrics });
        }
        ElementKind::Rod2 | ElementKind::Beam2 => return Err(Error::Invalid(format!("quality metrics are not defined for {}", kind.name()))),
        ElementKind::Tet4 | ElementKind::Tet10 => shape::tet(corners(&p)?),
        ElementKind::Wedge6 => {
            let c: [[f64; 3]; 6] = corners(&p)?;
            let edges = [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (0, 3), (1, 4), (2, 5)];
            vec![(Metric::ScaledJacobian, shape::wedge_scaled_jacobian(c)), (Metric::EdgeRatio, shape::edge_ratio(&c, &edges))]
        }
        ElementKind::Hex8 | ElementKind::Hex20 => vec![(Metric::EdgeRatio, shape::edge_ratio(&corners::<8>(&p)?, &jacobian::HEX_EDGES))],
    };
    let (columns, check) = checked(kind, xyz, &p)?;
    if let Some(columns) = columns.as_ref().filter(|_| matches!(kind, ElementKind::Hex8 | ElementKind::Hex20)) {
        let (scaled, knupp) = hex_metrics(columns);
        metrics.extend([(Metric::ScaledJacobian, scaled), (Metric::Shape, knupp)]);
    }
    metrics.push((Metric::JacobianRatio, check.ratio));
    Ok(ElementQuality { kind, size: check.volume, jacobian: Some(check), metrics })
}

/// The exact validity of a planar bilinear quadrilateral with corners `p` in counterclockwise
/// order. Its Jacobian determinant is affine in each reference coordinate (the `ξη` term of a
/// planar bilinear map cancels), so it is positive on the whole element exactly when it is
/// positive at the four corners, i.e. when every corner turns left: `orient2d(p[i-1], p[i],
/// p[i+1]) > 0`, decided by the exact predicate. A zero corner (degenerate) or a negative one
/// (concave, bow-tie or clockwise) is [`Validity::Invalid`]. Coordinates below the predicates'
/// smallest magnitude are rounded to zero first.
pub fn planar_quad_validity(p: &[[f64; 2]; 4]) -> Result<Validity> {
    check_coordinates(p.as_flattened())?;
    let q = p.map(|c| Point2::new(snap_to_domain(c[0]), snap_to_domain(c[1])));
    let mut pts = Vec::with_capacity(4);
    for c in q {
        pts.push(c?);
    }
    let ok = (0..4).all(|i| match (pts.get((i + 3) % 4), pts.get(i), pts.get((i + 1) % 4)) {
        (Some(a), Some(b), Some(c)) => orient2d(a, b, c) == Sign::Positive,
        _ => false,
    });
    Ok(if ok { Validity::Valid } else { Validity::Invalid })
}

/// Every metric of a planar quadrilateral (counterclockwise corners in the xy plane) with its
/// exact validity ([`planar_quad_validity`]): the Jacobian ratio is the smallest corner
/// determinant over the largest magnitude (exact bounds, see there) and the size is the signed
/// area. The shape metrics are those of [`element_quality`] for a `Quad4`.
pub fn planar_quad_quality(p: &[[f64; 2]; 4]) -> Result<ElementQuality> {
    let validity = planar_quad_validity(p)?;
    let o = p[0];
    let l = p.map(|c| [c[0] - o[0], c[1] - o[1]]);
    let corner = |i: usize| {
        let (a, b, c) = (l[(i + 3) % 4], l[i], l[(i + 1) % 4]);
        (c[0] - b[0]) * (a[1] - b[1]) - (c[1] - b[1]) * (a[0] - b[0])
    };
    let dets = [0, 1, 2, 3].map(corner);
    let lo = dets.iter().fold(f64::INFINITY, |m, &d| m.min(d));
    let hi = dets.iter().fold(0.0_f64, |m, &d| m.max(d.abs()));
    let ratio = if hi > 0.0 { (lo / hi).clamp(-1.0, 1.0) } else { 0.0 };
    let area = 0.5 * ((l[2][0] - l[0][0]) * (l[3][1] - l[1][1]) - (l[2][1] - l[0][1]) * (l[3][0] - l[1][0]));
    let lifted = l.map(|c| [c[0], c[1], 0.0]);
    let (_, mut metrics) = shape::quad(lifted);
    // Corner scaled Jacobians measured against +z (the 3D form measures against the diagonals'
    // normal, which flips with a clockwise quad).
    let scaled = [0, 1, 2, 3]
        .map(|i| {
            let (a, b, c) = (l[(i + 3) % 4], l[i], l[(i + 1) % 4]);
            let (u, v) = ([c[0] - b[0], c[1] - b[1]], [a[0] - b[0], a[1] - b[1]]);
            let den = (u[0] * u[0] + u[1] * u[1]).sqrt() * (v[0] * v[0] + v[1] * v[1]).sqrt();
            let r = corner(i) / den;
            if den > 0.0 && r.is_finite() { r } else { 0.0 }
        })
        .iter()
        .fold(f64::INFINITY, |m, &x| m.min(x));
    for (m, v) in metrics.iter_mut() {
        if *m == Metric::ScaledJacobian {
            *v = scaled;
        }
    }
    metrics.push((Metric::JacobianRatio, ratio));
    Ok(ElementQuality { kind: ElementKind::Quad4, size: area, jacobian: Some(JacobianCheck { validity, ratio, volume: area }), metrics })
}

#[cfg(test)]
mod tests;
