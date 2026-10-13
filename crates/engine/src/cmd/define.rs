//! Model definition commands: create, edit and delete sets, materials, sections, reference
//! points, couplings, boundary conditions, loads and steps, and the step activity table.
//!
//! Each definition's parameters are its JSON form in the model (the same form a `.modelcraft`
//! file stores), plus its `name`. `create` fails when the name is taken, `edit` replaces an
//! existing definition, `delete` refuses while another definition refers to it.

use std::collections::BTreeSet;

use modelcraft_model::{BoundaryCondition, Coupling, Load, Material, Model, OutputVariable, Procedure, ReferencePoint, Section, Set, Step};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{CommandSpec, EngineError, Result, Session};

/// The `name` parameter.
fn name(p: &Value) -> Result<&str> {
    p.get("name").and_then(Value::as_str).ok_or_else(|| EngineError::Params("`name` must be the definition's name".into()))
}

/// The parameters other than `name`, read as a definition's JSON form.
fn fields<T: DeserializeOwned>(p: &Value, what: &str) -> Result<T> {
    let mut obj = p.as_object().cloned().ok_or_else(|| EngineError::Params(format!("the parameters of a {what} are a JSON object")))?;
    obj.remove("name");
    serde_json::from_value(Value::Object(obj)).map_err(|e| EngineError::Params(format!("{what}: {e}")))
}

/// Create, edit and delete commands for one kind of definition.
macro_rules! definition_commands {
    ($what:literal, $ty:ty, $id:literal, $label:literal, $doc:literal, $add:ident, $replace:ident, $remove:ident,
     $CREATE:ident, $EDIT:ident, $DELETE:ident) => {
        pub const $CREATE: CommandSpec = CommandSpec {
            id: concat!("model.", $id, ".create"),
            label: concat!("New ", $label),
            params: concat!($doc, " — fails if the name is taken"),
            run: |s: &mut Session, p: &Value| {
                let n = name(p)?;
                s.model.$add(n, fields::<$ty>(p, $what)?)?;
        Ok(json!({"name": n}))
            },
        };
        pub const $EDIT: CommandSpec = CommandSpec {
            id: concat!("model.", $id, ".edit"),
            label: concat!("Edit ", $label),
            params: concat!($doc, " — replaces the existing definition of that name"),
            run: |s: &mut Session, p: &Value| {
                let n = name(p)?;
                s.model.$replace(n, fields::<$ty>(p, $what)?)?;
        Ok(json!({"name": n}))
            },
        };
        pub const $DELETE: CommandSpec = CommandSpec {
            id: concat!("model.", $id, ".delete"),
            label: concat!("Delete ", $label),
            params: r#"{"name": "…"} — refused while another definition refers to it"#,
            run: |s: &mut Session, p: &Value| {
                s.model.$remove(name(p)?)?;
                Ok(Value::Null)
            },
        };
    };
}

definition_commands!(
    "set",
    Set,
    "set",
    "Set",
    r#"{"name": "root", "geometry": [{"body": "Body1", "face": "F2:top"}, {"body": "Body1", "edge": "…"}, {"body": "Body1", "vertex": "…"}, {"body": "Body1"}], "nodes": [labels], "elements": [labels], "faces": [{"element": 1, "face": 3}]} — any of the member lists; geometry members resolve against the mesh's geometry tags after every remesh (a face name also selects its split pieces `name#0`, `name#1`, …)"#,
    add_set,
    replace_set,
    remove_set,
    SET_CREATE,
    SET_EDIT,
    SET_DELETE
);

definition_commands!(
    "material",
    Material,
    "material",
    "Material",
    r#"{"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3} | {"type": "engineering_constants", "young": [E1, E2, E3], "poisson": [ν12, ν13, ν23], "shear": [G12, G13, G23]}, "density": 7.85e-9} — in the model's unit system (model.units), never converted"#,
    add_material,
    replace_material,
    remove_material,
    MATERIAL_CREATE,
    MATERIAL_EDIT,
    MATERIAL_DELETE
);

definition_commands!(
    "section",
    Section,
    "section",
    "Section",
    r#"{"name": "web", "set": "all", "material": "Steel", "kind": {"type": "solid"} | {"type": "shell", "thickness": 2, "offset": 0 (-0.5..=0.5 of the thickness), "points": 5} | {"type": "beam", "profile": {"type": "rectangle", "width", "height"} | {"type": "circle", "radius"} | {"type": "pipe", "radius", "thickness"} | {"type": "i", "height", "width", "web", "flange"} | {"type": "box", "width", "height", "thickness"}, "orientation": [x, y, z]} | {"type": "rod", "area": 10}, "formulation": {"integration": "full|reduced", "incompatible_modes": false, "hybrid": false}}"#,
    add_section,
    replace_section,
    remove_section,
    SECTION_CREATE,
    SECTION_EDIT,
    SECTION_DELETE
);

definition_commands!(
    "reference point",
    ReferencePoint,
    "reference_point",
    "Reference Point",
    r#"{"name": "RP", "xyz": [x, y, z]}"#,
    add_reference_point,
    replace_reference_point,
    remove_reference_point,
    REFERENCE_POINT_CREATE,
    REFERENCE_POINT_EDIT,
    REFERENCE_POINT_DELETE
);

definition_commands!(
    "coupling",
    Coupling,
    "coupling",
    "Coupling",
    r#"{"name": "pad", "reference_point": "RP", "set": "hole", "kind": "kinematic|distributing", "dofs": [1, 2, 3, 4, 5, 6] (default all)}"#,
    add_coupling,
    replace_coupling,
    remove_coupling,
    COUPLING_CREATE,
    COUPLING_EDIT,
    COUPLING_DELETE
);

definition_commands!(
    "boundary condition",
    BoundaryCondition,
    "boundary_condition",
    "Boundary Condition",
    r#"{"name": "clamp", "type": "fixed", "target": {"set": "root"} | {"reference_point": "RP"}} | {"name", "type": "displacement", "target", "values": [u1, u2, u3, ur1, ur2, ur3] (null leaves a degree of freedom free)} | {"name", "type": "symmetry", "target", "plane": "x|y|z" (the plane's normal)} — active in the steps that list it (model.step.activate)"#,
    add_boundary_condition,
    replace_boundary_condition,
    remove_boundary_condition,
    BOUNDARY_CONDITION_CREATE,
    BOUNDARY_CONDITION_EDIT,
    BOUNDARY_CONDITION_DELETE
);

definition_commands!(
    "load",
    Load,
    "load",
    "Load",
    r#"{"name": "push", "type": "force|moment", "target": {"set": "…"} | {"reference_point": "…"}, "vector": [x, y, z] (at every node of the target)} | {"name", "type": "pressure", "set", "magnitude" (positive pushes in)} | {"name", "type": "gravity", "acceleration": [x, y, z], "set" (optional, default the whole model)} | {"name", "type": "body_force", "set", "vector" (per volume)} — active in the steps that list it (model.step.activate)"#,
    add_load,
    replace_load,
    remove_load,
    LOAD_CREATE,
    LOAD_EDIT,
    LOAD_DELETE
);

/// An optional list of names.
fn names(p: &Value, key: &str) -> Result<Option<BTreeSet<String>>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|_| EngineError::Params(format!("`{key}` must be a list of names"))),
    }
}

fn procedure(p: &Value) -> Result<Procedure> {
    let v = p
        .get("procedure")
        .ok_or_else(|| EngineError::Params("`procedure` must be {\"type\": \"static\"} or {\"type\": \"frequency\", \"modes\": 10}".into()))?;
    serde_json::from_value(v.clone()).map_err(|e| EngineError::Params(format!("procedure: {e}")))
}

fn output(p: &Value) -> Result<Option<BTreeSet<OutputVariable>>> {
    match p.get("output") {
        None => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|_| EngineError::Params("`output` must be a list of displacement, reaction_force, stress, strain".into())),
    }
}

pub const STEP_CREATE: CommandSpec = CommandSpec {
    id: "model.step.create",
    label: "New Step",
    params: r#"{"name": "Bend", "procedure": {"type": "static"} | {"type": "frequency", "modes": 10, "min_hz": 0, "max_hz": 500}, "output": ["displacement", "reaction_force", "stress", "strain"] (default by procedure), "boundary_conditions": [names], "loads": [names] (active in this step; nothing carries over between steps), "before": "step" (default: last)}"#,
    run: step_create,
};

pub const STEP_EDIT: CommandSpec = CommandSpec {
    id: "model.step.edit",
    label: "Edit Step",
    params: r#"{"name": "Bend", "procedure", "output", "boundary_conditions", "loads"} — as model.step.create; what is left out keeps its value"#,
    run: step_edit,
};

pub const STEP_DELETE: CommandSpec = CommandSpec {
    id: "model.step.delete",
    label: "Delete Step",
    params: r#"{"name": "Bend"}"#,
    run: |s, p| s.model.remove_step(name(p)?).map(|_| Value::Null).map_err(Into::into),
};

pub const STEP_ACTIVATE: CommandSpec = CommandSpec {
    id: "model.step.activate",
    label: "Activate in Step",
    params: r#"{"step": "Bend", "boundary_conditions": [names], "loads": [names], "active": true} — switches the listed items on (or off with false) in that step only"#,
    run: step_activate,
};

pub const ACTIVITY: CommandSpec = CommandSpec { id: "model.activity", label: "Step Table", params: "{}", run: |s, _| Ok(activity(&s.model)) };

fn step_create(s: &mut Session, p: &Value) -> Result<Value> {
    let n = name(p)?;
    let procedure = procedure(p)?;
    let mut step = Step::new(n, procedure);
    if let Some(o) = output(p)? {
        step.output = o;
    }
    step.boundary_conditions = names(p, "boundary_conditions")?.unwrap_or_default();
    step.loads = names(p, "loads")?.unwrap_or_default();
    let before = match p.get("before") {
        None => None,
        Some(v) => Some(v.as_str().ok_or_else(|| EngineError::Params("`before` must be a step name".into()))?),
    };
    s.model.add_step(step, before)?;
    Ok(json!({"name": n}))
}

fn step_edit(s: &mut Session, p: &Value) -> Result<Value> {
    let n = name(p)?;
    let mut step = s.model.step(n).cloned().ok_or_else(|| EngineError::Params(format!("no step `{n}`")))?;
    if p.get("procedure").is_some() {
        step.procedure = procedure(p)?;
    }
    if let Some(o) = output(p)? {
        step.output = o;
    }
    if let Some(b) = names(p, "boundary_conditions")? {
        step.boundary_conditions = b;
    }
    if let Some(l) = names(p, "loads")? {
        step.loads = l;
    }
    s.model.replace_step(step)?;
    Ok(json!({"name": n}))
}

fn step_activate(s: &mut Session, p: &Value) -> Result<Value> {
    let step = p.get("step").and_then(Value::as_str).ok_or_else(|| EngineError::Params("`step` must be a step name".into()))?;
    let active = match p.get("active") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| EngineError::Params("`active` must be true or false".into()))?,
    };
    let bcs = names(p, "boundary_conditions")?.unwrap_or_default();
    let loads = names(p, "loads")?.unwrap_or_default();
    if bcs.is_empty() && loads.is_empty() {
        return Err(EngineError::Params("list `boundary_conditions` or `loads` to switch".into()));
    }
    for b in &bcs {
        s.model.set_boundary_condition_active(step, b, active)?;
    }
    for l in &loads {
        s.model.set_load_active(step, l, active)?;
    }
    Ok(activity(&s.model))
}

/// The step × boundary-condition/load table: the steps in order, and for each item the steps it
/// is active in.
fn activity(m: &Model) -> Value {
    let row =
        |active: &dyn Fn(&Step) -> bool| -> Value { m.steps().iter().filter(|s| active(s)).map(|s| s.name.as_str()).collect::<Vec<_>>().into() };
    let bcs: serde_json::Map<String, Value> =
        m.boundary_conditions().keys().map(|n| (n.clone(), row(&|s| s.boundary_conditions.contains(n)))).collect();
    let loads: serde_json::Map<String, Value> = m.loads().keys().map(|n| (n.clone(), row(&|s| s.loads.contains(n)))).collect();
    json!({
        "steps": m.steps().iter().map(|s| json!({"name": s.name, "procedure": s.procedure.name()})).collect::<Vec<_>>(),
        "boundary_conditions": bcs,
        "loads": loads,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use crate::Session;

    /// A two-element bar tagged with body `Bar` and faces `left` and `right`.
    pub(crate) fn tagged_bar(s: &mut Session) {
        for (i, x) in [0.0, 1.0, 2.0].into_iter().enumerate() {
            for (j, (y, z)) in [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)].into_iter().enumerate() {
                s.execute("mesh.node.add", &json!({"label": 1 + i * 4 + j, "xyz": [x, y, z]})).unwrap();
            }
        }
        // Node (i, y, z) = 1 + 4i + 2y + z.
        let n = |i: u32, y: u32, z: u32| 1 + 4 * i + 2 * y + z;
        for e in 0..2 {
            let nodes = [n(e, 0, 0), n(e + 1, 0, 0), n(e + 1, 1, 0), n(e, 1, 0), n(e, 0, 1), n(e + 1, 0, 1), n(e + 1, 1, 1), n(e, 1, 1)];
            s.execute("mesh.element.add", &json!({"label": e + 1, "kind": "hex8", "nodes": nodes})).unwrap();
        }
        s.execute("mesh.tag", &json!({"geometry": {"body": "Bar"}, "elements": [1, 2]})).unwrap();
        s.execute("mesh.tag", &json!({"geometry": {"body": "Bar", "face": "left"}, "faces": [{"element": 1, "face": 6}]})).unwrap();
        s.execute("mesh.tag", &json!({"geometry": {"body": "Bar", "face": "right"}, "faces": [{"element": 2, "face": 4}]})).unwrap();
    }

    pub(crate) fn defined_bar(s: &mut Session) {
        tagged_bar(s);
        let script = json!([
            {"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}},
            {"command": "model.set.create", "params": {"name": "all", "geometry": [{"body": "Bar"}]}},
            {"command": "model.set.create", "params": {"name": "root", "geometry": [{"body": "Bar", "face": "left"}]}},
            {"command": "model.set.create", "params": {"name": "end", "geometry": [{"body": "Bar", "face": "right"}]}},
            {"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}, "density": 7.85e-9}},
            {"command": "model.section.create", "params": {"name": "bar", "set": "all", "material": "Steel", "kind": {"type": "solid"}, "formulation": {"incompatible_modes": true}}},
            {"command": "model.reference_point.create", "params": {"name": "RP", "xyz": [3, 0.5, 0.5]}},
            {"command": "model.coupling.create", "params": {"name": "pad", "reference_point": "RP", "set": "end", "kind": "kinematic"}},
            {"command": "model.boundary_condition.create", "params": {"name": "clamp", "type": "fixed", "target": {"set": "root"}}},
            {"command": "model.load.create", "params": {"name": "push", "type": "force", "target": {"reference_point": "RP"}, "vector": [0, 0, -100]}},
            {"command": "model.load.create", "params": {"name": "press", "type": "pressure", "set": "end", "magnitude": 2}},
            {"command": "model.step.create", "params": {"name": "Bend", "procedure": {"type": "static"}, "boundary_conditions": ["clamp"], "loads": ["push"]}},
            {"command": "model.step.create", "params": {"name": "Modes", "procedure": {"type": "frequency", "modes": 6}, "boundary_conditions": ["clamp"]}},
        ]);
        s.run_script(&script).unwrap();
    }

    #[test]
    fn a_script_defines_a_model_that_checks_clean() {
        let mut s = Session::default();
        defined_bar(&mut s);
        let check = s.execute("model.check", &json!({})).unwrap();
        assert_eq!(check["ready"], true, "{check}");
        // `press` is defined but in no step: a warning, not an error.
        assert_eq!(check["warnings"], 1, "{check}");
        assert!(check["diagnostics"][0]["subject"] == "load `press`", "{check}");

        let r = s.execute("model.set.resolve", &json!({"name": "end"})).unwrap();
        assert_eq!(r["nodes"], json!({"count": 4, "labels": [9, 10, 11, 12]}));
        assert_eq!(r["faces"]["labels"], json!([{"element": 2, "face": 4}]));

        let table = s.execute("model.step.activate", &json!({"step": "Bend", "loads": ["press"]})).unwrap();
        assert_eq!(table["loads"]["press"], json!(["Bend"]));
        assert_eq!(table["boundary_conditions"]["clamp"], json!(["Bend", "Modes"]));
        assert_eq!(s.execute("model.check", &json!({})).unwrap()["warnings"], 0);
        let off = s.execute("model.step.activate", &json!({"step": "Bend", "loads": ["press", "push"], "active": false})).unwrap();
        assert_eq!(off["loads"], json!({"press": [], "push": []}));

        let info = s.execute("model.info", &json!({})).unwrap();
        assert_eq!(info["units"], "mm-t-s-N-MPa");
        assert_eq!(info["definitions"]["sets"], 3);
        assert_eq!(info["definitions"]["steps"], 2);
    }

    #[test]
    fn edits_and_deletes_keep_the_model_consistent() {
        let mut s = Session::default();
        defined_bar(&mut s);
        let journal = s.journal.len();

        // Refused: a taken name, a missing reference, a referenced item, a typo in a field.
        assert!(s.execute("model.set.create", &json!({"name": "all", "nodes": [1]})).is_err());
        assert!(s.execute("model.section.create", &json!({"name": "x", "set": "nope", "material": "Steel", "kind": {"type": "solid"}})).is_err());
        let e = s.execute("model.set.delete", &json!({"name": "end"})).unwrap_err().to_string();
        assert!(e.contains("coupling `pad`") && e.contains("load `press`"), "{e}");
        assert!(
            s.execute("model.material.create", &json!({"name": "Al", "elastic": {"type": "isotropic", "young": 70000, "poison": 0.33}})).is_err()
        );
        assert!(s.execute("model.step.edit", &json!({"name": "Bend", "loads": ["nope"]})).is_err());
        assert!(s.execute("model.step.create", &json!({"name": "Late", "procedure": {"type": "static"}, "before": "nope"})).is_err());
        assert_eq!(s.journal.len(), journal, "refusals change nothing");

        // Edits replace; the step keeps its table entries unless told otherwise.
        s.execute(
            "model.material.edit",
            &json!({"name": "Steel", "elastic": {"type": "isotropic", "young": 200000, "poisson": 0.29}, "density": 7.8e-9}),
        )
        .unwrap();
        s.execute("model.step.edit", &json!({"name": "Modes", "procedure": {"type": "frequency", "modes": 12, "max_hz": 1000}})).unwrap();
        assert!(s.model.step("Modes").unwrap().boundary_conditions.contains("clamp"));
        s.execute(
            "model.step.create",
            &json!({"name": "Pre", "procedure": {"type": "static"}, "before": "Bend", "boundary_conditions": ["clamp"], "loads": ["press"]}),
        )
        .unwrap();
        assert_eq!(s.model.steps().iter().map(|st| st.name.as_str()).collect::<Vec<_>>(), ["Pre", "Bend", "Modes"]);

        // Deleting a load clears it from every step.
        s.execute("model.load.delete", &json!({"name": "press"})).unwrap();
        assert!(s.model.step("Pre").unwrap().loads.is_empty());
        s.execute("model.coupling.delete", &json!({"name": "pad"})).unwrap();
        s.execute("model.set.delete", &json!({"name": "end"})).unwrap();
        s.execute("model.step.delete", &json!({"name": "Pre"})).unwrap();
        assert!(s.execute("model.step.delete", &json!({"name": "Pre"})).is_err());
    }

    #[test]
    fn the_check_names_the_bad_items_and_sets_follow_a_retag() {
        let mut s = Session::default();
        defined_bar(&mut s);
        // Point the section at a face set: it has no elements, and the elements lose their section.
        s.execute("model.section.edit", &json!({"name": "bar", "set": "root", "material": "Steel", "kind": {"type": "solid"}})).unwrap();
        let check = s.execute("model.check", &json!({})).unwrap();
        assert_eq!(check["ready"], false);
        let messages: Vec<String> = check["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| format!("{}: {}", d["subject"].as_str().unwrap(), d["message"].as_str().unwrap()))
            .collect();
        assert!(messages.contains(&"section `bar`: set `root` has no elements".to_string()), "{messages:#?}");
        assert!(messages.contains(&"model: elements 1, 2 have no section".to_string()), "{messages:#?}");

        // Clearing the tags leaves geometry sets unresolved, and says so.
        s.execute("mesh.tag.clear", &json!({})).unwrap();
        let e = s.execute("model.set.resolve", &json!({"name": "root"})).unwrap_err().to_string();
        assert!(e.contains("face left of body Bar is not on the mesh (the mesh carries no geometry tags)"), "{e}");
    }
}
