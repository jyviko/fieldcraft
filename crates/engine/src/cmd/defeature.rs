//! Defeaturing for meshing: `geometry.defeature` takes holes, fillets, chamfers and small bosses
//! out of the open design by suppressing their SolidCraft features (virtually, as a face merge,
//! when a feature cannot be suppressed on its own).
//!
//! A defeature is part of the model (`fieldcraft_model::Defeature`, by feature id); the geometry
//! layer applies it to the design as opened (`fieldcraft_geometry::defeature`), before the
//! partitions, whenever the design is opened or cut again ([`super::datum::partitioned`]), so the
//! meshers, the viewport and the checks all see the part without them.

use std::collections::BTreeSet;

use fieldcraft_model::Defeature;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const DEFEATURE: CommandSpec = CommandSpec {
    id: "geometry.defeature",
    label: "Defeature",
    params: r#"{"name": "Small", "features": [3, "F7"] (feature ids, as geometry.features lists them; or the text "3, F7")} — takes features of the open design out for meshing: the design is rebuilt without them, as SolidCraft builds it with them suppressed (a hole filled, a fillet or chamfer back to its sharp edge, a boss gone), and the other faces keep their persistent names. A feature that cannot be suppressed alone (a later feature needs it, or a body would go) is defeatured virtually instead: its faces are merged into the neighbour they share the longest edges with, for meshing only (virtual topology; the B-rep keeps it). A feature that builds a body or a sketch, or one the design lacks, is an error. Kept in the model by feature id and applied again whenever the design is opened (before the partitions); the same name again replaces its features. A suppressed feature affects every body it shaped. The bodies lose their meshes (`replaced`; remesh them); sets naming their faces are `stale_sets`. Replies `suppressed` (feature ids), `virtual` ([{feature, reason}]) and `bodies`. One undo step; geometry.defeature.delete turns them back on"#,
    form: Some(super::forms::DEFEATURE),
    kind: CommandKind::Model,
    requires: &[Requirement::Design],
    run: defeature,
};

pub const DEFEATURE_DELETE: CommandSpec = CommandSpec {
    id: "geometry.defeature.delete",
    label: "Delete Defeature",
    params: r#"{"name": "Small"} — turns a defeature's features back on: the design is rebuilt with them; the bodies they shaped lose their meshes (`replaced`)"#,
    form: Some(super::forms::DEFEATURE_DELETE),
    kind: CommandKind::Model,
    requires: &[Requirement::Defeature],
    run: delete,
};

/// A feature id: `3` or `"F3"`.
#[derive(Deserialize)]
#[serde(untagged)]
enum FeatureId {
    Id(u64),
    Name(String),
}

/// The feature ids of `features`: a list, or the form's text (`3, F7`).
fn ids(p: &Value) -> Result<Vec<u64>> {
    let given: Vec<FeatureId> = match p.get("features") {
        Some(Value::String(t)) if t.len() <= 64 * 1024 => {
            t.split([',', ' ', ';']).filter(|w| !w.is_empty()).map(|w| FeatureId::Name(w.to_string())).collect()
        }
        _ => super::req(p, "features")?,
    };
    if given.is_empty() || given.len() > fieldcraft_model::virtual_faces::MAX_DEFEATURE_FEATURES {
        return Err(EngineError::invalid("features", format!("1 to {} feature ids", fieldcraft_model::virtual_faces::MAX_DEFEATURE_FEATURES)));
    }
    let mut out = Vec::with_capacity(given.len());
    for f in given {
        let id = match f {
            FeatureId::Id(id) => Some(id),
            FeatureId::Name(n) => n.strip_prefix('F').unwrap_or(&n).parse().ok(),
        };
        match id {
            Some(id) if !out.contains(&id) => out.push(id),
            Some(_) => {}
            None => return Err(EngineError::invalid("features", "feature ids: 3 or \"F3\"")),
        }
    }
    Ok(out)
}

/// The bodies of the open design (as opened) that features `ids` shaped.
fn shaped(s: &Session, ids: &BTreeSet<u64>) -> Vec<String> {
    let Some(design) = s.design.as_deref() else { return Vec::new() };
    let graph = design.undefeatured().features(None).unwrap_or_default();
    let sources: Vec<String> = graph.into_iter().filter(|b| b.features.iter().any(|f| ids.contains(&f.id))).map(|b| b.body).collect();
    // A body the partitions cut: each of its cells.
    design.bodies.iter().map(|b| b.name.clone()).filter(|n| design.source_of(n).is_some_and(|src| sources.iter().any(|x| x == src))).collect()
}

/// What the design's defeatures did with the features of `name`, as the reply lists them.
fn outcome(s: &Session, name: &str) -> (Vec<u64>, Vec<Value>) {
    let report = s.design.as_deref().map(|d| d.defeat_report().to_vec()).unwrap_or_default();
    let mine = report.into_iter().filter(|r| r.defeature == name);
    let (mut suppressed, mut virt) = (Vec::new(), Vec::new());
    for r in mine {
        if r.method == "suppressed" {
            suppressed.push(r.feature);
        } else {
            virt.push(json!({"feature": r.feature, "reason": r.reason}));
        }
    }
    (suppressed, virt)
}

fn defeature(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?.to_string();
    let features = ids(p)?;
    let before: BTreeSet<u64> = s.model.defeatures().iter().filter(|d| d.name == name).flat_map(|d| d.features.clone()).collect();
    s.model.set_defeature(Defeature { name: name.clone(), features: features.clone() })?;
    let mut touched = before;
    touched.extend(&features);
    let bodies = shaped(s, &touched);
    let merges: Vec<String> = features.iter().map(|id| format!("merge {name}-F{id}")).collect();
    let mut errors = vec![format!("defeature {name}")];
    errors.extend(merges);
    let reply = super::virtual_faces::reapply(s, &errors, &bodies, json!({"name": name}))?;
    let (suppressed, virt) = outcome(s, &name);
    let bodies = shaped(s, &touched);
    let mut reply = reply;
    if let Some(o) = reply.as_object_mut() {
        o.insert("suppressed".into(), json!(suppressed));
        o.insert("virtual".into(), json!(virt));
        o.insert("bodies".into(), json!(bodies));
    }
    Ok(reply)
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?;
    let removed = s.model.remove_defeature(name)?;
    let bodies = shaped(s, &removed.features.iter().copied().collect());
    let mut cleared = super::datum::recut(s)?;
    cleared.extend(super::replace::clear(&mut s.model, bodies.iter().map(String::as_str)));
    Ok(cleared.report_if_any(s, json!({"deleted": name, "features": removed.features, "bodies": bodies})))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn faces(s: &Session) -> usize {
        s.design.as_ref().unwrap().bodies[0].faces().len()
    }

    fn hole(s: &mut Session) -> u64 {
        let r = s.execute("geometry.features", &json!({})).unwrap();
        r["bodies"][0]["features"].as_array().unwrap().iter().find(|f| f["operation"] == "cut").unwrap()["id"].as_u64().unwrap()
    }

    /// The sample plate: suppressing its hole gives a block meshed
    /// without it (tet and hex), undo and redo take it out and put it back, it is saved and
    /// replayed on open, and deleting the defeature brings the hole back.
    #[test]
    fn the_plate_hole_is_suppressed_meshed_undone_saved_and_restored() {
        let (mut s, dir) = super::super::datum::tests::plate();
        let body = s.design.as_ref().unwrap().bodies[0].name.clone();
        assert_eq!(faces(&s), 8);
        let id = hole(&mut s);
        s.execute("mesh.tet.generate", &json!({"body": body, "size": 4})).unwrap();
        let with_hole = s.model.elements().len();
        let r = s.execute("geometry.defeature", &json!({"name": "Holes", "features": [id]})).unwrap();
        assert_eq!(r["suppressed"], json!([id]), "{r}");
        assert_eq!(r["replaced"][0]["body"].as_str(), Some(body.as_str()), "{r}");
        assert_eq!(faces(&s), 6);
        // Meshed without it: a tet mesh fills the whole block, and the block sweeps into hexes.
        s.execute("mesh.tet.generate", &json!({"body": body, "size": 4})).unwrap();
        let volume = |s: &Session| s.design.as_ref().unwrap().bodies[0].measure().unwrap().volume;
        assert!((volume(&s) - 40.0 * 30.0 * 10.0).abs() < 1e-6, "{}", volume(&s));
        assert_ne!(s.model.elements().len(), with_hole);
        s.execute("mesh.hex.sweep", &json!({"body": body, "size": 5})).unwrap();
        assert!(s.model.elements().values().all(|e| e.nodes.len() == 8), "a block sweeps into hexes only");
        // Undo the meshes and the defeature: the hole is back; redo takes it out again.
        while !s.model.defeatures().is_empty() {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(faces(&s), 8);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(faces(&s), 6);
        // Saved, and replayed on open of the model and on a re-open of the design.
        let file = dir.join("defeatured.fieldcraft");
        s.execute("file.save", &json!({"path": file.to_str().unwrap()})).unwrap();
        let mut t = Session::default();
        t.execute("file.open", &json!({"path": file.to_str().unwrap()})).unwrap();
        assert_eq!((t.model.defeatures().len(), faces(&t)), (1, 6));
        let design = s.sources.design.as_ref().unwrap().path().to_string();
        s.execute("geometry.open", &json!({"path": design})).unwrap();
        assert_eq!(faces(&s), 6);
        // Switched back on.
        let r = s.execute("geometry.defeature.delete", &json!({"name": "Holes"})).unwrap();
        assert_eq!(r["features"], json!([id]), "{r}");
        assert_eq!(faces(&s), 8);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Unknown features, body-building ones and bad ids are refused and leave the model as it was.
    #[test]
    fn bad_features_are_refused() {
        let (mut s, dir) = super::super::datum::tests::plate();
        for features in [json!([999]), json!(["Fx"]), json!([]), json!("F1, x"), json!(7)] {
            assert!(s.execute("geometry.defeature", &json!({"name": "Bad", "features": features})).is_err(), "{features}");
        }
        let body = s.design.as_ref().unwrap().bodies[0].name.clone();
        let base = s.execute("geometry.features", &json!({"body": body})).unwrap()["bodies"][0]["features"][0]["id"].as_u64().unwrap();
        let e = s.execute("geometry.defeature", &json!({"name": "Bad", "features": [base]})).unwrap_err();
        assert!(e.to_string().contains("builds a body"), "{e}");
        assert!(s.model.defeatures().is_empty() && faces(&s) == 8);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The bracket's 6 mm fillet is defeatured back to a sharp corner and
    /// meshed; its holes stay.
    #[test]
    fn a_bracket_is_meshed_with_its_small_fillet_defeatured() {
        let (_dir, mut s) = super::super::datum::tests::scripted(json!([
            {"command": "sketch.create", "params": {"plane": "XY", "name": "Profile"}},
            {"command": "sketch.line", "params": {"points": [[0, 0], [60, 0], [60, 8], [8, 8], [8, 50], [0, 50]], "closed": true}},
            {"command": "sketch.finish", "params": {}},
            {"command": "solid.extrude", "params": {"distance": 40, "body_name": "Bracket"}},
            {"command": "solid.fillet", "params": {"edges": [[8, 8, 20]], "radius": 6}},
            {"command": "solid.hole", "params": {"position": [40, 8, 20], "diameter": 8}},
            {"command": "solid.hole", "params": {"position": [8, 32, 20], "diameter": 8}}
        ]));
        let g = s.execute("geometry.features", &json!({"body": "Bracket"})).unwrap();
        let fillet: Vec<serde_json::Value> =
            g["bodies"][0]["features"].as_array().unwrap().iter().filter(|f| f["type"] == "fillet").map(|f| f["id"].clone()).collect();
        assert_eq!(fillet.len(), 1);
        let before = faces(&s);
        let r = s.execute("geometry.defeature", &json!({"name": "Fillets", "features": fillet})).unwrap();
        assert_eq!(r["suppressed"], json!(fillet), "{r}");
        assert_eq!(faces(&s), before - 1);
        let r = s.execute("mesh.tet.generate", &json!({"body": "Bracket", "size": 4})).unwrap();
        assert!(r["elements"].as_u64().unwrap_or(0) > 0, "{r}");
    }
}
