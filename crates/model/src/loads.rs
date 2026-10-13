//! Loads. Which steps a load is active in is listed explicitly by each step ([`crate::steps`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::bcs::Target;
use crate::materials::context;
use crate::{Error, Model, Result, finite, insert_new, replace_existing, require};

/// A load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Load {
    /// A force vector applied at every node of the target (not divided among them: to apply a
    /// total force to a face, put it on a reference point coupled to the face).
    Force { target: Target, vector: [f64; 3] },
    /// A moment vector applied at every node of the target. Solid elements have no rotations, so
    /// a moment on a solid goes through a coupled reference point.
    Moment { target: Target, vector: [f64; 3] },
    /// Uniform pressure on the set's element faces, positive pushing into the elements (against
    /// the outward normal).
    Pressure { set: String, magnitude: f64 },
    /// Gravity: an acceleration of the mass of the set's elements, or of the whole model when no
    /// set is given.
    Gravity {
        acceleration: [f64; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set: Option<String>,
    },
    /// A force per volume on the set's elements.
    BodyForce { set: String, vector: [f64; 3] },
}

impl Load {
    /// `force`, `moment`, `pressure`, `gravity` or `body_force`.
    pub fn kind(&self) -> &'static str {
        match self {
            Load::Force { .. } => "force",
            Load::Moment { .. } => "moment",
            Load::Pressure { .. } => "pressure",
            Load::Gravity { .. } => "gravity",
            Load::BodyForce { .. } => "body_force",
        }
    }

    /// The set the load acts on, if any.
    pub fn set(&self) -> Option<&str> {
        match self {
            Load::Force { target, .. } | Load::Moment { target, .. } => target.set(),
            Load::Pressure { set, .. } | Load::BodyForce { set, .. } => Some(set),
            Load::Gravity { set, .. } => set.as_deref(),
        }
    }

    /// The reference point the load acts on, if any.
    pub fn reference_point(&self) -> Option<&str> {
        match self {
            Load::Force { target, .. } | Load::Moment { target, .. } => target.reference_point(),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let vector = match self {
            Load::Force { vector, .. } | Load::Moment { vector, .. } | Load::BodyForce { vector, .. } => vector,
            Load::Gravity { acceleration, .. } => acceleration,
            Load::Pressure { magnitude, .. } => {
                finite("pressure", *magnitude)?;
                return Ok(());
            }
        };
        for c in vector {
            finite(self.kind(), *c)?;
        }
        if vector.iter().all(|c| *c == 0.0) {
            return Err(Error::Invalid(format!("a {} vector must not be zero", self.kind())));
        }
        Ok(())
    }
}

impl Model {
    pub fn loads(&self) -> &BTreeMap<String, Load> {
        &self.loads
    }

    pub fn load(&self, name: &str) -> Option<&Load> {
        self.loads.get(name)
    }

    fn check_load(&self, name: &str, load: &Load) -> Result<()> {
        load.validate().map_err(|e| context("load", name, e))?;
        if let Some(s) = load.set() {
            require(&self.sets, "set", s).map_err(|e| context("load", name, e))?;
        }
        if let Some(r) = load.reference_point() {
            require(&self.reference_points, "reference point", r).map_err(|e| context("load", name, e))?;
        }
        Ok(())
    }

    pub fn add_load(&mut self, name: &str, load: Load) -> Result<()> {
        self.check_load(name, &load)?;
        insert_new(&mut self.loads, "load", name, load)
    }

    pub fn replace_load(&mut self, name: &str, load: Load) -> Result<()> {
        self.check_load(name, &load)?;
        replace_existing(&mut self.loads, "load", name, load)
    }

    /// Remove a load and its entries in the step table.
    pub fn remove_load(&mut self, name: &str) -> Result<()> {
        self.loads.remove(name).ok_or_else(|| Error::NotFound(format!("load `{name}`")))?;
        for s in &mut self.steps {
            s.loads.remove(name);
        }
        Ok(())
    }
}
