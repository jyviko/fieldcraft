//! Solver decks: the model written in a solver's input format.

use std::path::Path;

use modelcraft_model::Severity;
use serde_json::{Value, json};

use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const WRITE: CommandSpec = CommandSpec {
    id: "deck.write",
    label: "Write Solver Deck",
    params: r#"{"format": "abaqus", "path": "part.inp"} — writes the model as a solver deck; fails naming every error of the model check and everything the format cannot express"#,
    kind: CommandKind::Session,
    run: write,
};

/// The deck formats `deck.write` knows, by the name its `format` parameter takes.
pub const FORMATS: &[&str] = &["abaqus"];

/// The deck's text in `format`.
fn render(format: &str, s: &Session) -> Result<String> {
    match format {
        "abaqus" => Ok(modelcraft_decks::abaqus::write(&s.model)?),
        other => {
            let shown: String = other.chars().take(32).collect();
            Err(EngineError::Params(format!("unknown deck format `{shown}`; known: {}", FORMATS.join(", "))))
        }
    }
}

fn write(s: &mut Session, p: &Value) -> Result<Value> {
    let format =
        p.get("format").and_then(Value::as_str).ok_or_else(|| EngineError::Params(format!("`format` must be one of {}", FORMATS.join(", "))))?;
    let path =
        p.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).ok_or_else(|| EngineError::Params("`path` must be a file path".into()))?;
    let deck = render(format, s)?;
    modelcraft_io::fs::write_atomic(Path::new(path), deck.as_bytes(), false)?;
    let warnings: Vec<_> = s.model.check().into_iter().filter(|d| d.severity == Severity::Warning).collect();
    Ok(json!({"format": format, "path": path, "bytes": deck.len(), "warnings": warnings}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_defined_model_and_refuses_what_it_cannot() {
        let dir = std::env::temp_dir().join(format!("modelcraft-deck-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bar.inp");
        let path_str = path.to_string_lossy().into_owned();

        let mut s = Session::default();
        crate::cmd::define::tests::defined_bar(&mut s);
        let journal = s.journal.len();
        let out = s.execute("deck.write", &json!({"format": "abaqus", "path": path_str})).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, modelcraft_decks::abaqus::write(&s.model).unwrap());
        assert_eq!(out["bytes"], written.len());
        assert_eq!(out["warnings"].as_array().map(Vec::len), Some(1), "`press` is in no step: {out}");
        assert!(written.contains("*ELEMENT, TYPE=C3D8I"), "{written}");
        assert_eq!(s.journal.len(), journal, "writing a deck does not change the model");
        assert!(std::fs::read_dir(&dir).unwrap().count() == 1, "no partial file is left behind");

        // Unknown formats, missing parameters and models that are not ready fail without writing.
        let other = dir.join("other.inp").to_string_lossy().into_owned();
        assert!(s.execute("deck.write", &json!({"format": "nope", "path": other})).unwrap_err().to_string().contains("known: abaqus"));
        assert!(s.execute("deck.write", &json!({"path": other})).is_err());
        assert!(s.execute("deck.write", &json!({"format": "abaqus"})).is_err());
        let mut empty = Session::default();
        assert!(matches!(empty.execute("deck.write", &json!({"format": "abaqus", "path": other})), Err(EngineError::Deck(_))));
        assert!(!dir.join("other.inp").exists());
        assert!(matches!(s.execute("deck.write", &json!({"format": "abaqus", "path": dir.to_string_lossy()})), Err(EngineError::File(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
