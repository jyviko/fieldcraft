//! Tie: two surfaces of non-matching meshes bonded together.
//!
//! A tie names a `main` and a `secondary` surface, sets of element faces. The solvers make every
//! node of the secondary surface follow the main surface under it (the implicit solver by mortar
//! constraints with dual shape functions, so a constant stress crosses the tie exactly). Nodes of
//! the secondary surface farther than the `position_tolerance` from the main surface stay free
//! (by default half the thickness of the main element under them).

use serde::{Deserialize, Serialize};

use crate::{Error, Model, Result, context, insert_new, remove_existing, replace_existing, require};

/// Most ties in a model.
pub const MAX_TIES: usize = 1000;

/// A tie definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tie {
    pub main: String,
    pub secondary: String,
    /// How far a secondary node may lie from the main surface and still be tied (`None`: half
    /// the thickness of the main element under it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_tolerance: Option<f64>,
}

impl Tie {
    pub fn new(main: &str, secondary: &str) -> Tie {
        Tie { main: main.to_string(), secondary: secondary.to_string(), position_tolerance: None }
    }

    pub fn validate(&self) -> Result<()> {
        if self.main == self.secondary {
            return Err(Error::Invalid(format!("a tie needs two different surfaces, got `{}` twice", self.main)));
        }
        if let Some(t) = self.position_tolerance
            && !(t > 0.0 && t.is_finite())
        {
            return Err(Error::Invalid(format!("a tie's position tolerance is a positive length, got {t}")));
        }
        Ok(())
    }
}

impl Model {
    pub fn ties(&self) -> &std::collections::BTreeMap<String, Tie> {
        &self.ties
    }

    pub fn tie(&self, name: &str) -> Option<&Tie> {
        self.ties.get(name)
    }

    fn check_tie(&self, name: &str, t: &Tie) -> Result<()> {
        t.validate().map_err(|e| context("tie", name, e))?;
        for set in [&t.main, &t.secondary] {
            require(&self.sets, "set", set).map_err(|e| context("tie", name, e))?;
        }
        Ok(())
    }

    pub fn add_tie(&mut self, name: &str, tie: Tie) -> Result<()> {
        self.check_tie(name, &tie)?;
        if self.ties.len() >= MAX_TIES {
            return Err(Error::Limit(format!("a model holds at most {MAX_TIES} ties")));
        }
        insert_new(&mut self.ties, "tie", name, tie)
    }

    pub fn replace_tie(&mut self, name: &str, tie: Tie) -> Result<()> {
        self.check_tie(name, &tie)?;
        replace_existing(&mut self.ties, "tie", name, tie)
    }

    pub fn remove_tie(&mut self, name: &str) -> Result<()> {
        remove_existing(&mut self.ties, "tie", name).map(|_| ())
    }

    /// The ties that name a set, as ``tie `name` `` strings.
    pub(crate) fn tie_users<'a>(&'a self, set: &'a str) -> impl Iterator<Item = String> + 'a {
        self.ties.iter().filter(move |(_, t)| t.main == set || t.secondary == set).map(|(n, _)| format!("tie `{n}`"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Set;

    #[test]
    fn ties_validate_refer_to_sets_and_round_trip() {
        let mut m = Model::default();
        for s in ["a", "b"] {
            m.add_set(s, Set { nodes: vec![1], ..Set::default() }).unwrap();
        }
        m.add_tie("glue", Tie::new("a", "b")).unwrap();
        assert!(matches!(m.remove_set("a"), Err(Error::InUse(_))), "the tie refers to the set");
        assert!(m.add_tie("glue", Tie::new("a", "b")).is_err(), "names stay unique");
        assert!(m.add_tie("self", Tie::new("a", "a")).is_err());
        assert!(m.add_tie("far", Tie { position_tolerance: Some(-1.0), ..Tie::new("a", "b") }).is_err());
        assert!(matches!(m.add_tie("missing", Tie::new("a", "c")), Err(Error::NotFound(_))));
        let text = serde_json::to_string(&m).unwrap();
        let back: Model = serde_json::from_str(&text).unwrap();
        assert_eq!(back.ties(), m.ties());
        m.replace_tie("glue", Tie { position_tolerance: Some(0.1), ..Tie::new("b", "a") }).unwrap();
        m.remove_tie("glue").unwrap();
        assert!(m.remove_tie("glue").is_err());
    }
}
