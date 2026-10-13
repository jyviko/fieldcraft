//! The geometry boundary.
//!
//! Meshing in ModelCraft starts from a triangulated boundary surface. This crate produces it:
//! B-rep bodies come from SolidCraft's kernel (`solidcraft-kernel`, a pinned git dependency;
//! plan/adr/0001-libraries.md) and leave it as a [`SurfaceMesh`] from SolidCraft's tessellator,
//! with each triangle tagged by the B-rep face it lies on (for feature edges and for loads and
//! boundary conditions on faces). No SolidCraft type appears in the public API, so the kernel can
//! be replaced without touching the rest of the workspace. Only this crate may depend on
//! `solidcraft-*` (`cargo xtask layers` enforces it).
//!
//! Geometry enters two ways: shapes built in code (such as [`block`]) and SolidCraft designs
//! opened with [`open_design`], which rebuilds the design's parametric timeline and gives its
//! bodies with persistent face names (a boundary condition on a named face follows that face
//! through an edit of the design).
//!
//! Lengths are millimetres.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod design;
pub mod sample;
#[cfg(feature = "scripts")]
mod script;

pub use design::{Design, FeatureError, MAX_DESIGN_BYTES, open_design, open_design_file};
pub mod tagged_mesh;
#[cfg(feature = "scripts")]
pub use script::{MAX_SCRIPT_BYTES, script_design};
use solidcraft_kernel::{Body, KernelError, guard};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("the geometry operation failed: {0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<KernelError> for Error {
    fn from(e: KernelError) -> Self {
        match e {
            KernelError::Invalid(m) => Error::Invalid(m),
            other => Error::Failed(other.to_string()),
        }
    }
}

/// A solid body.
#[derive(Debug, Clone)]
pub struct Shape {
    pub name: String,
    body: Body,
    /// By B-rep face index (the [`SurfaceMesh::tri_face`] tags); empty for shapes built in code.
    faces: Vec<Face>,
}

/// A B-rep face's persistent identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    /// Stable across rebuilds of the design: named after the feature and sketch curve that made
    /// the face (`F2:side:l4`, `F2:top`, …), not after its position in the body.
    pub name: String,
    /// The name follows the history SolidCraft recorded; `false` when it was worked out from the
    /// geometry, which an upstream edit can fool.
    pub exact: bool,
}

/// Mass properties and extent of a [`Shape`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    pub volume: f64,
    pub area: f64,
    pub centroid: [f64; 3],
    pub bbox_min: [f64; 3],
    pub bbox_max: [f64; 3],
    pub faces: usize,
}

/// A triangulated boundary surface: the input to volume meshing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SurfaceMesh {
    pub positions: Vec<[f64; 3]>,
    /// Counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
    /// The B-rep face each triangle lies on.
    pub tri_face: Vec<u32>,
}

fn xyz(v: solidcraft_geom::Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

/// A box between two corners.
pub fn block(a: [f64; 3], b: [f64; 3]) -> Result<Shape> {
    let v = |p: [f64; 3]| solidcraft_geom::Vec3 { x: p[0], y: p[1], z: p[2] };
    let body = guard("block", || solidcraft_kernel::box_solid(v(a), v(b)))?;
    Ok(Shape { name: "Block".into(), body, faces: Vec::new() })
}

impl Shape {
    /// The persistent identities of the faces, by B-rep face index (empty for shapes built in code).
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// The index of the face with this persistent name: the [`SurfaceMesh::tri_face`] tag its
    /// triangles carry.
    pub fn face_index(&self, name: &str) -> Option<u32> {
        self.faces.iter().position(|f| f.name == name).and_then(|i| u32::try_from(i).ok())
    }

    pub fn measure(&self) -> Result<Measure> {
        let m = guard("measure", || solidcraft_kernel::measure(&self.body))?;
        Ok(Measure {
            volume: m.volume,
            area: m.area,
            centroid: xyz(m.centroid),
            bbox_min: xyz(m.bbox.min),
            bbox_max: xyz(m.bbox.max),
            faces: m.faces,
        })
    }

    /// Triangulate the boundary within `tolerance` (chordal deviation, mm).
    pub fn tessellate(&self, tolerance: f64) -> Result<SurfaceMesh> {
        if !(tolerance > 0.0 && tolerance.is_finite()) {
            return Err(Error::Invalid(format!("tessellation tolerance must be a positive number, got {tolerance}")));
        }
        let m = guard("tessellate", || self.body.tessellate(tolerance))?;
        Ok(SurfaceMesh { positions: m.positions.into_iter().map(xyz).collect(), triangles: m.triangles, tri_face: m.tri_face })
    }
}

impl SurfaceMesh {
    /// The volume the triangles enclose (divergence theorem: the sum of the signed tetrahedra each
    /// triangle spans with the origin). Equals the solid's volume for a closed, outward-oriented
    /// surface; a triangle with a vertex index out of range counts as nothing.
    pub fn enclosed_volume(&self) -> f64 {
        let six_v: f64 = self
            .triangles
            .iter()
            .filter_map(|t| {
                let [a, b, c] = [self.positions.get(t[0] as usize)?, self.positions.get(t[1] as usize)?, self.positions.get(t[2] as usize)?];
                Some(a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]))
            })
            .sum();
        six_v / 6.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_measures() {
        let m = block([0., 0., 0.], [10., 20., 30.]).unwrap().measure().unwrap();
        assert!((m.volume - 6000.0).abs() < 1e-6, "{}", m.volume);
        assert!((m.area - 2200.0).abs() < 1e-6, "{}", m.area);
        assert_eq!(m.faces, 6);
        assert_eq!(m.bbox_max, [10., 20., 30.]);
    }

    #[test]
    fn tessellation_covers_every_face_and_encloses_the_volume() {
        let mesh = block([0., 0., 0.], [2., 3., 4.]).unwrap().tessellate(0.01).unwrap();
        assert_eq!(mesh.triangles.len(), mesh.tri_face.len());
        let mut faces: Vec<u32> = mesh.tri_face.clone();
        faces.sort_unstable();
        faces.dedup();
        assert_eq!(faces.len(), 6);
        // The outward-oriented triangles enclose the block's volume.
        assert!((mesh.enclosed_volume() - 24.0).abs() < 1e-9, "{}", mesh.enclosed_volume());
    }

    #[test]
    fn hostile_input_is_an_error() {
        assert!(block([0.; 3], [0.; 3]).is_err());
        assert!(block([0.; 3], [f64::NAN, 1., 1.]).is_err());
        let b = block([0.; 3], [1.; 3]).unwrap();
        for t in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(b.tessellate(t).is_err());
        }
    }
}
