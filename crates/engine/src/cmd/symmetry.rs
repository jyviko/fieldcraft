//! `model.boundary_condition.symmetry`: a symmetry condition on a datum plane, so half of a
//! symmetric part can be solved as a half.
//!
//! The condition holds the displacement normal to the plane, and the rotations about the two
//! axes in it, at zero (`type: "symmetry"`). Its set is given, or made of the faces of the open
//! design's bodies that lie on the plane (the cut faces of a partition by that datum, `partition:
//! <name>:a|b`), so it follows the geometry through a remesh.

use fieldcraft_geometry::FaceSurface;
use fieldcraft_model::{BoundaryCondition, GeometryRef, Set, Symmetry, Target};
use serde_json::{Value, json};

use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const SYMMETRY: CommandSpec = CommandSpec {
    id: "model.boundary_condition.symmetry",
    label: "Symmetry Plane",
    params: r#"{"name": "Sym", "datum": "Mid" (a datum plane normal to X, Y or Z), "set"?: "cut" (default: a new set of the same name holding every face of the open design's bodies on the plane, e.g. a partition's cut faces), "bodies"?: ["Part/a"] (only their faces)} — a symmetry boundary condition on the plane: zero displacement normal to it and zero rotation about the axes in it, so half a symmetric part (loaded symmetrically, with half the load) is solved as a half. Add it to the steps (model.step.activate). Planes not normal to X, Y or Z are not supported yet"#,
    form: Some(super::forms::SYMMETRY),
    kind: CommandKind::Model,
    requires: &[Requirement::DatumPlane],
    run: symmetry,
};

/// A face lies on the plane within this fraction of its body's size.
const ON_PLANE: f64 = 1e-6;

fn symmetry(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?.to_string();
    let datum: String = super::req(p, "datum")?;
    let (point, normal) = s
        .model
        .datum(&datum)
        .and_then(|d| d.plane())
        .ok_or_else(|| EngineError::Params(format!("no datum plane `{}`", datum.chars().take(64).collect::<String>())))?;
    let axis = (0..3).find(|&k| (normal[k].abs() - 1.0).abs() < 1e-9).ok_or_else(|| {
        EngineError::Params(format!("not supported yet: symmetry about a plane not normal to X, Y or Z (datum `{datum}` has normal {normal:?})"))
    })?;
    let plane = [Symmetry::X, Symmetry::Y, Symmetry::Z][axis];
    let set = match p.get("set") {
        Some(_) => super::req::<String>(p, "set")?,
        None => {
            let only: Option<Vec<String>> = super::opt(p, "bodies")?;
            let design =
                s.design.as_deref().ok_or_else(|| EngineError::Params("open a design to find the faces on the plane, or give `set`".into()))?;
            let mut faces = Vec::new();
            for b in &design.bodies {
                if only.as_ref().is_some_and(|o| !o.contains(&b.name)) {
                    continue;
                }
                let tol = ON_PLANE * b.size()?.max(1.0);
                for (f, surface) in b.faces().iter().zip(b.face_surfaces()?) {
                    if let Some(FaceSurface::Plane { normal: m, point: q }) = surface {
                        let parallel = (m[axis].abs() - 1.0).abs() < 1e-9;
                        if parallel && (q[axis] - point[axis]).abs() < tol {
                            faces.push(GeometryRef::face(b.name.as_str(), f.name.as_str())?);
                        }
                    }
                }
            }
            if faces.is_empty() {
                return Err(EngineError::Params(format!("no face of the design lies on datum `{datum}`: partition by it first, or give `set`")));
            }
            s.model.add_set(&name, Set::of_geometry(faces))?;
            name.clone()
        }
    };
    s.model.add_boundary_condition(&name, BoundaryCondition::Symmetry { target: Target::Set(set.clone()), plane })?;
    let faces = s.model.set(&set).map_or(0, |x| x.geometry.len());
    let axis_name = ["x", "y", "z"].get(axis).copied().unwrap_or("x");
    Ok(json!({"name": name, "set": set, "plane": axis_name, "faces": faces}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40 × 10 × 10 bar along x (0..40, 0..10, 0..10), the datum `Mid` at y = 5.
    fn bar(half: bool) -> (crate::tests::TempDir, Session) {
        let commands = json!({"commands": [
            {"command": "solid.box", "params": {"length": 40, "width": 10, "height": 10, "body_name": "Bar"}},
        ]});
        let bytes = fieldcraft_geometry::script_design(&commands.to_string()).unwrap();
        let (dir, path) = crate::tests::design_file(&bytes);
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": path})).unwrap();
        s.execute("geometry.datum", &json!({"name": "Mid", "by": {"type": "offset", "plane": "zx", "offset": 5}})).unwrap();
        if half {
            s.execute("geometry.partition", &json!({"name": "Half", "datum": "Mid"})).unwrap();
        }
        (dir, s)
    }

    fn face(s: &Session, body: &str, point: [f64; 3]) -> String {
        let shape = s.design.as_ref().unwrap().body(body).unwrap();
        let f = shape.nearest_faces(&[point], 0.01).unwrap()[0].unwrap();
        shape.faces()[f as usize].name.clone()
    }

    /// Clamp the x = 0 end and push the x = 40 end down (−z) with `force`, solve, and give the
    /// largest downward tip deflection and the clamp's reaction.
    fn bend(s: &mut Session, body: &str, force: f64, extra: &[&str]) -> (f64, f64) {
        let (root, tip) = (face(s, body, [0.0, 2.5, 5.0]), face(s, body, [40.0, 2.5, 5.0]));
        let mut bcs = vec!["Clamp".to_string()];
        bcs.extend(extra.iter().map(|x| x.to_string()));
        let script = json!([
            {"command": "model.set.create", "params": {"name": "all", "geometry": [{"body": body}]}},
            {"command": "model.set.create", "params": {"name": "root", "geometry": [{"body": body, "face": root}]}},
            {"command": "model.set.create", "params": {"name": "tip", "geometry": [{"body": body, "face": tip}]}},
            {"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}}},
            {"command": "model.section.create", "params": {"name": "Bar", "set": "all", "material": "Steel", "kind": {"type": "solid"}}},
            {"command": "model.boundary_condition.create", "params": {"name": "Clamp", "type": "fixed", "target": {"set": "root"}}},
            {"command": "model.load.create", "params": {"name": "Push", "type": "force", "target": {"set": "tip"}, "vector": [0, 0, -force]}},
            {"command": "model.step.create", "params": {"name": "Load", "procedure": {"type": "static"}, "boundary_conditions": bcs, "loads": ["Push"]}},
        ]);
        s.run_script(&script).unwrap();
        s.execute("solve.static", &json!({})).unwrap();
        let tip = s.model.resolve_set("tip").unwrap().nodes;
        let probe = s.execute("solve.probe", &json!({"step": "Load", "nodes": tip})).unwrap();
        let w = probe["nodes"].as_array().unwrap().iter().map(|n| -n["displacement"][2].as_f64().unwrap()).fold(f64::MIN, f64::max);
        let r = s.execute("results.reactions", &json!({"step": "Load", "sets": ["root"]})).unwrap();
        (w, r["sets"][0]["force"][2].as_f64().unwrap_or(f64::NAN))
    }

    /// Half the bar, cut on its symmetry plane y = 5, with the symmetry condition and half the
    /// load, bends as the whole bar does: the same deflection, half the reaction.
    #[test]
    fn a_half_model_with_symmetry_matches_the_whole() {
        let (_d1, mut whole) = bar(false);
        whole.execute("mesh.hex.mapped", &json!({"body": "Bar", "size": 2.5, "order": 2})).unwrap();
        let full = bend(&mut whole, "Bar", 1000.0, &[]);

        let (_d2, mut half) = bar(true);
        half.execute("mesh.hex.mapped", &json!({"body": "Bar/a", "size": 2.5, "order": 2})).unwrap();
        let r = half.execute("model.boundary_condition.symmetry", &json!({"name": "Sym", "datum": "Mid", "bodies": ["Bar/a"]})).unwrap();
        assert_eq!(r, json!({"name": "Sym", "set": "Sym", "plane": "y", "faces": 1}));
        assert_eq!(half.model.set("Sym").unwrap().geometry[0].entity, fieldcraft_model::geometry::Entity::Face("partition:Half:a".into()));
        let halved = bend(&mut half, "Bar/a", 500.0, &["Sym"]);
        assert!((halved.0 - full.0).abs() < 1e-9 * full.0, "deflection {} vs {}", halved.0, full.0);
        assert!((2.0 * halved.1 - full.1).abs() < 1e-9 * full.1.abs(), "reaction {} vs {}", halved.1, full.1);
    }

    #[test]
    fn inclined_planes_and_hostile_params_change_nothing() {
        let (_d, mut s) = bar(true);
        s.execute("geometry.datum", &json!({"name": "Tilt", "by": {"type": "point_normal", "point": [0, 0, 0], "normal": [1, 1, 0]}})).unwrap();
        s.execute("geometry.datum", &json!({"name": "Out", "by": {"type": "offset", "plane": "xy", "offset": 100}})).unwrap();
        s.execute("geometry.datum.axis", &json!({"name": "Ax", "by": {"type": "principal", "axis": "x"}})).unwrap();
        let model = s.model.clone();
        let e = s.execute("model.boundary_condition.symmetry", &json!({"name": "S", "datum": "Tilt"})).unwrap_err();
        assert!(e.to_string().contains("not supported yet"), "{e}");
        for p in [
            json!({"name": "S", "datum": "Out"}),
            json!({"name": "S", "datum": "Ax"}),
            json!({"name": "S", "datum": "Nope"}),
            json!({"name": "S", "datum": "Mid", "set": "nope"}),
            json!({"name": "S", "datum": "Mid", "bodies": ["Nope"]}),
            json!({"name": "1S", "datum": "Mid"}),
            json!({"datum": "Mid"}),
        ] {
            assert!(s.execute("model.boundary_condition.symmetry", &p).is_err(), "{p}");
        }
        assert_eq!(s.model, model);
    }
}
