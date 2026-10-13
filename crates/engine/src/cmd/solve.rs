//! `solve.*`: run the model's steps with the implicit, the explicit or the lattice Boltzmann
//! solver, keep the results in the session, and read them back.

use modelcraft_explicit::{ExplicitResult, solve_explicit};
use modelcraft_lbm::{FluidResult, Obstacle};
use modelcraft_mesh::surface::{BoundarySurface, SurfaceOptions};
use modelcraft_model::FluidDomain;
use modelcraft_solver::{FrequencyOptions, Limits, Location, MassKind, StepResult as ImplicitResult, solve_frequency, solve_static};
use serde_json::{Value, json};

use super::{labels, opt, points, req};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session, StepResult};

pub const STATIC: CommandSpec = CommandSpec {
    id: "solve.static",
    label: "Solve Static Step",
    params: r#"{"step": "Load", "max_dofs": 6000000, "max_factor_entries": 400000000} — solves a linear static step (the only static step when `step` is left out) and keeps the result in the session; the caps may only be lowered"#,
    kind: CommandKind::Model,
    run: run_static,
};

pub const FREQUENCY: CommandSpec = CommandSpec {
    id: "solve.frequency",
    label: "Solve Frequency Step",
    params: r#"{"step": "Modes", "mass": "consistent|lumped", "shift": 0.0, "max_dofs": 6000000, "max_factor_entries": 400000000, "max_iterations": 3000} — natural frequencies and mode shapes of a frequency step (its modes and range); `max_iterations` caps the Lanczos vectors; the result stays in the session"#,
    kind: CommandKind::Model,
    run: run_frequency,
};

pub const EXPLICIT: CommandSpec = CommandSpec {
    id: "solve.explicit",
    label: "Solve Explicit Dynamic Step",
    params: r#"{"step": "Impact", "max_dofs": 6000000, "max_increments": 1000000000, "max_output_values": 50000000} — transient response of an explicit dynamic step (the only one when `step` is left out) by central differences; keeps its histories and field frames in the session; the caps may only be lowered (the step's own increment cap applies too)"#,
    kind: CommandKind::Model,
    run: run_explicit,
};

pub const HISTORY: CommandSpec = CommandSpec {
    id: "solve.history",
    label: "Result Histories",
    params: r#"{"step": "Impact", "every": 1} — the time histories of a solved explicit step: time, increment, energies, momentum, constraint force and the history nodes' displacement and velocity; `every` keeps one sample in so many (by default enough to return at most 10000)"#,
    kind: CommandKind::Model,
    run: history,
};

pub const FLUID: CommandSpec = CommandSpec {
    id: "solve.fluid",
    label: "Solve Fluid Step",
    params: r#"{"step": "Flow", "max_cells": 16000000, "max_steps": 10000000, "max_cell_updates": 1e12, "max_frame_bytes": 2e9} — runs a fluid step with the lattice Boltzmann solver (the only fluid step when `step` is left out): obstacles are triangulated from the open design or taken from imported surfaces; returns the time step, relaxation time, Mach number, mass and the final force on every wall; the caps may only be lowered"#,
    kind: CommandKind::Model,
    run: run_fluid,
};

pub const PROBE: CommandSpec = CommandSpec {
    id: "solve.probe",
    label: "Probe Results",
    params: r#"{"step": "Load", "nodes": [1, 2], "mode": 1, "frame": 0} — the stored result at nodes: displacement, reaction, stress and von Mises (static), the shape of mode `mode` (frequency, from 1), or displacement and velocity in field frame `frame` (explicit, from 0; the last by default) at `nodes` (or at the nodes nearest to points: "at": [[x, y, z], ...], lowest label on a tie, each entry then giving its `xyz` and `distance`) and, with `elements` (explicit), each element's stress, von Mises stress, strain, equivalent plastic strain, damage, temperature and whether it has eroded; a coupled reference point's label gives its six displacements (translations, rotations) and reactions (forces, moments). Fluid steps: {"step": "Flow", "points": [[x, y, z]], "frame": 10 (default the last)} gives velocity and pressure in the cells at the points; {"step": "Flow", "forces": ["x_min", "F2:top", "Body1/F2:top"]} gives those walls' force histories"#,
    kind: CommandKind::Model,
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
            .ok_or_else(|| EngineError::Params(format!("`{key}` must be a whole number from 1 to {default}"))),
    }
}

fn limits(p: &Value) -> Result<Limits> {
    Ok(Limits {
        max_dofs: cap(p, "max_dofs", Limits::MAX_DOFS)?,
        max_factor_entries: cap(p, "max_factor_entries", Limits::MAX_FACTOR_ENTRIES)?,
        max_lanczos_vectors: cap(p, "max_iterations", Limits::MAX_LANCZOS_VECTORS)?,
    })
}

fn largest(values: impl Iterator<Item = (u32, f64)>) -> Value {
    match values.fold(None, |best: Option<(u32, f64)>, (l, v)| if best.is_none_or(|(_, b)| v > b) { Some((l, v)) } else { best }) {
        Some((node, value)) => json!({"node": node, "value": value}),
        None => Value::Null,
    }
}

fn run_static(s: &mut Session, p: &Value) -> Result<Value> {
    let limits = limits(p)?;
    let step = opt::<String>(p, "step")?;
    let r = solve_static(&s.model, step.as_deref(), &limits)?;

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
        "warnings": r.warnings,
    });
    s.store_result(StepResult::Implicit(ImplicitResult::Static(r)));
    Ok(summary)
}

fn run_frequency(s: &mut Session, p: &Value) -> Result<Value> {
    let limits = limits(p)?;
    let step = opt::<String>(p, "step")?;
    let mass = opt::<MassKind>(p, "mass")?.unwrap_or_default();
    let shift = opt::<f64>(p, "shift")?;
    let r = solve_frequency(&s.model, step.as_deref(), &FrequencyOptions { mass, shift }, &limits)?;
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

fn run_explicit(s: &mut Session, p: &Value) -> Result<Value> {
    use modelcraft_explicit::Limits as ExplicitLimits;
    let limits = ExplicitLimits {
        max_dofs: cap(p, "max_dofs", ExplicitLimits::MAX_DOFS)?,
        max_increments: cap(p, "max_increments", ExplicitLimits::MAX_INCREMENTS)?,
        max_output_values: cap(p, "max_output_values", ExplicitLimits::MAX_OUTPUT_VALUES)?,
    };
    let step = opt::<String>(p, "step")?;
    let r = solve_explicit(&s.model, step.as_deref(), &limits)?;
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
        Some(0) => return Err(EngineError::Params("`every` must be at least 1".into())),
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
    let defaults = modelcraft_lbm::Limits::default();
    let limits = modelcraft_lbm::Limits {
        max_cells: cap(p, "max_cells", defaults.max_cells)?,
        max_steps: cap(p, "max_steps", defaults.max_steps)?,
        max_cell_updates: cap(p, "max_cell_updates", defaults.max_cell_updates)?,
        max_frame_bytes: cap(p, "max_frame_bytes", defaults.max_frame_bytes)?,
    };
    let step = opt::<String>(p, "step")?;
    let (_, domain) = modelcraft_lbm::fluid_step(&s.model, step.as_deref())?;
    let obstacles = obstacles(s, domain)?;
    let r = modelcraft_lbm::solve_fluid(&s.model, step.as_deref(), &obstacles, &limits)?;
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
    let step = opt::<String>(p, "step")?.ok_or_else(|| EngineError::Params("`step` is required".into()))?;
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
        (None, Some(_)) => {
            let at = points(p, "at", MAX_PROBE)?;
            let found = nearest_nodes(&s.model, result.nodes(), &at)?;
            (found.iter().map(|(l, _)| *l).collect(), Some(found))
        }
        (None, None) if matches!(result.as_ref(), StepResult::Explicit(_)) => (Vec::new(), None),
        _ => return Err(EngineError::Params("give either `nodes` or `at`".into())),
    };
    let mut out = match result.as_ref() {
        StepResult::Explicit(r) => {
            let elements = if p.get("elements").is_some() { labels(p, "elements", MAX_PROBE)? } else { Vec::new() };
            probe_frame(r, &step, &nodes, &elements, p)?
        }
        StepResult::Implicit(r) => probe_implicit(r, &step, &nodes, p)?,
        StepResult::Fluid(r) => probe_fluid(r, p)?,
    };
    if let (Some(found), Some(entries)) = (located, out.get_mut("nodes").and_then(Value::as_array_mut)) {
        for (entry, (label, distance)) in entries.iter_mut().zip(found) {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("xyz".into(), json!(s.model.node(label).map(|n| n.xyz)));
                obj.insert("distance".into(), json!(distance));
            }
        }
    }
    Ok(out)
}

/// The node with a result nearest to each point (lowest label on a tie), and its distance.
fn nearest_nodes(model: &modelcraft_model::Model, with_result: &[u32], at: &[[f64; 3]]) -> Result<Vec<(u32, f64)>> {
    at.iter()
        .map(|p| {
            with_result
                .iter()
                .filter_map(|&l| {
                    model.node(l).map(|n| (l, ((n.xyz[0] - p[0]).powi(2) + (n.xyz[1] - p[1]).powi(2) + (n.xyz[2] - p[2]).powi(2)).sqrt()))
                })
                .fold(None, |best: Option<(u32, f64)>, (l, d)| if best.is_none_or(|(_, bd)| d < bd) { Some((l, d)) } else { best })
                .ok_or_else(|| EngineError::Params("the result has no nodes to probe".into()))
        })
        .collect()
}

/// The result of an implicit step at nodes.
fn probe_implicit(result: &ImplicitResult, step: &str, nodes: &[u32], p: &Value) -> Result<Value> {
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
                    Location::Node(i) => json!({
                        "node": l,
                        "displacement": r.displacement.get(i),
                        "reaction": r.reaction.get(i),
                        "stress": r.stress.get(i),
                        "von_mises": r.von_mises.get(i),
                    }),
                })
            })
            .collect::<Result<_>>()?,
        ImplicitResult::Frequency(r) => {
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
                        Location::Node(i) => json!({"node": l, "shape": m.shape.get(i)}),
                    })
                })
                .collect::<Result<_>>()?
        }
    };
    Ok(json!({"step": step, "nodes": out}))
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
                "von_mises": f.stress.get(i).map(modelcraft_fem::von_mises),
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
        use modelcraft_geometry::sample::{Plate, plate_design};
        let dir = std::env::temp_dir().join(format!("modelcraft-fluid-{}", std::process::id()));
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
        use modelcraft_model::{ElementKind, Set};
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
        // Defining steps edited the model, so the impact result is stale.
        assert_eq!(s.solved_steps().count(), 0);
    }
}
