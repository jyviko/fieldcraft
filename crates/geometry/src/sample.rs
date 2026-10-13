//! Sample SolidCraft designs built in code: test parts whose recipe is this source, so no design
//! file is committed (docs/adr/0001-libraries.md, test geometry).

use solidcraft_doc::sketch::{ConstraintKind, Sketch};
use solidcraft_doc::{Direction, Document, Extent, FeatureKind, Operation, PlaneRef, ProfileSel};
use solidcraft_geom::Vec2;

use crate::{Error, Result};

/// A rectangular plate with a round through hole, as a parametric design: the sketch's width is
/// driven by the design parameter `width` and the extrusion by `thickness`, so editing either
/// rebuilds the same faces at a new size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plate {
    /// Along X, mm. At least `depth`.
    pub width: f64,
    /// Along Y, mm.
    pub depth: f64,
    /// Along Z, mm.
    pub thickness: f64,
    /// The hole is centred `depth / 2` from the corner at the origin, so it needs to be smaller
    /// than `depth`.
    pub hole_diameter: f64,
}

impl Default for Plate {
    fn default() -> Self {
        Plate { width: 40.0, depth: 30.0, thickness: 10.0, hole_diameter: 10.0 }
    }
}

fn failed(e: impl std::fmt::Display) -> Error {
    Error::Failed(format!("sample design: {e}"))
}

fn extrude(sketch: u64, distance: &str, operation: Operation) -> FeatureKind {
    FeatureKind::Extrude {
        sketch,
        profiles: ProfileSel::All,
        extent: Extent {
            distance: distance.into(),
            direction: Direction::Positive,
            distance2: None,
            start_offset: None,
            through_all: false,
            taper: None,
            to: None,
            to_offset: None,
            from: None,
        },
        operation,
        targets: vec![],
    }
}

/// The plate as `.solidcraft` file bytes.
pub fn plate_design(p: &Plate) -> Result<Vec<u8>> {
    let ok = |v: f64| v.is_finite() && v > 0.0 && v <= 1e6;
    if !(ok(p.width) && ok(p.depth) && ok(p.thickness) && ok(p.hole_diameter)) || p.width < p.depth || p.hole_diameter >= p.depth {
        return Err(Error::Invalid(format!("plate dimensions out of range: {p:?}")));
    }
    let mut doc = Document::new("Plate");
    doc.set_param("width", &format!("{} mm", p.width), Some("mm"), None).map_err(failed)?;
    doc.set_param("thickness", &format!("{} mm", p.thickness), Some("mm"), None).map_err(failed)?;

    let mut sk = Sketch::new();
    let corners = [Vec2::new(0.0, 0.0), Vec2::new(p.width, 0.0), Vec2::new(p.width, p.depth), Vec2::new(0.0, p.depth)];
    let mut pts = [0usize; 4];
    for (id, c) in pts.iter_mut().zip(corners) {
        *id = sk.add_point(c, None).map_err(failed)?;
    }
    let mut lines = [0usize; 4];
    for (i, l) in lines.iter_mut().enumerate() {
        *l = sk.add_line_pts(pts[i], pts[(i + 1) % 4], None).map_err(failed)?;
    }
    let [l0, l1, l2, l3] = lines;
    // Point 0 of a sketch is its origin.
    sk.add_constraint(ConstraintKind::Coincident { p: pts[0], q: 0 }, None).map_err(failed)?;
    sk.add_constraint(ConstraintKind::Horizontal { l: l0 }, None).map_err(failed)?;
    sk.add_constraint(ConstraintKind::Horizontal { l: l2 }, None).map_err(failed)?;
    sk.add_constraint(ConstraintKind::Vertical { l: l1 }, None).map_err(failed)?;
    sk.add_constraint(ConstraintKind::Vertical { l: l3 }, None).map_err(failed)?;
    let width = doc.new_model_param("width", "mm");
    sk.add_constraint(ConstraintKind::Length { l: l0, value: p.width }, Some(width)).map_err(failed)?;
    let depth = doc.new_model_param(&format!("{} mm", p.depth), "mm");
    sk.add_constraint(ConstraintKind::Length { l: l1, value: p.depth }, Some(depth)).map_err(failed)?;
    let outline =
        doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: sk }, Some("Outline")).map_err(failed)?;
    doc.add_feature(extrude(outline, "thickness", Operation::NewBody), Some("Plate")).map_err(failed)?;

    let mut sk = Sketch::new();
    let c = p.depth / 2.0;
    sk.add_circle(Vec2::new(c, c), p.hole_diameter / 2.0, None, None).map_err(failed)?;
    let hole =
        doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: sk }, Some("Hole sketch")).map_err(failed)?;
    // Past the top face, so the cut never ends on it.
    doc.add_feature(extrude(hole, "thickness + 1 mm", Operation::Cut), Some("Hole")).map_err(failed)?;

    solidcraft_io::write_design_container(&doc).map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_dimensions_are_an_error() {
        let base = Plate::default();
        for p in [
            Plate { width: f64::NAN, ..base },
            Plate { width: -1.0, ..base },
            Plate { thickness: 0.0, ..base },
            Plate { thickness: f64::INFINITY, ..base },
            Plate { width: 10.0, ..base },
            Plate { hole_diameter: 30.0, ..base },
            Plate { depth: 1e300, width: 1e300, ..base },
        ] {
            assert!(plate_design(&p).is_err(), "{p:?}");
        }
    }
}
