//! Virtual topology: faces merged for meshing only (`geometry.faces.merge`).
//!
//! A face merge is part of the model (`fieldcraft_model::virtual_faces`); the geometry layer
//! applies it to the open design's bodies (`fieldcraft_geometry::virtual_faces`), after the
//! partitions, whenever the design is opened or cut again ([`super::datum::partitioned`]), so the
//! meshers, the viewport and the checks all see the merged face. The B-rep is not changed.

use fieldcraft_geometry::virtual_faces::merged_face_name;
use fieldcraft_model::FaceMerge;
use fieldcraft_model::geometry::{Entity, GeometryRef};
use serde_json::{Value, json};

use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const FACES_MERGE: CommandSpec = CommandSpec {
    id: "geometry.faces.merge",
    label: "Merge Faces",
    params: r#"{"name": "Fillet", "faces": [{"body": "Base", "face": "F3:blend#0"}, {"body": "Base", "face": "F3:blend#1"}]} — makes two or more adjacent faces of one body one face for meshing only (virtual topology): the meshers, the checks and the viewport take them as one face `merge:<name>`, without the edges between them (an edge they leave in pieces along a neighbour is one edge), on the surface they share (none when they lie on different surfaces: the meshers keep to the tessellation). The design's B-rep is not changed. A face name also selects the pieces a partition cut it into, and may be a face an earlier merge made. The model keeps the merge and applies it again, by persistent face names, whenever the design is opened (after the partitions); one that no longer applies (a face gone, faces that no longer touch) is listed with the design's errors. The body loses its mesh (`replaced`; remesh it); sets naming the merged faces no longer resolve (`stale_sets`: name `merge:<name>` instead). Faces on several bodies, faces that do not touch along an edge, or a face the body lacks are an error. One undo step"#,
    form: Some(super::forms::FACES_MERGE),
    kind: CommandKind::Model,
    requires: &[Requirement::Design],
    run: merge,
};

pub const FACES_MERGE_DELETE: CommandSpec = CommandSpec {
    id: "geometry.faces.merge.delete",
    label: "Delete Face Merge",
    params: r#"{"name": "Fillet"} — removes a face merge: its faces are separate faces again (a later merge that names `merge:<name>` no longer applies and is listed with the design's errors); the body loses its mesh (`replaced`)"#,
    form: Some(super::forms::FACES_MERGE_DELETE),
    kind: CommandKind::Model,
    requires: &[Requirement::FaceMerge],
    run: delete,
};

/// Most faces one merge lists.
const MAX_FACES: usize = fieldcraft_model::virtual_faces::MAX_FACE_MERGE_FACES;

/// The body and the face names of `faces` (`[{body, face}]`, all on one body).
fn faces_of(p: &Value) -> Result<(String, Vec<String>)> {
    let refs: Vec<GeometryRef> = super::req(p, "faces")?;
    if !(2..=MAX_FACES).contains(&refs.len()) {
        return Err(EngineError::invalid("faces", format!("2 to {MAX_FACES} faces of one body")));
    }
    let body = refs.first().map(|r| r.body.clone()).unwrap_or_default();
    let mut names = Vec::with_capacity(refs.len());
    for r in &refs {
        match &r.entity {
            Entity::Face(f) if r.body == body => names.push(f.clone()),
            Entity::Face(_) => return Err(EngineError::invalid("faces", "faces of one body: a merge makes one face of one body")),
            _ => {
                return Err(EngineError::invalid("faces", format!("`{}`: give {{\"body\", \"face\"}}", r.body.chars().take(80).collect::<String>())));
            }
        }
    }
    Ok((body, names))
}

/// The open design's sets that name a face of `body` it no longer has.
fn stale_sets(s: &Session, body: &str) -> Vec<String> {
    let Some(shape) = s.design.as_deref().and_then(|d| d.body(body)) else { return Vec::new() };
    let gone = |r: &GeometryRef| r.body == body && matches!(&r.entity, Entity::Face(f) if shape.faces_named(f).is_empty());
    s.model.sets().iter().filter(|(_, set)| set.geometry.iter().any(gone)).map(|(n, _)| n.clone()).collect()
}

/// Apply the model's partitions and merges to the design again, failing with `feature`'s error
/// when it has one; the meshes of `body` go (its faces changed).
fn reapply(s: &mut Session, feature: &str, body: &str, reply: Value) -> Result<Value> {
    let mut cleared = super::datum::recut(s)?;
    let design = s.design.as_deref().ok_or_else(|| EngineError::Params("open a design first".into()))?;
    let errors: Vec<&str> = design.errors.iter().filter(|e| e.feature == feature).map(|e| e.message.as_str()).collect();
    if !errors.is_empty() {
        return Err(EngineError::Params(format!("{feature}: {}", errors.join("; "))));
    }
    cleared.extend(super::replace::clear(&mut s.model, [body]));
    let stale = stale_sets(s, body);
    let mut reply = cleared.report_if_any(s, reply);
    if !stale.is_empty()
        && let Some(o) = reply.as_object_mut()
    {
        let mut all: Vec<String> = o.get("stale_sets").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        all.extend(stale);
        all.sort();
        all.dedup();
        o.insert("stale_sets".into(), json!(all));
    }
    Ok(reply)
}

fn merge(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?.to_string();
    let (body, faces) = faces_of(p)?;
    if s.design.as_deref().and_then(|d| d.body(&body)).is_none() {
        return Err(EngineError::Params(format!("the open design has no body `{}`", body.chars().take(200).collect::<String>())));
    }
    s.model.add_face_merge(FaceMerge { name: name.clone(), body: body.clone(), faces: faces.clone() })?;
    let face = merged_face_name(&name);
    let reply = json!({"name": name, "body": body, "face": face});
    let mut reply = reapply(s, &format!("merge {name}"), &body, reply)?;
    let shape = s.design.as_deref().and_then(|d| d.body(&body));
    let brep = shape.and_then(|b| b.face_index(&face)).and_then(|f| shape?.merged_faces().into_iter().find(|(g, _)| *g == f)).map_or(0, |(_, n)| n);
    if let Some(o) = reply.as_object_mut() {
        o.insert("brep_faces".into(), json!(brep));
    }
    Ok(reply)
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = super::name(p)?;
    let removed = s.model.remove_face_merge(name)?;
    let mut cleared = super::datum::recut(s)?;
    cleared.extend(super::replace::clear(&mut s.model, [removed.body.as_str()]));
    Ok(cleared.report_if_any(s, json!({"deleted": name, "body": removed.body})))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use fieldcraft_geometry::FaceSurface;
    use serde_json::json;

    use crate::Session;

    /// The plate's hole halves (two half-cylinders), by name.
    fn halves(s: &Session) -> Vec<String> {
        let b = &s.design.as_ref().unwrap().bodies[0];
        b.faces()
            .iter()
            .zip(b.face_surfaces().unwrap())
            .filter(|(_, x)| matches!(x, Some(FaceSurface::Cylinder { .. })))
            .map(|(f, _)| f.name.clone())
            .collect()
    }

    fn merge_hole(s: &mut Session) -> serde_json::Value {
        let body = s.design.as_ref().unwrap().bodies[0].name.clone();
        let faces: Vec<_> = halves(s).into_iter().map(|f| json!({"body": body, "face": f})).collect();
        s.execute("geometry.faces.merge", &json!({"name": "Hole", "faces": faces})).unwrap()
    }

    fn faces(s: &Session) -> Vec<String> {
        s.design.as_ref().unwrap().bodies[0].faces().iter().map(|f| f.name.clone()).collect()
    }

    /// A merge makes the hole one face of the design; undo and redo take it out and put it back;
    /// it is saved with the model and applied again when the model and its design are opened.
    #[test]
    fn a_merge_is_one_face_undone_saved_and_replayed() {
        let (mut s, dir) = super::super::datum::tests::plate();
        let before = faces(&s);
        assert_eq!(before.len(), 8);
        let r = merge_hole(&mut s);
        assert_eq!((r["face"].as_str(), r["brep_faces"].as_u64()), (Some("merge:Hole"), Some(2)), "{r}");
        assert_eq!(faces(&s).len(), 7);
        assert!(faces(&s).contains(&"merge:Hole".to_string()));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(faces(&s), before);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(faces(&s).len(), 7);
        // Saved, and replayed on open.
        let file = dir.join("merged.fieldcraft");
        s.execute("file.save", &json!({"path": file.to_str().unwrap()})).unwrap();
        let mut t = Session::default();
        t.execute("file.open", &json!({"path": file.to_str().unwrap()})).unwrap();
        assert_eq!(t.model.face_merges().len(), 1);
        assert_eq!(faces(&t), faces(&s));
        // Opening the design again (after an edit of it) replays the merge.
        let design = s.sources.design.as_ref().unwrap().path().to_string();
        s.execute("geometry.open", &json!({"path": design})).unwrap();
        assert_eq!(faces(&s).len(), 7);
        // Deleting it brings the halves back.
        s.execute("geometry.faces.merge.delete", &json!({"name": "Hole"})).unwrap();
        assert_eq!(faces(&s), before);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The mesh nodes on the seams between the plate's hole halves (the B-rep edges between them).
    fn nodes_on_seams(s: &Session, seams: &[Vec<[f64; 3]>]) -> usize {
        let on = |p: [f64; 3]| {
            seams.iter().any(|line| {
                line.windows(2).any(|w| {
                    let (a, b) = (w[0], w[1]);
                    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
                    let l2 = ab.iter().map(|x| x * x).sum::<f64>();
                    let t = (ab.iter().zip(&ap).map(|(x, y)| x * y).sum::<f64>() / l2).clamp(0.0, 1.0);
                    (0..3).map(|k| (ap[k] - t * ab[k]).powi(2)).sum::<f64>().sqrt() < 1e-6
                })
            })
        };
        s.model.nodes().values().filter(|n| on(n.xyz)).count()
    }

    /// A mesher takes the merged face as one: its boundary element faces are tagged with it, the
    /// halves are gone from the tags, and the seams between the halves (no longer edges) carry at
    /// most `left` nodes.
    fn mesher_meshes_a_merged_face_as_one(command: &str, params: serde_json::Value, left: impl Fn(usize) -> usize) {
        let (mut s, dir) = super::super::datum::tests::plate();
        let hole = halves(&s);
        let b = &s.design.as_ref().unwrap().bodies[0];
        let seams: Vec<Vec<[f64; 3]>> = b
            .edges()
            .unwrap()
            .into_iter()
            .filter(|e| e.faces.len() == 2 && e.faces[0] != e.faces[1] && e.faces.iter().all(|&f| hole.contains(&b.faces()[f as usize].name)))
            .map(|e| e.points)
            .collect();
        assert_eq!(seams.len(), 2);
        s.execute(command, &params).unwrap();
        let before = nodes_on_seams(&s, &seams);
        assert!(before >= 6, "{command}: a node row on each seam before the merge: {before}");
        let r = merge_hole(&mut s);
        assert!(r["replaced"].as_array().is_some_and(|a| !a.is_empty()), "the mesh goes: {r}");
        assert!(s.model.elements().is_empty());
        s.execute(command, &params).unwrap();
        // The face, edge and vertex names the elements and nodes are tagged with.
        let c = s.model.classification();
        let used = c.elements().values().flat_map(|t| t.faces.values().chain(t.edges.values())).chain(c.nodes().values());
        let tagged: BTreeSet<String> = used.filter_map(|&t| c.entity(t)?.entity.name().map(str::to_string)).collect();
        assert!(tagged.contains("merge:Hole"), "{tagged:?}");
        assert!(!tagged.iter().any(|n| hole.iter().any(|h| n.split(['|', '~']).any(|p| p == h))), "{tagged:?}");
        let after = nodes_on_seams(&s, &seams);
        assert!(after <= left(before), "{command}: nodes on the seams after the merge: {after} (before {before})");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_tet_mesher_meshes_a_merged_face_as_one() {
        // Before: the surface mesh keeps the seams as feature lines (21 nodes); after: none.
        mesher_meshes_a_merged_face_as_one("mesh.tet.generate", json!({"size": 4}), |_| 0);
    }

    #[test]
    fn the_hex_mesher_meshes_a_merged_face_as_one() {
        // The swept O-grid round the hole had a node column on each seam; after, the rim is one
        // closed edge and only the column where its ring starts is left.
        mesher_meshes_a_merged_face_as_one("mesh.hex.generate", json!({"size": 2.5}), |before| before / 2);
    }

    /// The hex-meshing plan sees the merged hole as one face: the hole's feature and its ring
    /// project onto `merge:Hole` (still a cylinder), not the halves.
    #[test]
    fn the_plan_sees_a_merged_face_as_one() {
        let (mut s, dir) = super::super::datum::tests::plate();
        let body = s.design.as_ref().unwrap().bodies[0].name.clone();
        let hole = halves(&s);
        let before = s.execute("mesh.plan", &json!({"body": body})).unwrap();
        let ring = |r: &serde_json::Value| r["rings"][0]["faces"].clone();
        let mut halves_sorted = hole.clone();
        halves_sorted.sort();
        let mut got: Vec<String> = serde_json::from_value(ring(&before)).unwrap();
        got.sort();
        assert_eq!(got, halves_sorted, "{before}");
        merge_hole(&mut s);
        let after = s.execute("mesh.plan", &json!({"body": body})).unwrap();
        assert_eq!(ring(&after), json!(["merge:Hole"]), "{after}");
        let features = after["features"].as_array().unwrap();
        assert!(features.iter().any(|f| f["faces"] == json!(["merge:Hole"])), "{after}");
        assert!(!after.to_string().contains(&hole[0]), "{after}");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The viewport draws a merged face in one colour: the hole's triangles, and only those.
    #[test]
    fn a_merged_face_is_drawn_in_one_colour() {
        let (mut s, dir) = super::super::datum::tests::plate();
        let p = crate::view::Palette::LIGHT;
        let merged = |s: &Session| crate::view::scene(s, &p).0.tris.into_iter().filter(|t| t.color == p.merged).collect::<Vec<_>>();
        assert!(merged(&s).is_empty());
        merge_hole(&mut s);
        let tris = merged(&s);
        assert!(!tris.is_empty());
        // The hole: Ø10 about (15, 15).
        for t in &tris {
            assert!(t.p.iter().all(|v| ((v.x - 15.0).hypot(v.y - 15.0) - 5.0).abs() < 1e-3), "{t:?}");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// Sets naming a merged face go stale (and are named); bad merges change nothing.
    #[test]
    fn bad_merges_change_nothing() {
        let (mut s, dir) = super::super::datum::tests::plate();
        let body = s.design.as_ref().unwrap().bodies[0].name.clone();
        let names = faces(&s);
        let hole = halves(&s);
        s.execute("model.set.create", &json!({"name": "Bore", "geometry": [{"body": body, "face": hole[0]}]})).unwrap();
        let journal = s.journal.len();
        let f = |n: &str| json!({"body": body, "face": n});
        for p in [
            json!({}),
            json!({"name": "M"}),
            json!({"name": "M", "faces": [f(&hole[0])]}),
            json!({"name": "M", "faces": [f(&hole[0]), {"body": "Other", "face": hole[1]}]}),
            json!({"name": "M", "faces": [{"body": "Other", "face": "a"}, {"body": "Other", "face": "b"}]}),
            json!({"name": "M", "faces": [f(&hole[0]), f("nope")]}),
            json!({"name": "M", "faces": [f(&hole[0]), {"body": body}]}),
            json!({"name": "M", "faces": [f(&hole[0]), f(&hole[0])]}),
            json!({"name": "1M", "faces": [f(&hole[0]), f(&hole[1])]}),
            json!({"name": "M", "faces": "x"}),
        ] {
            assert!(s.execute("geometry.faces.merge", &p).is_err(), "{p}");
        }
        // Top and bottom do not touch.
        let b = &s.design.as_ref().unwrap().bodies[0];
        let caps: Vec<String> = b
            .faces()
            .iter()
            .zip(b.face_surfaces().unwrap())
            .filter(|(_, x)| matches!(x, Some(FaceSurface::Plane { normal, .. }) if normal[2].abs() > 0.5))
            .map(|(f, _)| f.name.clone())
            .collect();
        let e = s.execute("geometry.faces.merge", &json!({"name": "Caps", "faces": [f(&caps[0]), f(&caps[1])]})).unwrap_err();
        assert!(e.to_string().contains("does not touch"), "{e}");
        assert!(s.execute("geometry.faces.merge.delete", &json!({"name": "Caps"})).is_err());
        assert_eq!((s.journal.len(), faces(&s)), (journal, names));
        assert!(s.model.face_merges().is_empty());
        let r = merge_hole(&mut s);
        assert_eq!(r["stale_sets"], json!(["Bore"]), "{r}");
        std::fs::remove_dir_all(dir).ok();
    }
}
