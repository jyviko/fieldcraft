//! Virtual topology: faces of a body merged into one face for meshing only.
//!
//! A **face merge** names adjacent faces of a body, by persistent name, that the meshers take as
//! one face (the geometry layer remaps the tessellation they mesh; the B-rep is unchanged). The
//! model keeps the merges in the order they were made, because a merge may name the face an
//! earlier one made (`merge:<name>`); they are applied after the partitions, so a body named may
//! be a partition cell.
//!
//! A **stitch** closes an imported surface's gaps under a tolerance for meshing (the geometry
//! layer welds its open points and splits T-junctions).

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

/// Most face splits a model holds.
pub const MAX_FACE_SPLITS: usize = 1024;

/// A face of a body split along a datum plane for meshing (`split:<name>:a|b`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceSplit {
    pub name: String,
    pub body: String,
    pub face: String,
    /// The datum plane it splits along.
    pub datum: String,
}

impl Model {
    /// The face splits, in the order they apply (after the merges).
    pub fn face_splits(&self) -> &[FaceSplit] {
        &self.face_splits
    }

    /// Add a face split after the others.
    pub fn add_face_split(&mut self, split: FaceSplit) -> Result<()> {
        let name = split.name.clone();
        check_name("face split", &name)?;
        if self.face_splits.iter().any(|s| s.name == name) {
            return Err(Error::Invalid(format!("face split `{name}` already exists")));
        }
        if self.face_splits.len() >= MAX_FACE_SPLITS {
            return Err(Error::Limit(format!("a model holds at most {MAX_FACE_SPLITS} face splits")));
        }
        let invalid = |m: String| context("face split", &name, Error::Invalid(m));
        for n in [&split.body, &split.face] {
            if n.trim().is_empty() || n.len() > MAX_GEOMETRY_NAME_LEN {
                return Err(invalid(format!("body and face names are 1 to {MAX_GEOMETRY_NAME_LEN} bytes")));
            }
        }
        match self.datum(&split.datum) {
            None => return Err(context("face split", &name, Error::NotFound(format!("datum `{}`", split.datum)))),
            Some(d) if d.plane().is_none() => {
                return Err(invalid(format!("datum `{}` is a datum {}; a split is along a plane", split.datum, d.kind())));
            }
            Some(_) => {}
        }
        self.face_splits.push(split);
        Ok(())
    }

    pub fn remove_face_split(&mut self, name: &str) -> Result<FaceSplit> {
        let i = self.face_splits.iter().position(|s| s.name == name).ok_or_else(|| Error::NotFound(format!("face split `{name}`")))?;
        Ok(self.face_splits.remove(i))
    }
}

/// Most stitches a model holds.
pub const MAX_STITCHES: usize = 1024;

/// An imported surface's gaps under `tolerance` (mm) closed for meshing (`geometry.stitch`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stitch {
    pub name: String,
    /// The imported surface, by name.
    pub surface: String,
    pub tolerance: f64,
}

impl Model {
    /// The stitches, in the order they apply.
    pub fn stitches(&self) -> &[Stitch] {
        &self.stitches
    }

    /// Add a stitch after the others.
    pub fn add_stitch(&mut self, stitch: Stitch) -> Result<()> {
        let name = stitch.name.clone();
        check_name("stitch", &name)?;
        if self.stitches.iter().any(|s| s.name == name) {
            return Err(Error::Invalid(format!("stitch `{name}` already exists")));
        }
        if self.stitches.len() >= MAX_STITCHES {
            return Err(Error::Limit(format!("a model holds at most {MAX_STITCHES} stitches")));
        }
        let invalid = |m: String| context("stitch", &name, Error::Invalid(m));
        if stitch.surface.trim().is_empty() || stitch.surface.len() > MAX_GEOMETRY_NAME_LEN {
            return Err(invalid(format!("the surface name is 1 to {MAX_GEOMETRY_NAME_LEN} bytes")));
        }
        if !(stitch.tolerance > 0.0 && stitch.tolerance <= crate::MAX_COORD) {
            return Err(invalid("the tolerance must be a positive length".into()));
        }
        self.stitches.push(stitch);
        Ok(())
    }

    pub fn remove_stitch(&mut self, name: &str) -> Result<Stitch> {
        let i = self.stitches.iter().position(|s| s.name == name).ok_or_else(|| Error::NotFound(format!("stitch `{name}`")))?;
        Ok(self.stitches.remove(i))
    }
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

    #[test]
    fn splits_need_a_datum_plane_and_hold_it() {
        let mut m = Model::default();
        m.add_datum("Mid", crate::Datum::Plane { point: [0.0; 3], normal: [1.0, 0.0, 0.0] }).unwrap();
        m.add_datum("P", crate::Datum::Point { point: [0.0; 3] }).unwrap();
        let sp = |n: &str, d: &str| FaceSplit { name: n.into(), body: "Base".into(), face: "F1:+z".into(), datum: d.into() };
        m.add_face_split(sp("S", "Mid")).unwrap();
        for bad in [sp("S", "Mid"), sp("T", "P"), sp("U", "nope"), FaceSplit { face: String::new(), ..sp("V", "Mid") }] {
            assert!(m.add_face_split(bad).is_err());
        }
        assert!(m.remove_datum("Mid").is_err(), "in use by the split");
        let back: Model = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.face_splits(), m.face_splits());
        m.remove_face_split("S").unwrap();
        m.remove_datum("Mid").unwrap();
    }

    #[test]
    fn stitches_are_kept_in_order_and_checked() {
        let mut m = Model::default();
        let st = |n: &str, t: f64| Stitch { name: n.into(), surface: "Mesh1".into(), tolerance: t };
        m.add_stitch(st("Gaps", 0.1)).unwrap();
        for bad in [st("Gaps", 0.1), st("A", 0.0), st("B", f64::NAN), st("C", -1.0), Stitch { surface: " ".into(), ..st("D", 0.1) }] {
            assert!(m.add_stitch(bad).is_err());
        }
        let back: Model = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.stitches(), m.stitches());
        m.remove_stitch("Gaps").unwrap();
        assert!(m.stitches().is_empty());
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

/// Most defeatures a model holds.
pub const MAX_DEFEATURES: usize = 1024;
/// Most features one defeature names.
pub const MAX_DEFEATURE_FEATURES: usize = 1024;

/// Features of the design's timeline taken out for meshing (`geometry.defeature`): holes filled,
/// fillets and chamfers back to their sharp edges, bosses removed, by feature id (`F<id>` in
/// persistent names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defeature {
    pub name: String,
    pub features: Vec<u64>,
}

impl Model {
    /// The defeatures, in the order they were made.
    pub fn defeatures(&self) -> &[Defeature] {
        &self.defeatures
    }

    /// Add a defeature, or replace the features of the one of that name (in its place).
    pub fn set_defeature(&mut self, defeature: Defeature) -> Result<()> {
        let name = defeature.name.clone();
        check_name("defeature", &name)?;
        let invalid = |m: String| context("defeature", &name, Error::Invalid(m));
        if defeature.features.is_empty() || defeature.features.len() > MAX_DEFEATURE_FEATURES {
            return Err(invalid(format!("1 to {MAX_DEFEATURE_FEATURES} features")));
        }
        let mut seen = std::collections::BTreeSet::new();
        if let Some(id) = defeature.features.iter().find(|id| !seen.insert(**id)) {
            return Err(invalid(format!("feature F{id} is listed twice")));
        }
        match self.defeatures.iter().position(|d| d.name == name) {
            Some(i) => {
                if let Some(d) = self.defeatures.get_mut(i) {
                    *d = defeature;
                }
            }
            None if self.defeatures.len() >= MAX_DEFEATURES => {
                return Err(Error::Limit(format!("a model holds at most {MAX_DEFEATURES} defeatures")));
            }
            None => self.defeatures.push(defeature),
        }
        Ok(())
    }

    pub fn remove_defeature(&mut self, name: &str) -> Result<Defeature> {
        let i = self.defeatures.iter().position(|d| d.name == name).ok_or_else(|| Error::NotFound(format!("defeature `{name}`")))?;
        Ok(self.defeatures.remove(i))
    }
}
