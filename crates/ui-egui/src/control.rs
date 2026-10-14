//! Programmatic control of the running app (agents, tests). The desktop app reads JSON lines
//! from its socket and hands each request here; replies are `{"ok": true, "result": …}` or
//! `{"ok": false, "error": "…"}`. docs/control-protocol.md lists the methods.
//!
//! Requests are untrusted input: numbers are checked and clamped, and nothing here panics.
//! Adapted from SolidCraft's `crates/ui-egui/src/control.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use std::sync::mpsc::Sender;

use modelcraft_render::StandardView;
use serde_json::{Value, json};

use crate::capture::CaptureSize;
use crate::{ModelApp, UiState};

/// `ui.render`'s image size when none is given.
const RENDER_DEFAULT: [usize; 2] = [1280, 800];

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<Value>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx }, rx)
    }
}

pub enum Outcome {
    Done(Value),
    /// A window screenshot, answered once a frame is presented.
    Screenshot {
        path: Option<String>,
    },
}

fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}

fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}

fn wrap(r: Result<Value, String>) -> Outcome {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

fn button(name: Option<&str>) -> egui::PointerButton {
    match name {
        Some("right") => egui::PointerButton::Secondary,
        Some("middle") => egui::PointerButton::Middle,
        _ => egui::PointerButton::Primary,
    }
}

fn key_from(name: &str) -> Option<egui::Key> {
    egui::Key::from_name(name).or(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(egui::Key::Enter),
        "esc" | "escape" => Some(egui::Key::Escape),
        "delete" => Some(egui::Key::Delete),
        "space" => Some(egui::Key::Space),
        "tab" => Some(egui::Key::Tab),
        _ => None,
    })
}

/// An image side for `ui.render`: absent gives `default`; anything but a whole number of pixels
/// in 1..=MAX_DIM is an error (the renderer also caps the total pixel count).
fn render_dim(p: &Value, key: &str, default: usize) -> Result<usize, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_f64()
            .filter(|x| x.fract() == 0.0 && (1.0..=modelcraft_render::MAX_DIM as f64).contains(x))
            .map(|x| x as usize)
            .ok_or_else(|| format!("`{key}` must be a whole number of pixels in 1..={}", modelcraft_render::MAX_DIM)),
    }
}

pub fn handle(app: &mut ModelApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let f = |k: &str| p.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    let b = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
    let mods = || egui::Modifiers { shift: b("shift"), ctrl: b("ctrl"), command: b("ctrl"), alt: b("alt"), ..Default::default() };
    match req.method.as_str() {
        "engine.execute" => {
            let Some(id) = s("command") else { return err("missing `command`") };
            let params = p.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            wrap(app.run(id, params))
        }
        "engine.script" => {
            let Some(steps) = p.get("commands") else { return err("missing `commands` (a list of {command, params})") };
            wrap(app.run_script(steps).map(|()| json!({"journal": app.session.journal.len()})))
        }
        "engine.commands" => {
            ok(Value::Array(modelcraft_engine::commands().iter().map(|c| json!({"id": c.id, "label": c.label, "params": c.params})).collect()))
        }
        "ui.inspect" => ok(json!({
            "ui": app.ui,
            "camera": app.cam,
            "viewport": app.viewport.rect.map(|r| json!([r.left(), r.top(), r.width(), r.height()])),
            "renderer": if app.viewport.gpu.is_some() { "gpu" } else { "cpu" },
            "gpu": app.gpu_info,
            "dialog": app.dialog.as_ref().map(|d| json!({"command": d.command, "text": d.text, "error": d.error})),
            "status": app.status,
            "model": {"nodes": app.session.model.nodes().len(), "elements": app.session.model.elements().len()},
            "results": app.session.results_report(),
            "probe": app.probe,
            "busy": app.busy(),
            "selected": app.selected.as_ref().map(|(k, n)| json!({"kind": k, "name": n})),
            "nav": app.nav,
            "layout": app.layout.iter().map(|(k, r)| (k.to_string(), json!([r.left(), r.top(), r.width(), r.height()]))).collect::<serde_json::Map<String, Value>>(),
            "prompt": app.prompt.as_ref().map(|p| match p {
                crate::files::Prompt::Unsaved(next) => json!({"unsaved": match next {
                    crate::files::Replace::File(p) => p.clone(),
                    crate::files::Replace::Sample(n) => format!("sample {n}"),
                }}),
                crate::files::Prompt::Recover => json!("recover"),
                crate::files::Prompt::Samples => json!("samples"),
            }),
            "recent": app.recent.paths,
            "version": crate::VERSION,
        })),
        "ui.toolbar" => ok(crate::toolbar::describe(app)),
        "ui.select" => {
            // Select a definition as a browser click does (its properties show on the right).
            let kind = p.get("kind").cloned().and_then(|k| serde_json::from_value::<crate::browser::Def>(k).ok());
            match (kind, s("name")) {
                (Some(kind), Some(name)) if crate::browser::definition(&app.session.model, kind, name).is_some() => {
                    app.selected = Some((kind, name.to_string()));
                    ok(Value::Null)
                }
                (Some(_), Some(name)) => err(format!("no such definition `{name}`")),
                _ => {
                    app.selected = None;
                    ok(Value::Null)
                }
            }
        }
        "ui.set" => {
            let mut cur = serde_json::to_value(&app.ui).unwrap_or(json!({}));
            if let (Some(o), Some(src)) = (cur.as_object_mut(), p.as_object()) {
                for (k, v) in src {
                    if !o.contains_key(k) {
                        return err(format!("unknown setting `{k}`"));
                    }
                    o.insert(k.clone(), v.clone());
                }
            }
            match serde_json::from_value::<UiState>(cur) {
                Ok(u) => {
                    app.ui = u;
                    ok(json!(app.ui))
                }
                Err(e) => err(e),
            }
        }
        "ui.view" => {
            match s("view").unwrap_or("iso") {
                "fit" => app.fit_view(),
                "home" => {
                    app.set_view(StandardView::Iso);
                    app.fit_view();
                }
                v => match StandardView::parse(v) {
                    Some(sv) => app.set_view(sv),
                    None => return err(format!("unknown view `{v}` (front, back, left, right, top, bottom, iso, home, fit)")),
                },
            }
            ok(json!(app.cam))
        }
        "ui.start" => {
            let Some(id) = s("command") else { return err("missing `command`") };
            match app.start_programmatic(id) {
                Ok(()) => ok(json!({"dialog": app.dialog.as_ref().map(|d| d.command)})),
                Err(e) => err(e),
            }
        }
        "ui.dialog" => {
            if app.dialog.is_none() {
                return err("no command form is open (ui.start opens one)");
            }
            if b("cancel") {
                app.dialog = None;
                return ok(Value::Null);
            }
            if let (Some(d), Some(params)) = (app.dialog.as_mut(), p.get("params")) {
                d.text = serde_json::to_string_pretty(params).unwrap_or_default();
            }
            if b("run") {
                return wrap(app.run_dialog());
            }
            ok(json!(app.dialog.as_ref().map(|d| json!({"command": d.command, "text": d.text, "error": d.error}))))
        }
        "ui.prompt" => {
            let Some(prompt) = app.prompt.clone() else { return err("no prompt is open") };
            match (prompt, s("answer")) {
                (crate::files::Prompt::Unsaved(_), Some(a @ ("save" | "discard" | "cancel"))) => {
                    let answer = match a {
                        "save" => crate::files::Answer::Save,
                        "discard" => crate::files::Answer::Discard,
                        _ => crate::files::Answer::Cancel,
                    };
                    app.answer_unsaved(answer);
                    ok(json!({"busy": app.busy()}))
                }
                (crate::files::Prompt::Samples, Some("open")) => match s("name") {
                    Some(name) => {
                        app.prompt = None;
                        app.open_sample(name);
                        ok(json!({"prompt": app.prompt.is_some()}))
                    }
                    None => err("missing `name` (a sample)"),
                },
                (crate::files::Prompt::Recover | crate::files::Prompt::Samples, Some("close" | "cancel")) => {
                    app.prompt = None;
                    ok(Value::Null)
                }
                (crate::files::Prompt::Recover, Some("recover")) => match s("id") {
                    Some(id) => {
                        app.recover(id);
                        ok(json!({"busy": app.busy()}))
                    }
                    None => err("missing `id` (from file.recovery.list)"),
                },
                (_, a) => err(format!(
                    "`answer` {a:?} does not answer this prompt (unsaved: save, discard, cancel; recover: recover with id, close; samples: open with name, close)"
                )),
            }
        }
        "ui.move" | "ui.click" => {
            let (Some(x), Some(y)) = (f("x"), f("y")) else { return err("missing x/y") };
            let pos = egui::pos2(x as f32, y as f32);
            app.synthetic.push(egui::Event::PointerMoved(pos));
            if req.method == "ui.click" {
                let (button, modifiers) = (button(s("button")), mods());
                // `double: true` sends two clicks in one go, which egui reads as a double click.
                for _ in 0..if b("double") { 2 } else { 1 } {
                    app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers });
                    app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers });
                }
            }
            ok(Value::Null)
        }
        "ui.drag" => {
            let (Some(x0), Some(y0), Some(x1), Some(y1)) = (f("x0"), f("y0"), f("x1"), f("y1")) else { return err("missing x0/y0/x1/y1") };
            let (button, modifiers) = (button(s("button")), mods());
            let (a, z) = (egui::pos2(x0 as f32, y0 as f32), egui::pos2(x1 as f32, y1 as f32));
            app.synthetic.push(egui::Event::PointerMoved(a));
            app.synthetic.push(egui::Event::PointerButton { pos: a, button, pressed: true, modifiers });
            let steps = f("steps").unwrap_or(10.0).clamp(1.0, 200.0) as usize;
            for k in 1..=steps {
                app.synthetic.push(egui::Event::PointerMoved(a + (z - a) * (k as f32 / steps as f32)));
            }
            app.synthetic.push(egui::Event::PointerButton { pos: z, button, pressed: false, modifiers });
            ok(Value::Null)
        }
        "ui.scroll" => {
            let (Some(x), Some(y)) = (f("x"), f("y")) else { return err("missing x/y") };
            app.synthetic.push(egui::Event::PointerMoved(egui::pos2(x as f32, y as f32)));
            // Wheel lines; negative zooms out, positive zooms in.
            let lines = f("delta").unwrap_or(1.0).clamp(-100.0, 100.0);
            app.synthetic.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, lines as f32),
                modifiers: Default::default(),
                phase: egui::TouchPhase::Move,
            });
            ok(Value::Null)
        }
        "ui.key" => {
            let Some(key) = s("key").and_then(key_from) else { return err("unknown key") };
            let modifiers = egui::Modifiers { shift: b("shift"), command: b("cmd"), ctrl: b("cmd") || b("ctrl"), ..Default::default() };
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
            ok(Value::Null)
        }
        "ui.text" => {
            app.synthetic.push(egui::Event::Text(s("text").unwrap_or("").to_string()));
            ok(Value::Null)
        }
        "ui.worldToScreen" => {
            let pts: Vec<Value> = p
                .get("points")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(1000)
                .map(|v| {
                    let c = |i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(f64::NAN);
                    crate::viewport::to_screen(app, modelcraft_render::Vec3::new(c(0), c(1), c(2))).map_or(Value::Null, |q| json!([q.x, q.y]))
                })
                .collect();
            ok(json!(pts))
        }
        "ui.resize" => {
            let (Some(w), Some(h)) = (f("width"), f("height")) else { return err("missing width/height") };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w.clamp(320.0, 8192.0) as f32, h.clamp(240.0, 8192.0) as f32)));
            ok(Value::Null)
        }
        "ui.screenshot" => Outcome::Screenshot { path: s("path").map(str::to_string) },
        "ui.capture" => {
            let live = window_size(ctx);
            let dim = |k: &str, default: f32| match p.get(k) {
                None | Some(Value::Null) => Ok(default),
                Some(v) => v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32).ok_or_else(|| format!("`{k}` must be a number")),
            };
            let size = match (dim("width", live.width), dim("height", live.height), dim("scale", live.scale)) {
                (Ok(width), Ok(height), Ok(scale)) => CaptureSize { width, height, scale },
                (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => return err(e),
            };
            Outcome::Done(capture_to(app, size, s("path")))
        }
        "ui.render" => {
            let Some(path) = s("path") else { return err("missing `path`") };
            let (w, h) = match (render_dim(p, "width", RENDER_DEFAULT[0]), render_dim(p, "height", RENDER_DEFAULT[1])) {
                (Ok(w), Ok(h)) => (w, h),
                (Err(e), _) | (_, Err(e)) => return err(e),
            };
            let png = match app.render_png(w, h) {
                Ok(png) => png,
                Err(e) => return err(e),
            };
            match std::fs::write(path, &png) {
                Ok(()) => ok(json!({"path": path, "width": w, "height": h, "bytes": png.len()})),
                Err(e) => err(format!("{path}: {e}")),
            }
        }
        "app.quit" => {
            app.quit_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(Value::Null)
        }
        other => err(format!("unknown method `{other}`")),
    }
}

/// The window's size in points and its pixels per point (1400 × 900 at 1 before a window was
/// laid out).
pub fn window_size(ctx: &egui::Context) -> CaptureSize {
    let r = ctx.content_rect();
    let fits = |v: f32| v.is_finite() && (1.0..=crate::capture::MAX_SIDE as f32).contains(&v);
    let scale = Some(ctx.pixels_per_point()).filter(|s| crate::capture::SCALE_RANGE.contains(s)).unwrap_or(1.0);
    if fits(r.width()) && fits(r.height()) {
        CaptureSize { width: r.width(), height: r.height(), scale }
    } else {
        CaptureSize { width: 1400.0, height: 900.0, scale }
    }
}

/// Capture the whole window offscreen ([`ModelApp::capture`]) and save it as PNG (a reply).
pub fn capture_to(app: &mut ModelApp, size: CaptureSize, path: Option<&str>) -> Value {
    match app.capture(size) {
        Ok(image) => {
            let mut reply = save_screenshot(&image, path);
            if let Some(r) = reply.get_mut("result").and_then(Value::as_object_mut) {
                r.insert("offscreen".into(), json!(true));
                r.insert("scale".into(), json!(size.scale));
            }
            reply
        }
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// Save a screenshot as PNG: at `path`, or `modelcraft-shot.png` in the temporary folder.
pub fn save_screenshot(image: &egui::ColorImage, path: Option<&str>) -> Value {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let Some(png) = modelcraft_render::png(w, h, &rgba) else { return json!({"ok": false, "error": "could not encode the screenshot"}) };
    let path = path.map(str::to_string).unwrap_or_else(|| std::env::temp_dir().join("modelcraft-shot.png").to_string_lossy().to_string());
    match std::fs::write(&path, &png) {
        Ok(()) => json!({"ok": true, "result": {"path": path, "width": w, "height": h}}),
        Err(e) => json!({"ok": false, "error": format!("{path}: {e}")}),
    }
}

#[cfg(test)]
mod tests {
    use modelcraft_engine::Session;

    use super::*;

    fn call(app: &mut ModelApp, method: &str, params: Value) -> Value {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(app, &egui::Context::default(), &req) {
            Outcome::Done(v) => v,
            Outcome::Screenshot { .. } => json!({"screenshot": true}),
        }
    }

    #[test]
    fn engine_view_and_settings_methods() {
        let mut app = ModelApp::new(Session::default());
        let r = call(&mut app, "engine.script", json!({"commands": modelcraft_engine::sample::script()}));
        assert_eq!(r["ok"], true, "{r}");
        let r = call(&mut app, "engine.execute", json!({"command": "model.info"}));
        assert_eq!(r["result"]["elements"]["hex8"], 64);
        assert_eq!(call(&mut app, "engine.execute", json!({"command": "no.such"}))["ok"], false);
        assert!(call(&mut app, "engine.commands", json!({}))["result"].as_array().is_some_and(|a| a.len() == modelcraft_engine::commands().len()));
        let r = call(&mut app, "ui.set", json!({"style": "wireframe", "show_nodes": true}));
        assert_eq!(r["result"]["style"], "wireframe");
        assert_eq!(app.ui.style, crate::Style::Wireframe);
        assert_eq!(call(&mut app, "ui.set", json!({"style": "cartoon"}))["ok"], false);
        assert_eq!(call(&mut app, "ui.set", json!({"nope": 1}))["ok"], false);
        assert_eq!(call(&mut app, "ui.view", json!({"view": "top"}))["ok"], true);
        assert!((app.cam.pitch - std::f64::consts::FRAC_PI_2).abs() < 1e-3);
        assert_eq!(call(&mut app, "ui.view", json!({"view": "sideways"}))["ok"], false);
        assert_eq!(call(&mut app, "ui.inspect", json!({}))["result"]["model"]["elements"], 64 + 162);
        assert_eq!(call(&mut app, "ui.screenshot", json!({})), json!({"screenshot": true}));
        assert_eq!(call(&mut app, "no.method", json!({}))["ok"], false);
    }

    #[test]
    fn the_command_form_is_driven_after_ui_start() {
        let mut app = ModelApp::new(Session::default());
        assert_eq!(call(&mut app, "ui.dialog", json!({"run": true}))["ok"], false);
        assert_eq!(call(&mut app, "ui.start", json!({"command": "mesh.node.add"}))["result"]["dialog"], "mesh.node.add");
        let r = call(&mut app, "ui.dialog", json!({"params": {"label": 3, "xyz": [1, 2, 3]}, "run": true}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(app.dialog.is_none() && app.session.model.node(3).is_some());
    }

    #[test]
    fn ui_render_writes_a_png_of_the_model() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&modelcraft_engine::sample::script()).unwrap();
        let path = std::env::temp_dir().join(format!("modelcraft-render-{}.png", std::process::id()));
        let r = call(&mut app, "ui.render", json!({"path": path.to_string_lossy(), "width": 120, "height": 80}));
        assert_eq!(r["ok"], true, "{r}");
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
        let _ = std::fs::remove_file(&path);
        assert_eq!(call(&mut app, "ui.render", json!({}))["ok"], false);
    }

    #[test]
    fn ui_render_refuses_hostile_sizes() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&modelcraft_engine::sample::script()).unwrap();
        let path = std::env::temp_dir().join(format!("modelcraft-render-hostile-{}.png", std::process::id()));
        let path = path.to_string_lossy();
        for (w, h) in [
            (json!(100_000), json!(100_000)),
            (json!(8192), json!(8192)),
            (json!(1e12), json!(10)),
            (json!(-5), json!(10)),
            (json!(0), json!(10)),
            (json!(10.5), json!(10)),
            (json!("big"), json!(10)),
            (json!(f64::MAX), json!(f64::MIN)),
        ] {
            let r = call(&mut app, "ui.render", json!({"path": path, "width": w, "height": h}));
            assert_eq!(r["ok"], false, "{w} x {h}: {r}");
        }
        assert!(std::fs::metadata(&*path).is_err(), "nothing is written for a refused size");
    }

    #[test]
    fn hostile_requests_never_panic() {
        let mut app = ModelApp::new(Session::default());
        let hostile = [
            json!(null),
            json!([]),
            json!({"x": 1e300, "y": f64::MAX, "x0": -1e308, "y0": 0, "x1": 1e308, "y1": 1, "steps": 1e12, "delta": 1e300}),
            json!({"width": 1e12, "height": -5, "path": ""}),
            json!({"command": 5, "params": "x", "commands": {"a": 1}}),
            json!({"key": "NoSuchKey", "text": 7, "view": null, "points": [[1], "x", null]}),
            json!({"tab": "nope", "command": "mesh.tet.generate"}),
            json!({"tab": 3}),
        ];
        for method in [
            "engine.execute",
            "engine.script",
            "ui.set",
            "ui.view",
            "ui.start",
            "ui.dialog",
            "ui.move",
            "ui.click",
            "ui.drag",
            "ui.scroll",
            "ui.key",
            "ui.text",
            "ui.worldToScreen",
            "ui.resize",
            "ui.render",
            "ui.capture",
            "ui.toolbar",
            "ui.prompt",
            "ui.select",
        ] {
            for p in &hostile {
                let _ = call(&mut app, method, p.clone());
            }
        }
        assert!(app.synthetic.len() < 1000);
    }

    #[test]
    fn the_toolbar_lists_every_entry_and_switches_tabs() {
        let mut app = ModelApp::new(Session::default());
        assert_eq!(call(&mut app, "ui.set", json!({"tab": "mesh"}))["ok"], true);
        let r = call(&mut app, "ui.toolbar", json!({}));
        assert_eq!(r["result"]["tab"], "mesh", "{r}");
        let entries = r["result"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), modelcraft_engine::catalogue::CATALOGUE.len());
        let soon = entries.iter().find(|e| e["id"] == "mesh.order.quadratic").unwrap();
        assert_eq!((soon["coming_soon"].as_str(), soon["command"].is_null()), (Some("M2"), true));
        assert_eq!(call(&mut app, "ui.set", json!({"tab": "nope"}))["ok"], false);
        let r = call(&mut app, "ui.start", json!({"command": "mesh.order.quadratic"}));
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some("Quadratic is coming soon (M2)")));
    }

    #[test]
    fn ui_capture_saves_the_whole_window() {
        let mut app = ModelApp::new(Session::default());
        let path = std::env::temp_dir().join(format!("modelcraft-capture-{}.png", std::process::id()));
        let r = call(&mut app, "ui.capture", json!({"path": path.to_string_lossy(), "width": 640, "height": 400, "scale": 1}));
        assert_eq!(
            (r["ok"].as_bool(), r["result"]["width"].as_u64(), r["result"]["offscreen"].as_bool()),
            (Some(true), Some(640), Some(true)),
            "{r}"
        );
        assert!(std::fs::read(&path).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));
        let _ = std::fs::remove_file(&path);
        for bad in [json!({"width": "x"}), json!({"width": 1e9}), json!({"scale": 0}), json!({"height": -1})] {
            assert_eq!(call(&mut app, "ui.capture", bad.clone())["ok"], false, "{bad}");
        }
    }

    #[test]
    fn screenshots_are_saved_as_png() {
        let image = egui::ColorImage::filled([4, 3], egui::Color32::RED);
        let path = std::env::temp_dir().join(format!("modelcraft-shot-{}.png", std::process::id()));
        let r = save_screenshot(&image, Some(&path.to_string_lossy()));
        assert_eq!(r["ok"], true, "{r}");
        assert!(std::fs::read(&path).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));
        let _ = std::fs::remove_file(&path);
    }
}
