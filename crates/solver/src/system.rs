//! From the resolved model to the discrete system: the solid elements with their materials and
//! formulations, the degrees of freedom, prescribed values and the load vector of a step.

use modelcraft_fem::element::{Formulation, Solid};
use modelcraft_fem::{Elasticity, elasticity, face};
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

/// The analysed mesh: the nodes that carry degrees of freedom (those of the solid elements),
/// sorted by label, and the elements sorted by label.
#[derive(Debug, Clone)]
pub struct Mesh {
    pub labels: Vec<u32>,
    pub xyz: Vec<[f64; 3]>,
    pub elements: Vec<Elem>,
}

impl Mesh {
    pub fn node(&self, label: u32) -> Option<usize> {
        self.labels.binary_search(&label).ok()
    }

    pub fn element(&self, label: u32) -> Option<&Elem> {
        self.elements.binary_search_by_key(&label, |e| e.label).ok().and_then(|i| self.elements.get(i))
    }

    pub fn dofs(&self) -> usize {
        3 * self.labels.len()
    }

    /// Build it from the model and its resolved view, refusing what the solver cannot represent.
    pub fn new(model: &Model, resolved: &Resolved, max_dofs: usize) -> Result<Mesh> {
        if let Some(name) = model.couplings().keys().next() {
            return Err(Error::NotSupported(format!(
                "couplings (coupling `{name}`); the solver handles solid elements without reference points so far"
            )));
        }
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
        if 3 * labels.len() > max_dofs {
            return Err(Error::Limit(format!("the model has {} degrees of freedom; the limit is {max_dofs}", 3 * labels.len())));
        }
        let xyz: Vec<[f64; 3]> = labels
            .iter()
            .map(|l| model.node(*l).map(|n| n.xyz).ok_or_else(|| Error::Invalid(format!("node {l} is missing"))))
            .collect::<Result<_>>()?;
        let mut mesh = Mesh { labels, xyz, elements: Vec::with_capacity(elements.len()) };
        for (label, kind, nodes, formulation, d, density) in elements {
            let idx: Vec<usize> = nodes.iter().filter_map(|n| mesh.node(*n)).collect();
            let exyz: Vec<[f64; 3]> = idx.iter().filter_map(|i| mesh.xyz.get(*i).copied()).collect();
            mesh.elements.push(Elem { label, kind, nodes: idx, xyz: exyz, formulation, d, density });
        }
        Ok(mesh)
    }
}

/// The prescribed value of every degree of freedom (`None` = free) under a step's boundary
/// conditions. Rotational degrees of freedom do not exist on solid nodes and are ignored.
pub fn prescribed(model: &Model, resolved: &Resolved, mesh: &Mesh, step: &Step, values: bool) -> Result<Vec<Option<f64>>> {
    let mut out: Vec<Option<f64>> = vec![None; mesh.dofs()];
    for name in &step.boundary_conditions {
        let bc = model.boundary_condition(name).ok_or_else(|| Error::Invalid(format!("boundary condition `{name}` is missing")))?;
        if let Target::ReferencePoint(rp) = bc.target() {
            return Err(Error::NotSupported(format!("boundary conditions on reference points (`{name}` on `{rp}`)")));
        }
        for &label in resolved.target_nodes(bc.target()) {
            let Some(node) = mesh.node(label) else { continue };
            for dof in 1..=3u8 {
                let Some(v) = bc.value(dof) else { continue };
                let v = if values { v } else { 0.0 };
                let slot = out.get_mut(3 * node + usize::from(dof - 1)).ok_or_else(|| Error::Internal("dof index".into()))?;
                match *slot {
                    Some(old) if old != v => {
                        return Err(Error::Invalid(format!(
                            "node {label} direction {dof} is prescribed both {old} and {v} (boundary condition `{name}` and another)"
                        )));
                    }
                    _ => *slot = Some(v),
                }
            }
        }
    }
    Ok(out)
}

fn add(f: &mut [f64], node: usize, v: [f64; 3]) {
    for (k, c) in v.iter().enumerate() {
        if let Some(x) = f.get_mut(3 * node + k) {
            *x += c;
        }
    }
}

/// The load vector of a static step: nodal forces and tractions as the model distributes them,
/// consistent pressure loads, gravity and body forces.
pub fn loads(model: &Model, resolved: &Resolved, mesh: &Mesh, step: &Step) -> Result<Vec<f64>> {
    let mut f = vec![0.0; mesh.dofs()];
    for name in &step.loads {
        let load = model.load(name).ok_or_else(|| Error::Invalid(format!("load `{name}` is missing")))?;
        match load {
            Load::Force { target: Target::ReferencePoint(rp), .. } => {
                return Err(Error::NotSupported(format!("loads on reference points (`{name}` on `{rp}`)")));
            }
            Load::Force { .. } | Load::Traction { .. } => {
                for (label, v) in resolved.nodal_loads.get(name).map(Vec::as_slice).unwrap_or_default() {
                    let node = mesh.node(*label).ok_or_else(|| Error::Invalid(format!("load `{name}`: node {label} belongs to no solid element")))?;
                    add(&mut f, node, *v);
                }
            }
            Load::Moment { .. } => {
                return Err(Error::NotSupported(format!("moments (`{name}`): solid nodes have no rotations; a coupled reference point is needed")));
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
                        add(&mut f, *i, v);
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
                    body(&mut f, e, acceleration.map(|a| rho * a))?;
                }
            }
            Load::BodyForce { set, vector } => {
                for label in resolved.sets.get(set).map(|r| r.elements.as_slice()).unwrap_or_default() {
                    let e = mesh.element(*label).ok_or_else(|| Error::Invalid(format!("load `{name}`: element {label} is not a solid element")))?;
                    body(&mut f, e, *vector)?;
                }
            }
        }
    }
    Ok(f)
}

fn body(f: &mut [f64], e: &Elem, b: [f64; 3]) -> Result<()> {
    let forces = e.solid()?.body_force(b).map_err(|err| Error::element(e.label, err))?;
    for (node, v) in e.nodes.iter().zip(forces) {
        add(f, *node, v);
    }
    Ok(())
}

/// Equation numbers of the free degrees of freedom, following a node order.
pub fn equations(order: &[usize], prescribed: &[Option<f64>]) -> (Vec<Option<usize>>, usize) {
    let mut map = vec![None; prescribed.len()];
    let mut next = 0;
    for &node in order {
        for c in 0..3 {
            let dof = 3 * node + c;
            if prescribed.get(dof).is_some_and(Option::is_none) {
                if let Some(m) = map.get_mut(dof) {
                    *m = Some(next);
                }
                next += 1;
            }
        }
    }
    (map, next)
}
