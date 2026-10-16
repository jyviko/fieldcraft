//! Defeaturing for meshing: holes, fillets, chamfers and small bosses taken out of a design so they
//! do not drive the mesh.
//!
//! A design opened from its timeline knows its features ([`Design::features`]), so a hole is
//! removed by rebuilding the design with the hole's feature suppressed, exactly as SolidCraft
//! would build it without it: the hole is filled, a fillet or chamfer goes back to its sharp edge,
//! a boss is gone. The faces the other features made keep their persistent names, so sets, seeds
//! and loads on them still resolve.
//!
//! A feature that cannot be suppressed on its own (a later feature needs it: a fillet along the
//! hole's edge, a pattern of it), or whose suppression would take a body away, is kept in the
//! B-rep and defeatured for meshing only instead, as virtual topology
//! ([`crate::virtual_faces`]): its faces are merged into the neighbour they share the longest
//! edges with, so the meshers see no edges around it (the material stays). Such a feature is
//! listed with `virtual` in [`Design::defeat_report`].
//!
//! Defeatures live in the FieldCraft model by feature id (the `F<id>` of persistent names, stable
//! across edits of the design) and are applied whenever the design is opened, before the
//! partitions. [`Design::defeatured`] keeps the design as opened, so switching a feature back on
//! rebuilds from it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::Serialize;
use solidcraft_doc::FeatureKind;

use crate::features::FeatureNode;
use crate::virtual_faces::Merge;
use crate::{Design, Error, FeatureError, Result};

/// Most features all of a design's defeatures name together.
pub const MAX_DEFEATURED: usize = 1024;
/// Most rebuilds spent finding which features suppress on their own when suppressing them all
/// at once breaks the design; the features past it are defeatured virtually.
const MAX_TRIALS: usize = 64;

/// Features of the design's timeline suppressed for meshing (`geometry.defeature`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defeature {
    pub name: String,
    /// Feature ids (`F<id>`).
    pub features: Vec<u64>,
}

/// How a defeatured feature was taken out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Defeated {
    /// The defeature that names it.
    pub defeature: String,
    pub feature: u64,
    /// `suppressed` (the design rebuilt without it) or `virtual` (its faces merged into a
    /// neighbour for meshing; the B-rep keeps it).
    pub method: &'static str,
    /// Why it could not be suppressed, for a virtual one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The defeatures applied to a design, and the design as opened.
#[derive(Debug)]
pub(crate) struct Applied {
    /// The design as opened, before any defeature.
    original: Design,
    defeatures: Vec<Defeature>,
    report: Vec<Defeated>,
    /// The virtual defeatures, as merges applied with the model's own.
    merges: Vec<Merge>,
}

/// A feature a mesh-prep pass may take out, and its size.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub id: u64,
    pub name: String,
    /// The feature's definition type (`hole`, `extrude`, `fillet`, …).
    #[serde(rename = "type")]
    pub kind: String,
    /// `hole`, `fillet`, `chamfer` or `boss`.
    pub category: &'static str,
    /// The size compared: a hole's diameter, a fillet's radius, a chamfer's distance, a boss's
    /// height (mm).
    pub size: f64,
    /// The body's faces it made.
    pub faces: Vec<String>,
}

/// Whether the feature `node` is a hole, fillet, chamfer or boss, and its size.
pub fn candidate(node: &FeatureNode) -> Option<Candidate> {
    let v = |k: &str| node.values.get(k).copied();
    let op = node.operation.as_deref();
    // A circular profile's diameter (an extrude of a circle).
    let circle = || {
        let p = node.sketch.as_ref()?.profiles.first()?;
        match p.outer.as_slice() {
            [crate::Curve::Circle { radius, .. }] if p.holes.is_empty() => Some(2.0 * radius),
            _ => None,
        }
    };
    let (category, size) = match (node.kind.as_str(), op) {
        ("hole", _) => ("hole", v("diameter")?),
        ("fillet", _) => ("fillet", v("radius")?),
        ("chamfer", _) => ("chamfer", v("distance")?),
        ("cylinder", Some("cut")) => ("hole", 2.0 * v("radius")?),
        ("extrude", Some("cut")) => ("hole", circle()?),
        ("extrude", Some("join")) => ("boss", v("distance")?.abs()),
        ("cylinder" | "box", Some("join")) => ("boss", v("height")?.abs()),
        _ => return None,
    };
    (size.is_finite() && size > 0.0).then(|| Candidate {
        id: node.id,
        name: node.name.clone(),
        kind: node.kind.clone(),
        category,
        size,
        faces: node.faces.clone(),
    })
}

impl Design {
    /// The design as opened, before its defeatures.
    pub fn undefeatured(&self) -> &Design {
        self.defeat.as_deref().map_or(self, |a| &a.original)
    }

    /// How each defeatured feature was taken out, in the defeatures' order.
    pub fn defeat_report(&self) -> &[Defeated] {
        self.defeat.as_deref().map_or(&[], |a| a.report.as_slice())
    }

    /// The design with `defeatures` applied, from the design as opened, and the merges that
    /// defeature its features virtually (apply them with the model's own, [`Design::merged`]).
    /// A defeature naming a feature the timeline lacks, or one that builds a body or a sketch, is
    /// listed in [`Design::errors`] (`defeature <name>`) and the rest still apply. A design
    /// without a timeline defeatures nothing (each is an error).
    pub fn defeatured(&self, defeatures: &[Defeature]) -> Result<(Design, Vec<Merge>)> {
        let base = self.undefeatured();
        if defeatures.iter().all(|d| d.features.is_empty()) {
            return Ok((base.clone(), Vec::new()));
        }
        if let Some(a) = self.defeat.as_deref().filter(|a| a.defeatures == defeatures) {
            return Ok((self.clone(), a.merges.clone()));
        }
        if defeatures.iter().map(|d| d.features.len()).sum::<usize>() > MAX_DEFEATURED {
            return Err(Error::Invalid(format!("a design takes at most {MAX_DEFEATURED} defeatured features")));
        }
        let mut errors = Vec::new();
        let fail = |d: &Defeature, message: String| FeatureError { feature: format!("defeature {}", d.name), message, bodies: Vec::new() };
        // The features to take out, each with the first defeature naming it.
        let mut wanted: BTreeMap<u64, &str> = BTreeMap::new();
        let mut order = Vec::new();
        for d in defeatures {
            for &id in &d.features {
                let refused = match base.history.as_deref().map(|h| h.doc.feature(id)) {
                    None => Some("the design has no feature timeline".to_string()),
                    Some(None) => Some(format!("the design has no feature F{id}")),
                    Some(Some(f)) if !suppressible(&f.kind) => {
                        Some(format!("F{id} ({}) builds a body or a sketch; it is not a feature to defeature", f.name))
                    }
                    Some(Some(_)) => None,
                };
                match refused {
                    Some(m) => errors.push(fail(d, m)),
                    None if !wanted.contains_key(&id) => {
                        wanted.insert(id, &d.name);
                        order.push(id);
                    }
                    None => {}
                }
            }
        }
        // Suppress them all at once; when that breaks the design, one at a time, keeping those
        // that rebuild cleanly.
        let mut kept: BTreeSet<u64> = order.iter().copied().collect();
        let mut rebuilt = rebuild_without(base, &kept)?.ok();
        let mut reasons: BTreeMap<u64, String> = BTreeMap::new();
        if rebuilt.is_none() {
            kept.clear();
            for (k, &id) in order.iter().enumerate() {
                if k >= MAX_TRIALS {
                    reasons.insert(id, format!("more than {MAX_TRIALS} features to try one at a time"));
                    continue;
                }
                let mut next = kept.clone();
                next.insert(id);
                match rebuild_without(base, &next)? {
                    Ok(d) => {
                        kept = next;
                        rebuilt = Some(d);
                    }
                    Err(why) => {
                        reasons.insert(id, why);
                    }
                }
            }
        }
        let mut design = match rebuilt {
            Some(d) if !kept.is_empty() => d,
            _ => base.clone(),
        };
        // The rest: their faces merged into a neighbour, body by body.
        let mut merges = Vec::new();
        let mut report = Vec::new();
        let graph = if reasons.is_empty() { Vec::new() } else { base.features(None)? };
        for &id in &order {
            let defeature = wanted.get(&id).map_or_else(String::new, |n| n.to_string());
            if kept.contains(&id) {
                report.push(Defeated { defeature, feature: id, method: "suppressed", reason: None });
                continue;
            }
            for b in &graph {
                let Some(node) = b.features.iter().find(|f| f.id == id).filter(|f| !f.faces.is_empty()) else { continue };
                match absorb(base, &b.body, &node.faces) {
                    Ok(Some(into)) => {
                        let mut faces = node.faces.clone();
                        faces.push(into.clone());
                        merges.push(Merge { name: format!("{defeature}-F{id}"), body: b.body.clone(), faces, into: Some(into) });
                    }
                    Ok(None) => {}
                    Err(e) => errors.push(FeatureError {
                        feature: format!("defeature {defeature}"),
                        message: format!("F{id} on `{}`: {e}", b.body),
                        bodies: vec![b.body.clone()],
                    }),
                }
            }
            report.push(Defeated { defeature, feature: id, method: "virtual", reason: reasons.get(&id).cloned() });
        }
        design.errors.extend(errors);
        design.defeat = Some(Arc::new(Applied { original: base.clone(), defeatures: defeatures.to_vec(), report, merges: merges.clone() }));
        Ok((design, merges))
    }
}

/// Whether a feature of this kind may be taken out without taking a body or a sketch with it.
fn suppressible(kind: &FeatureKind) -> bool {
    use solidcraft_doc::Operation::NewBody;
    match kind {
        FeatureKind::Sketch { .. }
        | FeatureKind::ConstructionPlane { .. }
        | FeatureKind::ConstructionAxis { .. }
        | FeatureKind::ConstructionPoint { .. }
        | FeatureKind::Combine { .. }
        | FeatureKind::Split { .. } => false,
        FeatureKind::Extrude { operation, .. }
        | FeatureKind::Revolve { operation, .. }
        | FeatureKind::Sweep { operation, .. }
        | FeatureKind::Loft { operation, .. }
        | FeatureKind::Box { operation, .. }
        | FeatureKind::Cylinder { operation, .. }
        | FeatureKind::Sphere { operation, .. }
        | FeatureKind::Torus { operation, .. } => *operation != NewBody,
        _ => true,
    }
}

/// `base` rebuilt with `ids` suppressed: `Err(why)` when that fails a feature that built in
/// `base` or loses one of its bodies.
fn rebuild_without(base: &Design, ids: &BTreeSet<u64>) -> Result<std::result::Result<Design, String>> {
    let history = base.history.as_deref().ok_or_else(|| Error::Invalid("the design has no feature timeline".into()))?;
    let mut doc = history.doc.clone();
    for &id in ids {
        if let Some(f) = doc.feature_mut(id) {
            f.suppressed = true;
        }
    }
    let design = crate::design::rebuild(&doc)?;
    let failed: Vec<String> = design
        .errors
        .iter()
        .filter(|e| !base.errors.iter().any(|b| b.feature == e.feature))
        .map(|e| format!("{} no longer builds: {}", e.feature, e.message))
        .collect();
    if !failed.is_empty() {
        return Ok(Err(failed.join("; ")));
    }
    let lost: Vec<&str> = base.source.iter().map(|b| b.name.as_str()).filter(|n| design.source.iter().all(|b| b.name != *n)).collect();
    if !lost.is_empty() {
        return Ok(Err(format!("body {} would be gone", lost.join(", "))));
    }
    Ok(Ok(design))
}

/// The face of `body` (in `base` as opened) outside `faces` that `faces` share the longest edges
/// with; `None` when the body has none of them.
fn absorb(base: &Design, body: &str, faces: &[String]) -> Result<Option<String>> {
    let Some(shape) = base.source_body(body) else { return Ok(None) };
    for f in faces {
        if shape.faces_named(f).is_empty() {
            continue;
        }
        // A face of the feature may border only the feature's own faces (the middle of a
        // fillet chain): the next one finds the outside.
        let mut probe = shape.clone();
        let others: Vec<String> = faces.iter().filter(|g| *g != f && !shape.faces_named(g).is_empty()).cloned().collect();
        if !others.is_empty() {
            let mut all = vec![f.clone()];
            all.extend(others);
            match shape.merge_faces(f, &all) {
                Ok(m) => probe = m,
                Err(_) => continue,
            }
        }
        if let Ok(n) = probe.absorbing_neighbour(f) {
            return Ok(Some(n));
        }
    }
    Err(Error::Invalid("its faces have no neighbour to merge into".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{Plate, plate_design};

    fn plate() -> Design {
        crate::open_design(&plate_design(&Plate::default()).unwrap()).unwrap()
    }

    fn hole(d: &Design) -> u64 {
        d.features(None).unwrap()[0].features.iter().filter_map(candidate).find(|c| c.category == "hole").unwrap().id
    }

    /// Suppressing the plate's hole fills it (the body is a solid block, its faces keep their
    /// names); an empty list brings it back.
    #[test]
    fn the_plate_without_its_hole_is_a_block() {
        let d = plate();
        let id = hole(&d);
        let c = d.features(None).unwrap()[0].features.iter().filter_map(candidate).next().unwrap();
        assert_eq!((c.category, c.size), ("hole", Plate::default().hole_diameter));
        let def = [Defeature { name: "Holes".into(), features: vec![id] }];
        let (filled, merges) = d.defeatured(&def).unwrap();
        assert!(merges.is_empty());
        let p = Plate::default();
        let v = filled.bodies[0].measure().unwrap().volume;
        assert!((v - p.width * p.depth * p.thickness).abs() < 1e-6 * v, "{v}");
        assert_eq!(filled.bodies[0].faces().len(), 6);
        assert!(filled.bodies[0].faces().iter().all(|f| d.bodies[0].faces().iter().any(|g| g.name == f.name)));
        assert_eq!(filled.defeat_report(), [Defeated { defeature: "Holes".into(), feature: id, method: "suppressed", reason: None }]);
        // The feature graph of the defeatured design leaves the hole out; the design as opened keeps it.
        assert_eq!(filled.features(None).unwrap()[0].features.len(), 1);
        assert_eq!(filled.undefeatured().features(None).unwrap()[0].features.len(), 2);
        // Applied again with the same list: the same design, no rebuild; with none: as opened.
        let (again, _) = filled.defeatured(&def).unwrap();
        assert_eq!(again.bodies[0].faces().len(), 6);
        let (back, _) = filled.defeatured(&[]).unwrap();
        assert_eq!(back.bodies[0].faces().len(), d.bodies[0].faces().len());
        assert!(back.defeat_report().is_empty());
    }

    /// A feature the timeline lacks, or the base extrude, is an error of the defeature, not of
    /// the call.
    #[test]
    fn unknown_and_body_features_are_listed_errors() {
        let d = plate();
        let base = d.features(None).unwrap()[0].features[0].id;
        let (out, _) = d.defeatured(&[Defeature { name: "Bad".into(), features: vec![base, 999] }]).unwrap();
        let errors: Vec<&str> = out.errors.iter().filter(|e| e.feature == "defeature Bad").map(|e| e.message.as_str()).collect();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("builds a body") && errors[1].contains("no feature F999"), "{errors:?}");
        assert_eq!(out.bodies[0].faces().len(), d.bodies[0].faces().len());
    }

    /// The bracket's fillet is suppressed (back to a sharp corner) and listed as one.
    #[cfg(feature = "scripts")]
    #[test]
    fn a_bracket_fillet_goes_back_to_its_sharp_edge() {
        let script = r#"[
            {"command": "sketch.create", "params": {"plane": "XY", "name": "Profile"}},
            {"command": "sketch.line", "params": {"points": [[0, 0], [60, 0], [60, 8], [8, 8], [8, 50], [0, 50]], "closed": true}},
            {"command": "sketch.finish", "params": {}},
            {"command": "solid.extrude", "params": {"distance": 40, "body_name": "Bracket"}},
            {"command": "solid.fillet", "params": {"edges": [[8, 8, 20]], "radius": 6}},
            {"command": "solid.hole", "params": {"position": [40, 8, 20], "diameter": 8}}
        ]"#;
        let d = crate::open_design(&crate::script_design(script).unwrap()).unwrap();
        let cands: Vec<Candidate> = d.features(None).unwrap()[0].features.iter().filter_map(candidate).collect();
        assert_eq!(cands.iter().map(|c| (c.category, c.size)).collect::<Vec<_>>(), [("fillet", 6.0), ("hole", 8.0)]);
        let fillet = cands[0].id;
        let (sharp, merges) = d.defeatured(&[Defeature { name: "Small".into(), features: vec![fillet] }]).unwrap();
        assert!(merges.is_empty());
        let (before, after) = (d.bodies[0].measure().unwrap().volume, sharp.bodies[0].measure().unwrap().volume);
        // The fillet's material comes out: (1 − π/4) r² × 40 in the inside corner.
        let fill = (1.0 - std::f64::consts::FRAC_PI_4) * 36.0 * 40.0;
        assert!((before - after - fill).abs() < 1e-3 * fill, "{before} {after}");
        assert!(sharp.bodies[0].faces().iter().all(|f| !f.name.starts_with(&format!("F{fillet}:"))));
    }

    /// A hole whose rim a later fillet rounds cannot be suppressed alone (the fillet would lose
    /// its edge): it is defeatured virtually, its faces merged into a neighbour, the B-rep kept.
    #[cfg(feature = "scripts")]
    #[test]
    fn a_hole_a_fillet_needs_is_defeatured_virtually() {
        let script = r#"[
            {"command": "solid.box", "params": {"length": 40, "width": 40, "height": 10, "body_name": "Block"}},
            {"command": "solid.hole", "params": {"position": [20, 20, 10], "diameter": 10}},
            {"command": "solid.fillet", "params": {"edges": [[25, 20, 10]], "radius": 1}}
        ]"#;
        let d = crate::open_design(&crate::script_design(script).unwrap()).unwrap();
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        let graph = d.features(None).unwrap();
        let hole = graph[0].features.iter().find(|f| f.kind == "hole").unwrap();
        let (out, merges) = d.defeatured(&[Defeature { name: "H".into(), features: vec![hole.id] }]).unwrap();
        let report = out.defeat_report();
        assert_eq!((report.len(), report[0].method), (1, "virtual"), "{report:?}");
        assert!(report[0].reason.as_deref().is_some_and(|r| r.contains("no longer builds")), "{report:?}");
        assert_eq!(merges.len(), 1, "{merges:?}");
        let m = &merges[0];
        assert!(m.into.as_ref().is_some_and(|i| !hole.faces.contains(i)) && m.faces.len() == hole.faces.len() + 1, "{m:?}");
        // The B-rep is the design as opened; the merge applies.
        assert_eq!(out.bodies[0].faces().len(), d.bodies[0].faces().len());
        let merged = out.merged(&merges).unwrap();
        assert!(merged.errors.is_empty(), "{:?}", merged.errors);
        assert_eq!(merged.bodies[0].faces().len(), d.bodies[0].faces().len() - hole.faces.len());
    }
}
