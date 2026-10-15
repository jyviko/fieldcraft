//! A command's form drawn from its parameter schema ([`fieldcraft_engine::schema`]): typed fields
//! with units, drop-downs of the model's items, groups and one-of variants. The form edits the
//! command's JSON parameters in place; it never builds anything the command does not take.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Id, RichText, Ui};
use fieldcraft_engine::schema::{self, Collection, Field, Kind, MAX_DEPTH, MAX_ROWS, join};
use fieldcraft_model::UnitSystem;
use serde_json::{Map, Value, json};

use crate::fields::{self, NumberEdit, Style};
use crate::theme::Tokens;

/// The label column.
const LABEL_W: f32 = 112.0;
/// A field without its unit.
const FIELD_W: f32 = 120.0;
/// Indentation of a group's fields.
const INDENT: f32 = 10.0;

/// What the form reads beside the parameters.
pub struct Context<'a> {
    pub units: UnitSystem,
    /// The names of a collection's items.
    pub names: &'a dyn Fn(Collection) -> Vec<String>,
    /// A salt for the widget ids: the open form (its command and the opening), so no widget
    /// state is shared with another form.
    pub salt: Id,
    /// The fields the user has edited: their problems show beside them; the others wait.
    pub touched: &'a BTreeSet<String>,
    /// OK was tried: every problem shows.
    pub attempted: bool,
    /// The command refused a parameter: its path and the problem, shown beside the field.
    pub refused: Option<(&'a str, &'a str)>,
    /// What is missing or wrong in the parameters ([`messages`]).
    pub problems: &'a [(String, String)],
}

/// What a frame of the form did.
#[derive(Debug, Default)]
pub struct Outcome {
    pub changed: bool,
    /// Fields whose typed text is not a value yet (OK waits for them).
    pub invalid: usize,
    /// A reference field's "Create …" was clicked: the collection to add to and the field's path.
    pub create: Option<(Collection, String)>,
    /// The paths of the fields edited this frame.
    pub touched: Vec<String>,
}

struct Form<'a> {
    cx: &'a Context<'a>,
    style: Style,
    /// Each field's problem, by path, phrased with its label ([`messages`]).
    problems: BTreeMap<String, String>,
    names: BTreeMap<&'static str, Vec<String>>,
    out: Outcome,
}

pub fn style(t: &Tokens) -> Style {
    Style { label_width: LABEL_W, field_width: FIELD_W, dim: t.text_dim, error: t.error, required: t.accent }
}

/// Draw `fields` editing `params` (an object).
pub fn show(ui: &mut Ui, t: &Tokens, cx: &Context, fields: &'static [Field], params: &mut Map<String, Value>) -> Outcome {
    let problems = cx.problems.iter().cloned().collect();
    let mut f = Form { cx, style: style(t), problems, names: BTreeMap::new(), out: Outcome::default() };
    ui.spacing_mut().item_spacing.y = 3.0;
    f.fields(ui, fields, fields, params, "", 0);
    f.out
}

impl Form<'_> {
    /// The problem with the field at `path`, under it: the command's refusal at once, the
    /// form's own once the field was edited or OK was tried.
    fn problem_note(&self, ui: &mut Ui, path: &str, label: &str) {
        let message = match self.cx.refused {
            Some((key, problem)) if key == path => Some(format!("{label}: {problem}")),
            _ => self.problems.get(path).filter(|_| self.cx.attempted || self.cx.touched.contains(path)).cloned(),
        };
        if let Some(message) = message {
            fields::note(ui, &self.style, &message, self.style.error);
        }
    }

    fn touch(&mut self, path: &str) {
        self.out.changed = true;
        self.out.touched.push(path.to_string());
    }

    fn id(&self, path: &str) -> Id {
        Id::new(("fc_form", self.cx.salt, path))
    }

    /// A value the fields cannot show: it stays as it is, editable in the JSON view.
    fn json_only(&self, ui: &mut Ui) {
        ui.label(RichText::new("set in JSON").small().color(self.style.dim)).on_hover_text("Edit it in the JSON view");
    }

    fn names(&mut self, c: Collection) -> &[String] {
        self.names.entry(c.label()).or_insert_with(|| (self.cx.names)(c))
    }

    fn unit(&self, dim: schema::Dim) -> Option<String> {
        dim.unit(self.cx.units)
    }

    /// The fields of one object. `common` are the fields beside a flattened variant (kept when it
    /// switches).
    fn fields(&mut self, ui: &mut Ui, fields: &[Field], common: &[Field], obj: &mut Map<String, Value>, prefix: &str, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for f in fields {
            if let Kind::Variants { tag, variants } = f.kind
                && f.key.is_empty()
            {
                let current = obj.get(tag).and_then(Value::as_str).map(str::to_string);
                let path = join(prefix, tag);
                if let Some(t) = self.variant_choice(ui, f, variants, current.as_deref(), &path) {
                    *obj = schema::switch_variant(f, common, obj, &t);
                    self.touch(&path);
                }
                self.problem_note(ui, &path, f.label);
                if let Some(v) = obj.get(tag).and_then(Value::as_str).and_then(|t| f.kind.variant(t)) {
                    self.fields(ui, v.fields, common, obj, prefix, depth + 1);
                }
                continue;
            }
            self.field(ui, f, obj, prefix, depth);
        }
    }

    /// A Type row: the variant's drop-down. Returns the tag picked.
    fn variant_choice(&mut self, ui: &mut Ui, f: &Field, variants: &[schema::Variant], current: Option<&str>, path: &str) -> Option<String> {
        let selected = variants.iter().find(|v| Some(v.tag) == current).map_or(current.unwrap_or("—"), |v| v.label);
        let labels: Vec<&str> = variants.iter().map(|v| v.label).collect();
        let id = self.id(path);
        let picked = fields::row(ui, &self.style, f.label, f.required, |ui| fields::combo(ui, id, selected, &labels, FIELD_W + 40.0));
        picked.and_then(|i| variants.get(i)).map(|v| v.tag.to_string()).filter(|t| Some(t.as_str()) != current)
    }

    fn field(&mut self, ui: &mut Ui, f: &Field, obj: &mut Map<String, Value>, prefix: &str, depth: usize) {
        let path = join(prefix, f.key);
        match f.kind {
            Kind::Group(_) | Kind::Variants { .. } | Kind::Rows { .. } => self.section(ui, f, obj, &path, depth),
            _ => {
                let value = obj.get(f.key).cloned();
                let style = self.style;
                let new = fields::row(ui, &style, f.label, f.required, |ui| self.scalar(ui, f, value.as_ref(), &path));
                if let Some(new) = new {
                    match new {
                        Value::Null => obj.remove(f.key),
                        v => obj.insert(f.key.into(), v),
                    };
                    self.touch(&path);
                }
                if let Kind::Either(options) = f.kind {
                    self.either_value(ui, f, options, obj, &path);
                }
            }
        }
        self.problem_note(ui, &path, f.label);
    }

    /// A one-row field; returns its new value (`Null` removes it).
    fn scalar(&mut self, ui: &mut Ui, f: &Field, value: Option<&Value>, path: &str) -> Option<Value> {
        let id = self.id(path);
        let s = self.style;
        match f.kind {
            Kind::Number { dim, .. } => {
                let unit = self.unit(dim);
                let out = self.number(ui, id, value, f.hint, false, FIELD_W);
                fields::unit(ui, &s, unit.as_deref());
                out
            }
            Kind::Integer { .. } => self.number(ui, id, value, f.hint, true, FIELD_W),
            Kind::Text | Kind::Path { .. } => {
                let mut text = value.and_then(Value::as_str).unwrap_or_default().to_string();
                fields::text(ui, &s, id, &mut text, f.hint, FIELD_W + 40.0)
                    .then(|| if text.is_empty() && !f.required { Value::Null } else { json!(text) })
            }
            Kind::Bool => {
                let mut on = value.and_then(Value::as_bool).unwrap_or(false);
                let r = ui.checkbox(&mut on, "");
                if !f.hint.is_empty() {
                    r.clone().on_hover_text(f.hint);
                }
                r.changed().then(|| json!(on))
            }
            Kind::Choice(options) => {
                let current = value.and_then(Value::as_str);
                let selected = options
                    .iter()
                    .find(|(v, _)| Some(*v) == current)
                    .map_or(current.unwrap_or(if f.hint.is_empty() { "—" } else { f.hint }), |(_, l)| *l);
                let mut labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
                let optional = !f.required;
                if optional {
                    labels.push("(default)");
                }
                fields::combo(ui, id, selected, &labels, FIELD_W + 40.0).map(|i| options.get(i).map_or(Value::Null, |(v, _)| json!(v)))
            }
            Kind::Vector { dim, labels, nullable } => self.vector(ui, id, value, dim, labels, nullable),
            Kind::Ref(c) => self.reference(ui, id, path, value, c, f.hint, f.required),
            Kind::RefList(c) => self.reference_list(ui, path, value, c),
            Kind::Either(options) => {
                let key = match value {
                    Some(Value::String(s)) => Some(s.as_str()),
                    Some(Value::Object(o)) => o.keys().next().map(String::as_str),
                    _ => None,
                };
                let selected = options.iter().find(|o| Some(o.key) == key).map_or(if f.hint.is_empty() { "—" } else { f.hint }, |o| o.label);
                let mut labels: Vec<&str> = options.iter().map(|o| o.label).collect();
                if !f.required {
                    labels.push("(none)");
                }
                fields::combo(ui, id, selected, &labels, FIELD_W + 40.0).and_then(|i| match options.get(i) {
                    Some(o) if Some(o.key) == key => None,
                    Some(o) if matches!(o.kind, Kind::Unit) => Some(json!(o.key)),
                    Some(o) => Some(json!({o.key: schema::prefill(&[*o], &json!({})).get(o.key).cloned().unwrap_or(Value::Null)})),
                    None => Some(Value::Null),
                })
            }
            Kind::Json => self.json(ui, id, value),
            Kind::Unit | Kind::Group(_) | Kind::Variants { .. } | Kind::Rows { .. } => None,
        }
    }

    /// The chosen option's value of an [`Kind::Either`] field, on its own row.
    fn either_value(&mut self, ui: &mut Ui, f: &Field, options: &[Field], obj: &mut Map<String, Value>, path: &str) {
        let Some(Value::Object(inner)) = obj.get(f.key).cloned() else { return };
        let Some((key, value)) = inner.into_iter().next() else { return };
        let Some(option) = options.iter().find(|o| o.key == key) else { return };
        let sub = join(path, &key);
        let style = self.style;
        let new = fields::row(ui, &style, "", false, |ui| self.scalar(ui, option, Some(&value), &sub));
        if let Some(new) = new {
            obj.insert(f.key.into(), json!({key: new}));
            self.touch(path);
        }
    }

    fn number(&mut self, ui: &mut Ui, id: Id, value: Option<&Value>, hint: &str, integer: bool, width: f32) -> Option<Value> {
        let current = value.and_then(Value::as_f64);
        if value.is_some_and(|v| !v.is_null() && current.is_none()) {
            self.json_only(ui);
            return None;
        }
        match fields::number(ui, &self.style, id, current, hint, integer, width) {
            NumberEdit::Set(Some(x)) if integer => Some(json!(x as i64)),
            NumberEdit::Set(Some(x)) => Some(json!(x)),
            NumberEdit::Set(None) => Some(Value::Null),
            NumberEdit::Invalid => {
                self.out.invalid += 1;
                None
            }
            NumberEdit::Unchanged => None,
        }
    }

    fn vector(&mut self, ui: &mut Ui, id: Id, value: Option<&Value>, dim: schema::Dim, labels: &[&str], nullable: bool) -> Option<Value> {
        let current: Vec<Value> = match value {
            Some(Value::Array(a)) if a.len() == labels.len() => a.clone(),
            Some(v) if !v.is_null() => {
                self.json_only(ui);
                return None;
            }
            // Empty slots show their placeholder (x, y, z), not a 0 nobody typed.
            _ => vec![Value::Null; labels.len()],
        };
        let mut next = current.clone();
        let mut changed = false;
        let width = if labels.len() > 3 { 34.0 } else { 44.0 };
        let unit = self.unit(dim);
        ui.vertical(|ui| {
            for chunk in (0..labels.len()).collect::<Vec<_>>().chunks(3) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for &i in chunk {
                        let hint = labels.get(i).copied().unwrap_or("");
                        let slot = current.get(i).cloned().unwrap_or(Value::Null);
                        let shown = (!slot.is_null()).then_some(&slot);
                        if let Some(v) = self.number(ui, id.with(i), shown, hint, false, width) {
                            if let Some(n) = next.get_mut(i) {
                                *n = v;
                            }
                            changed = true;
                        }
                    }
                    if chunk.first() == Some(&0) {
                        fields::unit(ui, &self.style, unit.as_deref());
                    }
                });
            }
        });
        // A vector left empty is not written until edited (a required one shows as missing).
        // The slots left empty of a vector that takes no blanks are 0.
        changed.then(|| Value::Array(next.into_iter().map(|x| if x.is_null() && !nullable { json!(0.0) } else { x }).collect()))
    }

    /// A drop-down of the collection's items; never free text. With none, the drop-down is
    /// greyed and a button opens the command that makes one ([`Outcome::create`]).
    #[allow(clippy::too_many_arguments)]
    fn reference(&mut self, ui: &mut Ui, id: Id, path: &str, value: Option<&Value>, c: Collection, hint: &str, required: bool) -> Option<Value> {
        let current = value.and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
        let names = self.names(c).to_vec();
        if names.is_empty() {
            ui.vertical(|ui| {
                let none = format!("No {} yet", plural(c.label()));
                ui.add_enabled_ui(false, |ui| fields::combo(ui, id, current.as_deref().unwrap_or(&none), &[], FIELD_W + 40.0));
                self.create_button(ui, c, path);
            });
            return None;
        }
        let empty = if hint.is_empty() { "—" } else { hint };
        let selected = current.clone().unwrap_or_else(|| empty.to_string());
        let mut labels: Vec<&str> = names.iter().map(String::as_str).collect();
        if !required {
            labels.push("(default)");
        }
        fields::combo(ui, id, &selected, &labels, FIELD_W + 40.0).map(|i| names.get(i).map_or(Value::Null, |n| json!(n)))
    }

    fn reference_list(&mut self, ui: &mut Ui, path: &str, value: Option<&Value>, c: Collection) -> Option<Value> {
        let chosen: Vec<String> =
            value.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let names = self.names(c).to_vec();
        let mut changed = None;
        ui.vertical(|ui| {
            if names.is_empty() {
                ui.label(RichText::new(format!("No {} yet", plural(c.label()))).color(self.style.dim));
                self.create_button(ui, c, path);
            }
            for n in names.iter().take(MAX_ROWS) {
                let mut on = chosen.contains(n);
                if ui.checkbox(&mut on, n).changed() {
                    let mut next: Vec<String> = names.iter().filter(|m| if *m == n { on } else { chosen.contains(m) }).cloned().collect();
                    next.dedup();
                    changed = Some(if next.is_empty() { Value::Null } else { json!(next) });
                }
            }
        });
        changed
    }

    /// "Create material…": opens the command that makes an item of `c`, then returns here.
    fn create_button(&mut self, ui: &mut Ui, c: Collection, path: &str) {
        let r = c.requirement();
        let text = match r.provider().ends_with(".create") {
            true => format!("Create {}…", c.label()),
            false => format!("{}…", fieldcraft_engine::catalogue::entry_of(r.provider()).map_or(r.provider(), |e| e.label)),
        };
        if ui.button(text).on_hover_text(format!("{} ({})", r.place(), r.provider())).clicked() {
            self.out.create = Some((c, path.to_string()));
        }
    }

    /// A JSON value typed as text: kept while it does not parse.
    fn json(&mut self, ui: &mut Ui, id: Id, value: Option<&Value>) -> Option<Value> {
        let shown = value.map(Value::to_string).unwrap_or_default();
        let mut text = ui.data(|d| d.get_temp::<(String, String)>(id)).filter(|(_, over)| *over == shown).map_or_else(|| shown.clone(), |(t, _)| t);
        let bad = !text.trim().is_empty() && serde_json::from_str::<Value>(&text).is_err();
        let mut edit = egui::TextEdit::singleline(&mut text).id(id).code_editor().desired_width(FIELD_W + 40.0);
        if bad {
            edit = edit.text_color(self.style.error);
            self.out.invalid += 1;
        }
        let r = ui.add(edit);
        let mut out = None;
        if r.changed() {
            if text.trim().is_empty() {
                out = Some(Value::Null);
            } else if let Ok(v) = serde_json::from_str::<Value>(&text) {
                out = Some(v);
            }
        }
        let over = out.as_ref().map_or(shown, |v| if v.is_null() { String::new() } else { v.to_string() });
        ui.data_mut(|d| d.insert_temp(id, (text, over)));
        out
    }

    /// A group, a keyed variant or a list of rows: a sub-heading (with a checkbox when optional),
    /// then its fields, indented.
    fn section(&mut self, ui: &mut Ui, f: &Field, obj: &mut Map<String, Value>, path: &str, depth: usize) {
        let present = obj.get(f.key).is_some_and(|v| !v.is_null());
        let mut on = present;
        ui.add_space(2.0);
        if f.required {
            ui.label(RichText::new(f.label).strong());
        } else {
            ui.checkbox(&mut on, RichText::new(f.label).strong());
        }
        if !f.hint.is_empty() && !present {
            fields::note(ui, &Style { label_width: INDENT, ..self.style }, f.hint, self.style.dim);
        }
        if on != present {
            if on {
                let filled = schema::prefill(&[Field { required: true, ..*f }], &json!({}));
                let start = filled.get(f.key).cloned().unwrap_or_else(|| if matches!(f.kind, Kind::Rows { .. }) { json!([]) } else { json!({}) });
                obj.insert(f.key.into(), start);
            } else {
                obj.remove(f.key);
            }
            self.touch(path);
        }
        if !on {
            return;
        }
        ui.horizontal(|ui| {
            ui.add_space(INDENT);
            ui.vertical(|ui| match f.kind {
                Kind::Group(fields) => {
                    if let Some(Value::Object(child)) = obj.get_mut(f.key) {
                        self.fields(ui, fields, fields, child, path, depth + 1);
                    }
                }
                Kind::Variants { tag, variants } => {
                    let Some(Value::Object(child)) = obj.get_mut(f.key) else {
                        self.json_only(ui);
                        return;
                    };
                    let current = child.get(tag).and_then(Value::as_str).map(str::to_string);
                    let row = Field { label: "Type", required: true, ..*f };
                    let tag_path = join(path, tag);
                    if let Some(t) = self.variant_choice(ui, &row, variants, current.as_deref(), &tag_path) {
                        *child = schema::switch_variant(f, &[], child, &t);
                        self.touch(&tag_path);
                    }
                    self.problem_note(ui, &tag_path, row.label);
                    if let Some(v) = child.get(tag).and_then(Value::as_str).and_then(|t| f.kind.variant(t)) {
                        self.fields(ui, v.fields, v.fields, child, path, depth + 1);
                    }
                }
                Kind::Rows { fields, max } => self.rows(ui, fields, max, obj, f.key, path, depth),
                _ => {}
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn rows(&mut self, ui: &mut Ui, fields: &[Field], max: usize, obj: &mut Map<String, Value>, key: &str, path: &str, depth: usize) {
        let Some(Value::Array(list)) = obj.get_mut(key) else {
            self.json_only(ui);
            return;
        };
        let mut remove = None;
        for (i, row) in list.iter_mut().enumerate().take(max) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}", i + 1)).small().color(self.style.dim));
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
            });
            if let Value::Object(child) = row {
                self.fields(ui, fields, fields, child, &format!("{path}/{i}"), depth + 1);
            }
        }
        if let Some(i) = remove {
            list.remove(i);
            self.touch(path);
        }
        if list.len() < max && ui.small_button("+ Add").clicked() {
            list.push(schema::prefill(fields, &json!({})));
            self.touch(path);
        }
    }
}

/// What is missing or wrong in `params`, in the form's order: (field path, message), each
/// phrased with the field's label as the form shows it ("Material: choose a material"), never
/// its JSON key.
pub fn messages(fields: &'static [Field], params: &Map<String, Value>) -> Vec<(String, String)> {
    schema::object_problems(fields, params)
        .into_iter()
        .map(|p| {
            let message = field_at(fields, &p.path).map_or_else(|| format!("{}: {}", p.path, p.message), |f| phrase(f, &p.message));
            (p.path, message)
        })
        .collect()
}

/// The field at `path` (`elastic/young`, `rows/0/name`). A variant's tag gives its
/// [`Kind::Variants`] field; an [`Kind::Either`] option's value gives the option.
pub fn field_at(fields: &'static [Field], path: &str) -> Option<&'static Field> {
    let mut segments = path.split('/');
    let mut field = find(fields, segments.next()?)?;
    for (depth, segment) in segments.enumerate() {
        if depth > MAX_DEPTH {
            return None;
        }
        field = match field.kind {
            Kind::Rows { .. } if segment.parse::<usize>().is_ok() => field,
            Kind::Group(inner) | Kind::Rows { fields: inner, .. } => find(inner, segment)?,
            Kind::Variants { tag, .. } if tag == segment => field,
            Kind::Variants { variants, .. } => variants.iter().find_map(|v| find(v.fields, segment))?,
            Kind::Either(options) => options.iter().find(|o| o.key == segment)?,
            _ => return None,
        };
    }
    Some(field)
}

/// The field of `fields` with key `key`, looking into flattened variants.
fn find(fields: &'static [Field], key: &str) -> Option<&'static Field> {
    fields.iter().find_map(|f| match f.kind {
        Kind::Variants { tag, variants } if f.key.is_empty() => {
            if tag == key {
                Some(f)
            } else {
                variants.iter().find_map(|v| find(v.fields, key))
            }
        }
        _ => (f.key == key).then_some(f),
    })
}

/// A schema problem ([`schema::Problem::message`]) of field `f`, as the form says it.
fn phrase(f: &Field, problem: &str) -> String {
    let required = problem == "required";
    let text = match f.kind {
        Kind::Ref(c) if required => format!("choose {}", article(c.label())),
        Kind::RefList(c) if required => format!("choose at least one {}", c.label()),
        Kind::Either(options) => format!("choose {}", options.iter().map(|o| article(&lower(o.label))).collect::<Vec<_>>().join(" or ")),
        Kind::Choice(options) => format!("choose one of {}", options.iter().map(|(_, l)| *l).collect::<Vec<_>>().join(", ")),
        Kind::Variants { .. } => format!("choose {}", article(&lower(f.label))),
        Kind::Path { .. } if required => "choose a file".into(),
        Kind::Text if required && f.key.ends_with("name") => "enter a name".into(),
        _ if required => "enter a value".into(),
        _ => format!("must be {problem}"),
    };
    format!("{}: {text}", f.label)
}

/// "a material", "an element".
fn article(noun: &str) -> String {
    let an = noun.starts_with(['a', 'e', 'i', 'o', 'u']);
    format!("{} {noun}", if an { "an" } else { "a" })
}

/// A label lower-cased, but not its acronyms ("Reference point" → "reference point", "RBE2" stays).
fn lower(label: &str) -> String {
    let mut chars = label.chars();
    match (chars.next(), chars.next()) {
        (Some(c), Some(d)) if d.is_lowercase() => c.to_lowercase().chain(label.chars().skip(1)).collect(),
        _ => label.to_string(),
    }
}

/// A collection's label in the plural: "materials", "point masses", "bodies".
fn plural(label: &str) -> String {
    if let Some(stem) = label.strip_suffix('y') {
        format!("{stem}ies")
    } else if label.ends_with('s') {
        format!("{label}es")
    } else {
        format!("{label}s")
    }
}

/// Fill each empty required reference that has one obvious candidate, `pick(collection)` (the
/// browser's selection, or a collection's only item). The form starts with it, so the fields and
/// the JSON view agree. Returns whether anything was filled.
pub fn preselect(fields: &[Field], obj: &mut Map<String, Value>, pick: &dyn Fn(Collection) -> Option<String>) -> bool {
    preselect_in(fields, obj, pick, 0)
}

fn preselect_in(fields: &[Field], obj: &mut Map<String, Value>, pick: &dyn Fn(Collection) -> Option<String>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut changed = false;
    for f in fields {
        match f.kind {
            Kind::Variants { tag, .. } if f.key.is_empty() => {
                if let Some(v) = obj.get(tag).and_then(Value::as_str).and_then(|t| f.kind.variant(t)) {
                    changed |= preselect_in(v.fields, obj, pick, depth + 1);
                }
            }
            Kind::Group(inner) => {
                if let Some(Value::Object(child)) = obj.get_mut(f.key) {
                    changed |= preselect_in(inner, child, pick, depth + 1);
                }
            }
            Kind::Variants { tag, .. } => {
                if let Some(Value::Object(child)) = obj.get_mut(f.key)
                    && let Some(v) = child.get(tag).and_then(Value::as_str).and_then(|t| f.kind.variant(t))
                {
                    changed |= preselect_in(v.fields, child, pick, depth + 1);
                }
            }
            Kind::Ref(c) if f.required && obj.get(f.key).is_none_or(Value::is_null) => {
                if let Some(name) = pick(c) {
                    obj.insert(f.key.into(), json!(name));
                    changed = true;
                }
            }
            _ => {}
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command's form draws, with its defaults and with hostile parameters, with and without
    /// items to reference, without panicking, and drawing alone changes nothing.
    #[test]
    fn every_form_draws() {
        let ctx = egui::Context::default();
        let one = |c: Collection| vec![format!("{}1", c.label())];
        let none = |_: Collection| Vec::new();
        let hostile =
            [json!({}), json!({"type": 7, "name": [], "elastic": "x", "target": {"set": 1, "reference_point": 2}, "values": [1], "geometry": {}})];
        for (c, names) in fieldcraft_engine::commands().iter().flat_map(|c| [(c, &one as &dyn Fn(Collection) -> Vec<String>), (c, &none)]) {
            let Some(form) = c.form else { continue };
            for start in hostile.iter().chain([&schema::prefill(form, &json!({}))]) {
                let mut params = start.as_object().cloned().unwrap_or_default();
                let before = params.clone();
                let mut outcome = Outcome::default();
                let width = crate::context::WIDTH - 16.0;
                let mut used = 0.0;
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let touched = BTreeSet::new();
                    let cx = Context {
                        units: UnitSystem::MmTonneS,
                        names,
                        salt: Id::new(c.id),
                        touched: &touched,
                        attempted: true,
                        refused: None,
                        problems: &messages(form, &params),
                    };
                    ui.vertical(|ui| {
                        ui.set_max_width(width);
                        outcome = show(ui, &Tokens::get(false), &cx, form, &mut params);
                        used = ui.min_rect().width();
                    });
                });
                // Wider would push the context panel over the viewport.
                assert!(used <= width + 0.5, "{}: {used}", c.id);
                out.textures_delta.clear();
                assert!(!outcome.changed, "{}", c.id);
                assert_eq!(params, before, "{}", c.id);
            }
        }
    }

    /// In every form, each required field left empty is a problem, so OK waits, and the message
    /// names the field by its label, never by a JSON key.
    #[test]
    fn every_required_field_left_empty_is_named_by_its_label() {
        let mut checked = 0;
        for c in fieldcraft_engine::commands() {
            let Some(form) = c.form else { continue };
            let full = schema::prefill(form, &json!({}));
            let Some(full) = full.as_object() else { continue };
            for (path, message) in messages(form, full) {
                assert!(!message.contains('`'), "{}: {path}: {message}", c.id);
            }
            for f in form.iter().filter(|f| f.required) {
                let key = match f.kind {
                    Kind::Variants { tag, .. } if f.key.is_empty() => tag,
                    _ => f.key,
                };
                let mut params = full.clone();
                params.remove(key);
                let found: BTreeMap<String, String> = messages(form, &params).into_iter().collect();
                let message = found.get(key).unwrap_or_else(|| panic!("{}: `{key}` empty is no problem: {found:?}", c.id));
                assert!(message.starts_with(&format!("{}: ", f.label)), "{}: {message}", c.id);
                for m in found.values() {
                    assert!(!m.contains('`'), "{}: {m}", c.id);
                }
                checked += 1;
            }
        }
        assert!(checked > 50, "{checked}");
    }

    #[test]
    fn messages_read_as_the_form_shows_them() {
        let plasticity = fieldcraft_engine::find_command("model.material.plasticity").and_then(|c| c.form).unwrap();
        let m = messages(plasticity, &Map::new());
        assert_eq!(m.first(), Some(&("name".to_string(), "Material: choose a material".to_string())), "{m:?}");
        let load = fieldcraft_engine::find_command("model.load.create").and_then(|c| c.form).unwrap();
        let m: Vec<String> = messages(load, &json!({"type": "force"}).as_object().cloned().unwrap_or_default()).into_iter().map(|(_, m)| m).collect();
        assert_eq!(m, ["Name: enter a name", "Target: choose a set or a reference point", "Force: enter a value"]);
    }

    /// A problem waits until its field is edited or OK is tried; a refused parameter shows at once.
    #[test]
    fn problems_show_once_touched_or_attempted() {
        let form = fieldcraft_engine::find_command("model.material.plasticity").and_then(|c| c.form).unwrap();
        let ctx = egui::Context::default();
        let draw = |touched: &BTreeSet<String>, attempted: bool, refused: Option<(&str, &str)>| {
            let mut shown = String::new();
            let mut params = Map::new();
            let names = |_: Collection| Vec::new();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let cx = Context {
                    units: UnitSystem::MmTonneS,
                    names: &names,
                    salt: Id::new("t"),
                    touched,
                    attempted,
                    refused,
                    problems: &messages(form, &params),
                };
                show(ui, &Tokens::get(false), &cx, form, &mut params);
            });
            out.textures_delta.clear();
            // The text drawn this frame.
            for clipped in &out.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    shown.push_str(t.galley.text());
                    shown.push('\n');
                }
            }
            shown
        };
        let none = BTreeSet::new();
        assert!(!draw(&none, false, None).contains("choose a material"));
        assert!(draw(&none, true, None).contains("Material: choose a material"));
        let touched: BTreeSet<String> = ["name".to_string()].into();
        assert!(draw(&touched, false, None).contains("Material: choose a material"));
        let refused = draw(&none, false, Some(("name", "names no material")));
        assert!(refused.contains("Material: names no material") && !refused.contains("choose a material"), "{refused}");
    }
}
