//! `results.*`: what the results display shows (deformed shape, contours, mode shapes,
//! animation) and reaction sums. The display is session state ([`ResultsView`]): these
//! commands are not edits, so they are not journalled or undone.

use crate::schema::ProblemKind;
use serde_json::{Value, json};

use super::{opt, xyz};
use crate::requirement::Requirement;
use crate::results::{Field, Kind, MAX_BANDS, MIN_BANDS, ResultsView};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const DEFORMED: CommandSpec = CommandSpec {
    id: "results.deformed",
    label: "Deformed Shape",
    params: r#"{"step": "Load", "show": true, "scale": "auto"} — shows a solved step's deformed shape (`show: false` the undeformed mesh); `scale` is a factor (0 draws the undeformed shape, so a sequence from 0 swings it out) or "auto" (the largest displacement drawn a tenth of the model's size); `step` defaults to the step shown or the first solved one"#,
    form: Some(super::forms::DEFORMED),
    kind: CommandKind::Session,
    requires: &[Requirement::Result],
    run: deformed,
};

pub const CONTOUR: CommandSpec = CommandSpec {
    id: "results.contour",
    label: "Contours",
    params: r#"{"step": "Load", "field": "von_mises", "bands": 12, "range"?: {"min"?: 0, "max"?: 80} | "auto"} — contours of a nodal field: displacement, displacement_x/y/z, von_mises, max/mid/min_principal, stress_xx/yy/zz/xy/xz/yz (stresses for static steps), contact_pressure, contact_gap, contact_status (0 open, 1 closed or stick, 2 slip), contact_shear, contact_slip (friction; nonlinear static steps with contact, at the contacts' secondary surfaces); "none" turns them off; 2 to 24 bands; `range` clamps the legend (a bound left out, null or "auto" follows the field; values above `max` are drawn in the over colour), kept until the field changes"#,
    form: Some(super::forms::CONTOUR),
    kind: CommandKind::Session,
    requires: &[Requirement::Result],
    run: contour,
};

pub const MODES: CommandSpec = CommandSpec {
    id: "results.modes",
    label: "Mode Shapes",
    params: r#"{"step": "Modes", "mode": 1} — shows a mode shape of a solved frequency step (from 1) and lists every mode's frequency"#,
    form: Some(super::forms::MODES),
    kind: CommandKind::Session,
    requires: &[Requirement::Result],
    run: modes,
};

pub const ANIMATE: CommandSpec = CommandSpec {
    id: "results.animate",
    label: "Animate",
    params: r#"{"on": true} — animates the deformed shape: a mode swings through its positive and negative shape, a static step grows from undeformed to deformed"#,
    form: Some(super::forms::ANIMATE),
    kind: CommandKind::Session,
    requires: &[Requirement::Result],
    run: animate,
};

pub const FRAME: CommandSpec = CommandSpec {
    id: "results.frame",
    label: "Field Frame",
    params: r#"{"step": "Impact", "frame": 3, "time": 0.002} — shows a field frame of a solved explicit step, by index from 0 or the frame nearest a time (neither: the last), or of a nonlinear static step (one per converged increment, `time` its load factor; the deformed shape follows it, its contours show the final state); the deformed shape and contours follow it"#,
    form: Some(super::forms::FRAME),
    kind: CommandKind::Session,
    requires: &[Requirement::Result],
    run: frame,
};

pub const REACTIONS: CommandSpec = CommandSpec {
    id: "results.reactions",
    label: "Reaction Sums",
    params: r#"{"step": "Load", "sets": ["root"], "about": [0, 0, 0]} — reaction force and moment (about the point) summed over each node set, and over the whole model; `sets` (none or empty) defaults to the sets the step's boundary conditions hold"#,
    form: Some(super::forms::REACTIONS),
    kind: CommandKind::Session,
    requires: &[Requirement::Result],
    run: reactions,
};

pub const EXPORT_CSV: CommandSpec = CommandSpec {
    id: "results.export.csv",
    label: "Export Results (CSV)",
    params: r#"{"what": "probe" | "reactions", "path": "tip.csv", "step": "Load", …} — writes probe or reaction results as a CSV file (RFC 4180, a header line, numbers in shortest round-trip form); the other parameters are solve.probe's (`nodes`, `at` or `set`, `mode`, `frame`) or results.reactions' (`sets`, `about`). Probe: a row per node with node, x, y, z and the quantities the step has; reactions: a row per set, a total row and an applied row. Replies with the path, rows, columns, step and units"#,
    form: Some(super::forms::EXPORT_CSV),
    kind: CommandKind::Session,
    requires: &[Requirement::Result],
    run: export_csv,
};

pub const CLEAR: CommandSpec = CommandSpec {
    id: "results.clear",
    label: "Hide Results",
    params: "{} — back to the model: hides the deformed shape, contours and modes",
    form: Some(&[]),
    kind: CommandKind::Session,
    requires: &[Requirement::ResultsShown],
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

/// The steps a command works on.
#[derive(Clone, Copy, PartialEq)]
enum Want {
    Any,
    Frequency,
    /// Steps with frames: explicit dynamics and nonlinear static (one per converged increment).
    Frames,
}

impl Want {
    fn fits(self, s: &Session, step: &str) -> bool {
        s.latest_result(step).is_some_and(|(r, _)| match (self, r.kind()) {
            (Want::Any, _) | (Want::Frequency, Kind::Frequency(_)) | (Want::Frames, Kind::Explicit(_)) => true,
            (Want::Frames, Kind::Static(r)) => r.nonlinear.is_some(),
            (Want::Frequency | Want::Frames, _) => false,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Want::Any => "solved",
            Want::Frequency => "frequency",
            Want::Frames => "explicit or nonlinear static",
        }
    }
}

/// The step a command works on: `step`, else the step shown, else the first solved one
/// (current results first) of the kind it wants.
fn step_for(s: &Session, p: &Value, want: Want) -> Result<String> {
    if let Some(name) = opt::<String>(p, "step")? {
        if s.latest_result(&name).is_none() {
            return Err(EngineError::Params(format!("step `{name}` has no result; {}", solved_list(s))));
        }
        if !want.fits(s, &name) {
            return Err(EngineError::Params(format!("step `{name}` is not a {} step", want.name())));
        }
        return Ok(name);
    }
    if let Some(v) = &s.results_view
        && want.fits(s, &v.step)
    {
        return Ok(v.step.clone());
    }
    let mut steps: Vec<(&str, bool)> = s.result_steps().filter(|(n, _)| want.fits(s, n)).collect();
    // Current results first, then by name (stable).
    steps.sort_by_key(|(_, current)| !*current);
    steps.first().map(|(n, _)| n.to_string()).ok_or_else(|| {
        EngineError::Params(if want == Want::Any { solved_list(s) } else { format!("no {} step has a result; {}", want.name(), solved_list(s)) })
    })
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
    let step = step_for(s, p, Want::Any)?;
    let show = opt::<bool>(p, "show")?.unwrap_or(true);
    let scale = match p.get("scale") {
        None => None,
        Some(Value::String(a)) if a == "auto" => Some(None),
        Some(v) => Some(Some(
            v.as_f64()
                .filter(|x| x.is_finite() && (0.0..=MAX_SCALE).contains(x))
                .map(f64::abs)
                .ok_or_else(|| EngineError::Params(format!("`scale` must be \"auto\" or a factor from 0 (undeformed) to {MAX_SCALE}")))?,
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
    let step = step_for(s, p, Want::Any)?;
    let name = opt::<String>(p, "field")?.ok_or_else(|| EngineError::param("field", ProblemKind::Required))?;
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
    let range = match p.get("range") {
        None => None,
        Some(r) => Some(legend_range(r)?),
    };
    let v = view_of(s, &step)?;
    if v.contour != field {
        (v.range_min, v.range_max) = (None, None);
    }
    v.contour = field;
    if let Some(b) = bands {
        v.bands = b;
    }
    if let Some((lo, hi)) = range {
        (v.range_min, v.range_max) = (lo, hi);
    }
    Ok(s.results_report())
}

/// `results.contour`'s `range`: `"auto"` or null (follow the field), or `{min?, max?}`, each a
/// finite number, null or `"auto"`; `min` below `max`.
fn legend_range(r: &Value) -> Result<(Option<f64>, Option<f64>)> {
    let bound = |k: &str| -> Result<Option<f64>> {
        match r.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(a)) if a == "auto" => Ok(None),
            Some(v) => {
                v.as_f64().filter(|x| x.is_finite()).map(Some).ok_or_else(|| EngineError::Params(format!("`range.{k}` must be a number or \"auto\"")))
            }
        }
    };
    match r {
        Value::Null => Ok((None, None)),
        Value::String(a) if a == "auto" => Ok((None, None)),
        Value::Object(_) => {
            let (lo, hi) = (bound("min")?, bound("max")?);
            if let (Some(a), Some(b)) = (lo, hi)
                && a >= b
            {
                return Err(EngineError::Params(format!("`range.min` ({a}) must be below `range.max` ({b})")));
            }
            Ok((lo, hi))
        }
        _ => Err(EngineError::Params("`range` must be {\"min\"?, \"max\"?} or \"auto\"".into())),
    }
}

fn modes(s: &mut Session, p: &Value) -> Result<Value> {
    let step = step_for(s, p, Want::Frequency)?;
    let count = match s.latest_result(&step).map(|(r, _)| r.kind()) {
        Some(Kind::Frequency(f)) => f.modes.len(),
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
    let step = step_for(s, &json!({}), Want::Any)?;
    let v = view_of(s, &step)?;
    v.animate = true;
    v.deformed = true;
    Ok(s.results_report())
}

fn frame(s: &mut Session, p: &Value) -> Result<Value> {
    let step = step_for(s, p, Want::Frames)?;
    let times: Vec<f64> = match s.latest_result(&step).map(|(r, _)| r.kind()) {
        Some(Kind::Explicit(e)) => e.frames.iter().map(|f| f.time).collect(),
        Some(Kind::Static(r)) => r.nonlinear.as_ref().map(|h| h.load_factors()).unwrap_or_default(),
        _ => Vec::new(),
    };
    let last = times.len().checked_sub(1).ok_or_else(|| EngineError::Params(format!("step `{step}` has no field frames")))?;
    let k = match (p.get("frame"), opt::<f64>(p, "time")?) {
        (Some(f), _) => f
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= last)
            .ok_or_else(|| EngineError::Params(format!("`frame` must be from 0 to {last}")))?,
        (None, Some(t)) if t.is_finite() => {
            times.iter().enumerate().min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs())).map_or(last, |(i, _)| i)
        }
        (None, Some(_)) => return Err(EngineError::param("time", ProblemKind::NotANumber)),
        (None, None) => last,
    };
    let v = view_of(s, &step)?;
    v.frame = Some(k);
    Ok(s.results_report())
}

fn clear(s: &mut Session, _: &Value) -> Result<Value> {
    s.results_view = None;
    Ok(json!({"view": null}))
}

fn reactions(s: &mut Session, p: &Value) -> Result<Value> {
    let step = step_for(s, p, Want::Any)?;
    let about = if p.get("about").is_some() { xyz(p, "about")? } else { [0.0; 3] };
    if !about.iter().all(|c| c.is_finite()) {
        return Err(EngineError::param("about", ProblemKind::NotAVector { len: 3 }));
    }
    let Some((result, current)) = s.latest_result(&step) else { return Err(EngineError::Params(solved_list(s))) };
    let Kind::Static(r) = result.kind() else {
        return Err(EngineError::Params(format!(
            "step `{step}` is not a static step: reactions are kept for static steps (explicit steps keep the constraint force history, solve.history)"
        )));
    };
    // No sets (or an empty list, a form with none ticked): the step's boundary-condition sets.
    let sets: Vec<String> = match p.get("sets").filter(|v| v.as_array().is_none_or(|a| !a.is_empty())) {
        Some(v) => {
            let list = v
                .as_array()
                .filter(|a| a.len() <= MAX_SETS)
                .ok_or_else(|| EngineError::Params(format!("`sets` must be a list of at most {MAX_SETS} set names")))?;
            list.iter()
                .map(|n| n.as_str().map(str::to_string).ok_or_else(|| EngineError::param("sets", ProblemKind::NotNames)))
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

fn export_csv(s: &mut Session, p: &Value) -> Result<Value> {
    let what: String = super::req(p, "what")?;
    let path = super::file_param(s, p, "path", fieldcraft_io::roots::Access::Write)?;
    let (step, table) = match what.as_str() {
        "probe" => {
            let r = super::solve::probe(s, p)?;
            (r.get("step").cloned().unwrap_or(Value::Null), probe_table(&r, &|label| s.model.node(label).map(|n| n.xyz))?)
        }
        "reactions" => {
            let r = reactions(s, p)?;
            if r.get("stale").and_then(Value::as_bool).unwrap_or(false) {
                let step = r.get("step").and_then(Value::as_str).unwrap_or_default();
                return Err(EngineError::Params(format!(
                    "the result of step `{step}` is stale (the model changed since it was solved); solve it again"
                )));
            }
            (r.get("step").cloned().unwrap_or(Value::Null), reactions_table(&r))
        }
        _ => return Err(EngineError::Params("`what` must be \"probe\" or \"reactions\"".into())),
    };
    let text = table.csv();
    fieldcraft_io::fs::write_atomic(std::path::Path::new(&path), text.as_bytes(), false)?;
    Ok(json!({"path": path, "rows": table.rows.len(), "columns": table.header, "step": step, "units": s.model.units()}))
}

/// A table of cells (a number, text, or empty) under a header, written as CSV.
struct Table {
    header: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug, PartialEq)]
enum Cell {
    Number(f64),
    Text(String),
    Empty,
}

impl Table {
    /// RFC 4180: comma-separated, CRLF line ends, a field with a comma, quote or line break in
    /// quotes (its quotes doubled); numbers in Rust's shortest round-trip form.
    fn csv(&self) -> String {
        let field = |s: &str| if s.contains([',', '"', '\r', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
        let mut out = String::new();
        let line = |cells: Vec<String>, out: &mut String| {
            out.push_str(&cells.join(","));
            out.push_str("\r\n");
        };
        line(self.header.iter().map(|h| field(h)).collect(), &mut out);
        for row in &self.rows {
            let cells = row
                .iter()
                .map(|c| match c {
                    Cell::Number(v) => format!("{v}"),
                    Cell::Text(t) => field(t),
                    Cell::Empty => String::new(),
                })
                .collect();
            line(cells, &mut out);
        }
        out
    }

    /// Rows of named cells into a table whose columns are every name that has a value, in the
    /// order first met; a column no row has a value in is left out.
    fn from_named(rows: Vec<Vec<(String, Cell)>>) -> Table {
        let mut header: Vec<String> = Vec::new();
        for row in &rows {
            for (name, c) in row {
                if *c != Cell::Empty && !header.contains(name) {
                    header.push(name.clone());
                }
            }
        }
        let rows = rows
            .into_iter()
            .map(|row| header.iter().map(|h| row.iter().find(|(n, _)| n == h).map_or(Cell::Empty, |(_, c)| c.clone())).collect())
            .collect();
        Table { header, rows }
    }
}

/// The CSV columns of a quantity of a probe's node entry: a vector's components (and a
/// displacement's magnitude), a stress tensor's six components, a scalar under its own name.
fn quantity_cells(key: &str, v: &Value, out: &mut Vec<(String, Cell)>) {
    const VECTORS: [(&str, &[&str]); 7] = [
        ("displacement", &["ux", "uy", "uz"]),
        ("rotation", &["rx", "ry", "rz"]),
        ("reaction", &["rfx", "rfy", "rfz"]),
        ("reaction_moment", &["rmx", "rmy", "rmz"]),
        ("velocity", &["vx", "vy", "vz"]),
        ("shape", &["ux", "uy", "uz", "rx", "ry", "rz"]),
        ("stress", &["s_xx", "s_yy", "s_zz", "s_xy", "s_xz", "s_yz"]),
    ];
    match v {
        Value::Number(n) => out.push((key.to_string(), n.as_f64().map_or(Cell::Empty, Cell::Number))),
        Value::Array(items) if items.iter().all(Value::is_number) => {
            let nums: Vec<f64> = items.iter().filter_map(Value::as_f64).collect();
            let names: Vec<String> = match VECTORS.iter().find(|(k, _)| *k == key) {
                Some((_, n)) if n.len() == nums.len() => n.iter().map(|s| (*s).to_string()).collect(),
                _ => (1..=nums.len()).map(|i| format!("{key}_{i}")).collect(),
            };
            out.extend(names.into_iter().zip(nums.iter().map(|&x| Cell::Number(x))));
            if key == "displacement" && nums.len() == 3 {
                out.push(("u_mag".into(), Cell::Number(nums.iter().map(|x| x * x).sum::<f64>().sqrt())));
            }
        }
        _ => {}
    }
}

/// `solve.probe`'s reply as a table: a row per node with its label, undeformed position and
/// the quantities the step has.
fn probe_table(r: &Value, position: &dyn Fn(u32) -> Option<[f64; 3]>) -> Result<Table> {
    let nodes = r.get("nodes").and_then(Value::as_array).ok_or_else(|| EngineError::Params("the probe has no node results to export".into()))?;
    let rows = nodes
        .iter()
        .map(|e| {
            let label = e.get("node").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
            let mut row = vec![("node".to_string(), label.map_or(Cell::Empty, |l| Cell::Number(f64::from(l))))];
            if let Some(name) = e.get("reference_point").and_then(Value::as_str) {
                row.push(("reference_point".into(), Cell::Text(name.to_string())));
            }
            let xyz = label.and_then(position);
            for (k, axis) in ["x", "y", "z"].iter().enumerate() {
                row.push(((*axis).to_string(), xyz.and_then(|p| p.get(k).copied()).map_or(Cell::Empty, Cell::Number)));
            }
            if let Some(o) = e.as_object() {
                for (k, v) in o.iter().filter(|(k, _)| !matches!(k.as_str(), "node" | "reference_point" | "xyz" | "distance")) {
                    quantity_cells(k, v, &mut row);
                }
            }
            row
        })
        .collect();
    Ok(Table::from_named(rows))
}

/// `results.reactions`' reply as a table: a row per set, the total and the applied load.
fn reactions_table(r: &Value) -> Table {
    let row = |name: &str, e: &Value| {
        let mut row = vec![("set".to_string(), Cell::Text(name.to_string()))];
        row.push(("nodes".into(), e.get("nodes").and_then(Value::as_f64).map_or(Cell::Empty, Cell::Number)));
        for (key, names) in [("force", ["fx", "fy", "fz"]), ("moment", ["mx", "my", "mz"])] {
            let v = e.get(key).and_then(Value::as_array);
            for (k, n) in names.iter().enumerate() {
                row.push(((*n).to_string(), v.and_then(|a| a.get(k)).and_then(Value::as_f64).map_or(Cell::Empty, Cell::Number)));
            }
        }
        row
    };
    let mut rows: Vec<Vec<(String, Cell)>> = r
        .get("sets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| row(e.get("set").and_then(Value::as_str).unwrap_or_default(), e))
        .collect();
    rows.push(row("total", r.get("total").unwrap_or(&Value::Null)));
    if let Some(applied) = r.get("applied").filter(|a| a.is_array()) {
        rows.push(row("applied", &json!({"force": applied})));
    }
    Table::from_named(rows)
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

    /// `range` clamps the legend (#191): kept with the field, cleared by "auto" or another
    /// field, reported in the reply; the true range is still there.
    /// A CSV reader (RFC 4180) for the tests: rows of fields, quotes undone.
    fn read_csv(text: &str) -> Vec<Vec<String>> {
        let (mut rows, mut row, mut field, mut quoted, mut chars) = (Vec::new(), Vec::new(), String::new(), false, text.chars().peekable());
        while let Some(c) = chars.next() {
            match (quoted, c) {
                (true, '"') if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                (true, '"') => quoted = false,
                (true, c) => field.push(c),
                (false, '"') => quoted = true,
                (false, ',') => row.push(std::mem::take(&mut field)),
                (false, '\r') => {}
                (false, '\n') => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                (false, c) => field.push(c),
            }
        }
        rows
    }

    fn temp_csv(tag: &str) -> String {
        std::env::temp_dir().join(format!("fieldcraft-csv-{tag}-{}.csv", std::process::id())).to_string_lossy().into_owned()
    }

    /// `results.export.csv` writes what results.reactions and solve.probe reply, bit for bit
    /// after parsing; a set name with a comma and a quote round-trips (#92).
    #[test]
    fn results_export_as_csv() {
        let mut s = solved();
        let path = temp_csv("reactions");
        let sets = json!(["root", "tip"]);
        let reply = s.execute("results.export.csv", &json!({"what": "reactions", "path": path, "step": "Bend", "sets": sets})).unwrap();
        let sums = s.execute("results.reactions", &json!({"step": "Bend", "sets": sets})).unwrap();
        let rows = read_csv(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(rows[0], ["set", "nodes", "fx", "fy", "fz", "mx", "my", "mz"]);
        assert_eq!((reply["rows"].as_u64(), reply["step"].as_str()), (Some(4), Some("Bend")), "{reply}");
        // A name with a comma and a quote round-trips through a CSV reader.
        let named =
            Table { header: vec!["set".into(), "fx".into()], rows: vec![vec![Cell::Text("the \"root\", clamped".into()), Cell::Number(0.1)]] };
        assert_eq!(read_csv(&named.csv()), [vec!["set", "fx"], vec!["the \"root\", clamped", "0.1"]]);
        let total = rows.iter().find(|r| r[0] == "total").unwrap();
        assert_eq!(total[4].parse::<f64>().unwrap(), sums["total"]["force"][2].as_f64().unwrap(), "bit for bit");
        let applied = rows.iter().find(|r| r[0] == "applied").unwrap();
        assert_eq!((applied[1].as_str(), applied[5].as_str()), ("", ""), "the applied load has no nodes or moments");
        let path = temp_csv("probe");
        s.execute("results.export.csv", &json!({"what": "probe", "path": path, "step": "Bend", "nodes": [1, 2, 3]})).unwrap();
        let probe = s.execute("solve.probe", &json!({"step": "Bend", "nodes": [1, 2, 3]})).unwrap();
        let rows = read_csv(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(rows.len(), 4);
        let col = |name: &str| rows[0].iter().position(|h| h == name).unwrap_or_else(|| panic!("no {name}: {:?}", rows[0]));
        for (i, node) in probe["nodes"].as_array().unwrap().iter().enumerate() {
            assert_eq!(rows[i + 1][col("node")], node["node"].to_string());
            assert_eq!(rows[i + 1][col("uz")].parse::<f64>().unwrap(), node["displacement"][2].as_f64().unwrap());
            assert_eq!(rows[i + 1][col("s_yz")].parse::<f64>().unwrap(), node["stress"][5].as_f64().unwrap());
            assert_eq!(rows[i + 1][col("von_mises")].parse::<f64>().unwrap(), node["von_mises"].as_f64().unwrap());
        }
        assert!(!rows[0].iter().any(|h| h == "temperature"), "columns the step does not have are left out: {:?}", rows[0]);
        // A stale result is refused.
        // Every parameter the description names is taken: a set's nodes, points, a moment point.
        let set = s.execute("results.export.csv", &json!({"what": "probe", "path": path, "step": "Bend", "set": "tip"})).unwrap();
        assert!(set["rows"].as_u64().unwrap_or(0) > 0, "{set}");
        s.execute("results.export.csv", &json!({"what": "probe", "path": path, "step": "Bend", "at": [[0, 0, 0]]})).unwrap();
        s.execute("results.export.csv", &json!({"what": "reactions", "path": path, "step": "Bend", "about": [1, 0, 0]})).unwrap();
        s.execute("mesh.node.add", &json!({"label": 99_999, "xyz": [0, 0, 0]})).unwrap();
        assert!(s.execute("results.export.csv", &json!({"what": "reactions", "path": temp_csv("stale"), "step": "Bend"})).is_err());
        for p in ["reactions", "probe"] {
            let _ = std::fs::remove_file(temp_csv(p));
        }
    }

    #[test]
    fn the_legend_range_can_be_clamped() {
        let mut s = solved();
        let r = s.execute("results.contour", &json!({"field": "von_mises", "range": {"max": 10}})).unwrap();
        let top = r["contour"]["range"]["max"]["value"].as_f64().unwrap();
        assert!(top > 10.0, "{r}");
        assert_eq!(r["contour"]["legend"], json!({"min": null, "max": 10.0}), "{r}");
        let v = s.results_view.clone().unwrap();
        let b = v.bands(0.0, top);
        assert!(b.over && b.max == 10.0 && b.band(top) == Some(b.count));
        s.execute("results.contour", &json!({"field": "von_mises", "bands": 6})).unwrap();
        assert_eq!(s.results_view.as_ref().unwrap().range_max, Some(10.0), "kept with the field");
        s.execute("results.contour", &json!({"field": "von_mises", "range": {"max": "auto", "min": 2}})).unwrap();
        assert_eq!(s.results_view.as_ref().map(|v| (v.range_min, v.range_max)), Some((Some(2.0), None)));
        s.execute("results.contour", &json!({"field": "displacement"})).unwrap();
        assert_eq!(s.results_view.as_ref().map(|v| (v.range_min, v.range_max)), Some((None, None)), "another field follows its own range");
        s.execute("results.contour", &json!({"field": "displacement", "range": {"max": 1e-3}})).unwrap();
        let r = s.execute("results.contour", &json!({"field": "displacement", "range": "auto"})).unwrap();
        assert!(r["contour"].get("legend").is_none(), "{r}");
    }

    #[test]
    fn the_display_follows_the_commands_and_is_not_an_edit() {
        let mut s = solved();
        let (journal, revision) = (s.journal.len(), s.revision());
        // Solving showed the first solved step, a static one: contours on the undeformed shape.
        assert_eq!(s.results_view.as_ref().map(|v| (v.step.as_str(), v.contour, v.deformed)), Some(("Bend", Some(Field::VonMises), false)));
        s.execute("results.clear", &json!({})).unwrap();
        let r = s.execute("results.contour", &json!({"field": "von_mises"})).unwrap();
        assert_eq!(r["view"]["step"], "Bend", "{r}");
        assert_eq!(r["contour"]["field"], "von_mises");
        assert!(r["contour"]["range"]["max"]["value"].as_f64().unwrap() > 0.0, "{r}");
        let r = s.execute("results.deformed", &json!({"scale": 2.5})).unwrap();
        assert_eq!((r["view"]["deformed"].as_bool(), r["view"]["scale"].as_f64()), (Some(true), Some(2.5)));
        // Scale 0: the undeformed shape, still the deformed view (an animation's first picture).
        let r = s.execute("results.deformed", &json!({"scale": 0})).unwrap();
        assert_eq!((r["view"]["deformed"].as_bool(), r["view"]["scale"].as_f64()), (Some(true), Some(0.0)));
        s.execute("results.deformed", &json!({"scale": "auto", "show": false})).unwrap();
        assert_eq!(s.results_view.as_ref().map(|v| (v.deformed, v.scale)), Some((false, None)));
        // Modes switch to the frequency step (its own first look) and list the frequencies.
        let r = s.execute("results.modes", &json!({"mode": 3})).unwrap();
        assert_eq!((r["view"]["step"].as_str(), r["view"]["mode"].as_u64()), (Some("Modes"), Some(3)));
        assert_eq!(r["modes"].as_array().map(Vec::len), Some(4));
        assert_eq!(s.results_view.as_ref().map(|v| (v.contour, v.deformed)), Some((Some(Field::Displacement), true)), "a mode shape is deformed");
        assert!(s.execute("results.contour", &json!({"field": "von_mises"})).is_err());
        s.execute("results.contour", &json!({"field": "displacement", "bands": 6})).unwrap();
        s.execute("results.animate", &json!({})).unwrap();
        // Changing the field keeps the deformation, its scale and the animation (#332).
        s.execute("results.deformed", &json!({"scale": 3})).unwrap();
        let r = s.execute("results.contour", &json!({"step": "Modes", "field": "displacement_z"})).unwrap();
        assert_eq!(
            (r["view"]["deformed"].as_bool(), r["view"]["scale"].as_f64(), r["view"]["animate"].as_bool()),
            (Some(true), Some(3.0), Some(true))
        );
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
        let empty = s.execute("results.reactions", &json!({"sets": []})).unwrap();
        assert_eq!(empty["sets"][0]["set"], "root", "an empty list sums the step's boundary-condition sets: {empty}");
        assert!(s.execute("results.reactions", &json!({"step": "Modes"})).is_err());
        assert!(s.execute("results.reactions", &json!({"sets": ["nope"]})).is_err());
    }

    #[test]
    fn explicit_frames_drive_the_display() {
        let mut s = Session::default();
        s.run_script(&crate::sample::impact()).unwrap();
        s.execute("solve.explicit", &json!({})).unwrap();
        // The first look: the last frame, true scale, von Mises averaged from the elements.
        let r = s.results_report();
        assert_eq!((r["view"]["step"].as_str(), r["view"]["scale"].as_f64(), r["view"]["frame"].is_null()), (Some("Impact"), Some(1.0), true), "{r}");
        assert_eq!(r["frames"].as_array().map(Vec::len), Some(11));
        assert!(r["contour"]["range"]["max"]["value"].as_f64().unwrap() > 0.0, "{r}");
        // A frame by time: the nearest one.
        let r = s.execute("results.frame", &json!({"time": 1.3e-5})).unwrap();
        assert_eq!(r["view"]["frame"], 3, "{r}");
        assert!((r["time"].as_f64().unwrap() - 1.3e-5).abs() < 1e-6, "frames land on increments: {r}");
        // At frame 0 the bar has not hit yet: no stress.
        let r = s.execute("results.frame", &json!({"frame": 0})).unwrap();
        assert!(r["contour"]["range"]["max"]["value"].as_f64().unwrap().abs() < 1e-6, "{r}");
        s.execute("results.contour", &json!({"field": "displacement_x"})).unwrap();
        // Frames keep velocities: every node starts at 10 m/s.
        let r = s.execute("results.contour", &json!({"field": "velocity"})).unwrap();
        assert!((r["contour"]["range"]["min"]["value"].as_f64().unwrap() - 10000.0).abs() < 1e-6, "{r}");
        assert!(s.execute("results.frame", &json!({"frame": 11})).is_err());
        assert!(s.execute("results.modes", &json!({})).is_err());
        assert!(s.execute("results.reactions", &json!({})).is_err());
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
            json!({"field": "von_mises", "range": {"max": "high"}}),
            json!({"field": "von_mises", "range": {"min": 5, "max": 5}}),
            json!({"field": "von_mises", "range": {"min": 1e308, "max": -1e308}}),
            json!({"field": "von_mises", "range": 7}),
            json!({"what": 7, "path": "x.csv"}),
            json!({"what": "probe"}),
            json!({"what": "probe", "path": "", "nodes": [1]}),
            json!({"what": "probe", "path": "/no/such/dir/x.csv", "nodes": [1]}),
            json!({"what": "probe", "path": "x.csv", "nodes": vec![1; 1_000_000]}),
            json!({"what": "reactions", "path": "x.csv", "step": "Modes"}),
            json!({"what": "reactions", "path": "x.csv", "step": "Nope"}),
            json!({"scale": -1}),
            json!({"scale": -0.0001}),
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
            json!({"frame": -1}),
            json!({"frame": 1e300}),
            json!({"time": "late"}),
            json!({"time": 1e308}),
            json!({"step": "Bend", "frame": 0}),
        ];
        for id in ["results.deformed", "results.contour", "results.modes", "results.animate", "results.frame", "results.reactions", "results.clear"] {
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
