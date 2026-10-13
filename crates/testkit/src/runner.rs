//! The corpus runner behind `cargo xtask corpus [--part NAME] [--no-write]`: build every part of
//! the meshing test corpus ([`crate::corpus`]), tessellate it, run each [`Stage`] on it and write
//! the results table to docs/meshing/corpus.md.
//!
//! The table is a fixed part columns block (bodies, faces, the boundary surface and the time to
//! build it) followed by one column group per stage. A mesher joins the corpus by implementing
//! [`Stage`] over a [`Loaded`] part (its tessellated boundary surfaces are ready in
//! `Loaded::surfaces`) and adding itself to [`stages`]. A stage that fails on a part shows the
//! error in its first column; the run goes on.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use crate::corpus::{self, CATEGORIES, Part};
use modelcraft_geometry::{Design, SurfaceMesh};

pub const FILE: &str = "docs/meshing/corpus.md";
/// Chordal tolerance of the boundary surface, relative to the part's bounding-box diagonal.
const RELATIVE_TOLERANCE: f64 = 1e-3;

/// A built part with its boundary surfaces.
pub struct Loaded {
    pub part: Part,
    pub design: Design,
    /// One per body, in the design's order.
    pub surfaces: Vec<SurfaceMesh>,
    /// The chordal tolerance the surfaces were made with (mm).
    pub tolerance: f64,
}

/// A column group of the results table: a mesher, a check.
pub trait Stage {
    fn headers(&self) -> &'static [&'static str];
    /// One cell per header.
    fn run(&self, part: &Loaded) -> Result<Vec<String>, String>;
}

/// The stages, in column order. Meshers add theirs here.
fn stages() -> Vec<Box<dyn Stage>> {
    vec![Box::new(SurfaceVolume)]
}

/// The boundary surface's enclosed volume against SolidCraft's measured volume and the closed
/// form: a surface that is open, inverted or misplaced shows here first.
struct SurfaceVolume;

impl Stage for SurfaceVolume {
    fn headers(&self) -> &'static [&'static str] {
        &["enclosed mm³", "SolidCraft mm³", "rel. diff", "exact mm³"]
    }

    fn run(&self, l: &Loaded) -> Result<Vec<String>, String> {
        let enclosed: f64 = l.surfaces.iter().map(SurfaceMesh::enclosed_volume).sum();
        let mut measured = 0.0;
        for b in &l.design.bodies {
            measured += b.measure().map_err(|e| format!("measure {}: {e}", b.name))?.volume;
        }
        let rel = if measured.abs() > 0.0 { (enclosed - measured) / measured } else { f64::NAN };
        Ok(vec![format!("{enclosed:.1}"), format!("{measured:.1}"), format!("{rel:.1e}"), l.part.volume.map_or("–".into(), |v| format!("{v:.1}"))])
    }
}

fn load(part: Part) -> Result<(Loaded, f64), String> {
    let start = Instant::now();
    let design = corpus::load(&part).map_err(|e| e.to_string())?;
    if let Some(e) = design.errors.first() {
        return Err(format!("feature {} failed: {}", e.feature, e.message));
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for b in &design.bodies {
        let m = b.measure().map_err(|e| format!("measure {}: {e}", b.name))?;
        for i in 0..3 {
            lo[i] = lo[i].min(m.bbox_min[i]);
            hi[i] = hi[i].max(m.bbox_max[i]);
        }
    }
    let diagonal = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt();
    let tolerance = RELATIVE_TOLERANCE * if diagonal.is_finite() && diagonal > 0.0 { diagonal } else { 1.0 };
    let mut surfaces = Vec::with_capacity(design.bodies.len());
    for b in &design.bodies {
        surfaces.push(b.tessellate(tolerance).map_err(|e| format!("tessellate {}: {e}", b.name))?);
    }
    let ms = start.elapsed().as_secs_f64() * 1e3;
    Ok((Loaded { part, design, surfaces, tolerance }, ms))
}

/// Run the corpus; with `write` (and every part) write [`FILE`] under the workspace `root`.
pub fn run(root: &Path, only: Option<&str>, write: bool) -> Result<(), String> {
    let mut parts = corpus::parts().map_err(|e| e.to_string())?;
    if let Some(name) = only {
        parts.retain(|p| p.name == name);
        if parts.is_empty() {
            return Err(format!("no corpus part named `{name}`"));
        }
    }
    let stages = stages();
    let mut header = vec!["part", "categories", "bodies", "faces", "triangles", "tol. mm", "build ms"];
    for s in &stages {
        header.extend_from_slice(s.headers());
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut failures = 0;
    let mut covered: Vec<(&str, usize)> = CATEGORIES.iter().map(|(c, _)| (*c, 0)).collect();
    let mut descriptions = Vec::new();
    for part in parts {
        let name = part.name.clone();
        eprintln!("corpus: {name}");
        for (c, n) in &mut covered {
            if part.categories.iter().any(|x| x == c) {
                *n += 1;
            }
        }
        descriptions.push((name.clone(), part.description.clone()));
        let categories = part.categories.join(", ");
        let mut row = vec![name.clone(), categories];
        match load(part) {
            Err(e) => {
                failures += 1;
                row.push(format!("error: {e}"));
                row.resize(header.len(), String::new());
            }
            Ok((l, ms)) => {
                let faces: usize = l.design.bodies.iter().map(|b| b.measure().map(|m| m.faces).unwrap_or(0)).sum();
                let triangles: usize = l.surfaces.iter().map(|s| s.triangles.len()).sum();
                row.extend([
                    l.design.bodies.len().to_string(),
                    faces.to_string(),
                    triangles.to_string(),
                    format!("{:.3}", l.tolerance),
                    format!("{ms:.0}"),
                ]);
                for s in &stages {
                    match s.run(&l) {
                        Ok(cells) => row.extend(cells),
                        Err(e) => {
                            failures += 1;
                            row.push(format!("error: {e}"));
                            row.extend(std::iter::repeat_n(String::new(), s.headers().len().saturating_sub(1)));
                        }
                    }
                }
            }
        }
        rows.push(row);
    }
    let table = markdown_table(&header, &rows);
    println!("{table}");
    if write && only.is_none() {
        let path = root.join(FILE);
        std::fs::write(&path, document(&table, &covered, &descriptions)).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("corpus: wrote {FILE}");
    }
    if failures > 0 { Err(format!("corpus: {failures} part(s) or stage(s) failed")) } else { Ok(()) }
}

fn markdown_table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut s = format!("| {} |\n|{}\n", header.join(" | "), "---|".repeat(header.len()));
    for r in rows {
        let _ = writeln!(s, "| {} |", r.iter().map(|c| c.replace('|', "\\|")).collect::<Vec<_>>().join(" | "));
    }
    s
}

fn document(table: &str, covered: &[(&str, usize)], descriptions: &[(String, String)]) -> String {
    let mut s = String::from(
        "# Meshing test corpus\n\n\
         Generated by `cargo xtask corpus`; do not edit. The parts are our own: SolidCraft recipes in\n\
         `tests/corpus/<name>/recipe.json` and analytic shapes generated in code\n\
         (`crates/testkit/src/corpus.rs`). Each run rebuilds every part, tessellates it at a chordal\n\
         tolerance of 1e-3 of its bounding-box diagonal and runs every stage on it. Times are wall\n\
         clock on the machine that ran it; every other column is deterministic.\n\n\
         ## Results\n\n",
    );
    s.push_str(table);
    s.push_str("\n## Categories\n\n| category | stresses | parts |\n|---|---|---|\n");
    for (c, n) in covered {
        let what = CATEGORIES.iter().find(|(k, _)| k == c).map_or("", |(_, w)| *w);
        let _ = writeln!(s, "| {c} | {what} | {n} |");
    }
    s.push_str("\n## Parts\n\n");
    for (name, d) in descriptions {
        let _ = writeln!(s, "- `{name}`: {d}");
    }
    s
}
