use modelcraft_mesh::delaunay::Triangulation;
use modelcraft_mesh::quality::{self, QualityReport};
use modelcraft_model::{ElementKind, MAX_ELEMENTS};
use serde_json::{Value, json};

use super::{label, labels, xyz};
use crate::{CommandSpec, EngineError, Result, Session};

pub const NODE_ADD: CommandSpec = CommandSpec {
    id: "mesh.node.add",
    label: "Add Node",
    params: r#"{"label": 1, "xyz": [x, y, z]} — adds the node, or moves it if the label exists"#,
    run: node_add,
};

pub const NODE_REMOVE: CommandSpec =
    CommandSpec { id: "mesh.node.remove", label: "Remove Node", params: r#"{"label": 1} — only a node no element uses"#, run: node_remove };

pub const ELEMENT_ADD: CommandSpec = CommandSpec {
    id: "mesh.element.add",
    label: "Add Element",
    params: r#"{"label": 1, "kind": "rod2|beam2|tri3|tri6|quad4|quad8|tet4|tet10|wedge6|hex8|hex20", "nodes": [labels]} — adds or replaces"#,
    run: element_add,
};

pub const ELEMENT_REMOVE: CommandSpec =
    CommandSpec { id: "mesh.element.remove", label: "Remove Element", params: r#"{"label": 1}"#, run: element_remove };

pub const QUALITY: CommandSpec = CommandSpec {
    id: "mesh.quality",
    label: "Mesh Quality",
    params: r#"{"elements": [labels] (optional, default all), "bins": 10 (1..=1000)} — certified validity (det J > 0 everywhere), volume, and min/max/mean/worst/histogram per metric (angles in degrees; histograms over each metric's fixed range, out-of-range values in the end bins)"#,
    run: quality_report,
};

pub const DELAUNAY_POINTS: CommandSpec = CommandSpec {
    id: "mesh.delaunay.points",
    label: "Delaunay of Points",
    params: r#"{"points": [[x, y, z], ...] (at most 20000)} — the exact Delaunay tetrahedralization (a debugging report; the model is not changed): {"dimension", "vertices", "duplicates": [[point, kept point], ...], "tets": [[i, j, k, l], ...]} with 0-based point indices in tet4 order; collinear or coplanar points give no tets"#,
    run: delaunay_points,
};

/// The most points `mesh.delaunay.points` takes (its report lists every tetrahedron).
const DELAUNAY_MAX_POINTS: usize = 20_000;

fn delaunay_points(_: &mut Session, p: &Value) -> Result<Value> {
    let bad = || EngineError::Params(format!("`points` must be a list of at most {DELAUNAY_MAX_POINTS} [x, y, z]"));
    let raw = p.get("points").and_then(Value::as_array).filter(|a| a.len() <= DELAUNAY_MAX_POINTS).ok_or_else(bad)?;
    let points = raw.iter().map(|v| xyz(&json!({ "p": v }), "p")).collect::<Result<Vec<[f64; 3]>>>().map_err(|_| bad())?;
    let tri = Triangulation::new(&points)?;
    tri.verify()?;
    let duplicates: Vec<[u32; 2]> = (0..points.len() as u32).filter_map(|v| tri.duplicate_of(v).map(|d| [v, d])).collect();
    let tets: Vec<[u32; 4]> = tri.finite_tets().map(|(_, vs)| vs).collect();
    Ok(json!({
        "dimension": tri.dimension(),
        "vertices": points.len() - duplicates.len(),
        "duplicates": duplicates,
        "tets": tets,
    }))
}

fn node_add(s: &mut Session, p: &Value) -> Result<Value> {
    let l = label(p, "label")?;
    s.model.set_node(l, xyz(p, "xyz")?)?;
    Ok(json!({"label": l}))
}

fn node_remove(s: &mut Session, p: &Value) -> Result<Value> {
    s.model.remove_node(label(p, "label")?)?;
    Ok(Value::Null)
}

fn element_add(s: &mut Session, p: &Value) -> Result<Value> {
    let l = label(p, "label")?;
    let name = p.get("kind").and_then(Value::as_str).unwrap_or_default();
    let kind = ElementKind::from_name(name).ok_or_else(|| EngineError::Params(format!("unknown element kind `{name}`")))?;
    let nodes = labels(p, "nodes", ElementKind::Hex20.node_count())?;
    if nodes.len() != kind.node_count() {
        return Err(EngineError::Params(format!("{name} needs {} nodes, got {}", kind.node_count(), nodes.len())));
    }
    s.model.set_element(l, kind, nodes)?;
    Ok(json!({"label": l}))
}

fn element_remove(s: &mut Session, p: &Value) -> Result<Value> {
    s.model.remove_element(label(p, "label")?)?;
    Ok(Value::Null)
}

/// Labels listed in a report: the first ones, and how many there are.
const LISTED: usize = 100;

/// A JSON number, or a string for ±∞ (JSON has no infinity).
fn number(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { json!(if v > 0.0 { "inf" } else { "-inf" }) }
}

fn listed(v: &[u32]) -> Value {
    json!({"count": v.len(), "labels": v.iter().take(LISTED).collect::<Vec<_>>()})
}

fn quality_report(s: &mut Session, p: &Value) -> Result<Value> {
    let bins = match p.get("bins") {
        None => 10,
        Some(v) => v.as_u64().and_then(|b| usize::try_from(b).ok()).ok_or_else(|| EngineError::Params("`bins` must be a whole number".into()))?,
    };
    let selection = match p.get("elements") {
        None => None,
        Some(_) => Some(labels(p, "elements", MAX_ELEMENTS)?),
    };
    let r: QualityReport = quality::quality_report(&s.model, selection.as_deref(), bins)?;
    let skipped: serde_json::Map<String, Value> = r.skipped.iter().map(|(k, n)| (k.name().to_string(), json!(n))).collect();
    let metrics: serde_json::Map<String, Value> = r
        .metrics
        .iter()
        .map(|m| {
            let (lo, hi) = m.metric.range();
            let stats = json!({
                "count": m.count,
                "min": number(m.min),
                "max": number(m.max),
                "mean": m.mean.map(number),
                "worst": m.worst,
                "histogram": {"range": [lo, hi], "counts": m.histogram},
            });
            (m.metric.name().to_string(), stats)
        })
        .collect();
    Ok(json!({
        "measured": r.measured,
        "skipped": skipped,
        "valid": r.valid,
        "invalid": listed(&r.invalid),
        "undetermined": listed(&r.undetermined),
        "volume": r.volume,
        "area": r.area,
        "metrics": metrics,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn hex(s: &mut Session, label: u32, first_node: u32, mirror: bool) {
        let corners = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];
        for (i, c) in corners.iter().enumerate() {
            let x = if mirror { -c[0] - 2 } else { c[0] };
            s.execute("mesh.node.add", &json!({"label": first_node + i as u32, "xyz": [x, c[1], c[2]]})).unwrap();
        }
        let nodes: Vec<u32> = (first_node..first_node + 8).collect();
        s.execute("mesh.element.add", &json!({"label": label, "kind": "hex8", "nodes": nodes})).unwrap();
    }

    #[test]
    fn delaunay_of_points_reports_tets_and_duplicates() {
        let mut s = Session::default();
        let cube = json!({"points": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1], [1, 1, 1]]});
        let r = s.execute("mesh.delaunay.points", &cube).unwrap();
        assert_eq!(r["dimension"], 3);
        assert_eq!(r["vertices"], 8);
        assert_eq!(r["duplicates"], json!([[8, 7]]));
        let tets = r["tets"].as_array().unwrap();
        assert!((5..=6).contains(&tets.len()), "{tets:?}");
        assert!(s.journal.is_empty(), "a report does not change the model");

        let flat = s.execute("mesh.delaunay.points", &json!({"points": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0]]})).unwrap();
        assert_eq!((flat["dimension"].clone(), flat["tets"].clone()), (json!(2), json!([])));
        for bad in [
            json!({}),
            json!({"points": [[0, 0]]}),
            json!({"points": [[0, 0, "x"]]}),
            json!({"points": [[f64::MAX, 0, 0]]}),
            json!({"points": [[1e-300, 0, 0]]}),
            json!({"points": vec![[0, 0, 0]; 20_001]}),
        ] {
            assert!(s.execute("mesh.delaunay.points", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reports_quality_and_inverted_elements() {
        let mut s = Session::default();
        hex(&mut s, 1, 1, false);
        hex(&mut s, 2, 11, true);
        let journal = s.journal.len();
        let r = s.execute("mesh.quality", &json!({"bins": 2})).unwrap();
        assert_eq!(r["measured"], 2);
        assert_eq!(r["valid"], 1);
        assert_eq!(r["invalid"], json!({"count": 1, "labels": [2]}));
        assert_eq!(r["volume"], 0.0);
        let sj = &r["metrics"]["scaled_jacobian"];
        assert_eq!((sj["min"].clone(), sj["max"].clone(), sj["worst"].clone()), (json!(-1.0), json!(1.0), json!(2)));
        assert_eq!(sj["histogram"]["counts"], json!([1, 1]));
        assert_eq!(s.journal.len(), journal, "a report does not change the model");

        let one = s.execute("mesh.quality", &json!({"elements": [1, 1]})).unwrap();
        assert_eq!((one["measured"].clone(), one["volume"].clone()), (json!(1), json!(1.0)));
        assert!(s.execute("mesh.quality", &json!({"elements": [3]})).is_err());
        assert!(s.execute("mesh.quality", &json!({"elements": [0]})).is_err());
        assert!(s.execute("mesh.quality", &json!({"bins": 0})).is_err());
        assert!(s.execute("mesh.quality", &json!({"bins": 1e9})).is_err());
        assert!(s.execute("mesh.quality", &json!({"elements": "all"})).is_err());
    }
}
