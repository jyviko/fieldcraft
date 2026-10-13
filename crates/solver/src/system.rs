//! From the resolved model to the discrete system: the solid elements with their materials and
//! formulations, the coupled reference points, the degrees of freedom, prescribed values and the
//! load vector of a step.
//!
//! Degrees of freedom ([`DofLayout`]): the kinds each mesh node's elements declare (three
//! translations for solids), nodes by label, then six per coupled reference point (points by
//! name): translations along X, Y, Z and rotations about them.

use modelcraft_fem::element::{Formulation, Solid};
use modelcraft_fem::{DofKind, DofLayout, DofSet, Elasticity, elasticity, face};
use std::collections::BTreeSet;

use modelcraft_model::{ElementKind, Load, Model, Resolved, SectionKind, Step, Target};

use crate::{Error, Result};

/// A solid element ready for the kernels.
#[derive(Debug, Clone)]
pub struct Elem {
    pub label: u32,
    pub kind: ElementKind,
    /// Indices into [`Mesh::labels`].
    pub nodes: Vec<usize>,
    pub xyz: Vec<[f64; 3]>,
    pub formulation: Formulation,
    pub d: Elasticity,
    pub density: Option<f64>,
}

impl Elem {
    pub fn solid(&self) -> Result<Solid<'_>> {
        Solid::new(self.kind, &self.xyz, self.formulation).map_err(|e| Error::element(self.label, e))
    }
}

/// A reference point some coupling ties to the mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub name: String,
    pub label: u32,
    pub xyz: [f64; 3],
}

/// The analysed mesh: the nodes that carry degrees of freedom (those of the solid elements),
/// sorted by label, the elements sorted by label, the coupled reference points by name, and the
/// numbering of their degrees of freedom (mesh node `n` is layout slot `n`, reference point `k`
/// is [`DofLayout::point_slot`]`(k)`).
#[derive(Debug, Clone)]
pub struct Mesh {
    pub labels: Vec<u32>,
    pub xyz: Vec<[f64; 3]>,
    pub elements: Vec<Elem>,
    pub points: Vec<Point>,
    pub layout: DofLayout,
}

impl Mesh {
    pub fn node(&self, label: u32) -> Option<usize> {
        self.labels.binary_search(&label).ok()
    }

    pub fn element(&self, label: u32) -> Option<&Elem> {
        self.elements.binary_search_by_key(&label, |e| e.label).ok().and_then(|i| self.elements.get(i))
    }

    pub fn point(&self, name: &str) -> Option<usize> {
        self.points.binary_search_by(|p| p.name.as_str().cmp(name)).ok()
    }

    /// The degrees of freedom of the mesh nodes; the reference points' follow them.
    pub fn mesh_dofs(&self) -> usize {
        self.layout.node_dofs()
    }

    /// Every degree of freedom.
    pub fn dofs(&self) -> usize {
        self.layout.dofs()
    }

    /// The degree of freedom of `kind` at layout slot `slot` (a mesh node or reference point).
    pub fn dof(&self, slot: usize, kind: DofKind) -> Result<usize> {
        self.layout.dof(slot, kind).ok_or_else(|| Error::Internal(format!("slot {slot} has no {kind:?} degree of freedom")))
    }

    /// The kind of a degree of freedom.
    pub fn kind(&self, dof: usize) -> Option<DofKind> {
        self.layout.locate(dof).map(|(_, kind)| kind)
    }

    /// What a degree of freedom is, in the user's terms.
    pub fn describe_dof(&self, dof: usize) -> String {
        let Some((slot, kind)) = self.layout.locate(dof) else { return format!("degree of freedom {dof}") };
        match self.labels.get(slot) {
            Some(label) => format!("node {label} in {}", kind.describe()),
            None => {
                let name = self.points.get(slot - self.labels.len()).map_or("?", |p| p.name.as_str());
                format!("reference point `{name}` in {}", kind.describe())
            }
        }
    }

    /// Build it from the model and its resolved view, refusing what the solver cannot represent.
    pub fn new(model: &Model, resolved: &Resolved, max_dofs: usize) -> Result<Mesh> {
        let mut elements = Vec::new();
        for (name, section) in model.sections() {
            if !matches!(section.kind, SectionKind::Solid {}) {
                return Err(Error::NotSupported(format!(
                    "{} sections (section `{name}`); the solver has solid elements so far",
                    section.kind.name()
                )));
            }
            let material = model
                .material(&section.material)
                .ok_or_else(|| Error::Invalid(format!("section `{name}`: material `{}` is missing", section.material)))?;
            let elastic =
                material.elastic.as_ref().ok_or_else(|| Error::Invalid(format!("material `{}` has no elastic constants", section.material)))?;
            let d = elasticity(elastic).map_err(|e| Error::Invalid(format!("material `{}`: {e}", section.material)))?;
            let set = resolved.sets.get(&section.set).ok_or_else(|| Error::Invalid(format!("set `{}` is not resolved", section.set)))?;
            for &label in &set.elements {
                let el = model.element(label).ok_or_else(|| Error::Invalid(format!("element {label} is missing")))?;
                let formulation = Formulation::from_section(el.kind, &section.formulation).map_err(|e| Error::element(label, e))?;
                elements.push((label, el.kind, el.nodes.clone(), formulation, d, material.density));
            }
        }
        elements.sort_by_key(|e| e.0);
        let mut labels: Vec<u32> = elements.iter().flat_map(|e| e.2.iter().copied()).collect();
        labels.sort_unstable();
        labels.dedup();
        let coupled: BTreeSet<&str> = model.couplings().values().map(|c| c.reference_point.as_str()).collect();
        let points: Vec<Point> = coupled
            .into_iter()
            .map(|name| {
                let p = model.reference_point(name).ok_or_else(|| Error::Invalid(format!("reference point `{name}` is missing")))?;
                Ok(Point { name: name.to_string(), label: p.label, xyz: p.xyz })
            })
            .collect::<Result<_>>()?;
        let mut layout = DofLayout::builder();
        for e in &elements {
            layout.element(&e.2, e.3.dofs());
        }
        for p in &points {
            layout.point(p.label, DofSet::STRUCTURAL);
        }
        let dofs = layout.dofs();
        if dofs > max_dofs {
            return Err(Error::Limit(format!("the model has {dofs} degrees of freedom; the limit is {max_dofs}")));
        }
        let xyz: Vec<[f64; 3]> = labels
            .iter()
            .map(|l| model.node(*l).map(|n| n.xyz).ok_or_else(|| Error::Invalid(format!("node {l} is missing"))))
            .collect::<Result<_>>()?;
        let mut mesh = Mesh { labels, xyz, elements: Vec::with_capacity(elements.len()), points, layout: layout.build() };
        for (label, kind, nodes, formulation, d, density) in elements {
            let idx: Vec<usize> = nodes.iter().filter_map(|n| mesh.node(*n)).collect();
            let exyz: Vec<[f64; 3]> = idx.iter().filter_map(|i| mesh.xyz.get(*i).copied()).collect();
            mesh.elements.push(Elem { label, kind, nodes: idx, xyz: exyz, formulation, d, density });
        }
        Ok(mesh)
    }
}

/// The prescribed value of every degree of freedom (`None` = free) under a step's boundary
/// conditions; `values: false` holds them at zero (frequency steps). Rotations of solid nodes do
/// not exist and are ignored, like the directions of a reference point no coupling ties
/// (`active`); a boundary condition on a reference point that no coupling ties to the mesh holds
/// nothing and is reported in `warnings`.
pub fn prescribed(
    model: &Model,
    resolved: &Resolved,
    mesh: &Mesh,
    step: &Step,
    values: bool,
    active: &[bool],
    warnings: &mut Vec<String>,
) -> Result<Vec<Option<f64>>> {
    let mut out: Vec<Option<f64>> = vec![None; mesh.dofs()];
    for name in &step.boundary_conditions {
        let bc = model.boundary_condition(name).ok_or_else(|| Error::Invalid(format!("boundary condition `{name}` is missing")))?;
        if let Target::ReferencePoint(rp) = bc.target()
            && mesh.point(rp).is_none()
        {
            warnings
                .push(format!("boundary condition `{name}` acts on reference point `{rp}`, which no coupling ties to the mesh; it holds nothing"));
            continue;
        }
        for slot in resolved.target_nodes(bc.target()).iter().filter_map(|l| mesh.layout.slot(*l)) {
            for kind in mesh.layout.kinds(slot).iter() {
                let Some(v) = bc.value(kind.number()) else { continue };
                let index = mesh.dof(slot, kind)?;
                if !active.get(index).copied().unwrap_or(false) {
                    continue;
                }
                let v = if values { v } else { 0.0 };
                let slot = out.get_mut(index).ok_or_else(|| Error::Internal("dof index".into()))?;
                match *slot {
                    Some(old) if old != v => {
                        return Err(Error::Invalid(format!(
                            "{} is prescribed both {old} and {v} (boundary condition `{name}` and another)",
                            mesh.describe_dof(index)
                        )));
                    }
                    _ => *slot = Some(v),
                }
            }
        }
    }
    Ok(out)
}

/// Add a vector to the three degrees of freedom `dofs`.
fn add(f: &mut [f64], dofs: [usize; 3], v: [f64; 3]) {
    for (d, c) in dofs.iter().zip(v) {
        if let Some(x) = f.get_mut(*d) {
            *x += c;
        }
    }
}

/// The translations (or rotations) of layout slot `slot`, if it has them.
fn vector_dofs(mesh: &Mesh, slot: usize, rotation: bool) -> Option<[usize; 3]> {
    let kind = |c| if rotation { DofKind::rotation(c) } else { DofKind::translation(c) };
    let [a, b, c] = [0, 1, 2].map(|c| kind(c).and_then(|k| mesh.layout.dof(slot, k)));
    Some([a?, b?, c?])
}

/// The translations of mesh node `node`.
fn translations(mesh: &Mesh, node: usize) -> Result<[usize; 3]> {
    vector_dofs(mesh, node, false).ok_or_else(|| Error::Internal(format!("mesh node {node} has no translations")))
}

/// The load vector of a static step: nodal forces and tractions as the model distributes them,
/// forces and moments on coupled reference points, consistent pressure loads, gravity and body
/// forces.
pub fn loads(model: &Model, resolved: &Resolved, mesh: &Mesh, step: &Step, active: &[bool]) -> Result<Vec<f64>> {
    let mut f = vec![0.0; mesh.dofs()];
    for name in &step.loads {
        let load = model.load(name).ok_or_else(|| Error::Invalid(format!("load `{name}` is missing")))?;
        match load {
            Load::Force { .. } | Load::Moment { .. } | Load::Traction { .. } => {
                if let Some(rp) = load.reference_point()
                    && mesh.point(rp).is_none()
                {
                    return Err(Error::Invalid(format!("load `{name}` acts on reference point `{rp}`, which no coupling ties to the mesh")));
                }
                // Moments act on the rotations, which only reference points have.
                let rotation = matches!(load, Load::Moment { .. });
                for (label, v) in resolved.nodal_loads.get(name).map(Vec::as_slice).unwrap_or_default() {
                    let slot =
                        mesh.layout.slot(*label).ok_or_else(|| Error::Invalid(format!("load `{name}`: node {label} belongs to no solid element")))?;
                    let dofs = vector_dofs(mesh, slot, rotation).ok_or_else(|| {
                        Error::Invalid(format!("moment `{name}`: solid nodes have no rotations; apply it to a coupled reference point"))
                    })?;
                    for (&dof, x) in dofs.iter().zip(v) {
                        if *x != 0.0 && !active.get(dof).copied().unwrap_or(false) {
                            return Err(Error::Invalid(format!(
                                "load `{name}`: {} is not coupled to the mesh (its coupling leaves that direction free)",
                                mesh.describe_dof(dof)
                            )));
                        }
                    }
                    add(&mut f, dofs, *v);
                }
            }
            Load::Pressure { set, magnitude } => {
                let faces = resolved.sets.get(set).map(|s| s.faces.as_slice()).unwrap_or_default();
                for fc in faces {
                    let labels = model
                        .face_nodes(*fc)
                        .ok_or_else(|| Error::Invalid(format!("load `{name}`: element {} face {} is missing", fc.element, fc.face)))?;
                    let idx: Vec<usize> = labels
                        .iter()
                        .map(|l| mesh.node(*l).ok_or_else(|| Error::Invalid(format!("load `{name}`: node {l} belongs to no solid element"))))
                        .collect::<Result<_>>()?;
                    let xyz: Vec<[f64; 3]> = idx.iter().filter_map(|i| mesh.xyz.get(*i).copied()).collect();
                    let forces = face::pressure(&xyz, *magnitude).map_err(|e| Error::element(fc.element, e))?;
                    for (i, v) in idx.iter().zip(forces) {
                        add(&mut f, translations(mesh, *i)?, v);
                    }
                }
            }
            Load::Gravity { acceleration, set } => {
                let chosen: Option<&[u32]> = set.as_ref().map(|s| resolved.sets.get(s).map(|r| r.elements.as_slice()).unwrap_or_default());
                for e in &mesh.elements {
                    if chosen.is_some_and(|c| c.binary_search(&e.label).is_err()) {
                        continue;
                    }
                    let rho = e.density.ok_or_else(|| Error::Invalid(format!("load `{name}`: element {} has no density", e.label)))?;
                    body(&mut f, mesh, e, acceleration.map(|a| rho * a))?;
                }
            }
            Load::BodyForce { set, vector } => {
                for label in resolved.sets.get(set).map(|r| r.elements.as_slice()).unwrap_or_default() {
                    let e = mesh.element(*label).ok_or_else(|| Error::Invalid(format!("load `{name}`: element {label} is not a solid element")))?;
                    body(&mut f, mesh, e, *vector)?;
                }
            }
        }
    }
    Ok(f)
}

fn body(f: &mut [f64], mesh: &Mesh, e: &Elem, b: [f64; 3]) -> Result<()> {
    let forces = e.solid()?.body_force(b).map_err(|err| Error::element(e.label, err))?;
    for (node, v) in e.nodes.iter().zip(forces) {
        add(f, translations(mesh, *node)?, v);
    }
    Ok(())
}
