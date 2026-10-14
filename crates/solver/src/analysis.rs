//! Linear static and frequency analysis of a model's step, and their results.

use modelcraft_fem::ElementType;
use modelcraft_fem::beam::BeamForces;
use modelcraft_fem::element::{Extrapolation, extrapolation};
use modelcraft_fem::quadrature::Rule;
use modelcraft_fem::shell::ShellStress;
use modelcraft_fem::{DofKind, DofLayout, PointStress, von_mises};
use modelcraft_model::{ElementKind, Model, Procedure, Resolved, Step};
use serde::Serialize;

use crate::cholesky::{Factor, FactorError, UpperColumns};
use crate::constraints::{Budget, Reduction, coupling_rows};
use crate::eigen::{Operator, Wanted, lanczos};
use crate::ordering::nested_dissection;
use crate::sparse::{Csr, NodeGraph, add_block, add_diagonal, add_scalar};
use crate::system::{
    Mesh, Structural, StructuralKernel, add_thermal_loads, inelastic_warnings, loads, prescribed, structural_body_forces, temperature_loads,
    translation_block,
};
use crate::{Error, Limits, Result};

/// Size and effort of a solution.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    /// Degrees of freedom: those of the mesh nodes (3 per solid node) and 6 per coupled
    /// reference point.
    pub dofs: usize,
    /// Free degrees of freedom solved for.
    pub equations: usize,
    /// Entries in the Cholesky factor.
    pub factor_entries: usize,
    /// Lanczos vectors used (frequency analysis).
    pub lanczos_vectors: usize,
}

/// Section forces and surface stresses of one shell element.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellResult {
    pub label: u32,
    pub kind: ElementKind,
    pub stress: ShellStress,
}

/// End section forces of one beam element.
#[derive(Debug, Clone, PartialEq)]
pub struct BeamResult {
    pub label: u32,
    pub kind: ElementKind,
    pub forces: BeamForces,
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

/// The motion and reaction of a coupled reference point: translations along X, Y, Z, then
/// rotations about them (reactions: forces, then moments). Directions no coupling ties are zero.
#[derive(Debug, Clone, PartialEq)]
pub struct PointResult {
    pub name: String,
    pub label: u32,
    pub displacement: [f64; 6],
    pub reaction: [f64; 6],
}

/// The result of a static step.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticResult {
    pub step: String,
    /// Node labels with degrees of freedom, sorted; the per-node vectors follow this order.
    pub nodes: Vec<u32>,
    pub displacement: Vec<[f64; 3]>,
    /// Rotations about X, Y, Z (zero at nodes without rotations: those of solids and trusses
    /// only).
    pub rotation: Vec<[f64; 3]>,
    /// Reaction forces (zero where nothing is prescribed).
    pub reaction: Vec<[f64; 3]>,
    /// Reaction moments at held rotations.
    pub reaction_moment: Vec<[f64; 3]>,
    /// Stress averaged at the nodes over the elements that share them (xx, yy, zz, xy, xz, yz).
    pub stress: Vec<[f64; 6]>,
    /// Von Mises stress of the averaged nodal stress.
    pub von_mises: Vec<f64>,
    pub elements: Vec<ElementStress>,
    /// Shell section forces and surface stresses, by label (their top surface, the side their
    /// normal points to, joins the nodal stress average).
    pub shells: Vec<ShellResult>,
    /// Beam end section forces, by label.
    pub beams: Vec<BeamResult>,
    /// The coupled reference points, by name.
    pub reference_points: Vec<PointResult>,
    /// Sum of the applied forces and of the reaction forces (moments are not included).
    pub applied_total: [f64; 3],
    pub reaction_total: [f64; 3],
    pub stats: Stats,
    pub warnings: Vec<String>,
    /// The temperatures of a coupled temperature–displacement step, per node.
    pub temperature: Option<Vec<f64>>,
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
    /// The rotations of the shape (zero at nodes without rotations).
    pub rotation: Vec<[f64; 3]>,
    /// Participation factors Γ = φᵀ M r for unit translations along X, Y, Z.
    pub participation: [f64; 3],
    /// Effective mass Γ² along X, Y, Z.
    pub effective_mass: [f64; 3],
    /// The shape at the coupled reference points ([`FrequencyResult::reference_points`] order):
    /// translations, then rotations.
    pub point_shape: Vec<[f64; 6]>,
}

/// The result of a frequency step.
#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyResult {
    pub step: String,
    pub nodes: Vec<u32>,
    /// The coupled reference points' names and labels.
    pub reference_points: Vec<(String, u32)>,
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

/// The assembled matrices and the model's mass.
struct Assembled {
    k: Csr,
    m: Option<Csr>,
}

/// Assemble the stiffness (and the mass when `mass` is set) on the mesh's node graph.
fn assemble(mesh: &Mesh, graph: &NodeGraph, mass: Option<MassKind>) -> Result<Assembled> {
    let layout = &mesh.layout;
    let mut k = graph.zero_matrix(layout);
    let mut m = mass.map(|_| graph.zero_matrix(layout));
    for e in &mesh.elements {
        let kernel = e.kernel()?;
        let kinds = kernel.dofs().node;
        let at = |err| Error::element(e.label, err);
        add_block(&mut k, layout, &e.nodes, kinds, &kernel.stiffness(&e.d).map_err(at)?.data)?;
        if let (Some(mm), Some(kind)) = (m.as_mut(), mass) {
            let rho = e.density.ok_or_else(|| Error::Invalid(format!("element {} has no density", e.label)))?;
            match kind {
                MassKind::Consistent => add_scalar(mm, layout, &e.nodes, kinds, &kernel.mass(rho).map_err(at)?.data)?,
                MassKind::Lumped => add_diagonal(mm, layout, &e.nodes, kinds, &kernel.lumped_mass(rho).map_err(at)?)?,
            }
        }
    }
    let kinds = mesh.translation_kinds();
    for t in &mesh.trusses {
        let truss = t.truss()?;
        let at = |err| Error::element(t.label, err);
        let stiffness = translation_block(&truss.stiffness(t.young, t.area).map_err(at)?.data, t.nodes.len(), kinds);
        add_block(&mut k, layout, &t.nodes, kinds, &stiffness)?;
        if let (Some(mm), Some(kind)) = (m.as_mut(), mass) {
            let rho = t.density.ok_or_else(|| Error::Invalid(format!("element {} has no density", t.label)))?;
            match kind {
                MassKind::Consistent => add_scalar(mm, layout, &t.nodes, kinds, &truss.mass(rho, t.area).map_err(at)?.data)?,
                MassKind::Lumped => add_diagonal(mm, layout, &t.nodes, kinds, &truss.lumped_mass(rho, t.area).map_err(at)?)?,
            }
        }
    }
    for s in &mesh.structural {
        let kernel = s.kernel()?;
        let at = |err| Error::element(s.label, err);
        add_block(&mut k, layout, &s.nodes, Structural::KINDS, &kernel.stiffness().map_err(at)?.data)?;
        if let (Some(mm), Some(kind)) = (m.as_mut(), mass) {
            let rho = s.density.ok_or_else(|| Error::Invalid(format!("element {} has no density", s.label)))?;
            add_block(mm, layout, &s.nodes, Structural::KINDS, &kernel.mass(rho, kind).map_err(at)?.data)?;
        }
    }
    mesh.discrete.add_stiffness(&mut k)?;
    if let Some(mm) = m.as_mut() {
        mesh.discrete.add_mass(mm)?;
    }
    Ok(Assembled { k, m })
}

/// Factor a free-dof matrix; a singular pivot is reported as the free-dof column it hit.
fn factor_raw(a: &Csr, limits: &Limits) -> std::result::Result<Factor, FactorError> {
    Factor::new(&UpperColumns::from_symmetric(a), limits.max_factor_entries)
}

/// A factorization failure in the user's terms: the node (or reference point) and direction of
/// a singular pivot.
fn factor_error(e: FactorError, mesh: &Mesh, reduction: &Reduction) -> Error {
    match e {
        FactorError::TooLarge { entries, limit } => {
            Error::Limit(format!("the factorization needs {entries} entries; the limit is {limit} (refine less or raise the limit)"))
        }
        FactorError::OutOfMemory { entries } => Error::Limit(format!("not enough memory for the factorization ({entries} entries)")),
        FactorError::Internal(what) => Error::Internal(what),
        FactorError::NotPositiveDefinite { column } => {
            let at = reduction.eq_dof.get(column).map_or_else(|| "an unknown degree of freedom".to_string(), |d| mesh.describe_dof(*d));
            Error::Singular(format!(
                "the stiffness is singular at {at}: the model can move without straining (missing or insufficient boundary conditions, a part not connected to the rest, or a reference point direction nothing holds)"
            ))
        }
    }
}

/// The structural directions (0..N) of layout slot `slot` in a full vector, zero where the slot
/// has none.
fn directions<const N: usize>(full: &[f64], layout: &DofLayout, slot: usize) -> [f64; N] {
    std::array::from_fn(|c| DofKind::from_direction(c).and_then(|k| layout.dof(slot, k)).and_then(|d| full.get(d)).copied().unwrap_or(0.0))
}

/// A full vector split into the mesh nodes' translations and rotations (zero where a node has
/// none), per node, and the reference points' translations and rotations, per point.
fn split(full: &[f64], mesh: &Mesh) -> (Vec<[f64; 3]>, Vec<[f64; 3]>, Vec<[f64; 6]>) {
    let l = &mesh.layout;
    let (translation, rotation) = (0..l.nodes())
        .map(|n| {
            let [a, b, c, d, e, f] = directions::<6>(full, l, n);
            ([a, b, c], [d, e, f])
        })
        .unzip();
    (translation, rotation, (0..mesh.points.len()).map(|k| directions(full, l, l.point_slot(k))).collect())
}

/// The matrix pattern of the mesh (node blocks of its elements, trusses and discrete elements)
/// and the order of its nodes for the factorization.
fn pattern(mesh: &Mesh) -> (NodeGraph, Vec<usize>) {
    let graph = NodeGraph::new(mesh.block_count(), mesh.blocks());
    // The order covers the mesh nodes; reference point equations follow them.
    let n = mesh.labels.len();
    let order = if graph.adjacency.len() == n {
        nested_dissection(&mesh.xyz, &graph.adjacency)
    } else {
        let mesh_part: Vec<Vec<usize>> = graph.adjacency.iter().take(n).map(|row| row.iter().copied().filter(|b| *b < n).collect()).collect();
        nested_dissection(&mesh.xyz, &mesh_part)
    };
    (graph, order)
}

/// Warnings for what a step without velocities ignores.
fn ignored_dashpots(mesh: &Mesh, warnings: &mut Vec<String>) {
    let names = mesh.discrete.dashpots();
    if !names.is_empty() {
        warnings.push(format!(
            "dashpots {} act in dynamic steps only; this step ignores them",
            names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
        ));
    }
}

/// Constraint rows, prescribed values and their elimination for a step.
/// The model, its resolved view, the mesh and the step being solved.
struct Setup<'a> {
    model: &'a Model,
    resolved: &'a Resolved,
    mesh: &'a Mesh,
    step: &'a Step,
}

fn reduction(s: &Setup, values: bool, order: &[usize], budget: Budget, warnings: &mut Vec<String>) -> Result<Reduction> {
    let rows = coupling_rows(s.model, s.resolved, s.mesh)?;
    Reduction::new(s.mesh, order, &rows, |active| prescribed(s.model, s.resolved, s.mesh, s.step, values, active, warnings), budget)
}

/// The reduced matrices' memory budget: the factorization's.
fn budget(limits: &Limits, mesh: &Mesh, graph: &NodeGraph, matrices: usize) -> Budget {
    Budget { max_bytes: limits.max_factor_entries.saturating_mul(8), pattern_entries: graph.entries(&mesh.layout), matrices }
}

/// Solve a linear static step: the step named `step`, or the model's only static step.
///
/// A coupled temperature–displacement step is solved here too: its steady temperatures first
/// ([`crate::heat`]), then the equilibrium with their thermal strains. The coupling is one-way
/// (temperature strains the solid; deformation does not heat it), which is the whole of linear
/// quasi-static thermoelasticity without thermoelastic dissipation, so solving the two in turn
/// is the monolithic solution exactly.
pub fn solve_static(model: &Model, step: Option<&str>, limits: &Limits) -> Result<StaticResult> {
    let resolved = model.resolve()?;
    let step = model.select_step(
        step,
        |p| matches!(p, Procedure::Static {} | Procedure::CoupledTemperatureDisplacement {}),
        "static or coupled temperature–displacement",
    )?;
    let mesh = Mesh::new(model, &resolved, limits.max_dofs)?;
    let mut warnings: Vec<String> = resolved.warnings.iter().map(ToString::to_string).chain(mesh.warnings.iter().cloned()).collect();
    warnings.extend(inelastic_warnings(model));
    // The temperature field: the coupled step's own heat solution, or the step's temperature
    // loads.
    let (field, temperature) = match step.procedure {
        Procedure::CoupledTemperatureDisplacement {} => {
            let (labels, t) = crate::heat::temperatures(model, &resolved, step, limits)?;
            let at = |l: &u32| labels.binary_search(l).ok().and_then(|i| t.get(i)).copied();
            (Some(mesh.labels.iter().map(at).collect::<Vec<_>>()), Some(mesh.labels.iter().map(|l| at(l).unwrap_or(0.0)).collect()))
        }
        _ => (temperature_loads(model, &resolved, &mesh, step)?, None),
    };
    if field.is_some() && !mesh.structural.is_empty() {
        warnings.push("shells and beams take no thermal strain yet; the temperatures strain the solid, 2D and truss elements only".into());
    }
    let (graph, order) = pattern(&mesh);
    ignored_dashpots(&mesh, &mut warnings);
    let setup = Setup { model, resolved: &resolved, mesh: &mesh, step };
    let red = reduction(&setup, true, &order, budget(limits, &mesh, &graph, 1), &mut warnings)?;
    let mut f = loads(model, &resolved, &mesh, step, &red.active)?;
    if let Some(field) = &field {
        add_thermal_loads(&mut f, &mesh, field)?;
    }
    let k = assemble(&mesh, &graph, None)?.k;
    if red.neq() == 0 {
        return Err(Error::Invalid("every degree of freedom is prescribed; there is nothing to solve".into()));
    }
    let [kff] = red.reduce([&k])?;
    let chol = factor_raw(&kff, limits).map_err(|e| factor_error(e, &mesh, &red))?;
    drop(kff);
    let mut q = red.rhs(&f, &k);
    chol.solve(&mut q)?;
    let u = red.expand(&q, true);
    if u.iter().any(|x| !x.is_finite()) {
        return Err(Error::Singular("the solution is not finite; the model is ill-conditioned".into()));
    }
    let (displacement, rotation, point_motion) = split(&u, &mesh);
    let reactions = red.reactions(&k, &u, &f);
    let (reaction, reaction_moment, point_reaction) = split(&reactions, &mesh);
    let (mut applied_total, mut reaction_total) = ([0.0; 3], [0.0; 3]);
    for (dof, (fv, rv)) in f.iter().zip(&reactions).enumerate() {
        let Some(d) = mesh.kind(dof).and_then(DofKind::direction) else { continue };
        if let (Some(a), Some(r)) = (applied_total.get_mut(d), reaction_total.get_mut(d)) {
            *a += fv;
            *r += rv;
        }
    }
    let reference_points = mesh
        .points
        .iter()
        .zip(point_motion.iter().zip(&point_reaction))
        .map(|(p, (d, r))| PointResult { name: p.name.clone(), label: p.label, displacement: *d, reaction: *r })
        .collect();
    // Stresses: one extrapolation per element type and rule.
    let mut extrapolations: Vec<(&'static ElementType, Rule, Extrapolation)> = Vec::new();
    let mut elements = Vec::with_capacity(mesh.elements.len());
    let mut sum = vec![[0.0; 6]; mesh.labels.len()];
    let mut count = vec![0u32; mesh.labels.len()];
    for e in &mesh.elements {
        let kernel = e.kernel()?;
        let at = |err| Error::element(e.label, err);
        let ue: Vec<[f64; 3]> = e.nodes.iter().map(|n| displacement.get(*n).copied().unwrap_or_default()).collect();
        let thermal = field.as_deref().and_then(|t| e.thermal_strain(t));
        let points = kernel.stresses_with(&e.d, &ue, thermal.as_ref()).map_err(at)?;
        let values: Vec<[f64; 6]> = points.iter().map(|p| p.stress).collect();
        let (ty, rule) = (kernel.ty(), kernel.rule());
        let cached = extrapolations.iter().position(|(t, r, _)| std::ptr::eq(*t, ty) && *r == rule);
        let index = match cached {
            Some(i) => i,
            None => {
                extrapolations.push((ty, rule, extrapolation(ty, rule).map_err(at)?));
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
    for t in &mesh.trusses {
        let truss = t.truss()?;
        let at = |err| Error::element(t.label, err);
        let ue: Vec<[f64; 3]> = t.nodes.iter().map(|n| displacement.get(*n).copied().unwrap_or_default()).collect();
        let thermal = field.as_deref().and_then(|f| t.thermal_strain(f));
        let points = truss.stresses_with(t.young, &ue, thermal.as_ref()).map_err(at)?;
        let nodal = truss.extrapolate(&points.iter().map(|p| p.stress).collect::<Vec<_>>()).map_err(at)?;
        for (n, s) in t.nodes.iter().zip(&nodal) {
            if let (Some(acc), Some(c)) = (sum.get_mut(*n), count.get_mut(*n)) {
                for k in 0..6 {
                    acc[k] += s[k];
                }
                *c += 1;
            }
        }
        elements.push(ElementStress { label: t.label, kind: t.kind, points, nodal });
    }
    elements.sort_by_key(|e| e.label);
    let (mut shells, mut beams) = (Vec::new(), Vec::new());
    let per_volume = structural_body_forces(model, &resolved, &mesh, step)?;
    for (s, body) in mesh.structural.iter().zip(&per_volume) {
        let at = |err| Error::element(s.label, err);
        let dofs = mesh
            .layout
            .element_dofs(&s.nodes, Structural::KINDS)
            .ok_or_else(|| Error::Internal(format!("element {} degrees of freedom", s.label)))?;
        let ue: Vec<f64> = dofs.iter().map(|d| u.get(*d).copied().unwrap_or(0.0)).collect();
        match s.kernel()? {
            StructuralKernel::Shell(shell) => {
                let stress = shell.stresses(&ue).map_err(at)?;
                for (n, v) in s.nodes.iter().zip(&stress.top) {
                    if let (Some(acc), Some(c)) = (sum.get_mut(*n), count.get_mut(*n)) {
                        for k in 0..6 {
                            acc[k] += v[k];
                        }
                        *c += 1;
                    }
                }
                shells.push(ShellResult { label: s.label, kind: s.kind, stress });
            }
            StructuralKernel::Beam(beam) => beams.push(BeamResult { label: s.label, kind: s.kind, forces: beam.forces(&ue, *body).map_err(at)? }),
        }
    }
    let stress: Vec<[f64; 6]> = sum.iter().zip(&count).map(|(s, c)| s.map(|v| if *c > 0 { v / f64::from(*c) } else { 0.0 })).collect();
    let von_mises = stress.iter().map(von_mises).collect();
    let scale = applied_total.iter().chain(&reaction_total).fold(0.0f64, |m, v| m.max(v.abs()));
    let ground = mesh.discrete.ground_force(&u, &mesh);
    let imbalance = (0..3).map(|c| (applied_total[c] + reaction_total[c] + ground[c]).abs()).fold(0.0, f64::max);
    if scale > 0.0 && imbalance > 1e-6 * scale {
        warnings.push(format!("loads and reactions differ by {imbalance:.3e} (relative {:.1e})", imbalance / scale));
    }
    Ok(StaticResult {
        step: step.name.clone(),
        nodes: mesh.labels.clone(),
        displacement,
        rotation,
        reaction,
        reaction_moment,
        stress,
        von_mises,
        elements,
        shells,
        beams,
        reference_points,
        applied_total,
        reaction_total,
        stats: Stats { dofs: mesh.dofs(), equations: red.neq(), factor_entries: chol.entries(), lanczos_vectors: 0 },
        warnings,
        temperature,
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
    let step = model.select_step(step, |p| matches!(p, Procedure::Frequency { .. }), "frequency")?;
    let Procedure::Frequency { modes, min_hz, max_hz } = step.procedure else {
        return Err(Error::Internal("frequency step".into()));
    };
    if let Some(s) = options.shift
        && !s.is_finite()
    {
        return Err(Error::Invalid(format!("the shift must be a finite number, got {s}")));
    }
    let mesh = Mesh::new(model, &resolved, limits.max_dofs)?;
    let mut warnings: Vec<String> = resolved.warnings.iter().map(ToString::to_string).chain(mesh.warnings.iter().cloned()).collect();
    warnings.extend(inelastic_warnings(model));
    let (graph, order) = pattern(&mesh);
    ignored_dashpots(&mesh, &mut warnings);
    let setup = Setup { model, resolved: &resolved, mesh: &mesh, step };
    let red = reduction(&setup, false, &order, budget(limits, &mesh, &graph, 2), &mut warnings)?;
    let Assembled { k, m } = assemble(&mesh, &graph, Some(options.mass))?;
    let m = m.ok_or_else(|| Error::Internal("mass matrix".into()))?;
    // Every element's mass entries between translations sum to its mass, once per direction
    // (the mean over the directions for a point mass given per direction; rotary inertia, and
    // the translation–rotation coupling of shells and beams, are left out), added in storage
    // order.
    let translation: Vec<bool> = (0..m.n).map(|d| mesh.kind(d).and_then(DofKind::direction).is_some_and(|c| c < 3)).collect();
    let mut translational = 0.0;
    for d in (0..m.n).filter(|d| translation.get(*d).copied().unwrap_or(false)) {
        let (cols, vals) = m.row(d);
        for (c, v) in cols.iter().zip(vals) {
            if translation.get(*c).copied().unwrap_or(false) {
                translational += v;
            }
        }
    }
    let total_mass = translational / mesh.translation_kinds().len() as f64;
    let neq = red.neq();
    if neq == 0 {
        return Err(Error::Invalid("every degree of freedom is held; there is nothing to vibrate".into()));
    }
    let [kff, mff] = red.reduce([&k, &m])?;
    drop((k, m));
    let shifted = |s: f64| -> Result<Factor> { factor_raw(&kff.add_scaled(-s, &mff)?, limits).map_err(|e| factor_error(e, &mesh, &red)) };
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
            Err(e) => return Err(factor_error(e, &mesh, &red)),
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
    // Participation: Γ_d = φᵀ M r_d with r_d the unit translation along d on the free equations
    // (mesh and reference point translations).
    let directions: Vec<Option<usize>> = red.eq_dof.iter().map(|dof| mesh.kind(*dof).and_then(DofKind::direction)).collect();
    let mr: Vec<Vec<f64>> = (0..3)
        .map(|d| {
            let r: Vec<f64> = directions.iter().map(|dir| if *dir == Some(d) { 1.0 } else { 0.0 }).collect();
            let mut y = vec![0.0; neq];
            mff.mul(&r, &mut y);
            y
        })
        .collect();
    let mut out = Vec::with_capacity(pairs.values.len());
    for (lambda, phi) in pairs.values.iter().zip(&pairs.vectors) {
        let participation = [0, 1, 2].map(|d| mr.get(d).map_or(0.0, |y| y.iter().zip(phi).map(|(a, b)| a * b).sum()));
        let (shape, rotation, point_shape) = split(&red.expand(phi, false), &mesh);
        out.push(Mode {
            eigenvalue: *lambda,
            frequency: lambda.max(0.0).sqrt() / two_pi,
            shape,
            rotation,
            participation,
            effective_mass: participation.map(|g| g * g),
            point_shape,
        });
    }
    if out.len() < wanted.count && max_hz.is_none() {
        warnings.push(format!("found {} of the {modes} modes asked for", out.len()));
    }
    Ok(FrequencyResult {
        step: step.name.clone(),
        nodes: mesh.labels.clone(),
        reference_points: mesh.points.iter().map(|p| (p.name.clone(), p.label)).collect(),
        modes: out,
        total_mass,
        stats: Stats { dofs: mesh.dofs(), equations: neq, factor_entries: chol.entries(), lanczos_vectors: pairs.basis },
        warnings,
    })
}

/// Where a label's result is in a [`StepResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Node(usize),
    Point(usize),
}

/// The result of one step, as a session keeps it.
#[derive(Debug, Clone, PartialEq)]
// One per solved step; boxing the larger variant would buy nothing.
#[allow(clippy::large_enum_variant)]
pub enum StepResult {
    Static(StaticResult),
    Frequency(FrequencyResult),
    Heat(crate::heat::HeatResult),
}

impl StepResult {
    pub fn step(&self) -> &str {
        match self {
            StepResult::Static(r) => &r.step,
            StepResult::Frequency(r) => &r.step,
            StepResult::Heat(r) => &r.step,
        }
    }

    pub fn nodes(&self) -> &[u32] {
        match self {
            StepResult::Static(r) => &r.nodes,
            StepResult::Frequency(r) => &r.nodes,
            StepResult::Heat(r) => &r.nodes,
        }
    }

    /// Where a label's result is: a mesh node (index into [`Self::nodes`]) or a coupled
    /// reference point (index into the result's reference points).
    pub fn locate(&self, label: u32) -> Option<Location> {
        if let Ok(i) = self.nodes().binary_search(&label) {
            return Some(Location::Node(i));
        }
        match self {
            StepResult::Static(r) => r.reference_points.iter().position(|p| p.label == label),
            StepResult::Frequency(r) => r.reference_points.iter().position(|p| p.1 == label),
            StepResult::Heat(_) => None,
        }
        .map(Location::Point)
    }
}
