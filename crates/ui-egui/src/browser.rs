//! The browser (left): the model as a tree. Geometry and Mesh with eye toggles, then the model's
//! definitions (sets, materials, sections, steps, loads and boundary conditions, interactions)
//! with counts, and the solved steps' results. Clicking a definition selects it (its properties
//! show on the right), a double click opens its edit form, and clicking a result step shows it.
//!
//! Row drawing follows SolidCraft's `crates/ui-egui/src/browser.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};
use modelcraft_model::Model;
use serde::Serialize;
use serde_json::{Value, json};

use crate::ModelApp;
use crate::icons::{self, Look};
use crate::theme::Tokens;

/// The browser's width.
pub const WIDTH: f32 = 250.0;
const ROW_H: f32 = 22.0;
const INDENT: f32 = 14.0;

/// A kind of model definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Def {
    Set,
    Material,
    Section,
    Step,
    BoundaryCondition,
    Load,
    ReferencePoint,
    Coupling,
    FluidDomain,
}

impl Def {
    /// The commands' stem: `model.<stem>.create|edit|delete`.
    pub fn stem(self) -> &'static str {
        match self {
            Def::Set => "set",
            Def::Material => "material",
            Def::Section => "section",
            Def::Step => "step",
            Def::BoundaryCondition => "boundary_condition",
            Def::Load => "load",
            Def::ReferencePoint => "reference_point",
            Def::Coupling => "coupling",
            Def::FluidDomain => "fluid_domain",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Def::Set => "Set",
            Def::Material => "Material",
            Def::Section => "Section",
            Def::Step => "Step",
            Def::BoundaryCondition => "Boundary Condition",
            Def::Load => "Load",
            Def::ReferencePoint => "Reference Point",
            Def::Coupling => "Coupling",
            Def::FluidDomain => "Fluid Domain",
        }
    }

    /// Its icon: the catalogue entry that creates it.
    pub fn icon(self) -> &'static str {
        match self {
            Def::Set => "model.set.create",
            Def::Material => "model.material.create",
            Def::Section => "model.section.create",
            Def::Step => "model.step.create",
            Def::BoundaryCondition => "model.boundary_condition.create",
            Def::Load => "model.load.create",
            Def::ReferencePoint => "model.reference_point.create",
            Def::Coupling => "model.coupling.create",
            Def::FluidDomain => "model.fluid_domain.create",
        }
    }

    /// The command of this kind with `verb` (`edit`, `delete`).
    pub fn command(self, verb: &str) -> String {
        format!("model.{}.{verb}", self.stem())
    }

    /// How many definitions of this kind the model has.
    pub fn count(self, m: &Model) -> usize {
        match self {
            Def::Set => m.sets().len(),
            Def::Material => m.materials().len(),
            Def::Section => m.sections().len(),
            Def::Step => m.steps().len(),
            Def::BoundaryCondition => m.boundary_conditions().len(),
            Def::Load => m.loads().len(),
            Def::ReferencePoint => m.reference_points().len(),
            Def::Coupling => m.couplings().len(),
            Def::FluidDomain => m.fluid_domains().len(),
        }
    }

    /// Whether the model has a definition of this kind called `name`.
    pub fn exists(self, m: &Model, name: &str) -> bool {
        match self {
            Def::Set => m.sets().contains_key(name),
            Def::Material => m.materials().contains_key(name),
            Def::Section => m.sections().contains_key(name),
            Def::Step => m.step(name).is_some(),
            Def::BoundaryCondition => m.boundary_conditions().contains_key(name),
            Def::Load => m.loads().contains_key(name),
            Def::ReferencePoint => m.reference_points().contains_key(name),
            Def::Coupling => m.couplings().contains_key(name),
            Def::FluidDomain => m.fluid_domain(name).is_some(),
        }
    }

    /// The names of this kind's definitions, in the model's order.
    pub fn names(self, m: &Model) -> Vec<String> {
        match self {
            Def::Set => m.sets().keys().cloned().collect(),
            Def::Material => m.materials().keys().cloned().collect(),
            Def::Section => m.sections().keys().cloned().collect(),
            Def::Step => m.steps().iter().map(|s| s.name.clone()).collect(),
            Def::BoundaryCondition => m.boundary_conditions().keys().cloned().collect(),
            Def::Load => m.loads().keys().cloned().collect(),
            Def::ReferencePoint => m.reference_points().keys().cloned().collect(),
            Def::Coupling => m.couplings().keys().cloned().collect(),
            Def::FluidDomain => m.fluid_domains().keys().cloned().collect(),
        }
    }
}

/// A definition as JSON (its command parameters' shape, with its name); `None` when it is gone.
pub fn definition(m: &Model, kind: Def, name: &str) -> Option<Value> {
    let v = match kind {
        Def::Set => serde_json::to_value(m.sets().get(name)?),
        Def::Material => serde_json::to_value(m.materials().get(name)?),
        Def::Section => serde_json::to_value(m.sections().get(name)?),
        Def::Step => serde_json::to_value(m.step(name)?),
        Def::BoundaryCondition => serde_json::to_value(m.boundary_conditions().get(name)?),
        Def::Load => serde_json::to_value(m.loads().get(name)?),
        Def::ReferencePoint => serde_json::to_value(m.reference_points().get(name)?),
        Def::Coupling => serde_json::to_value(m.couplings().get(name)?),
        Def::FluidDomain => serde_json::to_value(m.fluid_domain(name)?),
    }
    .ok()?;
    let mut v = if v.is_object() { v } else { json!({"value": v}) };
    if let Some(o) = v.as_object_mut() {
        o.insert("name".into(), json!(name));
    }
    Some(v)
}

/// One row's look.
#[derive(Default)]
struct Row<'a> {
    depth: usize,
    /// Some(open) for rows that fold.
    fold: Option<bool>,
    /// Some(visible) for rows with an eye.
    eye: Option<bool>,
    icon: Option<&'static str>,
    /// A colour swatch instead of an icon (element kinds).
    swatch: Option<Color32>,
    label: &'a str,
    /// Dim text after the label (a count).
    note: Option<String>,
    selected: bool,
    dim: bool,
}

/// What happened on a row.
#[derive(Default)]
struct RowResp {
    clicked: bool,
    double: bool,
    fold: bool,
    eye: bool,
}

fn draw_row(app: &mut ModelApp, ui: &mut egui::Ui, id: egui::Id, row: &Row) -> RowResp {
    let t = Tokens::get(app.ui.dark);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    if !ui.is_rect_visible(r) {
        return RowResp::default();
    }
    let resp = ui.interact(r, id, Sense::click());
    if row.selected {
        ui.painter().rect_filled(r, 3.0, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    let mut x = r.left() + 2.0 + row.depth as f32 * INDENT;
    let cy = r.center().y;
    let fold_r = Rect::from_min_max(pos2(x - 4.0, r.top()), pos2(x + 15.0, r.bottom()));
    let eye_r = Rect::from_center_size(pos2(x + 14.0 + 8.0, cy), vec2(19.0, r.height()));
    let at = |p: Pos2| -> u8 {
        if row.fold.is_some() && fold_r.contains(p) {
            1
        } else if row.eye.is_some() && eye_r.contains(p) {
            2
        } else {
            0
        }
    };
    let click_on = resp.interact_pointer_pos().map_or(0, at);
    let hover_on = resp.hover_pos().map_or(0, at);
    let mut out = RowResp::default();
    if let Some(open) = row.fold {
        let ar = Rect::from_center_size(pos2(x + 6.0, cy), vec2(12.0, 12.0));
        let pts = if open {
            vec![pos2(ar.left() + 2.0, ar.top() + 4.0), pos2(ar.right() - 2.0, ar.top() + 4.0), pos2(ar.center().x, ar.bottom() - 3.0)]
        } else {
            vec![pos2(ar.left() + 4.0, ar.top() + 2.0), pos2(ar.right() - 3.0, ar.center().y), pos2(ar.left() + 4.0, ar.bottom() - 2.0)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, if hover_on == 1 { t.text } else { t.text_dim }, Stroke::NONE));
        out.fold = resp.clicked() && click_on == 1;
    }
    x += 14.0;
    if let Some(vis) = row.eye {
        let er = Rect::from_center_size(pos2(x + 8.0, cy), Vec2::splat(15.0));
        icons::paint(app, ui, "ui.eye", er, Look::of(vis));
        if !vis {
            ui.painter().line_segment([er.left_bottom() + vec2(2.0, -2.0), er.right_top() + vec2(-2.0, 2.0)], Stroke::new(1.4, t.text_dim));
        }
        out.eye = resp.clicked() && click_on == 2;
        x += 20.0;
    }
    if let Some(icon) = row.icon {
        icons::paint(app, ui, icon, Rect::from_min_size(pos2(x, cy - 8.0), Vec2::splat(16.0)), Look::of(!row.dim));
        x += 21.0;
    } else if let Some(c) = row.swatch {
        ui.painter().rect_filled(Rect::from_center_size(pos2(x + 6.0, cy), Vec2::splat(10.0)), 2.0, c);
        x += 18.0;
    }
    let col = if row.dim { t.text_dim } else { t.text };
    let galley = ui.painter().layout_no_wrap(row.label.to_string(), FontId::proportional(12.5), col);
    let lw = galley.size().x;
    ui.painter().with_clip_rect(r).galley(pos2(x, cy - galley.size().y / 2.0), galley, col);
    if let Some(n) = &row.note {
        ui.painter().with_clip_rect(r).text(pos2(x + lw + 6.0, cy), egui::Align2::LEFT_CENTER, n, FontId::proportional(11.5), t.text_dim);
    }
    let tip = match hover_on {
        1 => Some(if row.fold == Some(true) { "Collapse" } else { "Expand" }),
        2 => Some(if row.eye == Some(true) { "Hide" } else { "Show" }),
        _ => None,
    };
    let resp = match tip {
        Some(tip) => resp.on_hover_text(tip),
        None => resp,
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, row.label));
    out.clicked = resp.clicked() && click_on == 0;
    out.double = resp.double_clicked() && click_on == 0;
    out
}

/// What the browser asks for after drawing (it cannot run commands while it borrows the model).
enum Act {
    Fold(String),
    Select(Def, String),
    Edit(Def, String),
    ShowResult(String),
}

pub fn browser(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let mut acts: Vec<Act> = Vec::new();
    egui::Panel::left("mc_browser")
        .exact_size(WIDTH)
        .resizable(false)
        .show_separator_line(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(6, 6)))
        .show(ui, |ui| {
            app.place("browser", ui.max_rect());
            ui.label(RichText::new("BROWSER").size(11.0).strong().color(t.text_dim));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("mc_browser_scroll").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                rows(app, ui, &mut acts);
            });
        });
    for a in acts {
        match a {
            Act::Fold(key) => {
                if !app.tree_open.remove(&key) {
                    app.tree_open.insert(key);
                }
            }
            Act::Select(kind, name) => {
                app.selected = if app.selected.as_ref() == Some(&(kind, name.clone())) { None } else { Some((kind, name)) };
            }
            Act::Edit(kind, name) => {
                if let Some(v) = definition(&app.session.model, kind, &name) {
                    app.selected = Some((kind, name));
                    app.open_form(&kind.command("edit"), v);
                }
            }
            Act::ShowResult(step) => {
                // Errors show in the status bar.
                let _ = app.run("results.deformed", json!({"step": step}));
            }
        }
    }
}

fn rows(app: &mut ModelApp, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    let t = Tokens::get(app.ui.dark);
    let open = |app: &ModelApp, key: &str| app.tree_open.contains(key);
    let name = app.model_name();
    let dirty = app.session.is_dirty();
    draw_row(
        app,
        ui,
        egui::Id::new("mc_row_root"),
        &Row { icon: Some("ui.views"), label: &name, note: dirty.then(|| "unsaved".into()), ..Default::default() },
    );
    // Geometry.
    let design = app.session.design.clone();
    let surfaces: Vec<String> = app.session.surfaces.iter().map(|s| s.name.clone()).collect();
    let bodies = design.as_ref().map_or(0, |d| d.bodies.len());
    let r = draw_row(
        app,
        ui,
        egui::Id::new("mc_row_geometry"),
        &Row {
            depth: 1,
            fold: Some(open(app, "geometry")),
            eye: Some(app.ui.show_geometry),
            icon: Some("geometry.open"),
            label: "Geometry",
            note: Some(format!("{}", bodies + surfaces.len())),
            ..Default::default()
        },
    );
    if r.eye {
        app.ui.show_geometry = !app.ui.show_geometry;
    } else if r.fold || r.clicked {
        acts.push(Act::Fold("geometry".into()));
    }
    if open(app, "geometry") {
        if let Some(d) = &design {
            let label = d.name.clone();
            draw_row(
                app,
                ui,
                egui::Id::new("mc_row_design"),
                &Row { depth: 2, icon: Some("geometry.open"), label: &label, note: Some(format!("{bodies} bodies")), ..Default::default() },
            );
            for (i, b) in d.bodies.iter().enumerate() {
                draw_row(
                    app,
                    ui,
                    egui::Id::new(("mc_row_body", i)),
                    &Row { depth: 3, icon: Some("geometry.bodies"), label: &b.name, ..Default::default() },
                );
            }
        }
        for (i, s) in surfaces.iter().enumerate() {
            draw_row(
                app,
                ui,
                egui::Id::new(("mc_row_surface", i)),
                &Row { depth: 2, icon: Some("geometry.import.mesh"), label: s, ..Default::default() },
            );
        }
        if design.is_none() && surfaces.is_empty() {
            draw_row(
                app,
                ui,
                egui::Id::new("mc_row_no_geometry"),
                &Row { depth: 2, label: "No geometry: Open Design", dim: true, ..Default::default() },
            );
        }
    }
    // Mesh.
    let (nodes, elements) = (app.session.model.nodes().len(), app.session.model.elements().len());
    let r = draw_row(
        app,
        ui,
        egui::Id::new("mc_row_mesh"),
        &Row {
            depth: 1,
            fold: Some(open(app, "mesh")),
            eye: Some(app.ui.show_mesh),
            icon: Some("mesh.tet.generate"),
            label: "Mesh",
            note: Some(format!("{elements}")),
            ..Default::default()
        },
    );
    if r.eye {
        app.ui.show_mesh = !app.ui.show_mesh;
    } else if r.fold || r.clicked {
        acts.push(Act::Fold("mesh".into()));
    }
    if open(app, "mesh") {
        let r = draw_row(
            app,
            ui,
            egui::Id::new("mc_row_nodes"),
            &Row {
                depth: 2,
                eye: Some(app.ui.show_nodes),
                icon: Some("mesh.node.add"),
                label: "Nodes",
                note: Some(format!("{nodes}")),
                ..Default::default()
            },
        );
        if r.eye {
            app.ui.show_nodes = !app.ui.show_nodes;
        }
        let revision = app.session.revision();
        if app.element_counts.as_ref().is_none_or(|(r, _)| *r != revision) {
            app.element_counts = Some((revision, app.session.model.element_counts()));
        }
        for (kind, n) in app.element_counts.as_ref().map(|(_, c)| c.clone()).unwrap_or_default() {
            draw_row(
                app,
                ui,
                egui::Id::new(("mc_row_kind", kind.name())),
                &Row { depth: 2, swatch: Some(t.element(kind)), label: kind.name(), note: Some(format!("{n}")), ..Default::default() },
            );
        }
    }
    // Definitions.
    let groups: [(&str, &str, &[Def]); 7] = [
        ("sets", "Sets", &[Def::Set]),
        ("materials", "Materials", &[Def::Material]),
        ("sections", "Sections", &[Def::Section]),
        ("steps", "Steps", &[Def::Step]),
        ("loads", "Loads & BCs", &[Def::BoundaryCondition, Def::Load]),
        ("interactions", "Interactions", &[Def::ReferencePoint, Def::Coupling]),
        ("fluids", "Fluid Domains", &[Def::FluidDomain]),
    ];
    for (key, label, kinds) in groups {
        let items: Vec<(Def, String)> = kinds.iter().flat_map(|k| k.names(&app.session.model).into_iter().map(move |n| (*k, n))).collect();
        let icon = kinds.first().map(|k| k.icon());
        let r = draw_row(
            app,
            ui,
            egui::Id::new(("mc_row_group", key)),
            &Row {
                depth: 1,
                fold: Some(open(app, key)),
                icon,
                label,
                note: Some(format!("{}", items.len())),
                dim: items.is_empty(),
                ..Default::default()
            },
        );
        if r.fold || r.clicked {
            acts.push(Act::Fold(key.into()));
        }
        if open(app, key) {
            for (kind, name) in items {
                let selected = app.selected.as_ref() == Some(&(kind, name.clone()));
                let r = draw_row(
                    app,
                    ui,
                    egui::Id::new(("mc_row_def", kind.stem(), &name)),
                    &Row { depth: 2, icon: Some(kind.icon()), label: &name, selected, ..Default::default() },
                );
                if r.double {
                    acts.push(Act::Edit(kind, name));
                } else if r.clicked {
                    acts.push(Act::Select(kind, name));
                }
            }
        }
    }
    // Results.
    let steps: Vec<(String, bool)> = app.session.result_steps().map(|(n, c)| (n.to_string(), c)).collect();
    let shown = app.session.results_view.as_ref().map(|v| v.step.clone());
    let r = draw_row(
        app,
        ui,
        egui::Id::new("mc_row_results"),
        &Row {
            depth: 1,
            fold: Some(open(app, "results")),
            icon: Some("results.contour"),
            label: "Results",
            note: Some(format!("{}", steps.len())),
            dim: steps.is_empty(),
            ..Default::default()
        },
    );
    if r.fold || r.clicked {
        acts.push(Act::Fold("results".into()));
    }
    if open(app, "results") {
        for (step, current) in steps {
            let label = if current { step.clone() } else { format!("{step} (stale)") };
            let selected = shown.as_deref() == Some(step.as_str());
            let r = draw_row(
                app,
                ui,
                egui::Id::new(("mc_row_result", &step)),
                &Row { depth: 2, icon: Some("results.deformed"), label: &label, selected, dim: !current, ..Default::default() },
            );
            if r.clicked {
                acts.push(Act::ShowResult(step));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use modelcraft_engine::Session;

    use super::*;

    #[test]
    fn definitions_read_back_as_their_parameters() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&modelcraft_engine::sample::cantilever()).unwrap();
        let m = &app.session.model;
        assert_eq!(Def::Step.names(m), ["Load", "Modes"]);
        let steel = definition(m, Def::Material, "Steel").unwrap();
        assert_eq!(steel["name"], "Steel");
        assert!(steel.get("elastic").is_some(), "{steel}");
        assert!(definition(m, Def::Load, "nope").is_none());
        for kind in [
            Def::Set,
            Def::Material,
            Def::Section,
            Def::Step,
            Def::BoundaryCondition,
            Def::Load,
            Def::ReferencePoint,
            Def::Coupling,
            Def::FluidDomain,
        ] {
            for verb in ["create", "edit", "delete"] {
                assert!(modelcraft_engine::find_command(&kind.command(verb)).is_some(), "{kind:?} {verb}");
            }
            assert!(modelcraft_engine::catalogue::find(kind.icon()).is_some());
        }
    }
}
