//! The protocol and the tools driven through JSON-RPC lines, as a client sends them, with
//! hostile input: malformed messages, huge parameters, paths outside the roots, unknown tools.

use std::path::{Path, PathBuf};

use fieldcraft_io::roots::Roots;
use serde_json::{Value, json};

use crate::{Headless, MAX_LINE, Options, Server};

fn server() -> Server {
    Server::new(Box::new(Headless::default()))
}

fn confined(read: &Path, write: &Path) -> Server {
    Server::new(Box::new(Headless::new(Roots::new(Some(read), Some(write)).unwrap())))
}

fn send(s: &mut Server, msg: Value) -> Value {
    serde_json::from_str(&s.handle_line(&msg.to_string()).unwrap()).unwrap()
}

/// A tool call: (text of the first content block, is_error).
fn call(s: &mut Server, name: &str, args: Value) -> (String, bool) {
    let r = send(s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}));
    let result = &r["result"];
    assert!(result.is_object(), "{name}: {r}");
    let text = result["content"].as_array().unwrap().iter().find(|c| c["type"] == "text").map(|c| c["text"].as_str().unwrap().to_string());
    (text.unwrap_or_default(), result["isError"] == json!(true))
}

fn ok(s: &mut Server, name: &str, args: Value) -> Value {
    let (text, err) = call(s, name, args.clone());
    assert!(!err, "{name} {args}: {text}");
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("fieldcraft-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(std::fs::canonicalize(&d).unwrap())
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn initialize_lists_tools_and_resources() {
    let mut s = server();
    let r = send(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}));
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(r["result"]["serverInfo"]["name"], "fieldcraft");
    assert!(r["result"]["instructions"].as_str().unwrap().contains("headless"));
    let r = send(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}));
    assert_eq!(r["result"]["protocolVersion"], crate::PROTOCOL_VERSION);
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    assert!(s.is_initialized());
    let tools = send(&mut s, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for t in [
        "list_commands",
        "execute",
        "batch",
        "inspect_model",
        "check_model",
        "open",
        "save",
        "write_deck",
        "mesh",
        "solve",
        "show_results",
        "probe",
        "reactions",
        "screenshot",
        "undo",
        "redo",
    ] {
        assert!(names.contains(&t), "{t}");
    }
    let r = send(&mut s, json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "fieldcraft://model"}}));
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("nodes"));
    let r = send(&mut s, json!({"jsonrpc": "2.0", "id": 5, "method": "resources/read", "params": {"uri": "fieldcraft://commands"}}));
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("solve.static"));
    assert_eq!(send(&mut s, json!({"jsonrpc": "2.0", "id": 6, "method": "ping"}))["result"], json!({}));
}

#[test]
fn malformed_messages_get_json_rpc_errors() {
    let mut s = server();
    let code = |s: &mut Server, line: &str| -> i64 {
        let r: Value = serde_json::from_str(&s.handle_line(line).unwrap()).unwrap();
        r["error"]["code"].as_i64().unwrap()
    };
    assert_eq!(code(&mut s, "{not json"), -32700);
    assert_eq!(code(&mut s, "\"just a string\""), -32600);
    assert_eq!(code(&mut s, "42"), -32600);
    assert_eq!(code(&mut s, "[]"), -32600);
    assert_eq!(code(&mut s, &format!("[{}]", vec!["{}"; 1000].join(","))), -32600);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1}"#), -32600);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#), -32600);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#), -32601);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#), -32602);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"rm_rf"}}"#), -32602);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":[]}}"#), -32602);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{}}}"#), -32602);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{"command":7}}}"#), -32602);
    assert_eq!(
        code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{"command":"x","extra":1}}}"#),
        -32602
    );
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mesh","arguments":{"method":"voodoo"}}}"#), -32602);
    assert_eq!(code(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"file:///etc/passwd"}}"#), -32002);
    // Deeply nested JSON is refused by the parser, not by a stack overflow.
    assert_eq!(code(&mut s, &format!("{}{}", "[".repeat(100_000), "]".repeat(100_000))), -32700);
    // Notifications and responses get no reply; a batch replies to its requests only.
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#).is_none());
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#).is_none());
    let r: Value =
        serde_json::from_str(&s.handle_line(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"x"}]"#).unwrap()).unwrap();
    assert_eq!(r.as_array().unwrap().len(), 1);
}

#[test]
fn serve_survives_huge_lines_and_bad_bytes() {
    let mut s = server();
    let mut input = Vec::new();
    input.extend(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
    input.push(b'\n');
    input.extend(br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"execute","arguments":{"command":"model.info","params":{"x":""#);
    input.extend(vec![b'a'; MAX_LINE + 10]);
    input.extend(b"\"}}}}\n");
    input.extend([0xff, 0xfe, b'\n']);
    input.extend(br#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#);
    input.push(b'\n');
    let mut out = Vec::new();
    s.serve(input.as_slice(), &mut out).unwrap();
    let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), 4, "{replies:?}");
    assert_eq!(replies[0]["id"], 1);
    assert!(replies[1]["error"]["message"].as_str().unwrap().contains("too large"));
    assert_eq!(replies[2]["error"]["code"], -32700);
    assert_eq!(replies[3]["id"], 3, "the server keeps serving");
}

#[test]
fn hostile_tool_arguments_are_tool_errors() {
    let mut s = server();
    let huge = "x".repeat(1 << 20);
    for (tool, args) in [
        ("execute", json!({"command": "nope.nothing"})),
        ("execute", json!({"command": huge})),
        ("execute", json!({"command": "mesh.node.add", "params": {"label": -1, "xyz": [1e308, null, "a"]}})),
        ("execute", json!({"command": "mesh.element.add", "params": {"label": 1, "kind": "hex20", "nodes": vec![u32::MAX; 20]}})),
        ("execute", json!({"command": "file.open", "params": {"path": "\u{0}"}})),
        ("open", json!({"path": "/etc/passwd"})),
        ("open", json!({"path": "/no/such/model.fieldcraft"})),
        ("open", json!({"path": huge.clone() + ".solidcraft"})),
        ("save", json!({})),
        ("write_deck", json!({"format": "abaqus", "path": "/no/such/dir/x.inp"})),
        ("mesh", json!({"method": "tet", "params": {"body": "Nope", "tolerance": 0.5}})),
        ("mesh", json!({"method": "tet", "params": {"surface": 1e300}})),
        ("solve", json!({"analysis": "static"})),
        ("solve", json!({"analysis": "frequency", "step": "Nope", "options": {"max_dofs": -1}})),
        ("show_results", json!({})),
        ("show_results", json!({"bands": 5})),
        ("show_results", json!({"field": "von_mises"})),
        ("probe", json!({"nodes": [1, 2, 3]})),
        ("reactions", json!({"sets": ["nope"], "about": [1e308, 0, 0]})),
        ("screenshot", json!({"view": "iso", "path": "/no/such/dir/x.png"})),
        ("batch", json!({"commands": [{"command": "model.info"}, {"params": {}}, {"command": "model.info"}]})),
        ("batch", json!({"commands": vec![json!({"command": "model.info"}); crate::tools::MAX_STEPS + 1]})),
        ("undo", json!({})),
        ("redo", json!({})),
    ] {
        let (text, err) = call(&mut s, tool, args.clone());
        assert!(err, "{tool} should fail: {text}");
        assert!(text.len() < 100_000, "{tool}: errors stay short");
    }
    // Nothing changed on the way.
    assert_eq!(ok(&mut s, "inspect_model", json!({}))["model"]["nodes"], 0);
    // A huge but valid image size is clamped, not refused or allocated.
    let r = send(
        &mut s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "screenshot", "arguments": {"width": u64::MAX, "height": 16, "source": "model"}}}),
    );
    assert_eq!(r["result"]["content"][0]["type"], "image", "{r}");
}

#[test]
fn a_batch_stops_at_the_first_error() {
    let mut s = server();
    let r = call(
        &mut s,
        "batch",
        json!({"commands": [
            {"command": "mesh.node.add", "params": {"label": 1, "xyz": [0, 0, 0]}},
            {"command": "mesh.node.add", "params": {"label": 0, "xyz": [0, 0, 0]}},
            {"command": "mesh.node.add", "params": {"label": 2, "xyz": [0, 0, 0]}},
        ]}),
    );
    assert!(r.1);
    let v: Value = serde_json::from_str(&r.0).unwrap();
    assert_eq!((v["completed"].clone(), v["failed"]["index"].clone()), (json!(1), json!(1)));
    assert_eq!(ok(&mut s, "inspect_model", json!({}))["model"]["nodes"], 1);
    ok(&mut s, "undo", json!({}));
    assert_eq!(ok(&mut s, "inspect_model", json!({}))["model"]["nodes"], 0);
    let listed = ok(&mut s, "list_commands", json!({"filter": "SOLVE."}));
    assert!(listed.as_array().unwrap().iter().all(|c| c["id"].as_str().unwrap().starts_with("solve.")));
}

#[test]
fn file_access_stays_inside_the_roots() {
    let d = Dir::new("roots");
    let (input, output, outside) = (d.0.join("in"), d.0.join("out"), d.0.join("outside"));
    for f in [&input, &output, &outside] {
        std::fs::create_dir_all(f).unwrap();
    }
    let design = fieldcraft_testkit::corpus::Analytic::Cylinder { radius: 5.0, height: 10.0 }.design().unwrap();
    std::fs::write(outside.join("c.solidcraft"), &design).unwrap();
    std::fs::write(input.join("c.solidcraft"), &design).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.join("c.solidcraft"), input.join("link.solidcraft")).unwrap();
    let mut s = confined(&input, &output);
    let refused = |s: &mut Server, tool: &str, args: Value| {
        let (text, err) = call(s, tool, args.clone());
        assert!(err && text.contains("automation"), "{tool} {args}: {text}");
    };
    refused(&mut s, "open", json!({"path": outside.join("c.solidcraft")}));
    #[cfg(unix)]
    refused(&mut s, "open", json!({"path": input.join("link.solidcraft")}));
    refused(&mut s, "execute", json!({"command": "geometry.open", "params": {"path": input.join("../outside/c.solidcraft")}}));
    ok(&mut s, "open", json!({"path": input.join("c.solidcraft")}));
    refused(&mut s, "save", json!({"path": outside.join("m.fieldcraft")}));
    refused(&mut s, "save", json!({"path": input.join("m.fieldcraft")}));
    refused(&mut s, "execute", json!({"command": "file.save_as", "params": {"path": outside.join("m.fieldcraft")}}));
    refused(&mut s, "write_deck", json!({"format": "abaqus", "path": outside.join("m.inp")}));
    refused(&mut s, "screenshot", json!({"path": outside.join("x.png")}));
    refused(&mut s, "screenshot", json!({"path": input.join("x.png")}));
    refused(&mut s, "execute", json!({"command": "file.recovery.list", "params": {"dir": outside}}));
    ok(&mut s, "save", json!({"path": output.join("m.fieldcraft")}));
    ok(&mut s, "screenshot", json!({"path": output.join("x.png")}));
    assert!(output.join("m.fieldcraft").is_file() && output.join("x.png").is_file());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1, "nothing written outside");
}

#[test]
fn roots_imply_headless() {
    let d = Dir::new("connect");
    let options = Options { connect: Some("127.0.0.1:1".into()), roots: Roots::new(Some(&d.0), None).unwrap() };
    assert!(crate::backend(options).err().unwrap().contains("cannot be combined"));
    // An app that cannot be reached: headless, with a note.
    let (b, note) = crate::backend(Options { connect: Some("127.0.0.1:1".into()), roots: Roots::default() }).unwrap();
    assert!(note.unwrap().contains("running headless"));
    assert!(b.describe().contains("headless"));
}

#[test]
fn base64_matches_the_rfc_vectors() {
    use crate::tools::base64;
    for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
        assert_eq!(base64(i.as_bytes()), o);
    }
}
