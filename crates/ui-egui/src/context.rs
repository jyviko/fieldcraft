//! The context panel (right): shown only while there is context, as in the other Craft apps.
//! From the top: the open command's form, the properties of the definition selected in the
//! browser (with Edit and Delete), the results controls while results are shown, and last the
//! reply of a report (Check Model, Model Info), so what the person is working on stays on top. Each section has
//! a heading band; everything acts through commands.

use egui::{RichText, vec2};
use serde_json::{Value, json};

use crate::browser::Def;
use crate::frame::{Level, heading};
use crate::theme::{Tokens, label_font};
use crate::{ModelApp, selection};

/// The panel's width.
pub const WIDTH: f32 = 310.0;

/// Whether the panel has anything to show.
pub fn has_context(app: &mut ModelApp) -> bool {
    app.dialog.is_some() || app.selected.is_some() || app.reply.is_some() || app.plot().is_some()
}

pub fn panel(app: &mut ModelApp, ui: &mut egui::Ui) {
    if !has_context(app) {
        return;
    }
    let t = Tokens::get(app.ui.dark);
    egui::Panel::right("fc_context")
        .exact_size(WIDTH)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.dialog_bg).inner_margin(egui::Margin::symmetric(8, 8)))
        .show(ui, |ui| {
            app.place("context", ui.max_rect());
            egui::ScrollArea::vertical().id_salt("fc_context_scroll").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(WIDTH - 16.0);
                if app.dialog.is_some() {
                    form(app, ui, &t);
                    ui.add_space(10.0);
                }
                if let Some((kind, name)) = app.selected.clone() {
                    properties(app, ui, &t, kind, &name);
                    ui.add_space(10.0);
                }
                if let Some(plot) = app.plot() {
                    let mut close = false;
                    heading(ui, &t, Level::Title, |ui| {
                        ui.label(RichText::new("RESULTS").font(label_font(ui.ctx(), 12.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close = ui.small_button("Close").on_hover_text("Back to the model (results.clear)").clicked();
                        });
                    });
                    ui.add_space(4.0);
                    crate::results_panel::controls(app, &plot, ui, &t);
                    if close {
                        let _ = app.run("results.clear", json!({}));
                    }
                    ui.add_space(10.0);
                }
                if app.reply.is_some() {
                    crate::reply::show(app, ui, &t);
                }
            });
        });
}

/// JSON shorter than this is shown pretty-printed in the JSON view.
const MAX_PRETTY: usize = 100_000;

/// The open command's form: typed fields drawn from its schema (or its JSON text, the Advanced
/// view), with the command's parameter documentation; OK runs it.
/// A command that applies to part of the model (a set, a boundary condition, a load, a section, a
/// coupling) has a Target row that takes the viewport's selection.
fn form(app: &mut ModelApp, ui: &mut egui::Ui, t: &Tokens) {
    let selected = (!app.pick.selection.is_empty()).then(|| app.pick.selection.summary());
    let units = app.session.model.units();
    let session = &app.session;
    // The set Use selection will make on OK is offered with the others.
    let pending = app.dialog.as_ref().and_then(|d| d.pending_set.as_ref()).and_then(|s| s.get("name")).and_then(Value::as_str).map(str::to_string);
    let mut use_selection = false;
    let Some(d) = app.dialog.as_mut() else { return };
    let Some(spec) = fieldcraft_engine::find_command(d.command) else {
        app.dialog = None;
        return;
    };
    // The form's widgets are its own, keyed by this opening: the keyboard focus and typed text of
    // the form it replaces (egui drops focus from a widget no longer drawn) never land here.
    let form_id = egui::Id::new(("fc_form", spec.id, d.opening));
    let (mut run, mut cancel) = (false, false);
    heading(ui, t, Level::Title, |ui| {
        ui.label(RichText::new(spec.label.to_uppercase()).font(label_font(ui.ctx(), 12.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel = ui.small_button("×").on_hover_text("Cancel (Esc)").clicked();
        });
    });
    ui.add_space(4.0);
    ui.label(RichText::new(spec.id).small().color(t.text_dim));
    let params = serde_json::from_str::<Value>(&d.text).unwrap_or(Value::Null);
    if let Some(target) = selection::target(spec.id, &params) {
        heading(ui, t, Level::Section, |ui| {
            ui.label(RichText::new("Target").size(12.0));
        });
        ui.horizontal(|ui| {
            let tip = match target {
                selection::Target::Members => "The selection becomes the set's members (geometry by name, the mesh by label)",
                selection::Target::Set | selection::Target::TargetSet => {
                    "Names a set of the selection here: an existing set with the same members, or a new one made on OK (model.set.create)"
                }
                selection::Target::Edges => "The selected design edges (Select: Edges) become the edges seeded",
            };
            use_selection = ui.add_enabled(selected.is_some(), egui::Button::new("Use selection")).on_hover_text(tip).clicked();
            match &selected {
                Some(text) => ui.add(egui::Label::new(RichText::new(text).color(t.pick_selected)).wrap()),
                None => ui.add(egui::Label::new(RichText::new("Pick in the viewport, or choose below").color(t.text_dim)).wrap()),
            };
        });
    }
    let mut advanced = d.advanced;
    heading(ui, t, Level::Section, |ui| {
        ui.label(RichText::new("Parameters").size(12.0));
        if spec.form.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut advanced, RichText::new("{ } JSON").small()).on_hover_text("Advanced: edit the parameters as JSON");
            });
        }
    });
    let mut invalid = 0;
    let mut create = None;
    // What is missing or wrong, by the fields' labels: OK waits for it (in either view).
    let problems = match (spec.form, params.as_object()) {
        (Some(fields), Some(obj)) => crate::form::messages(fields, obj),
        _ => Vec::new(),
    };
    let missing = problems.iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>().join("\n");
    // The typed fields edit the JSON text's object; text that is not an object stays in the
    // JSON view until it is.
    let object = params.as_object().cloned();
    if advanced != d.advanced && !advanced && object.is_none() {
        d.error = Some("fix the JSON before going back to the fields".into());
        advanced = true;
    }
    // Pretty-printed once, when the JSON view opens.
    if advanced && !d.advanced && params.is_object() && d.text.len() < MAX_PRETTY {
        d.text = serde_json::to_string_pretty(&params).unwrap_or_else(|_| d.text.clone());
    }
    d.advanced = advanced;
    match (spec.form, object) {
        (Some(fields), Some(mut obj)) if !d.advanced => {
            let refused = d.refused.as_ref().map(|(k, p)| (k.as_str(), p.as_str()));
            let cx = crate::form::Context {
                units,
                names: &|c| {
                    let mut names = session.names(c);
                    if c == fieldcraft_engine::schema::Collection::Set
                        && let Some(p) = &pending
                        && !names.contains(p)
                    {
                        names.push(p.clone());
                    }
                    names
                },
                salt: form_id,
                touched: &d.touched,
                attempted: d.attempted,
                refused,
                problems: &problems,
            };
            let out = crate::form::show(ui, t, &cx, fields, &mut obj);
            invalid = out.invalid;
            create = out.create;
            if out.changed {
                d.text = Value::Object(obj).to_string();
                d.error = None;
                d.refused = None;
            }
            d.touched.extend(out.touched);
        }
        _ => {
            ui.add(egui::TextEdit::multiline(&mut d.text).id(form_id.with("json")).code_editor().desired_rows(6).desired_width(f32::INFINITY));
        }
    }
    if let Some(e) = &d.error {
        ui.label(RichText::new(e).color(t.error));
    } else if d.attempted && !missing.is_empty() {
        ui.label(RichText::new(&missing).color(t.error));
    }
    ui.collapsing(RichText::new("What it takes").size(12.0).color(t.text_dim), |ui| {
        ui.label(RichText::new(spec.params).small().color(t.text_dim));
    });
    ui.add_space(4.0);
    let ready = invalid == 0 && missing.is_empty();
    let not_ready =
        || if invalid > 0 { format!("{missing}\nA field holds something that is not a value").trim().to_string() } else { missing.clone() };
    let mut attempted = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel |= ui.button("Cancel").clicked();
            let ok =
                ui.add_enabled(ready, egui::Button::new(RichText::new("OK").color(egui::Color32::WHITE)).fill(t.accent).min_size(vec2(64.0, 0.0)));
            // A click on the greyed OK shows every problem, beside its field.
            attempted = !ready && ok.contains_pointer() && ui.input(|i| i.pointer.primary_clicked());
            run = ok
                .on_disabled_hover_ui(|ui| {
                    ui.label(not_ready());
                })
                .clicked();
        });
    });
    if attempted && let Some(d) = app.dialog.as_mut() {
        d.attempted = true;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && app.ui.menu.is_none() && !app.ui.palette {
        cancel = true;
    }
    if cancel {
        app.close_dialog();
    } else if let Some((collection, path)) = create {
        app.create_from_form(collection, path);
    } else if run {
        app.submit_dialog();
    } else if use_selection
        && let Err(e) = app.use_selection_in_dialog()
        && let Some(d) = app.dialog.as_mut()
    {
        d.error = Some(e);
    }
}

/// The selected definition's fields as (label, text) in its form's words and units, read again only
/// when the model changes.
fn property_rows(app: &mut ModelApp, kind: Def, name: &str) -> Option<std::sync::Arc<Vec<(String, String)>>> {
    let key = (app.session.revision(), kind, name.to_string());
    if let Some((k, rows)) = &app.properties
        && *k == key
    {
        return Some(rows.clone());
    }
    let rows = crate::describe::rows(&app.session.model, kind, name)?;
    let rows = std::sync::Arc::new(rows);
    app.properties = Some((key, rows.clone()));
    Some(rows)
}

/// The selected definition's fields, with Edit (its form, filled in) and Delete.
fn properties(app: &mut ModelApp, ui: &mut egui::Ui, t: &Tokens, kind: Def, name: &str) {
    let Some(rows) = property_rows(app, kind, name) else { return };
    let mut deselect = false;
    heading(ui, t, Level::Title, |ui| {
        ui.label(RichText::new(kind.label().to_uppercase()).font(label_font(ui.ctx(), 12.0)));
        ui.label(RichText::new(name).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            deselect = ui.small_button("×").on_hover_text("Deselect").clicked();
        });
    });
    ui.add_space(4.0);
    egui::Grid::new("fc_properties").num_columns(2).spacing(vec2(10.0, 3.0)).show(ui, |ui| {
        for (k, text) in rows.iter() {
            ui.label(RichText::new(k).color(t.text_dim));
            ui.add(egui::Label::new(text).wrap());
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    let (mut edit, mut delete) = (false, false);
    let editing = kind.edit_command(&app.session.model, name);
    ui.horizontal(|ui| {
        if let Some(command) = &editing {
            edit = ui.button("Edit…").on_hover_text(command).clicked();
        }
        delete = ui.button("Delete…").on_hover_text(format!("{} (Delete key)", kind.command("delete"))).clicked();
    });
    if deselect {
        app.selected = None;
    } else if edit && let Some((command, params)) = kind.edit(&app.session.model, name) {
        app.open_form(&command, params);
    } else if delete {
        app.ask_delete(kind, name);
    }
}

/// A field's value as short text: numbers and strings as they are, long lists as their length
/// and first items, other lists and objects as compact JSON (long ones shortened).
pub(crate) fn value_text(v: &Value) -> String {
    const MAX: usize = 160;
    /// Lists longer than this show their length and first items.
    const LIST: usize = 12;
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Null => "—".into(),
        Value::Array(a) if a.len() > LIST => {
            let first: Vec<String> = a.iter().take(LIST).map(Value::to_string).collect();
            format!("{} items: {}, …", a.len(), first.join(", "))
        }
        other => other.to_string(),
    };
    if s.chars().count() > MAX { format!("{}…", s.chars().take(MAX).collect::<String>()) } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_values_are_shortened() {
        assert_eq!(value_text(&json!("Steel")), "Steel");
        assert_eq!(value_text(&json!([1, 2])), "[1,2]");
        assert_eq!(value_text(&Value::Null), "—");
        let long = json!((0..500).collect::<Vec<_>>());
        assert!(value_text(&long).starts_with("500 items: 0, 1, 2"));
        let text = json!({"a": "x".repeat(400)});
        assert!(value_text(&text).ends_with('…') && value_text(&text).chars().count() == 161);
    }
}
