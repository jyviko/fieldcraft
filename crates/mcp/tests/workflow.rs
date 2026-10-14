//! A whole analysis driven only through MCP messages on a headless, confined server: open a
//! SolidCraft design built from a corpus recipe, mesh it, define sets, material, section,
//! boundary condition, load and step, solve, read the largest displacement and render a PNG.

use std::path::PathBuf;

use fieldcraft_io::roots::Roots;
use fieldcraft_mcp::{Headless, Server};
use serde_json::{Value, json};

struct Client {
    server: Server,
    next: u64,
}

impl Client {
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let line = json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}).to_string();
        let reply: Value = serde_json::from_str(&self.server.handle_line(&line).unwrap()).unwrap();
        assert_eq!(reply["id"], self.next);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    /// A tool call that must succeed: its content blocks.
    fn tool(&mut self, name: &str, arguments: Value) -> Vec<Value> {
        let r = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        let content = r["content"].as_array().unwrap().clone();
        assert!(r["isError"] != json!(true), "{name}: {}", content[0]["text"]);
        content
    }

    /// A tool call's JSON reply.
    fn json(&mut self, name: &str, arguments: Value) -> Value {
        let content = self.tool(name, arguments);
        serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap()
    }
}

fn base64_decode(s: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let bytes: Vec<u8> = s.bytes().filter(|c| *c != b'=').collect();
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, c)| n | (u32::from(value(*c)) << (18 - 6 * i)));
        out.extend(n.to_be_bytes().into_iter().skip(1).take(chunk.len() - 1));
    }
    out
}

#[test]
fn a_corpus_part_is_meshed_loaded_solved_and_rendered_over_mcp() {
    let dir: PathBuf = std::env::temp_dir().join(format!("fieldcraft-mcp-workflow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = std::fs::canonicalize(&dir).unwrap();
    let part = fieldcraft_testkit::corpus::read_recipe(&fieldcraft_testkit::corpus::corpus_dir().join("stepped-block")).unwrap();
    let design = dir.join("steps.solidcraft");
    std::fs::write(&design, fieldcraft_testkit::corpus::design_bytes(&part).unwrap()).unwrap();

    let roots = Roots::new(Some(&dir), Some(&dir)).unwrap();
    let mut c = Client { server: Server::new(Box::new(Headless::new(roots))), next: 0 };
    let init = c.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
    assert_eq!(init["serverInfo"]["name"], "fieldcraft");
    assert!(c.server.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());

    // Open the design and look at its faces.
    let opened = c.json("open", json!({"path": design}));
    assert_eq!(opened["command"], "geometry.open");
    let model = c.json("inspect_model", json!({}));
    let body = &model["bodies"][0];
    let faces: Vec<&str> = body["faces"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    // The union's x = 0 end (one face, from the first box) and the bottom step's x = 60 end.
    assert!(faces.contains(&"F1:-x") && faces.contains(&"F1:+x"), "{faces:?}");

    // Mesh: constrained Delaunay tets, every boundary face tagged with its B-rep face.
    let meshed = c.json("mesh", json!({"method": "tet", "params": {"body": "Steps", "tolerance": 0.5}}));
    let tets = meshed["model"]["elements"]["tet4"].as_u64().unwrap();
    assert!(tets > 10, "{meshed}");
    assert_eq!((meshed["quality"]["valid"].as_u64(), meshed["quality"]["invalid"]["count"].as_u64()), (Some(tets), Some(0)), "{}", meshed["quality"]);

    // Steel in mm-t-s-N-MPa, clamped at x = 0, 1 kN down on the x = 60 end, one static step.
    let script = json!([
        {"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}},
        {"command": "model.set.create", "params": {"name": "all", "geometry": [{"body": "Steps"}]}},
        {"command": "model.set.create", "params": {"name": "root", "geometry": [{"body": "Steps", "face": "F1:-x"}]}},
        {"command": "model.set.create", "params": {"name": "tip", "geometry": [{"body": "Steps", "face": "F1:+x"}]}},
        {"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}, "density": 7.85e-9}},
        {"command": "model.section.create", "params": {"name": "solid", "set": "all", "material": "Steel", "kind": {"type": "solid"}}},
        {"command": "model.boundary_condition.create", "params": {"name": "clamp", "type": "fixed", "target": {"set": "root"}}},
        {"command": "model.load.create", "params": {"name": "push", "type": "force", "target": {"set": "tip"}, "vector": [0, 0, -1000]}},
        {"command": "model.step.create", "params": {"name": "Load", "procedure": {"type": "static"}, "boundary_conditions": ["clamp"], "loads": ["push"]}},
    ]);
    let batch = c.json("batch", json!({"commands": script}));
    assert_eq!(batch["completed"], 9, "{batch}");
    let check = c.json("check_model", json!({}));
    assert_eq!(check["errors"], 0, "{check}");

    // Solve, then read the largest displacement from the contour range and the reactions.
    let solved = c.json("solve", json!({"analysis": "static", "step": "Load"}));
    assert!(solved.is_object(), "{solved}");
    let shown = c.json("show_results", json!({"step": "Load", "field": "displacement", "deformed": true}));
    let max = &shown["contour"]["range"]["max"];
    let largest = max["value"].as_f64().unwrap();
    // Order of magnitude: a 60 mm steel cantilever, 40 × 10 at the tip end, loaded with 1 kN,
    // bends a few micrometres (a coarse linear tet mesh is stiff, so a little less).
    assert!(largest > 1e-4 && largest < 0.1, "{shown}");
    let node = max["node"].as_u64().unwrap();
    let probe = c.json("probe", json!({"step": "Load", "nodes": [node]}));
    let u = probe["nodes"][0]["displacement"].as_array().unwrap();
    let norm = u.iter().map(|v| v.as_f64().unwrap().powi(2)).sum::<f64>().sqrt();
    assert!((norm - largest).abs() <= 1e-9 * largest, "{probe}");
    assert!(u[2].as_f64().unwrap() < 0.0, "the tip goes down: {probe}");
    let reactions = c.json("reactions", json!({"step": "Load"}));
    let fz = reactions["total"]["force"][2].as_f64().unwrap();
    assert!((fz - 1000.0).abs() < 1e-6, "the clamp carries the load: {reactions}");

    // Look at it: a PNG image block, also saved inside the write root.
    let picture = dir.join("load.png");
    let content = c.tool("screenshot", json!({"view": "iso", "width": 480, "height": 320, "path": picture}));
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["mimeType"], "image/png");
    let png = base64_decode(content[0]["data"].as_str().unwrap());
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    assert_eq!(std::fs::read(&picture).unwrap(), png);
    let info: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(info["results"]["step"], "Load", "{info}");

    // Save the model and write a deck, both inside the root.
    c.json("save", json!({"path": dir.join("steps.fieldcraft")}));
    c.json("write_deck", json!({"format": "abaqus", "path": dir.join("steps.inp")}));
    assert!(dir.join("steps.fieldcraft").is_file() && dir.join("steps.inp").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
