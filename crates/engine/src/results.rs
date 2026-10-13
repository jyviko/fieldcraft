//! What the results display shows ([`ResultsView`], set by the `results.*` commands) and the
//! nodal values it draws: displacements, mode shapes and stress fields, read from a solved
//! step's [`StepResult`].
//!
//! The view is session state, like the camera: changing it is not an edit, so it is neither
//! journalled nor undone, and it does not change the session revision. A result kept from an
//! earlier revision is still shown, marked stale ([`Session::latest_result`]).

use modelcraft_solver::StepResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// A nodal quantity drawn as contours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Displacement magnitude (a mode's shape magnitude for a frequency step).
    Displacement,
    DisplacementX,
    DisplacementY,
    DisplacementZ,
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
}

impl Field {
    pub const ALL: [Field; 14] = [
        Field::Displacement,
        Field::DisplacementX,
        Field::DisplacementY,
        Field::DisplacementZ,
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
    ];

    /// The id commands take (`von_mises`…).
    pub fn id(self) -> &'static str {
        match self {
            Field::Displacement => "displacement",
            Field::DisplacementX => "displacement_x",
            Field::DisplacementY => "displacement_y",
            Field::DisplacementZ => "displacement_z",
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
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Field::Displacement => "Displacement magnitude",
            Field::DisplacementX => "Displacement X",
            Field::DisplacementY => "Displacement Y",
            Field::DisplacementZ => "Displacement Z",
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
        }
    }

    pub fn parse(id: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.id() == id)
    }

    /// Whether it is a stress (static steps only; a mode shape has no stress scale).
    pub fn is_stress(self) -> bool {
        !matches!(self, Field::Displacement | Field::DisplacementX | Field::DisplacementY | Field::DisplacementZ)
    }

    /// The fields a result has.
    pub fn available(r: &StepResult) -> impl Iterator<Item = Field> + '_ {
        Field::ALL.into_iter().filter(move |f| matches!(r, StepResult::Static(_)) || !f.is_stress())
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
    /// Animate the deformation (a mode swings through ±, a static step grows from 0).
    pub animate: bool,
}

impl ResultsView {
    pub fn new(step: &str) -> ResultsView {
        ResultsView { step: step.to_string(), deformed: true, scale: None, contour: None, bands: DEFAULT_BANDS, mode: 1, animate: false }
    }
}

/// The nodal displacements a result's deformed shape uses: the static displacement, or the
/// shape of `mode` (from 1), in [`StepResult::nodes`] order.
pub fn displacements(r: &StepResult, mode: usize) -> Result<&[[f64; 3]]> {
    match r {
        StepResult::Static(s) => Ok(&s.displacement),
        StepResult::Frequency(f) => mode
            .checked_sub(1)
            .and_then(|k| f.modes.get(k))
            .map(|m| m.shape.as_slice())
            .ok_or_else(|| EngineError::Params(format!("step `{}` has modes 1 to {}; there is no mode {mode}", f.step, f.modes.len()))),
    }
}

/// A field's value at every node of the result, in [`StepResult::nodes`] order.
pub fn field_values(r: &StepResult, field: Field, mode: usize) -> Result<Vec<f64>> {
    let u = |c: usize| -> Result<Vec<f64>> { Ok(displacements(r, mode)?.iter().map(|v| v.get(c).copied().unwrap_or(f64::NAN)).collect()) };
    let StepResult::Static(s) = r else {
        return match field {
            Field::Displacement => Ok(displacements(r, mode)?.iter().map(|v| magnitude(*v)).collect()),
            Field::DisplacementX => u(0),
            Field::DisplacementY => u(1),
            Field::DisplacementZ => u(2),
            _ => Err(EngineError::Params(format!("{} is not available for a frequency step (mode shapes have no stress scale)", field.label()))),
        };
    };
    let stress = |c: usize| s.stress.iter().map(|t| t.get(c).copied().unwrap_or(f64::NAN)).collect();
    let principal = |k: usize| s.stress.iter().map(|t| modelcraft_fem::principal(t).get(k).copied().unwrap_or(f64::NAN)).collect();
    Ok(match field {
        Field::Displacement => s.displacement.iter().map(|v| magnitude(*v)).collect(),
        Field::DisplacementX => u(0)?,
        Field::DisplacementY => u(1)?,
        Field::DisplacementZ => u(2)?,
        Field::VonMises => s.von_mises.clone(),
        Field::MaxPrincipal => principal(0),
        Field::MidPrincipal => principal(1),
        Field::MinPrincipal => principal(2),
        Field::StressXx => stress(0),
        Field::StressYy => stress(1),
        Field::StressZz => stress(2),
        Field::StressXy => stress(3),
        Field::StressXz => stress(4),
        Field::StressYz => stress(5),
    })
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
    /// field's range and a frequency step's modes (the `results.*` commands' reply).
    pub fn results_report(&self) -> Value {
        let Some(v) = &self.results_view else { return json!({"view": null}) };
        let Some((r, current)) = self.latest_result(&v.step) else { return json!({"view": v}) };
        let mut out = json!({"view": v, "stale": !current});
        if let Some(f) = v.contour
            && let Ok(values) = field_values(r, f, v.mode)
        {
            let ends =
                range(r.nodes(), &values).map(|((ln, lv), (hn, hv))| json!({"min": {"node": ln, "value": lv}, "max": {"node": hn, "value": hv}}));
            out["contour"] = json!({"field": f, "label": f.label(), "range": ends});
        }
        if let StepResult::Frequency(fr) = r.as_ref() {
            out["modes"] = fr.modes.iter().enumerate().map(|(i, m)| json!({"mode": i + 1, "frequency": m.frequency})).collect();
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
        let mag = field_values(bend, Field::Displacement, 1).unwrap();
        let uz = field_values(bend, Field::DisplacementZ, 1).unwrap();
        assert_eq!(mag.len(), bend.nodes().len());
        for (m, z) in mag.iter().zip(&uz) {
            assert!(*m >= z.abs() - 1e-12);
        }
        // The tip bends down the most; the clamped root does not move.
        let ((lo_node, lo), (_, hi)) = range(bend.nodes(), &uz).unwrap();
        assert!(lo < -3.8 && hi.abs() < 1e-12, "{lo} {hi}");
        assert!([11, 22, 33, 44].contains(&lo_node), "{lo_node}");
        let vm = field_values(bend, Field::VonMises, 1).unwrap();
        let max = field_values(bend, Field::MaxPrincipal, 1).unwrap();
        let min = field_values(bend, Field::MinPrincipal, 1).unwrap();
        for ((v, a), b) in vm.iter().zip(&max).zip(&min) {
            assert!(a >= b && *v >= 0.0, "{v} {a} {b}");
        }
        let (modes, _) = s.latest_result("Modes").unwrap();
        assert_eq!(Field::available(modes).count(), 4);
        assert!(field_values(modes, Field::VonMises, 1).is_err());
        assert!(field_values(modes, Field::Displacement, 9).is_err());
        assert_eq!(field_values(modes, Field::Displacement, 2).unwrap().len(), modes.nodes().len());
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
