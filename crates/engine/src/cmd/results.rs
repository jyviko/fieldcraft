//! `results.*`: what the results display shows (deformed shape, contours, mode shapes,
//! animation) and reaction sums. The display is session state ([`ResultsView`]): these
//! commands are not edits, so they are not journalled or undone.

use modelcraft_solver::StepResult;
use serde_json::{Value, json};

use super::{opt, xyz};
use crate::results::{Field, MAX_BANDS, MIN_BANDS, ResultsView};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const DEFORMED: CommandSpec = CommandSpec {
    id: "results.deformed",
    label: "Deformed Shape",
    params: r#"{"step": "Load", "show": true, "scale": "auto"} — shows a solved step's deformed shape (`show: false` the undeformed mesh); `scale` is a factor or "auto" (the largest displacement drawn a tenth of the model's size); `step` defaults to the step shown or the first solved one"#,
    kind: CommandKind::Session,
    run: deformed,
};

pub const CONTOUR: CommandSpec = CommandSpec {
    id: "results.contour",
    label: "Contours",
    params: r#"{"step": "Load", "field": "von_mises", "bands": 12} — contours of a nodal field: displacement, displacement_x/y/z, von_mises, max/mid/min_principal, stress_xx/yy/zz/xy/xz/yz (stresses for static steps); "none" turns them off; 2 to 24 bands"#,
    kind: CommandKind::Session,
    run: contour,
};

pub const MODES: CommandSpec = CommandSpec {
    id: "results.modes",
    label: "Mode Shapes",
    params: r#"{"step": "Modes", "mode": 1} — shows a mode shape of a solved frequency step (from 1) and lists every mode's frequency"#,
    kind: CommandKind::Session,
    run: modes,
};

pub const ANIMATE: CommandSpec = CommandSpec {
    id: "results.animate",
    label: "Animate",
    params: r#"{"on": true} — animates the deformed shape: a mode swings through its positive and negative shape, a static step grows from undeformed to deformed"#,
    kind: CommandKind::Session,
    run: animate,
};

pub const REACTIONS: CommandSpec = CommandSpec {
    id: "results.reactions",
    label: "Reaction Sums",
    params: r#"{"step": "Load", "sets": ["root"], "about": [0, 0, 0]} — reaction force and moment (about the point) summed over each node set, and over the whole model; `sets` defaults to the sets the step's boundary conditions hold"#,
    kind: CommandKind::Session,
    run: reactions,
};

pub const CLEAR: CommandSpec = CommandSpec {
    id: "results.clear",
    label: "Hide Results",
    params: "{} — back to the model: hides the deformed shape, contours and modes",
    kind: CommandKind::Session,
    run: clear,
};

/// Most sets one reaction sum lists.
const MAX_SETS: usize = 1000;
/// Largest deformation scale factor.
const MAX_SCALE: f64 = 1e12;

fn solved_list(s: &Session) -> String {
    let names: Vec<&str> = s.result_steps().map(|(n, _)| n).collect();
    if names.is_empty() { "no step is solved yet (solve.static, solve.frequency)".into() } else { format!("solved steps: {}", names.join(", ")) }
}

fn is_frequency(s: &Session, step: &str) -> bool {
    s.latest_result(step).is_some_and(|(r, _)| matches!(r.as_ref(), StepResult::Frequency(_)))
}

/// The step a command works on: `step`, else the step shown, else the first solved one
/// (current results first). `frequency` asks for a frequency step.
fn step_for(s: &Session, p: &Value, frequency: bool) -> Result<String> {
    let fits = |name: &str| s.latest_result(name).is_some() && (!frequency || is_frequency(s, name));
    if let Some(name) = opt::<String>(p, "step")? {
        if s.latest_result(&name).is_none() {
            return Err(EngineError::Params(format!("step `{name}` has no result; {}", solved_list(s))));
        }
        if !fits(&name) {
            return Err(EngineError::Params(format!("step `{name}` is not a frequency step")));
        }
        return Ok(name);
    }
    if let Some(v) = &s.results_view
        && fits(&v.step)
    {
        return Ok(v.step.clone());
    }
    let mut steps: Vec<(&str, bool)> = s.result_steps().filter(|(n, _)| fits(n)).collect();
    // Current results first, then by name (stable).
    steps.sort_by_key(|(_, current)| !*current);
    steps
        .first()
        .map(|(n, _)| n.to_string())
        .ok_or_else(|| EngineError::Params(if frequency { format!("no frequency step has a result; {}", solved_list(s)) } else { solved_list(s) }))
}

/// The view of `step`: the one shown, or the step's first look ([`ResultsView::for_result`])
/// when another step (or none) is shown.
fn view_of<'a>(s: &'a mut Session, step: &str) -> Result<&'a mut ResultsView> {
    if s.results_view.as_ref().is_none_or(|v| v.step != step) {
        let fresh = s.latest_result(step).map(|(r, _)| ResultsView::for_result(r)).ok_or_else(|| EngineError::Params(solved_list(s)))?;
        s.results_view = Some(fresh);
    }
    s.results_view.as_mut().ok_or_else(|| EngineError::Params("no results are shown".into()))
}

fn deformed(s: &mut Session, p: &Value) -> Result<Value> {
    let step = step_for(s, p, false)?;
    let show = opt::<bool>(p, "show")?.unwrap_or(true);
    let scale = match p.get("scale") {
        None => None,
        Some(Value::String(a)) if a == "auto" => Some(None),
        Some(v) => Some(Some(
            v.as_f64()
                .filter(|x| x.is_finite() && *x > 0.0 && *x <= MAX_SCALE)
                .ok_or_else(|| EngineError::Params(format!("`scale` must be \"auto\" or a factor above 0 and at most {MAX_SCALE}")))?,
        )),
    };
    let v = view_of(s, &step)?;
    v.deformed = show;
    if let Some(scale) = scale {
        v.scale = scale;
    }
    Ok(s.results_report())
}

fn contour(s: &mut Session, p: &Value) -> Result<Value> {
    let step = step_for(s, p, false)?;
    let name = opt::<String>(p, "field")?.ok_or_else(|| EngineError::Params("`field` is required (a field id, or \"none\")".into()))?;
    let field = match name.as_str() {
        "none" => None,
        id => Some(Field::parse(id).ok_or_else(|| {
            let ids: Vec<&str> = Field::ALL.iter().map(|f| f.id()).collect();
            EngineError::Params(format!("unknown field `{id}` (one of {}, or none)", ids.join(", ")))
        })?),
    };
    let bands = match p.get("bands") {
        None => None,
        Some(b) => Some(
            b.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (MIN_BANDS..=MAX_BANDS).contains(n))
                .ok_or_else(|| EngineError::Params(format!("`bands` must be a whole number from {MIN_BANDS} to {MAX_BANDS}")))?,
        ),
    };
    if let (Some(f), Some((r, _))) = (field, s.latest_result(&step))
        && !Field::available(r).any(|a| a == f)
    {
        return Err(EngineError::Params(format!("{} is not available for step `{step}` (mode shapes have no stress scale)", f.label())));
    }
    let v = view_of(s, &step)?;
    v.contour = field;
    if let Some(b) = bands {
        v.bands = b;
    }
    Ok(s.results_report())
}

fn modes(s: &mut Session, p: &Value) -> Result<Value> {
    let step = step_for(s, p, true)?;
    let count = match s.latest_result(&step).map(|(r, _)| r.as_ref()) {
        Some(StepResult::Frequency(f)) => f.modes.len(),
        _ => 0,
    };
    let mode = match p.get("mode") {
        None => s.results_view.as_ref().filter(|v| v.step == step).map_or(1, |v| v.mode),
        Some(m) => m
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=count).contains(n))
            .ok_or_else(|| EngineError::Params(format!("`mode` must be from 1 to {count}")))?,
    };
    let v = view_of(s, &step)?;
    v.mode = mode;
    v.deformed = true;
    Ok(s.results_report())
}

fn animate(s: &mut Session, p: &Value) -> Result<Value> {
    let on = opt::<bool>(p, "on")?.unwrap_or(true);
    if !on {
        if let Some(v) = &mut s.results_view {
            v.animate = false;
        }
        return Ok(s.results_report());
    }
    let step = step_for(s, &json!({}), false)?;
    let v = view_of(s, &step)?;
    v.animate = true;
    v.deformed = true;
    Ok(s.results_report())
}

fn clear(s: &mut Session, _: &Value) -> Result<Value> {
    s.results_view = None;
    Ok(json!({"view": null}))
}

fn reactions(s: &mut Session, p: &Value) -> Result<Value> {
    let step = step_for(s, p, false)?;
    let about = if p.get("about").is_some() { xyz(p, "about")? } else { [0.0; 3] };
    if !about.iter().all(|c| c.is_finite()) {
        return Err(EngineError::Params("`about` must be a finite [x, y, z]".into()));
    }
    let Some((result, current)) = s.latest_result(&step) else { return Err(EngineError::Params(solved_list(s))) };
    let StepResult::Static(r) = result.as_ref() else {
        return Err(EngineError::Params(format!("step `{step}` is a frequency step: it has no reactions")));
    };
    let sets: Vec<String> = match p.get("sets") {
        Some(v) => {
            let list = v
                .as_array()
                .filter(|a| a.len() <= MAX_SETS)
                .ok_or_else(|| EngineError::Params(format!("`sets` must be a list of at most {MAX_SETS} set names")))?;
            list.iter()
                .map(|n| n.as_str().map(str::to_string).ok_or_else(|| EngineError::Params("`sets` must be set names".into())))
                .collect::<Result<_>>()?
        }
        None => {
            let mut names: Vec<String> = s
                .model
                .step(&step)
                .map(|st| st.boundary_conditions.iter().filter_map(|bc| s.model.boundary_condition(bc)?.target().set().map(str::to_string)).collect())
                .unwrap_or_default();
            names.sort();
            names.dedup();
            names
        }
    };
    // Σ R and Σ (x − about) × R over nodes (undeformed positions).
    let sum = |nodes: &mut dyn Iterator<Item = u32>| {
        let (mut force, mut moment, mut count) = ([0.0; 3], [0.0; 3], 0usize);
        for label in nodes {
            let (Ok(i), Some(node)) = (r.nodes.binary_search(&label), s.model.node(label)) else { continue };
            let Some(f) = r.reaction.get(i) else { continue };
            let d = [node.xyz[0] - about[0], node.xyz[1] - about[1], node.xyz[2] - about[2]];
            for k in 0..3 {
                force[k] += f[k];
            }
            moment[0] += d[1] * f[2] - d[2] * f[1];
            moment[1] += d[2] * f[0] - d[0] * f[2];
            moment[2] += d[0] * f[1] - d[1] * f[0];
            count += 1;
        }
        json!({"nodes": count, "force": force, "moment": moment})
    };
    let mut per_set = Vec::with_capacity(sets.len());
    for name in &sets {
        let resolved = s.model.resolve_set(name)?;
        let mut row = sum(&mut resolved.nodes.iter().copied());
        row["set"] = json!(name);
        per_set.push(row);
    }
    let total = sum(&mut r.nodes.iter().copied());
    Ok(json!({"step": step, "stale": !current, "about": about, "sets": per_set, "total": total, "applied": r.applied_total}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::solve::tests::cantilever;

    fn solved() -> Session {
        let mut s = Session::default();
        cantilever(&mut s);
        s.execute("solve.static", &json!({})).unwrap();
        s.execute("solve.frequency", &json!({})).unwrap();
        s
    }

    #[test]
    fn the_display_follows_the_commands_and_is_not_an_edit() {
        let mut s = solved();
        let (journal, revision) = (s.journal.len(), s.revision());
        // Solving showed the first solved step.
        assert_eq!(s.results_view.as_ref().map(|v| (v.step.as_str(), v.contour)), Some(("Bend", Some(Field::VonMises))));
        s.execute("results.clear", &json!({})).unwrap();
        let r = s.execute("results.contour", &json!({"field": "von_mises"})).unwrap();
        assert_eq!(r["view"]["step"], "Bend", "{r}");
        assert_eq!(r["contour"]["field"], "von_mises");
        assert!(r["contour"]["range"]["max"]["value"].as_f64().unwrap() > 0.0, "{r}");
        let r = s.execute("results.deformed", &json!({"scale": 2.5})).unwrap();
        assert_eq!((r["view"]["deformed"].as_bool(), r["view"]["scale"].as_f64()), (Some(true), Some(2.5)));
        s.execute("results.deformed", &json!({"scale": "auto", "show": false})).unwrap();
        assert_eq!(s.results_view.as_ref().map(|v| (v.deformed, v.scale)), Some((false, None)));
        // Modes switch to the frequency step (its own first look) and list the frequencies.
        let r = s.execute("results.modes", &json!({"mode": 3})).unwrap();
        assert_eq!((r["view"]["step"].as_str(), r["view"]["mode"].as_u64()), (Some("Modes"), Some(3)));
        assert_eq!(r["modes"].as_array().map(Vec::len), Some(4));
        assert_eq!(s.results_view.as_ref().and_then(|v| v.contour), Some(Field::Displacement));
        assert!(s.execute("results.contour", &json!({"field": "von_mises"})).is_err());
        s.execute("results.contour", &json!({"field": "displacement", "bands": 6})).unwrap();
        s.execute("results.animate", &json!({})).unwrap();
        assert!(s.results_view.as_ref().is_some_and(|v| v.animate && v.bands == 6));
        s.execute("results.animate", &json!({"on": false})).unwrap();
        assert!(s.results_view.as_ref().is_some_and(|v| !v.animate));
        assert_eq!((s.journal.len(), s.revision()), (journal, revision), "display commands are not edits");
        // An edit makes the results stale; the display keeps showing them, marked.
        s.execute("model.material.edit", &json!({"name": "Mat", "elastic": {"type": "isotropic", "young": 2000, "poisson": 0}, "density": 1}))
            .unwrap();
        assert_eq!(s.results_report()["stale"], true);
        s.execute("results.clear", &json!({})).unwrap();
        assert!(s.results_view.is_none());
        assert!(s.execute("edit.undo", &json!({})).is_ok());
    }

    #[test]
    fn reactions_balance_the_load() {
        let mut s = solved();
        let r = s.execute("results.reactions", &json!({})).unwrap();
        // The clamp holds the whole downward tip force of 1 and its moment: about the origin the
        // tip force −1 z at x = 10 turns +10 about y, so the reactions turn −10.
        let root = &r["sets"][0];
        assert_eq!((root["set"].as_str(), root["nodes"].as_u64()), (Some("root"), Some(4)), "{r}");
        assert!((root["force"][2].as_f64().unwrap() - 1.0).abs() < 1e-9, "{r}");
        assert!((root["moment"][1].as_f64().unwrap() + 10.0).abs() < 1e-6, "{r}");
        assert!((r["total"]["force"][2].as_f64().unwrap() - 1.0).abs() < 1e-9);
        // About a point on the tip's line of action the moment about y vanishes; the tip set
        // has no reactions.
        let r = s.execute("results.reactions", &json!({"sets": ["root", "tip"], "about": [10, 0, 0]})).unwrap();
        assert!(r["sets"][0]["moment"][1].as_f64().unwrap().abs() < 1e-6, "{r}");
        assert_eq!(r["sets"][1]["force"][2].as_f64(), Some(0.0));
        assert!(s.execute("results.reactions", &json!({"step": "Modes"})).is_err());
        assert!(s.execute("results.reactions", &json!({"sets": ["nope"]})).is_err());
    }

    #[test]
    fn results_hostile_params_never_panic() {
        let s = solved();
        let hostile = [
            json!(null),
            json!([]),
            json!({"step": 7}),
            json!({"step": "Nope"}),
            json!({"step": "Bend", "field": "nope"}),
            json!({"field": 3, "bands": 1e300}),
            json!({"field": "von_mises", "bands": 0}),
            json!({"field": "von_mises", "bands": 25}),
            json!({"scale": -1}),
            json!({"scale": 1e300}),
            json!({"scale": "big"}),
            json!({"scale": f64::NAN}),
            json!({"show": "yes"}),
            json!({"mode": 0}),
            json!({"mode": 99}),
            json!({"mode": -1}),
            json!({"step": "Bend", "mode": 1}),
            json!({"on": 5}),
            json!({"sets": "root"}),
            json!({"sets": [1, 2]}),
            json!({"sets": vec!["root"; 2000]}),
            json!({"about": [1e308, 1e308, 1e308]}),
            json!({"about": "here"}),
        ];
        for id in ["results.deformed", "results.contour", "results.modes", "results.animate", "results.reactions", "results.clear"] {
            for p in &hostile {
                let mut t = s.clone();
                let _ = t.execute(id, p);
                assert_eq!(t.model, s.model);
            }
        }
        // Nothing solved: every command says so.
        let mut empty = Session::default();
        for id in ["results.deformed", "results.modes", "results.reactions", "results.animate"] {
            let e = empty.execute(id, &json!({"field": "displacement"})).unwrap_err().to_string();
            assert!(e.contains("solve"), "{id}: {e}");
        }
    }
}
