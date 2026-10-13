//! Opening SolidCraft designs (`.solidcraft`): read the design, rebuild its parametric timeline
//! with SolidCraft, and hand back its bodies with persistent face names.
//!
//! The design is rebuilt from its authoritative inputs (sketches, features, parameters); cached
//! display meshes in the file are never used. [`open_design`] takes bytes; [`open_design_file`]
//! reads a path through SolidCraft's file access, which in the browser is the in-memory files the
//! web host loaded.

use solidcraft_doc::{Document, Model, naming};
use solidcraft_kernel::{KernelError, guard};

use crate::{Error, Face, Result, Shape};

/// Largest design file opened (SolidCraft's own limit).
pub const MAX_DESIGN_BYTES: usize = solidcraft_io::MAX_DESIGN_BYTES;

/// A rebuilt design.
#[derive(Debug, Clone)]
pub struct Design {
    pub name: String,
    /// The bodies at the end of the timeline, in the design's order, placed in the world: bodies of
    /// components moved by their occurrences, one body per placement.
    pub bodies: Vec<Shape>,
    /// Features that failed to rebuild. The design still opens with the bodies the rest made,
    /// as it does in SolidCraft.
    pub errors: Vec<FeatureError>,
}

/// A feature of the design that failed to rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureError {
    pub feature: String,
    pub message: String,
}

impl Design {
    pub fn body(&self, name: &str) -> Option<&Shape> {
        self.bodies.iter().find(|b| b.name == name)
    }
}

/// Open a design from its file bytes (a `.solidcraft` container or a JSON design) and rebuild it.
pub fn open_design(bytes: &[u8]) -> Result<Design> {
    let doc = guard("read design", || solidcraft_io::read_design(bytes).map_err(|e| KernelError::Invalid(format!("not a readable design: {e}"))))?;
    rebuild(&doc)
}

/// Read a design file (a regular file of at most [`MAX_DESIGN_BYTES`]) and rebuild it.
pub fn open_design_file(path: &str) -> Result<Design> {
    let bytes = solidcraft_io::vfs::read(path, MAX_DESIGN_BYTES).map_err(|e| Error::Invalid(format!("cannot read `{path}`: {e}")))?;
    open_design(&bytes)
}

/// Rebuild an in-memory document (the design half of [`open_design`]).
pub(crate) fn rebuild(doc: &Document) -> Result<Design> {
    let model = guard("rebuild design", || {
        let mut m = Model::new();
        m.evaluate(doc);
        Ok(m)
    })?;
    let errors = model
        .results
        .iter()
        .filter(|r| !r.skipped)
        .filter_map(|r| r.error.as_ref().map(|e| FeatureError { feature: r.name.clone(), message: e.clone() }))
        .collect();
    // Placed in the world: a component's bodies through every occurrence of it (an instance is a
    // body of its own, named after its occurrence path), as SolidCraft displays and exports them.
    let state = guard("place components", || Ok(solidcraft_doc::world_state(doc, &model.state())))?;
    let mut bodies = Vec::with_capacity(state.bodies.len());
    for b in &state.bodies {
        let faces = guard("name faces", || {
            let names = naming::face_names(b);
            Ok(names.iter().enumerate().map(|(i, name)| Face { name: name.clone(), exact: naming::face_name_exact(b, i) }).collect())
        })?;
        bodies.push(Shape { name: b.name.clone(), body: b.body.clone(), faces });
    }
    Ok(Design { name: doc.name.clone(), bodies, errors })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{self, Plate};

    fn face_areas(shape: &Shape) -> Vec<f64> {
        let mesh = shape.tessellate(0.01).unwrap();
        let mut areas = vec![0.0; shape.faces().len()];
        for (t, f) in mesh.triangles.iter().zip(&mesh.tri_face) {
            let [a, b, c] = t.map(|i| {
                let [x, y, z] = mesh.positions[i as usize];
                solidcraft_geom::Vec3 { x, y, z }
            });
            areas[*f as usize] += 0.5 * (b - a).cross(c - a).len();
        }
        areas
    }

    #[test]
    fn opens_rebuilds_and_tessellates_a_design() {
        let plate = Plate::default();
        let design = open_design(&sample::plate_design(&plate).unwrap()).unwrap();
        assert!(design.errors.is_empty(), "{:?}", design.errors);
        assert_eq!(design.bodies.len(), 1);
        let body = &design.bodies[0];
        let m = body.measure().unwrap();
        let r = plate.hole_diameter / 2.0;
        let expect = (plate.width * plate.depth - std::f64::consts::PI * r * r) * plate.thickness;
        assert!((m.volume - expect).abs() < 1e-3 * expect, "{} vs {expect}", m.volume);
        // Four sides, top, bottom and the hole (a cylinder is two half faces).
        assert_eq!(m.faces, 8);
        assert_eq!(body.faces().len(), 8);
        let mesh = body.tessellate(0.01).unwrap();
        let mut tags = mesh.tri_face.clone();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), 8);
        assert!(tags.iter().all(|&t| (t as usize) < body.faces().len()));
        // Every face has its own name.
        let mut names: Vec<&str> = body.faces().iter().map(|f| f.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 8, "{names:?}");
    }

    #[test]
    fn face_names_survive_a_parameter_change() {
        let narrow = Plate::default();
        let wide = Plate { width: 70.0, thickness: 15.0, ..Plate::default() };
        let a = open_design(&sample::plate_design(&narrow).unwrap()).unwrap();
        let b = open_design(&sample::plate_design(&wide).unwrap()).unwrap();
        let (a, b) = (&a.bodies[0], &b.bodies[0]);
        let (area_a, area_b) = (face_areas(a), face_areas(b));
        let pi = std::f64::consts::PI;
        // Each half of the hole.
        let hole = |p: &Plate| pi * p.hole_diameter * p.thickness / 2.0;
        let cap = |p: &Plate| p.width * p.depth - pi * p.hole_diameter * p.hole_diameter / 4.0;
        for face in a.faces() {
            let i = a.face_index(&face.name).unwrap() as usize;
            let j = b.face_index(&face.name).unwrap_or_else(|| panic!("{} is gone after the edit", face.name)) as usize;
            // The same face by name: the hole stays the hole, a cap stays a cap, and a side keeps
            // its own dimension (depth sides grow with the thickness only, width sides with both).
            let (x, y) = (area_a[i], area_b[j]);
            let close = |v: f64, e: f64| (v - e).abs() < 0.02 * e;
            let kind = if close(x, hole(&narrow)) {
                assert!(close(y, hole(&wide)), "{}: {y}", face.name);
                "hole"
            } else if close(x, cap(&narrow)) {
                assert!(close(y, cap(&wide)), "{}: {y}", face.name);
                "cap"
            } else if close(x, narrow.width * narrow.thickness) {
                assert!(close(y, wide.width * wide.thickness), "{}: {y}", face.name);
                "width side"
            } else {
                assert!(close(x, narrow.depth * narrow.thickness), "{}: {x}", face.name);
                assert!(close(y, wide.depth * wide.thickness), "{}: {y}", face.name);
                "depth side"
            };
            assert!(face.exact, "{} ({kind}) is named from geometry", face.name);
        }
    }

    #[test]
    fn hostile_bytes_are_an_error() {
        for bytes in [&b""[..], b"PK\x03\x04", b"{", b"{}", b"\xff\xfe\x00", b"[1,2,3]", br#"{"format":"solidcraft/9999","name":"x"}"#] {
            assert!(open_design(bytes).is_err(), "{bytes:?}");
        }
        assert!(open_design(&vec![b' '; MAX_DESIGN_BYTES + 1]).is_err());
        for path in ["", "/", "/dev/zero", "/no/such/design.solidcraft"] {
            assert!(open_design_file(path).is_err(), "{path}");
        }
    }
}
