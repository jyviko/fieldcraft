//! Mesh-wide quality: statistics per metric, and the validity gate meshers run on their output.

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_model::{Element, ElementKind, MAX_COORD, Model};

use super::{ElementQuality, Metric, Validity, element_quality, element_validity, is_measured};
use crate::{Error, Result};

/// The most histogram bins a report may ask for.
pub const MAX_BINS: usize = 1000;

/// Statistics of one metric over the measured elements.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricStats {
    pub metric: Metric,
    /// Elements that have this metric.
    pub count: usize,
    pub min: f64,
    pub max: f64,
    /// Mean of the finite values (`None` when there are none).
    pub mean: Option<f64>,
    /// The element with the worst value (the first one on ties).
    pub worst: u32,
    /// Counts over [`Metric::range`] in equal bins; values outside the range fall in the end bins.
    pub histogram: Vec<usize>,
}

/// Quality of a set of elements.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityReport {
    /// Elements measured (solids and shells).
    pub measured: usize,
    /// Elements of kinds with no quality metrics (rods, beams), by kind.
    pub skipped: Vec<(ElementKind, usize)>,
    /// Solids proven valid.
    pub valid: usize,
    pub invalid: Vec<u32>,
    pub undetermined: Vec<u32>,
    /// Total signed volume of the solids and total area of the shells.
    pub volume: f64,
    pub area: f64,
    /// One entry per metric present, in [`Metric::ALL`] order.
    pub metrics: Vec<MetricStats>,
}

fn coordinates(model: &Model, label: u32, element: &Element) -> Result<Vec<[f64; 3]>> {
    element
        .nodes
        .iter()
        .map(|n| {
            model.node(*n).map(|node| node.xyz).ok_or_else(|| Error::Model(modelcraft_model::Error::NotFound(format!("element {label}: node {n}"))))
        })
        .collect()
}

/// The elements to look at: all of them, or the given labels (sorted, without repeats).
fn selection<'a>(model: &'a Model, labels: Option<&[u32]>) -> Result<Vec<(u32, &'a Element)>> {
    match labels {
        None => Ok(model.elements().iter().map(|(l, e)| (*l, e)).collect()),
        Some(labels) => labels
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|l| model.element(*l).map(|e| (*l, e)).ok_or_else(|| Error::Model(modelcraft_model::Error::NotFound(format!("element {l}")))))
            .collect(),
    }
}

/// Quality statistics of the model's elements, or of the given ones, with `bins` histogram bins
/// (1..=[`MAX_BINS`]) per metric. Deterministic: elements are visited in label order.
pub fn quality_report(model: &Model, labels: Option<&[u32]>, bins: usize) -> Result<QualityReport> {
    if !(1..=MAX_BINS).contains(&bins) {
        return Err(Error::Invalid(format!("bins must be in 1..={MAX_BINS}, got {bins}")));
    }
    let mut report = QualityReport {
        measured: 0,
        skipped: Vec::new(),
        valid: 0,
        invalid: Vec::new(),
        undetermined: Vec::new(),
        volume: 0.0,
        area: 0.0,
        metrics: Vec::new(),
    };
    let mut skipped: BTreeMap<usize, (ElementKind, usize)> = BTreeMap::new();
    let mut stats: BTreeMap<Metric, (MetricStats, f64, usize)> = BTreeMap::new();
    for (label, element) in selection(model, labels)? {
        if !is_measured(element.kind) {
            let order = ElementKind::ALL.iter().position(|k| *k == element.kind).unwrap_or(0);
            skipped.entry(order).or_insert((element.kind, 0)).1 += 1;
            continue;
        }
        let q: ElementQuality = element_quality(element.kind, &coordinates(model, label, element)?)?;
        report.measured += 1;
        match q.validity() {
            Some(Validity::Valid) => report.valid += 1,
            Some(Validity::Invalid) => report.invalid.push(label),
            Some(Validity::Undetermined) => report.undetermined.push(label),
            None => {}
        }
        if element.kind.dimension() == 3 {
            report.volume += q.size;
        } else {
            report.area += q.size;
        }
        for (metric, v) in q.metrics {
            let (s, sum, finite) = stats.entry(metric).or_insert_with(|| {
                (
                    MetricStats { metric, count: 0, min: f64::INFINITY, max: f64::NEG_INFINITY, mean: None, worst: label, histogram: vec![0; bins] },
                    0.0,
                    0,
                )
            });
            if v.is_nan() {
                continue;
            }
            let worse = if metric.higher_is_better() { v < s.min } else { v > s.max };
            if worse || s.count == 0 {
                s.worst = label;
            }
            s.count += 1;
            s.min = s.min.min(v);
            s.max = s.max.max(v);
            if v.is_finite() {
                *sum += v;
                *finite += 1;
            }
            let (lo, hi) = metric.range();
            // `as` saturates: −∞ lands in bin 0, +∞ in the last bin.
            let bin = (((v - lo) / (hi - lo)) * bins as f64).floor().max(0.0) as usize;
            if let Some(b) = s.histogram.get_mut(bin.min(bins - 1)) {
                *b += 1;
            }
        }
    }
    report.skipped = skipped.into_values().collect();
    report.metrics = stats
        .into_values()
        .map(|(mut s, sum, finite)| {
            s.mean = (finite > 0).then(|| sum / finite as f64);
            s
        })
        .collect();
    Ok(report)
}

/// What the validity gate checks besides element validity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateOptions {
    /// The volume the mesh must fill: the boundary's enclosed volume ([`enclosed_volume`]).
    pub expected_volume: Option<f64>,
    /// Relative tolerance on the volume. Curved (quadratic) meshes of a faceted boundary need a
    /// looser one than linear meshes.
    pub volume_tolerance: f64,
}

impl Default for GateOptions {
    fn default() -> Self {
        Self { expected_volume: None, volume_tolerance: 1e-9 }
    }
}

/// The result of the validity gate.
#[derive(Debug, Clone, PartialEq)]
pub struct GateReport {
    /// Solid elements checked (other kinds are ignored).
    pub checked: usize,
    pub invalid: Vec<u32>,
    pub undetermined: Vec<u32>,
    /// Total signed volume of the solids.
    pub volume: f64,
    pub options: GateOptions,
}

impl GateReport {
    /// Whether the volume matches the expected one (true when none is expected).
    pub fn volume_matches(&self) -> bool {
        self.options.expected_volume.is_none_or(|v| (self.volume - v).abs() <= self.options.volume_tolerance * v.abs())
    }

    /// Every solid is proven valid and the volume matches.
    pub fn passed(&self) -> bool {
        self.invalid.is_empty() && self.undetermined.is_empty() && self.volume_matches()
    }

    /// The report, or [`Error::Failed`] describing why the gate failed.
    pub fn into_result(self) -> Result<Self> {
        if self.passed() {
            return Ok(self);
        }
        let mut why = Vec::new();
        let list = |v: &[u32]| v.iter().take(10).map(u32::to_string).collect::<Vec<_>>().join(", ");
        if !self.invalid.is_empty() {
            why.push(format!("{} inverted or degenerate elements ({})", self.invalid.len(), list(&self.invalid)));
        }
        if !self.undetermined.is_empty() {
            why.push(format!("{} elements whose validity could not be proven ({})", self.undetermined.len(), list(&self.undetermined)));
        }
        if let (false, Some(v)) = (self.volume_matches(), self.options.expected_volume) {
            why.push(format!("mesh volume {} differs from the boundary's {v}", self.volume));
        }
        Err(Error::Failed(format!("mesh validity gate: {}", why.join("; "))))
    }
}

/// The validity gate every mesher runs on its output before it reaches the model: each solid
/// element's Jacobian determinant is proven positive everywhere, and the elements' total volume
/// equals the volume the boundary encloses. Call [`GateReport::into_result`] to turn a failure
/// into an error.
pub fn validity_gate(model: &Model, options: &GateOptions) -> Result<GateReport> {
    if !(options.volume_tolerance >= 0.0 && options.volume_tolerance.is_finite()) || options.expected_volume.is_some_and(|v| !v.is_finite()) {
        return Err(Error::Invalid("gate options must be finite, with a non-negative tolerance".into()));
    }
    let mut report = GateReport { checked: 0, invalid: Vec::new(), undetermined: Vec::new(), volume: 0.0, options: *options };
    for (label, element) in model.elements() {
        if element.kind.dimension() != 3 {
            continue;
        }
        let check = element_validity(element.kind, &coordinates(model, *label, element)?)?;
        report.checked += 1;
        report.volume += check.volume;
        match check.validity {
            Validity::Valid => {}
            Validity::Invalid => report.invalid.push(*label),
            Validity::Undetermined => report.undetermined.push(*label),
        }
    }
    Ok(report)
}

/// The signed volume enclosed by a closed triangle surface (positive when the triangles' right-
/// hand normals point outwards), by the divergence theorem.
pub fn enclosed_volume(points: &[[f64; 3]], triangles: &[[usize; 3]]) -> Result<f64> {
    if let Some(c) = points.iter().flatten().find(|c| !(c.abs() <= MAX_COORD)) {
        return Err(Error::Invalid(format!("coordinate {c} is not a finite number within ±{MAX_COORD}")));
    }
    let o = points.first().copied().unwrap_or([0.0; 3]);
    let mut six_v = 0.0;
    for t in triangles {
        let p = |i: usize| points.get(i).map(|p| super::shape::sub(*p, o)).ok_or_else(|| Error::Invalid(format!("triangle {t:?}: no point {i}")));
        six_v += super::shape::det(p(t[0])?, p(t[1])?, p(t[2])?);
    }
    Ok(six_v / 6.0)
}
