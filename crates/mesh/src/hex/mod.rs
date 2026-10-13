//! Structured all-hexahedral meshing of sweepable and mappable bodies.
//!
//! [`hex_mesh`] meshes one body's boundary surface (face-tagged, from surface preparation) with
//! hexahedra only, by the first method that fits:
//! 1. **Extrude** ([`layered`]): a straight multi-level sweep: every face is across the direction
//!    (a source or target level) or along it (a linking wall), possibly at several levels with
//!    regions of different heights (stepped blocks, ribs on plates, shelled boxes). The cross-
//!    section is imprinted, quad-meshed region by region and stacked layer by layer.
//! 2. **Sweep** ([`sweep`]): a one-to-one sweep between a planar source and a planar target face
//!    through rings of logically rectangular linking faces (bent tubes, lofts, partial
//!    revolutions): linking faces get transfinite grids projected onto their surfaces, and every
//!    layer's interior nodes come from the source mesh by the least-squares affine map of the
//!    boundary rings (Roca and Sarrate) with boundary residual correction (Knupp).
//! 3. **Revolve** ([`layered`] in a rotating frame): bodies of revolution whose cross-section does
//!    not reach the axis (rings, tori, grooved pulleys).
//! 4. **Mapped** ([`mapped`]): a logical cube (six four-sided faces, three edges at each of eight
//!    corners), meshed by transfinite interpolation of its face grids (Gordon and Thiel).
//!
//! A body none of these fits is [`Error::Unsupported`] with each method's reason: the input for
//! decomposition into sweepable and mappable regions.
//!
//! **Common steps.** Interval assignment ([`interval`]) picks the division counts of shared curves
//! together. Boundary nodes on analytic faces are placed on the exact surface; quadratic hexahedra
//! (hex20) get their mid-edge nodes projected onto the faces (and onto both faces of a B-rep
//! edge). Every boundary quadrilateral is tagged with the B-rep face it lies on, so model sets
//! named by face resolve. The output passes the certified validity gate (every element's
//! Jacobian determinant proven positive, [`crate::quality`]), a tiling check (the elements'
//! volume equals the volume their boundary encloses, so nothing overlaps), and a fidelity check
//! (the volume differs from the boundary surface's by no more than the gap between the mesh
//! boundary and the surface accounts for).

mod arrangement;
mod brep;
mod geom;
mod interval;
mod layered;
mod mapped;
mod region;
mod sweep;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

pub use geom::Surface;

use brep::Brep;
use geom::{TriangleTree, add, dist, scale, unit};

use crate::quad::QuadOptions;
use crate::quality::{GateOptions, GateReport, Metric, element_quality, validity_gate_checks};
use crate::size::SizeField;
use crate::surface::BoundarySurface;
use crate::vec3::{P, cross, dot, sub};
use crate::{Error, Result};
use modelcraft_model::ElementKind;

/// Which structured method to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HexMethod {
    /// The first method that fits, in the order extrude, sweep, revolve, mapped.
    Auto,
    /// The first sweeping method that fits: extrude, sweep, revolve.
    Swept,
    Extrude,
    Sweep,
    Revolve,
    Mapped,
}

impl HexMethod {
    pub fn name(self) -> &'static str {
        match self {
            HexMethod::Auto => "auto",
            HexMethod::Swept => "swept",
            HexMethod::Extrude => "extrude",
            HexMethod::Sweep => "sweep",
            HexMethod::Revolve => "revolve",
            HexMethod::Mapped => "mapped",
        }
    }

    pub fn from_name(name: &str) -> Option<HexMethod> {
        [HexMethod::Auto, HexMethod::Swept, HexMethod::Extrude, HexMethod::Sweep, HexMethod::Revolve, HexMethod::Mapped]
            .into_iter()
            .find(|m| m.name() == name)
    }
}

/// Options of [`hex_mesh`].
#[derive(Debug, Clone, PartialEq)]
pub struct HexOptions {
    pub method: HexMethod,
    /// Hex20 (mid-edge nodes on the geometry) instead of hex8.
    pub quadratic: bool,
    /// The chordal tolerance the boundary surface was tessellated with (0 when exact): copies of a
    /// curve at different levels closer than this are one curve, and the fidelity check allows
    /// it.
    pub chordal_tolerance: f64,
    /// The most hexahedra ([`Error::Limit`] beyond).
    pub max_elements: usize,
    /// Options of the face meshers.
    pub quad: QuadOptions,
}

impl Default for HexOptions {
    fn default() -> Self {
        HexOptions { method: HexMethod::Auto, quadratic: false, chordal_tolerance: 0.0, max_elements: 5_000_000, quad: QuadOptions::default() }
    }
}

/// A boundary face of the mesh and the B-rep face it lies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryFace {
    /// Element index (0-based, into [`HexMesh::element`]).
    pub element: u32,
    /// Face number (1-based, as in [`ElementKind::faces`]).
    pub face: u8,
    /// The B-rep face tag (`tri_face`).
    pub brep_face: u32,
}

/// How the mesh was made and how good it is.
#[derive(Debug, Clone, PartialEq)]
pub struct HexReport {
    pub method: HexMethod,
    /// What the method found (direction, levels, regions, layers, …).
    pub description: String,
    /// Smallest and mean scaled Jacobian (Knupp) over the elements.
    pub min_scaled_jacobian: f64,
    pub mean_scaled_jacobian: f64,
    /// The elements' total volume, and the volume the boundary surface encloses.
    pub volume: f64,
    pub surface_volume: f64,
    /// The validity gate's report.
    pub gate: GateReport,
}

/// An all-hexahedral mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct HexMesh {
    pub nodes: Vec<[f64; 3]>,
    /// [`ElementKind::Hex8`] or [`ElementKind::Hex20`].
    pub kind: ElementKind,
    /// Element node indices, `kind.node_count()` per element, in the model's node order.
    pub connectivity: Vec<u32>,
    pub boundary: Vec<BoundaryFace>,
    pub report: HexReport,
}

impl HexMesh {
    pub fn element_count(&self) -> usize {
        self.connectivity.len() / self.kind.node_count()
    }

    /// The node indices of element `e`.
    pub fn element(&self, e: usize) -> Option<&[u32]> {
        let n = self.kind.node_count();
        self.connectivity.get(e * n..(e + 1) * n)
    }
}

/// What every method gets.
pub(crate) struct Ctx<'a> {
    pub brep: &'a Brep<'a>,
    pub tree: &'a TriangleTree<'a>,
    pub field: &'a dyn SizeField,
    pub opts: &'a HexOptions,
    /// Classification tolerance (model units).
    pub tol: f64,
}

/// What a method produces: linear hexahedra in the model's node order.
pub(crate) struct Raw {
    pub nodes: Vec<P>,
    pub hexes: Vec<[u32; 8]>,
    pub description: String,
}

/// Refinements of the cross-section curves tried when a face mesher fails (each halves them).
const MAX_REFINE: u32 = 2;

/// Classification tolerance relative to the bounding-box diagonal.
const RELATIVE_TOLERANCE: f64 = 1e-6;

/// Mesh the solid bounded by `surface` with hexahedra. `faces[tag]` is the analytic surface of
/// B-rep face `tag` when known (exact node placement; free-form faces use the triangles);
/// `field` sets the element size. A body no structured method fits is [`Error::Unsupported`]
/// with the reasons.
pub fn hex_mesh(surface: &BoundarySurface, faces: &[Option<Surface>], field: &dyn SizeField, opts: &HexOptions) -> Result<HexMesh> {
    if !(opts.chordal_tolerance >= 0.0 && opts.chordal_tolerance.is_finite()) {
        return Err(Error::Invalid(format!("the chordal tolerance must be a non-negative number, got {}", opts.chordal_tolerance)));
    }
    if surface.shells().len() != 1 {
        return Err(Error::Unsupported(format!(
            "the boundary has {} shells; structured meshing takes one solid without internal voids",
            surface.shells().len()
        )));
    }
    // A mesh far beyond the element limit is refused before any work: the solid's volume over
    // the cube of the smallest size the field asks for at the boundary.
    let mut h_min = f64::INFINITY;
    for p in surface.points() {
        h_min = h_min.min(crate::size::checked_size(field, *p)?);
    }
    let estimate = surface.volume().abs() / h_min.powi(3);
    if !(estimate <= 8.0 * opts.max_elements as f64) {
        return Err(Error::Limit(format!("about {estimate:.3e} hexahedra at this size; the limit is {}", opts.max_elements)));
    }
    let brep = Brep::new(surface, faces)?;
    let tree = TriangleTree::new(surface)?;
    let ctx = Ctx { brep: &brep, tree: &tree, field, opts, tol: RELATIVE_TOLERANCE * brep.diagonal };
    let methods: &[HexMethod] = match opts.method {
        HexMethod::Auto => &[HexMethod::Extrude, HexMethod::Sweep, HexMethod::Revolve, HexMethod::Mapped],
        HexMethod::Swept => &[HexMethod::Extrude, HexMethod::Sweep, HexMethod::Revolve],
        HexMethod::Extrude => &[HexMethod::Extrude],
        HexMethod::Sweep => &[HexMethod::Sweep],
        HexMethod::Revolve => &[HexMethod::Revolve],
        HexMethod::Mapped => &[HexMethod::Mapped],
    };
    let mut reasons: Vec<String> = Vec::new();
    for &method in methods {
        // The ways this method can try the body: frames for the layered sweeps, one otherwise.
        let frames: Vec<Option<layered::Frame>> = match method {
            HexMethod::Extrude | HexMethod::Revolve => {
                let frames = if method == HexMethod::Extrude { layered::translations(&brep) } else { layered::rotations(&brep) };
                // Frames the body sweeps in, fewest levels first.
                let mut fitting: Vec<(usize, usize, layered::Frame)> = Vec::new();
                let mut first_reason = None;
                for (k, f) in frames.into_iter().enumerate() {
                    match layered::levels(&ctx, &f) {
                        Ok(n) => fitting.push((n, k, f)),
                        Err(e) => {
                            first_reason.get_or_insert(e);
                        }
                    }
                }
                fitting.sort_by_key(|x| (x.0, x.1));
                if fitting.is_empty() {
                    let none = if method == HexMethod::Extrude { "no direction to try" } else { "no axis to try" };
                    reasons.push(format!("{}: {}", method.name(), first_reason.unwrap_or_else(|| none.to_string())));
                    continue;
                }
                fitting.into_iter().take(layered::MAX_CANDIDATES).map(|(_, _, f)| Some(f)).collect()
            }
            _ => vec![None],
        };
        let mut last = None;
        for frame in frames {
            // A face mesher that fails on a coarse boundary gets finer cross-section curves.
            for refine in 0..=MAX_REFINE {
                let raw = match (method, frame) {
                    (HexMethod::Extrude | HexMethod::Revolve, Some(f)) => layered::mesh(&ctx, &f, refine),
                    (HexMethod::Sweep, _) => sweep::mesh(&ctx, refine),
                    (HexMethod::Mapped, _) => mapped::mesh(&ctx),
                    _ => Err(Error::Failed("no frame".into())),
                };
                match raw.and_then(|raw| finish(&ctx, raw, method)) {
                    Ok(mesh) => return Ok(mesh),
                    Err(e @ Error::Limit(_)) => return Err(e),
                    Err(e @ Error::Failed(_)) if refine < MAX_REFINE && method != HexMethod::Mapped => last = Some(e),
                    Err(e) => {
                        last = Some(e);
                        break;
                    }
                }
            }
        }
        if let Some(e) = last {
            reasons.push(format!("{}: {}", method.name(), plain(&e)));
        }
    }
    Err(Error::Unsupported(format!("no structured hex method fits this body ({})", reasons.join("; "))))
}

/// An error's message without the "not supported yet" prefix (for the combined reasons).
fn plain(e: &Error) -> String {
    match e {
        Error::Unsupported(m) => m.clone(),
        other => other.to_string(),
    }
}

/// Hex face `k` (0-based) corner positions in the element.
fn hex_face(k: usize) -> [usize; 4] {
    let f = ElementKind::Hex8.faces().get(k).copied().unwrap_or(&[0, 1, 2, 3]);
    [f.first().copied().unwrap_or(0), f.get(1).copied().unwrap_or(0), f.get(2).copied().unwrap_or(0), f.get(3).copied().unwrap_or(0)]
}

/// Tag, check, upgrade to hex20 when asked, gate and measure.
fn finish(ctx: &Ctx, raw: Raw, method: HexMethod) -> Result<HexMesh> {
    let Raw { mut nodes, hexes, description } = raw;
    if hexes.is_empty() {
        return Err(Error::Failed("the method produced no elements".into()));
    }
    if hexes.len() > ctx.opts.max_elements {
        return Err(Error::Limit(format!("the mesh has {} hexahedra; the limit is {}", hexes.len(), ctx.opts.max_elements)));
    }
    modelcraft_model::check_coordinates(nodes.as_flattened())?;
    let at = |nodes: &[P], i: u32| nodes.get(i as usize).copied().ok_or_else(|| Error::Failed(format!("node {i} out of range")));
    // Boundary faces: hex faces used once.
    let mut uses: HashMap<[u32; 4], u32> = HashMap::with_capacity(hexes.len() * 3);
    for h in &hexes {
        for k in 0..6 {
            let mut key = hex_face(k).map(|i| h.get(i).copied().unwrap_or(0));
            key.sort_unstable();
            *uses.entry(key).or_insert(0) += 1;
        }
    }
    if let Some(n) = uses.values().copied().max().filter(|&n| n > 2) {
        return Err(Error::Failed(format!("a hexahedron face is shared by {n} elements (the mesh is not a manifold)")));
    }
    let mut boundary = Vec::new();
    let mut fidelity = 0.0;
    for (e, h) in hexes.iter().enumerate() {
        for k in 0..6 {
            let corners = hex_face(k).map(|i| h.get(i).copied().unwrap_or(0));
            let mut key = corners;
            key.sort_unstable();
            if uses.get(&key) != Some(&1) {
                continue;
            }
            let [a, b, c, d] = [at(&nodes, corners[0])?, at(&nodes, corners[1])?, at(&nodes, corners[2])?, at(&nodes, corners[3])?];
            let centre = scale(add(add(a, b), add(c, d)), 0.25);
            let n = cross(sub(c, a), sub(d, b));
            let area = 0.5 * dot(n, n).sqrt();
            let Some(n) = unit(n) else { return Err(Error::Failed(format!("element {e} has a degenerate boundary face"))) };
            let hit = ctx
                .tree
                .closest(centre, &|t| dot(ctx.tree.normal(t), n) > 0.0)
                .ok_or_else(|| Error::Failed(format!("no boundary face faces the same way as element {e}'s face {}", k + 1)))?;
            let f = ctx.brep.face_of_triangle(hit.triangle).ok_or_else(|| Error::Failed("triangle without a face".into()))?;
            fidelity += area * (hit.dist + ctx.opts.chordal_tolerance);
            boundary.push(BoundaryFace { element: e as u32, face: k as u8 + 1, brep_face: ctx.brep.face_tag(f) });
        }
    }
    let kind = if ctx.opts.quadratic { ElementKind::Hex20 } else { ElementKind::Hex8 };
    let mut connectivity: Vec<u32> = Vec::with_capacity(hexes.len() * kind.node_count());
    if ctx.opts.quadratic {
        let mids = mid_edge_nodes(ctx, &mut nodes, &hexes, &boundary)?;
        for (h, m) in hexes.iter().zip(&mids) {
            connectivity.extend_from_slice(h);
            connectivity.extend_from_slice(m);
        }
    } else {
        for h in &hexes {
            connectivity.extend_from_slice(h);
        }
    }
    let npe = kind.node_count();
    let coords = |e: usize| -> Result<Vec<P>> { connectivity.get(e * npe..(e + 1) * npe).unwrap_or(&[]).iter().map(|&i| at(&nodes, i)).collect() };
    // The volume the mesh's own boundary encloses: equal to the elements' total exactly when they
    // tile it without overlapping.
    let mut enclosed = 0.0;
    for b in &boundary {
        let x = coords(b.element as usize)?;
        let local = kind.faces().get(b.face as usize - 1).copied().unwrap_or(&[]);
        let face: Vec<P> = local.iter().filter_map(|&i| x.get(i).copied()).collect();
        enclosed += face_flux(&face);
    }
    // One certified pass: every element's quality, with its Jacobian check for the gate.
    let (mut min_sj, mut sum_sj) = (f64::INFINITY, 0.0);
    let mut checks = Vec::with_capacity(hexes.len());
    for e in 0..hexes.len() {
        let q = element_quality(kind, &coords(e)?)?;
        let sj = q.get(Metric::ScaledJacobian).unwrap_or(-1.0);
        min_sj = min_sj.min(sj);
        sum_sj += sj;
        checks.push((e as u32 + 1, q.jacobian.ok_or_else(|| Error::Failed("an element was not certified".into()))?));
    }
    let gate = validity_gate_checks(checks.iter().map(|(l, c)| (*l, c)), &GateOptions { expected_volume: Some(enclosed), volume_tolerance: 1e-9 })?
        .into_result()
        .map_err(|e| Error::Failed(format!("{} ({description})", plain(&e))))?;
    let surface_volume = ctx.brep.surface.volume();
    let slack = 2.0 * fidelity + 1e-9 * surface_volume.abs();
    if (gate.volume - surface_volume).abs() > slack {
        return Err(Error::Failed(format!(
            "the mesh volume {} differs from the boundary's {surface_volume} by more than the gap between them allows ({slack}): the mesh does not fill the body ({description})",
            gate.volume
        )));
    }
    let report = HexReport {
        method,
        description,
        min_scaled_jacobian: min_sj,
        mean_scaled_jacobian: sum_sj / hexes.len().max(1) as f64,
        volume: gate.volume,
        surface_volume,
        gate,
    };
    Ok(HexMesh { nodes, kind, connectivity, boundary, report })
}

/// `∫ x n_x dA` over a quad4 or quad8 face (outward normal): its contribution to the enclosed
/// volume. The 3 × 3 Gauss rule integrates it exactly (degree at most 5 per direction).
fn face_flux(face: &[P]) -> f64 {
    const G: [(f64, f64); 3] = [(-0.774_596_669_241_483_4, 5.0 / 9.0), (0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)];
    let mut sum = 0.0;
    for (r, wr) in G {
        for (s, ws) in G {
            let (n, dr, ds) = quad_shape(face.len(), r, s);
            let (mut x, mut xr, mut xs) = ([0.0; 3], [0.0; 3], [0.0; 3]);
            for (k, p) in face.iter().enumerate() {
                let (a, b, c) = (n.get(k).copied().unwrap_or(0.0), dr.get(k).copied().unwrap_or(0.0), ds.get(k).copied().unwrap_or(0.0));
                x = add(x, scale(*p, a));
                xr = add(xr, scale(*p, b));
                xs = add(xs, scale(*p, c));
            }
            sum += wr * ws * x[0] * cross(xr, xs)[0];
        }
    }
    sum
}

/// Shape functions and derivatives of quad4 (4 nodes) or quad8 (8 nodes) at `(r, s)`.
fn quad_shape(n: usize, r: f64, s: f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    const C: [(f64, f64); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    if n == 8 {
        let mut v: Vec<f64> = C.iter().map(|(ri, si)| 0.25 * (1.0 + ri * r) * (1.0 + si * s) * (ri * r + si * s - 1.0)).collect();
        let mut dr: Vec<f64> = C.iter().map(|(ri, si)| 0.25 * ri * (1.0 + si * s) * (2.0 * ri * r + si * s)).collect();
        let mut ds: Vec<f64> = C.iter().map(|(ri, si)| 0.25 * si * (1.0 + ri * r) * (ri * r + 2.0 * si * s)).collect();
        // Mid-side nodes on sides 0-1 (s = −1), 1-2 (r = 1), 2-3 (s = 1), 3-0 (r = −1).
        v.extend([
            0.5 * (1.0 - r * r) * (1.0 - s),
            0.5 * (1.0 + r) * (1.0 - s * s),
            0.5 * (1.0 - r * r) * (1.0 + s),
            0.5 * (1.0 - r) * (1.0 - s * s),
        ]);
        dr.extend([-r * (1.0 - s), 0.5 * (1.0 - s * s), -r * (1.0 + s), -0.5 * (1.0 - s * s)]);
        ds.extend([-0.5 * (1.0 - r * r), -s * (1.0 + r), 0.5 * (1.0 - r * r), -s * (1.0 - r)]);
        (v, dr, ds)
    } else {
        let v = C.iter().map(|(ri, si)| 0.25 * (1.0 + ri * r) * (1.0 + si * s)).collect();
        let dr = C.iter().map(|(ri, si)| 0.25 * ri * (1.0 + si * s)).collect();
        let ds = C.iter().map(|(ri, si)| 0.25 * si * (1.0 + ri * r)).collect();
        (v, dr, ds)
    }
}

/// Projection rounds for a point on several faces (alternating projections onto each).
const PROJECTION_ROUNDS: usize = 24;

/// The closest point to `p` on B-rep face index `f`: its analytic surface, or its triangles.
pub(crate) fn project_to_face(ctx: &Ctx, f: usize, p: P) -> Option<P> {
    let face = ctx.brep.faces.get(f)?;
    if let Some(q) = face.surface.and_then(|s| s.project(p)) {
        return Some(q);
    }
    let tag = face.id;
    let tri_face = ctx.brep.surface.tri_face();
    ctx.tree.closest(p, &|t| tri_face.get(t as usize) == Some(&tag)).map(|h| h.point)
}

/// The point on every listed face (their common curve or corner) nearest `p`, by alternating
/// projections.
pub(crate) fn project_to_faces(ctx: &Ctx, faces: &[usize], p: P) -> P {
    let mut x = p;
    for _ in 0..PROJECTION_ROUNDS {
        let before = x;
        for &f in faces {
            if let Some(q) = project_to_face(ctx, f, x) {
                x = q;
            }
        }
        if faces.len() < 2 || dist(before, x) <= ctx.tol * 1e-3 {
            break;
        }
    }
    x
}

/// Mid-edge nodes of every hexahedron (Hex20 order: positions 8..20), shared between elements;
/// boundary edges get theirs on the B-rep faces of the boundary quads around them.
fn mid_edge_nodes(ctx: &Ctx, nodes: &mut Vec<P>, hexes: &[[u32; 8]], boundary: &[BoundaryFace]) -> Result<Vec<[u32; 12]>> {
    // B-rep faces at each boundary edge.
    let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for b in boundary {
        let Some(h) = hexes.get(b.element as usize) else { continue };
        let c = hex_face(b.face as usize - 1).map(|i| h.get(i).copied().unwrap_or(0));
        let f = ctx.brep.faces.binary_search_by_key(&b.brep_face, |x| x.id).map_err(|_| Error::Failed("boundary tag".into()))?;
        for k in 0..4 {
            let (a, z) = (c[k], c[(k + 1) % 4]);
            let list = edge_faces.entry((a.min(z), a.max(z))).or_default();
            if !list.contains(&f) {
                list.push(f);
            }
        }
    }
    let edges = ElementKind::Hex20.edges();
    let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
    let mut out = Vec::with_capacity(hexes.len());
    for h in hexes {
        let mut m = [0u32; 12];
        for (k, e) in edges.iter().enumerate() {
            let (Some(&i), Some(&j)) = (e.first().and_then(|&x| h.get(x)), e.get(1).and_then(|&x| h.get(x))) else {
                return Err(Error::Failed("hex20 edge table".into()));
            };
            let key = (i.min(j), i.max(j));
            let id = match mid.get(&key) {
                Some(&id) => id,
                None => {
                    let (a, b) = (nodes.get(i as usize).copied().unwrap_or([0.0; 3]), nodes.get(j as usize).copied().unwrap_or([0.0; 3]));
                    let p = scale(add(a, b), 0.5);
                    let p = match edge_faces.get(&key) {
                        Some(fs) => project_to_faces(ctx, fs, p),
                        None => p,
                    };
                    nodes.push(p);
                    let id = nodes.len() as u32 - 1;
                    mid.insert(key, id);
                    id
                }
            };
            if let Some(slot) = m.get_mut(k) {
                *slot = id;
            }
        }
        out.push(m);
    }
    modelcraft_model::check_coordinates(nodes.as_flattened())?;
    Ok(out)
}
