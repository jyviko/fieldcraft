//! Running a fluid step: the time loop, output frames and force histories, in model units.

use modelcraft_model::Model;

use crate::flow::Flow;
use crate::setup::{Obstacle, Setup};
use crate::voxel::Grid;
use crate::{Error, Limits, Result};

/// Most samples a force history keeps; longer runs record every k-th step.
pub const MAX_HISTORY: u64 = 10_000;

/// A relative density variation above this at the end of a run is reported.
pub const COMPRESSIBILITY_WARNING: f64 = 0.01;

/// The velocity and pressure field at one time, per cell (index `i + nx (j + ny k)`; zero in
/// solid cells). Stored in single precision: output for display and probing.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub time: f64,
    pub step: u64,
    pub velocity: Vec<[f32; 3]>,
    /// Gauge pressure.
    pub pressure: Vec<f32>,
    /// The total fluid mass (density times volume).
    pub mass: f64,
}

/// The force the fluid exerts on one wall over time.
#[derive(Debug, Clone, PartialEq)]
pub struct ForceHistory {
    /// `x_min` … for box faces, the face's persistent name for obstacle faces.
    pub name: String,
    /// The obstacle body, for obstacle faces.
    pub body: Option<String>,
    /// One value per [`FluidResult::history_times`].
    pub force: Vec<[f64; 3]>,
}

/// Run statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct FluidStats {
    pub cells: usize,
    pub fluid_cells: usize,
    pub steps: u64,
    pub time_step: f64,
    pub end_time: f64,
    pub tau_plus: f64,
    pub tau_minus: f64,
    pub magic: f64,
    /// The characteristic velocity (model units), its lattice value and its lattice Mach number.
    pub velocity: f64,
    pub lattice_velocity: f64,
    pub mach: f64,
    /// Fluid cells times steps.
    pub cell_updates: u64,
}

/// The result of a fluid step.
#[derive(Debug, Clone, PartialEq)]
pub struct FluidResult {
    pub step: String,
    pub grid: Grid,
    /// Per cell: not fluid (inside an obstacle).
    pub solid: Vec<bool>,
    /// The initial field and `frames` more, evenly spaced to the end.
    pub frames: Vec<Frame>,
    pub history_times: Vec<f64>,
    /// Every box wall and every obstacle face.
    pub forces: Vec<ForceHistory>,
    pub stats: FluidStats,
    pub warnings: Vec<String>,
}

impl FluidResult {
    /// The cell containing a point and the frame's velocity and pressure there (`None` outside
    /// the grid; zeros in a solid cell).
    pub fn probe(&self, frame: usize, p: [f64; 3]) -> Option<(usize, [f32; 3], f32)> {
        let cell = self.grid.cell_at(p)?;
        let f = self.frames.get(frame)?;
        Some((cell, *f.velocity.get(cell)?, *f.pressure.get(cell)?))
    }
}

/// Solve the fluid step named `step` (or the model's only fluid step) with the obstacles' surfaces
/// prepared by the caller.
pub fn solve_fluid(model: &Model, step: Option<&str>, obstacles: &[Obstacle], limits: &Limits) -> Result<FluidResult> {
    let setup = Setup::new(model, step, obstacles, limits)?;
    let mut flow = Flow::new(&setup, obstacles)?;
    run(&setup, &mut flow, limits)
}

fn frame(setup: &Setup, flow: &Flow, step: u64) -> Result<Frame> {
    let n = setup.grid.len();
    let mut velocity = Vec::new();
    let mut pressure = Vec::new();
    velocity.try_reserve_exact(n).map_err(|_| Error::Limit("not enough memory for an output frame".into()))?;
    pressure.try_reserve_exact(n).map_err(|_| Error::Limit("not enough memory for an output frame".into()))?;
    let vs = setup.velocity_scale();
    for cell in 0..n {
        match flow.moments(cell) {
            Some((rho, u)) => {
                velocity.push(u.map(|c| (c * vs) as f32));
                pressure.push(setup.gauge_pressure(rho) as f32);
            }
            None => {
                velocity.push([0.0; 3]);
                pressure.push(0.0);
            }
        }
    }
    let mass = flow.mass() * setup.density * setup.grid.spacing.powi(3);
    Ok(Frame { time: step as f64 * setup.time_step, step, velocity, pressure, mass })
}

/// Run a prepared flow (fresh from [`Flow::new`], possibly with its own initial state) through
/// the setup's steps, recording frames and force histories.
pub fn run(setup: &Setup, flow: &mut Flow, limits: &Limits) -> Result<FluidResult> {
    let n = setup.grid.len();
    let total = setup.steps;
    let frames = u64::from(setup.frames).min(total).max(1);
    let frame_bytes = (frames + 1) as f64 * n as f64 * 16.0;
    if !(frame_bytes <= limits.max_frame_bytes as f64) {
        return Err(Error::Limit(format!(
            "{} frames of {n} cells need {:.0} MB, above the cap of {:.0} MB; write fewer frames or raise `max_frame_bytes`",
            frames + 1,
            frame_bytes / 1e6,
            limits.max_frame_bytes as f64 / 1e6
        )));
    }
    let every = total.div_ceil(MAX_HISTORY).max(1);
    let fs = setup.force_scale();

    let mut out_frames = vec![frame(setup, flow, 0)?];
    let mut history_times = Vec::new();
    let mut forces: Vec<Vec<[f64; 3]>> = vec![Vec::new(); flow.groups().len()];
    let mut next_frame = 1u64;
    for k in 1..=total {
        flow.step()?;
        if k % every == 0 || k == total {
            history_times.push(k as f64 * setup.time_step);
            for (h, f) in forces.iter_mut().zip(flow.forces()) {
                h.push(f.map(|c| c * fs));
            }
        }
        // Frame j (1..=frames) at step round(j · total / frames).
        while next_frame <= frames && k == (next_frame * total + frames / 2) / frames {
            out_frames.push(frame(setup, flow, k)?);
            next_frame += 1;
        }
    }

    let mut warnings = setup.warnings.clone();
    // The weakly compressible scheme turns pressure differences into density differences of
    // Δp / (ρ c_s²) in lattice units; large ones show the time step is too long for the flow.
    let spread = (0..n).filter_map(|c| flow.moments(c)).map(|(rho, _)| (rho - 1.0).abs()).fold(0.0, f64::max);
    if spread > COMPRESSIBILITY_WARNING {
        warnings.push(format!(
            "the density varies by {:.1}% at the end: the weakly compressible error is visible; a shorter time step reduces it (quadratically)",
            100.0 * spread
        ));
    }
    for body in flow.empty_obstacles() {
        warnings.push(format!("obstacle `{body}` covers no cell centre: it is smaller than the spacing or outside the box"));
    }
    let solid: Vec<bool> = (0..n).map(|c| !flow.is_fluid(c)).collect();
    Ok(FluidResult {
        step: setup.step.clone(),
        grid: setup.grid,
        solid,
        frames: out_frames,
        history_times,
        forces: flow.groups().iter().zip(forces).map(|(g, force)| ForceHistory { name: g.name.clone(), body: g.body.clone(), force }).collect(),
        stats: FluidStats {
            cells: n,
            fluid_cells: flow.fluid_cells(),
            steps: total,
            time_step: setup.time_step,
            end_time: total as f64 * setup.time_step,
            tau_plus: setup.tau_plus,
            tau_minus: setup.tau_minus,
            magic: setup.magic,
            velocity: setup.velocity,
            lattice_velocity: setup.lattice_velocity,
            mach: setup.mach,
            cell_updates: (flow.fluid_cells() as u64).saturating_mul(total),
        },
        warnings,
    })
}
