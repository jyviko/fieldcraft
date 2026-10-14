//! What the results display shows ([`ResultsView`], set by the `results.*` commands) and the
//! nodal values it draws: displacements, mode shapes, explicit field frames and stress fields,
//! read from a solved step's [`StepResult`]. An explicit frame's one-point element stresses are
//! averaged to the nodes (unweighted, like the implicit solver's nodal stresses).
//!
//! The view is session state, like the camera: changing it is not an edit, so it is neither
//! journalled nor undone, and it does not change the session revision. A result kept from an
//! earlier revision is still shown, marked stale ([`Session::latest_result`]).

use modelcraft_explicit::{ExplicitResult, Frame};
use modelcraft_model::Model;
use modelcraft_solver::{FrequencyResult, HeatResult, StaticResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, StepResult};

/// A result by its procedure.
pub enum Kind<'a> {
    Static(&'a StaticResult),
    Frequency(&'a FrequencyResult),
    Explicit(&'a ExplicitResult),
    /// Per cell, not per node: no nodal fields to display yet.
    Fluid(&'a modelcraft_lbm::FluidResult),
    Heat(&'a HeatResult),
}

impl StepResult {
    pub fn kind(&self) -> Kind<'_> {
        match self {
            StepResult::Implicit(modelcraft_solver::StepResult::Static(r)) => Kind::Static(r),
            StepResult::Implicit(modelcraft_solver::StepResult::Frequency(r)) => Kind::Frequency(r),
            StepResult::Implicit(modelcraft_solver::StepResult::Heat(r)) => Kind::Heat(r),
            StepResult::Explicit(r) => Kind::Explicit(r),
            StepResult::Fluid(r) => Kind::Fluid(r),
        }
    }
}

/// Which part of a result is shown: the mode (from 1, frequency steps) and the field frame
/// (explicit steps; `None` is the last).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pick {
    pub mode: usize,
    pub frame: Option<usize>,
}

/// The temperature frame of a heat result `pick` asks for.
pub fn heat_frame(r: &HeatResult, pick: Pick) -> Result<(f64, &[f64])> {
    let last = r.frames.len().checked_sub(1).ok_or_else(|| EngineError::Params(format!("step `{}` has no temperatures", r.step)))?;
    let k = pick.frame.unwrap_or(last);
    match (r.times.get(k), r.frames.get(k)) {
        (Some(t), Some(f)) => Ok((*t, f)),
        _ => Err(EngineError::Params(format!("step `{}` has frames 0 to {last}; there is no frame {k}", r.step))),
    }
}

/// The field frame of an explicit result `pick` asks for.
pub fn frame(r: &ExplicitResult, pick: Pick) -> Result<&Frame> {
    let last = r.frames.len().checked_sub(1).ok_or_else(|| EngineError::Params(format!("step `{}` has no field frames", r.step)))?;
    let k = pick.frame.unwrap_or(last);
    r.frames.get(k).ok_or_else(|| EngineError::Params(format!("step `{}` has frames 0 to {last}; there is no frame {k}", r.step)))
}

/// A nodal quantity drawn as contours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Displacement magnitude (a mode's shape magnitude for a frequency step).
    Displacement,
    DisplacementX,
    DisplacementY,
    DisplacementZ,
    /// Velocity magnitude (explicit steps).
    Velocity,
    VonMises,
    MaxPrincipal,
    MidPrincipal,
    MinPrincipal,
    StressXx,
    StressYy,
    StressZz,
    StressXy,
    StressXz,
    StressYz,
    /// Temperature (heat transfer and coupled steps).
    Temperature,
    /// Heat flux magnitude (heat transfer steps, at the end of the step).
    HeatFlux,
}

impl Field {
    pub const ALL: [Field; 17] = [
        Field::Displacement,
        Field::DisplacementX,
        Field::DisplacementY,
        Field::DisplacementZ,
        Field::Velocity,
        Field::VonMises,
        Field::MaxPrincipal,
        Field::MidPrincipal,
        Field::MinPrincipal,
        Field::StressXx,
        Field::StressYy,
        Field::StressZz,
        Field::StressXy,
        Field::StressXz,
        Field::StressYz,
        Field::Temperature,
        Field::HeatFlux,
    ];

    /// The id commands take (`von_mises`…).
    pub fn id(self) -> &'static str {
        match self {
            Field::Displacement => "displacement",
            Field::DisplacementX => "displacement_x",
            Field::DisplacementY => "displacement_y",
            Field::DisplacementZ => "displacement_z",
            Field::Velocity => "velocity",
            Field::VonMises => "von_mises",
            Field::MaxPrincipal => "max_principal",
            Field::MidPrincipal => "mid_principal",
            Field::MinPrincipal => "min_principal",
            Field::StressXx => "stress_xx",
            Field::StressYy => "stress_yy",
            Field::StressZz => "stress_zz",
            Field::StressXy => "stress_xy",
            Field::StressXz => "stress_xz",
            Field::StressYz => "stress_yz",
            Field::Temperature => "temperature",
            Field::HeatFlux => "heat_flux",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Field::Displacement => "Displacement magnitude",
            Field::DisplacementX => "Displacement X",
            Field::DisplacementY => "Displacement Y",
            Field::DisplacementZ => "Displacement Z",
            Field::Velocity => "Velocity magnitude",
            Field::VonMises => "Von Mises stress",
            Field::MaxPrincipal => "Max. principal stress",
            Field::MidPrincipal => "Mid. principal stress",
            Field::MinPrincipal => "Min. principal stress",
            Field::StressXx => "Stress XX",
            Field::StressYy => "Stress YY",
            Field::StressZz => "Stress ZZ",
            Field::StressXy => "Stress XY",
            Field::StressXz => "Stress XZ",
            Field::StressYz => "Stress YZ",
            Field::Temperature => "Temperature",
            Field::HeatFlux => "Heat flux magnitude",
        }
    }

    pub fn parse(id: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.id() == id)
    }

    /// Whether it is a stress.
    pub fn is_stress(self) -> bool {
        !matches!(
            self,
            Field::Displacement
                | Field::DisplacementX
                | Field::DisplacementY
                | Field::DisplacementZ
                | Field::Velocity
                | Field::Temperature
                | Field::HeatFlux
        )
    }

    /// Whether it is a heat transfer quantity.
    pub fn is_thermal(self) -> bool {
        matches!(self, Field::Temperature | Field::HeatFlux)
    }

    /// The fields a result has: displacements everywhere, stresses for static steps and explicit
    /// frames that kept them (mode shapes have no stress scale), velocity for explicit frames.
    pub fn available(r: &StepResult) -> impl Iterator<Item = Field> + '_ {
        let (stress, velocity, temperature, flux) = match r.kind() {
            Kind::Static(s) => (true, false, s.temperature.is_some(), false),
            Kind::Frequency(_) => (false, false, false, false),
            Kind::Explicit(e) => {
                e.frames.last().map_or((false, false, false, false), |f| (!f.stress.is_empty(), !f.velocity.is_empty(), false, false))
            }
            // Cell fields have their own display (M12 streamlines); no nodal field.
            Kind::Fluid(_) => return Vec::new().into_iter(),
            Kind::Heat(_) => return Field::ALL.into_iter().filter(|f| f.is_thermal()).collect::<Vec<_>>().into_iter(),
        };
        Field::ALL
            .into_iter()
            .filter(move |f| match f {
                Field::Velocity => velocity,
                Field::Temperature => temperature,
                Field::HeatFlux => flux,
                f => stress || !f.is_stress(),
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// Most contour bands.
pub const MAX_BANDS: u32 = 24;
/// Fewest contour bands.
pub const MIN_BANDS: u32 = 2;
/// Contour bands when none are asked for.
pub const DEFAULT_BANDS: u32 = 12;

/// What the results display shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResultsView {
    /// The solved step shown.
    pub step: String,
    /// Draw the deformed shape.
    pub deformed: bool,
    /// Deformation scale factor; `None` scales the largest displacement to a tenth of the
    /// model's size.
    pub scale: Option<f64>,
    /// Contours of this field; `None` draws the deformed shape in element colours.
    pub contour: Option<Field>,
    pub bands: u32,
    /// The mode shown, from 1 (frequency steps).
    pub mode: usize,
    /// The field frame shown (explicit steps); `None` is the last.
    #[serde(default)]
    pub frame: Option<usize>,
    /// Animate the deformation (a mode swings through ±, a static step grows from 0).
    pub animate: bool,
}

impl ResultsView {
    /// How a step's result is shown first: deformed, with von Mises contours for a static step
    /// (and an explicit step that kept stresses), the first mode with displacement contours for
    /// a frequency step; an explicit step at its last frame, to true scale.
    pub fn for_result(r: &StepResult) -> ResultsView {
        let heat = matches!(r.kind(), Kind::Heat(_));
        let contour = if heat {
            Field::Temperature
        } else if Field::available(r).any(|f| f == Field::VonMises) {
            Field::VonMises
        } else {
            Field::Displacement
        };
        let scale = matches!(r.kind(), Kind::Explicit(_)).then_some(1.0);
        ResultsView {
            step: r.step().to_string(),
            deformed: !heat,
            scale,
            contour: Some(contour),
            bands: DEFAULT_BANDS,
            mode: 1,
            frame: None,
            animate: false,
        }
    }

    pub fn pick(&self) -> Pick {
        Pick { mode: self.mode, frame: self.frame }
    }
}

/// The nodal displacements a result's deformed shape uses: the static displacement, the shape
/// of the picked mode or the picked frame's displacement, in [`StepResult::nodes`] order.
pub fn displacements(r: &StepResult, pick: Pick) -> Result<&[[f64; 3]]> {
    match r.kind() {
        Kind::Static(s) => Ok(&s.displacement),
        Kind::Frequency(f) => pick
            .mode
            .checked_sub(1)
            .and_then(|k| f.modes.get(k))
            .map(|m| m.shape.as_slice())
            .ok_or_else(|| EngineError::Params(format!("step `{}` has modes 1 to {}; there is no mode {}", f.step, f.modes.len(), pick.mode))),
        Kind::Explicit(e) => {
            let f = frame(e, pick)?;
            if f.displacement.is_empty() {
                return Err(EngineError::Params(format!("step `{}` kept no displacements in its field frames", e.step)));
            }
            Ok(&f.displacement)
        }
        Kind::Fluid(f) => Err(EngineError::Params(format!("step `{}` is a fluid step: its fields are per cell (solve.probe)", f.step))),
        Kind::Heat(h) => Err(EngineError::Params(format!("step `{}` is heat transfer and has no displacements", h.step))),
    }
}

/// A field's value at every node of the result, in [`StepResult::nodes`] order. `model` gives
/// the element connectivity an explicit frame's element stresses are averaged over.
pub fn field_values(model: &Model, r: &StepResult, field: Field, pick: Pick) -> Result<Vec<f64>> {
    let unavailable = || EngineError::Params(format!("{} is not available for step `{}`", field.label(), r.step()));
    let component = |f: fn(&[f64; 3]) -> f64| -> Result<Vec<f64>> { Ok(displacements(r, pick)?.iter().map(f).collect()) };
    let stresses = || -> Result<std::borrow::Cow<'_, [[f64; 6]]>> {
        match r.kind() {
            Kind::Static(s) => Ok(std::borrow::Cow::Borrowed(s.stress.as_slice())),
            Kind::Frequency(_) | Kind::Fluid(_) | Kind::Heat(_) => Err(unavailable()),
            Kind::Explicit(e) => {
                let f = frame(e, pick)?;
                if f.stress.is_empty() {
                    return Err(unavailable());
                }
                Ok(std::borrow::Cow::Owned(nodal_average(model, &e.nodes, &e.elements, &f.stress)))
            }
        }
    };
    let each = |f: &dyn Fn(&[f64; 6]) -> f64| -> Result<Vec<f64>> { Ok(stresses()?.iter().map(f).collect()) };
    let principal = |k: usize| each(&|t| modelcraft_fem::principal(t).get(k).copied().unwrap_or(f64::NAN));
    match field {
        Field::Displacement => component(|v| magnitude(*v)),
        Field::DisplacementX => component(|v| v[0]),
        Field::DisplacementY => component(|v| v[1]),
        Field::DisplacementZ => component(|v| v[2]),
        Field::Velocity => match r.kind() {
            Kind::Explicit(e) => {
                let f = frame(e, pick)?;
                if f.velocity.is_empty() { Err(unavailable()) } else { Ok(f.velocity.iter().map(|v| magnitude(*v)).collect()) }
            }
            Kind::Static(_) | Kind::Frequency(_) | Kind::Fluid(_) | Kind::Heat(_) => Err(unavailable()),
        },
        Field::Temperature => match r.kind() {
            Kind::Heat(h) => Ok(heat_frame(h, pick)?.1.to_vec()),
            Kind::Static(StaticResult { temperature: Some(t), .. }) => Ok(t.clone()),
            _ => Err(unavailable()),
        },
        Field::HeatFlux => match r.kind() {
            Kind::Heat(h) => Ok(h.flux.iter().map(|v| magnitude(*v)).collect()),
            _ => Err(unavailable()),
        },
        Field::VonMises => each(&modelcraft_fem::von_mises),
        Field::MaxPrincipal => principal(0),
        Field::MidPrincipal => principal(1),
        Field::MinPrincipal => principal(2),
        Field::StressXx => each(&|t| t[0]),
        Field::StressYy => each(&|t| t[1]),
        Field::StressZz => each(&|t| t[2]),
        Field::StressXy => each(&|t| t[3]),
        Field::StressXz => each(&|t| t[4]),
        Field::StressYz => each(&|t| t[5]),
    }
}

/// Element values (in `elements` order) averaged at the nodes (in `nodes` order) of the
/// elements around them; a node no element reaches gets NaN.
fn nodal_average(model: &Model, nodes: &[u32], elements: &[u32], values: &[[f64; 6]]) -> Vec<[f64; 6]> {
    let mut sum = vec![[0.0; 6]; nodes.len()];
    let mut count = vec![0u32; nodes.len()];
    for (label, v) in elements.iter().zip(values) {
        let Some(e) = model.element(*label) else { continue };
        for n in &e.nodes {
            let Ok(i) = nodes.binary_search(n) else { continue };
            if let (Some(s), Some(c)) = (sum.get_mut(i), count.get_mut(i)) {
                for (a, b) in s.iter_mut().zip(v) {
                    *a += b;
                }
                *c += 1;
            }
        }
    }
    sum.into_iter().zip(count).map(|(s, c)| if c == 0 { [f64::NAN; 6] } else { s.map(|x| x / f64::from(c)) }).collect()
}

/// The smallest and largest finite value, with the nodes they are at (`labels` in the same
/// order as `values`).
pub fn range(labels: &[u32], values: &[f64]) -> Option<((u32, f64), (u32, f64))> {
    let mut out: Option<((u32, f64), (u32, f64))> = None;
    for (&l, &v) in labels.iter().zip(values).filter(|(_, v)| v.is_finite()) {
        out = Some(match out {
            None => ((l, v), (l, v)),
            Some((lo, hi)) => (if v < lo.1 { (l, v) } else { lo }, if v > hi.1 { (l, v) } else { hi }),
        });
    }
    out
}

pub fn magnitude(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

impl Session {
    /// The latest result of a step and whether it describes the current state (`false`: the
    /// model changed since it was solved; it is stale).
    pub fn latest_result(&self, step: &str) -> Option<(&std::sync::Arc<StepResult>, bool)> {
        self.results.get(step).map(|(revision, r)| (r, *revision == self.revision))
    }

    /// What the results display shows: the view, whether its result is stale, the contour
    /// field's range, a frequency step's modes and an explicit step's frame times (the
    /// `results.*` commands' reply).
    pub fn results_report(&self) -> Value {
        let Some(v) = &self.results_view else { return json!({"view": null}) };
        let Some((r, current)) = self.latest_result(&v.step) else { return json!({"view": v}) };
        let mut out = json!({"view": v, "stale": !current});
        if let Some(f) = v.contour
            && let Ok(values) = field_values(&self.model, r, f, v.pick())
        {
            let ends =
                range(r.nodes(), &values).map(|((ln, lv), (hn, hv))| json!({"min": {"node": ln, "value": lv}, "max": {"node": hn, "value": hv}}));
            out["contour"] = json!({"field": f, "label": f.label(), "range": ends});
        }
        match r.kind() {
            Kind::Frequency(fr) => {
                out["modes"] = fr.modes.iter().enumerate().map(|(i, m)| json!({"mode": i + 1, "frequency": m.frequency})).collect();
            }
            Kind::Explicit(e) => {
                out["frames"] = e.frames.iter().map(|f| f.time).collect();
                if let Ok(f) = frame(e, v.pick()) {
                    out["time"] = json!(f.time);
                }
            }
            Kind::Fluid(f) => {
                out["frames"] = f.frames.iter().map(|f| f.time).collect();
            }
            Kind::Heat(h) => {
                out["frames"] = json!(h.times);
                if let Ok((t, _)) = heat_frame(h, v.pick()) {
                    out["time"] = json!(t);
                }
            }
            Kind::Static(_) => {}
        }
        out
    }

    /// Every step with a result, current or stale, by name.
    pub fn result_steps(&self) -> impl Iterator<Item = (&str, bool)> {
        self.results.iter().map(|(name, (revision, _))| (name.as_str(), *revision == self.revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::solve::tests::cantilever;
    use serde_json::json;

    #[test]
    fn fields_follow_the_result() {
        let mut s = Session::default();
        cantilever(&mut s);
        s.execute("solve.static", &json!({})).unwrap();
        s.execute("solve.frequency", &json!({})).unwrap();
        let (bend, current) = s.latest_result("Bend").unwrap();
        assert!(current);
        let mag = field_values(&s.model, bend, Field::Displacement, Pick { mode: 1, frame: None }).unwrap();
        let uz = field_values(&s.model, bend, Field::DisplacementZ, Pick { mode: 1, frame: None }).unwrap();
        assert_eq!(mag.len(), bend.nodes().len());
        for (m, z) in mag.iter().zip(&uz) {
            assert!(*m >= z.abs() - 1e-12);
        }
        // The tip bends down the most; the clamped root does not move.
        let ((lo_node, lo), (_, hi)) = range(bend.nodes(), &uz).unwrap();
        assert!(lo < -3.8 && hi.abs() < 1e-12, "{lo} {hi}");
        assert!([11, 22, 33, 44].contains(&lo_node), "{lo_node}");
        let vm = field_values(&s.model, bend, Field::VonMises, Pick { mode: 1, frame: None }).unwrap();
        let max = field_values(&s.model, bend, Field::MaxPrincipal, Pick { mode: 1, frame: None }).unwrap();
        let min = field_values(&s.model, bend, Field::MinPrincipal, Pick { mode: 1, frame: None }).unwrap();
        for ((v, a), b) in vm.iter().zip(&max).zip(&min) {
            assert!(a >= b && *v >= 0.0, "{v} {a} {b}");
        }
        let (modes, _) = s.latest_result("Modes").unwrap();
        assert_eq!(Field::available(modes).count(), 4);
        assert_eq!(Field::available(bend).count(), Field::ALL.len() - 3, "all but velocity and the thermal fields");
        assert!(field_values(&s.model, modes, Field::VonMises, Pick { mode: 1, frame: None }).is_err());
        assert!(field_values(&s.model, modes, Field::Displacement, Pick { mode: 9, frame: None }).is_err());
        assert_eq!(field_values(&s.model, modes, Field::Displacement, Pick { mode: 2, frame: None }).unwrap().len(), modes.nodes().len());
        // An edit leaves the results stale, not gone.
        s.execute("model.material.edit", &json!({"name": "Mat", "elastic": {"type": "isotropic", "young": 2000, "poisson": 0}, "density": 1}))
            .unwrap();
        assert!(s.result("Bend").is_none());
        assert_eq!(s.latest_result("Bend").map(|(_, current)| current), Some(false));
        assert_eq!(s.result_steps().collect::<Vec<_>>(), vec![("Bend", false), ("Modes", false)]);
    }

    #[test]
    fn field_ids_round_trip() {
        for f in Field::ALL {
            assert_eq!(Field::parse(f.id()), Some(f));
            assert_eq!(serde_json::to_value(f).unwrap(), json!(f.id()));
        }
        assert_eq!(Field::parse("nope"), None);
        assert_eq!(range(&[1, 2], &[f64::NAN, f64::INFINITY]), None);
        assert_eq!(range(&[1, 2, 3], &[2.0, -1.0, 5.0]), Some(((2, -1.0), (3, 5.0))));
    }
}
