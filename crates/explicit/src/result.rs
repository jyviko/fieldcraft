//! What an explicit dynamic run keeps: histories sampled at the history interval and field
//! frames at the output interval.

use serde::Serialize;

/// The energy account at one time (Belytschko, Liu, Moran, Elkhodary 2014, §6.2.3). The works of
/// the internal and external forces are integrated with the trapezoidal rule over each
/// increment's displacement; the constraints' with their impulses (see [`crate::integrate`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Energies {
    /// ½ vᵀ M v.
    pub kinetic: f64,
    /// Work of the material stresses: the strain energy for an elastic material.
    pub internal: f64,
    /// Work of the hourglass forces (stored in the stiffness form, dissipated in the viscous one).
    pub hourglass: f64,
    /// Work of the bulk viscosity (dissipated).
    pub viscous: f64,
    /// Work of the loads.
    pub external: f64,
    /// Work of the boundary conditions and rigid walls on the model.
    pub constraint: f64,
    /// kinetic + internal + hourglass + viscous − external − constraint − the initial kinetic
    /// energy: zero when energy is conserved.
    pub balance: f64,
}

impl Energies {
    /// The balance relative to the largest energy in the account.
    pub fn relative_error(&self, initial_kinetic: f64) -> f64 {
        let scale = [initial_kinetic, self.kinetic, self.internal + self.hourglass + self.viscous, self.external.abs(), self.constraint.abs()]
            .into_iter()
            .fold(0.0, f64::max);
        if scale > 0.0 { self.balance.abs() / scale } else { 0.0 }
    }
}

/// The fields at one time. Vectors that the step's output does not request are empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub time: f64,
    /// Per node in [`ExplicitResult::nodes`] order.
    pub displacement: Vec<[f64; 3]>,
    pub velocity: Vec<[f64; 3]>,
    /// Per element in [`ExplicitResult::elements`] order (one-point elements: one value each):
    /// xx, yy, zz, xy, xz, yz, engineering shear strains.
    pub stress: Vec<[f64; 6]>,
    pub strain: Vec<[f64; 6]>,
}

/// Quantities sampled through the run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    pub time: Vec<f64>,
    /// The time increment taken at each sample.
    pub increment: Vec<f64>,
    pub energies: Vec<Energies>,
    /// Total momentum M v.
    pub momentum: Vec<[f64; 3]>,
    /// Sum of the forces the boundary conditions and rigid walls exert on the model.
    pub constraint_force: Vec<[f64; 3]>,
    /// The nodes with histories, and per sample their displacement and velocity (in this order).
    pub nodes: Vec<u32>,
    pub displacement: Vec<Vec<[f64; 3]>>,
    pub velocity: Vec<Vec<[f64; 3]>>,
}

/// An element that limits the stable time increment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ControllingElement {
    pub label: u32,
    /// Its critical increment at rest (no bulk viscosity), before the scale factor.
    pub increment: f64,
}

/// The result of an explicit dynamic step.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplicitResult {
    pub step: String,
    /// Node labels with degrees of freedom, sorted.
    pub nodes: Vec<u32>,
    /// Element labels, sorted.
    pub elements: Vec<u32>,
    pub frames: Vec<Frame>,
    pub history: History,
    /// The elements with the smallest critical increments (at most ten), smallest first.
    pub controlling: Vec<ControllingElement>,
    /// The first time increment taken (the scale times the smallest critical increment).
    pub initial_increment: f64,
    pub increments: u64,
    /// The time reached: the step's end time unless the run stopped at the increment cap.
    pub time: f64,
    pub completed: bool,
    pub total_mass: f64,
    /// The energy account at the end.
    pub energy: Energies,
    pub initial_kinetic: f64,
    pub warnings: Vec<String>,
}
