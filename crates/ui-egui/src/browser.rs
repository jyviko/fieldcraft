//! The browser (left): the model as a tree. Geometry (the design's bodies, datums and
//! partitions) and Mesh with eye toggles, then the model's
//! definitions (sets, materials, sections, steps, loads and boundary conditions, interactions)
//! with counts (Loads & BCs and Interactions with eye toggles for their viewport symbols), and
//! the solved steps' results. Clicking a definition selects it (its properties show on the
//! right), a double click opens its edit form, and clicking a result step shows it.
//!
//! The tree also shows where the model is in the analysis workflow
//! ([`fieldcraft_engine::progress`]): each top-level node that is a setup stage carries a mark at
//! the right of its row, beside its count (✓ done, → the next stage, ⚠ when its command cannot
//! run yet), with the reason and the place in its hover, and a node that still lacks what its
//! stage needs shows a ghost row ("＋ Add material…") that starts the providing command. Optional
//! stages (seeds and partitions) carry no mark.
//!
//! Row drawing follows SolidCraft's `crates/ui-egui/src/browser.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};
use fieldcraft_engine::progress::{Stage, StageState, Status};
use fieldcraft_engine::schema::Collection;
use fieldcraft_model::Model;
use serde::Serialize;
use serde_json::{Value, json};

use crate::ModelApp;
use crate::icons::{self, Look};
use crate::theme::Tokens;

/// The browser's width.
pub const WIDTH: f32 = 250.0;
const ROW_H: f32 = 22.0;
const INDENT: f32 = 14.0;
/// Between a row's label and its note.
const NOTE_GAP: f32 = 6.0;
/// Free space kept at a row's right end.
const ROW_PAD: f32 = 4.0;
/// The state mark's slot at a top-level row's right end.
const MARK_W: f32 = 14.0;

/// The browser's top-level nodes below the model, in order, with the setup stages each shows
/// (`ui.inspect`'s `browser`). Seeds and partitions ([`Stage::Prepare`]) are optional and sit
/// under Geometry and Mesh without a mark.
pub const NODES: [(&str, &[Stage]); 10] = [
    ("geometry", &[Stage::Geometry]),
    ("mesh", &[Stage::Mesh]),
    ("sets", &[Stage::Sets]),
    ("materials", &[Stage::Material]),
    ("sections", &[Stage::Section]),
    ("steps", &[Stage::Step]),
    ("loads", &[Stage::BoundaryConditions, Stage::Loads]),
    ("interactions", &[]),
    ("fluids", &[]),
    ("results", &[Stage::Solve, Stage::Results]),
];

fn node_stages(node: &str) -> &'static [Stage] {
    NODES.iter().find(|(n, _)| *n == node).map_or(&[], |(_, st)| st)
}

/// A stage's hover: what it needs and where; `None` once done or when optional.
fn stage_tip(s: &StageState) -> Option<String> {
    match s.status {
        Status::Next => Some(format!("Next: {} — {}", s.label, s.place)),
        Status::Blocked => Some(s.detail.clone().unwrap_or_else(|| s.place.clone())),
        Status::Todo => Some(format!("{} still to do — {}", s.label, s.place)),
        Status::Done | Status::Optional => None,
    }
}

/// A node's state and hover from its stages: the most pressing of them (next, blocked, to do),
/// else done; `None` for nodes that are no setup stage.
fn node_state(stages: &[StageState], node: &str) -> Option<(Status, String)> {
    let own: Vec<&StageState> = stages.iter().filter(|s| node_stages(node).contains(&s.stage)).collect();
    let status =
        [Status::Next, Status::Blocked, Status::Todo, Status::Done, Status::Optional].into_iter().find(|st| own.iter().any(|s| s.status == *st))?;
    let tip: Vec<String> = own.iter().filter_map(|s| stage_tip(s)).collect();
    let tip = if tip.is_empty() { "Done".to_string() } else { tip.join("\n") };
    Some((status, tip))
}

/// A ghost row's text: what its stage's command adds.
fn ghost_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Geometry => "Open design…",
        Stage::Prepare => "Seed edges…",
        Stage::Mesh => "Generate mesh…",
        Stage::Material => "Add material…",
        Stage::Section => "Add section…",
        Stage::Sets => "Add set…",
        Stage::BoundaryConditions => "Add boundary condition…",
        Stage::Loads => "Add load…",
        Stage::Step => "Add step…",
        Stage::Solve => "Solve…",
        Stage::Results => "Show results…",
    }
}

/// Where a stage's ghost row was drawn (`ui.inspect`'s `layout`).
pub fn ghost_region(stage: Stage) -> &'static str {
    match stage {
        Stage::Geometry => "browser/add/geometry",
        Stage::Prepare => "browser/add/prepare",
        Stage::Mesh => "browser/add/mesh",
        Stage::Material => "browser/add/material",
        Stage::Section => "browser/add/section",
        Stage::Sets => "browser/add/sets",
        Stage::BoundaryConditions => "browser/add/boundary_conditions",
        Stage::Loads => "browser/add/loads",
        Stage::Step => "browser/add/step",
        Stage::Solve => "browser/add/solve",
        Stage::Results => "browser/add/results",
    }
}

/// The stages of `node` still to do: each shows a ghost row, except a blocked one behind an
/// earlier stage of the same node still to do (Results offers Solve, not Show results, until a
/// solve).
fn ghosts<'s>(stages: &'s [StageState], node: &str) -> Vec<&'s StageState> {
    let mut pending = false;
    let mut out = Vec::new();
    for s in stages.iter().filter(|s| node_stages(node).contains(&s.stage)) {
        let todo = matches!(s.status, Status::Next | Status::Todo | Status::Blocked);
        if todo && !(pending && s.status == Status::Blocked) {
            out.push(s);
        }
        pending |= todo;
    }
    out
}

/// Each top-level node's setup state for `ui.inspect`: `[{node, state, tip, add: [{stage,
/// label, command, status}]}]`; `state` is null for nodes that are no setup stage.
pub fn describe(app: &mut ModelApp) -> Value {
    let stages = app.setup_stages();
    Value::Array(
        NODES
            .iter()
            .map(|(node, _)| {
                let state = node_state(&stages, node);
                let add: Vec<Value> = ghosts(&stages, node)
                    .into_iter()
                    .map(|s| json!({"stage": s.stage, "label": ghost_label(s.stage), "command": s.command, "status": s.status}))
                    .collect();
                json!({"node": node, "state": state.as_ref().map(|(st, _)| st), "tip": state.map(|(_, tip)| tip), "add": add})
            })
            .collect(),
    )
}

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
    Contact,
    FluidDomain,
}

impl Def {
    /// The reference fields that name one of these.
    pub fn collection(self) -> Collection {
        match self {
            Def::Set => Collection::Set,
            Def::Material => Collection::Material,
            Def::Section => Collection::Section,
            Def::Step => Collection::Step,
            Def::BoundaryCondition => Collection::BoundaryCondition,
            Def::Load => Collection::Load,
            Def::ReferencePoint => Collection::ReferencePoint,
            Def::Coupling => Collection::Coupling,
            Def::Contact => Collection::Contact,
            Def::FluidDomain => Collection::FluidDomain,
        }
    }

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
            Def::Contact => "contact",
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
            Def::Contact => "Contact",
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
            Def::Contact => "interaction.contact.pair",
            Def::FluidDomain => "model.fluid_domain.create",
        }
    }

    /// The command of this kind with `verb` (`create`, `edit`, `delete`); contacts are
    /// interactions, created as pairs or general contact.
    pub fn command(self, verb: &str) -> String {
        match (self, verb) {
            (Def::Contact, "create") => "interaction.contact.pair".into(),
            (Def::Contact, _) => format!("interaction.contact.{verb}"),
            _ => format!("model.{}.{verb}", self.stem()),
        }
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
            Def::Contact => m.contacts().len(),
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
            Def::Contact => m.contacts().contains_key(name),
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
            Def::Contact => m.contacts().keys().cloned().collect(),
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
        Def::Contact => serde_json::to_value(m.contacts().get(name)?),
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
    /// The setup state mark at the row's right end (top-level nodes).
    mark: Option<Status>,
    /// The hover text (a setup state's reason and place).
    tip: Option<String>,
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
    // The note (a count, a seed's size and bias) keeps its room; the label gets the rest, elided in
    // the middle when long, whole on hover.
    // The trailing area, right to left: the state mark's slot (top-level rows keep it so their
    // counts line up), then the note.
    let right = r.right() - ROW_PAD - if row.depth == 1 { MARK_W } else { 0.0 };
    if let Some(status) = row.mark {
        paint_mark(ui.painter(), Rect::from_center_size(pos2(right + MARK_W / 2.0, cy), Vec2::splat(9.0)), status, &t);
    }
    let note = row.note.as_ref().map(|n| ui.painter().layout_no_wrap(n.clone(), FontId::proportional(11.5), t.text_dim));
    let note_w = note.as_ref().map_or(0.0, |g| g.size().x + NOTE_GAP);
    let font = FontId::proportional(12.5);
    let room = (right - note_w - x).max(0.0);
    let label = crate::elide::middle_in(ui, row.label, &font, room);
    let elided = matches!(label, std::borrow::Cow::Owned(_));
    let galley = ui.painter().layout_no_wrap(label.into_owned(), font, col);
    ui.painter().with_clip_rect(r).galley(pos2(x, cy - galley.size().y / 2.0), galley, col);
    if let Some(n) = note {
        let at = pos2(right - n.size().x, cy - n.size().y / 2.0);
        ui.painter().with_clip_rect(r).galley(at, n, t.text_dim);
    }
    let tip = match hover_on {
        1 => Some((if row.fold == Some(true) { "Collapse" } else { "Expand" }).to_string()),
        2 => Some((if row.eye == Some(true) { "Hide" } else { "Show" }).to_string()),
        _ => match (&row.tip, elided) {
            (Some(tip), true) => Some(format!("{}\n\n{tip}", row.label)),
            (Some(tip), false) => Some(tip.clone()),
            (None, true) => Some(row.label.to_string()),
            (None, false) => None,
        },
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

/// ✓ done (success), → next (accent), ⚠ blocked (muted); nothing for the rest.
fn paint_mark(p: &egui::Painter, r: Rect, status: Status, t: &Tokens) {
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * r.width(), r.top() + y * r.height());
    match status {
        Status::Done => {
            let s = Stroke::new(1.6, t.ok);
            p.line_segment([at(0.05, 0.55), at(0.4, 0.88)], s);
            p.line_segment([at(0.4, 0.88), at(0.95, 0.15)], s);
        }
        Status::Next => {
            let s = Stroke::new(1.6, t.accent);
            p.line_segment([at(0.0, 0.5), at(0.95, 0.5)], s);
            p.line_segment([at(0.55, 0.15), at(0.95, 0.5)], s);
            p.line_segment([at(0.55, 0.85), at(0.95, 0.5)], s);
        }
        Status::Blocked => {
            let s = Stroke::new(1.2, t.text_dim);
            p.add(egui::Shape::closed_line(vec![at(0.5, 0.0), at(1.05, 1.0), at(-0.05, 1.0)], s));
            p.line_segment([at(0.5, 0.35), at(0.5, 0.65)], s);
            p.circle_filled(at(0.5, 0.84), 0.7, t.text_dim);
        }
        Status::Todo | Status::Optional => {}
    }
}

/// A ghost row under a node that lacks what its stage needs: "＋ Add material…", muted, the next
/// stage's in the accent colour. Clicking it (or Enter/Space when it has the keyboard focus)
/// starts the stage's command; true when activated.
fn draw_ghost(app: &mut ModelApp, ui: &mut egui::Ui, s: &StageState) -> bool {
    let t = Tokens::get(app.ui.dark);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    app.place(ghost_region(s.stage), r);
    if !ui.is_rect_visible(r) {
        return false;
    }
    let resp = ui.interact(r, egui::Id::new(("fc_row_add", ghost_region(s.stage))), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(r.shrink(0.5), 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    let col = if s.status == Status::Next { t.accent } else { t.text_dim };
    // Aligned with the node's children: past the fold arrow, the plus in the icon's place.
    let x = r.left() + 2.0 + 2.0 * INDENT + 14.0;
    let c = pos2(x + 8.0, r.center().y);
    let plus = Stroke::new(1.3, col);
    ui.painter().line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], plus);
    ui.painter().line_segment([c - vec2(0.0, 4.0), c + vec2(0.0, 4.0)], plus);
    let label = ghost_label(s.stage);
    ui.painter().with_clip_rect(r).text(pos2(x + 21.0, r.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(12.5), col);
    let resp = resp.on_hover_text(stage_tip(s).unwrap_or_default());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp.clicked()
}

/// What the browser asks for after drawing (it cannot run commands while it borrows the model).
enum Act {
    Fold(String),
    Select(Def, String),
    Edit(Def, String),
    ShowResult(String),
    /// A ghost row's command.
    Start(&'static str),
}

/// A seed in a few words: `8 exact`, `2.5 min, bias 4 end`.
fn seed_note(s: &fieldcraft_model::Seed) -> String {
    let what = match s.divisions {
        fieldcraft_model::Divisions::Count(n) => format!("{n}"),
        fieldcraft_model::Divisions::Size(h) => format!("size {h}"),
    };
    let bias = s.bias.map(|b| format!(", bias {} {}", b.ratio, json_name(&b.toward))).unwrap_or_default();
    format!("{what} {}{bias}", s.constraint.name())
}

fn json_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

pub fn browser(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let mut acts: Vec<Act> = Vec::new();
    egui::Panel::left("fc_browser")
        .exact_size(WIDTH)
        .resizable(false)
        .show_separator_line(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(6, 6)))
        .show(ui, |ui| {
            app.place("browser", ui.max_rect());
            crate::setup::card(app, ui);
            ui.label(RichText::new("BROWSER").font(crate::theme::label_font(ui.ctx(), 10.5)).color(t.text_dim));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("fc_browser_scroll").auto_shrink([false, false]).show(ui, |ui| {
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
            Act::Start(command) => {
                // A command that cannot run yet says why in the status bar.
                let _ = app.start(command);
            }
        }
    }
}

/// The viewport setting a definition group's eye shows and hides: the setup's symbols.
fn eye_flag<'u>(state: &'u mut crate::UiState, group: &str) -> Option<&'u mut bool> {
    match group {
        "loads" => Some(&mut state.show_loads),
        "interactions" => Some(&mut state.show_interactions),
        _ => None,
    }
}

/// The ghost rows of `node`'s stages still to do.
fn ghost_rows(app: &mut ModelApp, ui: &mut egui::Ui, stages: &[StageState], node: &str, acts: &mut Vec<Act>) {
    for s in ghosts(stages, node) {
        if draw_ghost(app, ui, s) {
            acts.push(Act::Start(s.command));
        }
    }
}

fn rows(app: &mut ModelApp, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    let t = Tokens::get(app.ui.dark);
    let stages = app.setup_stages();
    let state = |node: &str| node_state(&stages, node).map_or((None, None), |(st, tip)| (Some(st), Some(tip)));
    let open = |app: &ModelApp, key: &str| app.tree_open.contains(key);
    let name = app.model_name();
    let dirty = app.session.is_dirty();
    draw_row(
        app,
        ui,
        egui::Id::new("fc_row_root"),
        &Row { icon: Some("ui.views"), label: &name, note: dirty.then(|| "unsaved".into()), ..Default::default() },
    );
    // Geometry.
    let design = app.session.design.clone();
    let surfaces: Vec<String> = app.session.surfaces.iter().map(|s| s.name.clone()).collect();
    let bodies = design.as_ref().map_or(0, |d| d.bodies.len());
    let (mark, tip) = state("geometry");
    let r = draw_row(
        app,
        ui,
        egui::Id::new("fc_row_geometry"),
        &Row {
            depth: 1,
            fold: Some(open(app, "geometry")),
            eye: Some(app.ui.show_geometry),
            icon: Some("geometry.open"),
            label: "Geometry",
            note: Some(format!("{}", bodies + surfaces.len())),
            mark,
            tip,
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
                egui::Id::new("fc_row_design"),
                &Row { depth: 2, icon: Some("geometry.open"), label: &label, note: Some(format!("{bodies} bodies")), ..Default::default() },
            );
            for (i, b) in d.bodies.iter().enumerate() {
                draw_row(
                    app,
                    ui,
                    egui::Id::new(("fc_row_body", i)),
                    &Row { depth: 3, icon: Some("geometry.bodies"), label: &b.name, ..Default::default() },
                );
            }
        }
        for (i, s) in surfaces.iter().enumerate() {
            draw_row(
                app,
                ui,
                egui::Id::new(("fc_row_surface", i)),
                &Row { depth: 2, icon: Some("geometry.import.mesh"), label: s, ..Default::default() },
            );
        }
        // Datum planes and axes, then the partitions in the order they cut.
        let datums: Vec<(String, &'static str)> =
            app.session.model.datums().iter().map(|(n, d)| (n.clone(), if d.plane().is_some() { "plane" } else { "axis" })).collect();
        for (i, (n, kind)) in datums.iter().enumerate() {
            draw_row(
                app,
                ui,
                egui::Id::new(("fc_row_datum", i)),
                &Row { depth: 2, icon: Some("geometry.datum"), label: n, note: Some((*kind).into()), ..Default::default() },
            );
        }
        let partitions: Vec<(String, String)> = app.session.model.partitions().iter().map(|p| (p.name.clone(), p.datum.clone())).collect();
        for (i, (n, datum)) in partitions.iter().enumerate() {
            draw_row(
                app,
                ui,
                egui::Id::new(("fc_row_partition", i)),
                &Row { depth: 2, icon: Some("geometry.partition"), label: n, note: Some(format!("by {datum}")), ..Default::default() },
            );
        }
    }
    ghost_rows(app, ui, &stages, "geometry", acts);
    // Mesh.
    let (nodes, elements) = (app.session.model.nodes().len(), app.session.model.elements().len());
    let (mark, tip) = state("mesh");
    let r = draw_row(
        app,
        ui,
        egui::Id::new("fc_row_mesh"),
        &Row {
            depth: 1,
            fold: Some(open(app, "mesh")),
            eye: Some(app.ui.show_mesh),
            icon: Some("mesh.tet.generate"),
            label: "Mesh",
            note: Some(format!("{elements}")),
            mark,
            tip,
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
            egui::Id::new("fc_row_nodes"),
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
                egui::Id::new(("fc_row_kind", kind.name())),
                &Row { depth: 2, swatch: Some(t.element(kind)), label: kind.name(), note: Some(format!("{n}")), ..Default::default() },
            );
        }
        // Edge seeds: a click selects the seeded edge in the viewport.
        let seeds: Vec<(fieldcraft_model::GeometryRef, String)> = app.session.model.seeds().iter().map(|s| (s.edge.clone(), seed_note(s))).collect();
        if !seeds.is_empty() {
            let r = draw_row(
                app,
                ui,
                egui::Id::new("fc_row_seeds"),
                &Row {
                    depth: 2,
                    fold: Some(open(app, "seeds")),
                    icon: Some("mesh.seed.edge"),
                    label: "Seeds",
                    note: Some(format!("{}", seeds.len())),
                    ..Default::default()
                },
            );
            if r.fold || r.clicked {
                acts.push(Act::Fold("seeds".into()));
            }
            if open(app, "seeds") {
                for (i, (edge, note)) in seeds.into_iter().enumerate() {
                    let label = format!("{} · {}", edge.body, edge.entity.name().unwrap_or_default());
                    let r = draw_row(
                        app,
                        ui,
                        egui::Id::new(("fc_row_seed", i)),
                        &Row { depth: 3, label: &label, note: Some(note), ..Default::default() },
                    );
                    if r.clicked {
                        app.select([crate::selection::Item::Geometry(edge)], crate::selection::Mode::Replace);
                    }
                }
            }
        }
    }
    ghost_rows(app, ui, &stages, "mesh", acts);
    // Definitions.
    let groups: [(&str, &str, &[Def]); 7] = [
        ("sets", "Sets", &[Def::Set]),
        ("materials", "Materials", &[Def::Material]),
        ("sections", "Sections", &[Def::Section]),
        ("steps", "Steps", &[Def::Step]),
        ("loads", "Loads & BCs", &[Def::BoundaryCondition, Def::Load]),
        ("interactions", "Interactions", &[Def::ReferencePoint, Def::Coupling, Def::Contact]),
        ("fluids", "Fluid Domains", &[Def::FluidDomain]),
    ];
    for (key, label, kinds) in groups {
        let items: Vec<(Def, String)> = kinds.iter().flat_map(|k| k.names(&app.session.model).into_iter().map(move |n| (*k, n))).collect();
        let icon = kinds.first().map(|k| k.icon());
        let eye = eye_flag(&mut app.ui, key).map(|v| *v);
        let (mark, tip) = state(key);
        let r = draw_row(
            app,
            ui,
            egui::Id::new(("fc_row_group", key)),
            &Row {
                depth: 1,
                fold: Some(open(app, key)),
                eye,
                icon,
                label,
                note: Some(format!("{}", items.len())),
                dim: items.is_empty(),
                mark,
                tip,
                ..Default::default()
            },
        );
        if r.eye
            && let Some(v) = eye_flag(&mut app.ui, key)
        {
            *v = !*v;
        } else if r.fold || r.clicked {
            acts.push(Act::Fold(key.into()));
        }
        if open(app, key) {
            for (kind, name) in items {
                let selected = app.selected.as_ref() == Some(&(kind, name.clone()));
                let r = draw_row(
                    app,
                    ui,
                    egui::Id::new(("fc_row_def", kind.stem(), &name)),
                    &Row { depth: 2, icon: Some(kind.icon()), label: &name, selected, ..Default::default() },
                );
                if r.double {
                    acts.push(Act::Edit(kind, name));
                } else if r.clicked {
                    acts.push(Act::Select(kind, name));
                }
            }
        }
        ghost_rows(app, ui, &stages, key, acts);
    }
    // Results.
    let steps: Vec<(String, bool)> = app.session.result_steps().map(|(n, c)| (n.to_string(), c)).collect();
    let shown = app.session.results_view.as_ref().map(|v| v.step.clone());
    let (mark, tip) = state("results");
    let r = draw_row(
        app,
        ui,
        egui::Id::new("fc_row_results"),
        &Row {
            depth: 1,
            fold: Some(open(app, "results")),
            icon: Some("results.contour"),
            label: "Results",
            note: Some(format!("{}", steps.len())),
            dim: steps.is_empty(),
            mark,
            tip,
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
                egui::Id::new(("fc_row_result", &step)),
                &Row { depth: 2, icon: Some("results.deformed"), label: &label, selected, dim: !current, ..Default::default() },
            );
            if r.clicked {
                acts.push(Act::ShowResult(step));
            }
        }
    }
    ghost_rows(app, ui, &stages, "results", acts);
}

#[cfg(test)]
mod tests {
    use fieldcraft_engine::Session;

    use super::*;
    use Status::*;

    /// Each top-level node's state, and the commands of its ghost rows.
    fn marks(app: &mut ModelApp) -> Vec<(&'static str, Option<Status>, Vec<&'static str>)> {
        let stages = app.setup_stages();
        NODES
            .iter()
            .map(|(n, _)| (*n, node_state(&stages, n).map(|(st, _)| st), ghosts(&stages, n).into_iter().map(|s| s.command).collect()))
            .collect()
    }

    fn mark(app: &mut ModelApp, node: &str) -> Option<Status> {
        marks(app).into_iter().find(|(n, ..)| *n == node).and_then(|(_, st, _)| st)
    }

    /// The setup's progress on the tree from an empty model to a solved one: one next stage at a
    /// time, a warning where a command cannot run yet, ghost rows on nodes still lacking theirs,
    /// and no mark on nodes that are no setup stage.
    #[test]
    fn nodes_mark_the_setup_progress() {
        let mut app = ModelApp::new(Session::default());
        let m = marks(&mut app);
        assert_eq!(m[0], ("geometry", Some(Next), vec!["geometry.open"]));
        assert_eq!(m[1], ("mesh", Some(Blocked), vec!["mesh.tet.generate"]));
        assert_eq!(mark(&mut app, "materials"), Some(Todo));
        assert_eq!(mark(&mut app, "results"), Some(Blocked));
        assert_eq!(mark(&mut app, "interactions"), None);
        assert_eq!(mark(&mut app, "fluids"), None);
        let tip = node_state(&app.setup_stages(), "mesh").map(|(_, tip)| tip).unwrap_or_default();
        assert!(tip.starts_with("Needs geometry") && tip.ends_with("— Geometry › Open Design"), "{tip}");
        let next = |app: &mut ModelApp| marks(app).into_iter().filter(|(_, st, _)| *st == Some(Next)).map(|(n, ..)| n).collect::<Vec<_>>();
        assert_eq!(next(&mut app), ["geometry"]);

        // A design: meshing is next.
        let dir = std::env::temp_dir().join(format!("fieldcraft-browser-marks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let design = dir.join("plate.solidcraft");
        std::fs::write(&design, fieldcraft_geometry::sample::plate_design(&fieldcraft_geometry::sample::Plate::default()).unwrap()).unwrap();
        app.run("geometry.open", json!({"path": design.to_string_lossy()})).unwrap();
        assert_eq!(marks(&mut app)[0], ("geometry", Some(Done), vec![]));
        assert_eq!(next(&mut app), ["mesh"]);

        // Meshed: the material is next; the section waits for it.
        let body = app.session.design.as_ref().unwrap().bodies[0].name.clone();
        app.run("mesh.tet.generate", json!({"body": body, "tolerance": 2.0})).unwrap();
        assert_eq!(mark(&mut app, "mesh"), Some(Done));
        assert_eq!(next(&mut app), ["materials"]);
        assert_eq!(mark(&mut app, "sections"), Some(Blocked));
        let _ = std::fs::remove_dir_all(&dir);

        // Completely defined: solving is next, under Results.
        let mut app = ModelApp::new(Session::default());
        app.run_script(&fieldcraft_engine::sample::cantilever()).unwrap();
        for (node, st, add) in marks(&mut app) {
            match node {
                "results" => assert_eq!((st, add), (Some(Next), vec!["solve.run"])),
                "interactions" | "fluids" => assert_eq!(st, None),
                _ => assert_eq!((node, st, add), (node, Some(Done), vec![])),
            }
        }
        // Solved: every stage done, no ghost rows left.
        app.run("solve.run", json!({})).unwrap();
        assert!(marks(&mut app).iter().all(|(_, st, add)| matches!(st, None | Some(Done)) && add.is_empty()), "{:?}", marks(&mut app));
        let v = describe(&mut app);
        assert_eq!(v[2], json!({"node": "sets", "state": "done", "tip": "Done", "add": []}));
    }

    #[test]
    fn definitions_read_back_as_their_parameters() {
        let mut app = ModelApp::new(Session::default());
        app.run_script(&fieldcraft_engine::sample::cantilever()).unwrap();
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
            Def::Contact,
            Def::FluidDomain,
        ] {
            for verb in ["create", "edit", "delete"] {
                assert!(fieldcraft_engine::find_command(&kind.command(verb)).is_some(), "{kind:?} {verb}");
            }
            assert!(fieldcraft_engine::catalogue::find(kind.icon()).is_some());
        }
    }
}
