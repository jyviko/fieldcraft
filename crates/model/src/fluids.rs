//! Fluid domains: the box a fluid step solves the flow in.
//!
//! A [`FluidDomain`] is an axis-aligned box filled with a fluid material and divided into cubic
//! cells of one spacing. Each of the six box faces carries a [`FaceCondition`] (a wall, a moving
//! or slip wall, a velocity inlet, a pressure outlet, or a periodic pair). Solid bodies of the
//! geometry placed in the box ([`FluidDomain::obstacles`], by body name) take their cells out of
//! the fluid; their faces are no-slip walls unless a [`SurfaceBoundary`] names one as a moving
//! wall. The forces the fluid exerts on every face are a fluid step's result.
//!
//! Like the rest of the model this is solver-neutral and unit-consistent: lengths, velocities,
//! pressures and accelerations are in the model's unit system, and the solver chooses its own
//! scaled units from them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::geometry::{Entity, GeometryRef};
use crate::{Error, Model, Result, context, finite, insert_new, not_in_use, positive, remove_existing, replace_existing, require};

/// Most obstacle bodies in one domain.
pub const MAX_OBSTACLES: usize = 1000;

/// Most surface conditions in one domain.
pub const MAX_SURFACE_CONDITIONS: usize = 10_000;

/// Most cells along one side of a domain: the cell count is the product of the three, so this
/// bounds every count derived from a domain before the solver applies its own caps.
pub const MAX_CELLS_PER_SIDE: u64 = 1 << 20;

/// How far the box extent may be from a whole number of spacings, relative to the spacing.
const WHOLE_CELLS_TOLERANCE: f64 = 1e-6;

/// A face of the domain box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoxFace {
    XMin,
    XMax,
    YMin,
    YMax,
    ZMin,
    ZMax,
}

impl BoxFace {
    pub const ALL: [BoxFace; 6] = [BoxFace::XMin, BoxFace::XMax, BoxFace::YMin, BoxFace::YMax, BoxFace::ZMin, BoxFace::ZMax];

    /// The axis normal to the face: 0, 1 or 2.
    pub fn axis(self) -> usize {
        match self {
            BoxFace::XMin | BoxFace::XMax => 0,
            BoxFace::YMin | BoxFace::YMax => 1,
            BoxFace::ZMin | BoxFace::ZMax => 2,
        }
    }

    /// Whether the face is at the upper end of its axis.
    pub fn is_max(self) -> bool {
        matches!(self, BoxFace::XMax | BoxFace::YMax | BoxFace::ZMax)
    }

    /// The face across the box.
    pub fn opposite(self) -> BoxFace {
        match self {
            BoxFace::XMin => BoxFace::XMax,
            BoxFace::XMax => BoxFace::XMin,
            BoxFace::YMin => BoxFace::YMax,
            BoxFace::YMax => BoxFace::YMin,
            BoxFace::ZMin => BoxFace::ZMax,
            BoxFace::ZMax => BoxFace::ZMin,
        }
    }

    /// `x_min`, `x_max`, …
    pub fn name(self) -> &'static str {
        match self {
            BoxFace::XMin => "x_min",
            BoxFace::XMax => "x_max",
            BoxFace::YMin => "y_min",
            BoxFace::YMax => "y_max",
            BoxFace::ZMin => "z_min",
            BoxFace::ZMax => "z_max",
        }
    }

    /// The face's index in [`BoxFace::ALL`].
    pub fn index(self) -> usize {
        2 * self.axis() + usize::from(self.is_max())
    }

    /// The face of `axis` (0, 1, 2; larger values mean z) at its upper end when `max`.
    pub fn of(axis: usize, max: bool) -> BoxFace {
        match (axis, max) {
            (0, false) => BoxFace::XMin,
            (0, true) => BoxFace::XMax,
            (1, false) => BoxFace::YMin,
            (1, true) => BoxFace::YMax,
            (_, false) => BoxFace::ZMin,
            (_, true) => BoxFace::ZMax,
        }
    }
}

/// How a velocity inlet's velocity varies over its face.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// The same velocity everywhere on the face.
    #[default]
    Uniform,
    /// The velocity times `4 s (1 − s)` across each direction of the face that ends at walls
    /// (not periodic), `s` from 0 to 1 across the box: fully developed laminar flow between
    /// plates (one such direction) or in a rectangular duct approximated by the product (two).
    /// The given velocity is the peak.
    Parabolic,
}

/// The condition on a face of the domain box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FaceCondition {
    /// A no-slip wall at rest.
    Wall {},
    /// A no-slip wall sliding in its own plane (the velocity has no component normal to the
    /// face).
    MovingWall { velocity: [f64; 3] },
    /// A frictionless wall: no flow through it, no shear.
    Slip {},
    /// A velocity inlet (or outlet, if the velocity points out of the box).
    Velocity {
        velocity: [f64; 3],
        #[serde(default, skip_serializing_if = "is_uniform")]
        profile: Profile,
    },
    /// A pressure outlet (or inlet) at this gauge pressure; the flow crosses it normally.
    Pressure { pressure: f64 },
    /// What leaves through this face enters through the opposite one, which must be periodic too.
    Periodic {},
}

fn is_uniform(p: &Profile) -> bool {
    *p == Profile::Uniform
}

impl Default for FaceCondition {
    fn default() -> Self {
        FaceCondition::Wall {}
    }
}

impl FaceCondition {
    /// `wall`, `moving_wall`, `slip`, `velocity`, `pressure` or `periodic`.
    pub fn name(&self) -> &'static str {
        match self {
            FaceCondition::Wall {} => "wall",
            FaceCondition::MovingWall { .. } => "moving_wall",
            FaceCondition::Slip {} => "slip",
            FaceCondition::Velocity { .. } => "velocity",
            FaceCondition::Pressure { .. } => "pressure",
            FaceCondition::Periodic {} => "periodic",
        }
    }

    fn validate(&self, face: BoxFace) -> Result<()> {
        match *self {
            FaceCondition::Wall {} | FaceCondition::Slip {} | FaceCondition::Periodic {} => Ok(()),
            FaceCondition::MovingWall { velocity } => {
                vector("wall velocity", velocity)?;
                if velocity.get(face.axis()).is_some_and(|v| *v != 0.0) {
                    return Err(Error::Invalid(format!(
                        "a moving wall slides in its own plane: the {} component of its velocity must be 0",
                        ["x", "y", "z"].get(face.axis()).copied().unwrap_or("normal")
                    )));
                }
                Ok(())
            }
            FaceCondition::Velocity { velocity, .. } => vector("inlet velocity", velocity).map(|_| ()),
            FaceCondition::Pressure { pressure } => finite("pressure", pressure).map(|_| ()),
        }
    }
}

/// The conditions on the six faces of the box; every face is a wall unless given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoxFaces {
    pub x_min: FaceCondition,
    pub x_max: FaceCondition,
    pub y_min: FaceCondition,
    pub y_max: FaceCondition,
    pub z_min: FaceCondition,
    pub z_max: FaceCondition,
}

impl BoxFaces {
    pub fn get(&self, face: BoxFace) -> &FaceCondition {
        match face {
            BoxFace::XMin => &self.x_min,
            BoxFace::XMax => &self.x_max,
            BoxFace::YMin => &self.y_min,
            BoxFace::YMax => &self.y_max,
            BoxFace::ZMin => &self.z_min,
            BoxFace::ZMax => &self.z_max,
        }
    }

    pub fn get_mut(&mut self, face: BoxFace) -> &mut FaceCondition {
        match face {
            BoxFace::XMin => &mut self.x_min,
            BoxFace::XMax => &mut self.x_max,
            BoxFace::YMin => &mut self.y_min,
            BoxFace::YMax => &mut self.y_max,
            BoxFace::ZMin => &mut self.z_min,
            BoxFace::ZMax => &mut self.z_max,
        }
    }

    /// Whether the faces of `axis` (0, 1, 2) are a periodic pair.
    pub fn periodic(&self, axis: usize) -> bool {
        BoxFace::ALL.iter().any(|f| f.axis() == axis && matches!(self.get(*f), FaceCondition::Periodic {}))
    }
}

/// The condition on a face of an obstacle body.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SurfaceCondition {
    /// A no-slip wall at rest (every obstacle face is one unless told otherwise).
    Wall {},
    /// A no-slip wall moving with this velocity (tangential motion of a rotating or sliding
    /// surface; the body itself does not move).
    MovingWall { velocity: [f64; 3] },
}

/// A condition on one named face of an obstacle body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceBoundary {
    /// `{"body": "Body1", "face": "F2:top"}`; the face name also selects its split pieces
    /// (`F2:top#0`, …), as in sets.
    pub face: GeometryRef,
    pub condition: SurfaceCondition,
}

/// Where a boundary condition goes: a face of the box, or a named face of an obstacle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FluidBoundary {
    Box(BoxFace),
    Surface(GeometryRef),
}

/// A fluid domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FluidDomain {
    /// A material with a density and fluid behaviour ([`crate::Fluid`]).
    pub material: String,
    /// The box's lower and upper corners.
    pub min: [f64; 3],
    pub max: [f64; 3],
    /// The side of the cubic cells; each extent of the box must be a whole number of spacings.
    pub spacing: f64,
    #[serde(default)]
    pub faces: BoxFaces,
    /// Solid bodies of the geometry in the box (by name): their cells are not fluid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obstacles: Vec<String>,
    /// Conditions on named faces of the obstacles (each face once).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<SurfaceBoundary>,
    /// The fluid's velocity at the start.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub initial_velocity: [f64; 3],
    /// A body acceleration on the fluid (gravity, or a uniform driving force per mass).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub acceleration: [f64; 3],
    /// The chordal tolerance obstacles are triangulated with (default: a tenth of the spacing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f64>,
}

fn is_zero(v: &[f64; 3]) -> bool {
    v.iter().all(|c| *c == 0.0)
}

fn vector(what: &str, v: [f64; 3]) -> Result<[f64; 3]> {
    for c in v {
        finite(what, c)?;
    }
    Ok(v)
}

impl FluidDomain {
    /// The number of cells along each axis, checked to be whole and within
    /// [`MAX_CELLS_PER_SIDE`].
    pub fn cells(&self) -> Result<[u64; 3]> {
        let h = positive("spacing", self.spacing)?;
        let mut n = [0u64; 3];
        for (axis, slot) in n.iter_mut().enumerate() {
            let (lo, hi) = (self.min.get(axis).copied().unwrap_or(0.0), self.max.get(axis).copied().unwrap_or(0.0));
            let name = ["x", "y", "z"].get(axis).copied().unwrap_or("?");
            if !(lo.is_finite() && hi.is_finite() && hi > lo) {
                return Err(Error::Invalid(format!("the box's {name} range {lo}..{hi} is empty or not finite")));
            }
            let cells = (hi - lo) / h;
            let whole = cells.round();
            if !(whole >= 1.0 && whole <= MAX_CELLS_PER_SIDE as f64) {
                return Err(Error::Invalid(format!("the box spans {cells} spacings along {name}; it must span 1 to {MAX_CELLS_PER_SIDE} cells")));
            }
            if (cells - whole).abs() > WHOLE_CELLS_TOLERANCE * whole.max(1.0) {
                return Err(Error::Invalid(format!("the box's {name} extent {} is not a whole number of spacings {h} ({cells} cells)", hi - lo)));
            }
            *slot = whole as u64;
        }
        Ok(n)
    }

    /// Check everything that does not need the rest of the model.
    pub fn validate(&self) -> Result<()> {
        for (what, v) in [("min", self.min), ("max", self.max), ("initial velocity", self.initial_velocity), ("acceleration", self.acceleration)] {
            vector(what, v)?;
        }
        self.cells()?;
        for face in BoxFace::ALL {
            let c = self.faces.get(face);
            c.validate(face).map_err(|e| context("box face", face.name(), e))?;
            let other = self.faces.get(face.opposite());
            // Both faces of a pair are visited, so checking one direction finds every broken pair.
            if matches!(c, FaceCondition::Periodic {}) && !matches!(other, FaceCondition::Periodic {}) {
                return Err(Error::Invalid(format!(
                    "box face {} is periodic but {} is {}: periodic faces come in opposite pairs",
                    face.name(),
                    face.opposite().name(),
                    other.name()
                )));
            }
        }
        if self.obstacles.len() > MAX_OBSTACLES {
            return Err(Error::Limit(format!("a fluid domain holds at most {MAX_OBSTACLES} obstacles")));
        }
        for (i, b) in self.obstacles.iter().enumerate() {
            GeometryRef::body(b.as_str())?;
            if self.obstacles.get(..i).is_some_and(|before| before.contains(b)) {
                return Err(Error::Invalid(format!("obstacle `{b}` is listed twice")));
            }
        }
        if self.surfaces.len() > MAX_SURFACE_CONDITIONS {
            return Err(Error::Limit(format!("a fluid domain holds at most {MAX_SURFACE_CONDITIONS} surface conditions")));
        }
        for (i, s) in self.surfaces.iter().enumerate() {
            if !matches!(s.face.entity, Entity::Face(_)) {
                return Err(Error::Invalid(format!("surface conditions go on faces, not on the {}", s.face)));
            }
            if !self.obstacles.contains(&s.face.body) {
                return Err(Error::Invalid(format!(
                    "the {} is not on an obstacle of the domain (list body `{}` in `obstacles`)",
                    s.face, s.face.body
                )));
            }
            if let SurfaceCondition::MovingWall { velocity } = s.condition {
                vector("wall velocity", velocity)?;
            }
            if self.surfaces.get(..i).is_some_and(|before| before.iter().any(|o| o.face == s.face)) {
                return Err(Error::Invalid(format!("the {} has two conditions", s.face)));
            }
        }
        if let Some(t) = self.tolerance {
            positive("tolerance", t)?;
        }
        Ok(())
    }

    /// Set the condition on one boundary: a box face, or a named face of an obstacle, which
    /// takes a wall or a moving wall (a wall there removes the entry, walls being the default).
    /// The caller validates the result ([`Model::set_fluid_boundary`] does).
    pub fn set_boundary(&mut self, at: FluidBoundary, condition: FaceCondition) -> Result<()> {
        match at {
            FluidBoundary::Box(face) => *self.faces.get_mut(face) = condition,
            FluidBoundary::Surface(face) => {
                let c = match condition {
                    FaceCondition::Wall {} => SurfaceCondition::Wall {},
                    FaceCondition::MovingWall { velocity } => SurfaceCondition::MovingWall { velocity },
                    other => return Err(Error::Invalid(format!("the {face} takes a wall or a moving wall, not {}", other.name()))),
                };
                self.surfaces.retain(|s| s.face != face);
                if c != (SurfaceCondition::Wall {}) {
                    self.surfaces.push(SurfaceBoundary { face, condition: c });
                }
            }
        }
        Ok(())
    }
}

impl Model {
    pub fn fluid_domains(&self) -> &BTreeMap<String, FluidDomain> {
        &self.fluid_domains
    }

    pub fn fluid_domain(&self, name: &str) -> Option<&FluidDomain> {
        self.fluid_domains.get(name)
    }

    fn check_fluid_domain(&self, d: &FluidDomain) -> Result<()> {
        d.validate()?;
        require(&self.materials, "material", &d.material)?;
        match self.materials.get(&d.material) {
            Some(m) if m.fluid.is_none() => Err(Error::Invalid(format!("material `{}` has no fluid behaviour (a viscosity)", d.material))),
            Some(m) if m.density.is_none() => Err(Error::Invalid(format!("material `{}` has no density", d.material))),
            _ => Ok(()),
        }
    }

    pub fn add_fluid_domain(&mut self, name: &str, domain: FluidDomain) -> Result<()> {
        self.check_fluid_domain(&domain).map_err(|e| context("fluid domain", name, e))?;
        insert_new(&mut self.fluid_domains, "fluid domain", name, domain)
    }

    pub fn replace_fluid_domain(&mut self, name: &str, domain: FluidDomain) -> Result<()> {
        self.check_fluid_domain(&domain).map_err(|e| context("fluid domain", name, e))?;
        replace_existing(&mut self.fluid_domains, "fluid domain", name, domain)
    }

    pub fn remove_fluid_domain(&mut self, name: &str) -> Result<()> {
        let users = self.steps.iter().filter(|s| s.procedure.fluid_domain() == Some(name)).map(|s| format!("step `{}`", s.name)).collect();
        not_in_use("fluid domain", name, users)?;
        remove_existing(&mut self.fluid_domains, "fluid domain", name).map(|_| ())
    }

    /// Set the condition on one boundary of a domain.
    pub fn set_fluid_boundary(&mut self, name: &str, at: FluidBoundary, condition: FaceCondition) -> Result<()> {
        let mut d = self.fluid_domains.get(name).cloned().ok_or_else(|| Error::NotFound(format!("fluid domain `{name}`")))?;
        d.set_boundary(at, condition).map_err(|e| context("fluid domain", name, e))?;
        self.replace_fluid_domain(name, d)
    }

    /// The fluid domains that use a material.
    pub(crate) fn fluid_domain_users(&self, material: &str) -> impl Iterator<Item = &str> {
        self.fluid_domains.iter().filter(move |(_, d)| d.material == material).map(|(n, _)| n.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fluid, Material, Procedure, Step};

    fn water() -> Material {
        Material { density: Some(1.0e-9), fluid: Some(Fluid::Newtonian { viscosity: 1.0e-9 }), ..Material::default() }
    }

    pub(crate) fn channel() -> FluidDomain {
        FluidDomain {
            material: "Water".into(),
            min: [0.0, 0.0, 0.0],
            max: [10.0, 2.0, 1.0],
            spacing: 0.5,
            faces: BoxFaces {
                x_min: FaceCondition::Velocity { velocity: [1.0, 0.0, 0.0], profile: Profile::Parabolic },
                x_max: FaceCondition::Pressure { pressure: 0.0 },
                z_min: FaceCondition::Periodic {},
                z_max: FaceCondition::Periodic {},
                ..BoxFaces::default()
            },
            obstacles: vec![],
            surfaces: vec![],
            initial_velocity: [0.0; 3],
            acceleration: [0.0; 3],
            tolerance: None,
        }
    }

    #[test]
    fn domains_validate_and_refer_to_a_fluid_material() {
        let mut m = Model::default();
        assert!(m.add_fluid_domain("Flow", channel()).is_err(), "no material yet");
        m.add_material("Water", water()).unwrap();
        m.add_fluid_domain("Flow", channel()).unwrap();
        assert_eq!(m.fluid_domain("Flow").unwrap().cells().unwrap(), [20, 4, 2]);

        let bad = |f: &dyn Fn(&mut FluidDomain)| {
            let mut d = channel();
            f(&mut d);
            d
        };
        for d in [
            bad(&|d| d.spacing = 0.0),
            bad(&|d| d.spacing = f64::NAN),
            bad(&|d| d.spacing = 0.3),
            bad(&|d| d.max[0] = -1.0),
            bad(&|d| d.min[1] = f64::INFINITY),
            bad(&|d| d.spacing = 1e-300),
            bad(&|d| d.faces.z_max = FaceCondition::Wall {}),
            bad(&|d| d.faces.y_min = FaceCondition::MovingWall { velocity: [1.0, 1.0, 0.0] }),
            bad(&|d| d.faces.x_min = FaceCondition::Velocity { velocity: [f64::NAN, 0.0, 0.0], profile: Profile::Uniform }),
            bad(&|d| d.faces.x_max = FaceCondition::Pressure { pressure: f64::INFINITY }),
            bad(&|d| d.obstacles = vec!["A".into(), "A".into()]),
            bad(&|d| d.obstacles = vec![String::new()]),
            bad(&|d| d.surfaces = vec![SurfaceBoundary { face: GeometryRef::face("B", "f").unwrap(), condition: SurfaceCondition::Wall {} }]),
            bad(&|d| {
                d.obstacles = vec!["B".into()];
                d.surfaces = vec![SurfaceBoundary { face: GeometryRef::body("B").unwrap(), condition: SurfaceCondition::Wall {} }];
            }),
            bad(&|d| d.tolerance = Some(0.0)),
            bad(&|d| d.acceleration = [0.0, f64::NAN, 0.0]),
        ] {
            assert!(m.add_fluid_domain("Bad", d.clone()).is_err(), "{d:?}");
        }
        let mut solid = water();
        solid.fluid = None;
        m.add_material("Steel", Material { elastic: None, density: Some(7.85e-9), fluid: None }).unwrap();
        assert!(m.add_fluid_domain("Dry", FluidDomain { material: "Steel".into(), ..channel() }).is_err());
        assert!(m.replace_material("Water", solid).is_err(), "a domain needs the fluid behaviour");
        assert!(matches!(m.remove_material("Water"), Err(Error::InUse(_))));

        // A step refers to the domain, which then cannot be deleted.
        let fluid = Procedure::Fluid { domain: "Flow".into(), end_time: Some(1.0), steps: None, time_step: None, frames: 4, magic: None };
        m.add_step(Step::new("Run", fluid.clone()), None).unwrap();
        assert!(matches!(m.remove_fluid_domain("Flow"), Err(Error::InUse(_))));
        // The JSON form reads back to the same model, through the same checks.
        let text = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Model>(&text).unwrap(), m);
        assert!(serde_json::from_str::<Model>(&text.replace("\"spacing\":0.5", "\"spacing\":0.3")).is_err());
        // Fluid steps take nothing structural, and need a domain and one length.
        let mut listed = Step::new("Listed", fluid.clone());
        listed.output.insert(crate::OutputVariable::Stress);
        assert!(m.add_step(listed, None).is_err());
        for bad in [
            Procedure::Fluid { domain: "Nope".into(), end_time: Some(1.0), steps: None, time_step: None, frames: 4, magic: None },
            Procedure::Fluid { domain: "Flow".into(), end_time: Some(1.0), steps: Some(5), time_step: None, frames: 4, magic: None },
            Procedure::Fluid { domain: "Flow".into(), end_time: None, steps: None, time_step: None, frames: 4, magic: None },
            Procedure::Fluid { domain: "Flow".into(), end_time: Some(-1.0), steps: None, time_step: None, frames: 4, magic: None },
            Procedure::Fluid { domain: "Flow".into(), end_time: None, steps: Some(0), time_step: None, frames: 4, magic: None },
            Procedure::Fluid { domain: "Flow".into(), end_time: Some(1.0), steps: None, time_step: Some(0.0), frames: 4, magic: None },
            Procedure::Fluid { domain: "Flow".into(), end_time: Some(1.0), steps: None, time_step: None, frames: 0, magic: None },
            Procedure::Fluid { domain: "Flow".into(), end_time: Some(1.0), steps: None, time_step: None, frames: 4, magic: Some(0.0) },
            Procedure::Fluid { domain: "Flow".into(), end_time: Some(1.0), steps: None, time_step: None, frames: 4, magic: Some(f64::NAN) },
        ] {
            assert!(m.add_step(Step::new("Bad", bad.clone()), None).is_err(), "{bad:?}");
        }
        m.remove_step("Run").unwrap();
        m.remove_fluid_domain("Flow").unwrap();
        m.remove_material("Water").unwrap();
    }

    #[test]
    fn boundaries_are_set_one_at_a_time() {
        let mut m = Model::default();
        m.add_material("Water", water()).unwrap();
        let mut d = channel();
        d.obstacles = vec!["Ball".into()];
        m.add_fluid_domain("Flow", d).unwrap();
        m.set_fluid_boundary("Flow", FluidBoundary::Box(BoxFace::YMax), FaceCondition::Slip {}).unwrap();
        assert_eq!(m.fluid_domain("Flow").unwrap().faces.y_max, FaceCondition::Slip {});
        let face = GeometryRef::face("Ball", "F1").unwrap();
        let spin = FaceCondition::MovingWall { velocity: [0.0, 1.0, 0.0] };
        m.set_fluid_boundary("Flow", FluidBoundary::Surface(face.clone()), spin).unwrap();
        assert_eq!(m.fluid_domain("Flow").unwrap().surfaces.len(), 1);
        m.set_fluid_boundary("Flow", FluidBoundary::Surface(face.clone()), FaceCondition::Wall {}).unwrap();
        assert!(m.fluid_domain("Flow").unwrap().surfaces.is_empty(), "walls are the default");
        // Wrong kinds and broken pairs are refused and change nothing.
        let before = m.clone();
        assert!(m.set_fluid_boundary("Flow", FluidBoundary::Box(BoxFace::ZMin), FaceCondition::Wall {}).is_err());
        assert!(m.set_fluid_boundary("Flow", FluidBoundary::Surface(face), FaceCondition::Slip {}).is_err(), "obstacle faces are walls");
        assert!(m.set_fluid_boundary("Nope", FluidBoundary::Box(BoxFace::XMin), FaceCondition::Wall {}).is_err());
        assert_eq!(m, before);

        // The JSON forms.
        let at: FluidBoundary = serde_json::from_value(serde_json::json!("x_min")).unwrap();
        assert_eq!(at, FluidBoundary::Box(BoxFace::XMin));
        let at: FluidBoundary = serde_json::from_value(serde_json::json!({"body": "Ball", "face": "F1"})).unwrap();
        assert!(matches!(at, FluidBoundary::Surface(_)));
        assert!(serde_json::from_value::<FaceCondition>(serde_json::json!({"type": "wall", "velocity": [1, 0, 0]})).is_err());
    }
}
