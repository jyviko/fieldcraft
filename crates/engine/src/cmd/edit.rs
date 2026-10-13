//! Undo and redo ([`crate::history`]).

use crate::{CommandKind, CommandSpec};

pub const UNDO: CommandSpec = CommandSpec {
    id: "edit.undo",
    label: "Undo",
    params: "{} — takes back the last command that changed the model (it leaves the journal and can be redone)",
    kind: CommandKind::Session,
    run: |s, _| s.undo(),
};

pub const REDO: CommandSpec = CommandSpec {
    id: "edit.redo",
    label: "Redo",
    params: "{} — runs the last undone command's change again",
    kind: CommandKind::Session,
    run: |s, _| s.redo(),
};
