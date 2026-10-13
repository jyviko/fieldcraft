//! The command engine.
//!
//! Everything a user can do is a command ([`CommandSpec`]) with a JSON form. The desktop UI,
//! scripts, the CLI and the control channel all run the same commands through
//! [`Session::execute`], which never lets a failure or a panic damage the session: the model and
//! the geometry are restored and an [`EngineError`] comes back.
//!
//! Commands that change the model are journalled and can be undone and redone
//! (`edit.undo`, `edit.redo`; [`history`]). The session is saved to and opened from `.modelcraft`
//! files (`file.save`, `file.open`; [`modelcraft_io`]) and autosaved for recovery after a crash
//! ([`autosave`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod autosave;
pub mod cmd;
pub mod history;
pub mod sample;

use std::sync::Arc;

use modelcraft_geometry::Design;
use modelcraft_geometry::tagged_mesh::TaggedSurface;
use modelcraft_model::Model;
use serde_json::Value;

use crate::history::History;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("{0}")]
    Params(String),
    #[error(transparent)]
    Model(#[from] modelcraft_model::Error),
    #[error(transparent)]
    Geometry(#[from] modelcraft_geometry::Error),
    #[error(transparent)]
    Mesh(#[from] modelcraft_mesh::Error),
    #[error(transparent)]
    File(#[from] modelcraft_io::Error),
    #[error(transparent)]
    Deck(#[from] modelcraft_decks::Error),
    /// A command panicked; the model is unchanged.
    #[error("internal error in {command}: {message}")]
    Internal { command: String, message: String },
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// What a command works on, which decides how [`Session::execute`] records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// Reads or changes the model and its geometry. A run that changes something is journalled
    /// and becomes an undo step.
    Model,
    /// Works on the session itself: opens, saves, undoes and redoes. Never journalled; such a
    /// command keeps the journal and the undo history in step itself.
    Session,
}

/// A command: one user-visible action.
pub struct CommandSpec {
    /// Our own dotted, lower-case id: `<area>.<thing>[.<variant>]`, such as `mesh.node.add`.
    pub id: &'static str,
    pub label: &'static str,
    /// The JSON parameters, documented for scripts, `modelcraft-cli commands` and agents.
    pub params: &'static str,
    pub kind: CommandKind,
    pub run: fn(&mut Session, &Value) -> Result<Value>,
}

/// All commands.
pub fn commands() -> &'static [CommandSpec] {
    cmd::ALL
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    commands().iter().find(|c| c.id == id)
}

/// A new revision number, unique in the process: every state of every session has its own.
pub(crate) fn next_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The model being edited, the geometry it is built on and the commands that built it.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub model: Model,
    /// The open SolidCraft design (`geometry.open`); shared, so a snapshot costs nothing.
    pub design: Option<Arc<Design>>,
    /// Face-tagged boundary surfaces (`geometry.import.mesh`): the input to volume meshing.
    /// Each is shared, so the per-command snapshot is cheap.
    pub surfaces: Vec<Arc<TaggedSurface>>,
    /// The files the design and the surfaces were read from, with the hash of what was read:
    /// what a `.modelcraft` file records to open them again.
    pub sources: modelcraft_io::Geometry,
    /// Commands that changed the model, in order (a script that rebuilds it). Undo takes the
    /// last one off; redo puts it back.
    pub journal: Vec<(String, Value)>,
    /// The `.modelcraft` file the session was opened from or last saved to.
    pub path: Option<String>,
    history: History,
    /// Identifies the current state: it changes with every edit, undo and redo, and comes back
    /// when an undo returns to an earlier state.
    revision: u64,
    /// The revision last saved to (or opened from) `path`; `None` when the content was never
    /// saved, where only the empty start (revision 0) is clean.
    saved_revision: Option<u64>,
}

impl Session {
    /// Run a command. On an error or a panic the model, the design and the surfaces are left as
    /// they were.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        // Session commands (open, save, undo, redo) change the session only in their last step,
        // after everything that can fail, so they need no copy of the model to restore.
        let before = (spec.kind == CommandKind::Model).then(|| self.snapshot());
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, params)));
        let result = match outcome {
            Ok(r) => r,
            Err(p) => {
                let message = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                log::error!("command {id} panicked: {message}");
                Err(EngineError::Internal { command: id.to_string(), message })
            }
        };
        if let Some(before) = before {
            if result.is_err() {
                self.restore(before);
            } else if self.differs_from(&before) {
                self.journal.push((id.to_string(), params.clone()));
                self.revision = next_revision();
                self.record_undo(before);
            }
        }
        result
    }

    /// Run a script: a JSON array of `{"command": id, "params": {...}}`.
    pub fn run_script(&mut self, script: &Value) -> Result<()> {
        let steps = script.as_array().ok_or_else(|| EngineError::Params("a script is a JSON array of steps".into()))?;
        for (i, step) in steps.iter().enumerate() {
            let id = step.get("command").and_then(Value::as_str).ok_or_else(|| EngineError::Params(format!("step {}: no `command`", i + 1)))?;
            let params = step.get("params").cloned().unwrap_or(Value::Object(Default::default()));
            self.execute(id, &params).map_err(|e| EngineError::Params(format!("step {} ({id}): {e}", i + 1)))?;
        }
        Ok(())
    }

    /// Identifies the current state (see [`Session::is_dirty`]).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the session has changes its file does not hold. Undoing back to the saved state
    /// makes it clean again.
    pub fn is_dirty(&self) -> bool {
        self.saved_revision.unwrap_or(0) != self.revision
    }

    /// The current state is what `path` now holds.
    pub(crate) fn mark_saved(&mut self, path: String) {
        self.path = Some(path);
        self.saved_revision = Some(self.revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelcraft_geometry::sample::{Plate, plate_design};
    use serde_json::json;

    #[test]
    fn command_ids_are_unique_and_well_formed() {
        let mut ids: Vec<&str> = commands().iter().map(|c| c.id).collect();
        for id in &ids {
            assert!(id.split('.').count() >= 2 && id.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'), "{id}");
        }
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn script_builds_a_hex_and_failures_leave_the_model_alone() {
        let mut s = Session::default();
        let mut script = Vec::new();
        for (i, c) in [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]].iter().enumerate() {
            script.push(json!({"command": "mesh.node.add", "params": {"label": i + 1, "xyz": c}}));
        }
        script.push(json!({"command": "mesh.element.add", "params": {"label": 1, "kind": "hex8", "nodes": [1,2,3,4,5,6,7,8]}}));
        s.run_script(&Value::Array(script)).unwrap();
        assert_eq!(s.journal.len(), 9);
        let info = s.execute("model.info", &json!({})).unwrap();
        assert_eq!(info["nodes"], 8);
        assert_eq!(info["elements"]["hex8"], 1);

        let model = s.model.clone();
        assert!(s.execute("mesh.element.add", &json!({"label": 2, "kind": "tet4", "nodes": [1, 2, 3, 99]})).is_err());
        assert!(s.execute("mesh.node.remove", &json!({"label": 1})).is_err());
        assert_eq!(s.model, model);
        assert_eq!(s.journal.len(), 9);
    }

    #[test]
    fn hostile_params_never_panic() {
        let hostile = [
            json!(null),
            json!([]),
            json!("x"),
            json!({}),
            json!({"label": -1}),
            json!({"label": 1e300}),
            json!({"label": u64::MAX}),
            json!({"label": 1, "xyz": [1, 2]}),
            json!({"label": 1, "xyz": ["a", null, {}]}),
            json!({"label": 1, "xyz": [1e308, -1e308, 0]}),
            json!({"label": 1, "kind": "hex8", "nodes": null}),
            json!({"label": 1, "kind": "nope", "nodes": [1]}),
            json!({"label": 1, "kind": "tet4", "nodes": [1, 1, 1, 1]}),
            json!({"label": 1, "kind": "hex20", "nodes": vec![u32::MAX; 20]}),
            json!({"path": ""}),
            json!({"path": "/"}),
            json!({"path": "/dev/zero"}),
            json!({"path": "/no/such/design.solidcraft"}),
            json!({"path": "x".repeat(100_000)}),
            json!({"format": "abaqus"}),
            json!({"format": "abaqus", "path": ""}),
            json!({"format": "abaqus", "path": "/"}),
            json!({"format": "abaqus", "path": "/no/such/dir/part.inp"}),
            json!({"format": "abaqus", "path": format!("/no/such/{}", "x".repeat(100_000))}),
            json!({"format": "abaqus", "path": 7}),
            json!({"format": "x".repeat(100_000), "path": "/no/such/part.inp"}),
            json!({"format": "lsdyna", "path": "/no/such/dir/part.k"}),
            json!({"format": "lsdyna", "path": "/"}),
            json!({"format": "lsdyna", "path": "x\u{0}.k"}),
            json!({"format": null, "path": "/no/such/part.inp"}),
            json!({"format": "nastran", "path": "/no/such/dir/part.bdf"}),
            json!({"format": "nastran", "path": "/no/such/dir/part.bdf", "steps": ["Bend"]}),
            json!({"format": "nastran", "path": "/no/such/dir/part.bdf", "steps": [null, 3, "Modes"]}),
            json!({"format": "nastran", "path": "/no/such/dir/part.bdf", "steps": vec!["x".repeat(1000); 2000]}),
            json!({"format": "nastran", "path": "/no/such/dir/part.bdf", "steps": {}}),
            json!({"path": 7}),
            json!({"body": "Body1", "tolerance": 0.05}),
            json!({"body": null, "tolerance": "fine"}),
            json!({"elements": [u32::MAX, -1, 1.5]}),
            json!({"elements": {}, "bins": -5}),
            json!({"elements": [1], "bins": 1e300}),
            json!({"points": null}),
            json!({"points": [[1, 2]]}),
            json!({"points": [[1e308, -1e308, 0], [0, 0, 0]]}),
            json!({"points": [[0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]]}),
            json!({"points": vec![[1, 1, 1]; 30_000]}),
            json!({"surface": 0}),
            json!({"surface": -1}),
            json!({"surface": 1e300}),
            json!({"surface": "0", "weld": -1, "feature_angle": 1e300}),
            // Model definitions.
            json!({"name": "x".repeat(100_000)}),
            json!({"name": "A\u{0}"}),
            json!({"name": 7, "nodes": [1]}),
            json!({"name": "A", "nodes": [0, -1, 1e300]}),
            json!({"name": "A", "nodes": vec![u32::MAX; 3], "faces": [{"element": 1, "face": 0}]}),
            json!({"name": "A", "geometry": [{"body": ""}, {"body": "B", "face": "f", "edge": "e"}]}),
            json!({"name": "A", "geometry": [{"body": "x".repeat(10_000)}]}),
            json!({"name": "A", "elastic": {"type": "isotropic", "young": 1e308, "poisson": 0.49999999}}),
            json!({"name": "A", "elastic": {"type": "engineering_constants", "young": [1e-300, 1e300, 1], "poisson": [1e300, 0, 0], "shear": [0, 0, 0]}}),
            json!({"name": "A", "elastic": {"type": "isotropic", "young": "stiff"}, "density": -0.0}),
            json!({"name": "A", "set": "A", "material": "A", "kind": {"type": "solid"}}),
            json!({"name": "A", "set": "all", "material": "Steel", "kind": {"type": "beam", "profile": {"type": "pipe", "radius": 1, "thickness": 2}, "orientation": [0, 0, 0]}}),
            json!({"name": "A", "set": "all", "material": "Steel", "kind": {"type": "shell", "thickness": 1e308, "offset": -0.5, "points": 4294967295u64}}),
            json!({"name": "A", "xyz": [1e308, 0, 0]}),
            json!({"name": "A", "reference_point": "RP", "set": "end", "kind": "rigid", "dofs": [0, 7]}),
            json!({"name": "A", "type": "fixed", "target": {"set": "nope"}}),
            json!({"name": "A", "type": "displacement", "target": {"reference_point": "RP"}, "values": [null, null, null, null, null, null]}),
            json!({"name": "A", "type": "displacement", "target": {"set": "root"}, "values": [1e308, -1e308, 0, 0, 0, 0, 0]}),
            json!({"name": "A", "type": "pressure", "set": "end", "magnitude": -1e308}),
            json!({"name": "A", "type": "gravity", "acceleration": [0, 0, 0]}),
            json!({"name": "A", "type": "force", "target": {"set": "end"}, "vector": [1, 0, 0], "distribution": "lumpy"}),
            json!({"name": "A", "type": "traction", "set": "root", "vector": [1e308, 1e308, 1e308]}),
            json!({"name": "A", "type": "traction", "set": "nope", "vector": [1, 0, 0]}),
            json!({"name": "RP", "xyz": [0, 0, 0], "label": 4294967295u64}),
            json!({"name": "A", "xyz": [0, 0, 0], "label": 1}),
            json!({"name": "A", "type": "body_force", "set": "all", "vector": [1, 2]}),
            json!({"name": "A", "procedure": {"type": "frequency", "modes": 4294967295u64, "min_hz": -1}}),
            json!({"name": "A", "procedure": {"type": "frequency", "modes": 5}, "output": ["everything"], "before": 3}),
            json!({"name": "Bend", "procedure": {"type": "static"}, "loads": "push"}),
            json!({"step": "Bend", "loads": ["nope"], "active": "yes"}),
            json!({"step": "Bend", "boundary_conditions": [], "loads": []}),
            json!({"system": "furlongs"}),
            json!({"geometry": {"body": "Bar"}, "elements": [1, 99], "faces": [{"element": 1, "face": 200}]}),
            json!({"geometry": {"body": "Bar", "vertex": "v"}, "nodes": [1], "edges": [{"element": 1, "edge": 0}]}),
        ];
        for spec in commands() {
            for p in &hostile {
                let mut s = Session::default();
                let _ = s.execute(spec.id, &p.clone());
            }
        }
        assert!(Session::default().execute("no.such", &json!({})).is_err());

        // The same against a defined model, so edits and deletes reach existing definitions.
        let mut defined = Session::default();
        crate::cmd::define::tests::defined_bar(&mut defined);
        for spec in commands() {
            for p in &hostile {
                let mut s = defined.clone();
                let before = s.model.clone();
                if s.execute(spec.id, p).is_err() {
                    assert_eq!(s.model, before, "{} {p}", spec.id);
                }
            }
        }

        // The same against an open design.
        let (_dir, path) = sample_design_file(&Plate::default());
        let mut opened = Session::default();
        opened.execute("geometry.open", &json!({"path": path})).unwrap();
        let tess = [
            json!({"body": "Body1"}),
            json!({"body": "Body1", "tolerance": 0}),
            json!({"body": "Body1", "tolerance": -1}),
            json!({"body": "Body1", "tolerance": 1e-300}),
            json!({"body": "Body1", "tolerance": 1e300}),
            json!({"body": "Body2", "tolerance": 0.1}),
            json!({"body": "Body1", "tolerance": 0.5, "surface": 0}),
            json!({"body": "Body1", "tolerance": 0.5, "weld": 0}),
            json!({"body": "Body1", "tolerance": 0.5, "weld": 1e-3}),
            json!({"body": "Body1", "tolerance": 0.5, "weld": f64::MIN_POSITIVE}),
            json!({"body": "Body1", "tolerance": 0.5, "weld": 0, "weld_distance": 0}),
            json!({"body": "Body1", "tolerance": 0.5, "weld_distance": 1e300}),
            json!({"body": "Body1", "tolerance": 0.5, "weld_distance": 0.05}),
            json!({"body": "Body1", "tolerance": 0.5, "feature_angle": 1e-300}),
            json!({"body": "Body1", "tolerance": 0.5, "feature_angle": null}),
        ];
        for p in tess.iter().chain(&hostile) {
            for spec in commands() {
                let mut s = opened.clone();
                let _ = s.execute(spec.id, p);
            }
        }
    }

    /// A design file in a fresh temporary directory (removed when the guard drops).
    fn sample_design_file(plate: &Plate) -> (TempDir, String) {
        let dir = TempDir::new();
        let path = dir.0.join("plate.solidcraft");
        std::fs::write(&path, plate_design(plate).unwrap()).unwrap();
        (dir, path.to_string_lossy().into_owned())
    }

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("modelcraft-engine-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn open_a_design_list_its_bodies_and_tessellate_one() {
        let (_dir, path) = sample_design_file(&Plate::default());
        let mut s = Session::default();
        let opened = s.execute("geometry.open", &json!({"path": path})).unwrap();
        assert_eq!(opened["bodies"], json!(["Body1"]));
        assert_eq!(opened["errors"], json!([]));
        assert_eq!(s.journal.len(), 1);

        let bodies = s.execute("geometry.bodies", &json!({})).unwrap();
        let volume = bodies[0]["volume"].as_f64().unwrap();
        let expect = (40.0 * 30.0 - std::f64::consts::PI * 25.0) * 10.0;
        assert!((volume - expect).abs() < 1e-3 * expect, "{volume}");
        let faces = bodies[0]["faces"].as_array().unwrap();
        assert_eq!(faces.len(), 8);

        let t = s.execute("geometry.tessellate", &json!({"body": "Body1", "tolerance": 0.05})).unwrap();
        let per_face: Vec<u64> = t["faces"].as_array().unwrap().iter().map(|f| f["triangles"].as_u64().unwrap()).collect();
        assert!(per_face.iter().all(|&n| n > 0), "{t}");
        assert_eq!(per_face.iter().sum::<u64>(), t["triangles"].as_u64().unwrap());

        // A failed open keeps the design that was open.
        assert!(s.execute("geometry.open", &json!({"path": "/no/such/design.solidcraft"})).is_err());
        assert!(s.design.is_some());
        assert_eq!(s.journal.len(), 1);

        // Re-opening the edited design rebuilds it with the same face names.
        std::fs::write(&path, plate_design(&Plate { width: 60.0, ..Plate::default() }).unwrap()).unwrap();
        s.execute("geometry.open", &json!({"path": path})).unwrap();
        let wider = s.execute("geometry.bodies", &json!({})).unwrap();
        assert!(wider[0]["volume"].as_f64().unwrap() > volume);
        assert_eq!(wider[0]["faces"], bodies[0]["faces"]);
        assert_eq!(s.journal.len(), 2);
    }
}
