//! Boundary conditions: prescribed degrees of freedom on a set's nodes or a reference point.
//!
//! Degrees of freedom are numbered 1–3 for translations along X, Y, Z and 4–6 for rotations about
//! them, in the global axes. Which steps a boundary condition is active in is listed explicitly by
//! each step ([`crate::steps`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::materials::context;
use crate::{Error, Model, Result, finite, insert_new, replace_existing, require};

/// A set of degrees of freedom (1–6), written as a sorted list such as `[1, 2, 3]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub struct Dofs(u8);

impl Dofs {
    pub const TRANSLATIONS: Dofs = Dofs(0b000_111);

    pub fn all() -> Dofs {
        Dofs(0b111_111)
    }

    pub fn none() -> Dofs {
        Dofs(0)
    }

    pub fn contains(self, dof: u8) -> bool {
        (1..=6).contains(&dof) && self.0 & (1 << (dof - 1)) != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The degrees of freedom in order.
    pub fn list(self) -> Vec<u8> {
        (1..=6).filter(|d| self.contains(*d)).collect()
    }

    /// Whether any rotation (4–6) is included.
    pub fn has_rotations(self) -> bool {
        self.0 & 0b111_000 != 0
    }
}

impl TryFrom<Vec<u8>> for Dofs {
    type Error = Error;
    fn try_from(v: Vec<u8>) -> Result<Dofs> {
        let mut mask = 0u8;
        for d in v {
            if !(1..=6).contains(&d) {
                return Err(Error::Invalid(format!("degree of freedom {d} is not in 1..=6")));
            }
            mask |= 1 << (d - 1);
        }
        Ok(Dofs(mask))
    }
}

impl From<Dofs> for Vec<u8> {
    fn from(d: Dofs) -> Vec<u8> {
        d.list()
    }
}

/// What a boundary condition, force or moment acts on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// The nodes of a set.
    Set(String),
    /// A reference point.
    ReferencePoint(String),
}

impl Target {
    pub fn set(&self) -> Option<&str> {
        match self {
            Target::Set(s) => Some(s),
            Target::ReferencePoint(_) => None,
        }
    }

    pub fn reference_point(&self) -> Option<&str> {
        match self {
            Target::ReferencePoint(r) => Some(r),
            Target::Set(_) => None,
        }
    }

    pub(crate) fn check_exists(&self, model: &Model) -> Result<()> {
        match self {
            Target::Set(s) => require(&model.sets, "set", s),
            Target::ReferencePoint(r) => require(&model.reference_points, "reference point", r),
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Set(s) => write!(f, "set `{s}`"),
            Target::ReferencePoint(r) => write!(f, "reference point `{r}`"),
        }
    }
}

/// The plane of a symmetry boundary condition, named by its normal axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Symmetry {
    /// The plane normal to X: no X translation, no rotation about Y or Z.
    X,
    Y,
    Z,
}

impl Symmetry {
    /// The fixed degrees of freedom.
    pub fn dofs(self) -> Dofs {
        match self {
            Symmetry::X => Dofs(0b110_001),
            Symmetry::Y => Dofs(0b101_010),
            Symmetry::Z => Dofs(0b011_100),
        }
    }
}

/// A boundary condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundaryCondition {
    /// Every degree of freedom held at zero.
    Fixed { target: Target },
    /// Prescribed values for some degrees of freedom: `values[i]` is degree of freedom `i + 1`
    /// (`null` leaves it free, `0` holds it).
    Displacement { target: Target, values: [Option<f64>; 6] },
    /// Mirror symmetry about a plane through the nodes.
    Symmetry { target: Target, plane: Symmetry },
}

impl BoundaryCondition {
    pub fn target(&self) -> &Target {
        match self {
            BoundaryCondition::Fixed { target } | BoundaryCondition::Displacement { target, .. } | BoundaryCondition::Symmetry { target, .. } => {
                target
            }
        }
    }

    /// The constrained degrees of freedom.
    pub fn dofs(&self) -> Dofs {
        match self {
            BoundaryCondition::Fixed { .. } => Dofs::all(),
            BoundaryCondition::Displacement { values, .. } => {
                Dofs(values.iter().enumerate().filter(|(_, v)| v.is_some()).fold(0, |m, (i, _)| m | (1 << i)))
            }
            BoundaryCondition::Symmetry { plane, .. } => plane.dofs(),
        }
    }

    /// The prescribed value of a constrained degree of freedom (1–6); `None` when it is free.
    pub fn value(&self, dof: u8) -> Option<f64> {
        match self {
            BoundaryCondition::Displacement { values, .. } => values.get(usize::from(dof).checked_sub(1)?).copied().flatten(),
            _ => self.dofs().contains(dof).then_some(0.0),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let BoundaryCondition::Displacement { values, .. } = self {
            for (i, v) in values.iter().enumerate() {
                if let Some(v) = v {
                    finite(&format!("degree of freedom {}", i + 1), *v)?;
                }
            }
            if values.iter().all(Option::is_none) {
                return Err(Error::Invalid("a displacement boundary condition needs at least one value".into()));
            }
        }
        Ok(())
    }
}

impl Model {
    pub fn boundary_conditions(&self) -> &BTreeMap<String, BoundaryCondition> {
        &self.boundary_conditions
    }

    pub fn boundary_condition(&self, name: &str) -> Option<&BoundaryCondition> {
        self.boundary_conditions.get(name)
    }

    fn check_bc(&self, name: &str, bc: &BoundaryCondition) -> Result<()> {
        bc.validate().map_err(|e| context("boundary condition", name, e))?;
        bc.target().check_exists(self).map_err(|e| context("boundary condition", name, e))
    }

    pub fn add_boundary_condition(&mut self, name: &str, bc: BoundaryCondition) -> Result<()> {
        self.check_bc(name, &bc)?;
        insert_new(&mut self.boundary_conditions, "boundary condition", name, bc)
    }

    pub fn replace_boundary_condition(&mut self, name: &str, bc: BoundaryCondition) -> Result<()> {
        self.check_bc(name, &bc)?;
        replace_existing(&mut self.boundary_conditions, "boundary condition", name, bc)
    }

    /// Remove a boundary condition and its entries in the step table.
    pub fn remove_boundary_condition(&mut self, name: &str) -> Result<()> {
        self.boundary_conditions.remove(name).ok_or_else(|| Error::NotFound(format!("boundary condition `{name}`")))?;
        for s in &mut self.steps {
            s.boundary_conditions.remove(name);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dofs_and_values() {
        let d: Dofs = serde_json::from_value(serde_json::json!([3, 1, 1])).unwrap();
        assert_eq!(d.list(), vec![1, 3]);
        assert_eq!(serde_json::to_value(d).unwrap(), serde_json::json!([1, 3]));
        assert!(serde_json::from_value::<Dofs>(serde_json::json!([0])).is_err());
        assert!(serde_json::from_value::<Dofs>(serde_json::json!([7])).is_err());
        assert_eq!(Symmetry::X.dofs().list(), vec![1, 5, 6]);
        assert_eq!(Symmetry::Y.dofs().list(), vec![2, 4, 6]);
        assert_eq!(Symmetry::Z.dofs().list(), vec![3, 4, 5]);

        let t = Target::Set("s".into());
        let bc = BoundaryCondition::Displacement { target: t.clone(), values: [Some(0.0), None, Some(1.5), None, None, None] };
        assert_eq!(bc.dofs().list(), vec![1, 3]);
        assert_eq!((bc.value(1), bc.value(2), bc.value(3), bc.value(0), bc.value(9)), (Some(0.0), None, Some(1.5), None, None));
        assert_eq!(BoundaryCondition::Fixed { target: t.clone() }.value(6), Some(0.0));
        assert!(BoundaryCondition::Displacement { target: t.clone(), values: [None; 6] }.validate().is_err());
        assert!(BoundaryCondition::Displacement { target: t, values: [Some(f64::NAN), None, None, None, None, None] }.validate().is_err());
        let json = serde_json::json!({"symmetry": {"target": {"reference_point": "RP"}, "plane": "z"}});
        let parsed: BoundaryCondition = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), json);
    }
}
