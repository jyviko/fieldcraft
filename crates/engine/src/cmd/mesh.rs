use modelcraft_mesh::quality::{self, QualityReport};
use modelcraft_model::{ElementKind, MAX_ELEMENTS, MAX_LABEL};
use serde_json::{Value, json};

use super::{label, xyz};
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
    let raw = p.get("nodes").and_then(Value::as_array).ok_or_else(|| EngineError::Params("`nodes` must be a list of node labels".into()))?;
    if raw.len() != kind.node_count() {
        return Err(EngineError::Params(format!("{name} needs {} nodes, got {}", kind.node_count(), raw.len())));
    }
    let nodes = raw
        .iter()
        .map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| (1..=MAX_LABEL).contains(n)))
        .collect::<Option<Vec<u32>>>()
        .ok_or_else(|| EngineError::Params(format!("`nodes` must be node labels in 1..={MAX_LABEL}")))?;
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

fn labels(v: &[u32]) -> Value {
    json!({"count": v.len(), "labels": v.iter().take(LISTED).collect::<Vec<_>>()})
}

fn quality_report(s: &mut Session, p: &Value) -> Result<Value> {
    let bins = match p.get("bins") {
        None => 10,
        Some(v) => v.as_u64().and_then(|b| usize::try_from(b).ok()).ok_or_else(|| EngineError::Params("`bins` must be a whole number".into()))?,
    };
    let selection = match p.get("elements") {
        None => None,
        Some(v) => {
            let raw = v
                .as_array()
                .filter(|a| a.len() <= MAX_ELEMENTS)
                .ok_or_else(|| EngineError::Params("`elements` must be a list of element labels".into()))?;
            let labels = raw
                .iter()
                .map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| (1..=MAX_LABEL).contains(n)))
                .collect::<Option<Vec<u32>>>()
                .ok_or_else(|| EngineError::Params(format!("`elements` must be element labels in 1..={MAX_LABEL}")))?;
            Some(labels)
        }
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
        "invalid": labels(&r.invalid),
        "undetermined": labels(&r.undetermined),
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
