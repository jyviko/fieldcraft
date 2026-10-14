//! Files: open and save `.fieldcraft` models, and recover autosaved ones.
//!
//! A saved model refers to its design and imported meshes by path and content hash; opening it
//! opens them again. Paths inside the model file's folder are stored relative to it, so a folder
//! of a model and its design can be moved together. A geometry file that is missing, unreadable
//! or changed since the save does not stop the model from opening: it is reported in the result's
//! `warnings` (sets follow geometry names, so a remesh brings a changed design's mesh up to date).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use fieldcraft_geometry::tagged_mesh;
use fieldcraft_geometry::{Design, MAX_DESIGN_BYTES, open_design};
use fieldcraft_io::{Geometry, ModelFile, SourceFile};
use serde_json::{Value, json};

use fieldcraft_io::roots::{Access, Roots};

use super::{file_param, req};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session, next_revision};

pub const OPEN: CommandSpec = CommandSpec {
    id: "file.open",
    label: "Open Model",
    params: r#"{"path": "bracket.fieldcraft"} — opens the model, its mesh and its journal, and opens its design and imported meshes again from their files (one that is missing or changed is listed in `warnings`; the model still opens); unsaved changes of the current model are discarded and the undo history starts empty"#,
    form: Some(super::forms::FILE_OPEN),
    kind: CommandKind::Session,
    requires: &[],
    run: open,
};

pub const SAVE: CommandSpec = CommandSpec {
    id: "file.save",
    label: "Save Model",
    params: r#"{"path"?: "bracket.fieldcraft"} — saves to `path`, else to the file the model was opened from or last saved to; written atomically, the previous version kept as `<file>.bak`"#,
    form: Some(super::forms::FILE_SAVE),
    kind: CommandKind::Session,
    requires: &[],
    run: save,
};

pub const SAVE_AS: CommandSpec = CommandSpec {
    id: "file.save_as",
    label: "Save Model As",
    params: r#"{"path": "bracket.fieldcraft"} — saves to a new file, which becomes the model's file"#,
    form: Some(super::forms::FILE_SAVE_AS),
    kind: CommandKind::Session,
    requires: &[],
    run: save_as,
};

pub const RECOVERY_LIST: CommandSpec = CommandSpec {
    id: "file.recovery.list",
    label: "Recoverable Models",
    params: r#"{"dir"?: "…"} — models autosaved by apps that crashed or closed with unsaved changes, newest first (default folder: FIELDCRAFT_RECOVERY_DIR or the per-user data folder)"#,
    form: Some(super::forms::RECOVERY_LIST),
    kind: CommandKind::Session,
    requires: &[],
    run: recovery_list,
};

pub const RECOVER: CommandSpec = CommandSpec {
    id: "file.recover",
    label: "Recover Model",
    params: r#"{"id": "…", "dir"?: "…"} — opens an autosaved model (an id from file.recovery.list) with its original file path, as unsaved changes; the autosave stays until file.recovery.discard"#,
    form: Some(super::forms::RECOVER),
    kind: CommandKind::Session,
    requires: &[],
    run: recover,
};

pub const RECOVERY_DISCARD: CommandSpec = CommandSpec {
    id: "file.recovery.discard",
    label: "Discard Recovered Model",
    params: r#"{"id": "…"} or {"all": true}; "dir"?: "…" — deletes autosaves of apps that are no longer running"#,
    form: Some(super::forms::RECOVERY_DISCARD),
    kind: CommandKind::Session,
    requires: &[],
    run: recovery_discard,
};

pub const SAMPLE: CommandSpec = CommandSpec {
    id: "file.sample",
    label: "Open Sample",
    params: r#"{"name": "cantilever"} — opens a sample model (fieldcraft_engine::sample; the list is in docs/examples.md) in place of the current one, unsaved changes discarded; every sample but `parts` is ready to solve (solve.run)"#,
    form: Some(super::forms::SAMPLE),
    kind: CommandKind::Session,
    requires: &[],
    run: open_sample,
};

fn open_sample(s: &mut Session, p: &Value) -> Result<Value> {
    let name: &str = req(p, "name")?;
    let names = || crate::sample::SAMPLES.iter().map(|x| x.name).collect::<Vec<_>>().join(", ");
    let sample = crate::sample::find(name).ok_or_else(|| EngineError::Params(format!("no sample `{name}` (samples: {})", names())))?;
    let mut opened = Session::default();
    if let Some(recipe) = sample.design {
        opened.design = Some(Arc::new(open_design(&fieldcraft_geometry::script_design(recipe)?)?));
    }
    let bodies = opened.design.as_ref().map_or(&[][..], |d| d.bodies.as_slice());
    let script = (sample.script)(bodies);
    opened.run_script(&script)?;
    // A sample just opened has nothing to save: it opens again the same way.
    opened.saved_revision = Some(opened.revision);
    s.replace_with(opened);
    let mut reply = summary(s, &[]);
    reply["sample"] = json!(sample.name);
    Ok(reply)
}

/// A file of at most `max` bytes, through FieldCraft's file edge ([`fieldcraft_io::fs`]: the disk,
/// or the page's files in the browser).
pub(crate) fn read_file(path: &str, max: u64) -> Result<Vec<u8>> {
    Ok(fieldcraft_io::fs::read_capped(Path::new(path), max)?)
}

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
/// its path resolved, when it could not be). A file outside the read root of `roots` is not
/// opened.
fn reopen<T>(
    what: &str,
    stored: &SourceFile,
    base: Option<&Path>,
    roots: &Roots,
    max: u64,
    open: impl FnOnce(&[u8]) -> fieldcraft_geometry::Result<T>,
    warnings: &mut Vec<String>,
) -> (Option<T>, SourceFile) {
    let path = resolved_path(stored.path(), base);
    let kept = stored.with_path(path.clone()).unwrap_or_else(|_| stored.clone());
    let checked = match roots.check(Path::new(&path), Access::Read) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => {
            warnings.push(format!("{what} `{path}` was not opened ({e}); the model opens without it"));
            return (None, kept);
        }
    };
    let bytes = match read_file(&checked, max) {
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
    /// The bytes of the `.fieldcraft` file holding this session, with geometry paths inside
    /// `base` (the folder the file goes to) stored relative to it.
    pub fn file_bytes(&self, base: Option<&Path>) -> Result<Vec<u8>> {
        let stored = |s: &SourceFile| s.with_path(stored_path(s.path(), base));
        let geometry = Geometry {
            design: self.sources.design.as_ref().map(stored).transpose()?,
            meshes: self.sources.meshes.iter().map(stored).collect::<fieldcraft_io::Result<_>>()?,
        };
        Ok(fieldcraft_io::write(&self.model, &geometry, &self.journal)?)
    }

    /// A session holding what a file holds, its geometry opened again from the paths it names
    /// (relative ones against `base`) where `roots` lets it read them; problems with the
    /// geometry come back as warnings. The session's own file access is unconfined.
    pub fn from_file(file: ModelFile, base: Option<&Path>, roots: &Roots) -> (Session, Vec<String>) {
        let mut warnings = Vec::new();
        let mut sources = Geometry::default();
        let mut design: Option<Arc<Design>> = None;
        if let Some(stored) = &file.geometry.design {
            let (opened, source) = reopen("design", stored, base, roots, MAX_DESIGN_BYTES as u64, open_design, &mut warnings);
            design = opened.map(Arc::new);
            sources.design = Some(source);
        }
        let mut surfaces = Vec::new();
        for stored in &file.geometry.meshes {
            let (opened, source) = reopen("mesh", stored, base, roots, tagged_mesh::MAX_FILE_BYTES as u64, tagged_mesh::read_3mf, &mut warnings);
            surfaces.extend(opened.into_iter().flatten().map(Arc::new));
            sources.meshes.push(source);
        }
        let session =
            Session { model: file.model, design, surfaces, sources, journal: file.journal, revision: next_revision(), ..Session::default() };
        (session, warnings)
    }

    /// Replace this session with `opened`, keeping its undo memory limit, its work caps and its
    /// file access; the history starts empty.
    fn replace_with(&mut self, mut opened: Session) {
        opened.history = std::mem::take(&mut self.history);
        opened.limits = self.limits;
        opened.file_access = std::mem::take(&mut self.file_access);
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

/// Replace the session with a `.fieldcraft` file's content; `path` is its own file (geometry
/// paths are relative to its folder), `saved` whether that file holds this content.
fn load(s: &mut Session, bytes: &[u8], path: Option<String>, saved: bool) -> Result<Value> {
    let file = fieldcraft_io::read(bytes)?;
    let (mut opened, warnings) = Session::from_file(file, path.as_deref().and_then(|p| Path::new(p).parent()), &s.file_access);
    opened.path = path;
    if saved {
        opened.saved_revision = Some(opened.revision);
    }
    s.replace_with(opened);
    Ok(summary(s, &warnings))
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = absolute(&file_param(s, p, "path", Access::Read)?);
    let bytes = read_file(&path, fieldcraft_io::MAX_FILE_BYTES)?;
    load(s, &bytes, Some(path.clone()), true).map_err(|e| EngineError::Params(format!("{path}: {e}")))
}

/// Save to `path` (as given by the caller).
fn save_to(s: &mut Session, path: &str) -> Result<Value> {
    let target = absolute(path);
    if std::fs::metadata(&target).is_ok_and(|m| !m.is_file()) {
        return Err(EngineError::Params(format!("`{target}` exists and is not a regular file")));
    }
    let bytes = s.file_bytes(Path::new(&target).parent())?;
    fieldcraft_io::fs::write_atomic(Path::new(&target), &bytes, true)?;
    s.mark_saved(target.clone());
    Ok(json!({"path": target, "bytes": bytes.len()}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = match p.get("path") {
        Some(_) => file_param(s, p, "path", Access::Write)?,
        None => {
            let own = s.path.as_deref().ok_or_else(|| EngineError::Params("the model has no file yet: give `path` (or use file.save_as)".into()))?;
            s.checked_path(own, Access::Write)?
        }
    };
    save_to(s, &path)
}

fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = file_param(s, p, "path", Access::Write)?;
    save_to(s, &path)
}

/// The recovery folder: `dir` (checked for `access`), else the default one (the app's own folder,
/// which no client names, so it is not confined).
fn recovery_dir(s: &Session, p: &Value, access: Access) -> Result<PathBuf> {
    match p.get("dir") {
        Some(_) => Ok(PathBuf::from(file_param(s, p, "dir", access)?)),
        None => {
            crate::autosave::default_dir().ok_or_else(|| EngineError::Params("no recovery folder: set FIELDCRAFT_RECOVERY_DIR or give `dir`".into()))
        }
    }
}

fn recovery_list(s: &mut Session, p: &Value) -> Result<Value> {
    let entries = fieldcraft_io::recovery::orphans(&recovery_dir(s, p, Access::Read)?);
    serde_json::to_value(entries).map_err(|e| EngineError::Params(e.to_string()))
}

fn recover(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = recovery_dir(s, p, Access::Read)?;
    let (bytes, entry) = fieldcraft_io::recovery::load(&dir, req::<&str>(p, "id")?)?;
    // Recovered work is unsaved until it is saved. Autosaves keep geometry paths absolute.
    load(s, &bytes, entry.path, false)
}

fn recovery_discard(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = recovery_dir(s, p, Access::Write)?;
    if p.get("all").and_then(Value::as_bool) == Some(true) {
        let ids: Vec<String> = fieldcraft_io::recovery::orphans(&dir).into_iter().map(|e| e.id).collect();
        for id in &ids {
            fieldcraft_io::recovery::remove(&dir, id)?;
        }
        return Ok(json!({"discarded": ids}));
    }
    let id = req::<&str>(p, "id")?;
    fieldcraft_io::recovery::remove(&dir, id)?;
    Ok(json!({"discarded": [id]}))
}

#[cfg(test)]
mod tests {
    use fieldcraft_geometry::sample::{Plate, plate_design};
    use serde_json::json;

    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fieldcraft-file-{tag}-{}", std::process::id()));
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
        let path = d.join("bracket.fieldcraft");
        s.execute("file.save_as", &json!({"path": path})).unwrap();
        assert!(!s.is_dirty());
        let bytes = std::fs::read(&path).unwrap();
        let file = fieldcraft_io::read(&bytes).unwrap();
        assert_eq!(file.geometry.design.as_ref().unwrap().path(), "plate.solidcraft", "stored next to the model, relative");

        // Saving again keeps the previous version and gives the same bytes.
        s.execute("file.save", &json!({})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(fieldcraft_io::fs::backup_path(&path)).unwrap(), bytes);

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
    fn a_confined_session_reads_and_writes_only_inside_its_roots() {
        let d = dir("roots");
        let (input, output, outside) = (d.join("in"), d.join("out"), d.join("outside"));
        for f in [&input, &output, &outside] {
            std::fs::create_dir_all(f).unwrap();
        }
        let design = plate_design(&Plate::default()).unwrap();
        std::fs::write(input.join("plate.solidcraft"), &design).unwrap();
        std::fs::write(outside.join("plate.solidcraft"), &design).unwrap();
        let mut s = Session { file_access: Roots::new(Some(&input), Some(&output)).unwrap(), ..Session::default() };
        let refused = |r: Result<Value>| {
            let e = r.unwrap_err().to_string();
            assert!(e.starts_with("automation"), "{e}");
        };
        refused(s.execute("geometry.open", &json!({"path": outside.join("plate.solidcraft")})));
        refused(s.execute("geometry.import.mesh", &json!({"path": outside.join("x.3mf")})));
        refused(s.execute("file.open", &json!({"path": outside.join("x.fieldcraft")})));
        refused(s.execute("file.recovery.list", &json!({"dir": outside})));
        refused(s.execute("file.recovery.discard", &json!({"all": true, "dir": input})));
        s.execute("geometry.open", &json!({"path": input.join("plate.solidcraft")})).unwrap();
        defined(&mut s);
        // A section, so deck.write has all it needs and reaches its path.
        s.execute("model.set.create", &json!({"name": "all", "elements": [1]})).unwrap();
        s.execute("model.section.create", &json!({"name": "solid", "set": "all", "material": "Steel", "kind": {"type": "solid"}})).unwrap();
        // Writes go to the write root only: not the read root, not elsewhere.
        refused(s.execute("file.save_as", &json!({"path": input.join("m.fieldcraft")})));
        refused(s.execute("file.save_as", &json!({"path": outside.join("m.fieldcraft")})));
        refused(s.execute("file.save_as", &json!({"path": output.join("../outside/m.fieldcraft")})));
        refused(s.execute("deck.write", &json!({"format": "abaqus", "path": outside.join("m.inp")})));
        let saved = output.join("m.fieldcraft");
        s.execute("file.save_as", &json!({"path": saved})).unwrap();
        assert!(saved.is_file());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1, "nothing written outside");

        // The roots survive opening a file; a model whose design lies outside the read root
        // opens without it.
        let mut other = Session::default();
        other.execute("geometry.open", &json!({"path": outside.join("plate.solidcraft")})).unwrap();
        other.execute("file.save_as", &json!({"path": input.join("other.fieldcraft")})).unwrap();
        let r = s.execute("file.open", &json!({"path": input.join("other.fieldcraft")})).unwrap();
        assert!(r["warnings"][0].as_str().unwrap().contains("automation path rejected"), "{r}");
        assert!(s.design.is_none());
        assert!(s.file_access.is_confined());
        refused(s.execute("file.save", &json!({})));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hostile_file_params_and_files_change_nothing() {
        let d = dir("hostile");
        let junk = d.join("junk.fieldcraft");
        std::fs::write(&junk, b"PK\x03\x04 not a model").unwrap();
        let mut s = Session::default();
        defined(&mut s);
        let before = (s.model.clone(), s.journal.clone(), s.revision());
        for (id, p) in [
            ("file.open", json!({"path": junk})),
            ("file.open", json!({"path": d})),
            ("file.open", json!({"path": d.join("missing.fieldcraft")})),
            ("file.open", json!({"path": "\u{0}"})),
            ("file.open", json!({"path": 5})),
            ("file.save", json!({"path": ""})),
            ("file.save", json!({"path": d})),
            ("file.save_as", json!({"path": d.join("no/such/dir/a.fieldcraft")})),
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
