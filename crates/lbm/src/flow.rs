//! The lattice Boltzmann state and one time step: collide, stream, open boundaries.
//!
//! # Scheme
//! - **Lattice and equilibrium:** D3Q19 with the second-order equilibrium
//!   (Qian, d'Humières & Lallemand 1992; Krüger et al. 2017, ch. 3).
//! - **Collision:** two relaxation times (Ginzburg, Verhaeghe & d'Humières 2008): the symmetric
//!   part of each opposite pair relaxes with `1/τ⁺` (which sets the viscosity), the antisymmetric
//!   part with `1/τ⁻`, related by the parameter `Λ = (τ⁺ − ½)(τ⁻ − ½)`. With `Λ = 3/16`
//!   halfway bounce-back puts the wall exactly halfway between nodes and steady solutions do not
//!   depend on the viscosity.
//! - **Body force:** Guo, Zheng & Shi 2002 (PRE 65:046308): the velocity is
//!   `(Σ f c + F/2) / ρ` and the forcing term is split into its symmetric part, relaxed with
//!   `1 − 1/(2τ⁺)`, and its antisymmetric part, with `1 − 1/(2τ⁻)` (the TRT form of Guo's
//!   term, Krüger et al. 2017, ch. 10).
//! - **Walls:** halfway bounce-back at every link from a fluid cell to a solid cell or across a
//!   wall face of the box; moving walls add Ladd's term `6 w_q ρ₀ c_q·u_w` (Ladd 1994, JFM
//!   271:285). Slip faces reflect populations specularly.
//! - **Inlets and outlets:** Zou & He 1997 (Phys. Fluids 9:1591) on the first layer of cells: the
//!   unknown incoming populations are the opposite ones plus the equilibrium difference
//!   (bounce-back of the non-equilibrium part), the density (velocity inlet) or the normal velocity
//!   (pressure outlet) follow from mass and normal momentum, and the transverse momentum is
//!   corrected on the diagonal unknowns as M. Hecht and J. Harting do for D3Q19 (*Implementation
//!   of on-site velocity boundary conditions for D3Q19 lattice Boltzmann simulations*, J. Stat.
//!   Mech. 2010, P01018). The prescribed velocity is the one with the half force of Guo's scheme.
//! - **Forces:** momentum exchange on every wall link (Ladd 1994; R. Mei, D. Yu, W. Shyy, L.-S.
//!   Luo, *Force evaluation in the lattice Boltzmann method involving curved geometry*, PRE
//!   65:041203, 2002): a population `f*` leaving a fluid cell toward the wall and the one `f'`
//!   coming back transfer `c_out f* − c_in f'` to the wall.
//!
//! # Layout and determinism
//! Two arrays of populations, structure of arrays (`f[q n + cell]`), A–B pattern. Streaming
//! pulls: every population of a cell comes from one source (a neighbour, a periodic image, the
//! cell itself through a wall, or an open boundary). Cells whose 18 neighbours are all fluid
//! cells inside the box stream by one shifted copy per direction; the others follow per-link
//! rules. Every loop runs in a fixed order with no parallel reductions, so the result is the
//! same bit for bit on every platform.

use modelcraft_model::{BoxFace, FaceCondition, Profile};

use modelcraft_parallel as parallel;

use crate::lattice::{C, Q, W, c, equilibrium, moving_wall, opposite};
use crate::setup::{MAX_LATTICE_SPEED, Obstacle, Setup};
use crate::voxel::{Grid, TriangleTree, voxelise};
use crate::{Error, Result};

/// Where a population of a cell next to a wall or box face comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Rule {
    /// The same direction from another cell (a neighbour or a periodic image).
    Pull { from: u32 },
    /// Bounce-back at a wall of force group `group`; `du` is the moving-wall term.
    Bounce { group: u32, du: f64 },
    /// Specular reflection at a slip face: direction `q` from cell `from`.
    Reflect { from: u32, q: u8, group: u32 },
    /// From outside an inlet or outlet: set by the open boundary after streaming.
    Open,
}

/// A population, direction `q` of `cell`, and where it comes from.
#[derive(Debug, Clone, Copy)]
struct Link {
    cell: u32,
    q: u8,
    rule: Rule,
}

/// An inlet or outlet cell.
#[derive(Debug, Clone, Copy)]
struct OpenCell {
    cell: u32,
    axis: usize,
    /// +1 at a min face (the inward normal points up the axis), −1 at a max face.
    inward: f64,
    target: Target,
}

#[derive(Debug, Clone, Copy)]
enum Target {
    /// The velocity (lattice units).
    Velocity([f64; 3]),
    /// The density.
    Density(f64),
}

/// A force group: a box face or a face of an obstacle.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// `x_min` … for box faces; the face name for obstacle faces.
    pub name: String,
    /// The obstacle body, for obstacle faces.
    pub body: Option<String>,
}

/// The flow state.
#[derive(Debug, Clone)]
pub struct Flow {
    grid: Grid,
    n: usize,
    /// The distance between two directions' arrays in `f` and `g` (at least `n`).
    stride: usize,
    fluid: Vec<bool>,
    f: Vec<f64>,
    g: Vec<f64>,
    /// The links that do not stream by the shifted copies, in cell order.
    links: Vec<Link>,
    open: Vec<OpenCell>,
    omega_plus: f64,
    omega_minus: f64,
    acceleration: [f64; 3],
    /// The momentum transferred to each group in the last step (lattice units).
    forces: Vec<[f64; 3]>,
    groups: Vec<Group>,
    /// Per direction, the index offset of the neighbour a population streams from.
    offset: [isize; Q],
    fluid_cells: usize,
    steps_done: u64,
    /// Obstacles that cover no cell centre (smaller than the spacing, or outside the box).
    empty_obstacles: Vec<String>,
}

const NONE: u32 = u32::MAX;

/// Cells per parallel block of the collision and the streaming. The blocks are fixed, every
/// cell's update reads and writes only its own populations (collision) or writes only its own
/// (streaming), so the result does not depend on the number of threads.
const CELLS: usize = 4096;

/// The first `n` populations of each direction of an array with direction stride `stride`, cut
/// into blocks of [`CELLS`] cells: block b holds the Q directions' slices of cells
/// b·CELLS.. .
fn cell_blocks(data: &mut [f64], stride: usize, n: usize) -> Vec<Vec<&mut [f64]>> {
    let mut blocks: Vec<Vec<&mut [f64]>> = (0..n.div_ceil(CELLS)).map(|_| Vec::with_capacity(Q)).collect();
    for direction in data.chunks_mut(stride.max(1)).take(Q) {
        for (block, cells) in blocks.iter_mut().zip(direction.get_mut(..n).unwrap_or_default().chunks_mut(CELLS)) {
            block.push(cells);
        }
    }
    blocks
}

/// A vector allocated without aborting when memory runs out.
fn filled<T: Clone>(len: usize, value: T, what: &str) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error::Limit(format!("not enough memory for the {what} ({len} entries)")))?;
    v.resize(len, value);
    Ok(v)
}

impl Flow {
    /// Build the grid (voxelising the obstacles), the streaming rules and the initial state (at
    /// rest or the domain's initial velocity, density 1).
    pub fn new(setup: &Setup, obstacles: &[Obstacle]) -> Result<Flow> {
        let grid = setup.grid;
        let [nx, ny, nz] = grid.cells;
        let n = grid.len();
        if n == 0 || n >= NONE as usize {
            return Err(Error::Limit(format!("{n} cells cannot be addressed")));
        }
        // Each direction's array starts an odd number of cache lines (8 doubles) after the
        // previous one, so the 19 arrays never fall into the same cache sets (a power-of-two grid
        // would otherwise put them all in one).
        let lines = n.div_ceil(8);
        let stride = 8 * (lines + 1 - lines % 2);
        let q_n = stride.checked_mul(Q).ok_or_else(|| Error::Limit("the grid is too large".into()))?;

        // Solid cells, obstacle by obstacle (the first obstacle that covers a cell owns it).
        let mut fluid = filled(n, true, "cell flags")?;
        let mut owner = filled(n, NONE, "cell owners")?;
        let mut trees = Vec::with_capacity(setup.obstacles.len());
        let mut empty_obstacles = Vec::new();
        // Force groups: the six box faces, then every face of every obstacle.
        let mut groups: Vec<Group> = BoxFace::ALL.iter().map(|f| Group { name: f.name().to_string(), body: None }).collect();
        let mut base = Vec::with_capacity(setup.obstacles.len());
        for (k, faces) in setup.obstacles.iter().enumerate() {
            let o = obstacles.get(faces.index).ok_or_else(|| Error::NotFound(format!("obstacle {}", faces.index)))?;
            let mut inside = filled(n, false, "obstacle cells")?;
            if voxelise(&o.surface, &grid, &mut inside)? == 0 {
                empty_obstacles.push(o.body.clone());
            }
            for (c, &solid) in inside.iter().enumerate() {
                if solid && owner[c] == NONE {
                    owner[c] = k as u32;
                    fluid[c] = false;
                }
            }
            trees.push((TriangleTree::new(&o.surface), o.surface.tri_face()));
            base.push(groups.len() as u32);
            groups.extend(faces.names.iter().map(|name| Group { name: name.clone(), body: Some(o.body.clone()) }));
        }

        let domain = &setup.domain;
        let periodic = [domain.faces.periodic(0), domain.faces.periodic(1), domain.faces.periodic(2)];
        let wall_velocity = |face: BoxFace| match *domain.faces.get(face) {
            FaceCondition::MovingWall { velocity } => Some(setup.to_lattice(velocity)),
            FaceCondition::Wall {} => Some([0.0; 3]),
            _ => None,
        };
        let is_slip = |face: BoxFace| matches!(domain.faces.get(face), FaceCondition::Slip {});
        let dims = [nx as i64, ny as i64, nz as i64];
        let index = |p: [i64; 3]| grid.index([p[0] as usize, p[1] as usize, p[2] as usize]) as u32;
        let mut offset = [0isize; Q];
        for (q, o) in offset.iter_mut().enumerate() {
            let cq = C[q];
            *o = cq[0] as isize + nx as isize * (cq[1] as isize + ny as isize * cq[2] as isize);
        }

        // The group and moving-wall term of a link into fluid cell `cell` (direction q) from the
        // obstacle cell `solid`: the face of the triangle nearest the link's midpoint.
        let obstacle_link = |cell: usize, solid: u32, q: usize| -> Rule {
            let k = owner.get(solid as usize).copied().unwrap_or(NONE) as usize;
            let (Some((tree, tags)), Some(faces), Some(&first)) = (trees.get(k), setup.obstacles.get(k), base.get(k)) else {
                return Rule::Bounce { group: 0, du: 0.0 };
            };
            let (centre, cq) = (grid.centre(cell), c(q));
            let mid = [0, 1, 2].map(|a| centre[a] - 0.5 * cq[a] * grid.spacing);
            let tag = tree.nearest(mid).and_then(|t| tags.get(t as usize).copied()).unwrap_or(0);
            let u = faces.velocities.get(tag as usize).copied().unwrap_or([0.0; 3]);
            Rule::Bounce { group: first + tag, du: moving_wall(q, u) }
        };

        // The links that do not stream from the neighbour the shifted copies read.
        let mut links: Vec<Link> = Vec::new();
        let mut fluid_cells = 0;
        for cell in 0..n {
            if !fluid[cell] {
                continue;
            }
            fluid_cells += 1;
            let ijk = grid.coords(cell);
            let p = [ijk[0] as i64, ijk[1] as i64, ijk[2] as i64];
            for q in 1..Q {
                let cq = C[q];
                let mut s = [p[0] - i64::from(cq[0]), p[1] - i64::from(cq[1]), p[2] - i64::from(cq[2])];
                let mut wrapped = false;
                let mut crossed: [Option<BoxFace>; 3] = [None; 3];
                for a in 0..3 {
                    if s[a] < 0 || s[a] >= dims[a] {
                        if periodic[a] {
                            s[a] = s[a].rem_euclid(dims[a]);
                            wrapped = true;
                        } else {
                            crossed[a] = Some(BoxFace::of(a, s[a] >= dims[a]));
                        }
                    }
                }
                let crossed_faces = || crossed.iter().flatten().copied();
                let rule = match crossed_faces().next() {
                    None => {
                        let from = index(s);
                        match (fluid.get(from as usize).copied().unwrap_or(false), wrapped) {
                            (true, false) => continue,
                            (true, true) => Rule::Pull { from },
                            (false, _) => obstacle_link(cell, from, q),
                        }
                    }
                    Some(_) => {
                        if let Some(face) = crossed_faces().find(|f| wall_velocity(*f).is_some()) {
                            Rule::Bounce { group: face.index() as u32, du: moving_wall(q, wall_velocity(face).unwrap_or([0.0; 3])) }
                        } else if let Some(face) = crossed_faces().find(|f| is_slip(*f)) {
                            // The mirror source: the same cell layer, the tangential components shifted.
                            let a = face.axis();
                            let mut m = s;
                            m[a] = p[a];
                            let mut mirrored = C[q];
                            mirrored[a] = -mirrored[a];
                            let q2 = C.iter().position(|v| *v == mirrored).unwrap_or(opposite(q));
                            let single = crossed_faces().count() == 1;
                            let inside = (0..3).all(|b| m[b] >= 0 && m[b] < dims[b]);
                            match inside.then(|| index(m)) {
                                Some(from) if single && fluid.get(from as usize).copied().unwrap_or(false) => {
                                    Rule::Reflect { from, q: q2 as u8, group: face.index() as u32 }
                                }
                                _ => Rule::Bounce { group: face.index() as u32, du: 0.0 },
                            }
                        } else {
                            Rule::Open
                        }
                    }
                };
                links.push(Link { cell: cell as u32, q: q as u8, rule });
            }
        }

        // Inlets and outlets: the fluid cells of each open face's layer.
        let mut open = Vec::new();
        for face in BoxFace::ALL {
            let a = face.axis();
            let layer = if face.is_max() { grid.cells[a] - 1 } else { 0 };
            let (t1, t2) = ((a + 1) % 3, (a + 2) % 3);
            for j in 0..grid.cells[t2] {
                for i in 0..grid.cells[t1] {
                    let mut ijk = [0usize; 3];
                    ijk[a] = layer;
                    ijk[t1] = i;
                    ijk[t2] = j;
                    let cell = grid.index(ijk);
                    if !fluid[cell] {
                        continue;
                    }
                    let target = match *domain.faces.get(face) {
                        FaceCondition::Velocity { velocity, profile } => {
                            let mut scale = 1.0;
                            if profile == Profile::Parabolic {
                                for t in [t1, t2].into_iter().filter(|t| !periodic[*t]) {
                                    let s = (ijk[t] as f64 + 0.5) / grid.cells[t] as f64;
                                    scale *= 4.0 * s * (1.0 - s);
                                }
                            }
                            Target::Velocity(setup.to_lattice(velocity).map(|v| v * scale))
                        }
                        FaceCondition::Pressure { pressure } => Target::Density(setup.lattice_density(pressure)),
                        _ => break,
                    };
                    open.push(OpenCell { cell: cell as u32, axis: a, inward: if face.is_max() { -1.0 } else { 1.0 }, target });
                }
            }
        }

        // The initial state: equilibrium at density 1 and the initial velocity.
        let mut f = filled(q_n, 0.0, "populations")?;
        let g = filled(q_n, 0.0, "populations")?;
        for cell in 0..n {
            for q in 0..Q {
                f[q * stride + cell] = if fluid[cell] { equilibrium(q, 1.0, setup.initial_velocity) } else { 0.0 };
            }
        }
        Ok(Flow {
            grid,
            n,
            stride,
            fluid,
            f,
            g,
            links,
            open,
            omega_plus: 1.0 / setup.tau_plus,
            omega_minus: 1.0 / setup.tau_minus,
            acceleration: setup.acceleration,
            forces: vec![[0.0; 3]; groups.len()],
            groups,
            offset,
            fluid_cells,
            steps_done: 0,
            empty_obstacles,
        })
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn fluid_cells(&self) -> usize {
        self.fluid_cells
    }

    pub fn is_fluid(&self, cell: usize) -> bool {
        self.fluid.get(cell).copied().unwrap_or(false)
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The momentum each group received in the last step (lattice units: force per step).
    pub fn forces(&self) -> &[[f64; 3]] {
        &self.forces
    }

    /// Obstacles that cover no cell centre.
    pub fn empty_obstacles(&self) -> &[String] {
        &self.empty_obstacles
    }

    pub fn steps_done(&self) -> u64 {
        self.steps_done
    }

    /// Set every fluid cell to the equilibrium of `state(cell) = (density, velocity)` in lattice
    /// units (velocity as defined with the half force).
    pub fn set_equilibrium(&mut self, state: impl Fn(usize) -> (f64, [f64; 3])) {
        let n = self.n;
        let s = self.stride;
        for cell in 0..n {
            if !self.fluid[cell] {
                continue;
            }
            let (rho, u) = state(cell);
            let bare = [0, 1, 2].map(|a| u[a] - 0.5 * self.acceleration[a]);
            for q in 0..Q {
                self.f[q * s + cell] = equilibrium(q, rho, bare);
            }
        }
    }

    /// The density and velocity of a cell (lattice units), or `None` for a solid cell.
    pub fn moments(&self, cell: usize) -> Option<(f64, [f64; 3])> {
        if !self.is_fluid(cell) {
            return None;
        }
        let s = self.stride;
        let (mut rho, mut j) = (0.0, [0.0; 3]);
        for q in 0..Q {
            let fq = self.f.get(q * s + cell).copied().unwrap_or(0.0);
            let cq = c(q);
            rho += fq;
            j[0] += fq * cq[0];
            j[1] += fq * cq[1];
            j[2] += fq * cq[2];
        }
        let u = [0, 1, 2].map(|a| (j[a] + 0.5 * rho * self.acceleration[a]) / rho);
        Some((rho, u))
    }

    /// The total mass of the fluid cells (lattice units: the sum of every population).
    pub fn mass(&self) -> f64 {
        let n = self.n;
        let s = self.stride;
        let mut m = 0.0;
        for cell in (0..n).filter(|c| self.fluid[*c]) {
            for q in 0..Q {
                m += self.f[q * s + cell];
            }
        }
        m
    }

    /// One time step. Fails with [`Error::Unstable`] when a density stops being a positive
    /// number or a lattice speed exceeds [`MAX_LATTICE_SPEED`].
    pub fn step(&mut self) -> Result<()> {
        let max_u2 = self.collide();
        if !(max_u2 <= MAX_LATTICE_SPEED * MAX_LATTICE_SPEED) {
            return Err(Error::Unstable(format!(
                "the flow went unstable at time step {}: the lattice speed reached {:.3} (limit {MAX_LATTICE_SPEED}) or a density stopped being a positive number; lower the time step, refine the spacing or check the boundary conditions",
                self.steps_done + 1,
                max_u2.sqrt()
            )));
        }
        self.stream();
        self.open_boundaries();
        std::mem::swap(&mut self.f, &mut self.g);
        self.steps_done += 1;
        Ok(())
    }

    /// Two-relaxation-time collision with Guo forcing, in place, by blocks of [`CELLS`] cells
    /// in parallel. Returns the largest squared lattice speed, or NaN when a density is not
    /// positive.
    fn collide(&mut self) -> f64 {
        let n = self.n;
        let (wp, wm) = (self.omega_plus, self.omega_minus);
        let (sp, sm) = (1.0 - 0.5 * wp, 1.0 - 0.5 * wm);
        let a = self.acceleration;
        let forced = a != [0.0; 3];
        let fluid = &self.fluid;
        let mut blocks = cell_blocks(&mut self.f, self.stride, n);
        let mut maxima = vec![0.0f64; blocks.len()];
        let mut work: Vec<(&mut Vec<&mut [f64]>, &mut f64)> = blocks.iter_mut().zip(maxima.iter_mut()).collect();
        parallel::for_each_mut(&mut work, |b, (f, max_u2)| {
            let mut fq = [0.0; Q];
            let len = f.first().map_or(0, |d| d.len());
            for i in 0..len {
                if !fluid.get(b * CELLS + i).copied().unwrap_or(false) {
                    continue;
                }
                let (mut rho, mut j) = (0.0, [0.0; 3]);
                for q in 0..Q {
                    let v = f.get(q).and_then(|d| d.get(i)).copied().unwrap_or(0.0);
                    fq[q] = v;
                    let cq = C[q];
                    rho += v;
                    j[0] += v * f64::from(cq[0]);
                    j[1] += v * f64::from(cq[1]);
                    j[2] += v * f64::from(cq[2]);
                }
                if !(rho > 0.0 && rho.is_finite()) {
                    **max_u2 = f64::NAN;
                    return;
                }
                let force = [rho * a[0], rho * a[1], rho * a[2]];
                let u = [(j[0] + 0.5 * force[0]) / rho, (j[1] + 0.5 * force[1]) / rho, (j[2] + 0.5 * force[2]) / rho];
                let uu = u[0] * u[0] + u[1] * u[1] + u[2] * u[2];
                **max_u2 = max_u2.max(uu);
                let uf = u[0] * force[0] + u[1] * force[1] + u[2] * force[2];
                let mut put = |q: usize, v: f64| {
                    if let Some(x) = f.get_mut(q).and_then(|d| d.get_mut(i)) {
                        *x = v;
                    }
                };
                // Rest population: symmetric only.
                let eq0 = W[0] * rho * (1.0 - 1.5 * uu);
                let mut out0 = fq[0] - wp * (fq[0] - eq0);
                if forced {
                    out0 += sp * W[0] * (-3.0 * uf);
                }
                put(0, out0);
                for q in (1..Q).step_by(2) {
                    let o = q + 1;
                    let cq = C[q];
                    let cv = [f64::from(cq[0]), f64::from(cq[1]), f64::from(cq[2])];
                    let cu = cv[0] * u[0] + cv[1] * u[1] + cv[2] * u[2];
                    let w = W[q];
                    let eq_plus = w * rho * (1.0 + 4.5 * cu * cu - 1.5 * uu);
                    let eq_minus = w * rho * 3.0 * cu;
                    let plus = 0.5 * (fq[q] + fq[o]);
                    let minus = 0.5 * (fq[q] - fq[o]);
                    let relax_plus = wp * (plus - eq_plus);
                    let relax_minus = wm * (minus - eq_minus);
                    let (mut s_plus, mut s_minus) = (0.0, 0.0);
                    if forced {
                        let cf = cv[0] * force[0] + cv[1] * force[1] + cv[2] * force[2];
                        s_plus = sp * w * (9.0 * cu * cf - 3.0 * uf);
                        s_minus = sm * w * 3.0 * cf;
                    }
                    put(q, fq[q] - relax_plus - relax_minus + s_plus + s_minus);
                    put(o, fq[o] - relax_plus + relax_minus + s_plus - s_minus);
                }
            }
        });
        // The largest speed of all blocks (a maximum does not depend on the order), or NaN.
        if maxima.iter().any(|m| m.is_nan()) { f64::NAN } else { maxima.iter().fold(0.0, |m, x| m.max(*x)) }
    }

    /// Pull streaming into `g`, with the wall rules (blocks of [`CELLS`] destination cells in
    /// parallel), then the momentum exchange (in link order).
    fn stream(&mut self) {
        let n = self.n;
        let s = self.stride;
        let (f, links, offset) = (&self.f, &self.links, &self.offset);
        let mut blocks = cell_blocks(&mut self.g, s, n);
        parallel::for_each_mut(&mut blocks, |b, g| {
            let start = b * CELLS;
            // Every direction as one shifted copy: right for cells whose neighbours are all fluid
            // cells in the box; the others are overwritten by their rules below.
            for (q, dst) in g.iter_mut().enumerate() {
                let off = offset.get(q).copied().unwrap_or(0);
                let src = f.get(q * s..q * s + n).unwrap_or_default();
                for (i, x) in dst.iter_mut().enumerate() {
                    if let Some(from) = (start + i).checked_add_signed(-off)
                        && let Some(v) = src.get(from)
                    {
                        *x = *v;
                    }
                }
            }
            let first = links.partition_point(|l| (l.cell as usize) < start);
            for link in links.get(first..).unwrap_or_default().iter().take_while(|l| (l.cell as usize) < start + CELLS) {
                let (cell, q) = (link.cell as usize, link.q as usize);
                let v = match link.rule {
                    Rule::Pull { from } => f.get(q * s + from as usize).copied(),
                    Rule::Bounce { du, .. } => f.get(opposite(q) * s + cell).map(|out| out + du),
                    Rule::Reflect { from, q: q2, .. } => f.get(q2 as usize * s + from as usize).copied(),
                    // A placeholder the open boundary replaces.
                    Rule::Open => f.get(opposite(q) * s + cell).copied(),
                };
                if let (Some(v), Some(x)) = (v, g.get_mut(q).and_then(|d| d.get_mut(cell - start))) {
                    *x = v;
                }
            }
        });
        self.forces.fill([0.0; 3]);
        for link in &self.links {
            let (cell, q) = (link.cell as usize, link.q as usize);
            let cq = c(q);
            match link.rule {
                Rule::Bounce { group, du } => {
                    let out = self.f[opposite(q) * s + cell];
                    let back = out + du;
                    if let Some(force) = self.forces.get_mut(group as usize) {
                        // c_out f* − c_in f' with c_out = −c_q.
                        for a in 0..3 {
                            force[a] -= cq[a] * (out + back);
                        }
                    }
                }
                Rule::Reflect { from, q: q2, group } => {
                    let q2 = q2 as usize;
                    let out = self.f[q2 * s + from as usize];
                    let c2 = c(q2);
                    if let Some(force) = self.forces.get_mut(group as usize) {
                        for a in 0..3 {
                            force[a] += c2[a] * out - cq[a] * out;
                        }
                    }
                }
                Rule::Pull { .. } | Rule::Open => {}
            }
        }
    }

    /// Zou–He velocity and pressure boundaries on the streamed populations.
    fn open_boundaries(&mut self) {
        let s = self.stride;
        let a_half = self.acceleration.map(|v| 0.5 * v);
        for oc in &self.open {
            let cell = oc.cell as usize;
            let axis = oc.axis;
            let sign = oc.inward;
            let ci = |q: usize| f64::from(C[q][axis]);
            // Tangential (c_n = 0) and outgoing (c_n = −s, known) sums.
            let (mut tangential, mut outgoing) = (0.0, 0.0);
            for q in 0..Q {
                let v = self.g[q * s + cell];
                let cn = ci(q) * sign;
                if cn == 0.0 {
                    tangential += v;
                } else if cn < 0.0 {
                    outgoing += v;
                }
            }
            // The bare velocity (without the half force) the populations must carry.
            let (rho, bare) = match oc.target {
                Target::Velocity(u) => {
                    let bare = [u[0] - a_half[0], u[1] - a_half[1], u[2] - a_half[2]];
                    let un = sign * bare[axis];
                    ((tangential + 2.0 * outgoing) / (1.0 - un), bare)
                }
                Target::Density(rho) => {
                    let un = 1.0 - (tangential + 2.0 * outgoing) / rho;
                    let mut bare = [-a_half[0], -a_half[1], -a_half[2]];
                    bare[axis] = sign * un;
                    (rho, bare)
                }
            };
            // Unknowns: bounce-back of the non-equilibrium part.
            for q in (1..Q).filter(|q| ci(*q) * sign > 0.0) {
                let cq = c(q);
                let cu = cq[0] * bare[0] + cq[1] * bare[1] + cq[2] * bare[2];
                self.g[q * s + cell] = self.g[opposite(q) * s + cell] + 6.0 * W[q] * rho * cu;
            }
            // Transverse momentum: correct the two diagonal unknowns along each tangential axis.
            for t in (0..3).filter(|t| *t != axis) {
                let mut m = -rho * bare[t];
                for q in 0..Q {
                    m += self.g[q * s + cell] * f64::from(C[q][t]);
                }
                for q in (1..Q).filter(|q| ci(*q) * sign > 0.0 && C[*q][t] != 0) {
                    self.g[q * s + cell] -= 0.5 * m * f64::from(C[q][t]);
                }
            }
        }
    }
}
