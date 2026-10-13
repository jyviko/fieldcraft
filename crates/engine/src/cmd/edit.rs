//! Undo and redo ([`crate::history`]).

use serde_json::Value;

use crate::{CommandKind, CommandSpec, Result, Session};

pub const UNDO: CommandSpec = CommandSpec {
    id: "edit.undo",
    label: "Undo",
    params: "{} — takes back the last command that changed the model (it leaves the journal and can be redone)",
    kind: CommandKind::Session,
    run: undo,
};

pub const REDO: CommandSpec =
    CommandSpec {
        id: "edit.redo", label: "Redo", params: "{} — runs the last undone command's change again", kind: CommandKind::Session, run: redo
    };

fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    s.undo()
}

fn redo(s: &mut Session, _: &Value) -> Result<Value> {
    s.redo()
}
