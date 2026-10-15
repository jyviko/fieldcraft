//! How the mesh lies on the geometry.
//!
//! Geometry is referred to by name only ([`GeometryRef`]): a body's name and, inside it, the
//! persistent name of a face, edge or vertex (for SolidCraft designs, names such as `F2:side:l4`
//! that follow the face through edits of the design). The model never sees the geometry itself.
//!
//! A mesher records where it put each mesh entity ([`Classification`]): every element on the
//! body (solids), face (shells) or edge (beams) it fills, every boundary element face on the
//! geometry face it lies on, element edges on geometry edges and nodes on geometry vertices.
//! Sets defined by geometry names resolve through these tags ([`crate::sets`]), so they follow a
//! remesh: the new mesh carries new tags for the same names.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Error, Model, Result};

/// Longest body or entity name accepted (persistent names can be long; they are not written to
/// decks as such).
pub const MAX_GEOMETRY_NAME_LEN: usize = 256;

/// Most distinct geometry entities a mesh can be tagged with.
pub const MAX_ENTITIES: usize = 1_000_000;

/// An entity of a body.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Entity {
    /// The whole body.
    Body,
    Face(String),
    Edge(String),
    Vertex(String),
}

impl Entity {
    /// `body`, `face`, `edge` or `vertex`.
    pub fn kind(&self) -> &'static str {
        match self {
            Entity::Body => "body",
            Entity::Face(_) => "face",
            Entity::Edge(_) => "edge",
            Entity::Vertex(_) => "vertex",
        }
    }

    /// The entity's name within its body (`None` for the body itself).
    pub fn name(&self) -> Option<&str> {
        match self {
            Entity::Body => None,
            Entity::Face(n) | Entity::Edge(n) | Entity::Vertex(n) => Some(n),
        }
    }

    /// The topological dimension: 3 body, 2 face, 1 edge, 0 vertex.
    pub fn dimension(&self) -> u8 {
        match self {
            Entity::Body => 3,
            Entity::Face(_) => 2,
            Entity::Edge(_) => 1,
            Entity::Vertex(_) => 0,
        }
    }
}

/// A named piece of geometry: `{"body": "Body1"}`, `{"body": "Body1", "face": "F2:top"}`, or
/// with `edge` or `vertex` in place of `face`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "GeometryRefRepr", into = "GeometryRefRepr")]
pub struct GeometryRef {
    pub body: String,
    pub entity: Entity,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeometryRefRepr {
    body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    face: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vertex: Option<String>,
}

impl TryFrom<GeometryRefRepr> for GeometryRef {
    type Error = Error;
    fn try_from(r: GeometryRefRepr) -> Result<Self> {
        let entity = match (r.face, r.edge, r.vertex) {
            (None, None, None) => Entity::Body,
            (Some(n), None, None) => Entity::Face(n),
            (None, Some(n), None) => Entity::Edge(n),
            (None, None, Some(n)) => Entity::Vertex(n),
            _ => return Err(Error::Invalid("a geometry reference names at most one of `face`, `edge` or `vertex`".into())),
        };
        GeometryRef::new(r.body, entity)
    }
}

impl From<GeometryRef> for GeometryRefRepr {
    fn from(g: GeometryRef) -> Self {
        let mut r = GeometryRefRepr { body: g.body, face: None, edge: None, vertex: None };
        match g.entity {
            Entity::Body => {}
            Entity::Face(n) => r.face = Some(n),
            Entity::Edge(n) => r.edge = Some(n),
            Entity::Vertex(n) => r.vertex = Some(n),
        }
        r
    }
}

fn check_geometry_name(what: &str, name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_GEOMETRY_NAME_LEN || name.chars().any(char::is_control) {
        let shown: String = name.chars().take(64).filter(|c| !c.is_control()).collect();
        return Err(Error::Invalid(format!("{what} name `{shown}` must be 1 to {MAX_GEOMETRY_NAME_LEN} characters with no control characters")));
    }
    Ok(())
}

impl GeometryRef {
    pub fn new(body: impl Into<String>, entity: Entity) -> Result<GeometryRef> {
        let body = body.into();
        check_geometry_name("body", &body)?;
        if let Some(n) = entity.name() {
            check_geometry_name(entity.kind(), n)?;
        }
        Ok(GeometryRef { body, entity })
    }

    pub fn body(body: impl Into<String>) -> Result<GeometryRef> {
        GeometryRef::new(body, Entity::Body)
    }

    pub fn face(body: impl Into<String>, face: impl Into<String>) -> Result<GeometryRef> {
        GeometryRef::new(body, Entity::Face(face.into()))
    }

    /// Whether this reference (from a set) selects the tagged entity `tag`: the same body and
    /// kind, and the same name or the name of one of its pieces. A B-rep face the kernel splits
    /// into pieces (a full cylinder is two half-cylinders) carries the face's name with `#0`,
    /// `#1`, … appended; a reference to the plain name selects all of them. Edges separating the
    /// same faces are numbered `~0`, `~1`, … the same way.
    pub fn selects(&self, tag: &GeometryRef) -> bool {
        if self.body != tag.body || self.entity.kind() != tag.entity.kind() {
            return false;
        }
        match (self.entity.name(), tag.entity.name()) {
            (Some(want), Some(have)) => {
                // A face's pieces are numbered `#k`, an edge's `~k` (edges separating the same faces).
                let mark = if matches!(self.entity, Entity::Edge(_)) { '~' } else { '#' };
                have == want
                    || have
                        .strip_prefix(want)
                        .and_then(|r| r.strip_prefix(mark))
                        .is_some_and(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()))
            }
            _ => true,
        }
    }
}

impl std::fmt::Display for GeometryRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.entity.name() {
            None => write!(f, "body {}", self.body),
            Some(n) => write!(f, "{} {n} of body {}", self.entity.kind(), self.body),
        }
    }
}

/// The geometry tags of one element.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ElementTags {
    /// The entity the element fills: a body for solids, a face for shells, an edge for beams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<u32>,
    /// Face number (1-based, [`crate::ElementKind::faces`]) → the geometry face it lies on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub faces: BTreeMap<u8, u32>,
    /// Edge number (1-based, [`crate::ElementKind::edges`]) → the geometry edge it lies on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub edges: BTreeMap<u8, u32>,
}

/// Where the mesh lies on the geometry. Tags are indices into [`Classification::entities`].
///
/// It is read back from a file through [`Model`]'s deserializer, which replays every tag through
/// the checked `tag_*` methods.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Classification {
    entities: Vec<GeometryRef>,
    /// Entity → tag, so interning is a lookup.
    #[serde(skip)]
    index: BTreeMap<GeometryRef, u32>,
    elements: BTreeMap<u32, ElementTags>,
    /// Node → the geometry vertex it lies on.
    nodes: BTreeMap<u32, u32>,
}

/// A [`Classification`] as stored, before its tags are checked against the mesh.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassificationData {
    #[serde(default)]
    pub entities: Vec<GeometryRef>,
    #[serde(default)]
    pub elements: BTreeMap<u32, ElementTags>,
    #[serde(default)]
    pub nodes: BTreeMap<u32, u32>,
}

impl Classification {
    /// The tagged entities; a tag is an index into this list.
    pub fn entities(&self) -> &[GeometryRef] {
        &self.entities
    }

    pub fn entity(&self, tag: u32) -> Option<&GeometryRef> {
        self.entities.get(tag as usize)
    }

    pub fn elements(&self) -> &BTreeMap<u32, ElementTags> {
        &self.elements
    }

    pub fn nodes(&self) -> &BTreeMap<u32, u32> {
        &self.nodes
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty() && self.nodes.is_empty()
    }

    /// No entity interned and nothing tagged (left out of a model's JSON form).
    pub(crate) fn is_unused(&self) -> bool {
        self.entities.is_empty() && self.is_empty()
    }

    /// The tags of every entity `reference` selects ([`GeometryRef::selects`]).
    pub fn tags_selected_by(&self, reference: &GeometryRef) -> Vec<u32> {
        self.entities.iter().enumerate().filter(|(_, e)| reference.selects(e)).filter_map(|(i, _)| u32::try_from(i).ok()).collect()
    }

    pub(crate) fn forget_element(&mut self, label: u32) {
        self.elements.remove(&label);
    }

    /// Tag a node with a tag already interned (the caller checked both).
    pub(crate) fn tag_node_unchecked(&mut self, label: u32, tag: u32) {
        self.nodes.insert(label, tag);
    }

    pub(crate) fn forget_node(&mut self, label: u32) {
        self.nodes.remove(&label);
    }
}

impl Model {
    pub fn classification(&self) -> &Classification {
        &self.classification
    }

    /// The tag for a geometry entity, adding it on first use. Meshers call this once per entity,
    /// then tag mesh entities with the number.
    pub fn geometry_tag(&mut self, entity: &GeometryRef) -> Result<u32> {
        let c = &mut self.classification;
        if let Some(&tag) = c.index.get(entity) {
            return Ok(tag);
        }
        if c.entities.len() >= MAX_ENTITIES {
            return Err(Error::Limit(format!("a mesh is tagged with at most {MAX_ENTITIES} geometry entities")));
        }
        let tag = u32::try_from(c.entities.len()).map_err(|_| Error::Limit("too many geometry entities".into()))?;
        c.entities.push(entity.clone());
        c.index.insert(entity.clone(), tag);
        Ok(tag)
    }

    /// Check a tag names an entity of the given dimension.
    fn check_tag(&self, tag: u32, dimension: u8) -> Result<()> {
        let e = self.classification.entity(tag).ok_or_else(|| Error::NotFound(format!("geometry tag {tag}")))?;
        if e.entity.dimension() != dimension {
            return Err(Error::Invalid(format!("{e} is a {}, not a {}-dimensional entity", e.entity.kind(), dimension)));
        }
        Ok(())
    }

    fn element_kind(&self, element: u32) -> Result<crate::ElementKind> {
        Ok(self.element(element).ok_or_else(|| Error::NotFound(format!("element {element}")))?.kind)
    }

    /// Check that `number` (1-based) is one of `parts` of an element, naming `what` in the error.
    fn check_part(element: u32, kind: crate::ElementKind, what: &str, number: u8, parts: usize) -> Result<()> {
        if number == 0 || usize::from(number) > parts {
            return Err(Error::Invalid(format!("element {element} ({}) has no {what} {number}", kind.name())));
        }
        Ok(())
    }

    /// Record that an element fills a body (solids), a face (shells) or an edge (line elements).
    pub fn tag_element(&mut self, element: u32, tag: u32) -> Result<()> {
        let kind = self.element_kind(element)?;
        self.check_tag(tag, kind.dimension())?;
        self.classification.elements.entry(element).or_default().region = Some(tag);
        Ok(())
    }

    /// Record that face `face` (1-based) of an element lies on a geometry face.
    pub fn tag_element_face(&mut self, element: u32, face: u8, tag: u32) -> Result<()> {
        let kind = self.element_kind(element)?;
        Self::check_part(element, kind, "face", face, kind.faces().len())?;
        self.check_tag(tag, 2)?;
        self.classification.elements.entry(element).or_default().faces.insert(face, tag);
        Ok(())
    }

    /// Record that edge `edge` (1-based) of an element lies on a geometry edge.
    pub fn tag_element_edge(&mut self, element: u32, edge: u8, tag: u32) -> Result<()> {
        let kind = self.element_kind(element)?;
        Self::check_part(element, kind, "edge", edge, kind.edges().len())?;
        self.check_tag(tag, 1)?;
        self.classification.elements.entry(element).or_default().edges.insert(edge, tag);
        Ok(())
    }

    /// Record that a node lies on a geometry vertex.
    pub fn tag_node(&mut self, node: u32, tag: u32) -> Result<()> {
        if self.node(node).is_none() {
            return Err(Error::NotFound(format!("node {node}")));
        }
        self.check_tag(tag, 0)?;
        self.classification.nodes.insert(node, tag);
        Ok(())
    }

    /// Drop every geometry tag (before a remesh writes new ones).
    pub fn clear_geometry_tags(&mut self) {
        self.classification = Classification::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_parse_and_select_split_faces() {
        let r: GeometryRef = serde_json::from_value(serde_json::json!({"body": "Body1", "face": "F2:side"})).unwrap();
        assert_eq!(r, GeometryRef::face("Body1", "F2:side").unwrap());
        assert_eq!(serde_json::to_value(&r).unwrap(), serde_json::json!({"body": "Body1", "face": "F2:side"}));
        assert!(r.selects(&GeometryRef::face("Body1", "F2:side").unwrap()));
        assert!(r.selects(&GeometryRef::face("Body1", "F2:side#1").unwrap()));
        assert!(!r.selects(&GeometryRef::face("Body1", "F2:side#").unwrap()));
        assert!(!r.selects(&GeometryRef::face("Body1", "F2:sides").unwrap()));
        assert!(!r.selects(&GeometryRef::face("Body2", "F2:side").unwrap()));
        assert!(!r.selects(&GeometryRef::new("Body1", Entity::Edge("F2:side".into())).unwrap()));
        assert!(GeometryRef::body("Body1").unwrap().selects(&GeometryRef::body("Body1").unwrap()));
        let edge = |n: &str| GeometryRef::new("Body1", Entity::Edge(n.into())).unwrap();
        assert!(edge("A|B").selects(&edge("A|B~1")));
        assert!(!edge("A|B").selects(&edge("A|B#1")));
        assert!(!edge("A|B~0").selects(&edge("A|B~1")));

        for bad in [
            serde_json::json!({"body": ""}),
            serde_json::json!({"body": "B", "face": "a", "edge": "b"}),
            serde_json::json!({"body": "B", "face": ""}),
            serde_json::json!({"body": "B", "face": "a\nb"}),
            serde_json::json!({"body": "x".repeat(MAX_GEOMETRY_NAME_LEN + 1)}),
            serde_json::json!({"body": "B", "colour": "red"}),
            serde_json::json!({"face": "a"}),
        ] {
            assert!(serde_json::from_value::<GeometryRef>(bad.clone()).is_err(), "{bad}");
        }
    }
}
