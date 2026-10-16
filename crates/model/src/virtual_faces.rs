//! Virtual topology: faces of a body merged into one face for meshing only.
//!
//! A **face merge** names adjacent faces of a body, by persistent name, that the meshers take as
//! one face (the geometry layer remaps the tessellation they mesh; the B-rep is unchanged). The
//! model keeps the merges in the order they were made, because a merge may name the face an
//! earlier one made (`merge:<name>`); they are applied after the partitions, so a body named may
//! be a partition cell.

use serde::{Deserialize, Serialize};

use crate::geometry::MAX_GEOMETRY_NAME_LEN;
use crate::{Error, Model, Result, check_name, context};

/// Most face merges a model holds.
pub const MAX_FACE_MERGES: usize = 1024;
/// Most faces one merge names.
pub const MAX_FACE_MERGE_FACES: usize = 1024;

/// Faces of one body meshed as one face, `merge:<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceMerge {
    pub name: String,
    pub body: String,
    /// Two or more faces by persistent name (a name also stands for the pieces a partition cut
    /// it into).
    pub faces: Vec<String>,
    /// Absorb (`geometry.face.absorb`): the merged face keeps this one's name (one of `faces`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
}

impl Model {
    /// The face merges, in the order they apply.
    pub fn face_merges(&self) -> &[FaceMerge] {
        &self.face_merges
    }

    /// Add a face merge after the others.
    pub fn add_face_merge(&mut self, merge: FaceMerge) -> Result<()> {
        let name = merge.name.clone();
        check_name("face merge", &name)?;
        if self.face_merges.iter().any(|m| m.name == name) {
            return Err(Error::Invalid(format!("face merge `{name}` already exists")));
        }
        if self.face_merges.len() >= MAX_FACE_MERGES {
            return Err(Error::Limit(format!("a model holds at most {MAX_FACE_MERGES} face merges")));
        }
        let invalid = |m: String| context("face merge", &name, Error::Invalid(m));
        let named = |s: &str| !s.trim().is_empty() && s.len() <= MAX_GEOMETRY_NAME_LEN;
        if !named(&merge.body) {
            return Err(invalid(format!("the body name is 1 to {MAX_GEOMETRY_NAME_LEN} bytes")));
        }
        if !(2..=MAX_FACE_MERGE_FACES).contains(&merge.faces.len()) {
            return Err(invalid(format!("a merge names 2 to {MAX_FACE_MERGE_FACES} faces")));
        }
        for (i, f) in merge.faces.iter().enumerate() {
            if !named(f) {
                return Err(invalid(format!("face names are 1 to {MAX_GEOMETRY_NAME_LEN} bytes")));
            }
            if merge.faces.get(..i).is_some_and(|before| before.contains(f)) {
                return Err(invalid(format!("face `{f}` is listed twice")));
            }
        }
        if merge.into.as_ref().is_some_and(|i| !merge.faces.contains(i)) {
            return Err(invalid("the face it keeps is not one of its faces".into()));
        }
        self.face_merges.push(merge);
        Ok(())
    }

    pub fn remove_face_merge(&mut self, name: &str) -> Result<FaceMerge> {
        let i = self.face_merges.iter().position(|m| m.name == name).ok_or_else(|| Error::NotFound(format!("face merge `{name}`")))?;
        Ok(self.face_merges.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(name: &str, faces: &[&str]) -> FaceMerge {
        FaceMerge { name: name.into(), body: "Base".into(), faces: faces.iter().map(|f| f.to_string()).collect(), into: None }
    }

    /// Merges keep their order, are saved and read back, and bad ones are refused.
    #[test]
    fn merges_are_kept_in_order_and_checked() {
        let mut m = Model::default();
        m.add_face_merge(merge("Fillet", &["F3:a", "F3:b"])).unwrap();
        m.add_face_merge(merge("More", &["merge:Fillet", "F1:+z"])).unwrap();
        assert!(m.add_face_merge(merge("Fillet", &["x", "y"])).is_err(), "taken");
        assert!(m.add_face_merge(merge("One", &["x"])).is_err(), "one face");
        assert!(m.add_face_merge(merge("Twice", &["x", "x"])).is_err());
        assert!(m.add_face_merge(merge("Blank", &["x", " "])).is_err());
        assert!(m.add_face_merge(merge("9bad", &["x", "y"])).is_err());
        assert!(m.add_face_merge(FaceMerge { body: String::new(), ..merge("NoBody", &["x", "y"]) }).is_err());
        assert!(m.add_face_merge(FaceMerge { into: Some("z".into()), ..merge("Into", &["x", "y"]) }).is_err());
        m.add_face_merge(FaceMerge { into: Some("y".into()), ..merge("Absorb", &["x", "y"]) }).unwrap();
        let back: Model = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.face_merges(), m.face_merges());
        assert_eq!(m.remove_face_merge("Fillet").unwrap().name, "Fillet");
        assert!(m.remove_face_merge("Fillet").is_err());
        assert_eq!(m.face_merges().len(), 2);
    }
}
