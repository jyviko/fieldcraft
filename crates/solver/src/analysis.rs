//! Linear static and frequency analysis of a model's step, and their results.

use modelcraft_fem::element::{Extrapolation, extrapolation};
use modelcraft_fem::quadrature::Rule;
use modelcraft_fem::{PointStress, von_mises};
use modelcraft_model::{ElementKind, Model, Procedure, Resolved, Step};
use serde::Serialize;

use crate::cholesky::{Factor, FactorError, UpperColumns};
use crate::eigen::{Operator, Wanted, lanczos};
use crate::ordering::nested_dissection;
use crate::sparse::{Csr, NodeGraph, add_block, add_diagonal, add_scalar};
use crate::system::{Mesh, equations, loads, prescribed};
use crate::{Error, Limits, Result};

/// Size and effort of a solution.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    /// Degrees of freedom of the mesh (3 per node).
    pub dofs: usize,
    /// Free degrees of freedom solved for.
    pub equations: usize,
    /// Entries in the Cholesky factor.
    pub factor_entries: usize,
    /// Lanczos vectors used (frequency analysis).
    pub lanczos_vectors: usize,
}

/// Stresses of one element.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementStress {
    pub label: u32,
    pub kind: ElementKind,
    /// At the integration points.
    pub points: Vec<PointStress>,
    /// Extrapolated to the element's nodes, in its node order.
    pub nodal: Vec<[f64; 6]>,
}

/// The result of a static step.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticResult {
    pub step: String,
    /// Node labels with degrees of freedom, sorted; the per-node vectors follow this order.
    pub nodes: Vec<u32>,
    pub displacement: Vec<[f64; 3]>,
    /// Reaction forces (zero where nothing is prescribed).
    pub reaction: Vec<[f64; 3]>,
    /// Stress averaged at the nodes over the elements that share them (xx, yy, zz, xy, xz, yz).
    pub stress: Vec<[f64; 6]>,
    /// Von Mises stress of the averaged nodal stress.
    pub von_mises: Vec<f64>,
    pub elements: Vec<ElementStress>,
    /// Sum of the applied loads and of the reactions.
    pub applied_total: [f64; 3],
    pub reaction_total: [f64; 3],
    pub stats: Stats,
    pub warnings: Vec<String>,
}

/// One natural mode.
#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    /// λ = ω², in (rad per time unit)².
    pub eigenvalue: f64,
    /// Cycles per time unit (0 for a rigid-body mode).
    pub frequency: f64,
    /// Mass-normalized (φᵀ M φ = 1), per node in [`FrequencyResult::nodes`] order.
    pub shape: Vec<[f64; 3]>,
    /// Participation factors Γ = φᵀ M r for unit translations along X, Y, Z.
    pub participation: [f64; 3],
    /// Effective mass Γ² along X, Y, Z.
    pub effective_mass: [f64; 3],
}

/// The result of a frequency step.
#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyResult {
    pub step: String,
    pub nodes: Vec<u32>,
    pub modes: Vec<Mode>,
    /// Mass of the model.
    pub total_mass: f64,
    pub stats: Stats,
    pub warnings: Vec<String>,
}

/// Mass matrix choice for a frequency analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MassKind {
    #[default]
    Consistent,
    Lumped,
}

/// Options of a frequency analysis.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrequencyOptions {
    pub mass: MassKind,
    /// The spectral shift σ (eigenvalue units); by default 0 for a supported structure and a
    /// small negative value for a free one.
    pub shift: Option<f64>,
}

/// The step named `name`, or the only step of the procedure when no name is given; it must be
/// of that procedure.
fn select_step<'m>(model: &'m Model, name: Option<&str>, wanted: fn(&Procedure) -> bool, what: &str) -> Result<&'m Step> {
    let step = match name {
        Some(n) => model.step(n).ok_or_else(|| Error::Invalid(format!("step `{n}` does not exist")))?,
        None => {
            let matching: Vec<&Step> = model.steps().iter().filter(|s| wanted(&s.procedure)).collect();
            match matching.as_slice() {
                [one] => *one,
                [] => return Err(Error::Invalid(format!("the model has no {what} step"))),
                many => {
                    let names: Vec<&str> = many.iter().map(|s| s.name.as_str()).collect();
                    return Err(Error::Invalid(format!("name the step: the model has {what} steps {}", names.join(", "))));
                }
            }
        }
    };
    if !wanted(&step.procedure) {
        return Err(Error::Invalid(format!("step `{}` is a {} step, not a {what} one", step.name, step.procedure.name())));
    }
    Ok(step)
}

/// The assembled matrices and the model's mass.
struct Assembled {
    k: Csr,
    m: Option<Csr>,
}

/// Assemble the stiffness (and the mass when `mass` is set) on the mesh's node graph.
fn assemble(mesh: &Mesh, graph: &NodeGraph, mass: Option<MassKind>) -> Result<Assembled> {
    let mut k = graph.zero_matrix();
    let mut m = mass.map(|_| graph.zero_matrix());
    for e in &mesh.elements {
        let solid = e.solid()?;
        let at = |err| Error::element(e.label, err);
        add_block(&mut k, &e.nodes, &solid.stiffness(&e.d).map_err(at)?.data)?;
        if let (Some(mm), Some(kind)) = (m.as_mut(), mass) {
            let rho = e.density.ok_or_else(|| Error::Invalid(format!("element {} has no density", e.label)))?;
            match kind {
                MassKind::Consistent => add_scalar(mm, &e.nodes, &solid.mass(rho).map_err(at)?.data)?,
                MassKind::Lumped => add_diagonal(mm, &e.nodes, &solid.lumped_mass(rho).map_err(at)?)?,
            }
        }
    }
    Ok(Assembled { k, m })
}

/// Factor a free-dof matrix; a singular pivot is reported as the free-dof column it hit.
fn factor_raw(a: &Csr, limits: &Limits) -> std::result::Result<Factor, FactorError> {
    Factor::new(&UpperColumns::from_symmetric(a), limits.max_factor_entries)
}

/// A factorization failure in the user's terms: the node and direction of a singular pivot.
fn factor_error(e: FactorError, mesh: &Mesh, map: &[Option<usize>]) -> Error {
    match e {
        FactorError::TooLarge { entries, limit } => {
            Error::Limit(format!("the factorization needs {entries} entries; the limit is {limit} (refine less or raise the limit)"))
        }
        FactorError::NotPositiveDefinite { column } => {
            let dof = map.iter().position(|m| *m == Some(column)).unwrap_or(0);
            let label = mesh.labels.get(dof / 3).copied().unwrap_or(0);
            let axis = ["X", "Y", "Z"].get(dof % 3).copied().unwrap_or("?");
            Error::Singular(format!(
                "the stiffness is singular at node {label} in {axis}: the model can move without straining (missing or insufficient boundary conditions, or a part not connected to the rest)"
            ))
        }
    }
}

/// Spread a free-equation vector over every degree of freedom (`base` holds the prescribed
/// values) and group it per node.
fn per_node(x: &[f64], map: &[Option<usize>], base: Vec<f64>) -> Vec<[f64; 3]> {
    let mut full = base;
    for (dof, m) in map.iter().enumerate() {
        if let (Some(eq), Some(v)) = (m, full.get_mut(dof)) {
            *v = x.get(*eq).copied().unwrap_or(0.0);
        }
    }
    full.as_chunks::<3>().0.to_vec()
}

/// Solve a linear static step: the step named `step`, or the model's only static step.
pub fn solve_static(model: &Model, step: Option<&str>, limits: &Limits) -> Result<StaticResult> {
    let resolved = model.resolve()?;
    let step = select_step(model, step, |p| matches!(p, Procedure::Static {}), "static")?;
    let mesh = Mesh::new(model, &resolved, limits.max_dofs)?;
    let fixed = prescribed(model, &resolved, &mesh, step, true)?;
    let f = loads(model, &resolved, &mesh, step)?;
    let graph = NodeGraph::new(mesh.labels.len(), mesh.elements.iter().map(|e| e.nodes.as_slice()));
    let k = assemble(&mesh, &graph, None)?.k;
    let (map, neq) = equations(&nested_dissection(&mesh.xyz, &graph.adjacency), &fixed);
    if neq == 0 {
        return Err(Error::Invalid("every degree of freedom is prescribed; there is nothing to solve".into()));
    }
    let chol = factor_raw(&k.restrict(&map, neq), limits).map_err(|e| factor_error(e, &mesh, &map))?;
    // Right-hand side: f_free − K_fp u_p (K u_p only when something moves).
    let up: Vec<f64> = fixed.iter().map(|v| v.unwrap_or(0.0)).collect();
    let mut kup = vec![0.0; mesh.dofs()];
    if up.iter().any(|v| *v != 0.0) {
        k.mul(&up, &mut kup);
    }
    let mut rhs = vec![0.0; neq];
    for (dof, m) in map.iter().enumerate() {
        if let (Some(eq), Some(fv), Some(kv)) = (m, f.get(dof), kup.get(dof))
            && let Some(r) = rhs.get_mut(*eq)
        {
            *r = fv - kv;
        }
    }
    chol.solve(&mut rhs)?;
    let displacement = per_node(&rhs, &map, up);
    if displacement.iter().flatten().any(|x| !x.is_finite()) {
        return Err(Error::Singular("the solution is not finite; the model is ill-conditioned".into()));
    }
    // Reactions R = K u − f at the prescribed degrees of freedom (only their rows of K).
    let mut reaction = vec![[0.0; 3]; mesh.labels.len()];
    let (mut applied_total, mut reaction_total) = ([0.0; 3], [0.0; 3]);
    for (dof, fv) in f.iter().enumerate() {
        let (node, c) = (dof / 3, dof % 3);
        applied_total[c] += fv;
        if fixed.get(dof).is_some_and(Option::is_some) {
            let (cols, vals) = k.row(dof);
            let ku: f64 = cols.iter().zip(vals).map(|(j, v)| v * displacement.get(j / 3).map_or(0.0, |u| u[j % 3])).sum();
            if let Some(rn) = reaction.get_mut(node) {
                rn[c] = ku - fv;
            }
            reaction_total[c] += ku - fv;
        }
    }
    // Stresses: one extrapolation per element type and rule.
    let mut extrapolations: Vec<(ElementKind, Rule, Extrapolation)> = Vec::new();
    let mut elements = Vec::with_capacity(mesh.elements.len());
    let mut sum = vec![[0.0; 6]; mesh.labels.len()];
    let mut count = vec![0u32; mesh.labels.len()];
    for e in &mesh.elements {
        let solid = e.solid()?;
        let at = |err| Error::element(e.label, err);
        let ue: Vec<[f64; 3]> = e.nodes.iter().map(|n| displacement.get(*n).copied().unwrap_or_default()).collect();
        let points = solid.stresses(&e.d, &ue).map_err(at)?;
        let values: Vec<[f64; 6]> = points.iter().map(|p| p.stress).collect();
        let rule = solid.rule();
        let cached = extrapolations.iter().position(|(k, r, _)| *k == e.kind && *r == rule);
        let index = match cached {
            Some(i) => i,
            None => {
                extrapolations.push((e.kind, rule, extrapolation(solid.ty, rule).map_err(at)?));
                extrapolations.len() - 1
            }
        };
        let nodal = extrapolations.get(index).map(|(_, _, x)| x.apply(&values)).unwrap_or_default();
        for (n, s) in e.nodes.iter().zip(&nodal) {
            if let (Some(acc), Some(c)) = (sum.get_mut(*n), count.get_mut(*n)) {
                for k in 0..6 {
                    acc[k] += s[k];
                }
                *c += 1;
            }
        }
        elements.push(ElementStress { label: e.label, kind: e.kind, points, nodal });
    }
    let stress: Vec<[f64; 6]> = sum.iter().zip(&count).map(|(s, c)| s.map(|v| if *c > 0 { v / f64::from(*c) } else { 0.0 })).collect();
    let von_mises = stress.iter().map(von_mises).collect();
    let mut warnings: Vec<String> = resolved.warnings.iter().map(ToString::to_string).collect();
    let scale = applied_total.iter().chain(&reaction_total).fold(0.0f64, |m, v| m.max(v.abs()));
    let imbalance = (0..3).map(|c| (applied_total[c] + reaction_total[c]).abs()).fold(0.0, f64::max);
    if scale > 0.0 && imbalance > 1e-6 * scale {
        warnings.push(format!("loads and reactions differ by {imbalance:.3e} (relative {:.1e})", imbalance / scale));
    }
    Ok(StaticResult {
        step: step.name.clone(),
        nodes: mesh.labels.clone(),
        displacement,
        reaction,
        stress,
        von_mises,
        elements,
        applied_total,
        reaction_total,
        stats: Stats { dofs: mesh.dofs(), equations: neq, factor_entries: chol.entries(), lanczos_vectors: 0 },
        warnings,
    })
}

struct ShiftInvert<'a> {
    m: &'a Csr,
    factor: &'a Factor,
}

impl Operator for ShiftInvert<'_> {
    fn n(&self) -> usize {
        self.m.n
    }
    fn mass(&self, x: &[f64], y: &mut [f64]) {
        self.m.mul(x, y);
    }
    fn solve(&self, x: &mut [f64]) -> Result<()> {
        self.factor.solve(x)
    }
}

/// Block size of the Lanczos iteration: enough for the six rigid-body modes of a free body.
const BLOCK: usize = 6;

/// Relative residual of an accepted Ritz pair.
const TOLERANCE: f64 = 1e-9;

/// Solve a frequency step (the step named `step`, or the model's only frequency step): the
/// lowest natural modes within the step's frequency range.
pub fn solve_frequency(model: &Model, step: Option<&str>, options: &FrequencyOptions, limits: &Limits) -> Result<FrequencyResult> {
    let resolved: Resolved = model.resolve()?;
    let step = select_step(model, step, |p| matches!(p, Procedure::Frequency { .. }), "frequency")?;
    let Procedure::Frequency { modes, min_hz, max_hz } = step.procedure else {
        return Err(Error::Internal("frequency step".into()));
    };
    if let Some(s) = options.shift
        && !s.is_finite()
    {
        return Err(Error::Invalid(format!("the shift must be a finite number, got {s}")));
    }
    let mesh = Mesh::new(model, &resolved, limits.max_dofs)?;
    let fixed = prescribed(model, &resolved, &mesh, step, false)?;
    let graph = NodeGraph::new(mesh.labels.len(), mesh.elements.iter().map(|e| e.nodes.as_slice()));
    let Assembled { k, m } = assemble(&mesh, &graph, Some(options.mass))?;
    let m = m.ok_or_else(|| Error::Internal("mass matrix".into()))?;
    // Every element's mass entries sum to its mass, once per direction.
    let total_mass = m.val.iter().sum::<f64>() / 3.0;
    let (map, neq) = equations(&nested_dissection(&mesh.xyz, &graph.adjacency), &fixed);
    if neq == 0 {
        return Err(Error::Invalid("every degree of freedom is held; there is nothing to vibrate".into()));
    }
    let kff = k.restrict(&map, neq);
    let mff = m.restrict(&map, neq);
    drop((k, m));
    let mut warnings: Vec<String> = resolved.warnings.iter().map(ToString::to_string).collect();
    let shifted = |s: f64| -> Result<Factor> { factor_raw(&kff.add_scaled(-s, &mff)?, limits).map_err(|e| factor_error(e, &mesh, &map)) };
    let (shift, chol) = match options.shift {
        Some(s) => (s, shifted(s)?),
        None => match factor_raw(&kff, limits) {
            Ok(f) => (0.0, f),
            Err(FactorError::NotPositiveDefinite { .. }) => {
                // A free (or partly free) body: shift below zero so the rigid-body modes come out.
                let (tk, tm): (f64, f64) = (kff.diagonal().iter().sum(), mff.diagonal().iter().sum());
                if !(tk > 0.0 && tm > 0.0) {
                    return Err(Error::Singular("the stiffness or mass has no positive diagonal".into()));
                }
                let s = -1e-6 * tk / tm;
                warnings.push("the structure is not fully supported; rigid-body modes appear at (near) zero frequency".into());
                (s, shifted(s)?)
            }
            Err(e) => return Err(factor_error(e, &mesh, &map)),
        },
    };
    drop(kff);
    let two_pi = 2.0 * std::f64::consts::PI;
    let wanted = Wanted {
        count: usize::try_from(modes).unwrap_or(usize::MAX).min(neq),
        min: min_hz.map_or(f64::NEG_INFINITY, |f| (two_pi * f).powi(2)),
        max: max_hz.map_or(f64::INFINITY, |f| (two_pi * f).powi(2)),
    };
    // The basis (vectors and their mass products) costs 2 n × vectors; cap it with the factor's
    // entry limit.
    let by_memory = limits.max_factor_entries / (2 * neq).max(1);
    let max_basis = limits.max_lanczos_vectors.min(by_memory).max(BLOCK.min(neq));
    let op = ShiftInvert { m: &mff, factor: &chol };
    let pairs = lanczos(&op, shift, wanted, BLOCK, max_basis, TOLERANCE)?;
    // Participation: Γ_d = φᵀ M r_d with r_d the unit translation along d on the free dofs.
    let mr: Vec<Vec<f64>> = (0..3)
        .map(|d| {
            let mut r = vec![0.0; neq];
            for (dof, mm) in map.iter().enumerate() {
                if let Some(x) = mm.filter(|_| dof % 3 == d).and_then(|eq| r.get_mut(eq)) {
                    *x = 1.0;
                }
            }
            let mut y = vec![0.0; neq];
            mff.mul(&r, &mut y);
            y
        })
        .collect();
    let mut out = Vec::with_capacity(pairs.values.len());
    for (lambda, phi) in pairs.values.iter().zip(&pairs.vectors) {
        let participation = [0, 1, 2].map(|d| mr.get(d).map_or(0.0, |y| y.iter().zip(phi).map(|(a, b)| a * b).sum()));
        out.push(Mode {
            eigenvalue: *lambda,
            frequency: lambda.max(0.0).sqrt() / two_pi,
            shape: per_node(phi, &map, vec![0.0; mesh.dofs()]),
            participation,
            effective_mass: participation.map(|g| g * g),
        });
    }
    if out.len() < wanted.count && max_hz.is_none() {
        warnings.push(format!("found {} of the {modes} modes asked for", out.len()));
    }
    Ok(FrequencyResult {
        step: step.name.clone(),
        nodes: mesh.labels.clone(),
        modes: out,
        total_mass,
        stats: Stats { dofs: mesh.dofs(), equations: neq, factor_entries: chol.entries(), lanczos_vectors: pairs.basis },
        warnings,
    })
}

/// The result of one step, as a session keeps it.
#[derive(Debug, Clone, PartialEq)]
pub enum StepResult {
    Static(StaticResult),
    Frequency(FrequencyResult),
}

impl StepResult {
    pub fn step(&self) -> &str {
        match self {
            StepResult::Static(r) => &r.step,
            StepResult::Frequency(r) => &r.step,
        }
    }

    pub fn nodes(&self) -> &[u32] {
        match self {
            StepResult::Static(r) => &r.nodes,
            StepResult::Frequency(r) => &r.nodes,
        }
    }
}
