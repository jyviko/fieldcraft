//! The FieldCraft egui front end.
//!
//! A thin shell over [`fieldcraft_engine::Session`]: panels read engine state and act through
//! commands ([`ModelApp::run`]); nothing here owns model data. The window follows the Craft
//! family's layout (SolidCraft, VectorCraft):
//!
//! - the title bar ([`appbar`]): the app, quick actions (File menu, save, undo, redo, Solve,
//!   search, theme, help) and the model's document tab;
//! - the ribbon ([`toolbar`]): the workspace switcher, its tabs, and panels of icon buttons whose
//!   labelled drop-downs list the whole catalogue (coming-soon entries greyed with their
//!   milestone);
//! - the browser on the left ([`browser`]): the model's geometry, mesh and definitions;
//! - the viewport ([`viewport`]) with its navigation bar, orientation gizmo and selection bar
//!   (picking faces, edges, vertices, bodies, nodes and elements: [`selection`]);
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
pub mod elide;
pub mod fields;
pub mod files;
pub mod form;
pub mod frame;
pub mod framing;
pub mod gpu;
pub mod icons;
pub mod navigation;
pub mod palette;
pub mod reply;
pub mod results;
pub mod results_panel;
pub mod scene;
pub mod selection;
pub mod theme;
pub mod timeline;
pub mod titlebar;
pub mod toolbar;
pub mod viewport;

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use fieldcraft_engine::catalogue::{self, Tab, Workspace};
use fieldcraft_engine::{EngineError, Session};
use fieldcraft_render::glyphs::Glyphs;
use fieldcraft_render::{Aabb3, Camera, DisplayMesh, Scene, StandardView, Vec3};
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
    /// Boundary condition and load symbols.
    pub show_loads: bool,
    /// Reference point, coupling and rigid wall symbols.
    pub show_interactions: bool,
    pub perspective: bool,
    pub dark: bool,
    /// The browser (left).
    pub show_tree: bool,
    /// The journal timeline (bottom).
    pub show_timeline: bool,
    /// The mouse navigation preset ([`navigation::NAV_PRESETS`] id; remembered per user).
    pub nav_preset: String,
    /// The wheel zooms the other way (remembered per user).
    pub reverse_zoom: bool,
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
            show_loads: true,
            show_interactions: true,
            perspective: false,
            dark: true,
            show_tree: true,
            show_timeline: true,
            nav_preset: navigation::DEFAULT_PRESET.into(),
            reverse_zoom: false,
            workspace: Workspace::default(),
            tab: Tab::default(),
            palette: false,
            menu: None,
        }
    }
}

impl UiState {
    /// Show `tab`, on the current workspace when it has it, else on the first that does.
    pub fn select_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.workspace = Workspace::showing(tab, self.workspace);
    }

    /// Show workspace `w` on its first tab.
    pub fn select_workspace(&mut self, w: Workspace) {
        self.workspace = w;
        self.tab = w.tabs().first().copied().unwrap_or(Tab::Tools);
    }

    /// What the viewport shows.
    pub fn show(&self) -> scene::Show {
        scene::Show {
            geometry: self.show_geometry,
            mesh: self.show_mesh,
            nodes: self.show_nodes,
            loads: self.show_loads,
            interactions: self.show_interactions,
        }
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
    /// The command refused one parameter: its path and the problem phrased for the form, shown
    /// beside its field.
    pub refused: Option<(String, String)>,
    /// The paths of the fields edited: their problems show; the others wait for OK.
    pub touched: std::collections::BTreeSet<String>,
    /// OK was tried with something missing: every problem shows.
    pub attempted: bool,
    /// The JSON text is shown instead of the typed fields.
    pub advanced: bool,
    /// Which opening of a form this is (counts up): a form opened over another one, even of the
    /// same command, gets fresh widget ids, so nothing typed into the one it replaces carries over.
    pub opening: u64,
    /// Opened from another form's "Create …" button: the form to go back to.
    pub back: Option<Box<Back>>,
}

/// A command that failed: its message, and the parameter it refused (path, form message).
struct Failure {
    message: String,
    refused: Option<(String, String)>,
}

/// The form a "Create …" button left, and where the item made goes in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Back {
    pub dialog: ParamsDialog,
    pub collection: fieldcraft_engine::schema::Collection,
    /// The reference field's path in its parameters (`a/b`).
    pub path: String,
    /// The collection's names before: the one added is the new item.
    pub before: Vec<String>,
}

/// A status bar message.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

/// Selection by picking in the viewport ([`selection`]): the filter, what is selected and what
/// is under the pointer, and what they are drawn with.
#[derive(Default)]
pub struct PickState {
    pub filter: selection::Filter,
    pub selection: selection::Selection,
    /// What a click would pick under the pointer.
    pub hover: Option<selection::Item>,
    /// A left drag draws a selection box (nodes, element faces, elements) instead of orbiting.
    pub box_mode: bool,
    /// The box being dragged (window points).
    pub dragging: Option<(egui::Pos2, egui::Pos2)>,
    /// Where the pointer was when the hover was worked out (window points).
    pub hover_at: Option<egui::Pos2>,
    /// Bumped whenever the selection changes, and whenever the hover does.
    generation: u64,
    hover_generation: u64,
    /// The geometry tags behind the drawn mesh and the model revision they were read at.
    index: (Option<u64>, Arc<selection::Index>),
    /// The names of the drawn geometry surfaces, in the order they were added to the display.
    names: Vec<selection::BodyNames>,
    /// The selection's highlight and what it was built for (kept while only the hover changes).
    selected_overlay: (Option<u64>, Arc<Scene>),
    /// The highlight overlay (the selection's and the hover's) and what it was built for.
    overlay: (Option<u64>, Arc<Scene>),
    /// What the hover was last worked out for: the pointer, the camera, the scene, the filter
    /// and the viewport.
    pub(crate) hover_key: Option<(Option<egui::Pos2>, Camera, u64, selection::Filter, Option<egui::Rect>)>,
    /// The model revision the selection's mesh labels were last checked at.
    checked: Option<u64>,
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
    /// The selection highlight for the GPU, and the overlay key last handed to it.
    pub overlay_slot: gpu::SceneSlot,
    pub overlay_uploaded: Option<u64>,
    /// CPU fallback: the texture and what it shows (scene and overlay keys, camera, size).
    cpu: Option<(egui::TextureHandle, (u64, u64), Camera, [usize; 2])>,
    /// CPU fallback: the scene key the renderer last refused.
    cpu_failed: Option<u64>,
    /// GPU errors reported by the device (see [`gpu::GpuErrors`]), shown in the status bar.
    pub gpu_errors: gpu::GpuErrors,
    /// The overlays drawn over the viewport last frame (legend, selection bar, gizmo, navigation
    /// bar): a fit leaves them free ([`framing::free_part`]).
    pub overlays: Vec<egui::Rect>,
    /// What the current drag does to the camera, chosen when it starts.
    pub drag: Option<NavMode>,
    /// The point the current orbit turns about (drawn as a marker while orbiting).
    pub orbit_pivot: Option<Vec3>,
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
    view: fieldcraft_engine::results::ResultsView,
    /// The mode and field frame drawn (an animated explicit step changes frames).
    pick: fieldcraft_engine::results::Pick,
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
    /// Forms opened so far (numbers each [`ParamsDialog::opening`]).
    forms_opened: u64,
    pub viewport: Viewport,
    pub icons: icons::IconCache,
    /// The toolbar's buttons drawn last frame, by catalogue id (for the control channel).
    pub toolbar_rects: Vec<(&'static str, egui::Rect, toolbar::Fit)>,
    /// The GPU adapter, for the status bar and diagnostics.
    pub gpu_info: Option<String>,
    /// Bumped whenever the model may have changed.
    revision: u64,
    display: (Option<u64>, DisplayMesh),
    /// The analysis setup's symbols, built with the display data.
    glyphs: Glyphs,
    /// The results plot and what it was built for.
    plot: (Option<PlotKey>, Option<Arc<results::Plot>>),
    /// The node probed last (a click on a results plot).
    pub probe: Option<Probe>,
    /// The reaction sums asked for last in the results panel.
    pub reactions: Option<Value>,
    /// The last command reply with more to it than its status line, shown in the context panel.
    pub reply: Option<reply::Reply>,
    /// Whether results were shown last frame (the Results tab comes up when they appear).
    results_shown: bool,
    /// The framing the viewport does when it next draws ([`framing::Reframe`]).
    reframe: Option<framing::Reframe>,
    /// The open document ([`Session::opened`]) the view was framed for.
    framed_document: Option<u64>,
    /// Tessellated design bodies with their names, kept while the same design is open.
    bodies: (Option<Arc<fieldcraft_geometry::Design>>, Vec<(selection::BodyNames, fieldcraft_geometry::SurfaceMesh)>),
    /// The frame-independent part of the results plot (scale, contour) and what it was built for.
    prepared: (Option<PlotKey>, results::Prepared),
    /// The built scene, its key and the box around its symbols (they reach beyond the model).
    scene: (Option<u64>, Scene, Aabb3),
    /// The theme last applied to the context.
    styled: Option<bool>,
    /// Input injected by the control channel, fed to egui one pointer event per frame.
    pub synthetic: Vec<egui::Event>,
    synthetic_mods: egui::Modifiers,
    /// Files dropped on the window by the control channel (`ui.drop`): the next frame takes them
    /// with the window system's drops.
    pub synthetic_drops: Vec<String>,
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
    /// The user's preferences as last saved.
    prefs: files::Prefs,
    /// The user's preferences as last saved.
    /// The setup progress the browser marks, and what it was worked out for.
    setup: (Option<(u64, u64, bool, bool)>, std::sync::Arc<Vec<fieldcraft_engine::progress::StageState>>),
    /// The file opening in the background, and files waiting to open after it.
    pub loading: Option<files::Loading>,
    pub queued: std::collections::VecDeque<String>,
    /// The question the window is asking (unsaved changes, recover).
    pub prompt: Option<files::Prompt>,
    /// A design opened over a model of another part, waiting for [`files::Prompt::Design`]'s answer.
    pub held: Option<files::Opened>,
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
    /// What is picked in the viewport.
    pub pick: PickState,
    /// Set while [`ModelApp::capture`] draws the window offscreen: no pointer there is the
    /// user's, so the hover stays as it is.
    capturing: bool,
    /// What a left drag in the viewport does.
    pub nav: NavMode,
    /// Where the window's regions were drawn last frame (`ui.inspect`'s `layout`).
    pub layout: std::collections::BTreeMap<&'static str, egui::Rect>,
    /// The toolbar's panel labels drawn last frame, by `<tab>/<panel>`.
    pub panel_rects: Vec<(String, egui::Rect)>,
    /// The timeline's items and the journal state they were built for.
    timeline: Option<((u64, usize, (usize, usize)), std::sync::Arc<Vec<timeline::Item>>)>,
    /// The browser's element counts and the model revision they were counted at.
    element_counts: Option<(u64, Vec<(fieldcraft_model::ElementKind, usize)>)>,
    /// The selected definition's property rows and what they were read for.
    properties: Option<((u64, browser::Def, String), std::sync::Arc<Vec<(String, String)>>)>,
    /// The search's query and its results.
    search: Option<(String, std::sync::Arc<Vec<palette::Hit>>)>,
}

impl ModelApp {
    pub fn new(session: Session) -> ModelApp {
        ModelApp {
            session,
            ui: UiState::default(),
            cam: Camera::default(),
            status: None,
            dialog: None,
            forms_opened: 0,
            viewport: Viewport::default(),
            icons: icons::IconCache::default(),
            toolbar_rects: Vec::new(),
            gpu_info: None,
            revision: 0,
            display: (None, DisplayMesh::default()),
            glyphs: Glyphs::default(),
            plot: (None, None),
            probe: None,
            reactions: None,
            reply: None,
            results_shown: false,
            reframe: None,
            framed_document: None,
            bodies: (None, Vec::new()),
            prepared: (None, results::Prepared::default()),
            scene: (None, Scene::default(), Aabb3::EMPTY),
            styled: None,
            synthetic: Vec::new(),
            synthetic_mods: egui::Modifiers::default(),
            synthetic_drops: Vec::new(),
            control_rx: None,
            shots: Vec::new(),
            shot_token: 0,
            offscreen: None,
            now: 0.0,
            quit_requested: false,
            services: files::Services::default(),
            recent: files::Recent::default(),
            prefs: files::Prefs::default(),
            setup: (None, std::sync::Arc::default()),
            loading: None,
            queued: std::collections::VecDeque::new(),
            prompt: None,
            held: None,
            custom_titlebar: false,
            integrated_titlebar: false,
            palette_text: String::new(),
            command_line: String::new(),
            tree_open: ["mesh", "results"].into_iter().map(str::to_string).collect(),
            selected: None,
            pick: PickState::default(),
            capturing: false,
            nav: NavMode::default(),
            layout: std::collections::BTreeMap::new(),
            panel_rects: Vec::new(),
            timeline: None,
            element_counts: None,
            search: None,
            properties: None,
        }
    }

    /// Serve control-channel requests (see [`control`]).
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Use the host's services (file pickers, the recent-files file) and load the recent list.
    pub fn with_services(mut self, services: files::Services) -> Self {
        self.recent = files::Recent::load(services.recent_path.clone());
        self.prefs = files::Prefs::load(services.prefs_path.as_deref());
        self.ui.nav_preset = self.prefs.nav_preset.clone();
        self.ui.reverse_zoom = self.prefs.reverse_zoom;
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
        self.run_form(id, params, false).map_err(|f| f.message)
    }

    /// [`Self::run`], from a form when `form`: a refused parameter is then named by its field's
    /// label (the engine's message names its key, which agents need).
    fn run_form(&mut self, id: &str, params: Value, form: bool) -> Result<Value, Failure> {
        self.ensure_idle().map_err(|message| Failure { message, refused: None })?;
        let revision = self.session.revision();
        let label = command_label(id);
        let r = self.session.execute(id, &params).map_err(|e| match e {
            EngineError::Param(p) => {
                let message = if form { p.for_form() } else { p.for_agents() };
                let shown = p.for_form();
                Failure { refused: Some((p.path, shown)), message }
            }
            e => Failure { message: e.to_string(), refused: None },
        });
        let changed = self.session.revision() != revision;
        if changed {
            self.model_changed();
        }
        // The results display commands reply with what is shown, which the viewport shows.
        let display = (id.starts_with("results.") && id != "results.reactions") || id == "solve.run";
        if r.is_ok() && (changed || display) {
            self.follow_stage(id);
        }
        match &r {
            Ok(v) if v.is_null() || changed || display => self.set_status(format!("{label} done"), false),
            // A reply reads as one line; the whole of a rich one shows in the context panel (the
            // reaction sums have their own place in the results panel).
            Ok(v) => {
                let mut line = format!("{label}: {}", reply::summary(id, v));
                if reply::is_rich(v) && !(id == "results.reactions" && self.plot().is_some()) {
                    line.push_str(" (details in the panel)");
                    self.reply = Some(reply::Reply { command: id.to_string(), label: label.to_string(), params: params.clone(), value: v.clone() });
                }
                self.set_status(line, false)
            }
            Err(e) => self.set_status(format!("{label}: {}", e.message), true),
        }
        r
    }

    /// Run a script (a JSON array of `{command, params}`), stopping at the first error. The
    /// ribbon follows its last command that has a stage.
    pub fn run_script(&mut self, script: &Value) -> Result<(), String> {
        let r = self.session.run_script(script).map(|_| ()).map_err(|e| e.to_string());
        self.model_changed();
        if r.is_ok() {
            let commands = script.as_array().into_iter().flatten().filter_map(|step| step.get("command").and_then(Value::as_str));
            if let Some(id) = commands.rev().find(|id| catalogue::stage_tab(id).is_some()) {
                self.follow_stage(id);
            }
        }
        match &r {
            Ok(()) => self.set_status("Script done", false),
            Err(e) => self.set_status(format!("Script: {e}"), true),
        }
        r
    }

    /// The ribbon turns to the tab where `id` lives ([`catalogue::stage_tab`]), as a person's click
    /// there would, so commands run by scripts, the control channel and MCP show their stage.
    /// Called after a command changed the model or the results display; queries leave the ribbon
    /// alone, and a ribbon click never moves it.
    fn follow_stage(&mut self, id: &str) {
        if let Some(tab) = catalogue::stage_tab(id) {
            self.ui.select_tab(tab);
        }
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
        let spec = fieldcraft_engine::find_command(command).ok_or_else(|| format!("unknown command `{command}`"))?;
        // A command missing what it needs opens no form: the status names what is missing.
        if let Err(u) = self.session.availability(spec) {
            let e = u.to_string();
            self.set_status(e.clone(), true);
            return Err(e);
        }
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
        if let Some(spec) = fieldcraft_engine::find_command(command) {
            // The schema's defaults are filled in, so the form starts as the command would run.
            let mut params = spec.form.map_or(params.clone(), |form| fieldcraft_engine::schema::prefill(form, &params));
            // A required reference with one obvious candidate starts picked.
            if let (Some(form), Some(obj)) = (spec.form, params.as_object_mut()) {
                let selected = self.selected.clone();
                let session = &self.session;
                let pick = |c| match &selected {
                    Some((kind, name)) if kind.collection() == c => Some(name.clone()),
                    _ => Some(session.names(c)).filter(|n| n.len() == 1).and_then(|n| n.into_iter().next()),
                };
                form::preselect(form, obj, &pick);
            }
            self.forms_opened = self.forms_opened.wrapping_add(1);
            self.dialog = Some(ParamsDialog {
                command: spec.id,
                text: params.to_string(),
                error: None,
                refused: None,
                touched: Default::default(),
                attempted: false,
                advanced: spec.form.is_none(),
                opening: self.forms_opened,
                back: None,
            });
        }
    }

    /// A reference field's "Create …": open the command that makes an item of `collection` over
    /// the open form, which comes back (with the new item at `path`) when that command runs or
    /// is cancelled. A provider that opens no form (a file picker, a missing requirement) leaves
    /// the form as it was.
    pub fn create_from_form(&mut self, collection: fieldcraft_engine::schema::Collection, path: String) {
        let Some(parent) = self.dialog.take() else { return };
        let before = self.session.names(collection);
        let provider = collection.requirement().provider();
        let _ = self.start(provider);
        match self.dialog.as_mut() {
            Some(d) if d.command == provider && d.back.is_none() => d.back = Some(Box::new(Back { dialog: parent, collection, path, before })),
            _ => self.dialog = Some(parent),
        }
    }

    /// Close the open form: back to the form it was opened from, if any.
    pub fn close_dialog(&mut self) {
        self.dialog = self.dialog.take().and_then(|d| d.back).map(|b| b.dialog);
    }

    /// The open form's command ran: back to the form it was opened from, with the item it made
    /// in the reference field.
    fn dialog_done(&mut self) {
        let Some(back) = self.dialog.take().and_then(|d| d.back) else { return };
        let Back { mut dialog, collection, path, before } = *back;
        let made = self.session.names(collection).into_iter().find(|n| !before.contains(n));
        if let (Some(name), Ok(mut params)) = (made, serde_json::from_str::<Value>(&dialog.text)) {
            let (parent, key) = path.rsplit_once('/').unwrap_or(("", &path));
            let pointer = if parent.is_empty() { String::new() } else { format!("/{parent}") };
            if let Some(Value::Object(obj)) = params.pointer_mut(&pointer) {
                obj.insert(key.into(), json!(name));
            }
            dialog.text = params.to_string();
        }
        self.dialog = Some(dialog);
    }

    /// Run the open parameter form; it closes when the command succeeds.
    pub fn run_dialog(&mut self) -> Result<Value, String> {
        let (command, params) = self.dialog_params()?;
        match self.run_form(command, params, true) {
            Ok(v) => {
                self.dialog_done();
                Ok(v)
            }
            Err(Failure { message, refused }) => {
                if let Some(open) = self.dialog.as_mut() {
                    open.error = Some(message.clone());
                    open.refused = refused;
                }
                Err(message)
            }
        }
    }

    /// The open form's command and parameters; parameters that are not JSON are the form's error.
    pub(crate) fn dialog_params(&mut self) -> Result<(&'static str, Value), String> {
        let Some(d) = self.dialog.as_mut() else { return Err("no command form is open".into()) };
        match serde_json::from_str::<Value>(&d.text) {
            Ok(params) => {
                d.error = None;
                Ok((d.command, params))
            }
            Err(e) => {
                let e = format!("the parameters are not JSON: {e}");
                d.error = Some(e.clone());
                Err(e)
            }
        }
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
            lines.push(format!("U {}  |U| {}", triple(u), results::number(fieldcraft_engine::results::magnitude(u))));
        }
        if let Some(rf) = vector("reaction").filter(|r| r.iter().any(|c| *c != 0.0)) {
            lines.push(format!("RF {}", triple(rf)));
        }
        if let Some(m) = at.get("von_mises").and_then(Value::as_f64) {
            lines.push(format!("Von Mises {}", results::number(m)));
        }
        if let Some(v) = vector("velocity") {
            lines.push(format!("V {}  |V| {}", triple(v), results::number(fieldcraft_engine::results::magnitude(v))));
        }
        if let Some(shape) = vector("shape") {
            lines.push(format!("Mode {} shape {}", pick.mode, triple(shape)));
        }
        let contour = self.plot().and_then(|p| p.contour.as_ref().map(|c| (c.field, c.at(p.result.nodes(), node))));
        if let Some((field, v)) = contour.filter(|(f, _)| *f != fieldcraft_engine::results::Field::VonMises) {
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
        self.frame_new_document();
        // A reply about the model is read again; any other one is out of date.
        self.reply = self.reply.take().and_then(|mut r| {
            reply::LIVE.contains(&r.command.as_str()).then_some(())?;
            r.value = self.session.execute(&r.command, &r.params).ok()?;
            Some(r)
        });
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
                        let mesh = tolerance.and_then(|t| body.tessellate(t)).map_err(|e| failed.push(format!("{}: {e}", body.name))).ok()?;
                        let names = selection::BodyNames {
                            name: body.name.clone(),
                            faces: body.faces().iter().map(|f| f.name.clone()).collect(),
                            edges: body.edge_names(&mesh),
                        };
                        Some((names, mesh))
                    })
                    .collect();
                self.bodies = (design, meshes);
                if !failed.is_empty() {
                    self.set_status(format!("Some bodies cannot be drawn: {}", failed.join("; ")), true);
                }
            }
            self.pick.names.clear();
            for (names, mesh) in &self.bodies.1 {
                d.add_curves(&mesh.edges);
                d.add_surface(&mesh.positions, &mesh.triangles, &mesh.tri_face);
                self.pick.names.push(names.clone());
            }
            // An imported surface's faces are named by their tags.
            for s in &self.session.surfaces {
                d.add_surface(&s.mesh.positions, &s.mesh.triangles, &s.mesh.tri_face);
                self.pick.names.push(selection::BodyNames {
                    name: s.name.clone(),
                    faces: (0..s.faces).map(|f| f.to_string()).collect(),
                    edges: Vec::new(),
                });
            }
            let was_empty = self.display.1.bounds.is_empty();
            self.display = (Some(self.revision), d);
            self.glyphs = Glyphs::from_model(&self.session.model);
            if was_empty && !self.display.1.bounds.is_empty() {
                self.reframe = framing::Reframe::merge(self.reframe, framing::Reframe::Fit);
            }
        }
        &self.display.1
    }

    /// Definitions whose symbols are a sample (`Clamp: 400 of 12000 nodes`).
    pub fn glyph_notes(&self) -> &[String] {
        &self.glyphs.notes
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
            let mut s = match &self.plot.1 {
                Some(p) => results::scene(p, style, show, &t),
                None => scene::build(&self.display.1, style, show, &t),
            };
            // Symbols follow the drawn nodes: the deformed shape of a results plot.
            let drawn = self.plot.1.as_ref().map_or(&self.display.1, |p| &p.display);
            let mut symbols = scene::symbols(&mut s, &self.glyphs, |n| drawn.node_position(n), show, self.plot.1.is_some(), &t);
            if show.geometry && self.plot.1.is_none() {
                symbols.union(scene::datums(&mut s, &self.session.model, &self.display.1.bounds, &t));
                scene::seeds(&mut s, &self.session.model, &self.pick.names, &self.display.1, &t);
            }
            self.scene = (Some(key), s, symbols);
        }
        self.scene.1.radius = scene::radius(&self.drawn_bounds(), &self.cam);
        (key, &self.scene.1)
    }

    /// What the scene built last draws: the model, a results plot's (deformed) shape and the
    /// setup's symbols.
    fn drawn_bounds(&self) -> Aabb3 {
        let mut bounds = self.display.1.bounds;
        if let Some(p) = &self.plot.1 {
            bounds.union(p.display.bounds);
        }
        bounds.union(self.scene.2);
        bounds
    }

    /// A CPU render of the viewport's scene with the current camera and the selection, as PNG
    /// bytes.
    pub fn render_png(&mut self, w: usize, h: usize) -> Result<Vec<u8>, fieldcraft_render::RenderError> {
        let cam = self.cam;
        let (_, overlay) = self.overlay();
        let (_, scene) = self.scene();
        fieldcraft_render::render_layers_png(scene, Some(&overlay), &cam, w, h)
    }

    /// The geometry tags behind the drawn mesh, read again when the model changed.
    fn pick_index(&mut self) -> Arc<selection::Index> {
        self.display();
        if self.pick.index.0 != Some(self.revision) {
            self.pick.index = (Some(self.revision), Arc::new(selection::Index::new(&self.session.model, &self.display.1)));
        }
        self.pick.index.1.clone()
    }

    /// Run `f` with the picker of what the viewport draws: a results plot's elements, or the
    /// model's elements and its geometry where no element faces cover it (as [`scene::build`]).
    pub fn with_picker<R>(&mut self, f: impl FnOnce(&selection::Picker) -> R) -> R {
        let index = self.pick_index();
        let plot = self.plot();
        let (show_mesh, show_geometry) = (self.ui.show_mesh, self.ui.show_geometry);
        let display = plot.as_ref().map_or(&self.display.1, |p| &p.display);
        let mesh_faces = show_mesh && !display.tris.is_empty();
        let picker = selection::Picker {
            display,
            names: &self.pick.names,
            model: &self.session.model,
            index: &index,
            elements: show_mesh,
            geometry: show_geometry && !mesh_faces,
            curves: show_geometry,
        };
        f(&picker)
    }

    /// The viewport's projection as drawn last frame (`None` before it was drawn).
    pub fn projector(&mut self) -> Option<fieldcraft_render::pick::Projector> {
        let rect = self.viewport.rect?;
        let radius = self.scene().1.radius;
        Some(fieldcraft_render::pick::Projector::new(&self.cam, f64::from(rect.width()), f64::from(rect.height()), radius))
    }

    /// The nearest drawn point under window point `at`: the pick ray's first hit on the results
    /// plot when one shows, else on the shown elements' faces or the design's surfaces.
    pub fn surface_point(&mut self, at: egui::Pos2) -> Option<Vec3> {
        let rect = self.viewport.rect.filter(|r| r.contains(at))?;
        let (o, d) = self.cam.ray(f64::from(at.x - rect.left()), f64::from(at.y - rect.top()), f64::from(rect.width()), f64::from(rect.height()));
        let t = if let Some(plot) = self.plot() {
            plot.display.hit_element_face(o, d).map(|h| h.t)
        } else {
            let (mesh, geometry) = (self.ui.show_mesh, self.ui.show_geometry);
            let display = self.display();
            let faces = mesh.then(|| display.hit_element_face(o, d)).flatten().map(|h| h.t);
            let surface = geometry.then(|| display.hit_surface(o, d)).flatten().map(|h| h.t);
            faces.into_iter().chain(surface).min_by(f64::total_cmp)
        }?;
        let p = o + d * t;
        p.is_finite().then_some(p)
    }

    /// What an orbit that starts on empty space turns about: the centre of what is drawn (the
    /// target when nothing is).
    pub fn drawn_centre(&mut self) -> Vec3 {
        self.scene();
        let b = self.drawn_bounds();
        if b.is_empty() || !b.center().is_finite() { self.cam.target } else { b.center() }
    }

    /// What a click at window point `at` would pick with the current filter.
    pub fn pick_at(&mut self, at: egui::Pos2) -> Option<selection::Item> {
        let rect = self.viewport.rect.filter(|r| r.contains(at))?;
        let proj = self.projector()?;
        let filter = self.pick.filter;
        let local = [f64::from(at.x - rect.left()), f64::from(at.y - rect.top())];
        self.with_picker(|p| p.pick(&proj, local, filter))
    }

    /// Change the selection by `items` (see [`selection::Mode`]); the status bar says what is
    /// selected.
    pub fn select(&mut self, items: impl IntoIterator<Item = selection::Item>, mode: selection::Mode) {
        if self.pick.selection.apply(items, mode) {
            self.pick.generation = self.pick.generation.wrapping_add(1);
        }
        let mut text = format!("Selected: {}", self.pick.selection.summary());
        if self.pick.selection.capped() {
            text.push_str(&format!(" (at most {} items)", selection::MAX_SELECTION));
        }
        self.set_status(text, false);
    }

    pub fn clear_selection(&mut self) {
        if !self.pick.selection.is_empty() {
            self.pick.selection.clear();
            self.pick.generation = self.pick.generation.wrapping_add(1);
            self.set_status("Selection cleared", false);
        }
    }

    /// Pick at window point `at` and change the selection by it; a plain click on nothing clears
    /// the selection. Returns what was picked.
    pub fn select_at(&mut self, at: egui::Pos2, mode: selection::Mode) -> Option<selection::Item> {
        // A click where the hover was just worked out picks what is highlighted.
        let current =
            self.pick.hover_key.as_ref().is_some_and(|k| k.0 == Some(at) && k.1 == self.cam && k.2 == self.scene_key() && k.3 == self.pick.filter);
        let item = if current { self.pick.hover.clone() } else { self.pick_at(at) };
        match &item {
            Some(i) => self.select([i.clone()], mode),
            None if mode == selection::Mode::Replace => self.clear_selection(),
            None => {}
        }
        item
    }

    /// Select what is visible inside the box between window points `a` and `b` (nodes, element
    /// faces or elements, with the filter); returns how many items it held.
    pub fn select_box(&mut self, a: egui::Pos2, b: egui::Pos2, mode: selection::Mode) -> usize {
        let (Some(rect), Some(proj)) = (self.viewport.rect, self.projector()) else { return 0 };
        let filter = self.pick.filter;
        let r = [a.x - rect.left(), a.y - rect.top(), b.x - rect.left(), b.y - rect.top()].map(f64::from);
        let items = self.with_picker(|p| p.pick_box(&proj, r, filter));
        let n = items.len();
        self.select(items, mode);
        n
    }

    /// Set what is under the pointer (`None` when nothing, or the pointer is elsewhere).
    pub fn set_hover(&mut self, item: Option<selection::Item>) {
        if self.pick.hover != item {
            self.pick.hover = item;
            self.pick.hover_generation = self.pick.hover_generation.wrapping_add(1);
        }
    }

    /// The selection and the hover drawn over the scene in the theme's highlight colours,
    /// rebuilt when either or the scene changes; and its key.
    pub fn overlay(&mut self) -> (u64, Arc<Scene>) {
        let hash = |v: &dyn Fn(&mut std::hash::DefaultHasher)| {
            let mut h = std::hash::DefaultHasher::new();
            v(&mut h);
            h.finish()
        };
        let scene = self.scene_key();
        let selected = hash(&|h| (scene, self.pick.generation).hash(h));
        let key = hash(&|h| (selected, self.pick.hover_generation).hash(h));
        if self.pick.overlay.0 == Some(key) {
            return (key, self.pick.overlay.1.clone());
        }
        let t = theme::Tokens::get(self.ui.dark);
        // The selection's part is kept while only the hover changes.
        if self.pick.selected_overlay.0 != Some(selected) {
            let sel = std::mem::take(&mut self.pick.selection);
            let s = self.with_picker(|p| p.overlay(&sel, None, theme::rgb(t.pick_selected), theme::rgb(t.pick_hover)));
            self.pick.selection = sel;
            self.pick.selected_overlay = (Some(selected), Arc::new(s));
        }
        let mut s = (*self.pick.selected_overlay.1).clone();
        if let Some(hover) = self.pick.hover.clone() {
            // Over result contours the hover is an outline, so it never hides the result (#207).
            let fill = self.plot.1.is_none();
            let h = self.with_picker(|p| p.hover(&hover, theme::rgb(t.pick_hover), fill));
            s.tris.extend(h.tris);
            s.lines.extend(h.lines);
            s.points.extend(h.points);
        }
        self.pick.overlay = (Some(key), Arc::new(s));
        (key, self.pick.overlay.1.clone())
    }

    /// Drop selected nodes, elements and element faces the model no longer has (after an edit,
    /// an undo or a remesh); geometry stays selected by name.
    fn check_selection(&mut self) {
        if self.pick.checked == Some(self.revision) {
            return;
        }
        self.pick.checked = Some(self.revision);
        let model = &self.session.model;
        let before = self.pick.selection.len();
        self.pick.selection.retain(|item| match item {
            selection::Item::Geometry(_) => true,
            selection::Item::Node(n) => model.node(*n).is_some(),
            selection::Item::Element(e) => model.element(*e).is_some(),
            selection::Item::Face(f) => model.face_nodes(*f).is_some(),
        });
        if self.pick.selection.len() != before {
            self.pick.generation = self.pick.generation.wrapping_add(1);
        }
    }

    /// Open `model.set.create`'s form with the selection as the new set's members.
    pub fn create_set_from_selection(&mut self) -> Result<(), String> {
        let params = self.selection_set(None)?;
        self.open_form("model.set.create", params);
        Ok(())
    }

    /// `model.set.create`'s parameters for a set of the selection, named after `base` (`Set-1`
    /// by default; the first name not taken).
    fn selection_set(&self, base: Option<&str>) -> Result<Value, String> {
        if self.pick.selection.is_empty() {
            return Err("nothing is selected".into());
        }
        let name = selection::fresh_set_name(&self.session.model, base.unwrap_or("Set-1"));
        Ok(selection::with_members(&json!({"name": name}), &self.pick.selection))
    }

    /// Put the selection into the open command form: a set's members, or a new set made from it
    /// (`model.set.create`, named after the definition) as the target of a boundary condition,
    /// load, section or coupling.
    pub fn use_selection_in_dialog(&mut self) -> Result<(), String> {
        let Some(d) = self.dialog.as_ref() else { return Err("no command form is open".into()) };
        let command = d.command;
        if self.pick.selection.is_empty() {
            return Err("nothing is selected".into());
        }
        let params: Value = serde_json::from_str(&d.text).map_err(|e| format!("the parameters are not JSON: {e}"))?;
        if !params.is_object() {
            return Err("the parameters must be a JSON object".into());
        }
        let Some(target) = selection::target(command, &params) else { return Err(format!("{command} takes no selection")) };
        let filled = match target {
            selection::Target::Members => selection::with_members(&params, &self.pick.selection),
            selection::Target::Edges => {
                selection::with_edges(&params, &self.pick.selection).ok_or("no edge is selected: pick edges with Select: Edges")?
            }
            selection::Target::Set | selection::Target::TargetSet => {
                let base = params.get("name").and_then(Value::as_str).map(|n| format!("{n}-set"));
                let set = self.selection_set(base.as_deref())?;
                let name = set.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                self.run("model.set.create", set)?;
                selection::with_set(&params, target, &name)
            }
        };
        if let Some(open) = self.dialog.as_mut() {
            open.text = serde_json::to_string_pretty(&filled).unwrap_or_else(|_| filled.to_string());
            open.error = None;
        }
        Ok(())
    }

    /// Frame what is drawn in the part of the viewport the overlays leave free (see
    /// [`framing`]), keeping the direction of view.
    pub fn fit_view(&mut self) {
        // A view asked for replaces one still waiting to be done.
        self.reframe = None;
        self.scene();
        let b = self.drawn_bounds();
        let (aspect, free) = self.free_part();
        self.cam.fit_in(&b, aspect, free);
    }

    /// The viewport's aspect and the part of it the overlays leave free, as fractions (the whole
    /// of a square view before the viewport is first drawn).
    fn free_part(&self) -> (f64, [f64; 4]) {
        match self.viewport.rect.filter(|r| r.width() > 1.0 && r.height() > 1.0) {
            Some(r) => (f64::from(r.width()) / f64::from(r.height()), framing::free_part(r, &self.viewport.overlays)),
            None => (1.0, fieldcraft_render::FULL_VIEW),
        }
    }

    /// Whether every corner of what is drawn shows in the free part of the viewport.
    fn framed(&mut self) -> bool {
        let Some(r) = self.viewport.rect.filter(|r| r.width() > 1.0 && r.height() > 1.0) else { return true };
        self.scene();
        let b = self.drawn_bounds();
        if b.is_empty() {
            return true;
        }
        let [x0, y0, x1, y1] = framing::free_part(r, &self.viewport.overlays);
        let (w, h) = (f64::from(r.width()), f64::from(r.height()));
        let p = fieldcraft_render::pick::Projector::new(&self.cam, w, h, self.scene.1.radius);
        b.corners().into_iter().all(|c| p.screen(c).is_some_and(|[x, y, _]| x >= x0 * w && x <= x1 * w && y >= y0 * h && y <= y1 * h))
    }

    /// Another model, sample or design opened (or the window's first one): a fresh start's home
    /// view when the viewport next draws.
    fn frame_new_document(&mut self) {
        if self.framed_document != Some(self.session.opened()) {
            self.framed_document = Some(self.session.opened());
            self.reframe = framing::Reframe::merge(self.reframe, framing::Reframe::Home);
        }
    }

    /// The home view: isometric, the whole model framed.
    pub fn home_view(&mut self) {
        self.set_view(StandardView::Iso);
    }

    /// Look along a standard view, the whole model framed.
    pub fn set_view(&mut self, v: StandardView) {
        self.cam.set_view(v);
        self.fit_view();
    }

    /// Do the framing asked for ([`framing::Reframe`]) once there is something to frame; the
    /// viewport calls it when it knows its size and its overlays, before drawing.
    pub(crate) fn apply_reframe(&mut self) {
        let Some(r) = self.reframe else { return };
        if self.display().bounds.is_empty() {
            return;
        }
        self.reframe = None;
        match r {
            framing::Reframe::Home => self.home_view(),
            framing::Reframe::Fit => self.fit_view(),
            framing::Reframe::IfOffscreen => {
                if !self.framed() {
                    self.fit_view();
                }
            }
        }
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
            self.ui.select_tab(Tab::Results);
            self.reframe = framing::Reframe::merge(self.reframe, framing::Reframe::IfOffscreen);
        }
        self.frame_new_document();
        self.results_shown = shown;
        self.check_selection();
        // A selected definition that is gone (undo, delete) is no longer selected.
        if self.selected.as_ref().is_some_and(|(kind, name)| !kind.exists(&self.session.model, name)) {
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
        // Escape clears the viewport's selection first (a second one cancels an open form).
        if !self.pick.selection.is_empty()
            && self.ui.menu.is_none()
            && !self.ui.palette
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.clear_selection();
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
        self.save_prefs();
    }

    /// The setup progress ([`fieldcraft_engine::progress`]) the browser marks, worked out again
    /// only when the model or its results change.
    pub fn setup_stages(&mut self) -> std::sync::Arc<Vec<fieldcraft_engine::progress::StageState>> {
        use fieldcraft_engine::requirement::Requirement;
        let key = (self.revision, self.session.revision(), Requirement::Result.met(&self.session), Requirement::ResultsShown.met(&self.session));
        if self.setup.0 != Some(key) {
            self.setup = (Some(key), std::sync::Arc::new(self.session.progress()));
        }
        self.setup.1.clone()
    }

    /// Save the preferences the window changed (a View menu toggle, `ui.set`).
    pub(crate) fn save_prefs(&mut self) {
        if self.ui.nav_preset != self.prefs.nav_preset || self.ui.reverse_zoom != self.prefs.reverse_zoom {
            self.prefs = files::Prefs { nav_preset: self.ui.nav_preset.clone(), reverse_zoom: self.ui.reverse_zoom };
            self.prefs.save(self.services.prefs_path.as_deref());
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

impl ModelApp {
    /// Why a toolbar entry or a command cannot start now: what its command needs that the
    /// session lacks (`fieldcraft_engine::requirement`). `None` when it can start, and for
    /// coming-soon entries and unknown ids (which say so themselves).
    pub fn unavailable(&self, id: &str) -> Option<fieldcraft_engine::requirement::Unavailable> {
        let command = catalogue::find(id).map_or(Some(id), |c| c.command())?;
        let spec = fieldcraft_engine::find_command(command)?;
        self.session.availability(spec).err()
    }
}

/// A command's name as people see it (its label).
pub fn command_label(id: &str) -> &str {
    fieldcraft_engine::find_command(id).map_or(id, |c| c.label)
}

#[cfg(test)]
mod tests {
    use fieldcraft_render::Vec3;

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
        assert!(app.status.as_ref().is_some_and(|s| s.text.starts_with("Model Info: 1 node, 0 elements")), "{:?}", app.status);
    }

    /// Every command run from the UI reports one readable line, never its JSON, in every state
    /// tried (empty, a solved sample); a reply with more to it opens in the context panel.
    #[test]
    fn every_reply_reads_as_one_line() {
        let empty = Session::default();
        let mut solved = ModelApp::new(Session::default());
        solved.open_sample("cantilever");
        solved.run("solve.run", json!({})).unwrap();
        let mut ran = 0;
        for session in [empty, solved.session.clone()] {
            // Every command bare, and the queries that need parameters with ones the sample has.
            let bare = fieldcraft_engine::commands().iter().filter(|c| !c.id.starts_with("file.")).map(|c| (c.id, json!({})));
            let queries = [
                ("model.set.resolve", json!({"name": "root"})),
                ("solve.probe", json!({"step": "Load", "nodes": [1, 2]})),
                ("solve.probe", json!({"step": "Modes", "set": "tip", "mode": 1})),
                ("results.reactions", json!({"step": "Load"})),
            ];
            for (id, params) in bare.chain(queries) {
                let mut app = ModelApp::new(session.clone());
                let r = app.run(id, params);
                let Some(status) = app.status.clone() else { continue };
                let text = status.text;
                assert!(!text.contains('\n'), "{id}: {text}");
                if r.is_ok() {
                    ran += 1;
                    assert!(!text.contains("{\"") && !text.contains("[{") && text.chars().count() <= 200, "{id}: {text}");
                }
            }
        }
        assert!(ran >= 20, "only {ran} commands ran");
    }

    /// Check Model and the Step Table open their readable view in the context panel, which
    /// follows the model as it changes.
    #[test]
    fn check_model_opens_its_diagnostics_in_the_panel() {
        let mut app = ModelApp::new(Session::default());
        app.run("model.check", json!({})).unwrap();
        let status = app.status.clone().unwrap();
        assert_eq!(status.text, "Check Model: 2 errors, 1 warning (details in the panel)");
        assert!(app.reply.as_ref().is_some_and(|r| r.command == "model.check"));
        assert!(context::has_context(&mut app));
        app.run("model.units", json!({"system": "mm-t-s-N-MPa"})).unwrap();
        assert_eq!(app.reply.as_ref().map(|r| r.value["warnings"].clone()), Some(json!(0)), "the check follows the model");
        let mut app = ModelApp::new(Session::default());
        app.open_sample("cantilever");
        app.run("model.activity", json!({})).unwrap();
        assert_eq!(app.status.unwrap().text, "Step Table: 2 steps, 1 boundary condition, 1 load (details in the panel)");
    }

    /// Commands run from outside the ribbon bring their tab; queries and Tools commands leave it.
    #[test]
    fn the_ribbon_follows_commands_to_their_stage() {
        let mut app = ModelApp::new(Session::default());
        app.ui.select_tab(Tab::Results);
        app.run("mesh.node.add", json!({"label": 1, "xyz": [0, 0, 0]})).unwrap();
        assert_eq!((app.ui.workspace, app.ui.tab), (Workspace::Prepare, Tab::Mesh));
        app.run("model.material.create", json!({"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}})).unwrap();
        assert_eq!((app.ui.workspace, app.ui.tab), (Workspace::Model, Tab::Properties));
        // A query and a failed command leave it; so does undo (Tools).
        app.run("mesh.quality", json!({})).ok();
        let _ = app.run("model.load.create", json!({"name": "bad"}));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.ui.tab, Tab::Properties);
        // A script follows its last command with a stage.
        app.run_script(&json!([{"command": "mesh.node.add", "params": {"label": 2, "xyz": [1, 0, 0]}}, {"command": "model.info", "params": {}}]))
            .unwrap();
        assert_eq!(app.ui.tab, Tab::Mesh);
        // The ribbon's own entries never move it.
        app.ui.select_tab(Tab::Properties);
        app.start("model.units").unwrap();
        assert_eq!(app.ui.tab, Tab::Properties);
    }

    /// Mesh Quality reads as a sentence in the status bar, not as its JSON report.
    #[test]
    fn mesh_quality_reports_in_a_line() {
        let mut app = ModelApp::new(Session::default());
        // Nothing to measure: the command needs a mesh and says so.
        assert!(app.run("mesh.quality", json!({})).is_err());
        app.run_script(&fieldcraft_engine::sample::script()).unwrap();
        app.run("mesh.quality", json!({})).unwrap();
        let status = app.status.clone().unwrap();
        assert!(status.text.starts_with("Mesh Quality: 226 elements, all valid; scaled Jacobian min "), "{}", status.text);
        assert!(!status.error && !status.text.contains('{'));
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

    /// A parameter the command refuses is named by its field's label in the form and the status
    /// bar; the command itself (agents) still names the key.
    #[test]
    fn a_refused_parameter_is_named_by_its_label() {
        let mut app = ModelApp::new(Session::default());
        app.run("model.material.create", json!({"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}})).unwrap();
        app.start("model.material.plasticity").unwrap();
        app.dialog.as_mut().unwrap().text = r#"{"name": 7}"#.into();
        let e = app.run_dialog().unwrap_err();
        assert_eq!(e, "Material: must be a name");
        let d = app.dialog.as_ref().unwrap();
        assert_eq!(d.refused, Some(("name".to_string(), "Material: must be a name".to_string())));
        assert!(app.status.as_ref().is_some_and(|s| s.text == "Plasticity: Material: must be a name"));
        let raw = app.session.execute("model.material.plasticity", &json!({"name": 7})).unwrap_err();
        assert_eq!(raw.to_string(), "`name` must be a name");
    }

    /// A required reference with one candidate starts picked (the browser's selection wins); with
    /// none, "Create …" opens the providing command over the form, which comes back with the new
    /// item picked (or as it was, when that command is cancelled).
    #[test]
    fn reference_fields_pick_the_only_candidate_and_create_a_missing_one() {
        use fieldcraft_engine::schema::Collection;
        let params = |app: &ModelApp| serde_json::from_str::<Value>(&app.dialog.as_ref().unwrap().text).unwrap();
        let steel = json!({"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}});
        let mut app = ModelApp::new(Session::default());
        app.run("mesh.node.add", json!({"label": 1, "xyz": [0, 0, 0]})).unwrap();
        app.run("model.set.create", json!({"name": "all", "nodes": [1]})).unwrap();
        // (Opened directly: from the toolbar, Section waits for a material.)
        app.open_form("model.section.create", json!({}));
        let opened = params(&app);
        assert_eq!(opened["set"], "all");
        assert!(opened.get("material").is_none());

        // Cancelled: back to the section form as it was.
        app.create_from_form(Collection::Material, "material".into());
        assert_eq!(app.dialog.as_ref().map(|d| d.command), Some("model.material.create"));
        app.close_dialog();
        assert_eq!(app.dialog.as_ref().map(|d| d.command), Some("model.section.create"));
        assert_eq!(params(&app), opened);

        // Made: back with it picked.
        app.create_from_form(Collection::Material, "material".into());
        app.dialog.as_mut().unwrap().text = steel.to_string();
        app.run_dialog().unwrap();
        let d = app.dialog.as_ref().unwrap();
        assert_eq!(d.command, "model.section.create");
        assert!(d.back.is_none());
        assert_eq!(params(&app)["material"], "Steel");
        app.close_dialog();
        assert!(app.dialog.is_none());

        // One material: Plasticity starts with it.
        app.start("model.material.plasticity").unwrap();
        assert_eq!(params(&app)["name"], "Steel");
        // Two: the one selected in the browser, else nothing.
        app.run("model.material.create", json!({"name": "Al", "elastic": {"type": "isotropic", "young": 70000, "poisson": 0.33}})).unwrap();
        app.start("model.material.plasticity").unwrap();
        assert!(params(&app).get("name").is_none());
        app.selected = Some((browser::Def::Material, "Steel".into()));
        app.start("model.material.plasticity").unwrap();
        assert_eq!(params(&app)["name"], "Steel");
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
        // An entry whose command misses what it needs opens nothing and says what is missing.
        let err = app.start("deck.write.abaqus").unwrap_err();
        assert_eq!(err, "deck.write cannot run yet. Needs a mesh — Mesh › Tet Mesh");
        assert!(app.dialog.is_none());
        assert_eq!(app.status.as_ref().map(|s| (s.error, s.text.as_str())), Some((true, err.as_str())));
        // With a model, a deck entry opens deck.write's form with its format filled in.
        app.run("file.sample", json!({"name": "cantilever"})).unwrap();
        app.start("deck.write.abaqus").unwrap();
        let d = app.dialog.as_ref().unwrap();
        assert_eq!((d.command, d.text.as_str()), ("deck.write", r#"{"format":"abaqus"}"#));
    }

    /// The ribbon, the command search and Solve grey what the model cannot run yet, and say why.
    #[test]
    fn entries_missing_what_they_need_are_greyed_with_the_reason() {
        let mut app = ModelApp::new(Session::default());
        let entry =
            |app: &ModelApp, id: &str| crate::toolbar::describe(app)["entries"].as_array().unwrap().iter().find(|e| e["id"] == id).unwrap().clone();
        let plasticity = entry(&app, "model.material.plasticity");
        assert_eq!(plasticity["enabled"], false);
        assert_eq!(plasticity["unavailable"]["reason"], "Needs a material — Properties › Material");
        assert_eq!(plasticity["unavailable"]["missing"][0]["provider"], "model.material.create");
        assert_eq!(entry(&app, "model.material.create")["enabled"], true);
        assert_eq!(entry(&app, "mesh.order.quadratic")["enabled"], false, "coming soon");
        let solve = app.unavailable("solve.run").unwrap();
        assert_eq!(solve.lines(), "Needs a mesh — Mesh › Tet Mesh\nNeeds a step — Steps › Step");

        app.run("file.sample", json!({"name": "cantilever"})).unwrap();
        assert_eq!(entry(&app, "model.material.plasticity")["enabled"], true);
        assert!(app.unavailable("solve.run").is_none());
        assert_eq!(entry(&app, "results.contour")["unavailable"]["reason"], "Needs a solved step — Solve › Solve");
    }

    #[test]
    fn the_sample_fills_the_view_and_renders() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&fieldcraft_engine::sample::script()).unwrap();
        let d = app.display().clone();
        assert!(!d.tris.is_empty() && !d.edges.is_empty());
        // Framed when the viewport first draws.
        draw(&mut app, &egui::Context::default(), 1280.0, 800.0);
        assert!(app.framed(), "{:?}", app.cam);
        let png = app.render_png(64, 48).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn the_open_design_and_imported_surfaces_are_drawn() {
        let dir = std::env::temp_dir().join(format!("fieldcraft-ui-design-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let design = dir.join("plate.solidcraft");
        let bytes = fieldcraft_geometry::sample::plate_design(&fieldcraft_geometry::sample::Plate::default()).unwrap();
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
    fn setup_symbols_show_with_their_toggles_and_fade_over_results() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&fieldcraft_engine::sample::cantilever()).unwrap();
        let t = theme::Tokens::get(app.ui.dark);
        let count = |app: &mut ModelApp, c: egui::Color32| app.scene().1.tris.iter().filter(|x| x.color == theme::rgb(c)).count();
        assert!(count(&mut app, t.glyph_constraint) > 0, "the clamp's cones");
        assert!(app.scene().1.lines.iter().any(|l| l.color == theme::rgb(t.glyph_load)), "the push's arrows");
        // The symbols reach beyond the model: the clipping radius holds them.
        let radius = app.scene().1.radius;
        app.ui.show_loads = false;
        assert_eq!(count(&mut app, t.glyph_constraint), 0);
        assert!(app.scene().1.radius <= radius);
        app.ui.show_loads = true;
        // Over a results plot they are muted.
        app.run("solve.static", json!({})).unwrap();
        app.run("results.deformed", json!({"step": "Load"})).unwrap();
        assert_eq!(count(&mut app, t.glyph_constraint), 0);
        let muted = t.glyph_colors().over_results(theme::rgb(t.viewport_bottom));
        assert!(app.scene().1.tris.iter().any(|x| x.color == muted.constraint));
        assert!(app.glyph_notes().is_empty());
    }

    #[test]
    fn solving_shows_the_result_on_the_results_tab() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&fieldcraft_engine::sample::cantilever()).unwrap();
        app.start("solve.static").unwrap();
        app.run_dialog().unwrap();
        let view = app.session.results_view.clone().unwrap();
        // A static step first shows its contours on the undeformed shape (#77).
        assert_eq!((view.step.as_str(), view.contour, view.deformed), ("Load", Some(fieldcraft_engine::results::Field::VonMises), false));
        app.logic(&egui::Context::default());
        assert_eq!(app.ui.tab, Tab::Results);
        app.run("solve.frequency", json!({})).unwrap();
        app.run("results.deformed", json!({"step": "Modes"})).unwrap();
        let view = app.session.results_view.clone().unwrap();
        assert_eq!((view.step.as_str(), view.mode, view.contour), ("Modes", 1, Some(fieldcraft_engine::results::Field::Displacement)));
        // The plot draws contour bands: more triangles than the model's faces.
        let faces = app.display().tris.len();
        assert!(app.scene().1.tris.len() > faces);
        app.run("results.clear", json!({})).unwrap();
        assert!(app.plot().is_none());
        // The model's faces again (without the setup's symbols).
        app.ui.show_loads = false;
        assert_eq!(app.scene().1.tris.len(), faces);
    }

    /// One frame of the window at `w` × `h` points.
    fn draw(app: &mut ModelApp, ctx: &egui::Context, w: f32, h: f32) {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }

    fn is_iso(cam: &Camera) -> bool {
        let (yaw, pitch) = StandardView::Iso.angles();
        (cam.yaw - yaw).abs() < 1e-9 && (cam.pitch - pitch).abs() < 1e-9
    }

    /// Opening a sample, then another, frames each in the home view; an edit keeps the camera
    /// (#233, #223).
    #[test]
    fn opening_another_sample_frames_it_and_an_edit_keeps_the_camera() {
        let mut app = ModelApp::new(Session::default());
        let ctx = egui::Context::default();
        app.run("file.sample", json!({"name": "crank-arm"})).unwrap();
        draw(&mut app, &ctx, 1280.0, 800.0);
        assert!(is_iso(&app.cam) && app.framed(), "{:?}", app.cam);
        // Looked at from elsewhere, an edit keeps that camera.
        app.cam.orbit(120.0, -40.0);
        app.cam.zoom_at(0.2, None);
        let turned = app.cam;
        app.run("model.material.create", json!({"name": "Al", "elastic": {"type": "isotropic", "young": 70000, "poisson": 0.33}})).unwrap();
        draw(&mut app, &ctx, 1280.0, 800.0);
        assert_eq!(app.cam, turned);
        // Another sample: a fresh start's home view, all of it in sight.
        app.run("file.sample", json!({"name": "bracket-hex"})).unwrap();
        draw(&mut app, &ctx, 1280.0, 800.0);
        assert!(is_iso(&app.cam) && app.framed(), "{:?}", app.cam);
    }

    /// After a solve the model is in sight, clear of the legend, at the window sizes people use;
    /// a fit leaves the legend free with the deformed shape too (#223, #76).
    #[test]
    fn results_come_up_framed_clear_of_the_legend() {
        for (w, h) in [(1280.0, 800.0), (700.0, 1000.0)] {
            let mut app = ModelApp::new(Session::default());
            let ctx = egui::Context::default();
            app.run("file.sample", json!({"name": "crank-arm"})).unwrap();
            draw(&mut app, &ctx, w, h);
            // Zoomed in on a corner before solving: the results bring it all back.
            let corner = app.display().bounds.max;
            app.cam.zoom_at(0.3, Some(corner));
            app.run("solve.run", json!({})).unwrap();
            draw(&mut app, &ctx, w, h);
            draw(&mut app, &ctx, w, h);
            let legend = *app.layout.get("legend").expect("the legend is placed");
            assert!(app.framed(), "{w}×{h}: {:?}", app.cam);
            app.run("results.deformed", json!({"show": true, "scale": 200})).unwrap();
            draw(&mut app, &ctx, w, h);
            app.fit_view();
            assert!(app.framed(), "{w}×{h} deformed");
            let view = app.viewport.rect.unwrap();
            let b = app.drawn_bounds();
            for (k, c) in b.corners().into_iter().enumerate() {
                let at = crate::viewport::to_screen(&app, c).unwrap();
                assert!(!legend.contains(at) && view.contains(at), "{w}×{h}: corner {k} at {at:?}, legend {legend:?}");
            }
        }
    }

    /// Every standard view frames the whole model: the cantilever's front view keeps both ends
    /// (#147), in orthographic and perspective.
    #[test]
    fn standard_views_frame_the_whole_model() {
        for perspective in [false, true] {
            let mut app = ModelApp::new(Session::default());
            app.ui.perspective = perspective;
            let ctx = egui::Context::default();
            app.run("file.sample", json!({"name": "cantilever"})).unwrap();
            draw(&mut app, &ctx, 1280.0, 800.0);
            for v in StandardView::ALL {
                app.set_view(v);
                assert!(app.framed(), "{} view, perspective {perspective}: {:?}", v.label(), app.cam);
            }
        }
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
        let canvas = fieldcraft_render::render(app.scene().1, &cam, w, h).unwrap();
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
        app.run_script(&fieldcraft_engine::sample::impact()).unwrap();
        app.run("solve.explicit", json!({})).unwrap();
        // The last frame at true scale: the bar has moved back off the wall's side (x < 0 is
        // the wall, so nothing passes it) and carries stress contours.
        let last = app.plot().unwrap();
        assert_eq!(last.frame().map(|(k, _, n)| (k, n)), Some((10, 11)));
        assert!(last.contour.as_ref().is_some_and(|c| c.field == fieldcraft_engine::results::Field::VonMises));
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
        app.ui.select_tab(Tab::Mesh);
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
                app.ui.select_tab(tab);
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

    /// A face picked by name stays selected through a remesh and highlights the new mesh's faces;
    /// the set made from it resolves on the new mesh.
    #[test]
    fn a_geometry_pick_survives_a_remesh() {
        let block = |n: u32| {
            let corners = [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0], [0, 0, 10], [10, 0, 10], [10, 10, 10], [0, 10, 10]];
            json!({"body": "Bar", "kind": "hex8", "corners": corners, "divisions": [n, n, n], "faces": {"4": "tip"}})
        };
        let mut app = ModelApp::new(Session::default());
        app.run("mesh.block", block(2)).unwrap();
        app.viewport.rect = Some(egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 600.0)));
        app.set_view(StandardView::Right);
        app.fit_view();
        let at = viewport::to_screen(&app, Vec3::new(10.0, 5.5, 4.5)).unwrap();
        let tip = selection::Item::Geometry(fieldcraft_model::GeometryRef::face("Bar", "tip").unwrap());
        assert_eq!(app.select_at(at, selection::Mode::Replace), Some(tip.clone()));
        assert_eq!(app.overlay().1.tris.len(), 2 * 2 * 2);
        // A click beside the model clears the selection; a Shift click adds back.
        assert_eq!(app.select_at(egui::pos2(105.0, 55.0), selection::Mode::Replace), None);
        assert!(app.pick.selection.is_empty());
        app.select_at(at, selection::Mode::Add);
        // Remeshed finer: the tip is still selected and highlights the new faces.
        app.run("edit.undo", json!({})).unwrap();
        app.run("mesh.block", block(4)).unwrap();
        app.logic(&egui::Context::default());
        assert_eq!(app.pick.selection.items().iter().collect::<Vec<_>>(), vec![&tip]);
        assert_eq!(app.overlay().1.tris.len(), 4 * 4 * 2);
        app.create_set_from_selection().unwrap();
        assert_eq!(app.dialog.as_ref().map(|d| d.command), Some("model.set.create"));
        app.run_dialog().unwrap();
        assert_eq!(app.session.model.resolve_set("Set-1").unwrap().faces.len(), 16);
        // Mesh labels that are gone leave the selection.
        app.select([selection::Item::Node(1), selection::Item::Element(9999)], selection::Mode::Replace);
        app.run("mesh.node.add", json!({"label": 100_000, "xyz": [50, 50, 50]})).unwrap();
        app.logic(&egui::Context::default());
        assert_eq!(app.pick.selection.items().iter().collect::<Vec<_>>(), vec![&selection::Item::Node(1)]);
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

    /// An orbit pivots on the model point under the press (the pick ray's first hit, on the
    /// face toward the eye) and keeps it still on screen; from empty space it pivots on the
    /// centre of what is drawn (#262).
    #[test]
    fn orbit_pivots_on_the_point_under_the_cursor_or_the_drawn_centre() {
        let corners = [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0], [0, 0, 10], [10, 0, 10], [10, 10, 10], [0, 10, 10]];
        let mut app = ModelApp::new(Session::default());
        app.run("mesh.block", json!({"body": "Bar", "kind": "hex8", "corners": corners, "divisions": [2, 2, 2]})).unwrap();
        app.viewport.rect = Some(egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 600.0)));
        app.set_view(StandardView::Right);
        app.fit_view();
        // The right view looks along −X: the face under the cursor is x = 10.
        let on = Vec3::new(10.0, 7.0, 3.0);
        let at = viewport::to_screen(&app, on).unwrap();
        let pivot = app.surface_point(at).unwrap();
        assert!(pivot.dist(on) < 1e-6, "{pivot:?}");
        app.cam.orbit_about(pivot, 60.0, -30.0);
        let after = viewport::to_screen(&app, pivot).unwrap();
        assert!((after - at).length() < 1e-3, "{at:?} -> {after:?}");
        // Empty space: the centre of the drawn bounds.
        let pivot = app.drawn_centre();
        assert!(pivot.dist(Vec3::new(5.0, 5.0, 5.0)) < 1e-6, "{pivot:?}");
        assert!(app.surface_point(egui::pos2(105.0, 55.0)).is_none());
        // Nothing drawn: the target.
        let mut empty = ModelApp::new(Session::default());
        empty.cam.target = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(empty.drawn_centre(), Vec3::new(1.0, 2.0, 3.0));
    }
}
