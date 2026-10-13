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

mod bernstein;
pub mod jacobian;
mod report;
mod shape;

use modelcraft_model::{ElementKind, MAX_COORD};

pub use jacobian::{JacobianCheck, Validity};
pub use report::{GateOptions, GateReport, MetricStats, QualityReport, enclosed_volume, quality_report, validity_gate};

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
}

impl Metric {
    pub const ALL: [Metric; 11] = [
        Metric::ScaledJacobian,
        Metric::JacobianRatio,
        Metric::Shape,
        Metric::MinDihedral,
        Metric::MaxDihedral,
        Metric::RadiusRatio,
        Metric::AspectRatio,
        Metric::VolumeLength,
        Metric::EdgeRatio,
        Metric::MinAngle,
        Metric::MaxAngle,
    ];

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
        }
    }

    /// Whether a larger value is a better element.
    pub fn higher_is_better(self) -> bool {
        !matches!(self, Metric::MaxDihedral | Metric::AspectRatio | Metric::EdgeRatio | Metric::MaxAngle)
    }

    /// The fixed histogram range; values outside it fall in the first or last bin.
    pub fn range(self) -> (f64, f64) {
        match self {
            Metric::ScaledJacobian | Metric::JacobianRatio | Metric::RadiusRatio | Metric::VolumeLength => (-1.0, 1.0),
            Metric::Shape => (0.0, 1.0),
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
    if let Some(c) = xyz.iter().flatten().find(|c| !(c.abs() <= MAX_COORD)) {
        return Err(Error::Invalid(format!("coordinate {c} is not a finite number within ±{MAX_COORD}")));
    }
    let o = xyz.first().copied().unwrap_or([0.0; 3]);
    Ok(xyz.iter().map(|p| shape::sub(*p, o)).collect())
}

/// The first `N` points (corner nodes).
fn corners<const N: usize>(p: &[[f64; 3]]) -> Result<[[f64; 3]; N]> {
    p.get(..N).and_then(|s| <[[f64; 3]; N]>::try_from(s).ok()).ok_or_else(|| Error::Invalid(format!("need {N} corner nodes")))
}

/// The certified Jacobian check and exact volume of a solid element (tet4, tet10, wedge6, hex8,
/// hex20) from its node coordinates in node order.
pub fn element_validity(kind: ElementKind, xyz: &[[f64; 3]]) -> Result<JacobianCheck> {
    let p = local(kind, xyz)?;
    jacobian::certify(&jacobian::jacobian(kind, &p)?.det)
}

/// Every metric of one element from its node coordinates in node order. Quadratic elements are
/// certified on their curved geometry; their shape metrics are those of their corners (hex20
/// uses the curved map's corner Jacobians). Rods and beams are an error ([`is_measured`]).
pub fn element_quality(kind: ElementKind, xyz: &[[f64; 3]]) -> Result<ElementQuality> {
    let p = local(kind, xyz)?;
    let solid = |metrics: Vec<(Metric, f64)>| -> Result<ElementQuality> {
        let check = jacobian::certify(&jacobian::jacobian(kind, &p)?.det)?;
        let mut metrics = metrics;
        metrics.push((Metric::JacobianRatio, check.ratio));
        Ok(ElementQuality { kind, size: check.volume, jacobian: Some(check), metrics })
    };
    match kind {
        ElementKind::Tet4 | ElementKind::Tet10 => solid(shape::tet(corners(&p)?)),
        ElementKind::Wedge6 => {
            let c: [[f64; 3]; 6] = corners(&p)?;
            let edges = [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (0, 3), (1, 4), (2, 5)];
            solid(vec![(Metric::ScaledJacobian, shape::wedge_scaled_jacobian(c)), (Metric::EdgeRatio, shape::edge_ratio(&c, &edges))])
        }
        ElementKind::Hex8 | ElementKind::Hex20 => {
            let j = jacobian::jacobian(kind, &p)?;
            // Knupp's metrics of the Jacobian matrix at the eight corners and the centre.
            let at = |value: &dyn Fn(&bernstein::Bernstein) -> f64| {
                let [a, b, c] = j.columns.each_ref().map(|col| col.each_ref().map(value));
                shape::jacobian_metrics(a, b, c)
            };
            let half: &[f64] = &[0.5, 0.5];
            let mut samples: Vec<(f64, f64)> = jacobian::HEX_CORNERS.iter().map(|corner| at(&|q| q.at_vertex(corner))).collect();
            samples.push(at(&|q| q.eval(&[half, half, half])));
            let scaled = samples.iter().fold(f64::INFINITY, |m, s| m.min(s.0));
            let knupp = samples.iter().fold(f64::INFINITY, |m, s| m.min(s.1));
            let c: [[f64; 3]; 8] = corners(&p)?;
            let check = jacobian::certify(&j.det)?;
            let metrics = vec![
                (Metric::ScaledJacobian, scaled),
                (Metric::JacobianRatio, check.ratio),
                (Metric::Shape, knupp),
                (Metric::EdgeRatio, shape::edge_ratio(&c, &jacobian::HEX_EDGES)),
            ];
            Ok(ElementQuality { kind, size: check.volume, jacobian: Some(check), metrics })
        }
        ElementKind::Tri3 | ElementKind::Tri6 => {
            let (area, metrics) = shape::tri(corners(&p)?);
            Ok(ElementQuality { kind, size: area, jacobian: None, metrics })
        }
        ElementKind::Quad4 | ElementKind::Quad8 => {
            let (area, metrics) = shape::quad(corners(&p)?);
            Ok(ElementQuality { kind, size: area, jacobian: None, metrics })
        }
        ElementKind::Rod2 | ElementKind::Beam2 => Err(Error::Invalid(format!("quality metrics are not defined for {}", kind.name()))),
    }
}

#[cfg(test)]
mod tests;
