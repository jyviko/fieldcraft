use std::sync::Arc;

use fieldcraft_geometry::tagged_mesh;
use fieldcraft_geometry::{MAX_DESIGN_BYTES, Shape, SurfaceMesh, open_design};
use fieldcraft_io::SourceFile;
use fieldcraft_io::roots::Access;
use serde_json::{Value, json};

use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const OPEN: CommandSpec = CommandSpec {
    id: "geometry.open",
    label: "Open Design",
    params: r#"{"path": "part.solidcraft"} — rebuilds the SolidCraft design and makes its bodies the geometry; opening it again after an edit picks up the change"#,
    form: None,
    kind: CommandKind::Model,
    run: open,
};

pub const BODIES: CommandSpec = CommandSpec {
    id: "geometry.bodies",
    label: "Bodies",
    params: "{} — the open design's bodies: measures and persistent face names (by face index)",
    form: None,
    kind: CommandKind::Model,
    run: bodies,
};

pub const TESSELLATE: CommandSpec = CommandSpec {
    id: "geometry.tessellate",
    label: "Tessellate Body",
    params: r#"{"body": "Body1", "tolerance": 0.05} — triangulates the body's boundary within the chordal tolerance (mm); reports triangles per face"#,
    form: None,
    kind: CommandKind::Model,
    run: tessellate,
};

pub const IMPORT_MESH: CommandSpec = CommandSpec {
    id: "geometry.import.mesh",
    label: "Import Tagged Mesh",
    params: r#"{"path": "part.3mf"} — adds each build item of a 3MF package as a surface; triangles keep their B-rep face tags (docs/formats/tagged-mesh.md)"#,
    form: None,
    kind: CommandKind::Model,
    run: import_mesh,
};

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = super::file_param(s, p, "path", Access::Read)?;
    let bytes = super::file::read_file(&path, MAX_DESIGN_BYTES as u64)?;
    let source = SourceFile::of(super::file::absolute(&path), &bytes)?;
    let design = open_design(&bytes)?;
    let out = json!({
        "name": design.name,
        "bodies": design.bodies.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        "errors": design.errors.iter().map(|e| json!({"feature": e.feature, "message": e.message})).collect::<Vec<_>>(),
    });
    s.design = Some(Arc::new(design));
    s.sources.design = Some(source);
    Ok(out)
}

fn bodies(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(design) = &s.design else { return Ok(json!([])) };
    let mut out = Vec::with_capacity(design.bodies.len());
    for b in &design.bodies {
        let m = b.measure()?;
        out.push(json!({
            "name": b.name,
            "volume": m.volume,
            "area": m.area,
            "centroid": m.centroid,
            "bbox": [m.bbox_min, m.bbox_max],
            "faces": b.faces().iter().map(|f| json!({"name": f.name, "exact": f.exact})).collect::<Vec<_>>(),
        }));
    }
    Ok(Value::Array(out))
}

fn body<'a>(s: &'a Session, p: &Value) -> Result<&'a Shape> {
    let name = p.get("body").and_then(Value::as_str).ok_or_else(|| EngineError::Params("`body` must be a body name".into()))?;
    s.design.as_deref().and_then(|d| d.body(name)).ok_or_else(|| EngineError::Params(format!("no body `{name}` (open a design first)")))
}

/// The body named by `body`, tessellated within `tolerance`.
pub(crate) fn body_mesh<'a>(s: &'a Session, p: &Value) -> Result<(&'a Shape, SurfaceMesh)> {
    let shape = body(s, p)?;
    let tolerance = p.get("tolerance").and_then(Value::as_f64).ok_or_else(|| EngineError::Params("`tolerance` must be a number (mm)".into()))?;
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
