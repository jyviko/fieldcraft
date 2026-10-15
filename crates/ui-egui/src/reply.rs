//! What a command run from the UI answered, made readable: a one-line summary for the status bar
//! ([`summary`]) and, when the reply holds more than that line, the whole reply in the context
//! panel ([`show`]): Check Model as a list of its diagnostics with a way to each fix, the Step
//! Table as a grid of steps, any other reply as labelled rows and tables. Programmatic callers
//! (the control channel's `engine.execute`, MCP, scripts) still get the reply's JSON.

use egui::{RichText, vec2};
use serde_json::{Map, Value};

use crate::browser::{self, Def};
use crate::frame::{Level, heading};
use crate::results::number;
use crate::theme::{Tokens, label_font};
use crate::{ModelApp, Tab};

/// The reply shown in the context panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub command: String,
    pub label: String,
    pub params: Value,
    pub value: Value,
}

/// Replies that only read the model, refreshed when it changes; any other reply closes then.
pub const LIVE: [&str; 2] = ["model.check", "model.activity"];

/// The status line's text (after the command's label) is cut to this many characters.
const MAX_LINE: usize = 120;
/// Table rows shown before "… n more".
const MAX_ROWS: usize = 200;
/// Table columns shown at most.
const MAX_COLUMNS: usize = 8;
/// Nesting shown at most; deeper values are compact text.
const MAX_DEPTH: usize = 4;

/// One line saying what `command` answered: the reply's own `summary`, a sentence for the
/// commands with a well-known reply, otherwise its top-level numbers and words. Never JSON.
pub fn summary(command: &str, v: &Value) -> String {
    if let Some(s) = v.get("summary").and_then(Value::as_str) {
        return one_line(s);
    }
    let count = |key: &str| v.get(key).map_or(0, |x| x.as_array().map(Vec::len).or_else(|| x.as_object().map(Map::len)).unwrap_or(0));
    let line = match command {
        "model.check" => {
            let (e, w) = (v["errors"].as_u64().unwrap_or(0), v["warnings"].as_u64().unwrap_or(0));
            if e + w == 0 { "ready to solve, no issues".to_string() } else { format!("{}, {}", plural(e, "error"), plural(w, "warning")) }
        }
        "model.activity" => format!(
            "{}, {}, {}",
            plural(count("steps") as u64, "step"),
            plural(count("boundary_conditions") as u64, "boundary condition"),
            plural(count("loads") as u64, "load")
        ),
        "geometry.bodies" => match v.as_array().map(Vec::len) {
            Some(0) => "no bodies (open a design first)".to_string(),
            Some(n) => plural(n as u64, "body").replace("bodys", "bodies"),
            None => scalars(v),
        },
        "model.info" => {
            let elements: u64 = v["elements"].as_object().into_iter().flatten().filter_map(|(_, n)| n.as_u64()).sum();
            format!(
                "{}, {}, units {}",
                plural(v["nodes"].as_u64().unwrap_or(0), "node"),
                plural(elements, "element"),
                v["units"].as_str().unwrap_or("—")
            )
        }
        "interaction.contact.find" => {
            format!("{} among {}", plural(count("touching") as u64, "touching pair"), plural(v["parts"].as_u64().unwrap_or(0), "part"))
        }
        "results.reactions" => {
            let f = |k: usize| v.pointer(&format!("/total/force/{k}")).and_then(Value::as_f64).map_or("—".into(), number);
            format!("total reaction force ({}, {}, {})", f(0), f(1), f(2))
        }
        _ => scalars(v),
    };
    if line.is_empty() { "done".into() } else { one_line(&line) }
}

/// The status line after `command` changed the model or the display, written for the person
/// rather than from the command's label: "Showing Load deformed ×2", "Mode 2: 1637 Hz", "Added
/// load Push". Commands without a sentence of their own say "<label> done".
pub fn done(command: &str, label: &str, params: &Value, v: &Value) -> String {
    let view = v.get("view").filter(|x| x.is_object());
    let step = view.and_then(|x| x["step"].as_str()).unwrap_or("the step");
    let line = match command {
        "results.deformed" if view.is_some_and(|x| x["deformed"] == false) => Some(format!("Showing {step} undeformed")),
        "results.deformed" => {
            let scale = view.and_then(|x| x["scale"].as_f64()).map(|f| format!(" ×{}", crate::describe::quantity(f))).unwrap_or_default();
            Some(format!("Showing {step} deformed{scale}"))
        }
        "results.contour" => Some(match v.pointer("/contour/label").and_then(Value::as_str) {
            Some(field) => format!("Showing {field} on {step}"),
            None => format!("Contours off on {step}"),
        }),
        "results.modes" => {
            let mode = view.and_then(|x| x["mode"].as_u64()).unwrap_or(1);
            let hz = v
                .get("modes")
                .and_then(Value::as_array)
                .and_then(|m| m.iter().find(|m| m["mode"].as_u64() == Some(mode)))
                .and_then(|m| m["frequency"].as_f64());
            Some(match hz {
                Some(hz) => format!("Mode {mode}: {} Hz", crate::describe::quantity(hz)),
                None => format!("Showing mode {mode} of {step}"),
            })
        }
        "results.animate" => Some(if view.is_some_and(|x| x["animate"] == true) { "Animating" } else { "Animation stopped" }.to_string()),
        "results.frame" => Some(match view.and_then(|x| x["frame"].as_u64()) {
            Some(f) => format!("Showing {step} frame {f}"),
            None => format!("Showing {step}, last frame"),
        }),
        "results.clear" => Some("Back to the model".to_string()),
        "file.sample" => {
            let label = v.get("sample").and_then(Value::as_str).and_then(fieldcraft_engine::sample::find).map(|s| s.label);
            label.map(|l| format!("Opened the {l} sample"))
        }
        "solve.run" => v.get("steps").and_then(Value::as_array).map(|s| format!("Solved {}", plural(s.len() as u64, "step"))),
        c if c.starts_with("solve.") => Some(format!("Solved {step}", step = params["step"].as_str().unwrap_or("the step"))),
        _ => None,
    };
    if let Some(line) = line {
        return line;
    }
    // A definition added, changed or deleted, by its kind and name.
    let name = params.get("name").and_then(Value::as_str);
    let def = Def::ALL
        .into_iter()
        .find_map(|kind| ["create", "edit", "delete"].into_iter().find(|verb| kind.command(verb) == command).map(|verb| (kind, verb)));
    match (def, name) {
        (Some((kind, verb)), Some(name)) => {
            let did = match verb {
                "create" => "Added",
                "edit" => "Changed",
                _ => "Deleted",
            };
            format!("{did} {} {name}", kind.label().to_lowercase())
        }
        _ => format!("{label} done"),
    }
}

/// Whether the reply says more than its summary line, so it is worth the panel.
pub fn is_rich(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, x)| k != "summary" && !x.is_null()),
        Value::Array(a) => !a.is_empty(),
        _ => false,
    }
}

fn plural(n: u64, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// A scalar as short text.
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::Bool(b) => Some(if *b { "yes" } else { "no" }.into()),
        Value::Number(n) => Some(
            n.as_u64().map(|u| u.to_string()).or_else(|| n.as_i64().map(|i| i.to_string())).unwrap_or_else(|| number(n.as_f64().unwrap_or(f64::NAN))),
        ),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// The top-level scalars as `key value` pairs (lists as their length).
fn scalars(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let parts: Vec<String> = m
                .iter()
                .filter_map(|(k, x)| match x {
                    Value::Array(a) => Some(format!("{} {}", k.replace('_', " "), a.len())),
                    _ => scalar(x).map(|s| format!("{} {s}", k.replace('_', " "))),
                })
                .collect();
            parts.join(", ")
        }
        Value::Array(a) => plural(a.len() as u64, "item"),
        other => scalar(other).unwrap_or_default(),
    }
}

/// `s` on one line of at most [`MAX_LINE`] characters.
fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > MAX_LINE { format!("{}…", s.chars().take(MAX_LINE).collect::<String>()) } else { s }
}

/// What a click in the panel asks for, done after drawing.
enum Act {
    Close,
    Select(Def, String),
    Start(&'static str),
    Tab(Tab),
    Rerun,
    /// Pick a design entity in the viewport (a body, face or edge from the Bodies list).
    Pick(fieldcraft_model::GeometryRef),
}

/// The reply's section of the context panel.
pub fn show(app: &mut ModelApp, ui: &mut egui::Ui, t: &Tokens) {
    let Some(reply) = app.reply.clone() else { return };
    let mut acts = Vec::new();
    heading(ui, t, Level::Title, |ui| {
        ui.label(RichText::new(reply.label.to_uppercase()).font(label_font(ui.ctx(), 12.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("×").on_hover_text("Close").clicked() {
                acts.push(Act::Close);
            }
            if ui.small_button("Run again").on_hover_text(reply.command.as_str()).clicked() {
                acts.push(Act::Rerun);
            }
        });
    });
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(summary(&reply.command, &reply.value)).color(t.text_dim)).wrap());
    ui.add_space(4.0);
    match reply.command.as_str() {
        "model.check" => diagnostics(app, ui, t, &reply.value, &mut acts),
        "model.activity" => activity(ui, t, &reply.value),
        "geometry.bodies" => bodies(ui, t, &reply.value, &mut acts),
        _ => value(ui, t, egui::Id::new("fc_reply"), &reply.value, 0),
    }
    for a in acts {
        match a {
            Act::Close => app.reply = None,
            Act::Select(kind, name) => app.selected = Some((kind, name)),
            Act::Start(id) => {
                let _ = app.start(id);
            }
            Act::Tab(tab) => app.ui.select_tab(tab),
            Act::Rerun => {
                let _ = app.run(&reply.command, reply.params.clone());
            }
            Act::Pick(g) => app.select([crate::selection::Item::Geometry(g)], crate::selection::Mode::Replace),
        }
    }
}

/// The definition a diagnostic's subject names (``section `web` ``), when the model has it.
fn subject_definition(app: &ModelApp, subject: &str) -> Option<(Def, String)> {
    let (kind, rest) = subject.split_once(" `")?;
    let name = rest.strip_suffix('`')?;
    let kind: Def = serde_json::from_value(Value::String(kind.replace(' ', "_"))).ok()?;
    browser::definition(&app.session.model, kind, name).map(|_| (kind, name.to_string()))
}

/// Where a whole-model diagnostic is fixed: a button's text, tooltip and what it does.
fn model_fix(message: &str) -> Option<(&'static str, Act)> {
    if message.contains("mesh has no elements") {
        Some(("Mesh…", Act::Tab(Tab::Mesh)))
    } else if message.contains("no unit system") {
        Some(("Units…", Act::Start("model.units")))
    } else if message.contains("no analysis step") {
        Some(("Step…", Act::Start("model.step.create")))
    } else if message.contains("have no section") {
        Some(("Section…", Act::Start("model.section.create")))
    } else {
        None
    }
}

/// Check Model: each diagnostic with its severity, the item it is about and its message, and a
/// button to where it is fixed.
fn diagnostics(app: &ModelApp, ui: &mut egui::Ui, t: &Tokens, v: &Value, acts: &mut Vec<Act>) {
    let list = v["diagnostics"].as_array().map(Vec::as_slice).unwrap_or_default();
    if list.is_empty() {
        ui.label(RichText::new("✔ The model is ready to solve.").color(t.ok));
        return;
    }
    for (i, d) in list.iter().enumerate() {
        let error = d["severity"] == "error";
        let subject = d["subject"].as_str().unwrap_or_default();
        let message = d["message"].as_str().unwrap_or_default();
        ui.horizontal_top(|ui| {
            let (mark, colour, tip) = if error { ("⛔", t.error, "Error: blocks solving") } else { ("⚠", t.warm, "Warning") };
            ui.label(RichText::new(mark).color(colour)).on_hover_text(tip);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 70.0);
                ui.label(RichText::new(subject).strong());
                ui.add(egui::Label::new(message).wrap());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if let Some((kind, name)) = subject_definition(app, subject) {
                    if ui.small_button("Show").on_hover_text(format!("Select {subject} to see and edit it")).clicked() {
                        acts.push(Act::Select(kind, name));
                    }
                } else if subject == "model"
                    && let Some((text, act)) = model_fix(message)
                    && ui.small_button(text).clicked()
                {
                    acts.push(act);
                }
            });
        });
        if i + 1 < list.len() {
            ui.separator();
        }
    }
}

/// Faces or edges listed under a body at most.
const MAX_ENTITIES: usize = 500;

/// Geometry › Bodies: one entry per body, its name first, then volume, area, face and edge counts
/// and its size; a click on the name picks the body, and its faces and edges unfold, by
/// persistent name (edges with their lengths), each picked by a click: the names the seed and set
/// forms ask for. Design geometry is in millimetres.
fn bodies(ui: &mut egui::Ui, t: &Tokens, v: &Value, acts: &mut Vec<Act>) {
    let q = crate::describe::quantity;
    let pick = |body: &str, entity: fieldcraft_model::Entity| fieldcraft_model::GeometryRef { body: body.to_string(), entity };
    for (i, b) in v.as_array().into_iter().flatten().enumerate() {
        let name = b["name"].as_str().unwrap_or_default();
        let faces = b["faces"].as_array().map(Vec::as_slice).unwrap_or_default();
        let edges = b["edges"].as_array().map(Vec::as_slice).unwrap_or_default();
        let corner = |k: usize| -> Option<[f64; 3]> { serde_json::from_value(b["bbox"][k].clone()).ok() };
        let size =
            corner(0).zip(corner(1)).map(|(lo, hi)| format!("{} × {} × {} mm", q(sig(hi[0] - lo[0])), q(sig(hi[1] - lo[1])), q(sig(hi[2] - lo[2]))));
        let id = egui::Id::new(("fc_reply_body", i));
        if ui.link(RichText::new(name).strong()).on_hover_text("Pick this body in the viewport").clicked() {
            acts.push(Act::Pick(pick(name, fieldcraft_model::Entity::Body)));
        }
        egui::Grid::new(id.with("grid")).num_columns(2).spacing(vec2(10.0, 2.0)).show(ui, |ui| {
            let mut row = |k: &str, text: String| {
                ui.label(RichText::new(k).color(t.text_dim));
                ui.label(text);
                ui.end_row();
            };
            if let Some(v) = b["volume"].as_f64() {
                row("Volume", format!("{} mm³", q(sig(v))));
            }
            if let Some(a) = b["area"].as_f64() {
                row("Area", format!("{} mm²", q(sig(a))));
            }
            row("Faces, edges", format!("{}, {}", faces.len(), edges.len()));
            if let Some(size) = size {
                row("Size", size);
            }
        });
        body_list(ui, t, id.with("faces"), "Faces", faces, &|f| (f["name"].as_str().unwrap_or_default().to_string(), None), name, acts);
        body_list(
            ui,
            t,
            id.with("edges"),
            "Edges",
            edges,
            &|e| (e["name"].as_str().unwrap_or_default().to_string(), e["length"].as_f64()),
            name,
            acts,
        );
        ui.separator();
    }
}

/// A body's faces or edges, folded: each name picks it in the viewport (edges with their length).
#[allow(clippy::too_many_arguments)]
fn body_list(
    ui: &mut egui::Ui,
    t: &Tokens,
    id: egui::Id,
    title: &str,
    items: &[Value],
    read: &dyn Fn(&Value) -> (String, Option<f64>),
    body: &str,
    acts: &mut Vec<Act>,
) {
    if items.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(RichText::new(format!("{title} ({})", items.len())).size(12.0)).id_salt(id).show(ui, |ui| {
        for item in items.iter().take(MAX_ENTITIES) {
            let (name, length) = read(item);
            ui.horizontal(|ui| {
                if ui.link(&name).on_hover_text("Pick it in the viewport").clicked() {
                    let entity =
                        if title == "Edges" { fieldcraft_model::Entity::Edge(name.clone()) } else { fieldcraft_model::Entity::Face(name.clone()) };
                    acts.push(Act::Pick(fieldcraft_model::GeometryRef { body: body.to_string(), entity }));
                }
                if let Some(l) = length {
                    ui.label(RichText::new(format!("{} mm", crate::describe::quantity(sig(l)))).color(t.text_dim));
                }
            });
        }
        more_than(ui, t, items.len(), MAX_ENTITIES);
    });
}

/// `v` to 3 significant figures.
fn sig(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let scale = 10f64.powi(2 - v.abs().log10().floor() as i32);
    (v * scale).round() / scale
}

fn more_than(ui: &mut egui::Ui, t: &Tokens, n: usize, max: usize) {
    if n > max {
        ui.label(RichText::new(format!("… {} more", n - max)).small().color(t.text_dim));
    }
}

/// The Step Table: each boundary condition and load against the steps, ticked where active.
fn activity(ui: &mut egui::Ui, t: &Tokens, v: &Value) {
    let steps: Vec<&str> = v["steps"].as_array().into_iter().flatten().filter_map(|s| s["name"].as_str()).collect();
    if steps.is_empty() {
        ui.label(RichText::new("No analysis steps yet: Steps › Step creates one.").color(t.text_dim));
        return;
    }
    egui::ScrollArea::horizontal().id_salt("fc_reply_activity").show(ui, |ui| {
        egui::Grid::new("fc_reply_activity_grid").striped(true).spacing(vec2(10.0, 3.0)).show(ui, |ui| {
            ui.label("");
            for (s, step) in steps.iter().zip(v["steps"].as_array().into_iter().flatten()) {
                ui.label(RichText::new(*s).strong()).on_hover_text(step["procedure"].as_str().unwrap_or_default());
            }
            ui.end_row();
            for (key, kind) in [("boundary_conditions", "BC"), ("loads", "Load")] {
                for (name, active) in v[key].as_object().into_iter().flatten() {
                    ui.label(name.as_str()).on_hover_text(kind);
                    for s in &steps {
                        let on = active.as_array().is_some_and(|a| a.iter().any(|x| x == *s));
                        ui.label(if on { RichText::new("✔").color(t.ok) } else { RichText::new("—").color(t.text_dim) });
                    }
                    ui.end_row();
                }
            }
        });
    });
}

/// Any reply: objects as labelled rows, lists of objects as tables, nested parts folded.
fn value(ui: &mut egui::Ui, t: &Tokens, id: egui::Id, v: &Value, depth: usize) {
    match v {
        Value::Object(m) => {
            egui::Grid::new(id).num_columns(2).spacing(vec2(10.0, 3.0)).show(ui, |ui| {
                for (k, x) in m.iter().filter(|(k, x)| k.as_str() != "summary" && inline(x).is_some()) {
                    ui.label(RichText::new(k.replace('_', " ")).color(t.text_dim));
                    ui.add(egui::Label::new(inline(x).unwrap_or_default()).wrap());
                    ui.end_row();
                }
            });
            for (k, x) in m.iter().filter(|(_, x)| inline(x).is_none()) {
                let title = match x {
                    Value::Array(a) => format!("{} ({})", k.replace('_', " "), a.len()),
                    _ => k.replace('_', " "),
                };
                egui::CollapsingHeader::new(RichText::new(title).size(12.0)).id_salt(id.with(k)).default_open(depth == 0).show(ui, |ui| {
                    value(ui, t, id.with(k), x, depth + 1);
                });
            }
        }
        Value::Array(a) if depth < MAX_DEPTH && a.iter().all(Value::is_object) => table(ui, t, id, a),
        Value::Array(a) if depth < MAX_DEPTH => {
            for (i, x) in a.iter().take(MAX_ROWS).enumerate() {
                value(ui, t, id.with(i), x, depth + 1);
            }
            more(ui, t, a.len());
        }
        other => {
            ui.add(egui::Label::new(inline(other).unwrap_or_else(|| crate::context::value_text(other))).wrap());
        }
    }
}

/// A value short enough for one cell: scalars, and lists of up to four scalars.
fn inline(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("—".into()),
        Value::Array(a) if a.len() <= 4 && a.iter().all(|x| !x.is_object() && !x.is_array()) => {
            Some(a.iter().filter_map(scalar).collect::<Vec<_>>().join(", "))
        }
        Value::Array(_) | Value::Object(_) => None,
        other => scalar(other),
    }
}

/// A list of objects as a table: the columns are their keys (first seen first).
fn table(ui: &mut egui::Ui, t: &Tokens, id: egui::Id, rows: &[Value]) {
    let mut columns: Vec<&str> = Vec::new();
    for r in rows.iter().take(MAX_ROWS) {
        for k in r.as_object().into_iter().flat_map(Map::keys) {
            if !columns.contains(&k.as_str()) && columns.len() < MAX_COLUMNS {
                columns.push(k);
            }
        }
    }
    egui::ScrollArea::horizontal().id_salt(id.with("scroll")).show(ui, |ui| {
        egui::Grid::new(id).striped(true).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
            for c in &columns {
                ui.label(RichText::new(c.replace('_', " ")).small().color(t.text_dim));
            }
            ui.end_row();
            for r in rows.iter().take(MAX_ROWS) {
                for c in &columns {
                    ui.label(r.get(*c).map_or_else(String::new, |x| inline(x).unwrap_or_else(|| crate::context::value_text(x))));
                }
                ui.end_row();
            }
        });
    });
    more(ui, t, rows.len());
}

fn more(ui: &mut egui::Ui, t: &Tokens, n: usize) {
    if n > MAX_ROWS {
        ui.label(RichText::new(format!("… {} more", n - MAX_ROWS)).small().color(t.text_dim));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn summaries_are_sentences() {
        let check = json!({"ready": false, "errors": 2, "warnings": 1, "diagnostics": []});
        assert_eq!(summary("model.check", &check), "2 errors, 1 warning");
        assert_eq!(summary("model.check", &json!({"errors": 0, "warnings": 0})), "ready to solve, no issues");
        let activity = json!({"boundary_conditions": {"Clamp": ["Load"]}, "loads": {}, "steps": [{"name": "Load"}]});
        assert_eq!(summary("model.activity", &activity), "1 step, 1 boundary condition, 0 loads");
        assert_eq!(summary("geometry.bodies", &json!([])), "no bodies (open a design first)");
        assert_eq!(summary("geometry.bodies", &json!([{}, {}])), "2 bodies");
        assert_eq!(summary("solve.static", &json!({"dofs": 945, "modes": [1, 2], "x": {"y": 1}})), "dofs 945, modes 2");
        assert_eq!(summary("any", &json!({"summary": "a\nb"})), "a b");
        assert_eq!(summary("any", &json!({})), "done");
        assert!(summary("any", &json!({"s": "x".repeat(500)})).chars().count() <= MAX_LINE + 1);
    }

    #[test]
    fn model_level_diagnostics_lead_to_their_fix() {
        let mut s = fieldcraft_engine::Session::default();
        let check = s.execute("model.check", &json!({})).unwrap();
        let messages: Vec<&str> = check["diagnostics"].as_array().unwrap().iter().filter_map(|d| d["message"].as_str()).collect();
        assert!(messages.len() >= 3, "{check}");
        for m in messages {
            assert!(model_fix(m).is_some(), "no fix for `{m}`");
        }
    }
}
