//! Files: open and save `.modelcraft` models, and recover autosaved ones.
//!
//! A saved model refers to its design and imported meshes by path and content hash; opening it
//! opens them again. Paths inside the model file's folder are stored relative to it, so a folder
//! of a model and its design can be moved together. A geometry file that is missing, unreadable
//! or changed since the save does not stop the model from opening: it is reported in the result's
//! `warnings` (sets follow geometry names, so a remesh brings a changed design's mesh up to date).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use modelcraft_geometry::tagged_mesh;
use modelcraft_geometry::{Design, MAX_DESIGN_BYTES, open_design};
use modelcraft_io::{Geometry, ModelFile, SourceFile};
use serde_json::{Value, json};

use super::{path_param, req};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session, next_revision};

pub const OPEN: CommandSpec = CommandSpec {
    id: "file.open",
    label: "Open Model",
    params: r#"{"path": "bracket.modelcraft"} — opens the model, its mesh and its journal, and opens its design and imported meshes again from their files (one that is missing or changed is listed in `warnings`; the model still opens); unsaved changes of the current model are discarded and the undo history starts empty"#,
    kind: CommandKind::Session,
    run: open,
};

pub const SAVE: CommandSpec = CommandSpec {
    id: "file.save",
    label: "Save Model",
    params: r#"{"path"?: "bracket.modelcraft"} — saves to `path`, else to the file the model was opened from or last saved to; written atomically, the previous version kept as `<file>.bak`"#,
    kind: CommandKind::Session,
    run: save,
};

pub const SAVE_AS: CommandSpec = CommandSpec {
    id: "file.save_as",
    label: "Save Model As",
    params: r#"{"path": "bracket.modelcraft"} — saves to a new file, which becomes the model's file"#,
    kind: CommandKind::Session,
    run: save_as,
};

pub const RECOVERY_LIST: CommandSpec = CommandSpec {
    id: "file.recovery.list",
    label: "Recoverable Models",
    params: r#"{"dir"?: "…"} — models autosaved by apps that crashed or closed with unsaved changes, newest first (default folder: MODELCRAFT_RECOVERY_DIR or the per-user data folder)"#,
    kind: CommandKind::Session,
    run: recovery_list,
};

pub const RECOVER: CommandSpec = CommandSpec {
    id: "file.recover",
    label: "Recover Model",
    params: r#"{"id": "…", "dir"?: "…"} — opens an autosaved model (an id from file.recovery.list) with its original file path, as unsaved changes; the autosave stays until file.recovery.discard"#,
    kind: CommandKind::Session,
    run: recover,
};

pub const RECOVERY_DISCARD: CommandSpec = CommandSpec {
    id: "file.recovery.discard",
    label: "Discard Recovered Model",
    params: r#"{"id": "…"} or {"all": true}; "dir"?: "…" — deletes autosaves of apps that are no longer running"#,
    kind: CommandKind::Session,
    run: recovery_discard,
};

/// `path` made absolute against the working folder (as given where there is none).
pub(crate) fn absolute(path: &str) -> String {
    std::path::absolute(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string())
}

/// How a geometry path is stored in a file saved in `base`: relative (with `/`) when inside it.
fn stored_path(path: &str, base: Option<&Path>) -> String {
    let relative = base.and_then(|b| Path::new(path).strip_prefix(b).ok()).and_then(|rel| {
        let parts: Option<Vec<&str>> = rel.components().map(|c| if let Component::Normal(s) = c { s.to_str() } else { None }).collect();
        parts.filter(|p| !p.is_empty()).map(|p| p.join("/"))
    });
    relative.unwrap_or_else(|| path.to_string())
}

/// A stored geometry path resolved against the folder of the file that holds it.
fn resolved_path(path: &str, base: Option<&Path>) -> String {
    match base {
        Some(b) if !Path::new(path).is_absolute() => b.join(path).to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

/// Open one geometry file again: what it gives, and the source as now read (or as stored, with
/// its path resolved, when it could not be).
fn reopen<T>(
    what: &str,
    stored: &SourceFile,
    base: Option<&Path>,
    max: usize,
    open: impl FnOnce(&[u8]) -> modelcraft_geometry::Result<T>,
    warnings: &mut Vec<String>,
) -> (Option<T>, SourceFile) {
    let path = resolved_path(stored.path(), base);
    let kept = stored.with_path(path.clone()).unwrap_or_else(|_| stored.clone());
    let bytes = match modelcraft_geometry::read_file(&path, max) {
        Ok(b) => b,
        Err(e) => {
            warnings.push(format!("{what} `{path}` was not opened ({e}); the model opens without it"));
            return (None, kept);
        }
    };
    let Ok(read) = SourceFile::of(path.clone(), &bytes) else { return (None, kept) };
    if read.sha256() != stored.sha256() {
        warnings
            .push(format!("{what} `{path}` changed since the model was saved; sets follow geometry names, so remeshing brings the mesh up to date"));
    }
    match open(&bytes) {
        Ok(v) => (Some(v), read),
        Err(e) => {
            warnings.push(format!("{what} `{path}` could not be opened ({e}); the model opens without it"));
            (None, kept)
        }
    }
}

impl Session {
    /// The bytes of the `.modelcraft` file holding this session, with geometry paths inside
    /// `base` (the folder the file goes to) stored relative to it.
    pub fn file_bytes(&self, base: Option<&Path>) -> Result<Vec<u8>> {
        let stored = |s: &SourceFile| s.with_path(stored_path(s.path(), base));
        let geometry = Geometry {
            design: self.sources.design.as_ref().map(stored).transpose()?,
            meshes: self.sources.meshes.iter().map(stored).collect::<modelcraft_io::Result<_>>()?,
        };
        Ok(modelcraft_io::write(&self.model, &geometry, &self.journal)?)
    }

    /// A session holding what a file holds, its geometry opened again from the paths it names
    /// (relative ones against `base`); problems with the geometry come back as warnings.
    pub fn from_file(file: ModelFile, base: Option<&Path>) -> (Session, Vec<String>) {
        let mut warnings = Vec::new();
        let mut sources = Geometry::default();
        let mut design: Option<Arc<Design>> = None;
        if let Some(stored) = &file.geometry.design {
            let (opened, source) = reopen("design", stored, base, MAX_DESIGN_BYTES, open_design, &mut warnings);
            design = opened.map(Arc::new);
            sources.design = Some(source);
        }
        let mut surfaces = Vec::new();
        for stored in &file.geometry.meshes {
            let (opened, source) = reopen("mesh", stored, base, tagged_mesh::MAX_FILE_BYTES, tagged_mesh::read_3mf, &mut warnings);
            surfaces.extend(opened.into_iter().flatten().map(Arc::new));
            sources.meshes.push(source);
        }
        let session =
            Session { model: file.model, design, surfaces, sources, journal: file.journal, revision: next_revision(), ..Session::default() };
        (session, warnings)
    }

    /// Replace this session with `opened`, keeping its undo memory limit; the history starts empty.
    fn replace_with(&mut self, mut opened: Session) {
        opened.history = std::mem::take(&mut self.history);
        opened.clear_history();
        *self = opened;
    }
}

fn summary(s: &Session, warnings: &[String]) -> Value {
    json!({
        "path": s.path,
        "nodes": s.model.nodes().len(),
        "elements": s.model.elements().len(),
        "journal": s.journal.len(),
        "design": s.design.as_ref().map(|d| d.name.clone()),
        "surfaces": s.surfaces.len(),
        "warnings": warnings,
    })
}

/// Replace the session with a `.modelcraft` file's content; `path` is its own file (geometry
/// paths are relative to its folder), `saved` whether that file holds this content.
fn load(s: &mut Session, bytes: &[u8], path: Option<String>, saved: bool) -> Result<Value> {
    let file = modelcraft_io::read(bytes)?;
    let (mut opened, warnings) = Session::from_file(file, path.as_deref().and_then(|p| Path::new(p).parent()));
    opened.path = path;
    if saved {
        opened.saved_revision = Some(opened.revision);
    }
    s.replace_with(opened);
    Ok(summary(s, &warnings))
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = absolute(path_param(p, "path")?);
    let max = usize::try_from(modelcraft_io::MAX_FILE_BYTES).unwrap_or(usize::MAX);
    let bytes = modelcraft_geometry::read_file(&path, max)?;
    load(s, &bytes, Some(path.clone()), true).map_err(|e| EngineError::Params(format!("{path}: {e}")))
}

/// Save to `path` (as given by the caller).
fn save_to(s: &mut Session, path: &str) -> Result<Value> {
    let target = absolute(path);
    if std::fs::metadata(&target).is_ok_and(|m| !m.is_file()) {
        return Err(EngineError::Params(format!("`{target}` exists and is not a regular file")));
    }
    let bytes = s.file_bytes(Path::new(&target).parent())?;
    modelcraft_io::fs::write_atomic(Path::new(&target), &bytes, true)?;
    s.mark_saved(target.clone());
    Ok(json!({"path": target, "bytes": bytes.len()}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = match p.get("path") {
        Some(_) => path_param(p, "path")?.to_string(),
        None => s.path.clone().ok_or_else(|| EngineError::Params("the model has no file yet: give `path` (or use file.save_as)".into()))?,
    };
    save_to(s, &path)
}

fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_param(p, "path")?.to_string();
    save_to(s, &path)
}

/// The recovery folder: `dir`, else the default one.
fn recovery_dir(p: &Value) -> Result<PathBuf> {
    match p.get("dir") {
        Some(_) => Ok(PathBuf::from(path_param(p, "dir")?)),
        None => {
            crate::autosave::default_dir().ok_or_else(|| EngineError::Params("no recovery folder: set MODELCRAFT_RECOVERY_DIR or give `dir`".into()))
        }
    }
}

fn recovery_list(_: &mut Session, p: &Value) -> Result<Value> {
    let entries = modelcraft_io::recovery::orphans(&recovery_dir(p)?);
    serde_json::to_value(entries).map_err(|e| EngineError::Params(e.to_string()))
}

fn recover(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = recovery_dir(p)?;
    let (bytes, entry) = modelcraft_io::recovery::load(&dir, req::<&str>(p, "id")?)?;
    // Recovered work is unsaved until it is saved. Autosaves keep geometry paths absolute.
    load(s, &bytes, entry.path, false)
}

fn recovery_discard(_: &mut Session, p: &Value) -> Result<Value> {
    let dir = recovery_dir(p)?;
    if p.get("all").and_then(Value::as_bool) == Some(true) {
        let ids: Vec<String> = modelcraft_io::recovery::orphans(&dir).into_iter().map(|e| e.id).collect();
        for id in &ids {
            modelcraft_io::recovery::remove(&dir, id)?;
        }
        return Ok(json!({"discarded": ids}));
    }
    let id = req::<&str>(p, "id")?;
    modelcraft_io::recovery::remove(&dir, id)?;
    Ok(json!({"discarded": [id]}))
}

#[cfg(test)]
mod tests {
    use modelcraft_geometry::sample::{Plate, plate_design};
    use serde_json::json;

    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modelcraft-file-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn defined(s: &mut Session) {
        for (i, c) in [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]].iter().enumerate() {
            s.execute("mesh.node.add", &json!({"label": i + 1, "xyz": c})).unwrap();
        }
        s.execute("mesh.element.add", &json!({"label": 1, "kind": "tet4", "nodes": [1, 2, 3, 4]})).unwrap();
        s.execute("model.material.create", &json!({"name": "Steel", "elastic": {"type": "isotropic", "young": 210000.0, "poisson": 0.3}})).unwrap();
    }

    #[test]
    fn save_and_open_keep_the_model_journal_and_design() {
        let d = dir("roundtrip");
        let design = d.join("plate.solidcraft");
        std::fs::write(&design, plate_design(&Plate::default()).unwrap()).unwrap();
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": design})).unwrap();
        defined(&mut s);
        assert!(s.execute("file.save", &json!({})).is_err(), "no file yet, and no dialog");
        let path = d.join("bracket.modelcraft");
        s.execute("file.save_as", &json!({"path": path})).unwrap();
        assert!(!s.is_dirty());
        let bytes = std::fs::read(&path).unwrap();
        let file = modelcraft_io::read(&bytes).unwrap();
        assert_eq!(file.geometry.design.as_ref().unwrap().path(), "plate.solidcraft", "stored next to the model, relative");

        // Saving again keeps the previous version and gives the same bytes.
        s.execute("file.save", &json!({})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(modelcraft_io::fs::backup_path(&path)).unwrap(), bytes);

        let mut t = Session::default();
        t.execute("mesh.node.add", &json!({"label": 9, "xyz": [0, 0, 0]})).unwrap();
        let r = t.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(r["warnings"], json!([]));
        assert_eq!(r["design"], json!(s.design.as_ref().unwrap().name));
        assert_eq!(t.model, s.model);
        assert_eq!(t.journal, s.journal);
        assert_eq!(t.undo_depth(), (0, 0), "a new model's history starts empty");
        assert!(!t.is_dirty());
        assert!(t.design.is_some());

        // The design changed since: the model opens, with a warning.
        std::fs::write(&design, plate_design(&Plate { width: 60.0, ..Plate::default() }).unwrap()).unwrap();
        let r = t.execute("file.open", &json!({"path": path})).unwrap();
        assert!(r["warnings"][0].as_str().unwrap().contains("changed since"), "{r}");
        assert!(t.design.is_some());

        // The design is gone: the model opens without it and keeps the reference.
        std::fs::remove_file(&design).unwrap();
        let r = t.execute("file.open", &json!({"path": path})).unwrap();
        assert!(r["warnings"][0].as_str().unwrap().contains("not opened"), "{r}");
        assert!(t.design.is_none());
        assert_eq!(t.model, s.model);
        assert_eq!(t.sources.design.as_ref().unwrap().path(), design.to_string_lossy());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hostile_file_params_and_files_change_nothing() {
        let d = dir("hostile");
        let junk = d.join("junk.modelcraft");
        std::fs::write(&junk, b"PK\x03\x04 not a model").unwrap();
        let mut s = Session::default();
        defined(&mut s);
        let before = (s.model.clone(), s.journal.clone(), s.revision());
        for (id, p) in [
            ("file.open", json!({"path": junk})),
            ("file.open", json!({"path": d})),
            ("file.open", json!({"path": d.join("missing.modelcraft")})),
            ("file.open", json!({"path": "\u{0}"})),
            ("file.open", json!({"path": 5})),
            ("file.save", json!({"path": ""})),
            ("file.save", json!({"path": d})),
            ("file.save_as", json!({"path": d.join("no/such/dir/a.modelcraft")})),
            ("file.save_as", json!({"path": "x".repeat(5000)})),
            ("file.save_as", json!({})),
            ("file.recover", json!({"id": "../../etc/passwd", "dir": d})),
            ("file.recover", json!({"dir": d})),
            ("file.recovery.discard", json!({"dir": d})),
            ("file.recovery.list", json!({"dir": 7})),
        ] {
            assert!(s.execute(id, &p).is_err(), "{id} {p}");
            assert_eq!((s.model.clone(), s.journal.clone(), s.revision()), before, "{id} {p}");
        }
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "nothing written");
        let _ = std::fs::remove_dir_all(&d);
    }
}
