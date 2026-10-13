//! Linear static and frequency analysis of a model's step, and their results.

use modelcraft_fem::element::extrapolation;
use modelcraft_fem::{PointStress, von_mises};
use modelcraft_model::{ElementKind, Model, Procedure, Resolved, Step};
use serde::Serialize;

use crate::cholesky::{Factor, FactorError, UpperColumns};
use crate::eigen::{Operator, Wanted, lanczos};
use crate::ordering::nested_dissection;
use crate::sparse::{Csr, NodeGraph, add_element};
use crate::system::{Mesh, element_error, equations, loads, prescribed};
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

fn find_step<'m>(model: &'m Model, name: &str) -> Result<&'m Step> {
    model.step(name).ok_or_else(|| Error::Invalid(format!("step `{name}` does not exist")))
}

/// Assemble the stiffness (and the mass when `mass` is set) on the mesh's node graph.
fn assemble(mesh: &Mesh, graph: &NodeGraph, mass: Option<MassKind>) -> Result<(Csr, Option<Csr>)> {
    let mut k = graph.zero_matrix();
    let mut m = mass.map(|_| graph.zero_matrix());
    for e in &mesh.elements {
        let solid = e.solid()?;
        let ke = solid.stiffness(&e.d).map_err(|err| element_error(e.label, err))?;
        add_element(&mut k, &e.nodes, &ke.data, false)?;
        if let (Some(mm), Some(kind)) = (m.as_mut(), mass) {
            let rho = e.density.ok_or_else(|| Error::Invalid(format!("element {} has no density", e.label)))?;
            let me = match kind {
                MassKind::Consistent => solid.mass(rho).map_err(|err| element_error(e.label, err))?.data,
                MassKind::Lumped => {
                    let d = solid.lumped_mass(rho).map_err(|err| element_error(e.label, err))?;
                    let n = d.len();
                    let mut full = vec![0.0; n * n];
                    for (i, v) in d.iter().enumerate() {
                        if let Some(x) = full.get_mut(i * n + i) {
                            *x = *v;
                        }
                    }
                    full
                }
            };
            add_element(mm, &e.nodes, &me, true)?;
        }
    }
    Ok((k, m))
}

/// Factor a free-dof matrix, naming the node and direction of a singular pivot.
fn factor(a: &Csr, mesh: &Mesh, map: &[Option<usize>], limits: &Limits) -> std::result::Result<Factor, Error> {
    Factor::new(&UpperColumns::from_symmetric(a), limits.max_factor_entries).map_err(|e| match e {
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
    })
}

fn node_order(mesh: &Mesh, graph: &NodeGraph) -> Vec<usize> {
    nested_dissection(&mesh.xyz, &graph.adjacency)
}

/// Solve a linear static step.
pub fn solve_static(model: &Model, step_name: &str, limits: &Limits) -> Result<StaticResult> {
    let resolved = model.resolve()?;
    let step = find_step(model, step_name)?;
    if !matches!(step.procedure, Procedure::Static {}) {
        return Err(Error::Invalid(format!("step `{step_name}` is a {} step, not a static one", step.procedure.name())));
    }
    let mesh = Mesh::new(model, &resolved, limits.max_dofs)?;
    let fixed = prescribed(model, &resolved, &mesh, step, true)?;
    let f = loads(model, &resolved, &mesh, step)?;
    let graph = NodeGraph::new(mesh.labels.len(), mesh.elements.iter().map(|e| e.nodes.as_slice()));
    let (k, _) = assemble(&mesh, &graph, None)?;
    let (map, neq) = equations(&node_order(&mesh, &graph), &fixed);
    if neq == 0 {
        return Err(Error::Invalid("every degree of freedom is prescribed; there is nothing to solve".into()));
    }
    let kff = k.restrict(&map, neq);
    let chol = factor(&kff, &mesh, &map, limits)?;
    // Right-hand side: f_free − K_fp u_p.
    let up: Vec<f64> = fixed.iter().map(|v| v.unwrap_or(0.0)).collect();
    let mut kup = vec![0.0; mesh.dofs()];
    k.mul(&up, &mut kup);
    let mut rhs = vec![0.0; neq];
    for (dof, m) in map.iter().enumerate() {
        if let (Some(eq), Some(fv), Some(kv)) = (m, f.get(dof), kup.get(dof))
            && let Some(r) = rhs.get_mut(*eq)
        {
            *r = fv - kv;
        }
    }
    chol.solve(&mut rhs)?;
    let mut u = up;
    for (dof, m) in map.iter().enumerate() {
        if let (Some(eq), Some(x)) = (m, u.get_mut(dof)) {
            *x = rhs.get(*eq).copied().unwrap_or(0.0);
        }
    }
    if u.iter().any(|x| !x.is_finite()) {
        return Err(Error::Singular("the solution is not finite; the model is ill-conditioned".into()));
    }
    // Reactions R = K u − f at the prescribed degrees of freedom.
    let mut ku = vec![0.0; mesh.dofs()];
    k.mul(&u, &mut ku);
    let nn = mesh.labels.len();
    let mut reaction = vec![[0.0; 3]; nn];
    let mut displacement = vec![[0.0; 3]; nn];
    let (mut applied_total, mut reaction_total) = ([0.0; 3], [0.0; 3]);
    for dof in 0..mesh.dofs() {
        let (node, c) = (dof / 3, dof % 3);
        let fv = f.get(dof).copied().unwrap_or(0.0);
        applied_total[c] += fv;
        if let Some(d) = displacement.get_mut(node) {
            d[c] = u.get(dof).copied().unwrap_or(0.0);
        }
        if fixed.get(dof).is_some_and(Option::is_some) {
            let r = ku.get(dof).copied().unwrap_or(0.0) - fv;
            if let Some(rn) = reaction.get_mut(node) {
                rn[c] = r;
            }
            reaction_total[c] += r;
        }
    }
    // Stresses.
    let mut elements = Vec::with_capacity(mesh.elements.len());
    let mut sum = vec![[0.0; 6]; nn];
    let mut count = vec![0u32; nn];
    for e in &mesh.elements {
        let solid = e.solid()?;
        let ue: Vec<[f64; 3]> = e.nodes.iter().map(|n| displacement.get(*n).copied().unwrap_or_default()).collect();
        let points = solid.stresses(&e.d, &ue).map_err(|err| element_error(e.label, err))?;
        let values: Vec<[f64; 6]> = points.iter().map(|p| p.stress).collect();
        let nodal = extrapolation(solid.ty, solid.rule()).map_err(|err| element_error(e.label, err))?.apply(&values);
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
        step: step_name.to_string(),
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

/// Solve a frequency step: the lowest natural modes (within the step's frequency range).
pub fn solve_frequency(model: &Model, step_name: &str, options: &FrequencyOptions, limits: &Limits) -> Result<FrequencyResult> {
    let resolved: Resolved = model.resolve()?;
    let step = find_step(model, step_name)?;
    let Procedure::Frequency { modes, min_hz, max_hz } = step.procedure else {
        return Err(Error::Invalid(format!("step `{step_name}` is a {} step, not a frequency one", step.procedure.name())));
    };
    let mesh = Mesh::new(model, &resolved, limits.max_dofs)?;
    let fixed = prescribed(model, &resolved, &mesh, step, false)?;
    let graph = NodeGraph::new(mesh.labels.len(), mesh.elements.iter().map(|e| e.nodes.as_slice()));
    let (k, m) = assemble(&mesh, &graph, Some(options.mass))?;
    let m = m.ok_or_else(|| Error::Internal("mass matrix".into()))?;
    let (map, neq) = equations(&node_order(&mesh, &graph), &fixed);
    if neq == 0 {
        return Err(Error::Invalid("every degree of freedom is held; there is nothing to vibrate".into()));
    }
    let kff = k.restrict(&map, neq);
    let mff = m.restrict(&map, neq);
    let mut warnings: Vec<String> = resolved.warnings.iter().map(ToString::to_string).collect();
    let (shift, chol) = match options.shift {
        Some(s) if s.is_finite() => (s, factor(&kff.add_scaled(-s, &mff)?, &mesh, &map, limits)?),
        Some(s) => return Err(Error::Invalid(format!("the shift must be a finite number, got {s}"))),
        None => match factor(&kff, &mesh, &map, limits) {
            Ok(f) => (0.0, f),
            Err(Error::Singular(_)) => {
                // A free (or partly free) body: shift below zero so the rigid-body modes come out.
                let (tk, tm): (f64, f64) = (kff.diagonal().iter().sum(), mff.diagonal().iter().sum());
                if !(tk > 0.0 && tm > 0.0) {
                    return Err(Error::Singular("the stiffness or mass has no positive diagonal".into()));
                }
                let s = -1e-6 * tk / tm;
                warnings.push("the structure is not fully supported; rigid-body modes appear at (near) zero frequency".into());
                (s, factor(&kff.add_scaled(-s, &mff)?, &mesh, &map, limits)?)
            }
            Err(e) => return Err(e),
        },
    };
    let two_pi = 2.0 * std::f64::consts::PI;
    let wanted = Wanted {
        count: usize::try_from(modes).unwrap_or(usize::MAX).min(neq),
        min: min_hz.map_or(f64::NEG_INFINITY, |f| (two_pi * f).powi(2)),
        max: max_hz.map_or(f64::INFINITY, |f| (two_pi * f).powi(2)),
    };
    // The basis costs n × vectors memory; cap it with the factor's entry limit.
    let by_memory = limits.max_factor_entries / neq.max(1);
    let max_basis = limits.max_lanczos_vectors.min(by_memory).max(BLOCK.min(neq));
    let op = ShiftInvert { m: &mff, factor: &chol };
    let pairs = lanczos(&op, shift, wanted, BLOCK, max_basis, TOLERANCE)?;
    // Participation: Γ_d = φᵀ M r_d with r_d the unit translation along d on the free dofs.
    let mut rigid: Vec<Vec<f64>> = vec![vec![0.0; neq]; 3];
    for (dof, mm) in map.iter().enumerate() {
        if let (Some(eq), Some(r)) = (mm, rigid.get_mut(dof % 3))
            && let Some(x) = r.get_mut(*eq)
        {
            *x = 1.0;
        }
    }
    let mr: Vec<Vec<f64>> = rigid
        .iter()
        .map(|r| {
            let mut y = vec![0.0; neq];
            mff.mul(r, &mut y);
            y
        })
        .collect();
    let mut out = Vec::with_capacity(pairs.values.len());
    for (lambda, phi) in pairs.values.iter().zip(&pairs.vectors) {
        let mut shape = vec![[0.0; 3]; mesh.labels.len()];
        for (dof, mm) in map.iter().enumerate() {
            if let (Some(eq), Some(s)) = (mm, shape.get_mut(dof / 3)) {
                s[dof % 3] = phi.get(*eq).copied().unwrap_or(0.0);
            }
        }
        let participation = [0, 1, 2].map(|d| mr.get(d).map_or(0.0, |y| y.iter().zip(phi).map(|(a, b)| a * b).sum()));
        out.push(Mode {
            eigenvalue: *lambda,
            frequency: lambda.max(0.0).sqrt() / two_pi,
            shape,
            participation,
            effective_mass: participation.map(|g| g * g),
        });
    }
    if out.len() < wanted.count && max_hz.is_none() {
        warnings.push(format!("found {} of the {modes} modes asked for", out.len()));
    }
    let mut total_mass = 0.0;
    for e in &mesh.elements {
        total_mass += e.density.unwrap_or(0.0) * e.solid()?.volume().map_err(|err| element_error(e.label, err))?;
    }
    Ok(FrequencyResult {
        step: step_name.to_string(),
        nodes: mesh.labels.clone(),
        modes: out,
        total_mass,
        stats: Stats { dofs: mesh.dofs(), equations: neq, factor_entries: chol.entries(), lanczos_vectors: pairs.basis },
        warnings,
    })
}
