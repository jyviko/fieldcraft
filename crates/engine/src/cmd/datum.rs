//! Datum planes and axes, and partitions of the open design by datum planes.
//!
//! Datums and partitions are part of the model (`fieldcraft_model::datums`). A partition cuts the
//! design in memory (`fieldcraft_geometry::partition`); the model's partitions are applied again,
//! in order, whenever the design is opened (`geometry.open`, `file.open`, samples), so an edit
//! of the design followed by a re-open replays them.

use std::sync::Arc;

use fieldcraft_geometry::{Cut, Design};
use fieldcraft_model::{Datum, Model, Partition};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const DATUM_PLANE: CommandSpec = CommandSpec {
    id: "geometry.datum",
    label: "Datum Plane",
    params: r#"{"name": "Mid", "by": {"type": "offset", "plane": "yz" | "zx" | "xy", "offset": 15} | {"type": "three_points", "a": [x, y, z], "b": […], "c": […]} | {"type": "point_normal", "point": [x, y, z], "normal": [nx, ny, nz]}} — a named datum plane (model coordinates, mm for SolidCraft geometry): a principal plane moved along its normal (yz: normal +x, zx: normal +y, xy: normal +z), through three points (normal (b − a) × (c − a)), or through a point with a normal. Partitions (geometry.partition) cut by it; shown in the viewport"#,
    form: Some(super::forms::DATUM_PLANE),
    kind: CommandKind::Model,
    requires: &[],
    run: datum_plane,
};

pub const DATUM_AXIS: CommandSpec = CommandSpec {
    id: "geometry.datum.axis",
    label: "Datum Axis",
    params: r#"{"name": "Spindle", "by": {"type": "principal", "axis": "x" | "y" | "z", "through"?: [x, y, z] (default the origin)} | {"type": "two_points", "a": [x, y, z], "b": […]} | {"type": "point_direction", "point": [x, y, z], "direction": [dx, dy, dz]}} — a named datum axis, shown in the viewport"#,
    form: Some(super::forms::DATUM_AXIS),
    kind: CommandKind::Model,
    requires: &[],
    run: datum_axis,
};

pub const DATUM_DELETE: CommandSpec = CommandSpec {
    id: "geometry.datum.delete",
    label: "Delete Datum",
    params: r#"{"name": "Mid"} — deletes a datum plane or axis no partition uses"#,
    form: Some(super::forms::DATUM_DELETE),
    kind: CommandKind::Model,
    requires: &[Requirement::Datum],
    run: datum_delete,
};

pub const PARTITION: CommandSpec = CommandSpec {
    id: "geometry.partition",
    label: "Partition",
    params: r#"{"name": "Half", "datum": "Mid", "bodies"?: ["Body1"] (default: every body the plane divides)} — cuts bodies of the open design into cells by a datum plane, in memory (the design file is not changed); the model keeps the partition and applies it again whenever the design is opened, after the partitions before it. A cut body `B` becomes the cells `B/a` (on the plane's negative side) and `B/b` (several pieces on a side: `B/a1`, `B/a2`, …). The two faces of the cut are paired by name: `partition:<name>:a` on the cells on the negative side, `partition:<name>:b` on the positive side (`…#0`, `…#1` where a cell has several); every other face keeps the persistent name of the face it is a piece of (`<face>#k` for several pieces on one cell), so sets by face name still select it. Reports the cells made and the cut faces named; a plane that divides no body, or a listed body it does not divide, is an error"#,
    form: Some(super::forms::PARTITION),
    kind: CommandKind::Model,
    requires: &[Requirement::Design, Requirement::DatumPlane],
    run: partition,
};

pub const PARTITION_DELETE: CommandSpec = CommandSpec {
    id: "geometry.partition.delete",
    label: "Delete Partition",
    params: r#"{"name": "Half"} — removes a partition; the design is cut again by the ones left"#,
    form: Some(super::forms::PARTITION_DELETE),
    kind: CommandKind::Model,
    requires: &[Requirement::Partition],
    run: partition_delete,
};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Principal {
    Yz,
    Zx,
    Xy,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PlaneBy {
    Offset {
        plane: Principal,
        #[serde(default)]
        offset: f64,
    },
    ThreePoints {
        a: [f64; 3],
        b: [f64; 3],
        c: [f64; 3],
    },
    PointNormal {
        point: [f64; 3],
        normal: [f64; 3],
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Axis {
    X,
    Y,
    Z,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum AxisBy {
    Principal {
        axis: Axis,
        #[serde(default)]
        through: [f64; 3],
    },
    TwoPoints {
        a: [f64; 3],
        b: [f64; 3],
    },
    PointDirection {
        point: [f64; 3],
        direction: [f64; 3],
    },
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn plane_datum(by: PlaneBy) -> Result<Datum> {
    Ok(match by {
        PlaneBy::Offset { plane, offset } => {
            if !offset.is_finite() {
                return Err(EngineError::param("offset", "must be a finite number"));
            }
            let normal = match plane {
                Principal::Yz => [1.0, 0.0, 0.0],
                Principal::Zx => [0.0, 1.0, 0.0],
                Principal::Xy => [0.0, 0.0, 1.0],
            };
            Datum::Plane { point: normal.map(|n| n * offset), normal }
        }
        PlaneBy::ThreePoints { a, b, c } => {
            let (u, v) = (sub(b, a), sub(c, a));
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            // Collinear points: refused relative to the points' spread.
            let size = u.iter().chain(&v).map(|x| x * x).sum::<f64>();
            if !(n.iter().map(|x| x * x).sum::<f64>() > 1e-20 * size * size) {
                return Err(EngineError::Params("the three points must not lie on one line".into()));
            }
            Datum::Plane { point: a, normal: n }
        }
        PlaneBy::PointNormal { point, normal } => Datum::Plane { point, normal },
    })
}

fn axis_datum(by: AxisBy) -> Datum {
    match by {
        AxisBy::Principal { axis, through } => {
            let direction = match axis {
                Axis::X => [1.0, 0.0, 0.0],
                Axis::Y => [0.0, 1.0, 0.0],
                Axis::Z => [0.0, 0.0, 1.0],
            };
            Datum::Axis { point: through, direction }
        }
        AxisBy::TwoPoints { a, b } => Datum::Axis { point: a, direction: sub(b, a) },
        AxisBy::PointDirection { point, direction } => Datum::Axis { point, direction },
    }
}

fn datum_json(name: &str, d: &Datum) -> Value {
    let mut v = serde_json::to_value(d).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("name".into(), json!(name));
    }
    v
}

fn datum_plane(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?;
    let datum = plane_datum(super::req(p, "by")?)?;
    s.model.add_datum(name, datum)?;
    Ok(datum_json(name, s.model.datum(name).unwrap_or(&datum)))
}

fn datum_axis(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?;
    let datum = axis_datum(super::req(p, "by")?);
    s.model.add_datum(name, datum)?;
    Ok(datum_json(name, s.model.datum(name).unwrap_or(&datum)))
}

fn datum_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?;
    s.model.remove_datum(name)?;
    Ok(json!({"deleted": name}))
}

/// The model's partitions as cuts of the geometry.
fn cuts(model: &Model) -> Vec<Cut> {
    model
        .partitions()
        .iter()
        .filter_map(|p| {
            let (point, normal) = model.datum(&p.datum)?.plane()?;
            Some(Cut { name: p.name.clone(), point, normal, bodies: p.bodies.clone() })
        })
        .collect()
}

/// `design` cut by `model`'s partitions (the design as opened when the model has none).
pub(crate) fn partitioned(design: &Design, model: &Model) -> Result<Design> {
    Ok(design.partitioned(&cuts(model))?)
}

/// Cut the session's design by its model's partitions again.
fn recut(s: &mut Session) -> Result<()> {
    if let Some(d) = &s.design {
        let cut = partitioned(d, &s.model)?;
        s.design = Some(Arc::new(cut));
    }
    Ok(())
}

fn partition(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?.to_string();
    let datum: String = super::req(p, "datum")?;
    let bodies: Vec<String> = super::opt(p, "bodies")?.unwrap_or_default();
    s.model.add_partition(Partition { name: name.clone(), datum, bodies })?;
    recut(s)?;
    let design = s.design.as_deref().ok_or_else(|| EngineError::Params("open a design first".into()))?;
    let feature = format!("partition {name}");
    let errors: Vec<&str> = design.errors.iter().filter(|e| e.feature == feature).map(|e| e.message.as_str()).collect();
    if !errors.is_empty() {
        return Err(EngineError::Params(format!("partition `{name}`: {}", errors.join("; "))));
    }
    let report = design.cuts.iter().find(|c| c.name == name);
    let split = report.map(|r| r.split.clone()).unwrap_or_default();
    if split.is_empty() {
        return Err(EngineError::Params(format!("partition `{name}`: the plane divides no body of the design")));
    }
    let cells: Vec<&str> = design.bodies.iter().map(|b| b.name.as_str()).filter(|b| split.iter().any(|x| b.starts_with(&format!("{x}/")))).collect();
    Ok(json!({
        "name": name,
        "split": split,
        "cells": cells,
        "cut_faces": report.map_or(0, |r| r.faces),
        "bodies": design.bodies.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
    }))
}

fn partition_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?;
    s.model.remove_partition(name)?;
    recut(s)?;
    let bodies: Vec<String> = s.design.iter().flat_map(|d| d.bodies.iter().map(|b| b.name.clone())).collect();
    Ok(json!({"deleted": name, "bodies": bodies}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session with the sample plate (40 × 30 × 10, Ø10 hole at (15, 15)) open.
    pub(crate) fn plate() -> (Session, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("fieldcraft-partition-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plate.solidcraft");
        let plate = fieldcraft_geometry::sample::Plate::default();
        std::fs::write(&path, fieldcraft_geometry::sample::plate_design(&plate).unwrap()).unwrap();
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": path.to_str().unwrap()})).unwrap();
        (s, dir)
    }

    fn bodies(s: &Session) -> Vec<String> {
        s.design.as_ref().unwrap().bodies.iter().map(|b| b.name.clone()).collect()
    }

    #[test]
    fn datums_by_each_definition() {
        let mut s = Session::default();
        let r = s.execute("geometry.datum", &json!({"name": "Mid", "by": {"type": "offset", "plane": "yz", "offset": 15}})).unwrap();
        assert_eq!(r, json!({"name": "Mid", "type": "plane", "point": [15.0, 0.0, 0.0], "normal": [1.0, 0.0, 0.0]}));
        let r = s
            .execute("geometry.datum", &json!({"name": "Tri", "by": {"type": "three_points", "a": [0, 0, 1], "b": [1, 0, 1], "c": [0, 1, 1]}}))
            .unwrap();
        assert_eq!(r["normal"], json!([0.0, 0.0, 1.0]));
        s.execute("geometry.datum", &json!({"name": "Tilt", "by": {"type": "point_normal", "point": [0, 0, 0], "normal": [0, 3, 4]}})).unwrap();
        assert_eq!(s.model.datum("Tilt"), Some(&Datum::Plane { point: [0.0; 3], normal: [0.0, 0.6, 0.8] }));
        let r = s.execute("geometry.datum.axis", &json!({"name": "Z", "by": {"type": "principal", "axis": "z", "through": [1, 2, 0]}})).unwrap();
        assert_eq!(r, json!({"name": "Z", "type": "axis", "point": [1.0, 2.0, 0.0], "direction": [0.0, 0.0, 1.0]}));
        s.execute("geometry.datum.axis", &json!({"name": "D", "by": {"type": "two_points", "a": [0, 0, 0], "b": [0, 2, 0]}})).unwrap();
        s.execute("geometry.datum.axis", &json!({"name": "E", "by": {"type": "point_direction", "point": [0, 0, 0], "direction": [5, 0, 0]}}))
            .unwrap();
        assert_eq!(s.model.datums().len(), 6);
        s.execute("geometry.datum.delete", &json!({"name": "D"})).unwrap();
        assert_eq!(s.model.datums().len(), 5);
    }

    #[test]
    fn hostile_datum_params_change_nothing() {
        let mut s = Session::default();
        for p in [
            json!({}),
            json!({"name": "A"}),
            json!({"name": "A", "by": {"type": "offset", "plane": "ab"}}),
            json!({"name": "A", "by": {"type": "offset", "plane": "xy", "offset": "x"}}),
            json!({"name": "A", "by": {"type": "three_points", "a": [0, 0, 0], "b": [1, 1, 1], "c": [2, 2, 2]}}),
            json!({"name": "A", "by": {"type": "point_normal", "point": [0, 0, 0], "normal": [0, 0, 0]}}),
            json!({"name": "A", "by": {"type": "point_normal", "point": [1e300, 0, 0], "normal": [0, 0, 1]}}),
            json!({"name": "A", "by": {"type": "point_normal", "point": [0, 0, 0], "normal": [0, 0, 1], "extra": 1}}),
            json!({"name": "1A", "by": {"type": "offset", "plane": "xy"}}),
            json!({"name": "A", "by": {"type": "nope"}}),
        ] {
            assert!(s.execute("geometry.datum", &p).is_err(), "{p}");
        }
        for p in [
            json!({"name": "A", "by": {"type": "two_points", "a": [1, 1, 1], "b": [1, 1, 1]}}),
            json!({"name": "A", "by": {"type": "principal", "axis": "w"}}),
        ] {
            assert!(s.execute("geometry.datum.axis", &p).is_err(), "{p}");
        }
        assert!(s.model.datums().is_empty());
        assert!(s.journal.is_empty());
    }

    #[test]
    fn a_partition_cuts_the_design_and_is_replayed_when_it_opens_again() {
        let (mut s, dir) = plate();
        s.execute("geometry.datum", &json!({"name": "Mid", "by": {"type": "offset", "plane": "yz", "offset": 15}})).unwrap();
        let r = s.execute("geometry.partition", &json!({"name": "Half", "datum": "Mid"})).unwrap();
        let body = r["split"][0].as_str().unwrap().to_string();
        assert_eq!(r["cells"], json!([format!("{body}/a"), format!("{body}/b")]));
        assert_eq!(r["cut_faces"], json!(4));
        // The datum is in use.
        assert!(s.execute("geometry.datum.delete", &json!({"name": "Mid"})).is_err());

        // A set on a cut face selects the cut face's pieces.
        let r = s.execute("geometry.bodies", &json!({})).unwrap();
        let faces: Vec<&str> = r[0]["faces"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert!(faces.contains(&"partition:Half:a#0"), "{faces:?}");

        // Saved and opened again: the design is cut again by the model's partition.
        let file = dir.join("plate.fieldcraft");
        s.execute("file.save_as", &json!({"path": file.to_str().unwrap()})).unwrap();
        let mut t = Session::default();
        t.execute("file.open", &json!({"path": file.to_str().unwrap()})).unwrap();
        assert_eq!(bodies(&t), bodies(&s));
        // geometry.open again keeps the model, so its partition.
        let design = dir.join("plate.solidcraft");
        let r = t.execute("geometry.open", &json!({"path": design.to_str().unwrap()})).unwrap();
        assert_eq!(r["bodies"], json!(bodies(&s)));
        // A new model drops the partitions with the rest of the model.
        let r = t.execute("geometry.open", &json!({"path": design.to_str().unwrap(), "model": "new"})).unwrap();
        assert_eq!(r["bodies"], json!([body]));

        // Undo brings the whole design back; deleting the partition does too.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(bodies(&s), std::slice::from_ref(&body));
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(bodies(&s).len(), 2);
        s.execute("geometry.partition.delete", &json!({"name": "Half"})).unwrap();
        assert_eq!(bodies(&s), [body]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_partition_that_cuts_nothing_changes_nothing() {
        let (mut s, dir) = plate();
        s.execute("geometry.datum", &json!({"name": "Out", "by": {"type": "offset", "plane": "yz", "offset": 100}})).unwrap();
        s.execute("geometry.datum.axis", &json!({"name": "Ax", "by": {"type": "principal", "axis": "x"}})).unwrap();
        let journal = s.journal.len();
        for p in [
            json!({"name": "P", "datum": "Out"}),
            json!({"name": "P", "datum": "Ax"}),
            json!({"name": "P", "datum": "Nope"}),
            json!({"name": "P", "datum": "Out", "bodies": ["Nope"]}),
            json!({"name": "P", "datum": "Out", "bodies": "x"}),
            json!({"name": "", "datum": "Out"}),
        ] {
            assert!(s.execute("geometry.partition", &p).is_err(), "{p}");
        }
        assert!(s.model.partitions().is_empty());
        assert_eq!(bodies(&s).len(), 1);
        assert_eq!(s.journal.len(), journal);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
