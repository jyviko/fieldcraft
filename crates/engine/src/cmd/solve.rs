//! `solve.*`: run the model's steps with the implicit solver, keep the results in the session,
//! and read them back.

use modelcraft_solver::{FrequencyOptions, Limits, MassKind, StepResult, solve_frequency, solve_static};
use serde_json::{Value, json};

use super::{labels, opt};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

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

pub const PROBE: CommandSpec = CommandSpec {
    id: "solve.probe",
    label: "Probe Results",
    params: r#"{"step": "Load", "nodes": [1, 2], "mode": 1} — the stored result at nodes: displacement, reaction, stress and von Mises (static), or the shape of mode `mode` (frequency, from 1)"#,
    kind: CommandKind::Model,
    run: probe,
};

/// Most nodes one probe lists.
const MAX_PROBE: usize = 10_000;

/// An optional cap that may only lower the default.
fn cap(p: &Value, key: &str, default: usize) -> Result<usize> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=default).contains(n))
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
    let magnitude = |u: &[f64; 3]| (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    let summary = json!({
        "step": r.step,
        "dofs": r.stats.dofs,
        "equations": r.stats.equations,
        "factor_entries": r.stats.factor_entries,
        "max_displacement": largest(r.nodes.iter().zip(&r.displacement).map(|(l, u)| (*l, magnitude(u)))),
        "max_von_mises": largest(r.nodes.iter().zip(&r.von_mises).map(|(l, v)| (*l, *v))),
        "applied_total": r.applied_total,
        "reaction_total": r.reaction_total,
        "warnings": r.warnings,
    });
    s.store_result(StepResult::Static(r));
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
    s.store_result(StepResult::Frequency(r));
    Ok(summary)
}

fn probe(s: &mut Session, p: &Value) -> Result<Value> {
    let step = opt::<String>(p, "step")?.ok_or_else(|| EngineError::Params("`step` is required".into()))?;
    let result = s.result(&step).ok_or_else(|| EngineError::Params(format!("step `{step}` has no result; solve it first")))?;
    let nodes = labels(p, "nodes", MAX_PROBE)?;
    let index = |label: u32| {
        result.nodes().binary_search(&label).map_err(|_| EngineError::Params(format!("node {label} has no result (it belongs to no solved element)")))
    };
    let out: Vec<Value> = match result.as_ref() {
        StepResult::Static(r) => nodes
            .iter()
            .map(|&l| {
                let i = index(l)?;
                Ok(json!({
                    "node": l,
                    "displacement": r.displacement.get(i),
                    "reaction": r.reaction.get(i),
                    "stress": r.stress.get(i),
                    "von_mises": r.von_mises.get(i),
                }))
            })
            .collect::<Result<_>>()?,
        StepResult::Frequency(r) => {
            let mode = opt::<usize>(p, "mode")?.unwrap_or(1);
            let m = mode
                .checked_sub(1)
                .and_then(|k| r.modes.get(k))
                .ok_or_else(|| EngineError::Params(format!("`mode` must be from 1 to {}", r.modes.len())))?;
            nodes.iter().map(|&l| Ok(json!({"node": l, "shape": m.shape.get(index(l)?)}))).collect::<Result<_>>()?
        }
    };
    Ok(json!({"step": step, "nodes": out}))
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
        assert!(s.execute("solve.probe", &json!({"step": "Bend", "nodes": [1]})).is_err());
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
        // An unsupported model (a coupling, a reference point load) is an error, not a panic.
        let mut defined = Session::default();
        crate::cmd::define::tests::defined_bar(&mut defined);
        let e = defined.execute("solve.static", &json!({"step": "Bend"})).unwrap_err().to_string();
        assert!(e.contains("not supported yet"), "{e}");
    }
}
