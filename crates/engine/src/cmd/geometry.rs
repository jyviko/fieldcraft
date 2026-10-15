use std::collections::BTreeSet;
use std::sync::Arc;

use fieldcraft_geometry::tagged_mesh;
use fieldcraft_geometry::{Design, MAX_DESIGN_BYTES, Shape, SurfaceMesh, open_design};
use fieldcraft_io::SourceFile;
use fieldcraft_io::roots::Access;
use fieldcraft_model::Model;
use fieldcraft_model::geometry::{Entity, GeometryRef};
use serde_json::{Value, json};

use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const OPEN: CommandSpec = CommandSpec {
    id: "geometry.open",
    label: "Open Design",
    params: r#"{"path": "part.solidcraft", "model"?: "keep"} — rebuilds the SolidCraft design and makes its bodies the geometry. `model`: "keep" (default) keeps the mesh and every definition, so opening the same design again after an edit picks up the change and sets follow their persistent face names; "new" starts a new model with the design (no mesh, sets, materials, sections, BCs, loads or steps; the unit system stays; the model is untitled until saved, and undo restores the previous file). The reply's `related` is false when the model (before the open) refers to bodies (mesh tags, set geometry) and the design has none of them, and `stale` lists what no longer resolves: `mesh_bodies` (bodies the mesh is tagged with that the design lacks) `sets` (sets naming a body or face it lacks) and `seeds` (edge seeds on edges it lacks). One undo step"#,
    form: Some(super::forms::OPEN_DESIGN),
    kind: CommandKind::Model,
    requires: &[],
    run: open,
};

pub const BODIES: CommandSpec = CommandSpec {
    id: "geometry.bodies",
    label: "Bodies",
    params: "{} — the open design's bodies: measures, persistent face names (by face index) and edges (persistent name `<face>|<face>`, `~k` when several separate the same faces, and length)",
    form: Some(&[]),
    kind: CommandKind::Model,
    requires: &[], // No design is an answer too: no bodies.
    run: bodies,
};

pub const TESSELLATE: CommandSpec = CommandSpec {
    id: "geometry.tessellate",
    label: "Tessellate Body",
    params: r#"{"body": "Body1", "tolerance": 0.05} — triangulates the body's boundary within the chordal tolerance (mm); reports triangles per face"#,
    form: Some(super::forms::TESSELLATE),
    kind: CommandKind::Model,
    requires: &[Requirement::Design],
    run: tessellate,
};

pub const IMPORT_MESH: CommandSpec = CommandSpec {
    id: "geometry.import.mesh",
    label: "Import Tagged Mesh",
    params: r#"{"path": "part.3mf"} — adds each build item of a 3MF package as a surface; triangles keep their B-rep face tags (docs/formats/tagged-mesh.md)"#,
    form: Some(super::forms::IMPORT_MESH),
    kind: CommandKind::Model,
    requires: &[],
    run: import_mesh,
};

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let new_model = match p.get("model") {
        None | Some(Value::Null) => false,
        Some(v) => match v.as_str() {
            Some("keep") => false,
            Some("new") => true,
            _ => return Err(EngineError::Params(r#"`model` must be "keep" or "new""#.into())),
        },
    };
    let path = super::file_param(s, p, "path", Access::Read)?;
    let bytes = super::file::read_file(&path, MAX_DESIGN_BYTES as u64)?;
    let source = SourceFile::of(super::file::absolute(&path), &bytes)?;
    let opened = open_design(&bytes)?;
    // The model's partitions cut the design again (a new model has none).
    let design = if new_model { opened } else { super::datum::partitioned(&opened, &s.model)? };
    let mut fit = fit(s, &design);
    if new_model {
        let mut model = Model::default();
        model.set_units(s.model.units());
        s.model = model;
        s.surfaces.clear();
        s.sources.meshes.clear();
        s.results_view = None;
        // A new model is untitled: saving asks for a file instead of overwriting the old model's
        // (undo brings the old file back with the old model).
        s.set_document(Default::default());
        fit.mesh_bodies.clear();
        fit.sets.clear();
        fit.seeds.clear();
    }
    let out = json!({
        "name": design.name,
        "bodies": design.bodies.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        "errors": design.errors.iter().map(|e| json!({"feature": e.feature, "message": e.message})).collect::<Vec<_>>(),
        "model": if new_model { "new" } else { "kept" },
        "related": fit.related,
        "stale": {"mesh_bodies": fit.mesh_bodies, "sets": fit.sets, "seeds": fit.seeds},
    });
    // Another design file (or a new model) is another document; the same file opened again after
    // an edit is the same one.
    if new_model || s.sources.design.as_ref().is_none_or(|d| d.path() != source.path()) {
        s.mark_opened();
    }
    s.design = Some(Arc::new(design));
    s.sources.design = Some(source);
    Ok(out)
}

/// How the model's geometry references fit a design about to become the geometry.
struct Fit {
    /// Whether the design (or an imported surface) has a body the model refers to, or the model
    /// refers to none: `false` means the model is of another part.
    related: bool,
    /// Bodies the mesh is tagged with that neither the design nor a surface has.
    mesh_bodies: Vec<String>,
    /// Sets naming a body or a face that is not there.
    sets: Vec<String>,
    /// Edge seeds on an edge the design lacks.
    seeds: Vec<String>,
}

fn fit(s: &Session, design: &Design) -> Fit {
    let has = |body: &str| design.body(body).is_some() || s.surfaces.iter().any(|t| t.name == body);
    let resolves = |r: &GeometryRef| match (&r.entity, design.body(&r.body)) {
        (Entity::Face(face), Some(b)) => b.faces().iter().any(|f| &f.name == face),
        _ => has(&r.body),
    };
    let tagged: BTreeSet<&str> = s.model.classification().entities().iter().map(|r| r.body.as_str()).collect();
    let mut referenced = tagged.clone();
    referenced.extend(s.model.sets().values().flat_map(|set| set.geometry.iter().map(|r| r.body.as_str())));
    Fit {
        related: referenced.is_empty() || referenced.iter().any(|b| has(b)),
        mesh_bodies: tagged.into_iter().filter(|b| !has(b)).map(str::to_string).collect(),
        sets: s.model.sets().iter().filter(|(_, set)| !set.geometry.iter().all(resolves)).map(|(n, _)| n.clone()).collect(),
        seeds: s
            .model
            .seeds()
            .iter()
            .filter(|seed| super::seeds::seeded_edges(design, &seed.edge).map_or(true, |e| e.is_empty()))
            .map(|seed| seed.edge.to_string())
            .collect(),
    }
}

fn bodies(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(design) = &s.design else { return Ok(json!([])) };
    let mut out = Vec::with_capacity(design.bodies.len());
    for b in &design.bodies {
        let m = b.measure()?;
        let edges: Vec<Value> = b.edges()?.iter().map(|e| json!({"name": e.name, "length": e.length()})).collect();
        out.push(json!({
            "name": b.name,
            "volume": m.volume,
            "area": m.area,
            "centroid": m.centroid,
            "bbox": [m.bbox_min, m.bbox_max],
            "faces": b.faces().iter().map(|f| json!({"name": f.name, "exact": f.exact})).collect::<Vec<_>>(),
            "edges": edges,
        }));
    }
    Ok(Value::Array(out))
}

fn body<'a>(s: &'a Session, p: &Value) -> Result<&'a Shape> {
    let name = p.get("body").and_then(Value::as_str).ok_or_else(|| EngineError::param("body", "must be a body name"))?;
    s.design.as_deref().and_then(|d| d.body(name)).ok_or_else(|| EngineError::Params(format!("no body `{name}` (open a design first)")))
}

/// The body named by `body`, tessellated within `tolerance`.
pub(crate) fn body_mesh<'a>(s: &'a Session, p: &Value) -> Result<(&'a Shape, SurfaceMesh)> {
    let shape = body(s, p)?;
    let tolerance = p.get("tolerance").and_then(Value::as_f64).ok_or_else(|| EngineError::param("tolerance", "must be a number (mm)"))?;
    Ok((shape, shape.tessellate(tolerance)?))
}

fn tessellate(s: &mut Session, p: &Value) -> Result<Value> {
    let (shape, mesh) = body_mesh(s, p)?;
    let mut per_face = vec![0usize; shape.faces().len()];
    for &f in &mesh.tri_face {
        if let Some(n) = per_face.get_mut(f as usize) {
            *n += 1;
        }
    }
    let faces: Vec<Value> = shape.faces().iter().zip(per_face).map(|(f, n)| json!({"name": f.name, "triangles": n})).collect();
    Ok(json!({"body": shape.name, "vertices": mesh.positions.len(), "triangles": mesh.triangles.len(), "faces": faces}))
}

fn import_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    let path = super::file_param(s, p, "path", Access::Read)?;
    if s.sources.meshes.len() >= fieldcraft_io::MAX_MESH_SOURCES {
        return Err(EngineError::Params(format!("a model imports at most {} meshes", fieldcraft_io::MAX_MESH_SOURCES)));
    }
    let bytes = super::file::read_file(&path, tagged_mesh::MAX_FILE_BYTES as u64)?;
    let source = SourceFile::of(super::file::absolute(&path), &bytes)?;
    let surfaces = tagged_mesh::read_3mf(&bytes)?;
    let first = s.surfaces.len();
    let added: Vec<Value> = surfaces
        .iter()
        .enumerate()
        .map(|(k, t)| json!({"index": first + k, "name": t.name, "vertices": t.mesh.positions.len(), "triangles": t.mesh.triangles.len(), "faces": t.faces}))
        .collect();
    s.surfaces.extend(surfaces.into_iter().map(Arc::new));
    s.sources.meshes.push(source);
    Ok(json!({"path": path, "surfaces": added}))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn package(model: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("3D/3dmodel.model", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(model.as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn import_adds_tagged_surfaces_and_failures_change_nothing() {
        let dir = std::env::temp_dir().join(format!("fieldcraft-import-mesh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = format!(
            r#"<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:ft="{}"><resources>
<object id="1" name="Tet" ft:faces="2"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices>
<triangles><triangle v1="0" v2="2" v3="1" ft:face="0"/><triangle v1="0" v2="1" v3="3" ft:face="1"/><triangle v1="1" v2="2" v3="3" ft:face="1"/><triangle v1="0" v2="3" v3="2" ft:face="1"/></triangles></mesh></object>
</resources><build><item objectid="1"/></build></model>"#,
            tagged_mesh::FACE_TAGS_NS
        );
        let good = dir.join("tet.3mf");
        std::fs::write(&good, package(&model)).unwrap();
        let junk = dir.join("junk.3mf");
        std::fs::write(&junk, b"PK\x03\x04 not really").unwrap();

        let mut s = Session::default();
        let r = s.execute("geometry.import.mesh", &json!({"path": good.to_str().unwrap()})).unwrap();
        assert_eq!(r["surfaces"][0], json!({"index": 0, "name": "Tet", "vertices": 4, "triangles": 4, "faces": 2}));
        assert_eq!(s.surfaces[0].mesh.tri_face, vec![0, 1, 1, 1]);
        assert_eq!(s.journal.len(), 1);

        let hostile = [
            json!({"path": junk.to_str().unwrap()}),
            json!({"path": dir.join("missing.3mf").to_str().unwrap()}),
            json!({"path": dir.to_str().unwrap()}),
            json!({"path": "/dev/zero"}),
            json!({"path": ""}),
            json!({"path": "x".repeat(5000)}),
        ];
        for p in hostile {
            assert!(s.execute("geometry.import.mesh", &p).is_err(), "{p}");
        }
        assert_eq!(s.surfaces.len(), 1);
        assert_eq!(s.journal.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
