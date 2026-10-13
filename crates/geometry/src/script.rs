//! Designs from SolidCraft command scripts (feature `scripts`): the recipes of the test corpus.
//!
//! A script is SolidCraft's own script form, a JSON array of `{"command", "params"}` or an object
//! with a `commands` array, the same file `solidcraft-cli run` takes. [`script_design`] runs it in
//! a SolidCraft session and saves the result as `.solidcraft` file bytes, which then open through
//! [`crate::open_design`] like any design a person saved in SolidCraft.

use solidcraft_kernel::{KernelError, guard};

use crate::{Error, Result};

/// Largest script read (SolidCraft's design limit is far larger than any recipe needs).
pub const MAX_SCRIPT_BYTES: usize = 4 << 20;

/// Run a SolidCraft command script and return the design it builds as `.solidcraft` file bytes.
/// A command that fails stops the script and is the error.
pub fn script_design(script: &str) -> Result<Vec<u8>> {
    if script.len() > MAX_SCRIPT_BYTES {
        return Err(Error::Invalid(format!("script is larger than {MAX_SCRIPT_BYTES} bytes")));
    }
    let value: serde_json::Value = serde_json::from_str(script).map_err(|e| Error::Invalid(format!("script is not JSON: {e}")))?;
    let doc = guard("run script", || {
        let mut session = solidcraft_engine::Session::default();
        session.run_script(&value).map_err(|e| KernelError::Invalid(format!("script: {e}")))?;
        Ok(session.doc.clone())
    })?;
    guard("save design", || solidcraft_io::write_design_container(&doc).map_err(|e| KernelError::Internal(format!("save design: {e}"))))
        .map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_design;

    #[test]
    fn a_script_builds_a_design_that_opens() {
        let script = r#"{"commands": [
            {"command": "solid.box", "params": {"length": 10, "width": 20, "height": 30, "body_name": "B"}}
        ]}"#;
        let design = open_design(&script_design(script).unwrap()).unwrap();
        assert!(design.errors.is_empty(), "{:?}", design.errors);
        let m = design.body("B").unwrap().measure().unwrap();
        assert!((m.volume - 6000.0).abs() < 1e-6, "{}", m.volume);
    }

    #[test]
    fn hostile_scripts_are_an_error() {
        for s in ["", "{", "[1]", "{}", r#"[{"command": "no.such"}]"#, r#"[{"command": "solid.box", "params": {"length": -1e400}}]"#] {
            assert!(script_design(s).is_err(), "{s}");
        }
        assert!(script_design(&" ".repeat(MAX_SCRIPT_BYTES + 1)).is_err());
    }
}
