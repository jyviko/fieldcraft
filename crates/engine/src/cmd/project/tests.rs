use serde_json::{Value, json};

use crate::Session;

/// A session with the design the SolidCraft commands build.
fn session(commands: Value) -> (crate::tests::TempDir, Session) {
    let bytes = fieldcraft_geometry::script_design(&json!({ "commands": commands }).to_string()).unwrap();
    let (dir, path) = crate::tests::design_file(&bytes);
    let mut s = Session::default();
    s.execute("geometry.open", &json!({"path": path})).unwrap();
    (dir, s)
}

fn sj(r: &Value) -> f64 {
    r["min_scaled_jacobian"].as_f64().unwrap()
}

fn faces(s: &mut Session, body: &str) -> Vec<String> {
    let b = s.execute("geometry.bodies", &json!({})).unwrap();
    let body = b.as_array().unwrap().iter().find(|x| x["name"] == body).unwrap().clone();
    body["faces"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect()
}

/// The 7-block sphere at two refinements: all hex, min scaled Jacobian at least 0.5, the volume
/// converging, and a set on the sphere's face resolving to the projected boundary.
#[test]
fn sphere_seven_blocks_at_two_refinements() {
    let (_d, mut s) = session(json!([{"command": "solid.sphere", "params": {"radius": 10, "body_name": "Ball"}}]));
    let exact = 4.0 / 3.0 * std::f64::consts::PI * 1000.0;
    let mut errors = Vec::new();
    for size in [3.0, 1.5] {
        let r = s.execute("mesh.hex.project", &json!({"body": "Ball", "template": "sphere", "size": size})).unwrap();
        assert_eq!(r["blocks"], 7, "{r}");
        assert!(sj(&r) >= 0.5, "{r}");
        errors.push((r["volume"].as_f64().unwrap() - exact).abs() / exact);
        assert_eq!(s.model.elements().len() as u64, r["elements"].as_u64().unwrap());
    }
    assert!(errors[1] < errors[0] && errors[1] < 0.03, "{errors:?}");
    let names = faces(&mut s, "Ball");
    let geometry: Vec<Value> = names.iter().map(|f| json!({"body": "Ball", "face": f})).collect();
    s.execute("model.set.create", &json!({"name": "skin", "geometry": geometry})).unwrap();
    let skin = s.model.resolve_set("skin").unwrap();
    assert!(!skin.faces.is_empty());
    let q = s.execute("mesh.quality", &json!({})).unwrap();
    assert_eq!(q["valid"].as_u64(), Some(s.model.elements().len() as u64), "{q}");
}

/// A disc: the five-block butterfly, its rims on both circles.
#[test]
fn disc_butterfly() {
    let (_d, mut s) = session(json!([{"command": "solid.cylinder", "params": {"radius": 10, "height": 2, "body_name": "Disc"}}]));
    let r = s.execute("mesh.hex.project", &json!({"body": "Disc", "template": "butterfly", "size": 1})).unwrap();
    assert_eq!(r["blocks"], 5, "{r}");
    assert!(sj(&r) >= 0.5, "{r}");
    let v = r["volume"].as_f64().unwrap();
    assert!((v - std::f64::consts::PI * 200.0).abs() < 0.01 * 200.0 * std::f64::consts::PI, "{r}");
    assert_eq!(r["targets"].as_array().unwrap().len(), 3, "{r}");
}

/// A cylinder with a side branch: the main tube a butterfly along X, the top block of its middle
/// layer O-gridded open upward and its open face extruded into the branch; the junction edges go
/// onto the curve where the two cylinders meet.
#[test]
fn cylinder_with_a_side_branch() {
    let (_d, mut s) = session(json!([
        {"command": "solid.cylinder", "params": {"base": [-30, 0, 0], "axis": "X", "radius": 10, "height": 60, "body_name": "Tee"}},
        {"command": "solid.cylinder", "params": {"base": [0, 0, 0], "axis": "Z", "radius": 7, "height": 25, "operation": "join", "targets": ["Tee"]}}
    ]));
    let r = s
        .execute(
            "mesh.hex.project",
            &json!({"body": "Tee", "template": "butterfly", "center": [0, 0, 0], "axis": [1, 0, 0], "radius": 10, "levels": [-30, -7, 7, 30], "size": 1.5,
                "operations": [{"op": "o_grid", "block": 8, "open": ["+z"], "inset": 0.5, "extrude": {"offset": [0, 0, 18], "layers": 2}}]}),
        )
        .unwrap();
    assert!(sj(&r) >= 0.3, "{r}");
    let exact = std::f64::consts::PI * (100.0 * 60.0) + std::f64::consts::PI * 49.0 * 15.0;
    assert!((r["volume"].as_f64().unwrap() - exact).abs() < 0.03 * exact, "{r}");
    assert!(r["max_gap"].as_f64().unwrap() < 1e-6, "{r}");
}

#[test]
fn hostile_parameters_change_nothing() {
    let (_d, mut s) = session(json!([{"command": "solid.sphere", "params": {"radius": 10, "body_name": "Ball"}}]));
    let model = s.model.clone();
    for bad in [
        json!({"body": "Nope", "template": "sphere"}),
        json!({"body": "Ball", "template": "teapot"}),
        json!({"body": "Ball", "template": "sphere", "size": -1}),
        json!({"body": "Ball", "template": "sphere", "size": 1e-9}),
        json!({"body": "Ball", "template": "sphere", "radius": f64::MAX}),
        json!({"body": "Ball", "template": "sphere", "inset": 2}),
        json!({"body": "Ball", "template": "sphere", "smoothing": "magic"}),
        json!({"body": "Ball", "template": "sphere", "iterations": 1_000_000}),
        json!({"body": "Ball", "template": "sphere", "faces": [{"block": 0, "face": 1, "to": ["nope"]}]}),
        json!({"body": "Ball", "template": "sphere", "divisions": [{"block": 99, "direction": 1, "count": 2}]}),
        json!({"body": "Ball", "template": "sphere", "divisions": [{"block": 0, "direction": 4, "count": 2}]}),
        json!({"body": "Ball", "template": "sphere", "operations": [{"op": "remove", "blocks": [0, 1, 2, 3, 4, 5, 6]}]}),
        json!({"body": "Ball", "template": "sphere", "operations": [{"op": "o_grid", "block": 77}]}),
        json!({"body": "Ball", "template": "sphere", "operations": [{"op": "extrude", "faces": [[0, "+w"]], "offset": [0, 0, 1]}]}),
        json!({"body": "Ball", "template": "sphere", "operations": [{"op": "merge", "distance": -1}]}),
        json!({"body": "Ball", "template": "blocks"}),
        json!({"body": "Ball", "template": "blocks", "blocks": {"vertices": [[0, 0, 0]], "blocks": [[0, 0, 0, 0, 0, 0, 0, 0]]}}),
        json!({"body": "Ball", "grid": {"cells": [0, 1, 1]}}),
        json!({"body": "Ball", "grid": {"cells": [2, 2, 2], "carve": [{"q": [0, 1]}]}}),
        json!({"body": "Ball", "template": "sphere", "unknown": 1}),
    ] {
        assert!(s.execute("mesh.hex.project", &bad).is_err(), "{bad}");
        assert_eq!(s.model, model, "{bad}");
    }
    // A preview lists the topology without meshing.
    let r = s.execute("mesh.hex.project", &json!({"body": "Ball", "template": "sphere", "preview": true})).unwrap();
    assert_eq!(r["topology"].as_array().unwrap().len(), 7);
    assert_eq!(s.model, model);
}

fn cup_session() -> (crate::tests::TempDir, Session) {
    session(json!([
        {"command": "solid.cylinder", "params": {"radius": 20, "height": 30, "body_name": "Cup"}},
        {"command": "solid.cylinder", "params": {"base": [0, 0, 4], "radius": 16, "height": 27, "operation": "cut", "targets": ["Cup"]}},
        {"command": "solid.box", "params": {"length": 11, "width": 6, "height": 16, "corner": [19, -3, 8], "operation": "new", "body_name": "Handle"}},
        {"command": "solid.box", "params": {"length": 8, "width": 8, "height": 8, "corner": [18, -4, 12], "operation": "cut", "targets": ["Handle"]}},
        {"command": "solid.combine", "params": {"target": "Cup", "tools": ["Handle"], "operation": "join"}}
    ]))
}

fn cup_params(size: f64, inset: Value) -> Value {
    json!({"body": "Cup", "template": "butterfly", "center": [0, 0, 0], "axis": [0, 0, 1], "radius": 20, "levels": [0, 4, 6, 14, 18, 26, 30], "size": size,
    "operations": [
        {"op": "o_grid", "block": 0, "open": [1, 2], "inset": 0.6},
        {"op": "o_grid", "block": 14, "open": ["+x"], "inset": inset},
        {"op": "o_grid", "block": 22, "open": ["+x"], "inset": inset},
        {"op": "extrude", "faces": [[14, "+x"]], "offset": [12, 0, 0]},
        {"op": "extrude", "faces": [[44, "+x"]], "offset": [4, 0, 0]},
        {"op": "extrude", "faces": [[22, "+x"]], "offset": [12, 0, 0]},
        {"op": "extrude", "faces": [[46, "+x"]], "offset": [4, 0, 0]},
        {"op": "connect", "from": [45, "+z"], "to": [47, "-z"], "layers": 2},
        {"op": "remove", "blocks": [1, 2, 3, 4, 5]}
    ]})
}

/// A cup with a handle: a double O-grid section (the floor's butterfly inside the wall's), the
/// cavity carved, the handle a square bar extruded from O-grids in the wall and closed by a
/// connection. Open: a few elements at the upper handle junction stay folded (#399).
#[test]
#[ignore = "#399: the upper handle junction leaves a few folded elements"]
fn cup_with_a_handle() {
    let (_d, mut s) = cup_session();
    let r = s.execute("mesh.hex.project", &cup_params(2.0, json!([0.5, 0.3, 0.3]))).unwrap();
    assert!(sj(&r) >= 0.3, "{r}");
}

/// The cup without its handle: the floor's O-grid inside the wall's butterfly, cavity carved.
#[test]
fn cup_floor_double_o_grid() {
    let (_d, mut s) = session(json!([
        {"command": "solid.cylinder", "params": {"radius": 20, "height": 30, "body_name": "Cup"}},
        {"command": "solid.cylinder", "params": {"base": [0, 0, 4], "radius": 16, "height": 27, "operation": "cut", "targets": ["Cup"]}}
    ]));
    let mut p = cup_params(2.0, json!(0.5));
    let ops = p["operations"].as_array().unwrap().clone();
    p["operations"] = json!([ops[0], ops[8]]);
    let r = s.execute("mesh.hex.project", &p).unwrap();
    assert!(sj(&r) >= 0.5, "{r}");
    let (v, body) = (r["volume"].as_f64().unwrap(), r["body_volume"].as_f64().unwrap());
    assert!((v - body).abs() < 0.02 * body, "{r}");
}
