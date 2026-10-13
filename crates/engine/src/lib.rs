//! The command engine.
//!
//! Everything a user can do is a command ([`CommandSpec`]) with a JSON form. The desktop UI,
//! scripts, the CLI and the control channel all run the same commands through
//! [`Session::execute`], which never lets a failure or a panic damage the session: the model and
//! the surfaces are restored and an [`EngineError`] comes back.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod cmd;
pub mod sample;

use std::sync::Arc;

use modelcraft_geometry::Design;
use modelcraft_geometry::tagged_mesh::TaggedSurface;
use modelcraft_model::Model;
use serde_json::Value;

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
    /// A command panicked; the model is unchanged.
    #[error("internal error in {command}: {message}")]
    Internal { command: String, message: String },
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// A command: one user-visible action.
pub struct CommandSpec {
    /// Our own dotted, lower-case id: `<area>.<thing>[.<variant>]`, such as `mesh.node.add`.
    pub id: &'static str,
    pub label: &'static str,
    /// The JSON parameters, documented for scripts, `modelcraft-cli commands` and agents.
    pub params: &'static str,
    pub run: fn(&mut Session, &Value) -> Result<Value>,
}

/// All commands.
pub fn commands() -> &'static [CommandSpec] {
    cmd::ALL
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    commands().iter().find(|c| c.id == id)
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
    /// Commands that changed the model, in order (a script that rebuilds it).
    pub journal: Vec<(String, Value)>,
}

impl Session {
    /// Run a command. On an error or a panic the model, the design and the surfaces are left as
    /// they were.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        let before = self.model.clone();
        let design_before = self.design.clone();
        let surfaces_before = self.surfaces.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, params)));
        let result = match outcome {
            Ok(r) => r,
            Err(p) => {
                let message = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                log::error!("command {id} panicked: {message}");
                Err(EngineError::Internal { command: id.to_string(), message })
            }
        };
        let design_changed = self.design.as_ref().map(Arc::as_ptr) != design_before.as_ref().map(Arc::as_ptr);
        let surfaces_changed =
            self.surfaces.len() != surfaces_before.len() || self.surfaces.iter().zip(&surfaces_before).any(|(a, b)| !Arc::ptr_eq(a, b));
        match &result {
            Ok(_) if self.model != before || design_changed || surfaces_changed => self.journal.push((id.to_string(), params.clone())),
            Ok(_) => {}
            Err(_) => {
                self.model = before;
                self.design = design_before;
                self.surfaces = surfaces_before;
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
        ];
        for spec in commands() {
            for p in &hostile {
                let mut s = Session::default();
                let _ = s.execute(spec.id, &p.clone());
            }
        }
        assert!(Session::default().execute("no.such", &json!({})).is_err());

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
