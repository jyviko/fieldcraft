//! Reference points and couplings.
//!
//! A reference point is a single node off the mesh with three translations and three rotations.
//! A coupling ties it to a set's nodes, so a force, moment or boundary condition on the point acts
//! on a whole face (a bolt hole, a mounting pad) without hand-picked nodes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::bcs::Dofs;
use crate::materials::context;
use crate::{Error, MAX_LABEL, Model, Result, check_coordinates, insert_new, not_in_use, replace_existing, require};

/// A reference point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferencePoint {
    pub xyz: [f64; 3],
}

/// How a coupling ties the reference point to the set's nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CouplingKind {
    /// The nodes follow the reference point rigidly in the coupled degrees of freedom.
    Kinematic,
    /// The reference point's load is spread over the nodes (weighted equally), and its motion is
    /// their weighted average; the region stays free to deform.
    Distributing,
}

/// A coupling between a reference point and a set's nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coupling {
    pub reference_point: String,
    pub set: String,
    pub kind: CouplingKind,
    /// The coupled degrees of freedom (all six by default).
    #[serde(default = "Dofs::all")]
    pub dofs: Dofs,
}

impl Model {
    pub fn reference_points(&self) -> &BTreeMap<String, ReferencePoint> {
        &self.reference_points
    }

    pub fn reference_point(&self, name: &str) -> Option<&ReferencePoint> {
        self.reference_points.get(name)
    }

    pub fn add_reference_point(&mut self, name: &str, point: ReferencePoint) -> Result<()> {
        check_coordinates(&point.xyz).map_err(|e| context("reference point", name, e))?;
        insert_new(&mut self.reference_points, "reference point", name, point)
    }

    pub fn replace_reference_point(&mut self, name: &str, point: ReferencePoint) -> Result<()> {
        check_coordinates(&point.xyz).map_err(|e| context("reference point", name, e))?;
        replace_existing(&mut self.reference_points, "reference point", name, point)
    }

    pub fn remove_reference_point(&mut self, name: &str) -> Result<()> {
        let mut users: Vec<String> =
            self.couplings.iter().filter(|(_, c)| c.reference_point == name).map(|(n, _)| format!("coupling `{n}`")).collect();
        users.extend(
            self.boundary_conditions
                .iter()
                .filter(|(_, b)| b.target().reference_point() == Some(name))
                .map(|(n, _)| format!("boundary condition `{n}`")),
        );
        users.extend(self.loads.iter().filter(|(_, l)| l.reference_point() == Some(name)).map(|(n, _)| format!("load `{n}`")));
        not_in_use("reference point", name, users)?;
        self.reference_points.remove(name).map(|_| ()).ok_or_else(|| Error::NotFound(format!("reference point `{name}`")))
    }

    /// The node label each reference point takes in a deck: after the largest mesh node label, in
    /// name order. Every writer uses these, so the decks agree.
    pub fn reference_point_nodes(&self) -> Result<BTreeMap<String, u32>> {
        let first = self.nodes.keys().next_back().copied().unwrap_or(0);
        let mut out = BTreeMap::new();
        for (i, name) in self.reference_points.keys().enumerate() {
            let label = u32::try_from(i)
                .ok()
                .and_then(|i| first.checked_add(1)?.checked_add(i))
                .filter(|l| *l <= MAX_LABEL)
                .ok_or_else(|| Error::Limit(format!("no node label left for reference point `{name}` (labels stop at {MAX_LABEL})")))?;
            out.insert(name.clone(), label);
        }
        Ok(out)
    }

    pub fn couplings(&self) -> &BTreeMap<String, Coupling> {
        &self.couplings
    }

    pub fn coupling(&self, name: &str) -> Option<&Coupling> {
        self.couplings.get(name)
    }

    fn check_coupling(&self, name: &str, c: &Coupling) -> Result<()> {
        if c.dofs.is_empty() {
            return Err(context("coupling", name, Error::Invalid("couples no degree of freedom".into())));
        }
        require(&self.reference_points, "reference point", &c.reference_point).map_err(|e| context("coupling", name, e))?;
        require(&self.sets, "set", &c.set).map_err(|e| context("coupling", name, e))
    }

    pub fn add_coupling(&mut self, name: &str, coupling: Coupling) -> Result<()> {
        self.check_coupling(name, &coupling)?;
        insert_new(&mut self.couplings, "coupling", name, coupling)
    }

    pub fn replace_coupling(&mut self, name: &str, coupling: Coupling) -> Result<()> {
        self.check_coupling(name, &coupling)?;
        replace_existing(&mut self.couplings, "coupling", name, coupling)
    }

    pub fn remove_coupling(&mut self, name: &str) -> Result<()> {
        self.couplings.remove(name).map(|_| ()).ok_or_else(|| Error::NotFound(format!("coupling `{name}`")))
    }
}
