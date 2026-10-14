//! Heat transfer: steady and transient conduction with prescribed temperatures, surface fluxes,
//! convection and internal heat sources.
//!
//! Every node of the solid elements carries one temperature ([`modelcraft_fem::heat::HEAT_DOFS`]).
//! The conductivity K (with the convection films) and capacity C are assembled on the node
//! graph in element label order ([`modelcraft_fem::heat`]); prescribed temperatures are
//! eliminated, the free ones numbered by nested dissection and solved by sparse Cholesky.
//!
//! - **Steady**: K T = f.
//! - **Transient**, the generalized trapezoidal (θ) method with n equal increments Δt over the
//!   step (T. J. R. Hughes, *The Finite Element Method*, Dover 2000, §8.2; unconditionally stable
//!   for θ ≥ ½, θ = 1 backward Euler, θ = ½ Crank–Nicolson):
//!   (C + θΔt K) Tₙ₊₁ = (C − (1 − θ)Δt K) Tₙ + Δt f, from a uniform initial temperature, with
//!   the prescribed temperatures held from the first increment on. The matrix is factored once.
//!
//! The heat flux −k∇T is reported at the integration points and averaged at the nodes; the heat
//! flow at prescribed temperatures is the reaction K T − f (C Ṫ included in transient steps).

use modelcraft_fem::element::{Extrapolation, extrapolation};
use modelcraft_fem::heat::{Conduction, PointFlux, face_film, face_weights};
use modelcraft_fem::{DofLayout, DofSet};
use modelcraft_model::{ElementKind, Load, Model, Procedure, Resolved, SectionKind, Step, TEMPERATURE_DOF};

use crate::cholesky::{Factor, FactorError, UpperColumns};
use crate::ordering::nested_dissection;
use crate::sparse::{Csr, NodeGraph, add_block};
use crate::system::hold;
use crate::{Error, Limits, Result, Stats};

/// Most temperature values a transient result keeps (frames × nodes); frames are thinned
/// evenly to stay below it, the last increment always kept.
pub const MAX_TEMPERATURE_VALUES: usize = 50_000_000;

/// A solid element conducting heat.
#[derive(Debug, Clone)]
pub struct HeatElem {
    pub label: u32,
    pub kind: ElementKind,
    /// Indices into [`HeatMesh::labels`].
    pub nodes: Vec<usize>,
    pub xyz: Vec<[f64; 3]>,
    /// Conductivity along X, Y, Z.
    pub conductivity: [f64; 3],
    /// ρc, when the material has a density and a specific heat.
    pub capacity: Option<f64>,
}

impl HeatElem {
    fn kernel(&self) -> Result<Conduction<'_>> {
        Conduction::new(self.kind, &self.xyz).map_err(|e| Error::element(self.label, e))
    }
}

/// The conducting mesh: the nodes of the solid elements by label, and the elements by label.
#[derive(Debug, Clone)]
pub struct HeatMesh {
    pub labels: Vec<u32>,
    pub xyz: Vec<[f64; 3]>,
    pub elements: Vec<HeatElem>,
    pub layout: DofLayout,
}

impl HeatMesh {
    pub fn new(model: &Model, resolved: &Resolved, max_dofs: usize) -> Result<HeatMesh> {
        let mut elements = Vec::new();
        for (name, section) in model.sections() {
            if !matches!(section.kind, SectionKind::Solid {}) {
                return Err(Error::NotSupported(format!(
                    "heat transfer in {} sections (section `{name}`); it has solid elements so far",
                    section.kind.name()
                )));
            }
            let material = model
                .material(&section.material)
                .ok_or_else(|| Error::Invalid(format!("section `{name}`: material `{}` is missing", section.material)))?;
            let thermal = material.thermal.unwrap_or_default();
            let conductivity =
                thermal.conductivity.ok_or_else(|| Error::Invalid(format!("material `{}` has no conductivity", section.material)))?.axes();
            let capacity = material.density.zip(thermal.specific_heat).map(|(rho, c)| rho * c);
            let set = resolved.sets.get(&section.set).ok_or_else(|| Error::Invalid(format!("set `{}` is not resolved", section.set)))?;
            for &label in &set.elements {
                let el = model.element(label).ok_or_else(|| Error::Invalid(format!("element {label} is missing")))?;
                elements.push((label, el.kind, el.nodes.clone(), conductivity, capacity));
            }
        }
        elements.sort_by_key(|e| e.0);
        let layout = DofLayout::uniform(elements.iter().flat_map(|e| e.2.iter().copied()), DofSet::TEMPERATURE);
        if layout.dofs() > max_dofs {
            return Err(Error::Limit(format!("the model has {} temperatures; the limit is {max_dofs}", layout.dofs())));
        }
        let labels: Vec<u32> = (0..layout.nodes()).filter_map(|s| layout.label(s)).collect();
        let xyz: Vec<[f64; 3]> = labels
            .iter()
            .map(|l| model.node(*l).map(|n| n.xyz).ok_or_else(|| Error::Invalid(format!("node {l} is missing"))))
            .collect::<Result<_>>()?;
        let mut mesh = HeatMesh { labels, xyz, elements: Vec::with_capacity(elements.len()), layout };
        for (label, kind, nodes, conductivity, capacity) in elements {
            let idx: Vec<usize> = nodes.iter().filter_map(|n| mesh.layout.node(*n)).collect();
            let xyz = idx.iter().filter_map(|i| mesh.xyz.get(*i).copied()).collect();
            mesh.elements.push(HeatElem { label, kind, nodes: idx, xyz, conductivity, capacity });
        }
        Ok(mesh)
    }

    fn node_of(&self, what: &str, label: u32) -> Result<usize> {
        self.layout.node(label).ok_or_else(|| Error::Invalid(format!("{what}: node {label} belongs to no solid element")))
    }

    fn element(&self, label: u32) -> Option<&HeatElem> {
        self.elements.binary_search_by_key(&label, |e| e.label).ok().and_then(|i| self.elements.get(i))
    }
}

/// The heat flux in one element.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementFlux {
    pub label: u32,
    pub kind: ElementKind,
    pub points: Vec<PointFlux>,
}

/// The result of a heat transfer step.
#[derive(Debug, Clone, PartialEq)]
pub struct HeatResult {
    pub step: String,
    /// Node labels with a temperature, sorted; per-node vectors follow this order.
    pub nodes: Vec<u32>,
    /// The time of each frame (one frame at time 0 for a steady step).
    pub times: Vec<f64>,
    /// Nodal temperatures per frame; the last frame is the end of the step.
    pub frames: Vec<Vec<f64>>,
    /// Heat flux averaged at the nodes, at the end of the step.
    pub flux: Vec<[f64; 3]>,
    /// Heat flux at the integration points, at the end of the step.
    pub elements: Vec<ElementFlux>,
    /// The heat each prescribed temperature supplies to the body at the end of the step
    /// (negative where it takes heat away; zero at free nodes).
    pub reaction: Vec<f64>,
    /// Heat supplied by fluxes, convection and sources (at the end temperatures), and the total
    /// reaction heat flow; in a steady state they cancel.
    pub applied_total: f64,
    pub reaction_total: f64,
    pub stats: Stats,
    pub warnings: Vec<String>,
}

impl HeatResult {
    /// The temperatures at the end of the step.
    pub fn temperature(&self) -> &[f64] {
        self.frames.last().map(Vec::as_slice).unwrap_or_default()
    }
}

/// The assembled heat system of a step: conductivity (with films), capacity when transient,
/// and the heat supplied.
struct HeatSystem {
    k: Csr,
    c: Option<Csr>,
    f: Vec<f64>,
}

fn assemble(model: &Model, resolved: &Resolved, mesh: &HeatMesh, graph: &NodeGraph, step: &Step, transient: bool) -> Result<HeatSystem> {
    let layout = &mesh.layout;
    let mut k = graph.zero_matrix(layout);
    let mut c = transient.then(|| graph.zero_matrix(layout));
    for e in &mesh.elements {
        let kernel = e.kernel()?;
        let at = |err| Error::element(e.label, err);
        add_block(&mut k, layout, &e.nodes, DofSet::TEMPERATURE, &kernel.conductivity(e.conductivity).map_err(at)?.data)?;
        if let Some(c) = c.as_mut() {
            let rho_c = e.capacity.ok_or_else(|| Error::Invalid(format!("element {}: its material needs a density and a specific heat", e.label)))?;
            add_block(c, layout, &e.nodes, DofSet::TEMPERATURE, &kernel.capacity(rho_c).map_err(at)?.data)?;
        }
    }
    let mut f = vec![0.0; layout.dofs()];
    let add = |f: &mut [f64], nodes: &[usize], v: &[f64]| {
        for (n, x) in nodes.iter().zip(v) {
            if let Some(y) = f.get_mut(*n) {
                *y += x;
            }
        }
    };
    for name in &step.loads {
        let load = model.load(name).ok_or_else(|| Error::Invalid(format!("load `{name}` is missing")))?;
        let faces = || -> Result<Vec<(u32, Vec<usize>, Vec<[f64; 3]>)>> {
            let set = load.set().unwrap_or_default();
            resolved
                .sets
                .get(set)
                .map(|s| s.faces.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|fc| {
                    let labels = model
                        .face_nodes(*fc)
                        .ok_or_else(|| Error::Invalid(format!("load `{name}`: element {} face {} is missing", fc.element, fc.face)))?;
                    let idx: Vec<usize> = labels.iter().map(|l| mesh.node_of(&format!("load `{name}`"), *l)).collect::<Result<_>>()?;
                    let xyz = idx.iter().filter_map(|i| mesh.xyz.get(*i).copied()).collect();
                    Ok((fc.element, idx, xyz))
                })
                .collect()
        };
        match load {
            Load::SurfaceFlux { flux, .. } => {
                for (label, idx, xyz) in faces()? {
                    let w = face_weights(&xyz).map_err(|e| Error::element(label, e))?;
                    add(&mut f, &idx, &w.iter().map(|w| w * flux).collect::<Vec<_>>());
                }
            }
            Load::Convection { film, sink, .. } => {
                for (label, idx, xyz) in faces()? {
                    let at = |e| Error::element(label, e);
                    add_block(&mut k, layout, &idx, DofSet::TEMPERATURE, &face_film(&xyz, *film).map_err(at)?.data)?;
                    let w = face_weights(&xyz).map_err(at)?;
                    add(&mut f, &idx, &w.iter().map(|w| w * film * sink).collect::<Vec<_>>());
                }
            }
            Load::HeatSource { set, power } => {
                for label in resolved.sets.get(set).map(|s| s.elements.as_slice()).unwrap_or_default() {
                    let e = mesh.element(*label).ok_or_else(|| Error::Invalid(format!("load `{name}`: element {label} is not a solid element")))?;
                    let q = e.kernel()?.source(*power).map_err(|err| Error::element(e.label, err))?;
                    add(&mut f, &e.nodes, &q);
                }
            }
            // Mechanical loads and temperature fields do not act on heat conduction.
            _ => {}
        }
    }
    Ok(HeatSystem { k, c, f })
}

/// The prescribed temperature of every node (`None` = free) under the step's boundary
/// conditions.
fn prescribed(model: &Model, resolved: &Resolved, mesh: &HeatMesh, step: &Step) -> Result<Vec<Option<f64>>> {
    let mut out = vec![None; mesh.layout.dofs()];
    for name in &step.boundary_conditions {
        let bc = model.boundary_condition(name).ok_or_else(|| Error::Invalid(format!("boundary condition `{name}` is missing")))?;
        let Some(v) = bc.value(TEMPERATURE_DOF) else { continue };
        for &label in resolved.target_nodes(bc.target()) {
            let Some(n) = mesh.layout.node(label) else { continue };
            hold(out.get_mut(n), v, |old| format!("node {label} is held at both {old} and {v} (boundary condition `{name}` and another)"))?;
        }
    }
    Ok(out)
}

/// The free temperatures numbered in nested-dissection order.
struct Equations {
    /// Per node, its equation or `usize::MAX` when prescribed.
    eq: Vec<usize>,
    /// Per equation, its node.
    node: Vec<usize>,
}

impl Equations {
    fn new(mesh: &HeatMesh, graph: &NodeGraph, fixed: &[Option<f64>]) -> Equations {
        let mut eq = vec![usize::MAX; mesh.labels.len()];
        let mut node = Vec::new();
        for n in nested_dissection(&mesh.xyz, &graph.adjacency) {
            if fixed.get(n).is_some_and(Option::is_none)
                && let Some(e) = eq.get_mut(n)
            {
                *e = node.len();
                node.push(n);
            }
        }
        Equations { eq, node }
    }

    /// The free-free block of a node-level matrix, rows and columns in equation order.
    fn reduce(&self, a: &Csr) -> Csr {
        let mut ptr = vec![0];
        let (mut idx, mut val) = (Vec::new(), Vec::new());
        for &n in &self.node {
            let (cols, vals) = a.row(n);
            let mut row: Vec<(usize, f64)> =
                cols.iter().zip(vals).filter_map(|(c, v)| self.eq.get(*c).filter(|e| **e != usize::MAX).map(|e| (*e, *v))).collect();
            row.sort_unstable_by_key(|t| t.0);
            for (c, v) in row {
                idx.push(c);
                val.push(v);
            }
            ptr.push(idx.len());
        }
        Csr { n: self.node.len(), ptr, idx, val }
    }

    /// Solve for the free temperatures with right-hand side `rhs` (over every node; its free
    /// rows are used) and write them into `t`, whose prescribed entries are kept.
    fn solve(&self, chol: &Factor, rhs: &[f64], q: &mut Vec<f64>, t: &mut [f64]) -> Result<()> {
        q.clear();
        q.extend(self.node.iter().map(|n| rhs.get(*n).copied().unwrap_or(0.0)));
        chol.solve(q)?;
        for (n, v) in self.node.iter().zip(q.iter()) {
            if let Some(x) = t.get_mut(*n) {
                *x = *v;
            }
        }
        if t.iter().any(|x| !x.is_finite()) {
            return Err(Error::Singular("the temperatures are not finite; the model is ill-conditioned".into()));
        }
        Ok(())
    }
}

fn factor(a: &Csr, limits: &Limits, mesh: &HeatMesh, eqs: &Equations) -> Result<Factor> {
    Factor::new(&UpperColumns::from_symmetric(a), limits.max_factor_entries).map_err(|e| match e {
        FactorError::TooLarge { entries, limit } => {
            Error::Limit(format!("the factorization needs {entries} entries; the limit is {limit} (refine less or raise the limit)"))
        }
        FactorError::OutOfMemory { entries } => Error::Limit(format!("not enough memory for the factorization ({entries} entries)")),
        FactorError::Internal(what) => Error::Internal(what),
        FactorError::NotPositiveDefinite { column } => {
            let label = eqs.node.get(column).and_then(|n| mesh.labels.get(*n)).copied().unwrap_or(0);
            Error::Singular(format!(
                "the temperature is undetermined at node {label}: no temperature boundary condition or convection reaches its part of the model"
            ))
        }
    })
}

/// y = A x.
fn mul(a: &Csr, x: &[f64]) -> Vec<f64> {
    let mut y = vec![0.0; a.n];
    a.mul(x, &mut y);
    y
}

/// The temperature history of a heat transfer or coupled step and what produced it.
struct Solution {
    mesh: HeatMesh,
    eqs: Equations,
    k: Csr,
    c: Option<Csr>,
    f: Vec<f64>,
    fixed: Vec<Option<f64>>,
    times: Vec<f64>,
    frames: Vec<Vec<f64>>,
    /// Ṫ over the last increment (zero when steady).
    rate: Vec<f64>,
    factor_entries: usize,
}

impl Solution {
    fn temperature(&self) -> &[f64] {
        self.frames.last().map(Vec::as_slice).unwrap_or_default()
    }
}

/// Solve the step's temperatures (steady unless the step is transient).
fn run(model: &Model, resolved: &Resolved, step: &Step, limits: &Limits) -> Result<Solution> {
    let transient = match step.procedure {
        Procedure::HeatTransfer { transient } => transient,
        _ => None,
    };
    let mesh = HeatMesh::new(model, resolved, limits.max_dofs)?;
    let graph = NodeGraph::new(mesh.labels.len(), mesh.elements.iter().map(|e| e.nodes.as_slice()));
    let fixed = prescribed(model, resolved, &mesh, step)?;
    let eqs = Equations::new(&mesh, &graph, &fixed);
    if eqs.node.is_empty() {
        return Err(Error::Invalid("every temperature is prescribed; there is nothing to solve".into()));
    }
    let HeatSystem { k, c, f } = assemble(model, resolved, &mesh, &graph, step, transient.is_some())?;
    let n = mesh.labels.len();
    let held: Vec<f64> = fixed.iter().map(|v| v.unwrap_or(0.0)).collect();
    let mut q = Vec::with_capacity(eqs.node.len());
    let (times, frames, rate, factor_entries) = match transient.zip(c.as_ref()) {
        Some((tr, c)) => {
            let steps = tr.increments();
            let dt = tr.end_time / steps as f64;
            // (C + θΔt K) Tₙ₊₁ = (C − (1 − θ)Δt K) Tₙ + Δt f, the prescribed values held from the
            // first increment on (their part A_fp T_p moves to the right-hand side).
            let a = c.add_scaled(tr.theta * dt, &k)?;
            let b = c.add_scaled(-(1.0 - tr.theta) * dt, &k)?;
            let chol = factor(&eqs.reduce(&a), limits, &mesh, &eqs)?;
            let a_held = mul(&a, &held);
            let every = (steps.saturating_mul(n) / MAX_TEMPERATURE_VALUES.max(1)).saturating_add(1);
            let mut t = vec![tr.initial_temperature; n];
            let (mut frames, mut times) = (vec![t.clone()], vec![0.0]);
            let mut rhs = vec![0.0; n];
            let mut next = held.clone();
            for i in 1..=steps {
                next.copy_from_slice(&held);
                b.mul(&t, &mut rhs);
                for ((r, fj), aj) in rhs.iter_mut().zip(&f).zip(&a_held) {
                    *r += dt * fj - aj;
                }
                eqs.solve(&chol, &rhs, &mut q, &mut next)?;
                std::mem::swap(&mut t, &mut next);
                if i % every == 0 || i == steps {
                    frames.push(t.clone());
                    times.push(if i == steps { tr.end_time } else { i as f64 * dt });
                }
            }
            // After the swap `next` holds the state before the last increment.
            let rate = t.iter().zip(&next).map(|(a, b)| (a - b) / dt).collect();
            (times, frames, rate, chol.entries())
        }
        None => {
            let rhs: Vec<f64> = f.iter().zip(mul(&k, &held)).map(|(a, b)| a - b).collect();
            let chol = factor(&eqs.reduce(&k), limits, &mesh, &eqs)?;
            let mut t = held;
            eqs.solve(&chol, &rhs, &mut q, &mut t)?;
            (vec![0.0], vec![t], vec![0.0; n], chol.entries())
        }
    };
    Ok(Solution { mesh, eqs, k, c, f, fixed, times, frames, rate, factor_entries })
}

/// The steady temperatures of a coupled temperature–displacement step, by node label (sorted).
pub(crate) fn temperatures(model: &Model, resolved: &Resolved, step: &Step, limits: &Limits) -> Result<(Vec<u32>, Vec<f64>)> {
    let mut s = run(model, resolved, step, limits)?;
    let t = s.frames.pop().unwrap_or_default();
    Ok((s.mesh.labels, t))
}

/// Solve a heat transfer step: the step named `step`, or the model's only one.
pub fn solve_heat(model: &Model, step: Option<&str>, limits: &Limits) -> Result<HeatResult> {
    let resolved = model.resolve()?;
    let step = model.select_step(step, |p| matches!(p, Procedure::HeatTransfer { .. }), "heat transfer")?;
    let mut warnings: Vec<String> = resolved.warnings.iter().map(ToString::to_string).collect();
    let s = run(model, &resolved, step, limits)?;
    let t = s.temperature();
    // Reactions: the heat each prescribed node supplies, K T + C Ṫ − f.
    let kt = mul(&s.k, t);
    let ct = s.c.as_ref().map(|c| mul(c, &s.rate));
    let reaction: Vec<f64> = kt
        .iter()
        .zip(&s.f)
        .zip(&s.fixed)
        .enumerate()
        .map(|(i, ((kt, f), fixed))| if fixed.is_some() { kt + ct.as_ref().and_then(|c| c.get(i)).copied().unwrap_or(0.0) - f } else { 0.0 })
        .collect();
    // Heat supplied: fluxes and sources, and convection at the end temperatures, Σ (f − K_film T)
    // = Σ (f − K T) since conduction alone has zero row sums.
    let applied_total: f64 = kt.iter().zip(&s.f).map(|(a, b)| b - a).sum();
    let reaction_total: f64 = reaction.iter().sum();
    let (flux, elements) = fluxes(&s.mesh, t)?;
    // Measured against the gross heat flows, which cancel in the net sums.
    let scale = reaction.iter().chain(&s.f).map(|v| v.abs()).sum::<f64>();
    if s.c.is_none() && scale > 0.0 && (applied_total + reaction_total).abs() > 1e-6 * scale {
        warnings.push(format!("heat supplied and removed differ by {:.3e}", (applied_total + reaction_total).abs()));
    }
    let stats = Stats { dofs: s.mesh.layout.dofs(), equations: s.eqs.node.len(), factor_entries: s.factor_entries, lanczos_vectors: 0 };
    Ok(HeatResult {
        step: step.name.clone(),
        nodes: s.mesh.labels,
        times: s.times,
        frames: s.frames,
        flux,
        elements,
        reaction,
        applied_total,
        reaction_total,
        stats,
        warnings,
    })
}

/// The heat flux at the integration points and averaged at the nodes (one extrapolation per
/// element kind).
fn fluxes(mesh: &HeatMesh, t: &[f64]) -> Result<(Vec<[f64; 3]>, Vec<ElementFlux>)> {
    let mut sum = vec![[0.0; 3]; mesh.labels.len()];
    let mut count = vec![0u32; mesh.labels.len()];
    let mut out = Vec::with_capacity(mesh.elements.len());
    let mut extrapolations: Vec<(ElementKind, Extrapolation)> = Vec::new();
    for e in &mesh.elements {
        let kernel = e.kernel()?;
        let at = |err| Error::element(e.label, err);
        let te: Vec<f64> = e.nodes.iter().map(|n| t.get(*n).copied().unwrap_or(0.0)).collect();
        let points = kernel.flux(e.conductivity, &te).map_err(at)?;
        let padded: Vec<[f64; 6]> = points.iter().map(|p| [p.flux[0], p.flux[1], p.flux[2], 0.0, 0.0, 0.0]).collect();
        let index = match extrapolations.iter().position(|(k, _)| *k == e.kind) {
            Some(i) => i,
            None => {
                extrapolations.push((e.kind, extrapolation(kernel.ty, kernel.ty.full).map_err(at)?));
                extrapolations.len() - 1
            }
        };
        let nodal = extrapolations.get(index).map(|(_, x)| x.apply(&padded)).unwrap_or_default();
        for (n, v) in e.nodes.iter().zip(&nodal) {
            if let (Some(s), Some(c)) = (sum.get_mut(*n), count.get_mut(*n)) {
                for k in 0..3 {
                    s[k] += v[k];
                }
                *c += 1;
            }
        }
        out.push(ElementFlux { label: e.label, kind: e.kind, points });
    }
    let flux = sum.iter().zip(&count).map(|(s, c)| s.map(|v| if *c > 0 { v / f64::from(*c) } else { 0.0 })).collect();
    Ok((flux, out))
}
