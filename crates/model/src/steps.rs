//! Analysis steps and the step × load/boundary-condition table.
//!
//! Steps run in order. Each step lists the boundary conditions and loads active in it, by name;
//! nothing carries over from one step to the next unless it is listed again, so the table a user
//! sees is the whole truth (no propagation, no hidden modified or inactive states). The state
//! before the first step is the unloaded, unconstrained model; a boundary condition active in
//! every step can be written at model level by formats that have one.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::materials::context;
use crate::{Error, Model, Result, check_name, finite, require};

/// Most steps in a model.
pub const MAX_STEPS: usize = 1000;

/// Most modes a frequency step extracts.
pub const MAX_MODES: u32 = 100_000;

/// What a step solves.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Procedure {
    /// Linear static equilibrium under the active loads and boundary conditions.
    Static {},
    /// Natural frequencies and mode shapes: the lowest `modes` modes, optionally only those
    /// between `min_hz` and `max_hz` (cycles per time unit of the unit system).
    Frequency {
        modes: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_hz: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_hz: Option<f64>,
    },
}

impl Procedure {
    /// `static` or `frequency`.
    pub fn name(&self) -> &'static str {
        match self {
            Procedure::Static {} => "static",
            Procedure::Frequency { .. } => "frequency",
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let Procedure::Frequency { modes, min_hz, max_hz } = *self {
            if !(1..=MAX_MODES).contains(&modes) {
                return Err(Error::Invalid(format!("a frequency step extracts 1 to {MAX_MODES} modes, got {modes}")));
            }
            for f in [min_hz, max_hz].into_iter().flatten() {
                if finite("frequency", f)? < 0.0 {
                    return Err(Error::Invalid(format!("frequencies are not negative, got {f}")));
                }
            }
            if let (Some(lo), Some(hi)) = (min_hz, max_hz)
                && lo >= hi
            {
                return Err(Error::Invalid(format!("the frequency range {lo}..{hi} is empty")));
            }
        }
        Ok(())
    }

    /// The output a step of this procedure requests unless told otherwise.
    pub fn default_output(&self) -> BTreeSet<OutputVariable> {
        match self {
            Procedure::Static {} => OutputVariable::ALL.into_iter().collect(),
            Procedure::Frequency { .. } => [OutputVariable::Displacement].into_iter().collect(),
        }
    }
}

/// A result a step writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputVariable {
    /// Nodal displacements (mode shapes in a frequency step).
    Displacement,
    /// Reaction forces at constrained nodes.
    ReactionForce,
    Stress,
    Strain,
}

impl OutputVariable {
    pub const ALL: [OutputVariable; 4] =
        [OutputVariable::Displacement, OutputVariable::ReactionForce, OutputVariable::Stress, OutputVariable::Strain];

    pub fn name(self) -> &'static str {
        match self {
            OutputVariable::Displacement => "displacement",
            OutputVariable::ReactionForce => "reaction_force",
            OutputVariable::Stress => "stress",
            OutputVariable::Strain => "strain",
        }
    }

    pub fn from_name(name: &str) -> Option<OutputVariable> {
        OutputVariable::ALL.into_iter().find(|v| v.name() == name)
    }
}

/// An analysis step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    pub procedure: Procedure,
    pub output: BTreeSet<OutputVariable>,
    /// Boundary conditions active in this step.
    #[serde(default)]
    pub boundary_conditions: BTreeSet<String>,
    /// Loads active in this step.
    #[serde(default)]
    pub loads: BTreeSet<String>,
}

impl Step {
    /// A step with the procedure's default output and nothing active.
    pub fn new(name: impl Into<String>, procedure: Procedure) -> Step {
        Step { name: name.into(), output: procedure.default_output(), procedure, boundary_conditions: BTreeSet::new(), loads: BTreeSet::new() }
    }
}

impl Model {
    /// The steps in analysis order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn step(&self, name: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.name == name)
    }

    fn step_index(&self, name: &str) -> Result<usize> {
        self.steps.iter().position(|s| s.name == name).ok_or_else(|| Error::NotFound(format!("step `{name}`")))
    }

    fn check_step(&self, step: &Step) -> Result<()> {
        step.procedure.validate().map_err(|e| context("step", &step.name, e))?;
        for b in &step.boundary_conditions {
            require(&self.boundary_conditions, "boundary condition", b).map_err(|e| context("step", &step.name, e))?;
        }
        for l in &step.loads {
            require(&self.loads, "load", l).map_err(|e| context("step", &step.name, e))?;
        }
        Ok(())
    }

    /// Add a step at the end, or just before the step named `before`.
    pub fn add_step(&mut self, step: Step, before: Option<&str>) -> Result<()> {
        check_name("step", &step.name)?;
        if self.step(&step.name).is_some() {
            return Err(Error::Invalid(format!("step `{}` already exists", step.name)));
        }
        if self.steps.len() >= MAX_STEPS {
            return Err(Error::Limit(format!("a model holds at most {MAX_STEPS} steps")));
        }
        self.check_step(&step)?;
        let at = match before {
            Some(b) => self.step_index(b)?,
            None => self.steps.len(),
        };
        self.steps.insert(at, step);
        Ok(())
    }

    /// Replace a step's definition (procedure, output and table entries), keeping its place.
    pub fn replace_step(&mut self, step: Step) -> Result<()> {
        let i = self.step_index(&step.name)?;
        self.check_step(&step)?;
        if let Some(slot) = self.steps.get_mut(i) {
            *slot = step;
        }
        Ok(())
    }

    pub fn remove_step(&mut self, name: &str) -> Result<()> {
        let i = self.step_index(name)?;
        self.steps.remove(i);
        Ok(())
    }

    /// Make a boundary condition active or inactive in a step.
    pub fn set_boundary_condition_active(&mut self, step: &str, name: &str, active: bool) -> Result<()> {
        require(&self.boundary_conditions, "boundary condition", name)?;
        let i = self.step_index(step)?;
        if let Some(s) = self.steps.get_mut(i) {
            if active {
                s.boundary_conditions.insert(name.to_string());
            } else {
                s.boundary_conditions.remove(name);
            }
        }
        Ok(())
    }

    /// Make a load active or inactive in a step.
    pub fn set_load_active(&mut self, step: &str, name: &str, active: bool) -> Result<()> {
        require(&self.loads, "load", name)?;
        let i = self.step_index(step)?;
        if let Some(s) = self.steps.get_mut(i) {
            if active {
                s.loads.insert(name.to_string());
            } else {
                s.loads.remove(name);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoundaryCondition, Set, Target};

    #[test]
    fn steps_keep_order_and_an_explicit_table() {
        let mut m = Model::default();
        m.add_set("base", Set { nodes: vec![1], ..Set::default() }).unwrap();
        m.add_boundary_condition("fix", BoundaryCondition::Fixed { target: Target::Set("base".into()) }).unwrap();
        m.add_step(Step::new("Load", Procedure::Static {}), None).unwrap();
        m.add_step(Step::new("Modes", Procedure::Frequency { modes: 10, min_hz: None, max_hz: None }), Some("Load")).unwrap();
        assert_eq!(m.steps().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Modes", "Load"]);
        assert!(m.add_step(Step::new("Load", Procedure::Static {}), None).is_err());
        assert!(m.add_step(Step::new("X", Procedure::Static {}), Some("Nope")).is_err());
        assert!(m.add_step(Step::new("Y", Procedure::Frequency { modes: 0, min_hz: None, max_hz: None }), None).is_err());
        assert!(m.add_step(Step::new("Y", Procedure::Frequency { modes: 5, min_hz: Some(10.0), max_hz: Some(1.0) }), None).is_err());
        assert!(serde_json::from_value::<Procedure>(serde_json::json!({"type": "static", "modes": 3})).is_err(), "typos are errors");

        m.set_boundary_condition_active("Load", "fix", true).unwrap();
        assert!(m.step("Load").unwrap().boundary_conditions.contains("fix"));
        assert!(m.step("Modes").unwrap().boundary_conditions.is_empty(), "no propagation");
        assert!(m.set_boundary_condition_active("Load", "nope", true).is_err());
        assert!(m.set_load_active("Load", "fix", true).is_err(), "loads and boundary conditions are separate");

        // Deleting a boundary condition clears its row; a referenced set cannot be deleted.
        assert!(matches!(m.remove_set("base"), Err(Error::InUse(_))));
        m.remove_boundary_condition("fix").unwrap();
        assert!(m.step("Load").unwrap().boundary_conditions.is_empty());
        m.remove_set("base").unwrap();
        m.remove_step("Modes").unwrap();
        assert_eq!(m.steps().len(), 1);
    }
}
