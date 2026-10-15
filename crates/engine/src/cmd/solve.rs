//! `solve.*`: run the model's steps with the implicit, the explicit or the lattice Boltzmann
//! solver, keep the results in the session, and read them back.

use crate::schema::ProblemKind;
use fieldcraft_explicit::{ExplicitResult, solve_explicit};
use fieldcraft_lbm::{FluidResult, Obstacle};
use fieldcraft_mesh::surface::{BoundarySurface, SurfaceOptions};
use fieldcraft_model::FluidDomain;
use fieldcraft_parallel as parallel;
use fieldcraft_solver::{
    FrequencyOptions, Limits, Location, MassKind, StaticResult, StepResult as ImplicitResult, solve_buckle, solve_frequency, solve_heat,
    solve_nonlinear, solve_static,
};
use serde_json::{Value, json};

use super::{labels, opt, points, req};
use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session, StepResult};

pub const RUN: CommandSpec = CommandSpec {
    id: "solve.run",
    label: "Solve All Steps",
    params: r#"{} — solves every step in order with its procedure's solver (solve.static, solve.frequency, solve.explicit, solve.fluid with their default caps) and keeps the results; stops at the first step that fails, naming it"#,
    form: Some(&[]),
    kind: CommandKind::Model,
    requires: &[Requirement::Analysable, Requirement::ElementMaterial, Requirement::ElementSection, Requirement::Step],
    run: run_all,
};

pub const STATIC: CommandSpec = CommandSpec {
    id: "solve.static",
    label: "Solve Static Step",
    params: r#"{"step": "Load", "max_dofs": 6000000, "max_factor_entries": 400000000, "threads": 8} — solves a linear static or coupled temperature–displacement step (the only one when `step` is left out) and keeps the result in the session; the caps may only be lowered; `threads` (1 to 256, by default the session's: `FIELDCRAFT_THREADS` or the cores) changes only the time, never the result"#,
    form: Some(super::forms::SOLVE_STATIC),
    kind: CommandKind::Model,
    requires: &[Requirement::Analysable, Requirement::ElementMaterial, Requirement::ElementSection, Requirement::Step],
    run: run_static,
};

pub const NONLINEAR: CommandSpec = CommandSpec {
    id: "solve.nonlinear",
    label: "Solve Nonlinear Static Step",
    params: r#"{"step": "Inflate", "max_dofs": 6000000, "max_factor_entries": 400000000, "max_newton_iterations": 100000, "threads": 8} — solves a nonlinear static step (the only one when `step` is left out) in load increments with Newton–Raphson iterations and keeps the result in the session: the final displacements, reactions, Cauchy stresses and logarithmic strains (small strains without nonlinear geometry), the equivalent plastic strain of plastic materials (the largest in the summary), every increment's load factor, iterations and residuals (force, displacement and energy, relative to their tolerances' scales), and a displacement frame per converged increment, and with contact pairs (interaction.contact.pair: mortar contact by augmented Lagrangian, Coulomb friction by a return map) each contact's force, normal force, largest pressure, closed and slipping nodes, deepest penetration and augmentations (solve.probe gives each secondary node's pressure, gap and status, and with friction its shear traction and slip); the caps may only be lowered (`max_newton_iterations` over the whole step); `threads` as for solve.static"#,
    form: Some(super::forms::SOLVE_NONLINEAR),
    kind: CommandKind::Model,
    requires: &[Requirement::Analysable, Requirement::ElementMaterial, Requirement::ElementSection, Requirement::Step],
    run: run_nonlinear,
};

pub const HEAT: CommandSpec = CommandSpec {
    id: "solve.heat",
    label: "Solve Heat Transfer Step",
    params: r#"{"step": "Heat", "max_dofs": 6000000, "max_factor_entries": 400000000, "threads": 8} — steady or transient heat conduction of a heat transfer step (the only one when `step` is left out): temperatures, heat flux and the heat flow at prescribed temperatures, kept in the session; the caps may only be lowered; `threads` as for solve.static"#,
    form: Some(super::forms::SOLVE_STATIC),
    kind: CommandKind::Model,
    requires: &[Requirement::Analysable, Requirement::ElementMaterial, Requirement::ElementSection, Requirement::Step],
    run: run_heat,
};

pub const FREQUENCY: CommandSpec = CommandSpec {
    id: "solve.frequency",
    label: "Solve Frequency Step",
    params: r#"{"step": "Modes", "mass": "consistent|lumped", "shift": 0.0, "max_dofs": 6000000, "max_factor_entries": 400000000, "max_iterations": 3000, "threads": 8} — natural frequencies and mode shapes of a frequency step (its modes and range); `max_iterations` caps the Lanczos vectors; the result stays in the session; `threads` as for solve.static"#,
    form: Some(super::forms::SOLVE_FREQUENCY),
    kind: CommandKind::Model,
    requires: &[Requirement::Analysable, Requirement::ElementMaterial, Requirement::ElementSection, Requirement::Step],
    run: run_frequency,
};

pub const BUCKLE: CommandSpec = CommandSpec {
    id: "solve.buckle",
    label: "Solve Buckling Step",
    params: r#"{"step": "Buckle", "max_dofs": 6000000, "max_factor_entries": 400000000, "max_iterations": 3000, "threads": 8} — linear buckling of a buckling step (the only one when `step` is left out): the step's loads as a linear preload, then the lowest load factors λ with (K + λ K_σ) φ = 0 and their mode shapes (largest component 1), or those above the step's shift (whose LDLᵀ factor counts the load factors below it); negative load factors found are reported; the result stays in the session and its modes display like frequency modes; `threads` as for solve.static"#,
    form: Some(super::forms::SOLVE_BUCKLE),
    kind: CommandKind::Model,
    requires: &[Requirement::Analysable, Requirement::ElementMaterial, Requirement::ElementSection, Requirement::Step],
    run: run_buckle,
};

pub const EXPLICIT: CommandSpec = CommandSpec {
    id: "solve.explicit",
    label: "Solve Explicit Dynamic Step",
    params: r#"{"step": "Impact", "max_dofs": 6000000, "max_increments": 1000000000, "max_output_values": 50000000, "max_contact_faces": 10000000, "threads": 8} — transient response of an explicit dynamic step (the only one when `step` is left out) by central differences, with the model's contacts (interaction.contact.*) and the step's rigid walls; keeps its histories and field frames in the session and reports each contact's peak force, first and last contact, deepest penetration, friction dissipated and the nodes adjusted at the start; the caps may only be lowered (the step's own increment cap applies too); `threads` as for solve.static"#,
    form: Some(super::forms::SOLVE_EXPLICIT),
    kind: CommandKind::Model,
    requires: &[Requirement::Analysable, Requirement::ElementMaterial, Requirement::ElementSection, Requirement::Step],
    run: run_explicit,
};

pub const HISTORY: CommandSpec = CommandSpec {
    id: "solve.history",
    label: "Result Histories",
    params: r#"{"step": "Impact", "every": 1} — the time histories of a solved explicit step: time, increment, energies, momentum, constraint force, the normal force across each contact (in `contacts` order) and the history nodes' displacement and velocity; `every` keeps one sample in so many (by default enough to return at most 10000)"#,
    form: Some(super::forms::SOLVE_HISTORY),
    kind: CommandKind::Model,
    requires: &[Requirement::Result],
    run: history,
};

pub const FLUID: CommandSpec = CommandSpec {
    id: "solve.fluid",
    label: "Solve Fluid Step",
    params: r#"{"step": "Flow", "max_cells": 16000000, "max_steps": 10000000, "max_cell_updates": 1e12, "max_frame_bytes": 2e9, "threads": 8} — runs a fluid step with the lattice Boltzmann solver (the only fluid step when `step` is left out): obstacles are triangulated from the open design or taken from imported surfaces; returns the time step, relaxation time, Mach number, mass and the final force on every wall; the caps may only be lowered; `threads` as for solve.static"#,
    form: Some(super::forms::SOLVE_FLUID),
    kind: CommandKind::Model,
    requires: &[Requirement::FluidDomain, Requirement::Step],
    run: run_fluid,
};

pub const PROBE: CommandSpec = CommandSpec {
    id: "solve.probe",
    label: "Probe Results",
    params: r#"{"step": "Load", "nodes": [1, 2] (or "set": "fillet", a set's nodes), "mode": 1, "frame": 0} — the stored result at nodes (with `max_von_mises`, the largest among them, for static steps): displacement, rotation, reaction, reaction moment, stress and von Mises (static, with `elements` each element's results: solid, 2D and truss nodal stresses (and, in a nonlinear step, the equivalent plastic strain at the integration points of a plastic element), shell section forces (N, M, Q per length in the element's axes) and bottom, middle and top stresses, beam end forces (N, V_y, V_z, T, M_y, M_z in its axes)), the shape of mode `mode` with its rotations (frequency, from 1), or displacement and velocity in field frame `frame` (explicit, from 0; the last by default) at `nodes` (or at the nodes nearest to points: "at": [[x, y, z], ...], lowest label on a tie, each entry then giving its `xyz` and `distance`) and, with `elements` (explicit), each element's stress, von Mises stress, strain, equivalent plastic strain, damage, temperature and whether it has eroded; a coupled reference point's label gives its six displacements (translations, rotations) and reactions (forces, moments). Fluid steps: {"step": "Flow", "points": [[x, y, z]], "frame": 10 (default the last)} gives velocity and pressure in the cells at the points; {"step": "Flow", "forces": ["x_min", "F2:top", "Body1/F2:top"]} gives those walls' force histories"#,
    form: Some(super::forms::SOLVE_PROBE),
    kind: CommandKind::Model,
    requires: &[Requirement::Result],
    run: probe,
};

/// Most nodes one probe lists.
const MAX_PROBE: usize = 10_000;

/// Most force histories one fluid probe lists.
const MAX_PROBE_FORCES: usize = 100;

/// Most history samples one `solve.history` returns.
const MAX_HISTORY_SAMPLES: usize = 10_000;

/// An optional cap that may only lower the default.
fn cap<T: Copy + TryInto<u64> + TryFrom<u64> + std::fmt::Display>(p: &Value, key: &str, default: T) -> Result<T> {
    let most: u64 = default.try_into().unwrap_or(u64::MAX);
    match p.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=most).contains(n))
            .and_then(|n| T::try_from(n).ok())
            .ok_or_else(|| EngineError::param(key, ProblemKind::NotAWholeNumber { min: 1, max: i64::try_from(most).unwrap_or(i64::MAX) })),
    }
}

/// The `threads` parameter: 1 to [`parallel::MAX_THREADS`], by default the session's.
fn threads(s: &Session, p: &Value) -> Result<usize> {
    match p.get("threads") {
        None => Ok(s.limits.threads.clamp(1, parallel::MAX_THREADS)),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=parallel::MAX_THREADS).contains(n))
            .ok_or_else(|| EngineError::Params(format!("`threads` must be a whole number from 1 to {}", parallel::MAX_THREADS))),
    }
}

fn limits(p: &Value) -> Result<Limits> {
    Ok(Limits {
        max_dofs: cap(p, "max_dofs", Limits::MAX_DOFS)?,
        max_factor_entries: cap(p, "max_factor_entries", Limits::MAX_FACTOR_ENTRIES)?,
        max_lanczos_vectors: cap(p, "max_iterations", Limits::MAX_LANCZOS_VECTORS)?,
        max_newton_iterations: cap(p, "max_newton_iterations", Limits::MAX_NEWTON_ITERATIONS)?,
    })
}

/// The node with the value that `better` prefers, as `{"node", "value"}`.
fn extreme(values: impl Iterator<Item = (u32, f64)>, better: fn(f64, f64) -> bool) -> Value {
    match values.fold(None, |best: Option<(u32, f64)>, (l, v)| if best.is_none_or(|(_, b)| better(v, b)) { Some((l, v)) } else { best }) {
        Some((node, value)) => json!({"node": node, "value": value}),
        None => Value::Null,
    }
}

/// [`parallel::with_threads`], the caller's progress reporter carried onto the pool's thread.
fn with_threads<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    let progress = fieldcraft_model::progress::current();
    parallel::with_threads(threads, move || fieldcraft_model::progress::scoped(progress, f))
}

fn run_all(s: &mut Session, _: &Value) -> Result<Value> {
    use fieldcraft_model::Procedure;
    type Solve = fn(&mut Session, &Value) -> Result<Value>;
    let steps: Vec<(String, &'static str, Solve)> = s
        .model
        .steps()
        .iter()
        .map(|st| {
            let solve: Solve = match st.procedure {
                Procedure::Static {} | Procedure::CoupledTemperatureDisplacement {} => run_static,
                Procedure::HeatTransfer { .. } => run_heat,
                Procedure::Frequency { .. } => run_frequency,
                Procedure::Buckle { .. } => run_buckle,
                Procedure::ExplicitDynamic(_) => run_explicit,
                Procedure::Fluid { .. } => run_fluid,
                Procedure::NonlinearStatic(_) => run_nonlinear,
            };
            (st.name.clone(), st.procedure.name(), solve)
        })
        .collect();
    if steps.is_empty() {
        return Err(EngineError::Params("the model has no steps to solve: add one (model.step.create)".into()));
    }
    let mut done = Vec::new();
    let count = steps.len();
    for (index, (name, procedure, solve)) in steps.into_iter().enumerate() {
        fieldcraft_model::progress::step(&name, index + 1, count);
        let summary = solve(s, &json!({"step": name})).map_err(|e| EngineError::Params(format!("step `{name}`: {e}")))?;
        done.push(json!({"step": name, "procedure": procedure, "summary": summary}));
    }
    Ok(json!({"steps": done}))
}

fn largest(values: impl Iterator<Item = (u32, f64)>) -> Value {
    extreme(values, |v, b| v > b)
}

fn smallest(values: impl Iterator<Item = (u32, f64)>) -> Value {
    extreme(values, |v, b| v < b)
}

fn run_static(s: &mut Session, p: &Value) -> Result<Value> {
    let limits = limits(p)?;
    let step = opt::<String>(p, "step")?;
    let model = &s.model;
    let r = with_threads(threads(s, p)?, || solve_static(model, step.as_deref(), &limits))?;

    let summary = json!({
        "step": r.step,
        "dofs": r.stats.dofs,
        "equations": r.stats.equations,
        "factor_entries": r.stats.factor_entries,
        "max_displacement": largest(r.nodes.iter().zip(&r.displacement).map(|(l, u)| (*l, crate::results::magnitude(*u)))),
        "max_von_mises": largest(r.nodes.iter().zip(&r.von_mises).map(|(l, v)| (*l, *v))),
        "applied_total": r.applied_total,
        "reaction_total": r.reaction_total,
        "reference_points": r.reference_points.iter().map(|p| json!({
            "name": p.name,
            "label": p.label,
            "displacement": p.displacement,
            "reaction": p.reaction,
        })).collect::<Vec<_>>(),
        "max_temperature": r.temperature.as_ref().map(|t| largest(r.nodes.iter().copied().zip(t.iter().copied()))),
        "warnings": r.warnings,
    });
    s.store_result(StepResult::Implicit(ImplicitResult::Static(r)));
    Ok(summary)
}

fn run_nonlinear(s: &mut Session, p: &Value) -> Result<Value> {
    let limits = limits(p)?;
    let step = opt::<String>(p, "step")?;
    let model = &s.model;
    let r = with_threads(threads(s, p)?, || solve_nonlinear(model, step.as_deref(), &limits))?;
    let history = r.nonlinear.as_ref();
    let summary = json!({
        "step": r.step,
        "dofs": r.stats.dofs,
        "equations": r.stats.equations,
        "factor_entries": r.stats.factor_entries,
        "increments": history.map(|h| &h.increments),
        "iterations": history.map(|h| h.iterations),
        "frames": history.map(|h| h.load_factors()),
        "load_factor": history.and_then(|h| h.frames.last()).map(|f| f.load_factor),
        "critical_points": history.map(|h| &h.critical_points),
        "contacts": r.contacts.iter().map(|c| json!({
            "name": c.name,
            "normal_force": c.normal_force,
            "force": c.force,
            "max_pressure": largest(c.nodes.iter().copied().zip(c.pressure.iter().copied())),
            "closed_nodes": c.closed(),
            "slipping_nodes": c.status.iter().filter(|s| **s == fieldcraft_solver::contact::Status::Slip).count(),
            "penetration": c.penetration,
            "augmentations": c.augmentations,
        })).collect::<Vec<_>>(),
        "max_displacement": largest(r.nodes.iter().zip(&r.displacement).map(|(l, u)| (*l, crate::results::magnitude(*u)))),
        "max_von_mises": largest(r.nodes.iter().zip(&r.von_mises).map(|(l, v)| (*l, *v))),
        "max_plastic_strain": largest(r.elements.iter().flat_map(|e| e.plastic_strain.iter().map(|v| (e.label, *v)))),
        "applied_total": r.applied_total,
        "reaction_total": r.reaction_total,
        "reference_points": r.reference_points.iter().map(|p| json!({
            "name": p.name,
            "label": p.label,
            "displacement": p.displacement,
            "reaction": p.reaction,
        })).collect::<Vec<_>>(),
        "warnings": r.warnings,
    });
    s.store_result(StepResult::Implicit(ImplicitResult::Static(r)));
    Ok(summary)
}

fn run_heat(s: &mut Session, p: &Value) -> Result<Value> {
    let limits = limits(p)?;
    let step = opt::<String>(p, "step")?;
    let model = &s.model;
    let r = with_threads(threads(s, p)?, || solve_heat(model, step.as_deref(), &limits))?;
    let t = r.temperature();
    let summary = json!({
        "step": r.step,
        "dofs": r.stats.dofs,
        "equations": r.stats.equations,
        "factor_entries": r.stats.factor_entries,
        "frames": r.frames.len(),
        "end_time": r.times.last(),
        "min_temperature": smallest(r.nodes.iter().copied().zip(t.iter().copied())),
        "max_temperature": largest(r.nodes.iter().copied().zip(t.iter().copied())),
        "heat_supplied": r.applied_total,
        "heat_removed": r.reaction_total,
        "heat_flows": crate::results::heat_flows(&r),
        "warnings": r.warnings,
    });
    s.store_result(StepResult::Implicit(ImplicitResult::Heat(r)));
    Ok(summary)
}

fn run_frequency(s: &mut Session, p: &Value) -> Result<Value> {
    let limits = limits(p)?;
    let step = opt::<String>(p, "step")?;
    let mass = opt::<MassKind>(p, "mass")?.unwrap_or_default();
    let shift = opt::<f64>(p, "shift")?;
    let model = &s.model;
    let r = with_threads(threads(s, p)?, || solve_frequency(model, step.as_deref(), &FrequencyOptions { mass, shift }, &limits))?;
    let summary = json!({
        "step": r.step,
        "dofs": r.stats.dofs,
        "equations": r.stats.equations,
        "lanczos_vectors": r.stats.lanczos_vectors,
        "total_mass": r.total_mass,
        "modes": r.modes.iter().enumerate().map(|(i, m)| json!({
            "mode": i + 1,
            "frequency": m.frequency,
            "eigenvalue": m.eigenvalue,
            "effective_mass": m.effective_mass,
        })).collect::<Vec<_>>(),
        "warnings": r.warnings,
    });
    s.store_result(StepResult::Implicit(ImplicitResult::Frequency(r)));
    Ok(summary)
}

fn run_buckle(s: &mut Session, p: &Value) -> Result<Value> {
    let limits = limits(p)?;
    let step = opt::<String>(p, "step")?;
    let model = &s.model;
    let r = with_threads(threads(s, p)?, || solve_buckle(model, step.as_deref(), &limits))?;
    let summary = json!({
        "step": r.step,
        "dofs": r.stats.dofs,
        "equations": r.stats.equations,
        "lanczos_vectors": r.stats.lanczos_vectors,
        "modes": r.modes.iter().enumerate().map(|(i, m)| json!({"mode": i + 1, "load_factor": m.eigenvalue})).collect::<Vec<_>>(),
        "buckling": r.buckling,
        "warnings": r.warnings,
    });
    s.store_result(StepResult::Implicit(ImplicitResult::Frequency(r)));
    Ok(summary)
}

fn run_explicit(s: &mut Session, p: &Value) -> Result<Value> {
    use fieldcraft_explicit::Limits as ExplicitLimits;
    let limits = ExplicitLimits {
        max_dofs: cap(p, "max_dofs", ExplicitLimits::MAX_DOFS)?,
        max_increments: cap(p, "max_increments", ExplicitLimits::MAX_INCREMENTS)?,
        max_output_values: cap(p, "max_output_values", ExplicitLimits::MAX_OUTPUT_VALUES)?,
        max_contact_faces: cap(p, "max_contact_faces", ExplicitLimits::MAX_CONTACT_FACES)?,
    };
    let step = opt::<String>(p, "step")?;
    let model = &s.model;
    let r = with_threads(threads(s, p)?, || solve_explicit(model, step.as_deref(), &limits))?;
    let summary = explicit_summary(&r);
    s.store_result(StepResult::Explicit(r));
    Ok(summary)
}

fn explicit_summary(r: &ExplicitResult) -> Value {
    json!({
        "step": r.step,
        "dofs": 3 * r.nodes.len(),
        "elements": r.elements.len(),
        "increments": r.increments,
        "time": r.time,
        "completed": r.completed,
        "initial_increment": r.initial_increment,
        "controlling_elements": r.controlling,
        "total_mass": r.total_mass,
        "energy": r.energy,
        "energy_balance": r.energy.relative_error(r.initial_kinetic),
        "frames": r.frames.len(),
        "history_samples": r.history.time.len(),
        "eroded": r.eroded,
        "contacts": r.contacts,
        "mass_scaling": r.mass_scaling,
        "warnings": r.warnings,
    })
}

fn history(s: &mut Session, p: &Value) -> Result<Value> {
    let step: String = req(p, "step")?;
    let result = s.result(&step).ok_or_else(|| EngineError::Params(format!("step `{step}` has no result; solve it first")))?;
    let StepResult::Explicit(r) = result.as_ref() else {
        return Err(EngineError::Params(format!("step `{step}` is not an explicit dynamic step; it has no histories")));
    };
    let h = &r.history;
    let samples = h.time.len();
    let every = match opt::<u64>(p, "every")? {
        Some(0) => return Err(EngineError::param("every", ProblemKind::Below(1.0))),
        Some(k) => usize::try_from(k).unwrap_or(usize::MAX),
        None => samples.div_ceil(MAX_HISTORY_SAMPLES).max(1),
    };
    if samples.div_ceil(every) > MAX_HISTORY_SAMPLES {
        return Err(EngineError::Params(format!("{samples} samples in steps of {every} are more than {MAX_HISTORY_SAMPLES}; raise `every`")));
    }
    let pick = |v: &dyn Fn(usize) -> Value| -> Vec<Value> { (0..samples).step_by(every).map(v).collect() };
    let nodes: Vec<Value> = h
        .nodes
        .iter()
        .enumerate()
        .map(|(k, label)| {
            json!({
                "node": label,
                "displacement": pick(&|i| json!(h.displacement.get(i).and_then(|d| d.get(k)))),
                "velocity": pick(&|i| json!(h.velocity.get(i).and_then(|d| d.get(k)))),
            })
        })
        .collect();
    Ok(json!({
        "step": step,
        "every": every,
        "time": pick(&|i| json!(h.time.get(i))),
        "increment": pick(&|i| json!(h.increment.get(i))),
        "energies": pick(&|i| json!(h.energies.get(i))),
        "momentum": pick(&|i| json!(h.momentum.get(i))),
        "constraint_force": pick(&|i| json!(h.constraint_force.get(i))),
        "contact_force": pick(&|i| json!(h.contact_force.get(i))),
        "contacts": r.contacts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        "nodes": nodes,
    }))
}

/// The surfaces of a fluid domain's obstacles: bodies of the open design (triangulated at the
/// domain's tolerance) or imported surfaces, by name.
fn obstacles(s: &Session, domain: &FluidDomain) -> Result<Vec<Obstacle>> {
    let tolerance = domain.tolerance.unwrap_or(domain.spacing / 10.0);
    let mut out = Vec::with_capacity(domain.obstacles.len());
    for body in &domain.obstacles {
        let (mesh, faces) = if let Some(shape) = s.design.as_deref().and_then(|d| d.body(body)) {
            (std::borrow::Cow::Owned(shape.tessellate(tolerance)?), shape.faces().iter().map(|f| f.name.clone()).collect())
        } else if let Some(t) = s.surfaces.iter().find(|t| &t.name == body) {
            // Imported faces have no names: the solver calls them face0, face1, ….
            (std::borrow::Cow::Borrowed(&t.mesh), Vec::new())
        } else {
            return Err(EngineError::Params(format!("obstacle `{body}` is neither a body of the open design nor an imported surface")));
        };
        let options = SurfaceOptions { weld_distance: mesh.weld_distance, ..SurfaceOptions::default() };
        let surface = BoundarySurface::new(&mesh.positions, &mesh.triangles, &mesh.tri_face, &options)
            .map_err(|e| EngineError::Params(format!("obstacle `{body}`: its surface cannot bound a solid: {e}")))?;
        out.push(Obstacle { body: body.clone(), surface, faces });
    }
    Ok(out)
}

fn run_fluid(s: &mut Session, p: &Value) -> Result<Value> {
    let defaults = fieldcraft_lbm::Limits::default();
    let limits = fieldcraft_lbm::Limits {
        max_cells: cap(p, "max_cells", defaults.max_cells)?,
        max_steps: cap(p, "max_steps", defaults.max_steps)?,
        max_cell_updates: cap(p, "max_cell_updates", defaults.max_cell_updates)?,
        max_frame_bytes: cap(p, "max_frame_bytes", defaults.max_frame_bytes)?,
    };
    let step = opt::<String>(p, "step")?;
    let (_, domain) = fieldcraft_lbm::fluid_step(&s.model, step.as_deref())?;
    let obstacles = obstacles(s, domain)?;
    let model = &s.model;
    let r = with_threads(threads(s, p)?, || fieldcraft_lbm::solve_fluid(model, step.as_deref(), &obstacles, &limits))?;
    let summary = fluid_summary(&r);
    s.store_result(StepResult::Fluid(r));
    Ok(summary)
}

fn fluid_summary(r: &FluidResult) -> Value {
    let speed = |v: &[f32; 3]| (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2) + f64::from(v[2]).powi(2)).sqrt();
    let last = r.frames.last();
    let max_speed = last.map_or(0.0, |f| f.velocity.iter().map(speed).fold(0.0, f64::max));
    // The mean over the last quarter of the history smooths the acoustic noise of the
    // weakly compressible scheme.
    let tail = r.history_times.len().div_ceil(4).max(1);
    let forces: Vec<Value> = r
        .forces
        .iter()
        .filter(|f| f.force.iter().any(|v| *v != [0.0; 3]))
        .map(|f| {
            let recent = f.force.get(f.force.len().saturating_sub(tail)..).unwrap_or_default();
            let n = recent.len().max(1) as f64;
            let mean = [0, 1, 2].map(|a| recent.iter().map(|v| v[a]).sum::<f64>() / n);
            json!({"name": f.name, "body": f.body, "final": f.force.last(), "mean": mean})
        })
        .collect();
    let (m0, m1) = (r.frames.first().map_or(0.0, |f| f.mass), last.map_or(0.0, |f| f.mass));
    json!({
        "step": r.step,
        "cells": r.stats.cells,
        "fluid_cells": r.stats.fluid_cells,
        "steps": r.stats.steps,
        "time_step": r.stats.time_step,
        "end_time": r.stats.end_time,
        "tau": r.stats.tau_plus,
        "tau_minus": r.stats.tau_minus,
        "magic": r.stats.magic,
        "velocity_scale": r.stats.velocity,
        "lattice_velocity": r.stats.lattice_velocity,
        "mach": r.stats.mach,
        "frames": r.frames.iter().map(|f| f.time).collect::<Vec<_>>(),
        "max_speed": max_speed,
        "mass": {"first": m0, "last": m1, "relative_change": if m0 != 0.0 { (m1 - m0) / m0 } else { 0.0 }},
        "forces": forces,
        "warnings": r.warnings,
    })
}

fn probe_fluid(r: &FluidResult, p: &Value) -> Result<Value> {
    if let Some(names) = opt::<Vec<String>>(p, "forces")? {
        if names.len() > MAX_PROBE_FORCES {
            return Err(EngineError::Params(format!("`forces` lists at most {MAX_PROBE_FORCES} walls")));
        }
        let out: Vec<Value> = names
            .iter()
            .map(|n| {
                let f =
                    r.forces.iter().find(|f| &f.name == n || f.body.as_ref().is_some_and(|b| format!("{b}/{}", f.name) == *n)).ok_or_else(|| {
                        EngineError::Params(format!("no wall `{n}` (box faces x_min … z_max, or obstacle faces by name or body/name)"))
                    })?;
                Ok(json!({"name": f.name, "body": f.body, "force": f.force}))
            })
            .collect::<Result<_>>()?;
        return Ok(json!({"step": r.step, "times": r.history_times, "forces": out}));
    }
    let pts = points(p, "points", MAX_PROBE)?;
    let frame = match opt::<usize>(p, "frame")? {
        Some(k) if k < r.frames.len() => k,
        Some(_) => return Err(EngineError::Params(format!("`frame` must be from 0 to {}", r.frames.len().saturating_sub(1)))),
        None => r.frames.len().saturating_sub(1),
    };
    let time = r.frames.get(frame).map(|f| f.time);
    let out: Vec<Value> = pts
        .iter()
        .map(|&pt| match r.probe(frame, pt) {
            Some((cell, velocity, pressure)) => {
                json!({"point": pt, "cell": cell, "solid": r.solid.get(cell), "velocity": velocity, "pressure": pressure})
            }
            None => json!({"point": pt, "cell": null}),
        })
        .collect();
    Ok(json!({"step": r.step, "frame": frame, "time": time, "points": out}))
}

fn probe(s: &mut Session, p: &Value) -> Result<Value> {
    let step = opt::<String>(p, "step")?.ok_or_else(|| EngineError::param("step", ProblemKind::Required))?;
    let result = s.result(&step).ok_or_else(|| {
        EngineError::Params(if s.latest_result(&step).is_some() {
            format!("the result of step `{step}` is stale (the model changed since it was solved); solve it again")
        } else {
            format!("step `{step}` has no result; solve it first")
        })
    })?;
    if let StepResult::Fluid(r) = result.as_ref() {
        return probe_fluid(r, p);
    }
    let (nodes, located) = match (p.get("nodes"), p.get("at")) {
        (Some(_), None) => (labels(p, "nodes", MAX_PROBE)?, None),
        (None, None) if p.get("set").is_some() => {
            let name = opt::<String>(p, "set")?.unwrap_or_default();
            let set = s.model.resolve_set(&name)?;
            if set.nodes.len() > MAX_PROBE {
                return Err(EngineError::Params(format!("set `{name}` has {} nodes; a probe takes at most {MAX_PROBE}", set.nodes.len())));
            }
            (set.nodes, None)
        }
        (None, Some(_)) => {
            let at = points(p, "at", MAX_PROBE)?;
            let found = nearest_nodes(&s.model, result.nodes(), &at)?;
            (found.iter().map(|(l, _, _)| *l).collect(), Some(found))
        }
        (None, None) if matches!(result.as_ref(), StepResult::Explicit(_)) || p.get("elements").is_some() => (Vec::new(), None),
        _ => return Err(EngineError::Params("give one of `nodes`, `at` or `set`".into())),
    };
    let mut out = match result.as_ref() {
        StepResult::Explicit(r) => {
            let elements = if p.get("elements").is_some() { labels(p, "elements", MAX_PROBE)? } else { Vec::new() };
            probe_frame(r, &step, &nodes, &elements, p)?
        }
        StepResult::Implicit(r) => probe_implicit(r, &step, &nodes, p)?,
        StepResult::Fluid(r) => probe_fluid(r, p)?,
    };
    // The largest von Mises stress among the probed nodes (static results).
    let peak = out.get("nodes").and_then(Value::as_array).and_then(|entries| {
        entries
            .iter()
            .filter_map(|e| Some((e.get("node")?.as_u64()?, e.get("von_mises")?.as_f64()?)))
            .fold(None, |best: Option<(u64, f64)>, (n, v)| if best.is_none_or(|b| v > b.1) { Some((n, v)) } else { best })
    });
    if let (Some((node, value)), Some(obj)) = (peak, out.as_object_mut()) {
        obj.insert("max_von_mises".into(), json!({"node": node, "value": value}));
    }
    if let (Some(found), Some(entries)) = (located, out.get_mut("nodes").and_then(Value::as_array_mut)) {
        for (entry, (_, xyz, distance)) in entries.iter_mut().zip(found) {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("xyz".into(), json!(xyz));
                obj.insert("distance".into(), json!(distance));
            }
        }
    }
    Ok(out)
}

/// The node with a result nearest to each point (lowest label on a tie), where it is and how far.
fn nearest_nodes(model: &fieldcraft_model::Model, with_result: &[u32], at: &[[f64; 3]]) -> Result<Vec<(u32, [f64; 3], f64)>> {
    let candidates: Vec<(u32, [f64; 3])> = with_result.iter().filter_map(|&l| Some((l, model.node(l)?.xyz))).collect();
    at.iter()
        .map(|p| {
            candidates
                .iter()
                .map(|&(l, x)| (l, x, ((x[0] - p[0]).powi(2) + (x[1] - p[1]).powi(2) + (x[2] - p[2]).powi(2)).sqrt()))
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .ok_or_else(|| EngineError::Params("the result has no nodes to probe".into()))
        })
        .collect()
}

/// The result of an implicit step at nodes.
fn probe_implicit(result: &ImplicitResult, step: &str, nodes: &[u32], p: &Value) -> Result<Value> {
    let elements = if p.get("elements").is_some() { labels(p, "elements", MAX_PROBE)? } else { Vec::new() };
    let locate = |label: u32| {
        result
            .locate(label)
            .ok_or_else(|| EngineError::Params(format!("node {label} has no result (it belongs to no solved element or coupled reference point)")))
    };
    let out: Vec<Value> = match result {
        ImplicitResult::Static(r) => nodes
            .iter()
            .map(|&l| {
                Ok(match locate(l)? {
                    Location::Point(k) => {
                        let p = r.reference_points.get(k);
                        json!({"node": l, "reference_point": p.map(|p| &p.name), "displacement": p.map(|p| p.displacement), "reaction": p.map(|p| p.reaction)})
                    }
                    Location::Node(i) => {
                        let mut v = json!({
                            "node": l,
                            "displacement": r.displacement.get(i),
                            "rotation": r.rotation.get(i),
                            "reaction": r.reaction.get(i),
                            "reaction_moment": r.reaction_moment.get(i),
                            "stress": r.stress.get(i),
                            "von_mises": r.von_mises.get(i),
                            "temperature": r.temperature.as_ref().and_then(|t| t.get(i)),
                        });
                        // The contacts the node is a secondary node of: pressure, gap, status.
                        let contact: Vec<Value> = r
                            .contacts
                            .iter()
                            .filter_map(|c| {
                                let k = c.nodes.binary_search(&l).ok()?;
                                let mut entry = json!({"contact": c.name, "pressure": c.pressure.get(k), "gap": c.gap.get(k), "status": c.status.get(k)});
                                if let (Some(shear), Some(slip)) = (c.shear.get(k), c.slip.get(k)) {
                                    entry["shear"] = json!(shear);
                                    entry["slip"] = json!(slip);
                                }
                                Some(entry)
                            })
                            .collect();
                        if !contact.is_empty() {
                            v["contact"] = Value::Array(contact);
                        }
                        v
                    }
                })
            })
            .collect::<Result<_>>()?,
        ImplicitResult::Frequency(r) => {
            if !elements.is_empty() {
                return Err(EngineError::Params("element results are kept for static and explicit steps".into()));
            }
            let mode = opt::<usize>(p, "mode")?.unwrap_or(1);
            let m = mode
                .checked_sub(1)
                .and_then(|k| r.modes.get(k))
                .ok_or_else(|| EngineError::Params(format!("`mode` must be from 1 to {}", r.modes.len())))?;
            nodes
                .iter()
                .map(|&l| {
                    Ok(match locate(l)? {
                        Location::Point(k) => {
                            json!({"node": l, "reference_point": r.reference_points.get(k).map(|p| &p.0), "shape": m.point_shape.get(k)})
                        }
                        Location::Node(i) => json!({"node": l, "shape": m.shape.get(i), "rotation": m.rotation.get(i)}),
                    })
                })
                .collect::<Result<_>>()?
        }
        ImplicitResult::Heat(r) => {
            let (time, t) = crate::results::heat_frame(r, crate::results::Pick { mode: 1, frame: opt::<usize>(p, "frame")? })?;
            let out = nodes
                .iter()
                .map(|&l| {
                    let i = r.nodes.binary_search(&l).map_err(|_| EngineError::Params(format!("node {l} has no temperature (it belongs to no solid element)")))?;
                    Ok(json!({"node": l, "temperature": t.get(i), "heat_flux": r.flux.get(i), "reaction": r.reaction.get(i)}))
                })
                .collect::<Result<Vec<Value>>>()?;
            return Ok(json!({"step": step, "time": time, "nodes": out}));
        }
    };
    let mut response = json!({"step": step, "nodes": out});
    if let ImplicitResult::Static(r) = result
        && !elements.is_empty()
    {
        let found: Vec<Value> = elements
            .iter()
            .map(|&l| element_result(r, l).ok_or_else(|| EngineError::Params(format!("element {l} has no result in step `{step}`"))))
            .collect::<Result<_>>()?;
        response["elements"] = Value::Array(found);
    }
    Ok(response)
}

/// The entry of a label-sorted result list.
fn by_label<T>(list: &[T], label: u32, key: impl Fn(&T) -> u32) -> Option<&T> {
    list.binary_search_by_key(&label, key).ok().and_then(|i| list.get(i))
}

/// The stored results of one element of a static step.
fn element_result(r: &StaticResult, label: u32) -> Option<Value> {
    if let Some(e) = by_label(&r.elements, label, |e| e.label) {
        let mut v = json!({"element": label, "kind": e.kind.name(), "stress": e.nodal});
        if !e.plastic_strain.is_empty() {
            v["plastic_strain"] = json!(e.plastic_strain);
        }
        return Some(v);
    }
    if let Some(e) = by_label(&r.shells, label, |e| e.label) {
        let s = &e.stress;
        return Some(
            json!({"element": label, "kind": e.kind.name(), "axes": s.axes, "forces": s.forces, "bottom": s.bottom, "middle": s.middle, "top": s.top}),
        );
    }
    let e = by_label(&r.beams, label, |e| e.label)?;
    Some(json!({"element": label, "kind": e.kind.name(), "axes": e.forces.axes, "ends": e.forces.ends}))
}

/// Displacement and velocity at nodes, and element results, in one field frame of an explicit
/// result.
fn probe_frame(r: &ExplicitResult, step: &str, nodes: &[u32], elements: &[u32], p: &Value) -> Result<Value> {
    let last = r.frames.len().saturating_sub(1);
    let k = opt::<usize>(p, "frame")?.unwrap_or(last);
    let f = r.frames.get(k).ok_or_else(|| EngineError::Params(format!("`frame` must be from 0 to {last}")))?;
    if nodes.is_empty() && elements.is_empty() {
        return Err(EngineError::Params("give `nodes`, `elements` or both".into()));
    }
    if !elements.is_empty() && f.stress.is_empty() {
        return Err(EngineError::Params(format!("step `{step}` does not output stresses")));
    }
    let element_values: Vec<Value> = elements
        .iter()
        .map(|&l| {
            let i =
                r.elements.binary_search(&l).map_err(|_| EngineError::Params(format!("element {l} has no result (it is not a solved element)")))?;
            Ok(json!({
                "element": l,
                "stress": f.stress.get(i),
                "von_mises": f.stress.get(i).map(fieldcraft_fem::von_mises),
                "strain": f.strain.get(i),
                "plastic_strain": f.plastic_strain.get(i),
                "damage": f.damage.get(i),
                "temperature": f.temperature.get(i),
                "eroded": f.failed.get(i),
            }))
        })
        .collect::<Result<_>>()?;
    if nodes.is_empty() {
        return Ok(json!({"step": step, "frame": k, "time": f.time, "elements": element_values}));
    }
    if f.displacement.is_empty() {
        return Err(EngineError::Params(format!("step `{step}` does not output displacements")));
    }
    let out: Vec<Value> = nodes
        .iter()
        .map(|&l| {
            let i =
                r.nodes.binary_search(&l).map_err(|_| EngineError::Params(format!("node {l} has no result (it belongs to no solved element)")))?;
            Ok(json!({"node": l, "displacement": f.displacement.get(i), "velocity": f.velocity.get(i)}))
        })
        .collect::<Result<_>>()?;
    Ok(json!({"step": step, "frame": k, "time": f.time, "nodes": out, "elements": element_values}))
}

#[cfg(test)]
pub(crate) mod tests {
    /// `file.sample` opens the cantilever with nothing to save; `solve.run` solves both its steps
    /// in order and the results view comes up on the first.
    #[test]
    fn samples_open_and_solve_run_solves_every_step() {
        use serde_json::json;
        let mut s = Session::default();
        s.execute("mesh.node.add", &json!({"label": 1, "xyz": [0, 0, 0]})).unwrap();
        let r = s.execute("file.sample", &json!({"name": "cantilever"})).unwrap();
        assert_eq!(r["sample"], "cantilever");
        assert!(!s.is_dirty() && s.path.is_none());
        assert!(s.model.node(1).is_some_and(|n| n.xyz != [0.0, 0.0, 0.0]) || s.model.nodes().len() > 1);
        assert!(s.execute("file.sample", &json!({"name": "nope"})).unwrap_err().to_string().contains("cantilever"));
        let r = s.execute("solve.run", &json!({})).unwrap();
        let steps: Vec<&str> = r["steps"].as_array().unwrap().iter().map(|x| x["step"].as_str().unwrap()).collect();
        assert_eq!(steps, ["Load", "Modes"]);
        assert!(s.result("Load").is_some() && s.result("Modes").is_some());
        assert_eq!(s.results_view.as_ref().map(|v| v.step.as_str()), Some("Load"));
        s.execute("file.sample", &json!({"name": "impact"})).unwrap();
        assert!(s.results_view.is_none(), "a new model shows no old results");
        s.execute("solve.run", &json!({})).unwrap();
        assert!(s.result("Impact").is_some());
        let mut empty = Session::default();
        assert!(empty.execute("solve.run", &json!({})).unwrap_err().to_string().contains("Needs a mesh"));
    }

    use super::*;

    /// A cantilever 10 × 1 × 1 of ten hex8 elements with incompatible modes, clamped at x = 0,
    /// a total force of 1 down on the free end; a static and a frequency step.
    pub(crate) fn cantilever(s: &mut Session) {
        let mut script = Vec::new();
        let label = |i: usize, j: usize, k: usize| 1 + i + 11 * (j + 2 * k);
        for k in 0..2 {
            for j in 0..2 {
                for i in 0..11 {
                    script.push(json!({"command": "mesh.node.add", "params": {"label": label(i, j, k), "xyz": [i, j, k]}}));
                }
            }
        }
        for i in 0..10 {
            let nodes = [
                label(i, 0, 0),
                label(i + 1, 0, 0),
                label(i + 1, 1, 0),
                label(i, 1, 0),
                label(i, 0, 1),
                label(i + 1, 0, 1),
                label(i + 1, 1, 1),
                label(i, 1, 1),
            ];
            script.push(json!({"command": "mesh.element.add", "params": {"label": i + 1, "kind": "hex8", "nodes": nodes}}));
        }
        let root: Vec<usize> = (0..4).map(|n| label(0, n % 2, n / 2)).collect();
        let tip: Vec<usize> = (0..4).map(|n| label(10, n % 2, n / 2)).collect();
        script.extend([
            json!({"command": "model.set.create", "params": {"name": "all", "elements": (1..=10).collect::<Vec<_>>()}}),
            json!({"command": "model.set.create", "params": {"name": "root", "nodes": root}}),
            json!({"command": "model.set.create", "params": {"name": "tip", "nodes": tip}}),
            json!({"command": "model.material.create", "params": {"name": "Mat", "elastic": {"type": "isotropic", "young": 1000, "poisson": 0}, "density": 1}}),
            json!({"command": "model.section.create", "params": {"name": "bar", "set": "all", "material": "Mat", "kind": {"type": "solid"}, "formulation": {"incompatible_modes": true}}}),
            json!({"command": "model.boundary_condition.create", "params": {"name": "clamp", "type": "fixed", "target": {"set": "root"}}}),
            json!({"command": "model.load.create", "params": {"name": "push", "type": "force", "target": {"set": "tip"}, "vector": [0, 0, -1]}}),
            json!({"command": "model.step.create", "params": {"name": "Bend", "procedure": {"type": "static"}, "boundary_conditions": ["clamp"], "loads": ["push"]}}),
            json!({"command": "model.step.create", "params": {"name": "Modes", "procedure": {"type": "frequency", "modes": 4}, "boundary_conditions": ["clamp"]}}),
        ]);
        s.run_script(&Value::Array(script)).unwrap();
    }

    #[test]
    fn solve_keep_and_probe_results() {
        let mut s = Session::default();
        cantilever(&mut s);
        let journal = s.journal.len();
        let r = s.execute("solve.static", &json!({})).unwrap();
        assert_eq!(r["step"], "Bend");
        // Euler–Bernoulli P L³ / (3 E I) = 4, plus shear; the clamp and one element through the
        // depth keep it within a few percent.
        let d = r["max_displacement"]["value"].as_f64().unwrap();
        assert!((d - 4.024).abs() / 4.024 < 0.03, "{r}");
        assert!((r["reaction_total"][2].as_f64().unwrap() - 1.0).abs() < 1e-9, "{r}");
        assert_eq!(s.journal.len(), journal, "solving does not change the model");
        let probe = s.execute("solve.probe", &json!({"step": "Bend", "nodes": [11, 1]})).unwrap();
        assert!(probe["nodes"][0]["displacement"][2].as_f64().unwrap() < -3.8, "{probe}");
        assert!(probe["nodes"][1]["reaction"][2].as_f64().unwrap() != 0.0, "{probe}");
        // By location: the nearest node, with where it is.
        let at = s.execute("solve.probe", &json!({"step": "Bend", "at": [[10.1, -0.2, 0.1], [0, 0, 0]]})).unwrap();
        assert_eq!((at["nodes"][0]["node"].clone(), at["nodes"][1]["node"].clone()), (json!(11), json!(1)), "{at}");
        assert_eq!(at["nodes"][0]["displacement"], probe["nodes"][0]["displacement"]);
        assert_eq!(at["nodes"][0]["xyz"], json!([10.0, 0.0, 0.0]));
        assert!((at["nodes"][0]["distance"].as_f64().unwrap() - 0.06f64.sqrt()).abs() < 1e-12);
        for bad in [json!({"step": "Bend"}), json!({"step": "Bend", "nodes": [1], "at": [[0, 0, 0]]}), json!({"step": "Bend", "at": [[0, 0]]})] {
            assert!(s.execute("solve.probe", &bad).is_err(), "{bad}");
        }

        let f = s.execute("solve.frequency", &json!({"mass": "lumped"})).unwrap();
        assert_eq!(f["modes"].as_array().unwrap().len(), 4, "{f}");
        let shape = s.execute("solve.probe", &json!({"step": "Modes", "nodes": [44], "mode": 2})).unwrap();
        assert!(shape["nodes"][0]["shape"].is_array(), "{shape}");
        assert_eq!(s.solved_steps().count(), 2);

        // Results describe the model they were solved for: an edit clears them, a failed command
        // keeps them.
        assert!(s.execute("model.material.edit", &json!({"name": "Mat", "elastic": {"type": "isotropic", "young": -1}})).is_err());
        assert_eq!(s.solved_steps().count(), 2);
        s.execute("model.material.edit", &json!({"name": "Mat", "elastic": {"type": "isotropic", "young": 2000, "poisson": 0}, "density": 1}))
            .unwrap();
        assert_eq!(s.solved_steps().count(), 0);
        // Undo returns to the solved state, and its results are current again.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.solved_steps().count(), 2);
        s.execute("edit.redo", &json!({})).unwrap();
        let e = s.execute("solve.probe", &json!({"step": "Bend", "nodes": [1]})).unwrap_err().to_string();
        assert!(e.contains("stale"), "{e}");
    }

    #[test]
    fn solve_hostile_params_never_panic() {
        let mut s = Session::default();
        cantilever(&mut s);
        s.execute("solve.static", &json!({})).unwrap();
        s.execute("solve.frequency", &json!({})).unwrap();
        let hostile = [
            json!(null),
            json!({"step": 7}),
            json!({"step": "Nope"}),
            json!({"step": "Modes"}),
            json!({"step": "Bend", "max_dofs": 0}),
            json!({"step": "Bend", "max_dofs": 1e300}),
            json!({"step": "Bend", "max_dofs": 3}),
            json!({"step": "Bend", "max_factor_entries": -1}),
            json!({"step": "Bend", "max_factor_entries": 10}),
            json!({"step": "Modes", "max_iterations": 1}),
            json!({"step": "Modes", "max_iterations": 7}),
            json!({"step": "Modes", "mass": "heavy"}),
            json!({"step": "Modes", "shift": 1e308}),
            json!({"step": "Modes", "shift": -1e308}),
            json!({"step": "Modes", "shift": 5000.0}),
            json!({"step": "Modes", "shift": "x"}),
            json!({"step": "Modes", "nodes": [1], "mode": 0}),
            json!({"step": "Modes", "nodes": [1], "mode": 1e300}),
            json!({"step": "Bend", "nodes": [0, 99999999, 4294967296u64]}),
            json!({"step": "Bend", "nodes": vec![1; 20_000]}),
            json!({"step": "Bend", "threads": 0}),
            json!({"step": "Bend", "threads": -3}),
            json!({"step": "Bend", "threads": 257}),
            json!({"step": "Bend", "threads": 1e300}),
            json!({"step": "Bend", "threads": "all"}),
            json!({"step": "Modes", "threads": 18446744073709551615u64}),
            json!({"step": "Modes", "threads": 2.5}),
        ];
        for id in ["solve.static", "solve.frequency", "solve.probe"] {
            for p in &hostile {
                let mut t = s.clone();
                let before = t.model.clone();
                let results = t.solved_steps().count();
                if t.execute(id, p).is_err() {
                    assert_eq!(t.model, before);
                    assert_eq!(t.solved_steps().count(), results, "{id} {p}");
                }
            }
        }
    }

    /// Couette flow between y = 0 and y = 1 (eight cells), the top wall sliding at 0.1 along x,
    /// periodic in x and z: material, domain and fluid step through commands.
    pub(crate) fn couette(s: &mut Session) {
        let periodic = json!({"type": "periodic"});
        let script = json!([
            {"command": "model.material.create", "params": {"name": "Oil", "density": 1.0, "fluid": {"type": "newtonian", "viscosity": 0.1}}},
            {"command": "model.fluid_domain.create", "params": {"name": "Gap", "material": "Oil", "min": [0, 0, 0], "max": [0.125, 1, 0.125], "spacing": 0.125,
                "faces": {"x_min": periodic, "x_max": periodic, "z_min": periodic, "z_max": periodic}}},
            {"command": "model.fluid_domain.boundary", "params": {"domain": "Gap", "at": "y_max", "condition": {"type": "moving_wall", "velocity": [0.1, 0, 0]}}},
            {"command": "model.step.create", "params": {"name": "Shear", "procedure": {"type": "fluid", "domain": "Gap", "end_time": 50, "frames": 4}}},
        ]);
        s.run_script(&script).unwrap();
    }

    #[test]
    fn solve_a_fluid_step_and_probe_its_fields_and_wall_forces() {
        let mut s = Session::default();
        couette(&mut s);
        let journal = s.journal.len();
        let r = s.execute("solve.fluid", &json!({})).unwrap();
        assert_eq!(r["step"], "Shear");
        assert_eq!(r["frames"].as_array().unwrap().len(), 5, "{r}");
        assert!((r["tau"].as_f64().unwrap() - 1.0).abs() < 1e-3, "the automatic time step keeps τ ≤ 1: {r}");
        assert!(r["mass"]["relative_change"].as_f64().unwrap().abs() < 1e-12, "{r}");
        assert_eq!(s.journal.len(), journal, "solving does not change the model");
        // The wall shear μ U / H over the wall's area 0.125².
        let bottom = r["forces"].as_array().unwrap().iter().find(|f| f["name"] == "y_min").unwrap();
        let expected = 0.1 * 0.1 / 1.0 * 0.125 * 0.125;
        assert!((bottom["mean"][0].as_f64().unwrap() / expected - 1.0).abs() < 1e-3, "{bottom}");

        let probe = s.execute("solve.probe", &json!({"step": "Shear", "points": [[0.06, 0.5625, 0.06], [5, 5, 5]]})).unwrap();
        let u = probe["points"][0]["velocity"][0].as_f64().unwrap();
        assert!((u - 0.05625).abs() < 1e-5, "{probe}");
        assert!(probe["points"][1]["cell"].is_null());
        let first = s.execute("solve.probe", &json!({"step": "Shear", "points": [[0.06, 0.5625, 0.06]], "frame": 0})).unwrap();
        assert_eq!(first["points"][0]["velocity"][0], 0.0, "the initial frame is at rest");
        let h = s.execute("solve.probe", &json!({"step": "Shear", "forces": ["y_min", "y_max"]})).unwrap();
        assert_eq!(h["forces"].as_array().unwrap().len(), 2);
        assert_eq!(h["times"].as_array().unwrap().len(), h["forces"][0]["force"].as_array().unwrap().len());

        // An edit makes the result stale; the step solves again.
        s.execute(
            "model.fluid_domain.boundary",
            &json!({"domain": "Gap", "at": "y_max", "condition": {"type": "moving_wall", "velocity": [0.2, 0, 0]}}),
        )
        .unwrap();
        assert!(s.execute("solve.probe", &json!({"step": "Shear", "points": [[0, 0, 0]]})).is_err());
        // The domain cannot go while a step uses it; the material cannot lose its fluid behaviour.
        assert!(s.execute("model.fluid_domain.delete", &json!({"name": "Gap"})).is_err());
        assert!(s.execute("model.material.edit", &json!({"name": "Oil", "density": 1.0})).is_err());
        // A structural step is not a fluid one, and a fluid step is not written to decks.
        assert!(s.execute("solve.static", &json!({"step": "Shear"})).is_err());
        for format in ["abaqus", "nastran", "lsdyna"] {
            let e = s.execute("deck.write", &json!({"format": format, "path": "/no/such/dir/x.deck"})).unwrap_err().to_string();
            assert!(e.contains("fluid step"), "{format}: {e}");
        }
    }

    #[test]
    fn a_fluid_domain_takes_its_obstacles_from_the_open_design() {
        use fieldcraft_geometry::sample::{Plate, plate_design};
        let dir = std::env::temp_dir().join(format!("fieldcraft-fluid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plate.solidcraft");
        std::fs::write(&path, plate_design(&Plate::default()).unwrap()).unwrap();
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": path.to_str().unwrap()})).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let bodies = s.execute("geometry.bodies", &json!({})).unwrap();
        let [lo, hi] = [0, 1].map(|k| bodies[0]["bbox"][k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>());
        // A box 20 mm larger than the plate on every side, 5 mm cells: a viscous liquid (ν = 100 mm²/s) flowing along x.
        let min: Vec<f64> = lo.iter().map(|v| (v / 5.0).floor() * 5.0 - 20.0).collect();
        let max: Vec<f64> = hi.iter().map(|v| (v / 5.0).ceil() * 5.0 + 20.0).collect();
        let script = json!([
            {"command": "model.material.create", "params": {"name": "Water", "density": 1e-9, "fluid": {"type": "newtonian", "viscosity": 1e-7}}},
            {"command": "model.fluid_domain.create", "params": {"name": "Tank", "material": "Water", "min": min, "max": max, "spacing": 5, "obstacles": ["Body1"],
                "faces": {"x_min": {"type": "velocity", "velocity": [10, 0, 0]}, "x_max": {"type": "pressure", "pressure": 0}}}},
            {"command": "model.step.create", "params": {"name": "Flow", "procedure": {"type": "fluid", "domain": "Tank", "steps": 60, "frames": 2}}},
        ]);
        s.run_script(&script).unwrap();
        let r = s.execute("solve.fluid", &json!({"step": "Flow"})).unwrap();
        assert!(r["fluid_cells"].as_u64().unwrap() < r["cells"].as_u64().unwrap(), "{r}");
        let on_plate: Vec<&Value> = r["forces"].as_array().unwrap().iter().filter(|f| f["body"] == "Body1").collect();
        assert!(!on_plate.is_empty(), "{r}");
        let faces = bodies[0]["faces"].as_array().unwrap();
        for f in &on_plate {
            assert!(faces.iter().any(|g| g["name"] == f["name"]), "forces are reported per named face: {f}");
        }
        let drag: f64 = on_plate.iter().map(|f| f["final"][0].as_f64().unwrap()).sum();
        assert!(drag > 0.0, "the flow pushes the plate downstream: {r}");
        // An obstacle that is not in the design.
        s.execute(
            "model.fluid_domain.edit",
            &json!({"name": "Tank", "material": "Water", "min": min, "max": max, "spacing": 5, "obstacles": ["Nope"]}),
        )
        .unwrap();
        assert!(s.execute("solve.fluid", &json!({})).is_err());
    }

    #[test]
    fn solve_fluid_hostile_params_never_panic() {
        let mut s = Session::default();
        couette(&mut s);
        s.execute("solve.fluid", &json!({"step": "Shear"})).unwrap();
        let hostile = [
            json!(null),
            json!({"step": 7}),
            json!({"step": "Nope"}),
            json!({"step": "Shear", "max_cells": 0}),
            json!({"step": "Shear", "max_cells": 1e300}),
            json!({"step": "Shear", "max_cells": 3}),
            json!({"step": "Shear", "max_steps": 3}),
            json!({"step": "Shear", "max_steps": -1}),
            json!({"step": "Shear", "max_cell_updates": 10}),
            json!({"step": "Shear", "max_cell_updates": 0.5}),
            json!({"step": "Shear", "max_frame_bytes": 16}),
            json!({"step": "Shear", "max_frame_bytes": "lots"}),
            json!({"step": "Shear", "points": [[1e308, -1e308, 0]]}),
            json!({"step": "Shear", "points": [[f64::MAX, 0, 0]], "frame": 1e300}),
            json!({"step": "Shear", "points": [[0, 0, 0]], "frame": 99}),
            json!({"step": "Shear", "points": null}),
            json!({"step": "Shear", "points": vec![[0, 0, 0]; 20_000]}),
            json!({"step": "Shear", "forces": ["nope"]}),
            json!({"step": "Shear", "forces": vec!["y_min"; 1000]}),
            json!({"step": "Shear", "forces": "y_min"}),
            json!({"step": "Shear", "nodes": [1]}),
            json!({"step": "Shear", "threads": 0}),
            json!({"step": "Shear", "threads": 100000}),
            json!({"step": "Shear", "threads": null}),
        ];
        for id in ["solve.fluid", "solve.probe", "solve.static", "solve.frequency"] {
            for p in &hostile {
                let mut t = s.clone();
                let before = t.model.clone();
                let results = t.solved_steps().count();
                if t.execute(id, p).is_err() {
                    assert_eq!(t.model, before);
                    assert_eq!(t.solved_steps().count(), results, "{id} {p}");
                }
            }
        }
    }

    #[test]
    fn solve_and_probe_a_coupled_reference_point() {
        // The defined bar: a force on a reference point kinematically coupled to the free end.
        let mut s = Session::default();
        crate::cmd::define::tests::defined_bar(&mut s);
        let r = s.execute("solve.static", &json!({"step": "Bend"})).unwrap();
        let rp = &r["reference_points"][0];
        assert_eq!(rp["name"], "RP", "{r}");
        assert!(rp["displacement"][2].as_f64().unwrap() < 0.0, "{r}");
        assert!((r["reaction_total"][2].as_f64().unwrap() - 100.0).abs() < 1e-6, "{r}");
        let label = rp["label"].as_u64().unwrap();
        let probe = s.execute("solve.probe", &json!({"step": "Bend", "nodes": [label, 1]})).unwrap();
        assert_eq!(probe["nodes"][0]["reference_point"], "RP", "{probe}");
        assert_eq!(probe["nodes"][0]["displacement"].as_array().unwrap().len(), 6, "{probe}");
        assert!(probe["nodes"][1]["reaction"].is_array(), "{probe}");
        s.execute("solve.frequency", &json!({"step": "Modes"})).unwrap();
        let shape = s.execute("solve.probe", &json!({"step": "Modes", "nodes": [label], "mode": 1})).unwrap();
        assert_eq!(shape["nodes"][0]["shape"].as_array().unwrap().len(), 6, "{shape}");
        for p in [json!({"step": "Bend", "nodes": [label, label, 0]}), json!({"step": "Modes", "nodes": [label], "mode": 99})] {
            assert!(s.execute("solve.probe", &p).is_err(), "{p}");
        }
    }

    /// A cantilever L × 1 × 1 along x of `n[0]` × `n[1]` × `n[1]` hex8 elements, E = 1000, ν = 0,
    /// ρ = 1, clamped at x = 0; one node set per cross-section (`x0`, `x1`, …) and the tip's
    /// corner node `tip`; a frequency step `Modes` and the section `beam`.
    fn explicit_cantilever(s: &mut Session, length: f64, n: [usize; 2]) {
        use fieldcraft_model::{ElementKind, Set};
        let [nx, ny] = n;
        let label = |i: usize, j: usize, k: usize| (1 + i + (nx + 1) * (j + (ny + 1) * k)) as u32;
        for k in 0..=ny {
            for j in 0..=ny {
                for i in 0..=nx {
                    s.model.set_node(label(i, j, k), [length * i as f64 / nx as f64, j as f64 / ny as f64, k as f64 / ny as f64]).unwrap();
                }
            }
        }
        let corners = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];
        let mut e = 1;
        for k in 0..ny {
            for j in 0..ny {
                for i in 0..nx {
                    s.model.set_element(e, ElementKind::Hex8, corners.iter().map(|c| label(i + c[0], j + c[1], k + c[2])).collect()).unwrap();
                    e += 1;
                }
            }
        }
        for i in 0..=nx {
            let nodes = (0..=ny).flat_map(|k| (0..=ny).map(move |j| label(i, j, k))).collect();
            s.model.add_set(&format!("x{i}"), Set { nodes, ..Set::default() }).unwrap();
        }
        s.model.add_set("tip", Set { nodes: vec![label(nx, 0, 0)], ..Set::default() }).unwrap();
        s.model.add_set("all", Set { elements: (1..e).collect(), ..Set::default() }).unwrap();
        let script = json!([
            {"command": "model.material.create", "params": {"name": "Mat", "elastic": {"type": "isotropic", "young": 1000, "poisson": 0}, "density": 1}},
            {"command": "model.section.create", "params": {"name": "beam", "set": "all", "material": "Mat", "kind": {"type": "solid"}, "formulation": {"incompatible_modes": true}}},
            {"command": "model.boundary_condition.create", "params": {"name": "clamp", "type": "fixed", "target": {"set": "x0"}}},
            {"command": "model.step.create", "params": {"name": "Modes", "procedure": {"type": "frequency", "modes": 1}, "boundary_conditions": ["clamp"]}},
        ]);
        s.run_script(&script).unwrap();
    }

    /// The stored result of a step, hashed bit for bit (Debug prints every float exactly).
    fn result_hash(s: &Session, step: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        format!("{:?}", s.result(step).unwrap()).hash(&mut h);
        h.finish()
    }

    /// Implicit (static, frequency), explicit and lattice Boltzmann solves large enough for every
    /// parallel path (factorization tasks and tiles, element rounds, cell blocks) give the same
    /// bits on 1, 2 and 7 threads.
    #[test]
    fn results_do_not_depend_on_the_thread_count() {
        let solve_all = |threads: usize| -> Vec<u64> {
            let mut s = Session::default();
            explicit_cantilever(&mut s, 10.0, [30, 6]);
            let script = json!([
                {"command": "model.load.create", "params": {"name": "push", "type": "force", "target": {"set": "x30"}, "vector": [0, 0.3, -1]}},
                {"command": "model.step.create", "params": {"name": "Bend", "procedure": {"type": "static"}, "boundary_conditions": ["clamp"], "loads": ["push"]}},
            ]);
            s.run_script(&script).unwrap();
            s.execute("solve.static", &json!({"step": "Bend", "threads": threads})).unwrap();
            s.execute("solve.frequency", &json!({"step": "Modes", "threads": threads})).unwrap();
            let mut hashes = vec![result_hash(&s, "Bend"), result_hash(&s, "Modes")];
            let step = json!({
                "type": "explicit_dynamic", "end_time": 0.15, "output_interval": 0.05,
                "initial_velocities": [{"set": "x30", "velocity": [0, 0, -2]}], "hourglass": {"type": "stiffness"}
            });
            let script = json!([
                {"command": "model.section.edit", "params": {"name": "beam", "set": "all", "material": "Mat", "kind": {"type": "solid"}, "formulation": {"integration": "reduced"}}},
                {"command": "model.step.create", "params": {"name": "Hit", "procedure": step, "boundary_conditions": ["clamp"]}},
            ]);
            s.run_script(&script).unwrap();
            s.execute("solve.explicit", &json!({"step": "Hit", "threads": threads})).unwrap();
            hashes.push(result_hash(&s, "Hit"));
            let mut f = Session::default();
            let periodic = json!({"type": "periodic"});
            let script = json!([
                {"command": "model.material.create", "params": {"name": "Oil", "density": 1.0, "fluid": {"type": "newtonian", "viscosity": 0.1}}},
                {"command": "model.fluid_domain.create", "params": {"name": "Gap", "material": "Oil", "min": [0, 0, 0], "max": [5, 1, 2], "spacing": 0.125,
                    "faces": {"z_min": periodic, "z_max": periodic}}},
                {"command": "model.fluid_domain.boundary", "params": {"domain": "Gap", "at": "y_max", "condition": {"type": "moving_wall", "velocity": [0.1, 0, 0.02]}}},
                {"command": "model.step.create", "params": {"name": "Shear", "procedure": {"type": "fluid", "domain": "Gap", "end_time": 2, "frames": 2}}},
            ]);
            f.run_script(&script).unwrap();
            f.execute("solve.fluid", &json!({"step": "Shear", "threads": threads})).unwrap();
            hashes.push(result_hash(&f, "Shear"));
            hashes
        };
        let one = solve_all(1);
        for threads in [2, 7] {
            assert_eq!(solve_all(threads), one, "{threads} threads");
        }
        let mut s = Session::default();
        cantilever(&mut s);
        for bad in [json!(0), json!(257), json!(-1), json!(1.5), json!("8")] {
            let e = s.execute("solve.static", &json!({"threads": bad})).unwrap_err().to_string();
            assert!(e.contains("`threads` must be a whole number from 1 to 256"), "{e}");
        }
        s.limits.threads = 3;
        s.execute("solve.static", &json!({})).unwrap();
    }

    /// Upward zero crossings of a sampled signal, linearly interpolated.
    fn crossings(time: &[f64], x: &[f64]) -> Vec<f64> {
        let mut out = Vec::new();
        for k in 1..x.len() {
            let (a, b) = (x[k - 1], x[k]);
            if a < 0.0 && b >= 0.0 {
                out.push(time[k - 1] + (time[k] - time[k - 1]) * (-a) / (b - a));
            }
        }
        out
    }

    /// Free vibration of a cantilever after an initial velocity in the shape of its static
    /// deflection: the period of the explicit run (one-point hex8) against the first natural
    /// frequency of the implicit modal solver (incompatible-mode hex8, lumped mass) on the same
    /// mesh. The one-point element's mean strain misses the linear variation of the bending
    /// strain through each element: with n elements through the depth it keeps 1 − 1/n² of the
    /// bending energy, so without hourglass stiffness (viscous control) the period is longer by
    /// (1 − 1/n²)^{−½} (3.3% for n = 4); the stiffness form restores part of it.
    #[test]
    #[ignore = "heavy: cargo xtask test --heavy (CI heavy job)"]
    fn explicit_cantilever_period_matches_the_modal_frequency() {
        let (length, n) = (10.0, [40usize, 4usize]);
        let mut s = Session::default();
        explicit_cantilever(&mut s, length, n);
        let modes = s.execute("solve.frequency", &json!({"mass": "lumped"})).unwrap();
        let period = 1.0 / modes["modes"][0]["frequency"].as_f64().unwrap();
        // Euler–Bernoulli: f₁ = 1.875² / (2π L²) √(E I / (ρ A)), I/A = 1/12.
        let euler = 2.0 * std::f64::consts::PI * length * length / 1.875104f64.powi(2) / (1000.0f64 / 12.0).sqrt();
        assert!((period / euler - 1.0).abs() < 0.01, "{period} vs {euler}");
        let section = json!({"name": "beam", "set": "all", "material": "Mat", "kind": {"type": "solid"}, "formulation": {"integration": "reduced"}});
        s.execute("model.section.edit", &section).unwrap();
        // v(x) ∝ x²(3L − x): the static deflection of a tip load, close to the first mode.
        let velocities: Vec<Value> = (0..=n[0])
            .map(|i| {
                let x = length * i as f64 / n[0] as f64;
                json!({"set": format!("x{i}"), "velocity": [0, 0, 0.01 * x * x * (3.0 * length - x) / (2.0 * length.powi(3))]})
            })
            .collect();
        let one_point = period / (1.0 - 1.0 / (n[1] * n[1]) as f64).sqrt();
        let mut measured = Vec::new();
        for hourglass in ["viscous", "stiffness"] {
            let step = json!({
                "type": "explicit_dynamic", "end_time": 4.2 * period, "history_interval": period / 400.0, "output_interval": period,
                "history_nodes": "tip", "initial_velocities": velocities, "hourglass": {"type": hourglass}
            });
            let name = format!("Swing-{hourglass}");
            s.execute("model.step.create", &json!({"name": name, "procedure": step, "boundary_conditions": ["clamp"]})).unwrap();
            let r = s.execute("solve.explicit", &json!({"step": name})).unwrap();
            let h = s.execute("solve.history", &json!({"step": name})).unwrap();
            let time: Vec<f64> = h["time"].as_array().unwrap().iter().map(|t| t.as_f64().unwrap()).collect();
            let uz: Vec<f64> = h["nodes"][0]["displacement"].as_array().unwrap().iter().map(|d| d[2].as_f64().unwrap()).collect();
            // The tip starts at zero moving up: it crosses zero upwards once per period after that.
            let ups = crossings(&time, &uz);
            let t = (ups[ups.len() - 1] - ups[0]) / (ups.len() - 1) as f64;
            eprintln!(
                "cantilever {n:?}, {hourglass} hourglass: explicit period {t:.5} vs modal {period:.5} ({:+.3}%), one-point estimate {one_point:.5} ({:+.3}%), Euler–Bernoulli {euler:.5}; energy balance {:.1e}, {} increments",
                100.0 * (t / period - 1.0),
                100.0 * (t / one_point - 1.0),
                r["energy_balance"].as_f64().unwrap(),
                r["increments"]
            );
            assert!(ups.len() >= 4, "{ups:?}");
            assert!(r["energy_balance"].as_f64().unwrap() < 0.01, "{r}");
            assert!(r["warnings"].as_array().unwrap().iter().all(|w| w.as_str().unwrap().contains("unit system")), "{r}");
            measured.push(t);
        }
        // Viscous control adds no stiffness: the one-point estimate holds closely.
        assert!((measured[0] / one_point - 1.0).abs() < 0.005, "{measured:?} vs {one_point}");
        // The stiffness form lies between it and the modal period, and within 3% of the latter.
        assert!(measured[1] > period && measured[1] < measured[0], "{measured:?}");
        assert!((measured[1] / period - 1.0).abs() < 0.03, "{measured:?} vs {period}");
    }

    /// A small impact model through commands: a hex8 bar moving onto a rigid wall.
    fn explicit_impact(s: &mut Session) {
        explicit_cantilever(s, 4.0, [8, 1]);
        let step = json!({
            "type": "explicit_dynamic", "end_time": 0.2, "history_interval": 0.002, "output_interval": 0.05, "history_nodes": "tip",
            "initial_velocities": [{"set": "all_nodes", "velocity": [-0.1, 0, 0]}],
            "rigid_walls": [{"set": "x0", "point": [0, 0, 0], "normal": [1, 0, 0]}]
        });
        let nodes: Vec<u32> = s.model.nodes().keys().copied().collect();
        let script = json!([
            {"command": "model.set.create", "params": {"name": "all_nodes", "nodes": nodes}},
            {"command": "model.section.edit", "params": {"name": "beam", "set": "all", "material": "Mat", "kind": {"type": "solid"}, "formulation": {"integration": "reduced"}}},
            {"command": "model.step.create", "params": {"name": "Impact", "procedure": step}},
        ]);
        s.run_script(&script).unwrap();
    }

    #[test]
    fn explicit_results_are_kept_probed_and_guarded() {
        let mut s = Session::default();
        explicit_impact(&mut s);
        let journal = s.journal.len();
        let r = s.execute("solve.explicit", &json!({"step": "Impact"})).unwrap();
        assert_eq!((r["completed"].as_bool(), r["frames"].as_u64()), (Some(true), Some(5)), "{r}");
        // Eight elements, so eight controlling ones; on this coarse mesh the impact excites modes
        // near the stability limit, whose energy the integer-step account measures only roughly.
        assert_eq!(r["controlling_elements"].as_array().unwrap().len(), 8, "{r}");
        assert!(r["energy_balance"].as_f64().unwrap() < 0.1, "{r}");
        assert_eq!(s.journal.len(), journal, "solving does not change the model");
        let probe = s.execute("solve.probe", &json!({"step": "Impact", "nodes": [1, 9], "frame": 0})).unwrap();
        assert_eq!(probe["nodes"][1]["velocity"][0].as_f64(), Some(-0.1), "{probe}");
        let last = s.execute("solve.probe", &json!({"step": "Impact", "nodes": [9]})).unwrap();
        assert_eq!(last["frame"], 4, "{last}");
        let el = s.execute("solve.probe", &json!({"step": "Impact", "elements": [1, 8]})).unwrap();
        let e = &el["elements"][0];
        assert!(e["stress"].as_array().unwrap().len() == 6 && e["von_mises"].as_f64().unwrap() >= 0.0 && e["eroded"] == false, "{el}");
        assert!(e["plastic_strain"].is_null(), "an elastic model has no plastic strain: {el}");
        assert_eq!(r["eroded"].as_array().map(Vec::len), Some(0), "{r}");
        let h = s.execute("solve.history", &json!({"step": "Impact", "every": 2})).unwrap();
        assert_eq!(h["time"].as_array().unwrap().len(), h["energies"].as_array().unwrap().len());
        assert!(h["nodes"][0]["velocity"].as_array().unwrap().len() > 5, "{h}");
        assert_eq!(s.solved_steps().collect::<Vec<_>>(), ["Impact"]);
        assert!(s.execute("solve.history", &json!({"step": "Modes"})).is_err(), "not solved");

        let hostile = [
            json!(null),
            json!({"step": 7}),
            json!({"step": "Nope"}),
            json!({"step": "Modes"}),
            json!({"step": "Impact", "max_dofs": 0}),
            json!({"step": "Impact", "max_dofs": 5}),
            json!({"step": "Impact", "max_increments": 0}),
            json!({"step": "Impact", "max_increments": 3}),
            json!({"step": "Impact", "max_increments": -1}),
            json!({"step": "Impact", "max_increments": 1e300}),
            json!({"step": "Impact", "max_output_values": 1}),
            json!({"step": "Impact", "every": 0}),
            json!({"step": "Impact", "every": -3}),
            json!({"step": "Impact", "every": 1e300}),
            json!({"step": "Impact", "every": u64::MAX}),
            json!({"step": "Impact", "nodes": [1], "frame": 99}),
            json!({"step": "Impact", "nodes": [1], "frame": -1}),
            json!({"step": "Impact", "nodes": [0, 4294967296u64]}),
            json!({"step": "Impact"}),
            json!({"step": "Impact", "elements": [999]}),
            json!({"step": "Impact", "elements": [0, -1, 1.5]}),
            json!({"step": "Impact", "elements": vec![1; 20_000]}),
            json!({"step": "Impact", "elements": "all", "nodes": [1]}),
        ];
        for id in ["solve.explicit", "solve.history", "solve.probe"] {
            for p in &hostile {
                let mut t = s.clone();
                let before = t.model.clone();
                let results = t.solved_steps().count();
                if t.execute(id, p).is_err() {
                    assert_eq!(t.model, before);
                    assert_eq!(t.solved_steps().count(), results, "{id} {p}");
                }
            }
        }
        // Hostile steps are refused when they are defined; solving one that a later edit makes
        // impossible (a node behind its wall) is an error, not a panic.
        for bad in [
            json!({"type": "explicit_dynamic", "end_time": -1}),
            json!({"type": "explicit_dynamic", "end_time": 1, "dt_scale": 2}),
            json!({"type": "explicit_dynamic", "end_time": 1, "max_increments": 0}),
            json!({"type": "explicit_dynamic", "end_time": 1, "output_interval": 1e-300}),
            json!({"type": "explicit_dynamic", "end_time": 1e308, "history_interval": 1e-308}),
            json!({"type": "explicit_dynamic", "end_time": 1, "rigid_walls": [{"set": "x0", "point": [0, 0, 0], "normal": [0, 0, 0]}]}),
            json!({"type": "explicit_dynamic", "end_time": 1, "initial_velocities": [{"set": "nope", "velocity": [1, 0, 0]}]}),
            json!({"type": "explicit_dynamic", "end_time": 1, "hourglass": {"type": "magic"}}),
        ] {
            assert!(s.execute("model.step.create", &json!({"name": "Bad", "procedure": bad})).is_err(), "{bad}");
        }
        let behind = json!({"type": "explicit_dynamic", "end_time": 0.1, "rigid_walls": [{"set": "x0", "point": [1, 0, 0], "normal": [1, 0, 0]}]});
        s.execute("model.step.create", &json!({"name": "Behind", "procedure": behind})).unwrap();
        assert!(s.execute("solve.explicit", &json!({"step": "Behind"})).unwrap_err().to_string().contains("behind"));
        let long = json!({"type": "explicit_dynamic", "end_time": 1e6});
        s.execute("model.step.create", &json!({"name": "Long", "procedure": long})).unwrap();
        assert!(s.execute("solve.explicit", &json!({"step": "Long"})).unwrap_err().to_string().contains("increments"));
        assert!(s.execute("model.set.delete", &json!({"name": "all_nodes"})).unwrap_err().to_string().contains("step `Impact`"));
        // Defining other steps left what the impact step reads alone: its result is current.
        assert_eq!(s.solved_steps().collect::<Vec<_>>(), ["Impact"]);
    }

    /// A mass on a spring and a dashpot, defined, solved and written through commands; editing
    /// and deleting them goes through the same checks as every definition.
    #[test]
    fn spring_mass_through_commands() {
        let mut s = Session::default();
        let script = json!([
            {"command": "mesh.node.add", "params": {"label": 1, "xyz": [0, 0, 0]}},
            {"command": "model.set.create", "params": {"name": "n", "nodes": [1]}},
            {"command": "model.mass.create", "params": {"name": "m", "target": {"set": "n"}, "mass": 4}},
            {"command": "model.spring.create", "params": {"name": "k", "type": "spring", "first": {"set": "n"}, "direction": {"dof": 1}, "stiffness": 400}},
            {"command": "model.spring.create", "params": {"name": "c", "type": "dashpot", "first": {"set": "n"}, "direction": {"vector": [1, 0, 0]}, "coefficient": 1}},
            {"command": "model.boundary_condition.create", "params": {"name": "yz", "type": "displacement", "target": {"set": "n"}, "values": [null, 0, 0, null, null, null]}},
            {"command": "model.step.create", "params": {"name": "Modes", "procedure": {"type": "frequency", "modes": 1}, "boundary_conditions": ["yz"]}},
        ]);
        s.run_script(&script).unwrap();
        let r = s.execute("solve.frequency", &json!({})).unwrap();
        let f = r["modes"][0]["frequency"].as_f64().unwrap();
        assert!((f - 10.0 / (2.0 * std::f64::consts::PI)).abs() < 1e-12, "{r}");
        assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("dashpots `c`")), "{r}");
        assert!(s.execute("model.mass.edit", &json!({"name": "m", "target": {"set": "n"}, "mass": -1})).is_err());
        assert!(s.execute("model.set.delete", &json!({"name": "n"})).is_err(), "the set is in use");
        s.execute("model.mass.edit", &json!({"name": "m", "target": {"set": "n"}, "mass": 1})).unwrap();
        let r = s.execute("solve.frequency", &json!({})).unwrap();
        assert!((r["modes"][0]["frequency"].as_f64().unwrap() - 20.0 / (2.0 * std::f64::consts::PI)).abs() < 1e-12);
        s.execute("model.spring.delete", &json!({"name": "c"})).unwrap();
        assert!(s.model.springs().len() == 1 && s.model.mass("m").is_some());
    }
    #[test]
    fn solve_and_probe_shells_and_beams() {
        // A cantilever strip of two quad4 shells clamped at x = 0, stiffened along its tip edge
        // by a beam; a downward force at a tip corner.
        let mut s = Session::default();
        let mut script = Vec::new();
        for (label, xyz) in [(1, [0, 0, 0]), (2, [10, 0, 0]), (3, [20, 0, 0]), (4, [0, 5, 0]), (5, [10, 5, 0]), (6, [20, 5, 0])] {
            script.push(json!({"command": "mesh.node.add", "params": {"label": label, "xyz": xyz}}));
        }
        script.extend([
            json!({"command": "mesh.element.add", "params": {"label": 1, "kind": "quad4", "nodes": [1, 2, 5, 4]}}),
            json!({"command": "mesh.element.add", "params": {"label": 2, "kind": "quad4", "nodes": [2, 3, 6, 5]}}),
            json!({"command": "mesh.element.add", "params": {"label": 3, "kind": "beam2", "nodes": [3, 6]}}),
            json!({"command": "model.set.create", "params": {"name": "plate", "elements": [1, 2]}}),
            json!({"command": "model.set.create", "params": {"name": "stiffener", "elements": [3]}}),
            json!({"command": "model.set.create", "params": {"name": "root", "nodes": [1, 4]}}),
            json!({"command": "model.set.create", "params": {"name": "corner", "nodes": [3]}}),
            json!({"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}, "density": 7.85e-9}}),
            json!({"command": "model.section.create", "params": {"name": "skin", "set": "plate", "material": "Steel", "kind": {"type": "shell", "thickness": 1}}}),
            json!({"command": "model.section.create", "params": {"name": "rib", "set": "stiffener", "material": "Steel", "kind": {"type": "beam", "profile": {"type": "rectangle", "width": 1, "height": 2}, "orientation": [1, 0, 0]}}}),
            json!({"command": "model.boundary_condition.create", "params": {"name": "clamp", "type": "fixed", "target": {"set": "root"}}}),
            json!({"command": "model.load.create", "params": {"name": "push", "type": "force", "target": {"set": "corner"}, "vector": [0, 0, -1]}}),
            json!({"command": "model.step.create", "params": {"name": "Bend", "procedure": {"type": "static"}, "boundary_conditions": ["clamp"], "loads": ["push"]}}),
            json!({"command": "model.step.create", "params": {"name": "Modes", "procedure": {"type": "frequency", "modes": 3}, "boundary_conditions": ["clamp"]}}),
        ]);
        s.run_script(&Value::Array(script)).unwrap();
        let r = s.execute("solve.static", &json!({"step": "Bend"})).unwrap();
        assert!((r["reaction_total"][2].as_f64().unwrap() - 1.0).abs() < 1e-9, "{r}");
        let probe = s.execute("solve.probe", &json!({"step": "Bend", "nodes": [3, 1], "elements": [1, 3]})).unwrap();
        assert!(probe["nodes"][0]["displacement"][2].as_f64().unwrap() < 0.0, "{probe}");
        assert!(probe["nodes"][0]["rotation"][0].as_f64().unwrap() != 0.0, "{probe}");
        assert!(probe["nodes"][1]["reaction_moment"].is_array(), "{probe}");
        assert_eq!(probe["elements"][0]["forces"].as_array().unwrap().len(), 4, "{probe}");
        assert_eq!(probe["elements"][1]["ends"].as_array().unwrap().len(), 2, "{probe}");
        let only = s.execute("solve.probe", &json!({"step": "Bend", "elements": [2]})).unwrap();
        assert_eq!(only["nodes"].as_array().unwrap().len(), 0, "{only}");
        s.execute("solve.frequency", &json!({"step": "Modes"})).unwrap();
        let shape = s.execute("solve.probe", &json!({"step": "Modes", "nodes": [3], "mode": 1})).unwrap();
        assert!(shape["nodes"][0]["rotation"].is_array(), "{shape}");
        for p in [
            json!({"step": "Bend", "elements": [99]}),
            json!({"step": "Bend", "elements": [0]}),
            json!({"step": "Bend", "elements": "1"}),
            json!({"step": "Bend", "elements": vec![1; 20_000]}),
            json!({"step": "Modes", "nodes": [3], "elements": [1]}),
        ] {
            assert!(s.execute("solve.probe", &p).is_err(), "{p}");
        }
    }

    /// A heat balance per load: the convection `Air` carries away what the heater's prescribed
    /// temperature supplies (#362).
    #[test]
    fn heat_flows_balance_per_load() {
        let mut s = Session::default();
        cantilever(&mut s);
        let script = json!([
            {"command": "model.material.edit", "params": {"name": "Mat", "elastic": {"type": "isotropic", "young": 1000, "poisson": 0}, "density": 1, "thermal": {"conductivity": 2}}},
            {"command": "model.boundary_condition.create", "params": {"name": "Heater", "type": "temperature", "target": {"set": "root"}, "value": 100}},
            {"command": "model.set.create", "params": {"name": "skin", "faces": (1..=6).map(|f| json!({"element": 10, "face": f})).collect::<Vec<_>>()}},
            {"command": "model.load.create", "params": {"name": "Air", "type": "convection", "set": "skin", "film": 0.5, "sink": 20}},
            {"command": "model.step.create", "params": {"name": "Therm", "procedure": {"type": "heat_transfer"}, "boundary_conditions": ["Heater"], "loads": ["Air"]}},
        ]);
        s.run_script(&script).unwrap();
        let r = s.execute("solve.heat", &json!({"step": "Therm"})).unwrap();
        let flows = &r["heat_flows"];
        let air = flows["loads"].as_array().unwrap().iter().find(|l| l["name"] == "Air").unwrap();
        assert_eq!(air["type"], "convection");
        let (out, supplied) = (air["heat"].as_f64().unwrap(), flows["reactions"].as_f64().unwrap());
        assert!(out < 0.0 && supplied > 0.0, "{flows}");
        assert!(flows["imbalance"].as_f64().unwrap().abs() < 1e-6 * supplied, "{flows}");
        assert_eq!(s.results_report()["heat_flows"], *flows);
    }

    #[test]
    fn solve_heat_and_probe_temperatures() {
        // The cantilever bar as a conductor: 0 at the root, 100 at the tip, linear between.
        let mut s = Session::default();
        cantilever(&mut s);
        let script = json!([
            {"command": "model.material.edit", "params": {"name": "Mat", "elastic": {"type": "isotropic", "young": 1000, "poisson": 0}, "density": 1, "thermal": {"conductivity": 2, "specific_heat": 3, "expansion": 1e-5}}},
            {"command": "model.boundary_condition.create", "params": {"name": "cold", "type": "temperature", "target": {"set": "root"}, "value": 0}},
            {"command": "model.boundary_condition.create", "params": {"name": "hot", "type": "temperature", "target": {"set": "tip"}, "value": 100}},
            {"command": "model.step.create", "params": {"name": "Heat", "procedure": {"type": "heat_transfer"}, "boundary_conditions": ["cold", "hot"]}},
            {"command": "model.step.create", "params": {"name": "Warm", "procedure": {"type": "heat_transfer", "transient": {"end_time": 10, "increment": 1}}, "boundary_conditions": ["cold", "hot"]}},
        ]);
        s.run_script(&script).unwrap();
        let r = s.execute("solve.heat", &json!({"step": "Heat"})).unwrap();
        assert!((r["max_temperature"]["value"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{r}");
        assert!(r["min_temperature"]["value"].as_f64().unwrap().abs() < 1e-9, "{r}");
        let probe = s.execute("solve.probe", &json!({"step": "Heat", "nodes": [6]})).unwrap();
        assert!((probe["nodes"][0]["temperature"].as_f64().unwrap() - 50.0).abs() < 1e-9, "{probe}");
        assert!((probe["nodes"][0]["heat_flux"][0].as_f64().unwrap() + 20.0).abs() < 1e-9, "{probe}");
        let w = s.execute("solve.heat", &json!({"step": "Warm"})).unwrap();
        assert_eq!(w["frames"], 11, "{w}");
        let early = s.execute("solve.probe", &json!({"step": "Warm", "nodes": [6], "frame": 1})).unwrap();
        assert_eq!(early["time"], 1.0);
        assert!(s.execute("solve.probe", &json!({"step": "Warm", "nodes": [6], "frame": 99})).is_err());
        assert!(s.execute("solve.probe", &json!({"step": "Warm", "nodes": [999]})).is_err());
        assert!(s.execute("solve.heat", &json!({"step": "Bend"})).is_err(), "not a heat step");
        assert!(s.execute("solve.heat", &json!({"step": "Heat", "max_dofs": 0})).is_err());
        // The results display shows temperatures, undeformed.
        let view = crate::results::ResultsView::for_result(&s.latest_result("Heat").unwrap().0.clone());
        assert_eq!((view.contour, view.deformed), (Some(crate::results::Field::Temperature), false));
        let (heat, _) = s.latest_result("Heat").unwrap();
        let fields: Vec<_> = crate::results::Field::available(heat).collect();
        assert_eq!(fields, vec![crate::results::Field::Temperature, crate::results::Field::HeatFlux]);
        assert!(crate::results::displacements(heat, crate::results::Pick::default()).is_err());
    }

    /// `solve.nonlinear`: a cantilever bent far by a dead tip load, with its increment history,
    /// a frame per converged increment shown by `results.frame`, `solve.run` dispatching to it,
    /// and the same bits on 1, 2 and 7 threads.
    #[test]
    fn nonlinear_steps_solve_with_their_history_and_frames() {
        let solve = |threads: usize| -> (Value, u64, Session) {
            let mut s = Session::default();
            cantilever(&mut s);
            let script = json!([
                {"command": "model.section.edit", "params": {"name": "bar", "set": "all", "material": "Mat", "kind": {"type": "solid"}, "formulation": {}}},
                {"command": "model.step.create", "params": {"name": "Large", "procedure": {"type": "nonlinear_static", "initial_increment": 0.25},
                    "boundary_conditions": ["clamp"], "loads": ["push"]}},
                {"command": "model.load.edit", "params": {"name": "push", "type": "force", "target": {"set": "tip"}, "vector": [0, 0, -15]}},
            ]);
            s.run_script(&script).unwrap();
            let r = s.execute("solve.nonlinear", &json!({"step": "Large", "threads": threads})).unwrap();
            let hash = result_hash(&s, "Large");
            (r, hash, s)
        };
        let (r, one, mut s) = solve(1);
        for threads in [2, 7] {
            assert_eq!(solve(threads).1, one, "{threads} threads");
        }
        let increments = r["increments"].as_array().unwrap();
        assert!(increments.iter().any(|i| i["converged"] == true) && increments.last().unwrap()["load_factor"] == 1.0, "{r}");
        assert!(increments[0]["iterations"].as_array().is_some_and(|it| !it.is_empty() && it[0]["force"].is_number()), "{r}");
        let frames = r["frames"].as_array().unwrap().len();
        assert_eq!(frames, 1 + increments.iter().filter(|i| i["converged"] == true).count());
        // Far from linear: P L² / (E I) = 15·100/(1000/12) = 18, the tip swings well down and in.
        let tip = r["max_displacement"]["value"].as_f64().unwrap();
        assert!(tip > 8.0 && tip < 12.0, "{r}");
        let view = s.execute("results.frame", &json!({"step": "Large", "frame": 1})).unwrap();
        assert_eq!(view["view"]["frame"], 1, "{view}");
        assert!(view["load_factor"].as_f64().is_some_and(|f| f > 0.0 && f < 1.0), "{view}");
        assert!(s.execute("results.frame", &json!({"step": "Large", "frame": frames})).is_err());
        let probe = s.execute("solve.probe", &json!({"step": "Large", "nodes": [11]})).unwrap();
        assert!(probe["nodes"][0]["displacement"][2].as_f64().is_some_and(|w| w < -5.0), "{probe}");
        // solve.run solves every step with its own solver; the hostile caps are refused.
        assert!(s.execute("solve.nonlinear", &json!({"step": "Large", "max_newton_iterations": 0})).is_err());
        assert!(s.execute("solve.nonlinear", &json!({"step": "Large", "max_newton_iterations": 1e300})).is_err());
        let e = s.execute("solve.nonlinear", &json!({"step": "Large", "max_newton_iterations": 2})).unwrap_err().to_string();
        assert!(e.contains("Newton iterations"), "{e}");
        assert!(s.execute("solve.nonlinear", &json!({"step": "Bend"})).is_err(), "a linear step");
        let all = s.execute("solve.run", &json!({})).unwrap();
        assert_eq!(all["steps"].as_array().map(Vec::len), Some(3), "{all}");
    }

    /// Two blocks pressed together across a contact pair, as commands.
    fn pressed_blocks(s: &mut Session) {
        let script = json!([
            {"command": "mesh.block", "params": {"body": "Lower", "kind": "hex8", "corners": [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]], "divisions": [2, 2, 1], "faces": {"1": "bottom", "2": "top"}}},
            {"command": "mesh.block", "params": {"body": "Upper", "kind": "hex8", "corners": [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1], [0, 0, 2], [1, 0, 2], [1, 1, 2], [0, 1, 2]], "divisions": [3, 3, 1], "faces": {"1": "bottom", "2": "top"}, "merge": 0}},
            {"command": "model.set.create", "params": {"name": "all", "geometry": [{"body": "Lower"}, {"body": "Upper"}]}},
            {"command": "model.set.create", "params": {"name": "lower_top", "geometry": [{"body": "Lower", "face": "top"}]}},
            {"command": "model.set.create", "params": {"name": "upper_bottom", "geometry": [{"body": "Upper", "face": "bottom"}]}},
            {"command": "model.set.create", "params": {"name": "base", "geometry": [{"body": "Lower", "face": "bottom"}]}},
            {"command": "model.set.create", "params": {"name": "top", "geometry": [{"body": "Upper", "face": "top"}]}},
            {"command": "model.material.create", "params": {"name": "Mat", "elastic": {"type": "isotropic", "young": 1000.0, "poisson": 0.0}}},
            {"command": "model.section.create", "params": {"name": "all", "set": "all", "material": "Mat", "kind": {"type": "solid"}}},
            {"command": "interaction.contact.pair", "params": {"name": "interface", "main": "lower_top", "secondary": "upper_bottom"}},
            {"command": "model.boundary_condition.create", "params": {"name": "base", "type": "fixed", "target": {"set": "base"}}},
            {"command": "model.boundary_condition.create", "params": {"name": "push", "type": "displacement", "target": {"set": "top"}, "values": [0, 0, -0.02, null, null, null]}},
            {"command": "model.step.create", "params": {"name": "Squeeze", "procedure": {"type": "nonlinear_static", "nonlinear_geometry": false}, "boundary_conditions": ["base", "push"], "loads": []}}
        ]);
        s.run_script(&script).unwrap();
    }

    #[test]
    fn solve_and_probe_a_contact() {
        let mut s = Session::default();
        pressed_blocks(&mut s);
        let r = s.execute("solve.nonlinear", &json!({"step": "Squeeze"})).unwrap();
        let c = &r["contacts"][0];
        assert_eq!(c["name"], "interface");
        // ν = 0 and held ends: uniaxial strain 1% of E = 1000, over the unit area.
        let force = c["normal_force"].as_f64().unwrap();
        assert!((force - 10.0).abs() < 2e-3, "{r}");
        assert_eq!(c["closed_nodes"], 16, "{r}");
        // A secondary node gives its contact pressure, gap and status; a main one gives none.
        let third = 1.0 / 3.0;
        let probe = s.execute("solve.probe", &json!({"step": "Squeeze", "at": [[third, third, 1.0], [0.5, 0.5, 1.0]]})).unwrap();
        let contact = &probe["nodes"][0]["contact"][0];
        assert_eq!(contact["status"], "closed", "{probe}");
        assert!((contact["pressure"].as_f64().unwrap() - 10.0).abs() < 2e-3, "{probe}");
        assert!(contact["gap"].as_f64().unwrap() < 0.0, "{probe}");
        assert!(probe["nodes"][1].get("contact").is_none(), "{probe}");
        // The contact fields on the results display.
        let v = s.execute("results.contour", &json!({"step": "Squeeze", "field": "contact_pressure"})).unwrap();
        let range = &v["contour"]["range"];
        assert!((range["max"]["value"].as_f64().unwrap() - 10.0).abs() < 2e-3 && range["min"]["value"] == 0.0, "{v}");
        let status = s.execute("results.contour", &json!({"step": "Squeeze", "field": "contact_status"})).unwrap();
        assert_eq!(status["contour"]["range"]["max"]["value"], 1.0, "{status}");
        // The linear analyses refuse the contact by name.
        s.execute(
            "model.step.create",
            &json!({"name": "Linear", "procedure": {"type": "static"}, "boundary_conditions": ["base", "push"], "loads": []}),
        )
        .unwrap();
        let e = s.execute("solve.static", &json!({"step": "Linear"})).unwrap_err().to_string();
        assert!(e.contains("interface"), "{e}");
    }

    #[test]
    fn solve_and_probe_a_frictional_contact() {
        let mut s = Session::default();
        pressed_blocks(&mut s);
        s.execute("interaction.contact.edit", &json!({"name": "interface", "surfaces": {"type": "pair", "main": "lower_top", "secondary": "upper_bottom"}, "friction": {"static": 0.2}})).unwrap();
        s.execute(
            "model.boundary_condition.edit",
            &json!({"name": "push", "type": "displacement", "target": {"set": "top"}, "values": [0.05, 0, -0.02, null, null, null]}),
        )
        .unwrap();
        let r = s.execute("solve.nonlinear", &json!({"step": "Squeeze"})).unwrap();
        let c = &r["contacts"][0];
        assert_eq!(c["slipping_nodes"], 16, "{r}");
        // Slipping, the interface carries μ N sideways.
        let (n, t) = (c["force"][2].as_f64().unwrap(), c["force"][0].as_f64().unwrap());
        assert!((t + 0.2 * n).abs() < 1e-3 * n, "{r}");
        let third = 1.0 / 3.0;
        let probe = s.execute("solve.probe", &json!({"step": "Squeeze", "at": [[third, third, 1.0]]})).unwrap();
        let contact = &probe["nodes"][0]["contact"][0];
        assert_eq!(contact["status"], "slip", "{probe}");
        assert!(contact["shear"][0].as_f64().unwrap() < 0.0 && contact["slip"][0].as_f64().unwrap() > 0.0, "{probe}");
        let v = s.execute("results.contour", &json!({"step": "Squeeze", "field": "contact_status"})).unwrap();
        assert_eq!(v["contour"]["range"]["max"]["value"], 2.0, "{v}");
        let v = s.execute("results.contour", &json!({"step": "Squeeze", "field": "contact_shear"})).unwrap();
        assert!(v["contour"]["range"]["max"]["value"].as_f64().unwrap() > 1.0, "{v}");
    }
}
