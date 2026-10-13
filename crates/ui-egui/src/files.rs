//! Files the user opens and saves: the host's native file pickers ([`Services`]), opening off the
//! window thread, drag and drop, the recent-files list, and the prompts around them (unsaved
//! changes, autosaved models to recover).
//!
//! Only user actions reach a picker: a click (toolbar, title bar, File menu), a shortcut or a drop.
//! Programmatic calls (`engine.execute`, the control channel's `ui.start`) run commands with the
//! paths they are given and never open a native dialog. Without a picker service (tests, the
//! offscreen capture, the web), the same entries open their command's parameter form instead.
//!
//! A file opens on a copy of the session in a worker thread; the window keeps drawing and shows
//! the file's progress with Cancel. The finished copy replaces the session only when the command
//! succeeded, so a failed or cancelled open leaves the model as it was. Structure follows
//! SolidCraft's `crates/ui-egui/src/file_load.rs` and `documents.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use modelcraft_engine::Session;
use serde_json::{Value, json};

use crate::ModelApp;

/// A file type a picker offers: a name and its extensions (without the dot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

/// The host picks a file to open from the filters (the first is selected), or `None` when the
/// user cancelled.
pub type PickOpen = Box<dyn Fn(&[Filter]) -> Option<String>>;
/// The host picks where to save, starting from a file name; `None` when the user cancelled.
pub type PickSave = Box<dyn Fn(&str, &[Filter]) -> Option<String>>;

/// What the host provides: native file pickers and where the recent-files list is kept. The
/// desktop app installs them; tests answer the pickers with closures.
#[derive(Default)]
pub struct Services {
    pub pick_open: Option<PickOpen>,
    pub pick_save: Option<PickSave>,
    /// The recent-files list's file (JSON); `None` keeps the list for this run only.
    pub recent_path: Option<PathBuf>,
}

const MODEL: Filter = Filter { name: "ModelCraft model", extensions: &["modelcraft"] };
const DESIGN: Filter = Filter { name: "SolidCraft design", extensions: &["solidcraft"] };
const MESH: Filter = Filter { name: "Face-tagged mesh (3MF)", extensions: &["3mf", "3MF"] };
const ANY: Filter = Filter { name: "Models, designs and tagged meshes", extensions: &["modelcraft", "solidcraft", "3mf", "3MF"] };

/// The Open picker's filters: everything ModelCraft opens, then each type.
pub const OPEN_FILTERS: &[Filter] = &[ANY, MODEL, DESIGN, MESH];

/// What opening a file does with it, from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A `.modelcraft` model replaces the session (`file.open`).
    Model,
    /// A `.solidcraft` design becomes the geometry (`geometry.open`).
    Design,
    /// A `.3mf` face-tagged mesh is added as a surface (`geometry.import.mesh`).
    Mesh,
}

impl Kind {
    /// The kind of a file by its extension (any case); `None` for a file ModelCraft does not open.
    pub fn of(path: &str) -> Option<Kind> {
        let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "modelcraft" => Some(Kind::Model),
            "solidcraft" => Some(Kind::Design),
            "3mf" => Some(Kind::Mesh),
            _ => None,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Kind::Model => "file.open",
            Kind::Design => "geometry.open",
            Kind::Mesh => "geometry.import.mesh",
        }
    }

    /// The picker's filters when the user asked for this kind (its own type first).
    fn filters(self) -> &'static [Filter] {
        match self {
            Kind::Model => &[MODEL, ANY],
            Kind::Design => &[DESIGN, ANY],
            Kind::Mesh => &[MESH, ANY],
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Kind::Model | Kind::Design => "Opening",
            Kind::Mesh => "Importing",
        }
    }
}

/// The file a command opens, if it is one the File menu handles with a picker.
fn kind_of_command(command: &str) -> Option<Kind> {
    [Kind::Model, Kind::Design, Kind::Mesh].into_iter().find(|k| k.command() == command)
}

/// Most entries the recent-files list keeps, and the longest path it keeps.
pub const MAX_RECENT: usize = 10;
const MAX_RECENT_PATH: usize = 4096;
/// Largest recent-files file read.
const MAX_RECENT_BYTES: u64 = 256 * 1024;

/// Recently opened and saved files, newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recent {
    pub paths: Vec<String>,
    file: Option<PathBuf>,
}

impl Recent {
    /// The list kept in `file` (an unreadable or malformed file gives an empty list).
    pub fn load(file: Option<PathBuf>) -> Recent {
        let paths = file
            .as_ref()
            .filter(|f| std::fs::metadata(f).is_ok_and(|m| m.is_file() && m.len() <= MAX_RECENT_BYTES))
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|p| !p.is_empty() && p.len() <= MAX_RECENT_PATH && Kind::of(p).is_some())
            .take(MAX_RECENT)
            .collect();
        Recent { paths, file }
    }

    /// Put `path` first and save the list.
    pub fn add(&mut self, path: &str) {
        if path.is_empty() || path.len() > MAX_RECENT_PATH {
            return;
        }
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_string());
        self.paths.truncate(MAX_RECENT);
        self.save();
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.save();
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(file, serde_json::to_vec_pretty(&self.paths).unwrap_or_default())
        };
        if let Err(e) = write() {
            log::warn!("recent files {}: {e}", file.display());
        }
    }
}

/// A command running in the background on a copy of the session: a file opening, a solve.
pub struct Loading {
    /// What the window shows while it runs ("Opening beam.modelcraft").
    pub title: String,
    /// The status when it succeeded ("Opened beam.modelcraft").
    done: String,
    /// The file it opens, for the recent list.
    recent: Option<String>,
    /// The finished session copy and the command's reply.
    rx: Receiver<(Box<Session>, Result<Value, String>)>,
}

/// What waits for an answer about unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Replace {
    /// A `.modelcraft` file.
    File(String),
    /// A sample (`file.sample`).
    Sample(&'static str),
}

impl Replace {
    fn name(&self) -> String {
        match self {
            Replace::File(p) => file_name(p),
            Replace::Sample(n) => modelcraft_engine::sample::find(n).map_or_else(|| (*n).to_string(), |s| format!("the {} sample", s.label)),
        }
    }
}

/// A question the window asks before going on.
#[derive(Clone, Debug, PartialEq)]
pub enum Prompt {
    /// Opening a model or a sample discards the current one's unsaved changes.
    Unsaved(Replace),
    /// Autosaved models to recover (`file.recovery.list`).
    Recover,
    /// The samples to open (`file.sample`).
    Samples,
}

/// The answer to [`Prompt::Unsaved`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

/// The file name a model is saved under by default.
fn model_name(session: &Session) -> String {
    session.path.as_deref().and_then(|p| Path::new(p).file_stem()).and_then(|s| s.to_str()).map_or_else(|| "Untitled".to_string(), str::to_string)
}

/// `path` with `ext` added when it has no extension of that name (some pickers leave it off).
fn with_extension(path: String, ext: &str) -> String {
    let has = Path::new(&path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(ext));
    if has { path } else { format!("{path}.{ext}") }
}

/// Run `command` on `session` off the window thread (inline on the web, which has no threads).
fn spawn(session: Session, command: &'static str, params: Value) -> Receiver<(Box<Session>, Result<Value, String>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let work = move || {
        let mut s = session;
        let r = s.execute(command, &params).map_err(|e| e.to_string());
        let _ = tx.send((Box::new(s), r));
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(e) = std::thread::Builder::new().name(format!("modelcraft {command}")).spawn(work) {
        // The sender went with the closure: the job reports that it could not start.
        log::warn!("{command}: no worker thread: {e}");
    }
    #[cfg(target_arch = "wasm32")]
    work();
    rx
}

impl ModelApp {
    /// The model's name: its file's stem, or "Untitled".
    pub fn model_name(&self) -> String {
        model_name(&self.session)
    }

    /// What is running in the background ("Opening beam.modelcraft", "Solving"), if anything.
    pub fn busy(&self) -> Option<&str> {
        self.loading.as_ref().map(|l| l.title.as_str())
    }

    /// Run `command` in the background on a copy of the session; the copy replaces the session
    /// when it succeeds. One at a time.
    fn begin_job(&mut self, command: &'static str, params: Value, title: String, done: String, recent: Option<String>) -> bool {
        if let Some(busy) = self.busy() {
            let e = format!("{busy}: wait for it or cancel it");
            self.set_status(e, true);
            return false;
        }
        let rx = spawn(self.session.clone(), command, params);
        self.set_status(format!("{title}…"), false);
        self.loading = Some(Loading { title, done, recent, rx });
        true
    }

    /// File > Open: pick a model, design or tagged mesh and open it. `None` asks for any of them.
    /// Without a picker service the command's form opens instead.
    pub fn open_dialog(&mut self, kind: Option<Kind>) {
        let Some(pick) = self.services.pick_open.as_ref() else {
            self.open_form(kind.unwrap_or(Kind::Model).command(), json!({}));
            return;
        };
        if let Some(path) = pick(kind.map_or(OPEN_FILTERS, Kind::filters)) {
            self.open_path(&path);
        }
    }

    /// Whether replacing the model would lose work: unsaved changes made by commands.
    fn has_unsaved_work(&self) -> bool {
        self.session.is_dirty() && !self.session.journal.is_empty()
    }

    /// Open a file the user chose (picked, dropped or from the recent list) by its type. A model
    /// replacing unsaved changes asks first.
    pub fn open_path(&mut self, path: &str) {
        let Some(kind) = Kind::of(path) else {
            self.set_status(format!("{path}: ModelCraft opens .modelcraft models, .solidcraft designs and .3mf tagged meshes"), true);
            return;
        };
        if kind == Kind::Model && self.has_unsaved_work() {
            self.prompt = Some(Prompt::Unsaved(Replace::File(path.to_string())));
            return;
        }
        self.load(kind, path.to_string());
    }

    /// Open `path` in the background; files dropped while another opens wait their turn.
    fn load(&mut self, kind: Kind, path: String) {
        if self.loading.is_some() {
            self.queued.push_back(path);
            return;
        }
        let name = file_name(&path);
        let done = match kind {
            Kind::Mesh => format!("Imported {name}"),
            Kind::Model | Kind::Design => format!("Opened {name}"),
        };
        self.begin_job(kind.command(), json!({"path": path}), format!("{} {name}", kind.verb()), done, Some(path));
    }

    /// Help ▸ Samples (and the empty viewport): open a sample model, asking first when that
    /// would discard unsaved changes.
    pub fn open_sample(&mut self, name: &str) {
        let Some(sample) = modelcraft_engine::sample::find(name) else {
            self.set_status(format!("no sample `{name}`"), true);
            return;
        };
        if self.has_unsaved_work() {
            self.prompt = Some(Prompt::Unsaved(Replace::Sample(sample.name)));
            return;
        }
        self.load_sample(sample.name);
    }

    fn load_sample(&mut self, name: &str) {
        let Some(sample) = modelcraft_engine::sample::find(name) else { return };
        if self.run("file.sample", json!({"name": sample.name})).is_ok() {
            self.model_changed();
            self.fit_view();
            let next = if self.session.model.steps().is_empty() { "" } else { " — Solve shows its results" };
            self.set_status(format!("Opened the {} sample{next}", sample.label), false);
        }
    }

    /// Solve every step (`solve.run`) in the background; the results come up when it is done.
    pub fn solve_all(&mut self) {
        if self.session.model.steps().is_empty() {
            self.set_status("Nothing to solve: the model has no steps (Steps ▸ Step, or open a sample from Help ▸ Samples)", true);
            return;
        }
        let n = self.session.model.steps().len();
        let done = format!("Solved {n} step{}", if n == 1 { "" } else { "s" });
        self.begin_job("solve.run", json!({}), "Solving".into(), done, None);
    }

    /// Recover an autosaved model (`file.recover`) in the background.
    pub fn recover(&mut self, id: &str) {
        self.prompt = None;
        self.begin_job("file.recover", json!({"id": id}), "Recovering the autosaved model".into(), "Recovered the autosaved model".into(), None);
    }

    /// Stop waiting for the background job; the model stays as it was.
    pub fn cancel_loading(&mut self) {
        if let Some(l) = self.loading.take() {
            self.queued.clear();
            self.set_status(format!("Cancelled: {}", l.title), false);
        }
    }

    /// Take in a finished background job (called every frame); `true` while one is running.
    pub fn poll_loading(&mut self) -> bool {
        let Some(l) = &self.loading else { return false };
        let outcome = match l.rx.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => None,
        };
        let Some(l) = self.loading.take() else { return false };
        match outcome {
            Some((session, Ok(reply))) => {
                let replaced = l.recent.is_some() || l.title.starts_with("Recovering");
                self.session = *session;
                self.model_changed();
                if replaced {
                    self.fit_view();
                }
                if let Some(p) = &l.recent {
                    self.recent.add(p);
                }
                let warnings: Vec<&str> = reply.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                if warnings.is_empty() {
                    self.set_status(l.done, false);
                } else {
                    self.set_status(format!("{} — {}", l.done, warnings.join("; ")), true);
                }
            }
            Some((_, Err(e))) => {
                let what = l.recent.as_deref().map_or_else(|| l.title.clone(), file_name);
                self.set_status(format!("{what}: {e}"), true);
            }
            None => self.set_status(format!("{}: stopped unexpectedly", l.title), true),
        }
        if let Some(next) = self.queued.pop_front() {
            self.open_path(&next);
        }
        self.loading.is_some()
    }

    /// Block until the background open finishes (tests and the offscreen harness).
    pub fn wait_loaded(&mut self) {
        for _ in 0..6000 {
            if !self.poll_loading() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// File > Save: to the model's file, or Save As when it has none. Returns whether it saved.
    pub fn save(&mut self) -> bool {
        if self.session.path.is_none() {
            return self.save_as();
        }
        let saved = self.run("file.save", json!({})).is_ok();
        if saved && let Some(p) = self.session.path.clone() {
            self.recent.add(&p);
        }
        saved
    }

    /// File > Save As: pick a new file for the model. Without a picker the command's form opens.
    pub fn save_as(&mut self) -> bool {
        let Some(pick) = self.services.pick_save.as_ref() else {
            self.open_form("file.save_as", json!({}));
            return false;
        };
        let Some(path) = pick(&format!("{}.modelcraft", self.model_name()), &[MODEL]) else { return false };
        let path = with_extension(path, "modelcraft");
        let saved = self.run("file.save_as", json!({"path": path})).is_ok();
        if saved && let Some(p) = self.session.path.clone() {
            self.recent.add(&p);
        }
        saved
    }

    /// Write Deck: pick where to write the model as a `format` deck. Without a picker the
    /// command's form opens with the format filled in.
    pub fn write_deck(&mut self, format: &str) {
        let Some(f) = modelcraft_decks::format(format) else {
            self.set_status(format!("no {format} deck writer yet"), true);
            return;
        };
        let Some(pick) = self.services.pick_save.as_ref() else {
            self.open_form("deck.write", json!({"format": format}));
            return;
        };
        let exts: &'static [&'static str] = match f.extension {
            "inp" => &["inp"],
            "bdf" => &["bdf", "dat", "nas"],
            "k" => &["k", "key", "dyn"],
            _ => &[],
        };
        let filter = Filter { name: "Solver deck", extensions: exts };
        let Some(path) = pick(&format!("{}.{}", self.model_name(), f.extension), &[filter]) else { return };
        let path = if exts.iter().any(|e| Path::new(&path).extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(e))) {
            path
        } else {
            with_extension(path, f.extension)
        };
        let _ = self.run("deck.write", json!({"format": format, "path": path}));
    }

    /// The autosaved models that can be recovered (`file.recovery.list`), newest first.
    pub fn recoverable(&mut self) -> Vec<Value> {
        self.session.execute("file.recovery.list", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
    }

    /// Answer the unsaved-changes prompt.
    pub fn answer_unsaved(&mut self, answer: Answer) {
        let Some(Prompt::Unsaved(next)) = self.prompt.clone() else { return };
        self.prompt = None;
        let go = match answer {
            Answer::Save => self.save(),
            Answer::Discard => true,
            Answer::Cancel => false,
        };
        if !go {
            return;
        }
        match next {
            Replace::File(path) => {
                if let Some(kind) = Kind::of(&path) {
                    self.load(kind, path);
                }
            }
            Replace::Sample(name) => self.load_sample(name),
        }
    }

    /// What a click on a file entry does, when it is one: open, save, write a deck or recover
    /// through the host's pickers. `false` for every other command.
    pub(crate) fn file_action(&mut self, command: &str, preset: &Value) -> bool {
        match command {
            "file.save" => {
                self.save();
            }
            "file.save_as" => {
                self.save_as();
            }
            "file.recover" | "file.recovery.list" => self.prompt = Some(Prompt::Recover),
            "file.sample" => self.prompt = Some(Prompt::Samples),
            "solve.run" => self.solve_all(),
            "deck.write" if self.services.pick_save.is_some() => match preset.get("format").and_then(Value::as_str) {
                Some(f) => self.write_deck(f),
                None => return false,
            },
            other => match kind_of_command(other) {
                Some(kind) if self.services.pick_open.is_some() => self.open_dialog(Some(kind)),
                _ => return false,
            },
        }
        true
    }

    /// Files dropped on the window open in turn; a hovered drop shows where it goes.
    pub(crate) fn dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<String> = ctx
            .input(|i| i.raw.dropped_files.iter().filter_map(|f| Some(f.path().to_string_lossy().to_string()).filter(|p| !p.is_empty())).collect());
        // A model first: it replaces the session, then designs and meshes go into it.
        let (models, rest): (Vec<String>, Vec<String>) = dropped.into_iter().partition(|p| Kind::of(p) == Some(Kind::Model));
        for p in models.into_iter().take(1).chain(rest) {
            self.open_path(&p);
        }
    }
}

/// The last part of a path, for messages.
pub fn file_name(path: &str) -> String {
    Path::new(path).file_name().and_then(|n| n.to_str()).map_or_else(|| path.to_string(), str::to_string)
}

/// The prompts (unsaved changes, recover), the loading overlay and the drop hint.
pub fn show(app: &mut ModelApp, ctx: &egui::Context) {
    let t = crate::theme::Tokens::get(app.ui.dark);
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if hovering || app.loading.is_some() {
        let text = match app.loading.as_ref() {
            Some(l) => format!("{}…", l.title),
            None => "Drop to open: .modelcraft models, .solidcraft designs, .3mf tagged meshes".into(),
        };
        let mut cancel = false;
        egui::Area::new(egui::Id::new("mc_loading")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(
            ctx,
            |ui| {
                egui::Frame::popup(ui.style()).fill(t.overlay).inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if app.loading.is_some() {
                            ui.spinner();
                        }
                        ui.label(egui::RichText::new(text).color(t.overlay_text));
                        if app.loading.is_some() {
                            cancel = ui.button("Cancel").clicked();
                        }
                    });
                });
            },
        );
        if cancel {
            app.cancel_loading();
        }
        if app.loading.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    match app.prompt.clone() {
        Some(Prompt::Unsaved(next)) => {
            let mut answer = None;
            crate::frame::window(ctx, "Unsaved changes", crate::frame::Width::Normal).id(egui::Id::new("mc_unsaved")).show(ctx, |ui| {
                ui.label(format!("Save the changes to {} before opening {}?", app.model_name(), next.name()));
                ui.label(egui::RichText::new("Without saving, they are lost.").color(t.text_dim));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        answer = Some(Answer::Save);
                    }
                    if ui.button("Don't Save").clicked() {
                        answer = Some(Answer::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(Answer::Cancel);
                    }
                });
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer = Some(Answer::Cancel);
            }
            if let Some(a) = answer {
                app.answer_unsaved(a);
            }
        }
        Some(Prompt::Recover) => {
            let entries = app.recover_list(ctx);
            let (mut recover, mut discard, mut close) = (None, None, false);
            crate::frame::window(ctx, "Recover", crate::frame::Width::Wide).id(egui::Id::new("mc_recover")).show(ctx, |ui| {
                if entries.is_empty() {
                    ui.label(egui::RichText::new("No autosaved models to recover.").color(t.text_dim));
                }
                egui::Grid::new("mc_recover_grid").num_columns(3).spacing(egui::vec2(10.0, 4.0)).show(ui, |ui| {
                    for e in &entries {
                        let id = e.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                        let name = e.get("path").and_then(Value::as_str).map_or_else(|| "Untitled".to_string(), file_name);
                        let n = |k: &str| e.get(k).and_then(Value::as_u64).unwrap_or(0);
                        ui.label(name);
                        ui.label(egui::RichText::new(format!("{} nodes, {} elements", n("nodes"), n("elements"))).color(t.text_dim));
                        ui.horizontal(|ui| {
                            if ui.button("Recover").clicked() {
                                recover = Some(id.clone());
                            }
                            if ui.button("Discard").clicked() {
                                discard = Some(id.clone());
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                close = ui.button("Close").clicked();
            });
            if let Some(id) = recover {
                app.recover(&id);
            } else if let Some(id) = discard {
                let _ = app.run("file.recovery.discard", json!({"id": id}));
                ctx.data_mut(|d| d.remove::<Vec<Value>>(egui::Id::new("mc_recover_entries")));
            } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.prompt = None;
                ctx.data_mut(|d| d.remove::<Vec<Value>>(egui::Id::new("mc_recover_entries")));
            }
        }
        Some(Prompt::Samples) => {
            let (mut open, mut close) = (None, false);
            crate::frame::window(ctx, "Samples", crate::frame::Width::Normal).id(egui::Id::new("mc_samples")).show(ctx, |ui| {
                sample_rows(ui, &t, &mut open);
                ui.add_space(6.0);
                close = ui.button("Close").clicked();
            });
            if let Some(name) = open {
                app.prompt = None;
                app.open_sample(name);
            } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.prompt = None;
            }
        }
        None => {}
    }
}

/// One row per sample: its name, what it shows and an Open button; `open` gets the one clicked.
pub fn sample_rows(ui: &mut egui::Ui, t: &crate::theme::Tokens, open: &mut Option<&'static str>) {
    for sample in modelcraft_engine::sample::SAMPLES {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(sample.label).strong());
                ui.label(egui::RichText::new(sample.about).small().color(t.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Open").clicked() {
                    *open = Some(sample.name);
                }
            });
        });
        ui.add_space(4.0);
    }
}

impl ModelApp {
    /// The recover window's entries, read from disk once while it is open.
    fn recover_list(&mut self, ctx: &egui::Context) -> Vec<Value> {
        let key = egui::Id::new("mc_recover_entries");
        if let Some(v) = ctx.data(|d| d.get_temp::<Vec<Value>>(key)) {
            return v;
        }
        let v = self.recoverable();
        ctx.data_mut(|d| d.insert_temp(key, v.clone()));
        v
    }
}

/// The File menu's rows: open, save, decks, recover, recent files. Shared by the title bar's
/// File menu (and anything else that lists them).
pub fn file_menu(app: &mut ModelApp, ui: &mut egui::Ui) {
    let cmd = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
    if ui
        .add(egui::Button::new("Open…").shortcut_text(format!("{cmd}+O")))
        .on_hover_text("Open a model, a SolidCraft design or a tagged mesh")
        .clicked()
    {
        ui.close();
        app.open_dialog(None);
    }
    ui.menu_button("Open Recent", |ui| {
        if app.recent.paths.is_empty() {
            ui.label(egui::RichText::new("No recent files").weak());
        }
        let mut open = None;
        for p in &app.recent.paths {
            if ui.button(file_name(p)).on_hover_text(p).clicked() {
                open = Some(p.clone());
            }
        }
        if !app.recent.paths.is_empty() {
            ui.separator();
            if ui.button("Clear Recent").clicked() {
                app.recent.clear();
                ui.close();
            }
        }
        if let Some(p) = open {
            ui.close();
            app.open_path(&p);
        }
    });
    ui.separator();
    if ui.add(egui::Button::new("Save").shortcut_text(format!("{cmd}+S"))).clicked() {
        ui.close();
        app.save();
    }
    if ui.add(egui::Button::new("Save As…").shortcut_text(format!("{cmd}+Shift+S"))).clicked() {
        ui.close();
        app.save_as();
    }
    ui.separator();
    ui.menu_button("Write Deck", |ui| {
        for c in modelcraft_engine::catalogue::CATALOGUE.iter().filter(|c| c.panel == "Decks") {
            let format = c.preset().get("format").and_then(Value::as_str).map(str::to_string);
            match (format, c.milestone()) {
                (Some(f), None) => {
                    if ui.button(format!("{}…", c.label)).on_hover_text(c.tip).clicked() {
                        ui.close();
                        app.write_deck(&f);
                    }
                }
                (_, m) => {
                    ui.add_enabled(false, egui::Button::new(format!("{} — coming soon ({})", c.label, m.unwrap_or("later"))));
                }
            }
        }
    });
    if ui.button("Recover…").on_hover_text("Models autosaved by an app that crashed or closed with unsaved changes").clicked() {
        ui.close();
        app.prompt = Some(Prompt::Recover);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("modelcraft-files-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A picker that answers `path` and records the filters it was offered.
    fn answering(path: Option<String>, offered: Rc<RefCell<Vec<Vec<Filter>>>>) -> PickOpen {
        Box::new(move |f: &[Filter]| {
            offered.borrow_mut().push(f.to_vec());
            path.clone()
        })
    }

    #[test]
    fn file_types_dispatch_to_their_command() {
        assert_eq!(Kind::of("a/b.modelcraft"), Some(Kind::Model));
        assert_eq!(Kind::of("B.SOLIDCRAFT"), Some(Kind::Design));
        assert_eq!(Kind::of("part.3MF").map(Kind::command), Some("geometry.import.mesh"));
        assert_eq!(Kind::of("part.step"), None);
        assert_eq!(Kind::of("noext"), None);
        assert_eq!(with_extension("a/b".into(), "modelcraft"), "a/b.modelcraft");
        assert_eq!(with_extension("a/b.MODELCRAFT".into(), "modelcraft"), "a/b.MODELCRAFT");
    }

    /// Open (picker) → background open → the model, the recent list; Save As (picker) and Save.
    #[test]
    fn the_pickers_open_and_save_models() {
        let dir = temp("models");
        let saved = dir.join("beam.modelcraft").to_string_lossy().to_string();
        let mut first = ModelApp::new(Session::default());
        first.run_script(&modelcraft_engine::sample::script()).unwrap();
        // Save As through the save picker: the extension is added, the file becomes the model's.
        let bare = dir.join("beam").to_string_lossy().to_string();
        first.services.pick_save = Some(Box::new(move |name: &str, f: &[Filter]| {
            assert_eq!((name, f.first().map(|f| f.extensions)), ("Untitled.modelcraft", Some(&["modelcraft"][..])));
            Some(bare.clone())
        }));
        first.start("file.save_as").unwrap();
        assert_eq!(first.session.path.as_deref(), Some(saved.as_str()));
        assert!(!first.session.is_dirty());
        assert_eq!(first.recent.paths, vec![saved.clone()]);

        let offered = Rc::new(RefCell::new(Vec::new()));
        let mut app = ModelApp::new(Session::default());
        app.services.recent_path = None;
        app.services.pick_open = Some(answering(Some(saved.clone()), offered.clone()));
        app.start("file.open").unwrap();
        assert!(app.dialog.is_none(), "a picker, not the form");
        assert_eq!(offered.borrow()[0][0], MODEL, "Open Model offers models first");
        assert_eq!(app.busy(), Some("Opening beam.modelcraft"));
        app.wait_loaded();
        assert_eq!(app.session.model.elements().len(), first.session.model.elements().len());
        assert_eq!(app.session.path.as_deref(), Some(saved.as_str()));
        assert_eq!(app.recent.paths, vec![saved.clone()]);
        assert_eq!(app.status.as_ref().map(|s| (s.text.as_str(), s.error)), Some(("Opened beam.modelcraft", false)));
        // Save goes to the model's own file without asking.
        app.services.pick_save = Some(Box::new(|_: &str, _: &[Filter]| -> Option<String> { panic!("Save asked for a file") }));
        app.run("mesh.node.add", serde_json::json!({"label": 99999, "xyz": [0, 0, 0]})).unwrap();
        assert!(app.save());
        assert!(!app.session.is_dirty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelled_pickers_and_failed_opens_leave_the_model() {
        let mut app = ModelApp::new(Session::default());
        app.run("mesh.node.add", serde_json::json!({"label": 1, "xyz": [0, 0, 0]})).unwrap();
        let offered = Rc::new(RefCell::new(Vec::new()));
        app.services.pick_open = Some(answering(None, offered.clone()));
        app.open_dialog(None);
        assert_eq!(offered.borrow()[0], OPEN_FILTERS.to_vec(), "File > Open offers every type");
        assert!(app.loading.is_none());
        // A file that does not open: the model stays, the error is shown.
        let dir = temp("broken");
        let broken = dir.join("broken.solidcraft");
        std::fs::write(&broken, b"not a design").unwrap();
        app.open_path(&broken.to_string_lossy());
        app.wait_loaded();
        assert!(app.session.model.node(1).is_some());
        assert!(app.status.as_ref().is_some_and(|s| s.error && s.text.starts_with("broken.solidcraft:")), "{:?}", app.status);
        assert!(app.recent.paths.is_empty());
        app.open_path("drawing.dxf");
        assert!(app.status.as_ref().is_some_and(|s| s.error));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_model_over_unsaved_changes_asks_first() {
        let dir = temp("unsaved");
        let path = dir.join("other.modelcraft").to_string_lossy().to_string();
        let mut other = ModelApp::new(Session::default());
        other.run("mesh.node.add", serde_json::json!({"label": 5, "xyz": [0, 0, 0]})).unwrap();
        other.run("file.save_as", serde_json::json!({"path": path})).unwrap();
        let mut app = ModelApp::new(Session::default());
        app.run("mesh.node.add", serde_json::json!({"label": 1, "xyz": [0, 0, 0]})).unwrap();
        app.open_path(&path);
        assert_eq!(app.prompt, Some(Prompt::Unsaved(Replace::File(path.clone()))));
        assert!(app.loading.is_none());
        app.answer_unsaved(Answer::Cancel);
        assert!(app.prompt.is_none() && app.loading.is_none() && app.session.model.node(1).is_some());
        app.open_path(&path);
        app.answer_unsaved(Answer::Discard);
        app.wait_loaded();
        assert!(app.session.model.node(5).is_some() && app.session.model.node(1).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decks_are_written_where_the_picker_says() {
        let dir = temp("decks");
        let target = dir.join("cantilever").to_string_lossy().to_string();
        let mut app = ModelApp::new(Session::default());
        app.run_script(&modelcraft_engine::sample::cantilever()).unwrap();
        let t = target.clone();
        app.services.pick_save = Some(Box::new(move |name: &str, f: &[Filter]| {
            assert_eq!((name, f.first().map(|f| f.extensions)), ("Untitled.inp", Some(&["inp"][..])));
            Some(t.clone())
        }));
        app.start("deck.write.abaqus").unwrap();
        assert!(app.dialog.is_none());
        let text = std::fs::read_to_string(format!("{target}.inp")).unwrap();
        assert!(text.contains("*NODE") || text.contains("*Node"), "{}", &text[..text.len().min(200)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_recent_list_is_kept_newest_first_and_survives_bad_files() {
        let dir = temp("recent");
        let file = dir.join("recent.json");
        let mut r = Recent::load(Some(file.clone()));
        assert!(r.paths.is_empty());
        for i in 0..12 {
            r.add(&format!("/m/{i}.modelcraft"));
        }
        r.add("/m/3.modelcraft");
        let again = Recent::load(Some(file.clone()));
        assert_eq!(again.paths.len(), MAX_RECENT);
        assert_eq!(again.paths.first().map(String::as_str), Some("/m/3.modelcraft"));
        assert_eq!(again.paths.iter().filter(|p| *p == "/m/3.modelcraft").count(), 1);
        std::fs::write(&file, b"{not json").unwrap();
        assert!(Recent::load(Some(file.clone())).paths.is_empty());
        std::fs::write(&file, br#"["/x/a.modelcraft", "/x/b.exe", ""]"#).unwrap();
        assert_eq!(Recent::load(Some(file)).paths, vec!["/x/a.modelcraft".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A sample opens with nothing to save; Solve runs its steps in the background and the
    /// Results tab comes up with the first step's result.
    #[test]
    fn a_sample_solves_into_the_results_view() {
        let mut app = ModelApp::new(Session::default());
        app.start("file.sample").unwrap();
        assert_eq!(app.prompt, Some(Prompt::Samples));
        app.prompt = None;
        app.open_sample("cantilever");
        assert!(!app.session.is_dirty() && !app.session.model.steps().is_empty());
        app.start("solve.run").unwrap();
        assert_eq!(app.busy(), Some("Solving"));
        assert!(app.run("model.info", serde_json::json!({})).is_err(), "commands wait for the solve");
        app.wait_loaded();
        assert_eq!(app.status.as_ref().map(|s| s.text.as_str()), Some("Solved 2 steps"));
        app.logic(&egui::Context::default());
        assert_eq!(app.ui.tab, modelcraft_engine::catalogue::Tab::Results);
        assert!(app.plot().is_some());
        // Over unsaved changes, opening a sample asks first.
        app.run("mesh.node.add", serde_json::json!({"label": 900000, "xyz": [0, 0, 0]})).unwrap();
        app.open_sample("impact");
        assert_eq!(app.prompt, Some(Prompt::Unsaved(Replace::Sample("impact"))));
        app.answer_unsaved(Answer::Discard);
        assert!(app.session.model.step("Impact").is_some());
        // A model without steps says so instead of solving nothing.
        let mut empty = ModelApp::new(Session::default());
        empty.solve_all();
        assert!(empty.busy().is_none() && empty.status.as_ref().is_some_and(|s| s.error));
    }

    /// The control channel's `ui.start` never opens a picker: it opens the form.
    #[test]
    fn programmatic_starts_never_open_a_picker() {
        let mut app = ModelApp::new(Session::default());
        app.services.pick_open = Some(Box::new(|_: &[Filter]| -> Option<String> { panic!("a picker opened") }));
        app.start_programmatic("file.open").unwrap();
        assert_eq!(app.dialog.as_ref().map(|d| d.command), Some("file.open"));
    }
}
