//! From the resolved model to the explicit system: elements with their materials and stable
//! lengths, lumped masses, prescribed velocities, rigid walls, initial velocities and the load
//! vector of an explicit dynamic step.

use modelcraft_fem::material::Elasticity;
use modelcraft_fem::uniform::{Hex8, Tet4, modulus_bound};
use modelcraft_fem::{DofKind, DofLayout, DofSet, face};
use modelcraft_model::{Elastic, ElementKind, ExplicitDynamic, HourglassKind, Integration, Load, Model, Resolved, SectionKind, Step, Target};

use crate::elements::add_at;
use crate::{Error, Result};

/// Degrees of freedom per node: the three translations, node-major (see [`System`]).
pub const NODE_DOFS: usize = 3;

/// Most nodes whose histories a step records.
pub const MAX_HISTORY_NODES: usize = 1000;

/// A material as the elements use it.
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub elasticity: Elasticity,
    pub density: f64,
    /// Dilatational wave speed √(M/ρ), with M the largest normal stiffness (λ + 2μ when
    /// isotropic).
    pub wave_speed: f64,
    /// The bound of [`modelcraft_fem::uniform::modulus_bound`].
    pub modulus: f64,
    /// Bulk modulus, for the nodal-pressure tetrahedron (isotropic materials).
    pub bulk: Option<f64>,
}

/// A one-point hexahedron.
#[derive(Debug, Clone)]
pub struct Hex {
    pub label: u32,
    /// Position in [`System::elements`].
    pub index: usize,
    pub nodes: [usize; 8],
    pub material: usize,
    pub kinematics: Hex8,
    pub gamma: [[f64; 8]; 4],
    /// The hourglass coefficient: a damping (viscous) or a stiffness (stiffness form), see
    /// [`crate::elements`].
    pub hourglass: f64,
    /// The element's stable length: its critical increment without bulk viscosity is
    /// `length / wave_speed`.
    pub length: f64,
}

/// A nodal-pressure tetrahedron.
#[derive(Debug, Clone)]
pub struct Tet {
    pub label: u32,
    pub index: usize,
    pub nodes: [usize; 4],
    pub material: usize,
    pub kinematics: Tet4,
    pub length: f64,
}

/// A fixed, frictionless rigid plane.
#[derive(Debug, Clone)]
pub struct Wall {
    pub point: [f64; 3],
    /// Unit normal, towards the side the nodes stay on.
    pub normal: [f64; 3],
    pub nodes: Vec<usize>,
}

/// The discrete system of an explicit dynamic step.
///
/// Its elements are solids, so every node carries the three translations and the layout is
/// node-major: degree of freedom 3 n + axis, which the integrator's per-node loops rely on
/// (checked when the system is built).
#[derive(Debug, Clone)]
pub struct System {
    /// Node labels with degrees of freedom (those of the elements), sorted.
    pub labels: Vec<u32>,
    pub layout: DofLayout,
    pub xyz: Vec<[f64; 3]>,
    /// Element labels, sorted.
    pub elements: Vec<u32>,
    pub materials: Vec<Material>,
    pub hexes: Vec<Hex>,
    pub tets: Vec<Tet>,
    /// Row-sum lumped mass per node.
    pub mass: Vec<f64>,
    /// Prescribed velocity per degree of freedom (`None` = free).
    pub prescribed: Vec<Option<f64>>,
    pub walls: Vec<Wall>,
    /// Velocity per degree of freedom at the start (prescribed velocities included).
    pub initial_velocity: Vec<f64>,
    /// Constant external force per degree of freedom.
    pub external: Vec<f64>,
    /// Nodes whose histories are recorded.
    pub history_nodes: Vec<usize>,
    pub hourglass: HourglassKind,
    pub warnings: Vec<String>,
}

fn invalid(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

/// A material's constants for the explicit elements.
fn material(model: &Model, name: &str) -> Result<Material> {
    let m = model.material(name).ok_or_else(|| invalid(format!("material `{name}` is missing")))?;
    let elastic = m.elastic.as_ref().ok_or_else(|| invalid(format!("material `{name}` has no elastic constants")))?;
    let density = m
        .density
        .filter(|d| *d > 0.0 && d.is_finite())
        .ok_or_else(|| invalid(format!("material `{name}` has no density, and explicit dynamics needs one")))?;
    let d = modelcraft_fem::elasticity(elastic).map_err(|e| invalid(format!("material `{name}`: {e}")))?;
    let stiffest = d[0][0].max(d[1][1]).max(d[2][2]);
    let bulk = match *elastic {
        Elastic::Isotropic { young, poisson } => Some(young / (3.0 * (1.0 - 2.0 * poisson))),
        Elastic::EngineeringConstants { .. } => None,
    };
    Ok(Material {
        name: name.to_string(),
        elasticity: d,
        density,
        wave_speed: (stiffest / density).sqrt(),
        modulus: modulus_bound(elastic, &d),
        bulk,
    })
}

/// The largest absolute row sum of the hourglass vectors' Gram matrix Σ_I γ_αI γ_βI / v_I (an
/// upper bound of its largest eigenvalue, exact for parallelepipeds where the γ_α are
/// orthogonal).
fn hourglass_gram_bound(gamma: &[[f64; 8]; 4], nodal_volume: &[f64; 8]) -> f64 {
    let mut worst: f64 = 0.0;
    for a in gamma {
        let row: f64 = gamma.iter().map(|b| a.iter().zip(b).zip(nodal_volume).map(|((x, y), v)| x * y / v).sum::<f64>().abs()).sum();
        worst = worst.max(row);
    }
    worst
}

/// The section's elements of a kind the explicit solver has, or why not.
fn check_formulation(kind: ElementKind, section_name: &str, f: &modelcraft_model::Formulation) -> Result<()> {
    let refuse = |why: &str| Err(Error::NotSupported(format!("section `{section_name}`: {why}")));
    if f.hybrid {
        return refuse("hybrid elements in explicit dynamics");
    }
    match kind {
        ElementKind::Hex8 if f.incompatible_modes || f.integration == Integration::Full => refuse(
            "fully integrated or incompatible-mode hex8 in explicit dynamics (explicit hex8 elements are one-point with hourglass control: set the section's integration to reduced)",
        ),
        ElementKind::Hex8 => Ok(()),
        ElementKind::Tet4 if f.integration == Integration::Reduced => refuse("reduced integration of tet4 elements"),
        ElementKind::Tet4 => Ok(()),
        other => refuse(&format!("{} elements in explicit dynamics (it has one-point hex8 and nodal-pressure tet4 so far)", other.name())),
    }
}

impl System {
    pub fn new(model: &Model, resolved: &Resolved, step: &Step, settings: &ExplicitDynamic, max_dofs: usize) -> Result<System> {
        if let Some(name) = model.couplings().keys().next() {
            return Err(Error::NotSupported(format!("couplings (coupling `{name}`) in explicit dynamics")));
        }
        // Elements by label, with their section's material.
        let mut materials: Vec<Material> = Vec::new();
        let mut chosen: Vec<(u32, ElementKind, Vec<u32>, usize)> = Vec::new();
        for (name, section) in model.sections() {
            if !matches!(section.kind, SectionKind::Solid {}) {
                return Err(Error::NotSupported(format!(
                    "{} sections (section `{name}`); explicit dynamics has solid elements so far",
                    section.kind.name()
                )));
            }
            let mi = match materials.iter().position(|m| m.name == section.material) {
                Some(i) => i,
                None => {
                    materials.push(material(model, &section.material)?);
                    materials.len() - 1
                }
            };
            let set = resolved.sets.get(&section.set).ok_or_else(|| invalid(format!("set `{}` is not resolved", section.set)))?;
            for &label in &set.elements {
                let el = model.element(label).ok_or_else(|| invalid(format!("element {label} is missing")))?;
                check_formulation(el.kind, name, &section.formulation)?;
                if el.kind == ElementKind::Tet4 && materials.get(mi).is_some_and(|m| m.bulk.is_none()) {
                    return Err(Error::NotSupported(format!(
                        "nodal-pressure tet4 elements of the orthotropic material `{}` (section `{name}`)",
                        section.material
                    )));
                }
                chosen.push((label, el.kind, el.nodes.clone(), mi));
            }
        }
        chosen.sort_by_key(|e| e.0);
        let mut labels: Vec<u32> = chosen.iter().flat_map(|e| e.2.iter().copied()).collect();
        labels.sort_unstable();
        labels.dedup();
        let layout = DofLayout::uniform(labels.iter().copied(), DofSet::TRANSLATIONS);
        if layout.dofs() > max_dofs {
            return Err(Error::Limit(format!("the model has {} degrees of freedom; the limit is {max_dofs}", layout.dofs())));
        }
        if layout.stride() != Some(NODE_DOFS) {
            return Err(Error::Internal("explicit dynamics needs three translations per node".into()));
        }
        if labels.is_empty() {
            return Err(invalid("the model has no elements to analyse"));
        }
        let xyz: Vec<[f64; 3]> =
            labels.iter().map(|l| model.node(*l).map(|n| n.xyz).ok_or_else(|| invalid(format!("node {l} is missing")))).collect::<Result<_>>()?;
        let (n, dofs) = (labels.len(), layout.dofs());
        let mut sys = System {
            layout,
            elements: chosen.iter().map(|e| e.0).collect(),
            materials,
            hexes: Vec::new(),
            tets: Vec::new(),
            mass: vec![0.0; n],
            prescribed: vec![None; dofs],
            walls: Vec::new(),
            initial_velocity: vec![0.0; dofs],
            external: vec![0.0; dofs],
            history_nodes: Vec::new(),
            hourglass: settings.hourglass.kind,
            warnings: resolved.warnings.iter().map(ToString::to_string).collect(),
            labels,
            xyz,
        };
        for (index, (label, kind, nodes, mi)) in chosen.into_iter().enumerate() {
            let idx: Vec<usize> = nodes.iter().map(|l| sys.node(*l).ok_or_else(|| Error::Internal(format!("node {l}")))).collect::<Result<_>>()?;
            let mat = sys.materials.get(mi).ok_or_else(|| Error::Internal("material index".into()))?;
            let (rho, c) = (mat.density, mat.wave_speed);
            let at = |e| Error::element(label, e);
            match kind {
                ElementKind::Hex8 => {
                    let nodes: [usize; 8] = idx.try_into().map_err(|_| invalid(format!("element {label} does not have 8 nodes")))?;
                    let x = nodes.map(|i| sys.xyz.get(i).copied().unwrap_or_default());
                    let k = Hex8::hex8(&x).map_err(at)?;
                    let gamma = k.hourglass_vectors(&x);
                    // h: the edge of the cube with the element's volume and gradient norm.
                    let h = k.volume / (2.0 * k.gradient_norm()).sqrt();
                    let kappa = settings.hourglass.coefficient;
                    let mut omega2 = k.frequency_bound(mat.modulus, rho);
                    let hourglass = match settings.hourglass.kind {
                        HourglassKind::Viscous => kappa * rho * c * h * h / 16.0,
                        HourglassKind::Stiffness => {
                            let stiffness = kappa * rho * c * c * h / 16.0;
                            omega2 += stiffness * hourglass_gram_bound(&gamma, &k.nodal_volume) / rho;
                            stiffness
                        }
                    };
                    sys.lump(&nodes, &k.nodal_volume, rho);
                    sys.hexes.push(Hex { label, index, nodes, material: mi, kinematics: k, gamma, hourglass, length: 2.0 * c / omega2.sqrt() });
                }
                ElementKind::Tet4 => {
                    let nodes: [usize; 4] = idx.try_into().map_err(|_| invalid(format!("element {label} does not have 4 nodes")))?;
                    let x = nodes.map(|i| sys.xyz.get(i).copied().unwrap_or_default());
                    let k = Tet4::tet4(&x).map_err(at)?;
                    let omega2 = k.frequency_bound(mat.modulus, rho);
                    sys.lump(&nodes, &k.nodal_volume, rho);
                    sys.tets.push(Tet { label, index, nodes, material: mi, kinematics: k, length: 2.0 * c / omega2.sqrt() });
                }
                other => return Err(Error::Internal(format!("{} element kind", other.name()))),
            }
        }
        sys.prescribe(model, resolved, step, settings.end_time)?;
        sys.load(model, resolved, step)?;
        sys.conditions(resolved, settings)?;
        Ok(sys)
    }

    /// Add an element's row-sum lumped masses.
    fn lump(&mut self, nodes: &[usize], nodal_volume: &[f64], density: f64) {
        for (node, v) in nodes.iter().zip(nodal_volume) {
            if let Some(m) = self.mass.get_mut(*node) {
                *m += density * v;
            }
        }
    }

    pub fn node(&self, label: u32) -> Option<usize> {
        self.labels.binary_search(&label).ok()
    }

    pub fn dofs(&self) -> usize {
        self.layout.dofs()
    }

    /// Prescribed velocities from the step's boundary conditions: zero for held degrees of
    /// freedom, value / end time for a displacement reached over the step.
    fn prescribe(&mut self, model: &Model, resolved: &Resolved, step: &Step, end_time: f64) -> Result<()> {
        for name in &step.boundary_conditions {
            let bc = model.boundary_condition(name).ok_or_else(|| invalid(format!("boundary condition `{name}` is missing")))?;
            if let Target::ReferencePoint(rp) = bc.target() {
                return Err(Error::NotSupported(format!("boundary conditions on reference points (`{name}` on `{rp}`)")));
            }
            for &label in resolved.target_nodes(bc.target()) {
                let Some(node) = self.node(label) else { continue };
                for kind in self.layout.kinds(node).iter() {
                    let dof = kind.number();
                    let Some(value) = bc.value(dof) else { continue };
                    let velocity = value / end_time;
                    if !velocity.is_finite() {
                        return Err(invalid(format!("boundary condition `{name}`: the velocity {value} / {end_time} is not finite")));
                    }
                    let index = self.layout.dof(node, kind).ok_or_else(|| Error::Internal("dof index".into()))?;
                    let slot = self.prescribed.get_mut(index).ok_or_else(|| Error::Internal("dof index".into()))?;
                    match *slot {
                        Some(old) if old != velocity => {
                            return Err(invalid(format!(
                                "node {label} direction {dof} moves at both {old} and {velocity} (boundary condition `{name}` and another)"
                            )));
                        }
                        _ => *slot = Some(velocity),
                    }
                }
            }
        }
        Ok(())
    }

    /// The constant load vector: forces and tractions as the model distributes them, consistent
    /// pressure loads, gravity and body forces with the lumped (row-sum) weights.
    fn load(&mut self, model: &Model, resolved: &Resolved, step: &Step) -> Result<()> {
        let mut f = std::mem::take(&mut self.external);
        for name in &step.loads {
            let load = model.load(name).ok_or_else(|| invalid(format!("load `{name}` is missing")))?;
            let off_mesh = |l: u32| invalid(format!("load `{name}`: node {l} belongs to no element"));
            match load {
                Load::Force { target: Target::ReferencePoint(rp), .. } => {
                    return Err(Error::NotSupported(format!("loads on reference points (`{name}` on `{rp}`)")));
                }
                Load::Moment { .. } => {
                    return Err(Error::NotSupported(format!("moments (`{name}`): solid nodes have no rotations")));
                }
                Load::Force { .. } | Load::Traction { .. } => {
                    for (label, v) in resolved.nodal_loads.get(name).map(Vec::as_slice).unwrap_or_default() {
                        let node = self.node(*label).ok_or_else(|| off_mesh(*label))?;
                        add_at(&mut f, node, v);
                    }
                }
                Load::Pressure { set, magnitude } => {
                    for fc in resolved.sets.get(set).map(|s| s.faces.as_slice()).unwrap_or_default() {
                        let labels = model
                            .face_nodes(*fc)
                            .ok_or_else(|| invalid(format!("load `{name}`: element {} face {} is missing", fc.element, fc.face)))?;
                        let idx: Vec<usize> = labels.iter().map(|l| self.node(*l).ok_or_else(|| off_mesh(*l))).collect::<Result<_>>()?;
                        let xyz: Vec<[f64; 3]> = idx.iter().filter_map(|i| self.xyz.get(*i).copied()).collect();
                        let forces = face::pressure(&xyz, *magnitude).map_err(|e| Error::element(fc.element, e))?;
                        for (i, v) in idx.iter().zip(forces) {
                            add_at(&mut f, *i, &v);
                        }
                    }
                }
                Load::Gravity { acceleration, set } => {
                    let chosen: Option<&[u32]> = set.as_ref().map(|s| resolved.sets.get(s).map(|r| r.elements.as_slice()).unwrap_or_default());
                    self.each_element(chosen, |nodes, weights, rho| {
                        for (node, w) in nodes.iter().zip(weights) {
                            add_at(&mut f, *node, &acceleration.map(|a| rho * w * a));
                        }
                    });
                }
                Load::BodyForce { set, vector } => {
                    let chosen = resolved.sets.get(set).map(|r| r.elements.as_slice()).unwrap_or_default();
                    self.each_element(Some(chosen), |nodes, weights, _| {
                        for (node, w) in nodes.iter().zip(weights) {
                            add_at(&mut f, *node, &vector.map(|b| w * b));
                        }
                    });
                }
            }
        }
        self.external = f;
        Ok(())
    }

    /// Visit the elements (all, or those listed): their nodes, nodal volumes and density.
    fn each_element(&self, only: Option<&[u32]>, mut visit: impl FnMut(&[usize], &[f64], f64)) {
        let wanted = |label: u32| only.is_none_or(|l| l.binary_search(&label).is_ok());
        let density = |m: usize| self.materials.get(m).map_or(0.0, |m| m.density);
        for h in self.hexes.iter().filter(|h| wanted(h.label)) {
            visit(&h.nodes, &h.kinematics.nodal_volume, density(h.material));
        }
        for t in self.tets.iter().filter(|t| wanted(t.label)) {
            visit(&t.nodes, &t.kinematics.nodal_volume, density(t.material));
        }
    }

    /// The analysed nodes of a resolved set.
    fn nodes_of(&self, resolved: &Resolved, set: &str) -> Vec<usize> {
        resolved.sets.get(set).map(|s| s.nodes.as_slice()).unwrap_or_default().iter().filter_map(|l| self.node(*l)).collect()
    }

    /// Initial velocities, rigid walls and history nodes of the step.
    fn conditions(&mut self, resolved: &Resolved, settings: &ExplicitDynamic) -> Result<()> {
        for iv in &settings.initial_velocities {
            for node in self.nodes_of(resolved, &iv.set) {
                for (k, c) in iv.velocity.iter().enumerate() {
                    if let Some(v) =
                        DofKind::translation(k).and_then(|kind| self.layout.dof(node, kind)).and_then(|d| self.initial_velocity.get_mut(d))
                    {
                        *v = *c;
                    }
                }
            }
        }
        // A prescribed velocity holds from the start.
        for (v, p) in self.initial_velocity.iter_mut().zip(&self.prescribed) {
            if let Some(p) = p {
                *v = *p;
            }
        }
        let size = self.xyz.iter().flatten().fold(0.0f64, |m, c| m.max(c.abs())).max(1.0);
        for w in &settings.rigid_walls {
            let len = w.normal.iter().map(|c| c * c).sum::<f64>().sqrt();
            let normal = w.normal.map(|c| c / len);
            let nodes = self.nodes_of(resolved, &w.set);
            for &i in &nodes {
                let x = self.xyz.get(i).copied().unwrap_or_default();
                let gap: f64 = (0..3).map(|k| (x[k] - w.point[k]) * normal[k]).sum();
                if gap < -1e-9 * size {
                    let label = self.labels.get(i).copied().unwrap_or(0);
                    return Err(invalid(format!("node {label} of set `{}` starts behind its rigid wall (gap {gap:e})", w.set)));
                }
            }
            self.walls.push(Wall { point: w.point, normal, nodes });
        }
        if let Some(set) = &settings.history_nodes {
            let nodes = self.nodes_of(resolved, set);
            if nodes.len() > MAX_HISTORY_NODES {
                return Err(Error::Limit(format!("set `{set}` has {} nodes; histories are recorded for at most {MAX_HISTORY_NODES}", nodes.len())));
            }
            self.history_nodes = nodes;
        }
        Ok(())
    }
}
