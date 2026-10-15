//! The item menu: what a right click on a browser row offers, and on a datum or a partition's cut
//! face in the viewport. Edit… opens the item's edit form, Hide/Show and Isolate change the view
//! (`view.*`), Active in step switches a boundary condition or load on or off per step
//! (`model.step.activate`), and Delete… asks first ([`crate::files::Prompt::Delete`]): it lists
//! the definitions that use the item, and refuses, or for a datum deletes its partitions with it.
//! Every entry runs the same command as the ribbon, the forms and scripts.

use fieldcraft_engine::{ItemKind, ViewItem};
use serde_json::{Value, json};

use crate::ModelApp;
use crate::browser::Def;

/// What the menu is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A body of the open design.
    Body(String),
    /// An imported surface.
    Surface(String),
    /// A model definition, datum or partition.
    Def(Def, String),
}

impl Target {
    /// The viewport item it is, when the viewport draws it on its own.
    pub fn item(&self) -> Option<ViewItem> {
        let (kind, name) = match self {
            Target::Body(n) => (ItemKind::Body, n),
            Target::Surface(n) => (ItemKind::Surface, n),
            Target::Def(def, n) => (def.item_kind()?, n),
        };
        Some(ViewItem { kind, name: name.clone() })
    }
}

/// A menu entry chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Edit,
    Hide,
    Show,
    Isolate,
    ShowAll,
    /// Switch a boundary condition or load on or off in a step.
    Active {
        step: String,
        active: bool,
    },
    Delete,
}

/// Whether `target` is active in each step (boundary conditions and loads; empty for the rest).
fn activity(app: &ModelApp, target: &Target) -> Vec<(String, bool)> {
    let steps = app.session.model.steps();
    match target {
        Target::Def(Def::BoundaryCondition, n) => steps.iter().map(|s| (s.name.clone(), s.boundary_conditions.contains(n))).collect(),
        Target::Def(Def::Load, n) => steps.iter().map(|s| (s.name.clone(), s.loads.contains(n))).collect(),
        _ => Vec::new(),
    }
}

/// Draw the menu's entries for `target`; the entry clicked.
pub fn show(app: &ModelApp, ui: &mut egui::Ui, target: &Target) -> Option<Action> {
    let mut chosen = None;
    let mut entry = |ui: &mut egui::Ui, enabled: bool, label: &str, tip: &str, action: Action| {
        if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip).clicked() {
            chosen = Some(action);
        }
    };
    if let Target::Def(def, name) = target {
        let edit = def.edit_command(&app.session.model, name);
        let tip = edit.clone().unwrap_or_else(|| "Not editable: delete it and make it again".to_string());
        entry(ui, edit.is_some(), "Edit…", &tip, Action::Edit);
    }
    if let Some(item) = target.item() {
        if app.session.shows(item.kind, &item.name) {
            entry(ui, true, "Hide", "view.hide", Action::Hide);
        } else {
            entry(ui, true, "Show", "view.show", Action::Show);
        }
        entry(ui, true, "Isolate", "view.isolate: show only this", Action::Isolate);
    }
    if !app.session.view.is_empty() {
        entry(ui, true, "Show All", "view.show_all", Action::ShowAll);
    }
    let steps = activity(app, target);
    if !steps.is_empty() {
        ui.menu_button("Active in step", |ui| {
            for (step, on) in steps {
                let mut checked = on;
                if ui.checkbox(&mut checked, &step).on_hover_text("model.step.activate").changed() {
                    chosen = Some(Action::Active { step, active: checked });
                    ui.close();
                }
            }
        });
    }
    if let Target::Def(def, _) = target {
        ui.separator();
        let command = def.command("delete");
        if ui.button("Delete…").on_hover_text(format!("{command} (Delete key)")).clicked() {
            chosen = Some(Action::Delete);
        }
    }
    if chosen.is_some() {
        ui.close();
    }
    chosen
}

/// Do what the menu entry chosen asks.
pub fn apply(app: &mut ModelApp, target: Target, action: Action) {
    let items = |t: &Target| json!({"items": t.item().into_iter().collect::<Vec<_>>()});
    // Errors show in the status bar.
    let _ = match action {
        Action::Edit => {
            if let Target::Def(def, name) = target
                && let Some((command, params)) = def.edit(&app.session.model, &name)
            {
                app.selected = Some((def, name));
                app.open_form(&command, params);
            }
            Ok(Value::Null)
        }
        Action::Hide => app.run("view.hide", items(&target)),
        Action::Show => app.run("view.show", items(&target)),
        Action::Isolate => app.run("view.isolate", items(&target)),
        Action::ShowAll => app.run("view.show_all", json!({})),
        Action::Active { step, active } => match target {
            Target::Def(def @ (Def::BoundaryCondition | Def::Load), name) => {
                let key = if def == Def::Load { "loads" } else { "boundary_conditions" };
                app.run("model.step.activate", json!({"step": step, key: [name], "active": active}))
            }
            _ => Ok(Value::Null),
        },
        Action::Delete => {
            if let Target::Def(def, name) = target {
                app.ask_delete(def, &name);
            }
            Ok(Value::Null)
        }
    };
}

/// The question a deletion asks ([`crate::files::Prompt::Delete`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteAsk {
    pub kind: Def,
    pub name: String,
    /// The definitions that use it.
    pub users: Vec<String>,
}

impl DeleteAsk {
    /// Whether deleting goes ahead with its users: a datum's partitions are deleted with it; any
    /// other definition in use is refused.
    pub fn cascades(&self) -> bool {
        self.kind == Def::Datum
    }

    /// Whether it can be deleted at all.
    pub fn allowed(&self) -> bool {
        self.users.is_empty() || self.cascades()
    }

    /// The question, in words.
    pub fn question(&self) -> String {
        let what = format!("{} `{}`", self.kind.label().to_lowercase(), self.name);
        match (self.users.is_empty(), self.cascades()) {
            (true, _) => format!("Delete {what}?"),
            (false, true) => format!("Delete {what} and what uses it: {}?", self.users.join(", ")),
            (false, false) => format!("{what} is used by {}: change or delete those first.", self.users.join(", ")),
        }
    }
}

impl ModelApp {
    /// Ask before deleting definition `name` of `kind` (the item menu's Delete…, the Delete key).
    pub fn ask_delete(&mut self, kind: Def, name: &str) {
        if !kind.exists(&self.session.model, name) {
            return;
        }
        let users = kind.users(&self.session.model, name);
        self.prompt = Some(crate::files::Prompt::Delete(DeleteAsk { kind, name: name.to_string(), users }));
    }

    /// Answer [`crate::files::Prompt::Delete`]: delete (with its partitions, for a datum in use)
    /// or leave it.
    pub fn answer_delete(&mut self, delete: bool) {
        let Some(crate::files::Prompt::Delete(ask)) = self.prompt.clone() else { return };
        self.prompt = None;
        if !delete || !ask.allowed() {
            return;
        }
        let mut params = json!({"name": ask.name});
        if ask.cascades() && !ask.users.is_empty() {
            params["cascade"] = json!(true);
        }
        if self.run(&ask.kind.command("delete"), params).is_ok() && self.selected.as_ref() == Some(&(ask.kind, ask.name)) {
            self.selected = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use fieldcraft_engine::Session;
    use fieldcraft_render::StandardView;

    use super::*;
    use crate::viewport;

    /// The plate design open in an app with a datum plane `Mid` cutting it (partition `Half`)
    /// and a datum point `P`, framed in an 800 × 600 viewport.
    fn plate() -> (ModelApp, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("fieldcraft-ui-menu-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        let design = dir.join("plate.solidcraft");
        std::fs::write(&design, fieldcraft_geometry::sample::plate_design(&fieldcraft_geometry::sample::Plate::default()).unwrap()).unwrap();
        let mut app = ModelApp::new(Session::default());
        app.run("geometry.open", json!({"path": design.to_string_lossy()})).unwrap();
        app.run("geometry.datum", json!({"name": "Mid", "by": {"type": "offset", "plane": "yz", "offset": 15}})).unwrap();
        app.run("geometry.partition", json!({"name": "Half", "datum": "Mid"})).unwrap();
        app.run("geometry.datum.point", json!({"name": "P", "by": {"type": "coordinates", "point": [0, 0, 0]}})).unwrap();
        app.viewport.rect = Some(egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 600.0)));
        app.set_view(StandardView::Iso);
        app.fit_view();
        (app, dir)
    }

    /// A screen point on datum `name`'s drawn lines.
    fn on_datum(app: &mut ModelApp, name: &str) -> egui::Pos2 {
        let bounds = app.display().bounds;
        let mut s = fieldcraft_render::Scene::default();
        fieldcraft_render::glyphs::datums(&app.session.model, &bounds, &|n| n == name, &crate::theme::Tokens::dark().glyph_colors(), &mut s);
        let l = s.lines[0];
        viewport::to_screen(app, (l.a + l.b) * 0.5).unwrap()
    }

    /// The menu's entries run the commands the ribbon, forms and scripts run; hiding is view
    /// state (nothing journalled) and a hidden item is not picked.
    #[test]
    fn menu_entries_run_the_same_commands_and_hidden_items_are_not_picked() {
        let (mut app, dir) = plate();
        let mid = Target::Def(Def::Datum, "Mid".into());
        let at = on_datum(&mut app, "Mid");
        assert_eq!(viewport::menu_target(&mut app, at), Some(mid.clone()));
        let journal = app.session.journal.len();
        apply(&mut app, mid.clone(), Action::Hide);
        assert!(!app.session.shows(ItemKind::Datum, "Mid") && app.session.is_dirty());
        assert_eq!(app.session.journal.len(), journal);
        assert_ne!(viewport::menu_target(&mut app, at), Some(mid.clone()), "a hidden datum is not picked");
        apply(&mut app, mid.clone(), Action::Show);
        assert!(app.session.shows(ItemKind::Datum, "Mid"));

        // A hidden body is neither drawn nor picked.
        let cell = app.session.design.as_ref().unwrap().bodies[0].name.clone();
        let centre = app.session.design.as_ref().unwrap().bodies[0].measure().unwrap().centroid;
        let top = viewport::to_screen(&app, fieldcraft_render::Vec3::new(centre[0], centre[1], 10.0)).unwrap();
        assert!(app.pick_at(top).is_some());
        apply(&mut app, Target::Body(cell.clone()), Action::Isolate);
        assert!(app.session.shows(ItemKind::Body, &cell) && !app.session.shows(ItemKind::Datum, "P"));
        apply(&mut app, Target::Body(cell.clone()), Action::Hide);
        app.display();
        assert!(app.pick.names.iter().all(|n| n.name != cell));
        assert!(app.pick_at(top).is_none_or(|i| !i.describe().contains(cell.as_str())));
        apply(&mut app, Target::Body(cell), Action::ShowAll);
        assert!(app.session.view.is_empty());

        // Edit… opens the datum's edit form filled in; running it is the edit command.
        apply(&mut app, Target::Def(Def::Datum, "P".into()), Action::Edit);
        let d = app.dialog.as_mut().unwrap();
        assert_eq!(d.command, "geometry.datum.point.edit");
        assert!(d.text.contains("[0.0,0.0,0.0]"), "{}", d.text);
        d.text = json!({"name": "P", "by": {"type": "coordinates", "point": [1, 0, 0]}}).to_string();
        app.run_dialog().unwrap();
        assert_eq!(app.session.journal.last().map(|(c, _)| c.as_str()), Some("geometry.datum.point.edit"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Delete asks first: a datum a partition uses names it and deletes both on confirmation;
    /// a definition in use elsewhere is refused.
    #[test]
    fn deleting_lists_what_uses_it_and_cascades_or_refuses() {
        let (mut app, dir) = plate();
        apply(&mut app, Target::Def(Def::Datum, "Mid".into()), Action::Delete);
        let Some(crate::files::Prompt::Delete(ask)) = app.prompt.clone() else { panic!("{:?}", app.prompt) };
        assert_eq!(ask.users, ["partition `Half`"]);
        assert!(ask.allowed() && ask.question().contains("partition `Half`"));
        // Cancel changes nothing; Delete deletes the datum and its partition in one step.
        app.answer_delete(false);
        assert!(app.prompt.is_none() && app.session.model.datum("Mid").is_some());
        app.selected = Some((Def::Datum, "Mid".into()));
        app.ask_delete(Def::Datum, "Mid");
        app.answer_delete(true);
        assert_eq!(app.session.journal.last(), Some(&("geometry.datum.delete".to_string(), json!({"name": "Mid", "cascade": true}))));
        assert!(app.session.model.partitions().is_empty() && app.selected.is_none());

        // A material a section uses is refused: no Delete.
        let mut app = ModelApp::new(Session::default());
        app.open_sample("cantilever");
        app.ask_delete(Def::Material, "Steel");
        let Some(crate::files::Prompt::Delete(ask)) = app.prompt.clone() else { panic!() };
        assert!(!ask.allowed() && !ask.users.is_empty());
        let journal = app.session.journal.len();
        app.answer_delete(true);
        assert!(app.session.model.materials().contains_key("Steel"));
        assert_eq!(app.session.journal.len(), journal);
        // Active in step switches a boundary condition per step.
        let bc = app.session.model.boundary_conditions().keys().next().unwrap().clone();
        let step = app.session.model.steps()[0].name.clone();
        apply(&mut app, Target::Def(Def::BoundaryCondition, bc.clone()), Action::Active { step: step.clone(), active: false });
        assert!(!app.session.model.step(&step).unwrap().boundary_conditions.contains(&bc));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
