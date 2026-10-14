//! Undo and redo ([`crate::history`]).

use serde_json::Value;

use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

/// Most steps one undo or redo takes (the undo history holds at most this many).
const MAX_COUNT: u64 = crate::history::MAX_UNDO_STEPS as u64;

/// `count` steps (default 1), each a whole number in 1..=MAX_COUNT.
fn count(p: &Value) -> Result<usize> {
    match p.get("count") {
        None | Some(Value::Null) => Ok(1),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=MAX_COUNT).contains(n))
            .map(|n| n as usize)
            .ok_or_else(|| EngineError::Params(format!("`count` must be a whole number from 1 to {MAX_COUNT}"))),
    }
}

/// Run `step` `count` times; the reply is the last one's (an error when not even one could).
fn repeat(s: &mut Session, p: &Value, step: fn(&mut Session) -> Result<Value>) -> Result<Value> {
    let n = count(p)?;
    let mut reply = step(s)?;
    for _ in 1..n {
        match step(s) {
            Ok(r) => reply = r,
            Err(_) => break,
        }
    }
    Ok(reply)
}

pub const UNDO: CommandSpec = CommandSpec {
    id: "edit.undo",
    label: "Undo",
    params: r#"{"count"?: 1} — takes back the last command that changed the model (it leaves the journal and can be redone); `count` takes back that many, as far as the history reaches"#,
    form: Some(super::forms::COUNT),
    kind: CommandKind::Session,
    run: |s, p| repeat(s, p, Session::undo),
};

pub const REDO: CommandSpec = CommandSpec {
    id: "edit.redo",
    label: "Redo",
    params: r#"{"count"?: 1} — runs the last undone command's change again; `count` redoes that many, as far as there are undone commands"#,
    form: Some(super::forms::COUNT),
    kind: CommandKind::Session,
    run: |s, p| repeat(s, p, Session::redo),
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn undo_and_redo_take_a_count_and_stop_at_the_ends() {
        let mut s = Session::default();
        for l in 1..=4 {
            s.execute("mesh.node.add", &json!({"label": l, "xyz": [l, 0, 0]})).unwrap();
        }
        s.execute("edit.undo", &json!({"count": 3})).unwrap();
        assert_eq!(s.journal.len(), 1);
        s.execute("edit.redo", &json!({"count": 100})).unwrap();
        assert_eq!(s.journal.len(), 4, "as far as there are undone commands");
        for bad in [json!({"count": 0}), json!({"count": -1}), json!({"count": 1e9}), json!({"count": "x"}), json!({"count": 2.5})] {
            assert!(s.execute("edit.undo", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.journal.len(), 4);
    }
}
