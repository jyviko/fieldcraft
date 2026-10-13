//! Named sets: regions of the model that sections, couplings, boundary conditions and loads
//! apply to.
//!
//! A set is defined by geometry names, explicit labels or both; it is resolved against the
//! current mesh into nodes, elements and element faces ([`ResolvedSet`]). Defining sets by
//! geometry is the norm: the definition outlives the mesh, so a remesh (or an edit of the design
//! followed by a remesh) keeps every set, section and load in place. Explicit labels are for
//! meshes with no geometry, such as one read from a deck.
//!
//! What each member contributes:
//! - a body: the elements filling it, their nodes, and the element faces on the body's faces;
//! - a face: the element faces on it, the shell elements filling it, and their nodes;
//! - an edge: the nodes of element edges on it, the line elements filling it, and their nodes;
//! - a vertex: the node on it;
//! - node labels: those nodes; element labels: those elements and their nodes; element faces:
//!   those faces and their nodes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::check::ResolvedSet;
use crate::materials::context;
use crate::{ElementFace, Error, GeometryRef, MAX_ELEMENTS, MAX_NODES, Model, Result, insert_new, not_in_use, replace_existing};

/// Most geometry references in one set.
pub const MAX_SET_REFERENCES: usize = 100_000;

/// A set's definition. Members are kept sorted and without repeats.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub geometry: Vec<GeometryRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<ElementFace>,
}

impl Set {
    /// A set of geometry entities.
    pub fn of_geometry(geometry: Vec<GeometryRef>) -> Set {
        Set { geometry, ..Set::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.geometry.is_empty() && self.nodes.is_empty() && self.elements.is_empty() && self.faces.is_empty()
    }

    /// Check the definition's size and labels, and sort its members.
    pub fn normalized(mut self) -> Result<Set> {
        if self.is_empty() {
            return Err(Error::Invalid("a set needs geometry, nodes, elements or element faces".into()));
        }
        if self.geometry.len() > MAX_SET_REFERENCES {
            return Err(Error::Limit(format!("a set names at most {MAX_SET_REFERENCES} geometry entities")));
        }
        if self.nodes.len() > MAX_NODES || self.elements.len() > MAX_ELEMENTS || self.faces.len() > MAX_ELEMENTS {
            return Err(Error::Limit("a set lists at most as many members as a model holds".into()));
        }
        let bad_label = |l: &u32| *l == 0 || *l > crate::MAX_LABEL;
        if let Some(l) = self.nodes.iter().chain(&self.elements).chain(self.faces.iter().map(|f| &f.element)).find(|l| bad_label(l)) {
            return Err(Error::Invalid(format!("label {l} is outside 1..={}", crate::MAX_LABEL)));
        }
        if let Some(f) = self.faces.iter().find(|f| f.face == 0) {
            return Err(Error::Invalid(format!("element {} face 0: faces are numbered from 1", f.element)));
        }
        for v in [&mut self.nodes, &mut self.elements] {
            v.sort_unstable();
            v.dedup();
        }
        self.faces.sort_unstable();
        self.faces.dedup();
        self.geometry.sort();
        self.geometry.dedup();
        Ok(self)
    }
}

impl Model {
    pub fn sets(&self) -> &BTreeMap<String, Set> {
        &self.sets
    }

    pub fn set(&self, name: &str) -> Option<&Set> {
        self.sets.get(name)
    }

    pub fn add_set(&mut self, name: &str, set: Set) -> Result<()> {
        let set = set.normalized().map_err(|e| context("set", name, e))?;
        insert_new(&mut self.sets, "set", name, set)
    }

    pub fn replace_set(&mut self, name: &str, set: Set) -> Result<()> {
        let set = set.normalized().map_err(|e| context("set", name, e))?;
        replace_existing(&mut self.sets, "set", name, set)
    }

    pub fn remove_set(&mut self, name: &str) -> Result<()> {
        not_in_use("set", name, self.set_users(name))?;
        self.sets.remove(name).map(|_| ()).ok_or_else(|| Error::NotFound(format!("set `{name}`")))
    }

    /// The definitions that refer to a set, as `kind \`name\`` strings.
    pub(crate) fn set_users(&self, set: &str) -> Vec<String> {
        let mut users = Vec::new();
        users.extend(self.sections.iter().filter(|(_, s)| s.set == set).map(|(n, _)| format!("section `{n}`")));
        users.extend(self.couplings.iter().filter(|(_, c)| c.set == set).map(|(n, _)| format!("coupling `{n}`")));
        users
            .extend(self.boundary_conditions.iter().filter(|(_, b)| b.target().set() == Some(set)).map(|(n, _)| format!("boundary condition `{n}`")));
        users.extend(self.loads.iter().filter(|(_, l)| l.set() == Some(set)).map(|(n, _)| format!("load `{n}`")));
        users
    }

    /// Resolve one set against the current mesh. The error names every member that is not on
    /// the mesh.
    pub fn resolve_set(&self, name: &str) -> Result<ResolvedSet> {
        let set = self.sets.get(name).ok_or_else(|| Error::NotFound(format!("set `{name}`")))?;
        let (resolved, missing) = self.resolve_definition(set);
        if missing.is_empty() { Ok(resolved) } else { Err(Error::NotFound(format!("set `{name}`: {}", missing.join("; ")))) }
    }

    /// Resolve a definition, and list the members that are not on the mesh.
    pub(crate) fn resolve_definition(&self, set: &Set) -> (ResolvedSet, Vec<String>) {
        let c = &self.classification;
        let mut nodes = BTreeSet::new();
        let mut elements = BTreeSet::new();
        let mut faces = BTreeSet::new();
        let mut missing = Vec::new();

        // Geometry: the selected tags of each dimension, then one pass over the tagged elements.
        let mut regions = BTreeSet::new();
        let mut face_tags = BTreeSet::new();
        let mut edge_tags = BTreeSet::new();
        let mut vertex_tags = BTreeSet::new();
        for r in &set.geometry {
            let tags = c.tags_selected_by(r);
            if tags.is_empty() {
                missing.push(if c.is_empty() {
                    format!("{r} is not on the mesh (the mesh carries no geometry tags)")
                } else {
                    format!("{r} is not on the mesh")
                });
                continue;
            }
            match r.entity.dimension() {
                3 => {
                    regions.extend(tags);
                    // A body's skin: every tagged face of the body.
                    face_tags.extend(
                        c.entities()
                            .iter()
                            .enumerate()
                            .filter(|(_, e)| e.body == r.body && e.entity.dimension() == 2)
                            .filter_map(|(i, _)| u32::try_from(i).ok()),
                    );
                }
                2 => {
                    regions.extend(&tags);
                    face_tags.extend(tags);
                }
                1 => {
                    regions.extend(&tags);
                    edge_tags.extend(tags);
                }
                _ => vertex_tags.extend(tags),
            }
        }
        if !(regions.is_empty() && face_tags.is_empty() && edge_tags.is_empty()) {
            for (&label, tags) in c.elements() {
                let Some(element) = self.elements.get(&label) else { continue };
                if tags.region.is_some_and(|t| regions.contains(&t)) {
                    elements.insert(label);
                    nodes.extend(&element.nodes);
                }
                for (&face, t) in &tags.faces {
                    if face_tags.contains(t) {
                        faces.insert(ElementFace { element: label, face });
                    }
                }
                for (&edge, t) in &tags.edges {
                    if edge_tags.contains(t) {
                        let local = usize::from(edge).checked_sub(1).and_then(|i| element.kind.edges().get(i)).copied().unwrap_or_default();
                        nodes.extend(local.iter().filter_map(|&i| element.nodes.get(i)));
                    }
                }
            }
        }
        if !vertex_tags.is_empty() {
            nodes.extend(c.nodes().iter().filter(|(_, t)| vertex_tags.contains(t)).map(|(n, _)| n));
        }

        // Explicit labels.
        for &n in &set.nodes {
            if self.nodes.contains_key(&n) {
                nodes.insert(n);
            } else {
                missing.push(format!("node {n} is not in the mesh"));
            }
        }
        for &l in &set.elements {
            match self.elements.get(&l) {
                Some(e) => {
                    elements.insert(l);
                    nodes.extend(&e.nodes);
                }
                None => missing.push(format!("element {l} is not in the mesh")),
            }
        }
        for &f in &set.faces {
            match self.elements.get(&f.element) {
                Some(_) if self.face_nodes(f).is_some() => {
                    faces.insert(f);
                }
                Some(e) => missing.push(format!("element {} ({}) has no face {}", f.element, e.kind.name(), f.face)),
                None => missing.push(format!("element {} is not in the mesh", f.element)),
            }
        }
        for f in &faces {
            nodes.extend(self.face_nodes(*f).unwrap_or_default());
        }
        // Long lists of missing labels are summarised.
        if missing.len() > 20 {
            let more = missing.len() - 20;
            missing.truncate(20);
            missing.push(format!("and {more} more"));
        }
        (ResolvedSet { nodes: nodes.into_iter().collect(), elements: elements.into_iter().collect(), faces: faces.into_iter().collect() }, missing)
    }

    /// The node labels of an element face, in the face's order ([`crate::ElementKind::faces`]).
    pub fn face_nodes(&self, face: ElementFace) -> Option<Vec<u32>> {
        let e = self.elements.get(&face.element)?;
        let local = e.kind.faces().get(usize::from(face.face).checked_sub(1)?)?;
        local.iter().map(|&i| e.nodes.get(i).copied()).collect()
    }
}
