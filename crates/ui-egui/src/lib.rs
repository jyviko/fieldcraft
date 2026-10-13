//! The ModelCraft egui front end.
//!
//! A thin shell over [`modelcraft_engine::Session`]: panels read engine state and act through
//! commands ([`ModelApp::run`]); nothing here owns model data. The window has a menu bar, the
//! model tree on the left, the 3D viewport in the middle, the commands (generated from the
//! engine's registry) on the right and a status bar at the bottom. The control channel's
//! requests are handled here ([`control`]); the desktop app owns the socket.
//!
//! Structure and conventions follow SolidCraft's `crates/ui-egui` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod control;
pub mod gpu;
pub mod panels;
pub mod scene;
pub mod theme;
pub mod viewport;

use std::sync::mpsc::{Receiver, Sender};

use modelcraft_engine::Session;
use modelcraft_render::{Camera, DisplayMesh, Scene, StandardView};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use control::ControlRequest;
pub use scene::Style;

/// The version shown in Help and over the control channel.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// View settings: what is shown and how. `ui.set` changes any of them by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub style: Style,
    pub show_nodes: bool,
    pub perspective: bool,
    pub dark: bool,
    pub show_tree: bool,
    pub show_commands: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState { style: Style::ShadedEdges, show_nodes: false, perspective: false, dark: true, show_tree: true, show_commands: true }
    }
}

/// The parameter form a command opens when started from the Commands panel.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamsDialog {
    pub command: &'static str,
    /// The parameters as JSON text.
    pub text: String,
    pub error: Option<String>,
}

/// A status bar message.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

/// The viewport's renderer state.
#[derive(Default)]
pub struct Viewport {
    /// Where the viewport was drawn last frame (points).
    pub rect: Option<egui::Rect>,
    /// The GPU target when drawing with wgpu; `None` draws on the CPU.
    pub gpu: Option<gpu::GpuTarget>,
    pub slot: gpu::SceneSlot,
    /// The scene key last handed to the GPU.
    pub uploaded: Option<u64>,
    /// CPU fallback: the texture and what it shows (scene key, camera, size).
    cpu: Option<(egui::TextureHandle, u64, Camera, [usize; 2])>,
}

/// A screenshot asked for over the control channel.
struct Shot {
    token: u64,
    path: Option<String>,
    reply: Sender<Value>,
    /// Frames waited so far: the request's own changes get drawn before the picture is taken.
    frames: u32,
    requested: bool,
    /// App time after which the request is answered with an error.
    deadline: f64,
}

pub struct ModelApp {
    pub session: Session,
    pub ui: UiState,
    pub cam: Camera,
    pub status: Option<Status>,
    pub dialog: Option<ParamsDialog>,
    pub viewport: Viewport,
    /// The GPU adapter, for the status bar and diagnostics.
    pub gpu_info: Option<String>,
    /// Bumped whenever the model may have changed.
    revision: u64,
    display: (Option<u64>, DisplayMesh),
    /// The built scene and its key.
    scene: (Option<u64>, Scene),
    /// The theme last applied to the context.
    styled: Option<bool>,
    /// Input injected by the control channel, fed to egui one pointer event per frame.
    pub synthetic: Vec<egui::Event>,
    synthetic_mods: egui::Modifiers,
    control_rx: Option<Receiver<ControlRequest>>,
    shots: Vec<Shot>,
    shot_token: u64,
    /// The app time (seconds) of the current frame.
    pub now: f64,
    pub quit_requested: bool,
}

impl ModelApp {
    pub fn new(session: Session) -> ModelApp {
        let mut app = ModelApp {
            session,
            ui: UiState::default(),
            cam: Camera::default(),
            status: None,
            dialog: None,
            viewport: Viewport::default(),
            gpu_info: None,
            revision: 0,
            display: (None, DisplayMesh::default()),
            scene: (None, Scene::default()),
            styled: None,
            synthetic: Vec::new(),
            synthetic_mods: egui::Modifiers::default(),
            control_rx: None,
            shots: Vec::new(),
            shot_token: 0,
            now: 0.0,
            quit_requested: false,
        };
        app.fit_view();
        app
    }

    /// Serve control-channel requests (see [`control`]).
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Draw the viewport on the GPU (eframe's wgpu render state, with the app's depth buffer
    /// bits and MSAA sample count). Without it the viewport renders on the CPU.
    pub fn set_wgpu(&mut self, rs: &egui_wgpu::RenderState, depth_bits: u8, samples: u32) {
        self.viewport.gpu = Some(gpu::install(rs, depth_bits, samples));
        let info = rs.adapter.get_info();
        self.gpu_info = Some(format!("{} ({:?})", info.name.trim(), info.backend));
    }

    /// Run a command with JSON parameters: what every panel, the control channel and scripts
    /// do. The result or the error goes to the status bar.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let was_empty = self.session.model.nodes().is_empty();
        let journal = self.session.journal.len();
        let label = command_label(id);
        let r = self.session.execute(id, &params).map_err(|e| e.to_string());
        let changed = self.session.journal.len() != journal;
        if changed {
            self.model_changed();
        }
        match &r {
            Ok(v) if v.is_null() || changed => self.set_status(format!("{label} done"), false),
            Ok(v) => self.set_status(format!("{label}: {v}"), false),
            Err(e) => self.set_status(format!("{label}: {e}"), true),
        }
        if was_empty && !self.session.model.nodes().is_empty() {
            self.fit_view();
        }
        r
    }

    /// Run a script (a JSON array of `{command, params}`), stopping at the first error.
    pub fn run_script(&mut self, script: &Value) -> Result<(), String> {
        let was_empty = self.session.model.nodes().is_empty();
        let r = self.session.run_script(script).map_err(|e| e.to_string());
        self.model_changed();
        match &r {
            Ok(()) => self.set_status("Script done", false),
            Err(e) => self.set_status(format!("Script: {e}"), true),
        }
        if was_empty && !self.session.model.nodes().is_empty() {
            self.fit_view();
        }
        r
    }

    /// What clicking a command in the Commands panel does: a command without parameters runs,
    /// one with parameters opens its form. Programmatic calls use [`ModelApp::run`] instead.
    pub fn start(&mut self, id: &str) -> Result<(), String> {
        let spec = modelcraft_engine::find_command(id).ok_or_else(|| format!("unknown command `{id}`"))?;
        if spec.params.trim() == "{}" {
            self.dialog = None;
            self.run(spec.id, json!({})).map(|_| ())
        } else {
            self.dialog = Some(ParamsDialog { command: spec.id, text: "{}".into(), error: None });
            Ok(())
        }
    }

    /// Run the open parameter form; it closes when the command succeeds.
    pub fn run_dialog(&mut self) -> Result<Value, String> {
        let Some(d) = self.dialog.clone() else { return Err("no command form is open".into()) };
        let r = serde_json::from_str::<Value>(&d.text).map_err(|e| format!("the parameters are not JSON: {e}")).and_then(|p| self.run(d.command, p));
        match &r {
            Ok(_) => self.dialog = None,
            Err(e) => {
                if let Some(open) = self.dialog.as_mut() {
                    open.error = Some(e.clone());
                }
            }
        }
        r
    }

    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error {
            log::warn!("{text}");
        }
        self.status = Some(Status { text, error });
    }

    /// The model may have changed: display data and the scene are rebuilt when next drawn.
    pub fn model_changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// The model's display data, rebuilt when the model changed.
    pub fn display(&mut self) -> &DisplayMesh {
        if self.display.0 != Some(self.revision) {
            self.display = (Some(self.revision), DisplayMesh::from_model(&self.session.model));
        }
        &self.display.1
    }

    /// The key of the current scene: the model revision and every setting the scene depends on.
    pub fn scene_key(&self) -> u64 {
        let style = Style::ALL.iter().position(|s| *s == self.ui.style).unwrap_or(0) as u64;
        (self.revision << 4) | (style << 2) | (u64::from(self.ui.show_nodes) << 1) | u64::from(self.ui.dark)
    }

    /// The scene the viewport draws, rebuilt when its key changes; its radius fits the camera.
    pub fn scene(&mut self) -> (u64, &Scene) {
        let key = self.scene_key();
        if self.scene.0 != Some(key) {
            let t = theme::Tokens::get(self.ui.dark);
            let (style, nodes) = (self.ui.style, self.ui.show_nodes);
            let s = scene::build(self.display(), style, nodes, &t);
            self.scene = (Some(key), s);
        }
        self.scene.1.radius = scene::radius(&self.display.1, &self.cam);
        (key, &self.scene.1)
    }

    /// A CPU render of the viewport's scene with the current camera, as PNG bytes.
    pub fn render_png(&mut self, w: usize, h: usize) -> Option<Vec<u8>> {
        let cam = self.cam;
        let (_, scene) = self.scene();
        modelcraft_render::render_png(scene, &cam, w, h)
    }

    /// Frame the whole model.
    pub fn fit_view(&mut self) {
        let b = self.display().bounds;
        self.cam.fit(&b);
    }

    pub fn set_view(&mut self, v: StandardView) {
        self.cam.set_view(v);
    }

    /// Per-frame logic before layout: theme, camera projection, control requests, screenshots.
    pub fn logic(&mut self, ctx: &egui::Context) {
        self.now = ctx.input(|i| i.time);
        if self.styled != Some(self.ui.dark) {
            theme::apply(ctx, self.ui.dark);
            self.styled = Some(self.ui.dark);
        }
        self.drain_control(ctx);
        self.cam.fov = if self.ui.perspective { PERSPECTIVE_FOV } else { 0.0 };
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        }
        self.screenshots(ctx);
    }

    /// Inject synthetic events: one pointer event per frame (egui reads a press and release in
    /// the same frame as a click, but a drag needs frames between its moves), keys up to their
    /// release.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        let Some(first) = self.synthetic.first() else { return };
        let n = match first {
            egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. } => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        if let egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. } = first {
            raw.events.push(egui::Event::PointerMoved(*p));
        }
        // egui keeps the modifier keys as input state, changed by their own event.
        match first {
            egui::Event::PointerButton { modifiers, pressed, .. } => {
                // A drag keeps its press's modifiers until the release.
                self.synthetic_mods = if *pressed { *modifiers } else { egui::Modifiers::default() };
                raw.events.push(egui::Event::ModifiersChanged(*modifiers));
            }
            egui::Event::Key { modifiers, .. } => raw.events.push(egui::Event::ModifiersChanged(*modifiers)),
            egui::Event::PointerMoved(_) => raw.events.push(egui::Event::ModifiersChanged(self.synthetic_mods)),
            _ => {}
        }
        let n = n.min(self.synthetic.len());
        raw.events.extend(self.synthetic.drain(..n));
    }

    /// Lay out the whole window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        panels::menu_bar(self, ui);
        panels::status_bar(self, ui);
        if self.ui.show_tree {
            panels::model_tree(self, ui);
        }
        if self.ui.show_commands {
            panels::commands(self, ui);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| viewport::show(self, ui));
        panels::params_dialog(self, ui.ctx());
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        /// Requests run on this thread, so a flood would starve drawing; the rest wait for the
        /// next frame.
        const PER_FRAME: usize = 64;
        let Some(rx) = self.control_rx.take() else { return };
        for _ in 0..PER_FRAME {
            let Ok(req) = rx.try_recv() else { break };
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = req.reply.send(v);
                }
                control::Outcome::Screenshot { path } => {
                    self.shot_token = self.shot_token.wrapping_add(1);
                    let deadline = self.now + SHOT_TIMEOUT;
                    self.shots.push(Shot { token: self.shot_token, path, reply: req.reply.clone(), frames: 0, requested: false, deadline });
                }
            }
            ctx.request_repaint();
        }
        self.control_rx = Some(rx);
    }

    /// Ask for pending screenshots once their request's changes are drawn, and save the ones
    /// that arrived.
    fn screenshots(&mut self, ctx: &egui::Context) {
        if self.shots.is_empty() {
            return;
        }
        let arrived: Vec<(u64, std::sync::Arc<egui::ColorImage>)> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).map(|t| (*t, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in arrived {
            if let Some(i) = self.shots.iter().position(|s| s.token == token) {
                let shot = self.shots.remove(i);
                let _ = shot.reply.send(control::save_screenshot(&image, shot.path.as_deref()));
            }
        }
        let now = self.now;
        self.shots.retain_mut(|s| {
            if now > s.deadline {
                let _ = s.reply.send(json!({"ok": false, "error": "no frame was presented (window hidden?); use ui.render"}));
                return false;
            }
            s.frames += 1;
            if !s.requested && s.frames >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(s.token)));
                s.requested = true;
            }
            true
        });
        if !self.shots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

/// Seconds a screenshot waits for a presented frame.
const SHOT_TIMEOUT: f64 = 8.0;
/// Vertical field of view of the perspective camera (radians).
const PERSPECTIVE_FOV: f64 = 0.6;

/// A command's name as people see it (its label).
pub fn command_label(id: &str) -> &str {
    modelcraft_engine::find_command(id).map_or(id, |c| c.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_run_through_the_engine_and_report_in_the_status_bar() {
        let mut app = ModelApp::new(Session::default());
        let key = app.scene_key();
        app.run("mesh.node.add", json!({"label": 1, "xyz": [0, 0, 0]})).unwrap();
        assert_eq!(app.status.as_ref().map(|s| s.text.as_str()), Some("Add Node done"));
        assert_ne!(app.scene_key(), key, "a model change rebuilds the scene");
        let err = app.run("mesh.node.add", json!({"label": 0, "xyz": [0, 0, 0]})).unwrap_err();
        assert!(app.status.as_ref().is_some_and(|s| s.error && s.text.starts_with("Add Node:")), "{err}");
        app.run("model.info", json!({})).unwrap();
        assert!(app.status.as_ref().is_some_and(|s| s.text.contains("\"nodes\":1")));
    }

    #[test]
    fn starting_a_command_opens_its_form_and_running_it_closes_the_form() {
        let mut app = ModelApp::new(Session::default());
        app.start("model.info").unwrap();
        assert!(app.dialog.is_none());
        app.start("mesh.node.add").unwrap();
        assert_eq!(app.dialog.as_ref().map(|d| d.command), Some("mesh.node.add"));
        app.dialog.as_mut().unwrap().text = "not json".into();
        assert!(app.run_dialog().is_err());
        assert!(app.dialog.as_ref().is_some_and(|d| d.error.is_some()));
        app.dialog.as_mut().unwrap().text = r#"{"label": 7, "xyz": [1, 2, 3]}"#.into();
        app.run_dialog().unwrap();
        assert!(app.dialog.is_none());
        assert!(app.session.model.node(7).is_some());
        assert!(app.start("no.such").is_err());
    }

    #[test]
    fn the_sample_fills_the_view_and_renders() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&modelcraft_engine::sample::script()).unwrap();
        let d = app.display().clone();
        assert!(!d.tris.is_empty() && !d.edges.is_empty());
        // Fitted: the model's centre is the camera target.
        assert!(app.cam.target.dist(d.bounds.center()) < 1e-9);
        let png = app.render_png(64, 48).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
