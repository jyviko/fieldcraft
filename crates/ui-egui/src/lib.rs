//! The ModelCraft egui front end.
//!
//! A thin shell over [`modelcraft_engine::Session`]: panels read engine state and act through
//! commands ([`ModelApp::run`]); nothing here owns model data. The window follows the Craft
//! family's layout (SolidCraft, VectorCraft):
//!
//! - the title bar ([`appbar`]): the app, quick actions (File menu, save, undo, redo, Solve,
//!   search, theme, help) and the model's document tab;
//! - the ribbon ([`toolbar`]): the workspace switcher, its tabs, and panels of icon buttons whose
//!   labelled drop-downs list the whole catalogue (coming-soon entries greyed with their
//!   milestone);
//! - the browser on the left ([`browser`]): the model's geometry, mesh and definitions;
//! - the viewport ([`viewport`]) with its navigation bar and orientation gizmo;
//! - the context panel on the right ([`context`]), only while there is context: the open command
//!   form, the results controls, the selected definition's properties;
//! - the command line and status ([`palette`]), and the journal timeline at the bottom
//!   ([`timeline`]). Every command is also one search away (S or Cmd/Ctrl+K).
//!
//! The control channel's requests are handled here ([`control`]); the desktop app owns the
//! socket. Structure and conventions follow SolidCraft's `crates/ui-egui` (MIT OR Apache-2.0),
//! see ATTRIBUTION.md.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod appbar;
pub mod browser;
pub mod capture;
pub mod context;
pub mod control;
pub mod files;
pub mod frame;
pub mod gpu;
pub mod icons;
pub mod palette;
pub mod results;
pub mod results_panel;
pub mod scene;
pub mod theme;
pub mod timeline;
pub mod titlebar;
pub mod toolbar;
pub mod viewport;

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use modelcraft_engine::Session;
use modelcraft_engine::catalogue::{self, Tab, Workspace};
use modelcraft_render::{Aabb3, Camera, DisplayMesh, Scene, StandardView};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use control::ControlRequest;
pub use files::{Filter, Services};
pub use scene::Style;

/// The version shown in Help and over the control channel.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// View settings: what is shown and how, and which menu or search is open. `ui.set` changes any
/// of them by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub style: Style,
    pub show_nodes: bool,
    /// The open design and imported surfaces.
    pub show_geometry: bool,
    /// Elements.
    pub show_mesh: bool,
    pub perspective: bool,
    pub dark: bool,
    /// The browser (left).
    pub show_tree: bool,
    /// The journal timeline (bottom).
    pub show_timeline: bool,
    /// The workspace shown, and its toolbar tab.
    pub workspace: Workspace,
    pub tab: Tab,
    /// The command search (S, Cmd/Ctrl+K).
    pub palette: bool,
    /// The open menu: `file`, `help`, `workspace`, `view`, or a toolbar panel's drop-down as
    /// `<tab>/<panel>` (`mesh/Tet`).
    pub menu: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            style: Style::ShadedEdges,
            show_nodes: false,
            show_geometry: true,
            show_mesh: true,
            perspective: false,
            dark: true,
            show_tree: true,
            show_timeline: true,
            workspace: Workspace::default(),
            tab: Tab::default(),
            palette: false,
            menu: None,
        }
    }
}

impl UiState {
    /// What the viewport shows.
    pub fn show(&self) -> scene::Show {
        scene::Show { geometry: self.show_geometry, mesh: self.show_mesh, nodes: self.show_nodes }
    }
}

/// What a left drag in the viewport does (the navigation bar's modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NavMode {
    #[default]
    Orbit,
    Pan,
    Zoom,
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
    /// CPU fallback: the scene key the renderer last refused.
    cpu_failed: Option<u64>,
    /// GPU errors reported by the device (see [`gpu::GpuErrors`]), shown in the status bar.
    pub gpu_errors: gpu::GpuErrors,
}

/// A probed node: its values as lines of text, for the step, mode and frame they were read from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Probe {
    pub node: u32,
    pub step: String,
    pub mode: usize,
    /// The explicit field frame (`None`: the last).
    pub frame: Option<usize>,
    pub lines: Vec<String>,
}

/// What a results plot was built for.
#[derive(Clone, Debug, PartialEq)]
struct PlotKey {
    revision: u64,
    /// The result's identity: every solve stores a new one.
    result: usize,
    current: bool,
    view: modelcraft_engine::results::ResultsView,
    /// The mode and field frame drawn (an animated explicit step changes frames).
    pick: modelcraft_engine::results::Pick,
    frame: Option<u64>,
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
    pub icons: icons::IconCache,
    /// The toolbar's buttons drawn last frame, by catalogue id (for the control channel).
    pub toolbar_rects: Vec<(&'static str, egui::Rect, toolbar::Fit)>,
    /// The GPU adapter, for the status bar and diagnostics.
    pub gpu_info: Option<String>,
    /// Bumped whenever the model may have changed.
    revision: u64,
    display: (Option<u64>, DisplayMesh),
    /// The results plot and what it was built for.
    plot: (Option<PlotKey>, Option<Arc<results::Plot>>),
    /// The node probed last (a click on a results plot).
    pub probe: Option<Probe>,
    /// The reaction sums asked for last in the results panel.
    pub reactions: Option<Value>,
    /// Whether results were shown last frame (the Results tab comes up when they appear).
    results_shown: bool,
    /// Tessellated design bodies, kept while the same design is open.
    bodies: (Option<Arc<modelcraft_geometry::Design>>, Vec<modelcraft_geometry::SurfaceMesh>),
    /// The frame-independent part of the results plot (scale, contour) and what it was built for.
    prepared: (Option<PlotKey>, results::Prepared),
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
    /// The offscreen context `ui.capture` draws the window with (made on first use).
    offscreen: Option<Box<capture::Offscreen>>,
    /// The app time (seconds) of the current frame.
    pub now: f64,
    pub quit_requested: bool,
    /// Host services: native file pickers, the recent-files file.
    pub services: files::Services,
    /// Recently opened and saved files.
    pub recent: files::Recent,
    /// The file opening in the background, and files waiting to open after it.
    pub loading: Option<files::Loading>,
    pub queued: std::collections::VecDeque<String>,
    /// The question the window is asking (unsaved changes, recover).
    pub prompt: Option<files::Prompt>,
    /// The window draws its own title bar and caption buttons (Windows, Linux).
    pub custom_titlebar: bool,
    /// The title bar runs under the macOS traffic lights.
    pub integrated_titlebar: bool,
    /// The command search's query and the command line's text.
    pub palette_text: String,
    pub command_line: String,
    /// Browser sections that are unfolded.
    pub tree_open: std::collections::BTreeSet<String>,
    /// The definition selected in the browser: (kind, name), its properties on the right.
    pub selected: Option<(browser::Def, String)>,
    /// What a left drag in the viewport does.
    pub nav: NavMode,
    /// Where the window's regions were drawn last frame (`ui.inspect`'s `layout`).
    pub layout: std::collections::BTreeMap<&'static str, egui::Rect>,
    /// The toolbar's panel labels drawn last frame, by `<tab>/<panel>`.
    pub panel_rects: Vec<(String, egui::Rect)>,
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
            icons: icons::IconCache::default(),
            toolbar_rects: Vec::new(),
            gpu_info: None,
            revision: 0,
            display: (None, DisplayMesh::default()),
            plot: (None, None),
            probe: None,
            reactions: None,
            results_shown: false,
            bodies: (None, Vec::new()),
            prepared: (None, results::Prepared::default()),
            scene: (None, Scene::default()),
            styled: None,
            synthetic: Vec::new(),
            synthetic_mods: egui::Modifiers::default(),
            control_rx: None,
            shots: Vec::new(),
            shot_token: 0,
            offscreen: None,
            now: 0.0,
            quit_requested: false,
            services: files::Services::default(),
            recent: files::Recent::default(),
            loading: None,
            queued: std::collections::VecDeque::new(),
            prompt: None,
            custom_titlebar: false,
            integrated_titlebar: false,
            palette_text: String::new(),
            command_line: String::new(),
            tree_open: ["mesh", "results"].into_iter().map(str::to_string).collect(),
            selected: None,
            nav: NavMode::default(),
            layout: std::collections::BTreeMap::new(),
            panel_rects: Vec::new(),
        };
        app.fit_view();
        app
    }

    /// Serve control-channel requests (see [`control`]).
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Use the host's services (file pickers, the recent-files file) and load the recent list.
    pub fn with_services(mut self, services: files::Services) -> Self {
        self.recent = files::Recent::load(services.recent_path.clone());
        self.services = services;
        self
    }

    /// Draw the viewport on the GPU (eframe's wgpu render state, with the app's depth buffer
    /// bits and MSAA sample count). Without it the viewport renders on the CPU. Device errors
    /// show in the status bar.
    pub fn set_wgpu(&mut self, rs: &egui_wgpu::RenderState, depth_bits: u8, samples: u32, ctx: &egui::Context) {
        self.viewport.gpu = Some(gpu::install(rs, depth_bits, samples, self.viewport.gpu_errors.clone(), ctx.clone()));
        let info = rs.adapter.get_info();
        self.gpu_info = Some(format!("{} ({:?})", info.name.trim(), info.backend));
    }

    /// Run a command with JSON parameters: what every panel, the control channel and scripts
    /// do. The result or the error goes to the status bar.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if let Some(busy) = self.busy() {
            let e = format!("{busy}: wait for it or cancel it");
            self.set_status(e.clone(), true);
            return Err(e);
        }
        let revision = self.session.revision();
        let label = command_label(id);
        let r = self.session.execute(id, &params).map_err(|e| e.to_string());
        let changed = self.session.revision() != revision;
        if changed {
            self.model_changed();
        }
        // The results display commands reply with what is shown, which the viewport shows.
        let display = (id.starts_with("results.") && id != "results.reactions") || id == "solve.run";
        match &r {
            Ok(v) if v.is_null() || changed || display => self.set_status(format!("{label} done"), false),
            Ok(v) => self.set_status(format!("{label}: {v}"), false),
            Err(e) => self.set_status(format!("{label}: {e}"), true),
        }
        r
    }

    /// Run a script (a JSON array of `{command, params}`), stopping at the first error.
    pub fn run_script(&mut self, script: &Value) -> Result<(), String> {
        let r = self.session.run_script(script).map(|_| ()).map_err(|e| e.to_string());
        self.model_changed();
        match &r {
            Ok(()) => self.set_status("Script done", false),
            Err(e) => self.set_status(format!("Script: {e}"), true),
        }
        r
    }

    /// What a click on a toolbar entry, a menu row or a palette result does: a command without
    /// parameters runs, one with parameters opens its form, and the file entries (open, save,
    /// write a deck, recover) go through the host's file pickers. `id` is a command or a
    /// catalogue entry; an entry that is coming soon does nothing and says when it comes.
    /// Programmatic calls use [`ModelApp::run`] or [`ModelApp::start_programmatic`].
    pub fn start(&mut self, id: &str) -> Result<(), String> {
        self.begin(id, true)
    }

    /// `ui.start` from the control channel: as a click, but never a native file picker (entries
    /// that need one open their command's form).
    pub fn start_programmatic(&mut self, id: &str) -> Result<(), String> {
        self.begin(id, false)
    }

    fn begin(&mut self, id: &str, interactive: bool) -> Result<(), String> {
        let (command, preset) = match catalogue::find(id) {
            Some(c) => match c.command() {
                Some(command) => (command, c.preset()),
                None => {
                    let e = format!("{} is coming soon ({})", c.label, c.milestone().unwrap_or("later"));
                    self.set_status(e.clone(), false);
                    return Err(e);
                }
            },
            None => (id, json!({})),
        };
        let spec = modelcraft_engine::find_command(command).ok_or_else(|| format!("unknown command `{command}`"))?;
        if self.file_action(spec.id, &preset, interactive) {
            self.dialog = None;
            return Ok(());
        }
        if spec.params.trim() == "{}" {
            self.dialog = None;
            self.run(spec.id, json!({})).map(|_| ())
        } else {
            // The form starts with the entry's preset; the rest (a path…) is filled in there.
            self.open_form(spec.id, preset);
            Ok(())
        }
    }

    /// Open a command's parameter form with `params` filled in.
    pub fn open_form(&mut self, command: &str, params: Value) {
        if let Some(spec) = modelcraft_engine::find_command(command) {
            self.dialog = Some(ParamsDialog { command: spec.id, text: params.to_string(), error: None });
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

    /// Probe a node of the results shown (`solve.probe`) and keep its values for the viewport.
    pub fn probe_node(&mut self, node: u32) -> Result<(), String> {
        let Some(view) = self.session.results_view.clone() else { return Err("no results are shown".into()) };
        let pick = self.plot().map_or(view.pick(), |p| p.pick);
        let mut params = json!({"step": view.step, "nodes": [node], "mode": pick.mode});
        if let Some(frame) = pick.frame {
            params["frame"] = json!(frame);
        }
        let reply = match self.run("solve.probe", params) {
            Ok(v) => v,
            Err(e) => {
                self.probe = None;
                return Err(e);
            }
        };
        let at = reply.pointer("/nodes/0").cloned().unwrap_or(Value::Null);
        let vector = |key: &str| at.get(key).and_then(|v| serde_json::from_value::<[f64; 3]>(v.clone()).ok());
        let triple = |v: [f64; 3]| format!("{}, {}, {}", results::number(v[0]), results::number(v[1]), results::number(v[2]));
        let mut lines = vec![format!("Node {node}")];
        if let Some(u) = vector("displacement") {
            lines.push(format!("U {}  |U| {}", triple(u), results::number(modelcraft_engine::results::magnitude(u))));
        }
        if let Some(rf) = vector("reaction").filter(|r| r.iter().any(|c| *c != 0.0)) {
            lines.push(format!("RF {}", triple(rf)));
        }
        if let Some(m) = at.get("von_mises").and_then(Value::as_f64) {
            lines.push(format!("Von Mises {}", results::number(m)));
        }
        if let Some(v) = vector("velocity") {
            lines.push(format!("V {}  |V| {}", triple(v), results::number(modelcraft_engine::results::magnitude(v))));
        }
        if let Some(shape) = vector("shape") {
            lines.push(format!("Mode {} shape {}", pick.mode, triple(shape)));
        }
        let contour = self.plot().and_then(|p| p.contour.as_ref().map(|c| (c.field, c.at(p.result.nodes(), node))));
        if let Some((field, v)) = contour.filter(|(f, _)| *f != modelcraft_engine::results::Field::VonMises) {
            lines.push(format!("{} {}", field.label(), results::number(v)));
        }
        self.set_status(lines.join("   "), false);
        self.probe = Some(Probe { node, step: view.step, mode: pick.mode, frame: pick.frame, lines });
        Ok(())
    }

    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error {
            log::warn!("{text}");
        }
        self.status = Some(Status { text, error });
    }

    /// Move an error the graphics device reported into the status bar.
    fn show_gpu_errors(&mut self) {
        let error = self.viewport.gpu_errors.lock().ok().and_then(|mut e| e.take());
        if let Some(e) = error {
            self.status = Some(Status { text: format!("Graphics: {e}"), error: true });
        }
    }

    /// The model may have changed: display data and the scene are rebuilt when next drawn.
    pub fn model_changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// The model's display data with its geometry (the open design's bodies, tessellated once
    /// per design, and the imported surfaces), rebuilt when the model changed. The first content
    /// to appear is framed.
    pub fn display(&mut self) -> &DisplayMesh {
        if self.display.0 != Some(self.revision) {
            let mut d = DisplayMesh::from_model(&self.session.model);
            let design = self.session.design.clone();
            if !match (&design, &self.bodies.0) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            } {
                let mut failed = Vec::new();
                let meshes: Vec<_> = design
                    .iter()
                    .flat_map(|d| &d.bodies)
                    .filter_map(|body| {
                        let tolerance = body.measure().map(|m| display_tolerance(&Aabb3 { min: m.bbox_min.into(), max: m.bbox_max.into() }));
                        tolerance.and_then(|t| body.tessellate(t)).map_err(|e| failed.push(format!("{}: {e}", body.name))).ok()
                    })
                    .collect();
                self.bodies = (design, meshes);
                if !failed.is_empty() {
                    self.set_status(format!("Some bodies cannot be drawn: {}", failed.join("; ")), true);
                }
            }
            for mesh in self.bodies.1.iter().chain(self.session.surfaces.iter().map(|s| &s.mesh)) {
                d.add_surface(&mesh.positions, &mesh.triangles, &mesh.tri_face);
            }
            let was_empty = self.display.1.bounds.is_empty();
            self.display = (Some(self.revision), d);
            if was_empty && !self.display.1.bounds.is_empty() {
                self.cam.fit(&self.display.1.bounds);
            }
        }
        &self.display.1
    }

    /// The key of the current scene: the model revision, every setting the scene depends on and
    /// the results plot shown.
    pub fn scene_key(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        let plot = self.plot.1.as_ref().map(|p| Arc::as_ptr(p) as usize);
        (self.revision, self.ui.style, self.ui.show(), self.ui.dark, plot).hash(&mut h);
        h.finish()
    }

    /// What the results plot is built for: the model, the result, its view and the animation
    /// frame; `None` when no results are shown.
    fn plot_key(&self) -> Option<PlotKey> {
        let (view, result, current) = results::shown(&self.session)?;
        let frame = results::frame(view, self.now);
        let pick = results::pick(view, result, frame);
        Some(PlotKey { revision: self.revision, result: Arc::as_ptr(result) as usize, current, view: view.clone(), pick, frame })
    }

    /// The results plot the viewport draws, rebuilt when its key changes; `None` shows the
    /// model. Only the deformation follows the animation frame; the scale and contour values are
    /// kept while the rest of the key (the mode and field frame drawn among it) stays.
    pub fn plot(&mut self) -> Option<Arc<results::Plot>> {
        let key = self.plot_key();
        if self.plot.0 != key {
            self.display();
            let still = key.as_ref().map(|k| PlotKey { frame: None, ..k.clone() });
            if self.prepared.0 != still {
                let pick = key.as_ref().map(|k| k.pick).unwrap_or_default();
                self.prepared = (still, results::prepare(&self.session, &self.display.1, pick).unwrap_or_default());
            }
            let frame = key.as_ref().and_then(|k| k.frame);
            let p = key.as_ref().and_then(|_| results::plot(&self.session, &self.display.1, frame, &self.prepared.1));
            self.plot = (key, p.map(Arc::new));
        }
        self.plot.1.clone()
    }

    /// The scene the viewport draws, rebuilt when its key changes; its radius fits the camera.
    pub fn scene(&mut self) -> (u64, &Scene) {
        self.display();
        self.plot();
        let key = self.scene_key();
        if self.scene.0 != Some(key) {
            let t = theme::Tokens::get(self.ui.dark);
            let (style, show) = (self.ui.style, self.ui.show());
            let s = match &self.plot.1 {
                Some(p) => results::scene(p, style, show, &t),
                None => scene::build(&self.display.1, style, show, &t),
            };
            self.scene = (Some(key), s);
        }
        let mut bounds = self.display.1.bounds;
        if let Some(p) = &self.plot.1 {
            bounds.add(p.display.bounds.min);
            bounds.add(p.display.bounds.max);
        }
        self.scene.1.radius = scene::radius(&bounds, &self.cam);
        (key, &self.scene.1)
    }

    /// A CPU render of the viewport's scene with the current camera, as PNG bytes.
    pub fn render_png(&mut self, w: usize, h: usize) -> Result<Vec<u8>, modelcraft_render::RenderError> {
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
        if self.poll_loading() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.dropped_files(ctx);
        self.shortcuts(ctx);
        self.cam.fov = if self.ui.perspective { PERSPECTIVE_FOV } else { 0.0 };
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        }
        // Results coming up (a solve, a results command) bring the Results tab.
        let shown = self.session.results_view.is_some();
        if shown && !self.results_shown {
            self.ui.tab = Tab::Results;
        }
        self.results_shown = shown;
        // A tab set on its own (ui.set, the palette) brings its workspace.
        self.ui.workspace = Workspace::showing(self.ui.tab, self.ui.workspace);
        // A selected definition that is gone (undo, delete) is no longer selected.
        if self.selected.as_ref().is_some_and(|(kind, name)| browser::definition(&self.session.model, *kind, name).is_none()) {
            self.selected = None;
        }
        if self.session.results_view.as_ref().is_some_and(|v| v.animate) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(results::PERIOD / results::FRAMES as f64));
        }
        self.screenshots(ctx);
    }

    /// Undo (Ctrl/Cmd+Z), redo (Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y), open (Ctrl/Cmd+O), save
    /// (Ctrl/Cmd+S) and save as (Ctrl/Cmd+Shift+S), unless a text field has the keyboard (it
    /// undoes its own typing).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let shortcut = |modifiers, key| egui::KeyboardShortcut::new(modifiers, key);
        let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        // Shift first: the plain shortcut matches it too.
        let (redo, undo, save_as, save, open) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&shortcut(shift, egui::Key::Z)) || i.consume_shortcut(&shortcut(egui::Modifiers::COMMAND, egui::Key::Y));
            let undo = i.consume_shortcut(&shortcut(egui::Modifiers::COMMAND, egui::Key::Z));
            let save_as = i.consume_shortcut(&shortcut(shift, egui::Key::S));
            let save = i.consume_shortcut(&shortcut(egui::Modifiers::COMMAND, egui::Key::S));
            (redo, undo, save_as, save, i.consume_shortcut(&shortcut(egui::Modifiers::COMMAND, egui::Key::O)))
        });
        if redo {
            let _ = self.run("edit.redo", json!({}));
        } else if undo {
            let _ = self.run("edit.undo", json!({}));
        }
        if save_as {
            self.save_as();
        } else if save {
            self.save();
        }
        if open {
            self.open_dialog(None);
        }
        // The command search: S (as in SolidCraft) or Cmd/Ctrl+K.
        let search = ctx.input_mut(|i| {
            i.consume_shortcut(&shortcut(egui::Modifiers::COMMAND, egui::Key::K)) || i.consume_key(egui::Modifiers::NONE, egui::Key::S)
        });
        if search {
            self.ui.palette = true;
            self.ui.menu = None;
        }
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
        self.show_gpu_errors();
        self.ui.workspace = Workspace::showing(self.ui.tab, self.ui.workspace);
        self.layout.clear();
        appbar::app_bar(self, ui);
        toolbar::toolbar(self, ui);
        if self.ui.show_timeline {
            timeline::timeline(self, ui);
        }
        palette::command_line(self, ui);
        if self.ui.show_tree {
            browser::browser(self, ui);
        }
        context::panel(self, ui);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| viewport::show(self, ui));
        palette::popup(self, ui.ctx());
        files::show(self, ui.ctx());
        if self.custom_titlebar {
            titlebar::resize_zones(ui);
        }
    }

    /// Remember where a region of the window was drawn (`ui.inspect`'s `layout`).
    pub fn place(&mut self, region: &'static str, rect: egui::Rect) {
        self.layout.insert(region, rect);
    }

    /// Open, or close when it is open, a menu (see [`UiState::menu`]).
    pub fn toggle_menu(&mut self, key: &str) {
        self.ui.menu = if self.ui.menu.as_deref() == Some(key) { None } else { Some(key.to_string()) };
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
        // No frame was presented in time (a hidden window, a locked screen): draw the window
        // offscreen instead.
        let (late, waiting): (Vec<Shot>, Vec<Shot>) = std::mem::take(&mut self.shots).into_iter().partition(|s| now > s.deadline);
        self.shots = waiting;
        for s in late {
            let _ = s.reply.send(control::capture_to(self, control::window_size(ctx), s.path.as_deref()));
        }
        self.shots.retain_mut(|s| {
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

/// The chordal tolerance bodies are drawn with: a thousandth of their size, within 1 µm–1 mm.
fn display_tolerance(b: &Aabb3) -> f64 {
    let d = b.diagonal();
    if d.is_finite() { (d * 1e-3).clamp(1e-3, 1.0) } else { 0.1 }
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
    use modelcraft_render::Vec3;

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
    fn toolbar_entries_start_their_command_and_planned_ones_do_nothing() {
        let mut app = ModelApp::new(Session::default());
        app.start("model.material.create").unwrap();
        assert_eq!(app.dialog.as_ref().map(|d| d.command), Some("model.material.create"));
        app.dialog = None;
        let journal = app.session.journal.len();
        let err = app.start("mesh.order.quadratic").unwrap_err();
        assert_eq!(err, "Quadratic is coming soon (M2)");
        assert!(app.dialog.is_none());
        assert_eq!(app.session.journal.len(), journal);
        assert_eq!(app.status.as_ref().map(|s| s.error), Some(false));
        // A deck entry opens deck.write's form with its format filled in.
        app.start("deck.write.abaqus").unwrap();
        let d = app.dialog.as_ref().unwrap();
        assert_eq!((d.command, d.text.as_str()), ("deck.write", r#"{"format":"abaqus"}"#));
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

    #[test]
    fn the_open_design_and_imported_surfaces_are_drawn() {
        let dir = std::env::temp_dir().join(format!("modelcraft-ui-design-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let design = dir.join("plate.solidcraft");
        let bytes = modelcraft_geometry::sample::plate_design(&modelcraft_geometry::sample::Plate::default()).unwrap();
        std::fs::write(&design, bytes).unwrap();
        let mut app = ModelApp::new(Session::default());
        app.run("geometry.open", json!({"path": design.to_string_lossy()})).unwrap();
        let d = app.display().clone();
        assert!(!d.surface_tris.is_empty() && !d.surface_edges.is_empty(), "the design's bodies are drawn");
        assert!(d.tris.is_empty());
        // Without elements, geometry draws as faces with their boundaries; the camera frames it.
        let t = theme::Tokens::dark();
        let s = scene::build(&d, Style::ShadedEdges, scene::Show::DEFAULT, &t);
        assert_eq!(s.tris.len(), d.surface_tris.len());
        assert!(s.tris.iter().all(|tri| tri.color == theme::rgb(t.geometry)));
        assert!(!d.bounds.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The sample cantilever in an app, solved (static step `Load`, frequency step `Modes`).
    fn solved_app() -> ModelApp {
        ModelApp::new(results::tests::solved_cantilever())
    }

    fn node_at(app: &ModelApp, xyz: [f64; 3]) -> u32 {
        results::tests::node_at(&app.session, xyz)
    }

    #[test]
    fn solving_shows_the_result_on_the_results_tab() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&modelcraft_engine::sample::cantilever()).unwrap();
        app.start("solve.static").unwrap();
        app.run_dialog().unwrap();
        let view = app.session.results_view.clone().unwrap();
        assert_eq!((view.step.as_str(), view.contour, view.deformed), ("Load", Some(modelcraft_engine::results::Field::VonMises), true));
        app.logic(&egui::Context::default());
        assert_eq!(app.ui.tab, Tab::Results);
        app.run("solve.frequency", json!({})).unwrap();
        app.run("results.deformed", json!({"step": "Modes"})).unwrap();
        let view = app.session.results_view.clone().unwrap();
        assert_eq!((view.step.as_str(), view.mode, view.contour), ("Modes", 1, Some(modelcraft_engine::results::Field::Displacement)));
        // The plot draws contour bands: more triangles than the model's faces.
        let faces = app.display().tris.len();
        assert!(app.scene().1.tris.len() > faces);
        app.run("results.clear", json!({})).unwrap();
        assert!(app.plot().is_none());
        assert_eq!(app.scene().1.tris.len(), faces);
    }

    /// Rendered on the CPU from the front, undeformed, with Z displacement contours: the free end
    /// (lowest band) is blue, the clamped end (highest band) red.
    #[test]
    fn contours_render_with_known_colours_at_known_nodes() {
        let mut app = solved_app();
        app.run("results.contour", json!({"step": "Load", "field": "displacement_z", "bands": 6})).unwrap();
        app.run("results.deformed", json!({"show": false})).unwrap();
        app.ui.style = Style::Shaded;
        app.set_view(StandardView::Front);
        app.fit_view();
        let (w, h) = (400, 200);
        let cam = app.cam;
        let radius = app.scene().1.radius;
        let canvas = modelcraft_render::render(app.scene().1, &cam, w, h).unwrap();
        let pixel = |p: Vec3| {
            let (x, y, _) = cam.to_screen(p, w as f64, h as f64, radius).unwrap();
            let i = (y as usize * w + x as usize) * 4;
            [canvas.rgba[i], canvas.rgba[i + 1], canvas.rgba[i + 2]]
        };
        // Cell centres on the front face (y = 0), one at each end.
        let tip = pixel(Vec3::new(97.5, 0.0, 12.5));
        let root = pixel(Vec3::new(2.5, 0.0, 12.5));
        assert!(tip[2] > tip[0] + 60, "the free end is blue: {tip:?}");
        assert!(root[0] > root[2] + 60, "the clamped end is red: {root:?}");
    }

    #[test]
    fn probing_a_node_reads_its_values_and_stale_results_say_so() {
        let mut app = solved_app();
        app.run("results.deformed", json!({"step": "Load"})).unwrap();
        let tip = node_at(&app, [100.0, 0.0, 0.0]);
        app.probe_node(tip).unwrap();
        let probe = app.probe.clone().unwrap();
        assert_eq!((probe.node, probe.step.as_str()), (tip, "Load"));
        assert!(probe.lines.iter().any(|l| l.starts_with("U ")) && probe.lines.iter().any(|l| l.starts_with("Von Mises")), "{probe:?}");
        // A root node carries a reaction.
        app.probe_node(node_at(&app, [0.0, 0.0, 0.0])).unwrap();
        assert!(app.probe.as_ref().unwrap().lines.iter().any(|l| l.starts_with("RF ")));
        // The plot under a click: a ray onto the tip's end face picks a node there.
        let plot = app.plot().unwrap();
        let (node, _) = plot.display.pick(Vec3::new(1000.0, 0.5, 0.5), Vec3::new(-1.0, 0.0, 0.0)).unwrap();
        assert_eq!(app.session.model.node(node).unwrap().xyz[0], 100.0);
        // An edit leaves the plot shown but stale; probing it is refused.
        app.run(
            "model.material.edit",
            json!({"name": "Steel", "elastic": {"type": "isotropic", "young": 70000, "poisson": 0.33}, "density": 2.7e-9}),
        )
        .unwrap();
        assert!(app.plot().is_some_and(|p| p.stale));
        let e = app.probe_node(tip).unwrap_err();
        assert!(e.contains("stale") && app.probe.is_none(), "{e}");
    }

    #[test]
    fn explicit_frames_are_drawn_and_animated() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&modelcraft_engine::sample::impact()).unwrap();
        app.run("solve.explicit", json!({})).unwrap();
        // The last frame at true scale: the bar has moved back off the wall's side (x < 0 is
        // the wall, so nothing passes it) and carries stress contours.
        let last = app.plot().unwrap();
        assert_eq!(last.frame().map(|(k, _, n)| (k, n)), Some((10, 11)));
        assert!(last.contour.as_ref().is_some_and(|c| c.field == modelcraft_engine::results::Field::VonMises));
        assert!(last.display.bounds.min.x > -1e-6, "{:?}", last.display.bounds);
        // Frame 0 is the undeformed bar at rest on its initial velocity.
        app.run("results.frame", json!({"frame": 0})).unwrap();
        let first = app.plot().unwrap();
        assert_eq!(first.display.tris, app.display().tris);
        // Animated, the frames run through once per cycle.
        app.run("results.animate", json!({})).unwrap();
        app.now = results::PERIOD * 0.5;
        assert_eq!(app.plot().unwrap().pick.frame, Some(5));
        let image = app.capture(capture::CaptureSize { width: 1200.0, height: 800.0, scale: 1.0 }).unwrap();
        assert_eq!(image.size, [1200, 800]);
        // Probing reads the frame drawn.
        app.run("results.animate", json!({"on": false})).unwrap();
        app.run("results.frame", json!({"frame": 4})).unwrap();
        app.probe_node(node_at(&app, [100.0, 0.0, 0.0])).unwrap();
        let probe = app.probe.clone().unwrap();
        assert_eq!(probe.frame, Some(4));
        assert!(probe.lines.iter().any(|l| l.starts_with("U ")), "{probe:?}");
    }

    #[test]
    fn the_capture_shows_the_legend_and_panel_over_the_plot() {
        let mut app = solved_app();
        app.run("results.deformed", json!({"step": "Load"})).unwrap();
        let image = app.capture(capture::CaptureSize { width: 1200.0, height: 800.0, scale: 1.0 }).unwrap();
        let t = theme::Tokens::dark();
        // The legend's top swatch (the highest band, red) is drawn left of the middle.
        let red = t.contour[4];
        let near = |c: egui::Color32, d: egui::Color32| {
            (i32::from(c.r()) - i32::from(d.r())).abs() < 30
                && (i32::from(c.g()) - i32::from(d.g())).abs() < 40
                && (i32::from(c.b()) - i32::from(d.b())).abs() < 30
        };
        let found = (0..600).any(|x| (150..400).any(|y| near(image.pixels[y * 1200 + x], red)));
        assert!(found, "a red legend swatch");
    }

    fn region(app: &ModelApp, name: &str) -> egui::Rect {
        app.layout.get(name).copied().unwrap_or_else(|| panic!("no {name} in {:?}", app.layout.keys()))
    }

    /// The family layout: title bar, ribbon, browser left, viewport, command line and timeline at
    /// the bottom; the context panel on the right only while there is context.
    #[test]
    fn the_window_is_laid_out_like_the_other_crafts() {
        let mut app = ModelApp::new(Session::default());
        app.open_sample("cantilever");
        let size = capture::CaptureSize { width: 1400.0, height: 900.0, scale: 1.0 };
        // `layout` is the live window's; a capture leaves it, so draw a frame of the UI directly.
        let frame = |app: &mut ModelApp| {
            let ctx = egui::Context::default();
            for _ in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.width, size.height))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
                out.textures_delta.clear();
            }
        };
        frame(&mut app);
        let (bar, ribbon, browser, view) = (region(&app, "app_bar"), region(&app, "ribbon"), region(&app, "browser"), region(&app, "viewport"));
        let (line, timeline) = (region(&app, "command_line"), region(&app, "timeline"));
        assert_eq!((bar.top(), bar.height()), (0.0, appbar::HEIGHT));
        assert!((ribbon.top() - bar.bottom()).abs() < 1.0 && (ribbon.height() - toolbar::HEIGHT).abs() < 1.0, "{ribbon:?}");
        // The browser's content, inside its margin.
        assert!(browser.left() < 8.0 && (browser.width() - browser::WIDTH).abs() <= 14.0, "{browser:?}");
        assert!(view.left() >= browser.right() - 1.0 && view.top() >= ribbon.bottom() - 1.0 && view.bottom() <= line.top() + 1.0);
        assert!((timeline.bottom() - 900.0).abs() < 1.0 && line.bottom() <= timeline.top() + 1.0);
        assert!((view.right() - 1400.0).abs() < 1.0, "no context: the viewport reaches the right edge");
        assert!(!app.layout.contains_key("context"));
        for chrome in ["nav_bar", "axis_view", "workspace_switcher", "document_tab", "app_bar:solve"] {
            let r = region(&app, chrome);
            assert!(r.is_positive(), "{chrome}");
        }
        assert!(view.contains_rect(region(&app, "nav_bar")) && view.contains_rect(region(&app, "axis_view")));
        // A form brings the context panel; the viewport makes room.
        app.start("model.material.create").unwrap();
        frame(&mut app);
        let context = region(&app, "context");
        assert!(context.right() > 1390.0 && context.width() > 280.0, "{context:?}");
        assert!(region(&app, "viewport").right() <= context.left() + 1.0);
        // The search and a drop-down are drawn when their state says so.
        app.dialog = None;
        app.ui.palette = true;
        app.ui.menu = Some("mesh/Tet".into());
        app.ui.tab = Tab::Mesh;
        frame(&mut app);
        assert!(app.layout.contains_key("palette") && app.layout.contains_key("toolbar_dropdown"), "{:?}", app.layout.keys());
        assert_eq!(app.ui.workspace, Workspace::Prepare);
    }

    /// No entry is lost: on every tab, at a wide and a narrow window, every catalogue entry is a
    /// button, a collapsed panel or a » panel, and a row of its panel's drop-down once it is open.
    #[test]
    fn every_catalogue_entry_is_reachable_from_its_tab() {
        for width in [1400.0, 960.0] {
            for tab in Tab::ALL {
                let mut app = ModelApp::new(Session::default());
                app.ui.tab = tab;
                let ctx = egui::Context::default();
                let run = |app: &mut ModelApp| {
                    let raw = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.0))),
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
                    out.textures_delta.clear();
                };
                run(&mut app);
                for c in catalogue::tab_entries(tab) {
                    assert!(app.toolbar_rects.iter().any(|(id, ..)| *id == c.id), "{width}: {} is not on the {tab:?} tab", c.id);
                }
                for p in catalogue::panels(tab) {
                    let Some(panel) = p.first().map(|c| c.panel) else { continue };
                    let collapsed_or_more = app.toolbar_rects.iter().any(|(id, _, f)| *id == p[0].id && matches!(f, toolbar::Fit::More));
                    if collapsed_or_more {
                        continue;
                    }
                    app.ui.menu = Some(toolbar::panel_key(tab, panel));
                    run(&mut app);
                    run(&mut app);
                    for c in p {
                        let row = app.toolbar_rects.iter().any(|(id, r, f)| *id == c.id && *f == toolbar::Fit::Menu && r.is_positive());
                        assert!(row, "{width}: {} has no row in the {panel} drop-down", c.id);
                    }
                    app.ui.menu = None;
                }
            }
        }
    }

    #[test]
    fn display_tolerance_follows_the_size() {
        let b = |max: [f64; 3]| Aabb3 { min: Vec3::ZERO, max: max.into() };
        assert_eq!(display_tolerance(&b([100.0, 0.0, 0.0])), 0.1);
        assert_eq!(display_tolerance(&b([0.0; 3])), 1e-3);
        assert_eq!(display_tolerance(&b([1e9, 0.0, 0.0])), 1.0);
        assert_eq!(display_tolerance(&Aabb3::EMPTY), 1e-3);
    }

    #[test]
    fn graphics_device_errors_reach_the_status_bar() {
        let mut app = ModelApp::new(Session::default());
        if let Ok(mut e) = app.viewport.gpu_errors.lock() {
            *e = Some("Out of Memory".into());
        }
        app.show_gpu_errors();
        assert_eq!(app.status, Some(Status { text: "Graphics: Out of Memory".into(), error: true }));
        assert!(app.viewport.gpu_errors.lock().unwrap().is_none(), "each error is shown once");
    }
}
