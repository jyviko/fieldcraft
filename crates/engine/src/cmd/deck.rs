//! Solver decks: the model written in a solver's input format.

use std::path::Path;

use serde_json::{Value, json};

use super::req;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const WRITE: CommandSpec = CommandSpec {
    id: "deck.write",
    label: "Write Solver Deck",
    params: r#"{"format": "abaqus", "path": "part.inp"} — writes the model as a solver deck (formats: modelcraft_decks::FORMATS); fails naming every error of the model check and everything the format cannot express"#,
    kind: CommandKind::Session,
    run: write,
};

fn write(s: &mut Session, p: &Value) -> Result<Value> {
    let name: &str = req(p, "format")?;
    let format = modelcraft_decks::format(name).ok_or_else(|| {
        let shown: String = name.chars().take(32).collect();
        let known: Vec<&str> = modelcraft_decks::FORMATS.iter().map(|f| f.name).collect();
        EngineError::Params(format!("unknown deck format `{shown}`; known: {}", known.join(", ")))
    })?;
    let path = super::path_param(p, "path")?;
    let deck = format.write(&s.model)?;
    modelcraft_io::fs::write_atomic(Path::new(path), deck.text.as_bytes(), false)?;
    Ok(json!({"format": format.name, "path": path, "bytes": deck.text.len(), "warnings": deck.warnings}))
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
