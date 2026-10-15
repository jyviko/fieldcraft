//! Relaxation fields: what a node relaxation (`mesh.node.relax`) did, kept so it can be done again
//! on a new mesh of the same geometry.
//!
//! A field is the relaxed region of the mesh it was made on: each element there as its corners
//! were (`at`) and where they went (`moved`). A node of a later mesh that lies in one of these
//! cells takes the displacement interpolated with the cell's own shape functions at its
//! parametric coordinates (barycentric in a tetrahedron or triangle, trilinear `(u, v, w)` in a
//! hexahedron, …), so the relaxed shape carries over to a coarser or finer mesh. Fields are
//! definitions of the model: journalled with the command that made them and saved in the
//! `.fieldcraft` file.

use serde::{Deserialize, Serialize};

use crate::{ElementKind, Error, Result, check_coordinates};

/// Most cells one field holds.
pub const MAX_RELAXATION_CELLS: usize = 1_000_000;

/// One cell of a field: a linear element's corners before and after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelaxationCell {
    /// tri3, quad4, tet4, wedge6 or hex8 (a quadratic element's corners).
    pub kind: ElementKind,
    pub at: Vec<[f64; 3]>,
    pub moved: Vec<[f64; 3]>,
}

/// A relaxation field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relaxation {
    /// The bodies it was made on (empty: any).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<String>,
    pub cells: Vec<RelaxationCell>,
    /// Applied again by itself after each remesh of its bodies.
    #[serde(default)]
    pub reapply: bool,
}

impl Relaxation {
    pub fn check(&self) -> Result<()> {
        if self.cells.len() > MAX_RELAXATION_CELLS {
            return Err(Error::Limit(format!("a relaxation field holds at most {MAX_RELAXATION_CELLS} cells")));
        }
        for c in &self.cells {
            if !matches!(c.kind, ElementKind::Tri3 | ElementKind::Quad4 | ElementKind::Tet4 | ElementKind::Wedge6 | ElementKind::Hex8) {
                return Err(Error::Invalid(format!("a relaxation cell is linear, not {}", c.kind.name())));
            }
            let n = c.kind.node_count();
            if c.at.len() != n || c.moved.len() != n {
                return Err(Error::Invalid(format!("a {} relaxation cell has {n} corners", c.kind.name())));
            }
            check_coordinates(c.at.as_flattened())?;
            check_coordinates(c.moved.as_flattened())?;
        }
        Ok(())
    }
}
