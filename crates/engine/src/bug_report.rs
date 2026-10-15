//! Bug report drafts (Help › Report a Bug, and Report it after a crash): a GitHub new-issue link
//! that fills in the bug form (`.github/ISSUE_TEMPLATE/bug.yml`), and the files to attach.
//!
//! Nothing here sends anything. The user reviews the draft, then their browser opens the issue
//! form, which they submit or not.
//!
//! What a draft holds by default: the version, the OS, what went wrong (the user's words, the
//! last error or the crash's message) and, for a crash, where it happened. The user's home
//! folder is shown as `~`. Only with **Share my session** does it also carry the commands that
//! built the model (the replayable journal, which can name the user's files), and only with
//! **Include the model** as well the model file. A session too long for the link (GitHub
//! refuses long new-issue links) goes out as a file to attach and on the clipboard instead; the
//! model file is always one to attach.

use serde_json::{Value, json};

use crate::crash::without_home;

/// The issue form.
pub const ISSUE_URL: &str = "https://github.com/jyviko/fieldcraft/issues/new";
/// The longest link a draft opens: GitHub refuses new-issue links much past 8 KB.
pub const MAX_URL: usize = 7000;
/// The session file a draft attaches.
pub const SESSION_FILE: &str = "fieldcraft-session.json";
/// The model file a draft attaches.
pub const MODEL_FILE: &str = "model.fieldcraft";
/// Backtrace frames a crash's draft quotes.
const FRAMES: usize = 12;
/// A field's text is cut at this many bytes.
const MAX_FIELD: usize = 3000;

/// What to put in a draft.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub title: String,
    /// What happened: the user's words, or the last error.
    pub observed: String,
    /// A crash report (`crash::read`), when reporting one.
    pub crash: Option<Value>,
    /// Attach the replayable commands.
    pub share_session: bool,
    /// The commands (each `{command, params}`) that rebuild the model from `model` (or from an
    /// empty model when there is none).
    pub commands: Vec<Value>,
    /// Attach the model file too (only with `share_session`).
    pub include_model: bool,
    pub model: Option<Vec<u8>>,
}

/// A file the user attaches to the issue.
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub bytes: Vec<u8>,
    /// What it is, for the review.
    pub what: String,
}

/// A draft ready for review.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub title: String,
    /// The bug form's fields by id (`observed`, `reproduction`, `evidence`, `version`), as the
    /// link fills them.
    pub fields: Vec<(&'static str, String)>,
    /// The link that opens the filled form.
    pub url: String,
    /// Files to attach (the session when too long for the link, the model).
    pub attachments: Vec<Attachment>,
    /// Text to put on the clipboard (the session, when it is an attachment).
    pub clipboard: Option<String>,
}

impl Draft {
    /// The draft as JSON (attachments by name, size and what they are).
    pub fn to_json(&self) -> Value {
        json!({
            "title": self.title,
            "fields": self.fields.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<String, Value>>(),
            "url": self.url,
            "attachments": self.attachments.iter().map(|a| json!({"name": a.name, "bytes": a.bytes.len(), "what": a.what})).collect::<Vec<_>>(),
            "clipboard": self.clipboard.as_ref().map(String::len),
        })
    }
}

/// `v0.5.8 (abc1234), macos aarch64`.
pub fn version_line(version: &str, commit: &str) -> String {
    let commit = if commit.is_empty() { String::new() } else { format!(" ({commit})") };
    format!("v{version}{commit}, {} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Percent-encode `s` for a query value (RFC 3986 unreserved characters stay).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn cut(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[cut: {} bytes in all]", s.get(..end).unwrap_or_default(), s.len())
}

fn link(title: &str, fields: &[(&'static str, String)]) -> String {
    let mut url = format!("{ISSUE_URL}?template=bug.yml&title={}", encode(title));
    for (k, v) in fields {
        if !v.is_empty() {
            url.push_str(&format!("&{k}={}", encode(v)));
        }
    }
    url
}

/// What a crash report says, for the evidence field: its kind, message, place, command (its id;
/// its parameters only with the session) and the top of its backtrace.
fn crash_evidence(report: &Value, share: bool) -> String {
    let s = |k: &str| report.get(k).and_then(Value::as_str).unwrap_or("?");
    let mut out =
        format!("Crash report `{}` ({}), seen {} time(s)\n\n", s("id"), s("kind"), report.get("count").and_then(Value::as_u64).unwrap_or(1));
    out.push_str(&format!("Message: {}\nAt: {}\n", without_home(s("message")), without_home(s("location"))));
    if let Some(c) = report.get("command").filter(|c| !c.is_null()) {
        let id = c.get("command").and_then(Value::as_str).unwrap_or("?");
        match c.get("params").filter(|_| share) {
            Some(p) => out.push_str(&format!("Command: `{id}` {}\n", without_home(&p.to_string()))),
            None => out.push_str(&format!("Command: `{id}`\n")),
        }
    }
    if let Some(a) = report.get("last_activity").and_then(Value::as_str) {
        out.push_str(&format!("Last activity: {a}\n"));
    }
    let frames: Vec<&str> = report.get("backtrace").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).take(FRAMES).collect();
    if !frames.is_empty() {
        out.push_str("\nBacktrace (top):\n```\n");
        for f in frames {
            out.push_str(&without_home(f));
            out.push('\n');
        }
        out.push_str("```\n");
    }
    out
}

/// The draft for `input`, by the [`version_line`] `version`.
pub fn draft(input: &Input, version: &str) -> Draft {
    let crash = input.crash.as_ref();
    let mut title = input.title.trim().to_string();
    if title.is_empty() {
        title = match crash.and_then(|c| c.get("kind")).and_then(Value::as_str) {
            Some("unclean_exit") => "FieldCraft closed unexpectedly".to_string(),
            Some(_) => "Internal error".to_string(),
            None => match input.observed.trim().lines().next().filter(|l| !l.is_empty()) {
                Some(line) => format!("Bug: {}", line.chars().take(100).collect::<String>()),
                None => "Bug report".to_string(),
            },
        };
        if let Some(c) = crash.and_then(|c| c.get("command")).and_then(|c| c.get("command")).and_then(Value::as_str) {
            title.push_str(&format!(" in {c}"));
        }
    }
    let title = cut(&without_home(&title), 200);
    let mut observed = without_home(input.observed.trim());
    if observed.is_empty()
        && let Some(m) = crash.and_then(|c| c.get("message")).and_then(Value::as_str)
    {
        observed = without_home(m);
    }
    let observed = cut(&observed, MAX_FIELD);
    let share = input.share_session;
    let mut evidence = crash.map(|c| crash_evidence(c, share)).unwrap_or_default();
    let mut attachments = Vec::new();
    let mut clipboard = None;
    let mut reproduction = String::new();
    // The session: the replayable script, from the attached model when there is one.
    let model = input.model.as_ref().filter(|_| share && input.include_model);
    let script = share.then(|| {
        let mut steps = Vec::new();
        if model.is_some() {
            steps.push(json!({"command": "file.open", "params": {"path": MODEL_FILE}}));
        }
        steps.extend(input.commands.iter().cloned());
        Value::Array(steps)
    });
    if let Some(m) = model {
        attachments.push(Attachment { name: MODEL_FILE.into(), bytes: m.clone(), what: format!("the model ({} KB)", m.len().div_ceil(1024)) });
    }
    if let Some(script) = &script {
        let n = script.as_array().map_or(0, Vec::len);
        reproduction = format!("Replay the attached session ({n} command(s)): `fieldcraft-cli run {SESSION_FILE}`");
        if model.is_some() {
            reproduction.push_str(&format!(" in a folder holding `{MODEL_FILE}` too"));
        } else if crash.is_some_and(|c| c.get("files").and_then(|f| f.get("model")).is_some()) {
            reproduction.push_str(" (the crash replays from the model, which is not attached)");
        }
        reproduction.push('.');
    }
    let inline = |evidence: &str, script: &Value| {
        let text = serde_json::to_string_pretty(script).unwrap_or_default();
        format!("{evidence}\nSession ({SESSION_FILE}):\n```json\n{text}\n```\n")
    };
    let fields = |evidence: String| {
        vec![("observed", observed.clone()), ("reproduction", reproduction.clone()), ("evidence", evidence), ("version", version.to_string())]
    };
    // The session in the link when it fits, else a file to attach (also on the clipboard).
    let mut url = String::new();
    if let Some(script) = &script {
        let candidate = fields(inline(&evidence, script));
        let u = link(&title, &candidate);
        if u.len() <= MAX_URL {
            evidence = inline(&evidence, script);
            url = u;
        } else {
            let text = serde_json::to_string_pretty(script).unwrap_or_default();
            evidence.push_str(&format!(
                "\nThe session is too long for this form: FieldCraft saved it as `{SESSION_FILE}` and copied it to the clipboard. Attach the file here (drag it in) or paste it.\n"
            ));
            attachments.insert(0, Attachment { name: SESSION_FILE.into(), bytes: text.clone().into_bytes(), what: "the session's commands".into() });
            clipboard = Some(text);
        }
    }
    if model.is_some() {
        evidence.push_str(&format!("\nThe model is attached as `{MODEL_FILE}` (drag it in from the folder FieldCraft saved it to).\n"));
    }
    if url.is_empty() || model.is_some() {
        let mut f = fields(evidence.clone());
        url = link(&title, &f);
        // Still too long (a very long message): cut the longest field until it fits.
        while url.len() > MAX_URL {
            let Some((i, len)) = f.iter().enumerate().map(|(i, (_, v))| (i, v.len())).max_by_key(|(_, l)| *l) else { break };
            if len < 200 {
                break;
            }
            if let Some((_, v)) = f.get_mut(i) {
                *v = cut(v, len.saturating_sub(url.len() - MAX_URL + 64).min(len / 2 + 1));
            }
            url = link(&title, &f);
        }
        evidence = f.get(2).map(|(_, v)| v.clone()).unwrap_or_default();
    }
    let fields = fields(evidence);
    // The fields as the link carries them (a cut field included).
    let fields = if url == link(&title, &fields) { fields } else { decode_fields(&url) };
    Draft { title, fields, url, attachments, clipboard }
}

/// The fields a link carries (for the review, when a field had to be cut).
fn decode_fields(url: &str) -> Vec<(&'static str, String)> {
    let query = url.split_once('?').map_or("", |(_, q)| q);
    ["observed", "reproduction", "evidence", "version"]
        .into_iter()
        .map(|k| {
            let v = query.split('&').find_map(|kv| kv.strip_prefix(k).and_then(|r| r.strip_prefix('='))).unwrap_or_default();
            (k, decode(v))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (c, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> String {
        std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| "/home/someone".into())
    }

    fn crash() -> Value {
        json!({
            "id": "20261014-101500-00000000deadbeef", "kind": "command_panic", "count": 2,
            "message": format!("index out of bounds in {}/model.fieldcraft", home()),
            "location": "crates/mesh/src/tet.rs:42",
            "command": {"command": "mesh.tet.generate", "params": {"body": "Plate", "path": format!("{}/secret/plate.step", home())}},
            "backtrace": ["fieldcraft_mesh::tet::insert (crates/mesh/src/tet.rs:42)"],
            "files": {"model": "model.fieldcraft", "replay": "replay.json"},
        })
    }

    #[test]
    fn without_sharing_a_draft_names_no_paths_commands_or_model() {
        let input = Input {
            observed: format!("cannot open {}/parts/bracket.solidcraft", home()),
            crash: Some(crash()),
            commands: vec![json!({"command": "geometry.open", "params": {"path": format!("{}/parts/bracket.solidcraft", home())}})],
            model: Some(b"model bytes".to_vec()),
            include_model: true,
            ..Default::default()
        };
        let d = draft(&input, "v1.2.3, testos x");
        assert!(d.url.starts_with("https://github.com/jyviko/fieldcraft/issues/new?template=bug.yml&title="), "{}", d.url);
        assert!(d.attachments.is_empty() && d.clipboard.is_none(), "nothing attached without Share my session");
        let all = format!("{} {:?} {}", d.title, d.fields, decode(&d.url));
        assert!(!all.contains(&home()), "no home folder: {all}");
        assert!(!all.contains("secret") && !all.contains("geometry.open"), "no parameters or commands: {all}");
        assert!(all.contains("~/parts/bracket.solidcraft"), "the error, its home shown as ~: {all}");
        assert!(all.contains("mesh.tet.generate") && all.contains("tet.rs:42") && all.contains("v1.2.3, testos x"), "{all}");
        assert_eq!(d.title, "Internal error in mesh.tet.generate");
        assert_eq!(decode_fields(&d.url), d.fields, "the review shows what the link carries");
    }

    #[test]
    fn a_short_session_goes_in_the_link() {
        let input = Input {
            title: "Solve fails".into(),
            share_session: true,
            commands: vec![json!({"command": "file.sample", "params": {"name": "cantilever"}}), json!({"command": "solve.run", "params": {}})],
            ..Default::default()
        };
        let d = draft(&input, "v1");
        assert!(d.attachments.is_empty() && d.clipboard.is_none());
        let evidence = &d.fields.iter().find(|(k, _)| *k == "evidence").unwrap().1;
        assert!(evidence.contains("\"command\": \"solve.run\""), "{evidence}");
        assert!(d.url.len() <= MAX_URL);
        assert!(decode(&d.url).contains("file.sample"));
    }

    #[test]
    fn a_long_session_and_the_model_are_attachments() {
        let commands: Vec<Value> = (0..400).map(|i| json!({"command": "mesh.node.add", "params": {"label": i + 1, "xyz": [i, 0, 0]}})).collect();
        let input =
            Input { share_session: true, include_model: true, commands, model: Some(vec![7; 3000]), crash: Some(crash()), ..Default::default() };
        let d = draft(&input, "v1");
        assert!(d.url.len() <= MAX_URL, "{}", d.url.len());
        let names: Vec<&str> = d.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, [SESSION_FILE, MODEL_FILE]);
        let session: Value = serde_json::from_slice(&d.attachments[0].bytes).unwrap();
        assert_eq!(session[0], json!({"command": "file.open", "params": {"path": MODEL_FILE}}), "the replay opens the attached model");
        assert_eq!(session.as_array().unwrap().len(), 401);
        assert_eq!(d.clipboard.as_deref().map(str::as_bytes), Some(d.attachments[0].bytes.as_slice()));
        let text = decode(&d.url);
        assert!(text.contains("too long for this form") && text.contains(MODEL_FILE) && !text.contains("\"label\": 399"), "{text}");
        // With the session shared, the crash's command parameters are shown (home as ~).
        assert!(text.contains("~/secret/plate.step"), "{text}");
    }

    #[test]
    fn a_huge_message_is_cut_to_fit_the_link() {
        let input = Input { observed: "é".repeat(20_000), ..Default::default() };
        let d = draft(&input, "v1");
        assert!(d.url.len() <= MAX_URL, "{}", d.url.len());
        assert!(d.fields[0].1.contains("[cut:"));
        assert_eq!(decode_fields(&d.url), d.fields);
    }

    #[test]
    fn encoding_round_trips() {
        let s = "a b&c=d/é\n```json";
        assert_eq!(decode(&encode(s)), s);
        assert_eq!(encode("a b&"), "a%20b%26");
    }
}
