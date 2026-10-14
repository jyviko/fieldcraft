//! Typed parameter schemas: what a command's JSON parameters are, field by field.
//!
//! A command's [`CommandSpec::form`](crate::CommandSpec::form) lists its [`Field`]s: name, kind,
//! default, whether it is required, its unit dimension, its choices, the model collection it
//! names, nested groups and one-of variants. The schema describes the JSON the command already
//! takes; it never changes it. The UI draws its command forms from it, `fieldcraft-cli commands
//! --json` and the MCP `list_commands` tool give it as JSON Schema ([`json_schema`]).
//!
//! The schema is descriptive: the command's `run` stays the judge of its parameters. What the
//! schema cannot express (a mass given per direction, a table of load-path points) stays reachable
//! through the form's JSON view.

use fieldcraft_model::UnitSystem;
use serde_json::{Map, Value, json};

use crate::Session;

/// Rows a list field holds at most in a form.
pub const MAX_ROWS: usize = 1000;
/// Names a reference list offers at most.
pub const MAX_NAMES: usize = 10_000;

/// One parameter of a command.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// Its JSON key. Empty for [`Kind::Variants`] whose tag and fields sit in the parent object
    /// (an internally tagged definition such as a load: `{"name", "type": "force", "vector"}`).
    pub key: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub required: bool,
    /// The value the form starts with, as JSON text; `None` leaves the field empty.
    pub default: Option<&'static str>,
    /// What an empty field means ("every body", "1/25 of the diagonal"), or a short note.
    pub hint: &'static str,
}

/// What a field holds.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// A number in the model's unit system (or a fixed unit such as degrees).
    Number {
        dim: Dim,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// A whole number in `min..=max`.
    Integer {
        min: i64,
        max: i64,
    },
    Text,
    /// A file path; `save` when the command writes it.
    Path {
        save: bool,
    },
    Bool,
    /// One of these strings: (value, label).
    Choice(&'static [(&'static str, &'static str)]),
    /// A fixed-length list of numbers, one per label (`["x", "y", "z"]`); `nullable` entries may
    /// be null (a free degree of freedom).
    Vector {
        dim: Dim,
        labels: &'static [&'static str],
        nullable: bool,
    },
    /// The name of an item of the model or the open design.
    Ref(Collection),
    /// A list of such names.
    RefList(Collection),
    /// A nested object.
    Group(&'static [Field]),
    /// A nested object (or, with an empty key, the parent object) tagged by `tag`: the tag's value
    /// picks the variant whose fields apply.
    Variants {
        tag: &'static str,
        variants: &'static [Variant],
    },
    /// An object with exactly one of these keys (`{"set": …}` or `{"reference_point": …}`), or a
    /// bare string for a [`Kind::Unit`] option.
    Either(&'static [Field]),
    /// An option of [`Kind::Either`] written as its key alone (`"line"`).
    Unit,
    /// A list of objects with these fields, at most `max` long.
    Rows {
        fields: &'static [Field],
        max: usize,
    },
    /// Any JSON: what the schema does not describe.
    Json,
}

/// One variant of [`Kind::Variants`].
#[derive(Debug, Clone, Copy)]
pub struct Variant {
    pub tag: &'static str,
    pub label: &'static str,
    pub fields: &'static [Field],
}

/// Where a reference field's names come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    Set,
    Material,
    Section,
    ReferencePoint,
    Coupling,
    Contact,
    Tie,
    Mass,
    Spring,
    BoundaryCondition,
    Load,
    FluidDomain,
    Step,
    /// A solved step.
    SolvedStep,
    /// A body of the open design.
    Body,
    /// A datum plane or axis.
    Datum,
    /// A datum plane.
    DatumPlane,
    Partition,
}

impl Collection {
    pub fn label(self) -> &'static str {
        match self {
            Collection::Set => "set",
            Collection::Material => "material",
            Collection::Section => "section",
            Collection::ReferencePoint => "reference point",
            Collection::Coupling => "coupling",
            Collection::Contact => "contact",
            Collection::Tie => "tie",
            Collection::Mass => "point mass",
            Collection::Spring => "spring",
            Collection::BoundaryCondition => "boundary condition",
            Collection::Load => "load",
            Collection::FluidDomain => "fluid domain",
            Collection::Step => "step",
            Collection::SolvedStep => "solved step",
            Collection::Body => "body",
            Collection::Datum => "datum",
            Collection::DatumPlane => "datum plane",
            Collection::Partition => "partition",
        }
    }
}

impl Session {
    /// The names a reference field may take, in order (at most [`MAX_NAMES`]).
    pub fn names(&self, c: Collection) -> Vec<String> {
        let m = &self.model;
        let names: Box<dyn Iterator<Item = &str>> = match c {
            Collection::Set => Box::new(m.sets().keys().map(String::as_str)),
            Collection::Material => Box::new(m.materials().keys().map(String::as_str)),
            Collection::Section => Box::new(m.sections().keys().map(String::as_str)),
            Collection::ReferencePoint => Box::new(m.reference_points().keys().map(String::as_str)),
            Collection::Coupling => Box::new(m.couplings().keys().map(String::as_str)),
            Collection::Contact => Box::new(m.contacts().keys().map(String::as_str)),
            Collection::Tie => Box::new(m.ties().keys().map(String::as_str)),
            Collection::Mass => Box::new(m.masses().keys().map(String::as_str)),
            Collection::Spring => Box::new(m.springs().keys().map(String::as_str)),
            Collection::BoundaryCondition => Box::new(m.boundary_conditions().keys().map(String::as_str)),
            Collection::Load => Box::new(m.loads().keys().map(String::as_str)),
            Collection::FluidDomain => Box::new(m.fluid_domains().keys().map(String::as_str)),
            Collection::Step => Box::new(m.steps().iter().map(|s| s.name.as_str())),
            Collection::SolvedStep => Box::new(self.solved_steps()),
            Collection::Body => Box::new(self.design.iter().flat_map(|d| d.bodies.iter().map(|b| b.name.as_str()))),
            Collection::Datum => Box::new(m.datums().keys().map(String::as_str)),
            Collection::DatumPlane => Box::new(m.datums().iter().filter(|(_, d)| d.plane().is_some()).map(|(n, _)| n.as_str())),
            Collection::Partition => Box::new(m.partitions().iter().map(|p| p.name.as_str())),
        };
        names.take(MAX_NAMES).map(str::to_string).collect()
    }
}

/// A physical dimension: its unit follows the model's unit system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    /// No unit (a ratio, a count, a factor).
    None,
    Length,
    Area,
    Stress,
    Density,
    Force,
    Moment,
    Mass,
    Inertia,
    Time,
    Velocity,
    Acceleration,
    AngularVelocity,
    Stiffness,
    Damping,
    Viscosity,
    ForcePerVolume,
    Power,
    HeatFlux,
    FilmCoefficient,
    Conductivity,
    SpecificHeat,
    Expansion,
    Temperature,
    /// Always millimetres (geometry from SolidCraft).
    Millimetre,
    Degree,
    Hertz,
}

impl Dim {
    /// Its unit in `system`, or `None` (no unit, or the system is unspecified).
    pub fn unit(self, system: UnitSystem) -> Option<String> {
        let fixed = match self {
            Dim::Millimetre => Some("mm"),
            Dim::Degree => Some("°"),
            Dim::Hertz => Some("Hz"),
            Dim::Expansion => Some("1/K"),
            _ => None,
        };
        if let Some(f) = fixed {
            return Some(f.into());
        }
        let (l, m, t, f, s) = match system {
            UnitSystem::Unspecified => return None,
            UnitSystem::MKgS => ("m", "kg", "s", "N", "Pa"),
            UnitSystem::MmTonneS => ("mm", "t", "s", "N", "MPa"),
            UnitSystem::MmKgMs => ("mm", "kg", "ms", "kN", "GPa"),
            UnitSystem::MmGMs => ("mm", "g", "ms", "N", "MPa"),
            UnitSystem::InLbfS => ("in", "lbf·s²/in", "s", "lbf", "psi"),
        };
        let inch = system == UnitSystem::InLbfS;
        Some(match self {
            Dim::Length => l.into(),
            Dim::Area => format!("{l}²"),
            Dim::Stress => s.into(),
            Dim::Density if inch => "lbf·s²/in⁴".into(),
            Dim::Density => format!("{m}/{l}³"),
            Dim::Force => f.into(),
            Dim::Moment => format!("{f}·{l}"),
            Dim::Mass => m.into(),
            Dim::Inertia if inch => "lbf·s²·in".into(),
            Dim::Inertia => format!("{m}·{l}²"),
            Dim::Time => t.into(),
            Dim::Velocity => format!("{l}/{t}"),
            Dim::Acceleration => format!("{l}/{t}²"),
            Dim::AngularVelocity => format!("rad/{t}"),
            Dim::Stiffness => format!("{f}/{l}"),
            Dim::Damping => format!("{f}·{t}/{l}"),
            Dim::Viscosity => format!("{s}·{t}"),
            Dim::ForcePerVolume => format!("{f}/{l}³"),
            Dim::Power => format!("{f}·{l}/{t}"),
            Dim::HeatFlux => format!("{f}/({l}·{t})"),
            Dim::FilmCoefficient => format!("{f}/({l}·{t}·K)"),
            Dim::Conductivity => format!("{f}/({t}·K)"),
            Dim::SpecificHeat => format!("{l}²/({t}²·K)"),
            Dim::None | Dim::Temperature | Dim::Millimetre | Dim::Degree | Dim::Hertz | Dim::Expansion => return None,
        })
    }
}

// ---- Building fields (const, so schemas are static tables) ----

const fn field(key: &'static str, label: &'static str, kind: Kind) -> Field {
    Field { key, label, kind, required: false, default: None, hint: "" }
}

impl Field {
    /// Required.
    pub const fn req(self) -> Field {
        Field { required: true, ..self }
    }
    /// Starts with this JSON value.
    pub const fn def(self, json: &'static str) -> Field {
        Field { default: Some(json), ..self }
    }
    /// What an empty field means.
    pub const fn hint(self, hint: &'static str) -> Field {
        Field { hint, ..self }
    }
}

pub const fn num(key: &'static str, label: &'static str, dim: Dim) -> Field {
    field(key, label, Kind::Number { dim, min: None, max: None })
}
pub const fn range(key: &'static str, label: &'static str, dim: Dim, min: f64, max: f64) -> Field {
    field(key, label, Kind::Number { dim, min: Some(min), max: Some(max) })
}
pub const fn positive(key: &'static str, label: &'static str, dim: Dim) -> Field {
    field(key, label, Kind::Number { dim, min: Some(0.0), max: None })
}
pub const fn int(key: &'static str, label: &'static str, min: i64, max: i64) -> Field {
    field(key, label, Kind::Integer { min, max })
}
pub const fn text(key: &'static str, label: &'static str) -> Field {
    field(key, label, Kind::Text)
}
pub const fn path(key: &'static str, label: &'static str, save: bool) -> Field {
    field(key, label, Kind::Path { save })
}
pub const fn flag(key: &'static str, label: &'static str) -> Field {
    field(key, label, Kind::Bool)
}
pub const fn choice(key: &'static str, label: &'static str, options: &'static [(&'static str, &'static str)]) -> Field {
    field(key, label, Kind::Choice(options))
}
pub const fn xyz(key: &'static str, label: &'static str, dim: Dim) -> Field {
    field(key, label, Kind::Vector { dim, labels: &["x", "y", "z"], nullable: false })
}
/// A vector whose entries may be null (left free).
pub const fn free_vector(key: &'static str, label: &'static str, dim: Dim, labels: &'static [&'static str]) -> Field {
    field(key, label, Kind::Vector { dim, labels, nullable: true })
}
pub const fn vector(key: &'static str, label: &'static str, dim: Dim, labels: &'static [&'static str]) -> Field {
    field(key, label, Kind::Vector { dim, labels, nullable: false })
}
pub const fn reference(key: &'static str, label: &'static str, c: Collection) -> Field {
    field(key, label, Kind::Ref(c))
}
pub const fn references(key: &'static str, label: &'static str, c: Collection) -> Field {
    field(key, label, Kind::RefList(c))
}
pub const fn group(key: &'static str, label: &'static str, fields: &'static [Field]) -> Field {
    field(key, label, Kind::Group(fields))
}
pub const fn variants(key: &'static str, label: &'static str, variants: &'static [Variant]) -> Field {
    field(key, label, Kind::Variants { tag: "type", variants })
}
pub const fn either(key: &'static str, label: &'static str, options: &'static [Field]) -> Field {
    field(key, label, Kind::Either(options))
}
pub const fn unit(key: &'static str, label: &'static str) -> Field {
    field(key, label, Kind::Unit)
}
pub const fn rows(key: &'static str, label: &'static str, fields: &'static [Field]) -> Field {
    field(key, label, Kind::Rows { fields, max: MAX_ROWS })
}
pub const fn json(key: &'static str, label: &'static str) -> Field {
    field(key, label, Kind::Json)
}
pub const fn variant(tag: &'static str, label: &'static str, fields: &'static [Field]) -> Variant {
    Variant { tag, label, fields }
}

/// The `name` of a new or edited definition.
pub const NAME: Field = text("name", "Name").req();

impl Kind {
    /// The variant of `tag`, for [`Kind::Variants`].
    pub fn variant(&self, tag: &str) -> Option<&'static Variant> {
        match self {
            Kind::Variants { variants, .. } => variants.iter().find(|v| v.tag == tag),
            _ => None,
        }
    }
}

// ---- Prefilling, switching variants, checking ----

/// `params` with the defaults of the fields it leaves out filled in (required groups created,
/// the active variant's defaults too). What it already has is kept.
pub fn prefill(fields: &[Field], params: &Value) -> Value {
    let mut obj = params.as_object().cloned().unwrap_or_default();
    fill(fields, &mut obj, 0);
    Value::Object(obj)
}

/// Nesting deeper than this is not followed (schemas are shallow; this bounds the recursion).
pub const MAX_DEPTH: usize = 8;

fn fill(fields: &[Field], obj: &mut Map<String, Value>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for f in fields {
        match f.kind {
            Kind::Variants { tag, .. } if f.key.is_empty() => {
                if !obj.contains_key(tag)
                    && let Some(d) = parse_default(f)
                {
                    obj.insert(tag.into(), d);
                }
                if let Some(v) = obj.get(tag).and_then(Value::as_str).and_then(|t| f.kind.variant(t)) {
                    fill(v.fields, obj, depth + 1);
                }
            }
            Kind::Group(_) | Kind::Variants { .. } => {
                if !obj.contains_key(f.key) {
                    match parse_default(f) {
                        Some(d) => {
                            obj.insert(f.key.into(), d);
                        }
                        None if f.required => {
                            obj.insert(f.key.into(), Value::Object(Map::new()));
                        }
                        None => continue,
                    }
                }
                if let Some(Value::Object(child)) = obj.get_mut(f.key) {
                    fill_nested(f, child, depth + 1);
                }
            }
            _ => {
                if !obj.contains_key(f.key)
                    && let Some(d) = parse_default(f)
                {
                    obj.insert(f.key.into(), d);
                }
            }
        }
    }
}

/// Fill a group's or a (keyed) variant field's own object.
fn fill_nested(f: &Field, child: &mut Map<String, Value>, depth: usize) {
    match f.kind {
        Kind::Group(fields) => fill(fields, child, depth),
        Kind::Variants { tag, variants } => {
            if !child.contains_key(tag)
                && let Some(first) = variants.first()
            {
                child.insert(tag.into(), json!(first.tag));
            }
            if let Some(v) = child.get(tag).and_then(Value::as_str).and_then(|t| f.kind.variant(t)) {
                fill(v.fields, child, depth);
            }
        }
        _ => {}
    }
}

fn parse_default(f: &Field) -> Option<Value> {
    f.default.and_then(|d| serde_json::from_str(d).ok())
}

/// The object for variant `tag` of variants field `f`, from `old`: the tag, the values of `old`
/// the new variant (or `common`, the fields beside the variants) also has, and the new variant's
/// defaults.
pub fn switch_variant(f: &Field, common: &[Field], old: &Map<String, Value>, tag: &str) -> Map<String, Value> {
    let Kind::Variants { tag: tag_key, .. } = f.kind else { return old.clone() };
    let Some(v) = f.kind.variant(tag) else { return old.clone() };
    let keep = |k: &str| v.fields.iter().chain(common).any(|g| g.key == k && g.key != tag_key);
    let mut out: Map<String, Value> = old.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    out.insert(tag_key.into(), json!(tag));
    fill(v.fields, &mut out, 0);
    out
}

/// A problem with a field's value, for the form to show beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The JSON path of the field (`elastic/young`).
    pub path: String,
    pub message: String,
}

/// What is missing or out of range in `params`: required fields left out, numbers outside
/// their range, values of the wrong kind. An empty list does not promise the command accepts
/// them (it checks more); a problem means it would not.
pub fn problems(fields: &[Field], params: &Value) -> Vec<Problem> {
    match params.as_object() {
        Some(obj) => object_problems(fields, obj),
        None => vec![Problem { path: String::new(), message: "the parameters must be a JSON object".into() }],
    }
}

/// [`problems`] of an object's fields.
pub fn object_problems(fields: &[Field], obj: &Map<String, Value>) -> Vec<Problem> {
    let mut out = Vec::new();
    check(fields, obj, "", &mut out, 0);
    out
}

/// The path of field `key` inside `prefix` (`elastic/young`).
pub fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else if key.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}/{key}")
    }
}

fn check(fields: &[Field], obj: &Map<String, Value>, prefix: &str, out: &mut Vec<Problem>, depth: usize) {
    if depth > MAX_DEPTH || out.len() > MAX_ROWS {
        return;
    }
    for f in fields {
        if let Kind::Variants { tag, .. } = f.kind
            && f.key.is_empty()
        {
            match obj.get(tag).and_then(Value::as_str) {
                Some(t) => match f.kind.variant(t) {
                    Some(v) => check(v.fields, obj, prefix, out, depth + 1),
                    None => out.push(Problem { path: join(prefix, tag), message: format!("`{t}` is not a {}", f.label.to_lowercase()) }),
                },
                None if f.required => out.push(Problem { path: join(prefix, tag), message: "required".into() }),
                None => {}
            }
            continue;
        }
        let path = join(prefix, f.key);
        match obj.get(f.key) {
            None | Some(Value::Null) if f.required => out.push(Problem { path, message: "required".into() }),
            None | Some(Value::Null) => {}
            Some(v) => {
                if let Some(message) = value_problem(f, v, &path, out, depth) {
                    out.push(Problem { path, message });
                }
            }
        }
    }
}

/// What is wrong with `v` as the value of `f`; nested problems go to `out`.
fn value_problem(f: &Field, v: &Value, path: &str, out: &mut Vec<Problem>, depth: usize) -> Option<String> {
    match f.kind {
        Kind::Number { min, max, .. } => match (v.as_f64().filter(|x| x.is_finite()), min, max) {
            (None, ..) => Some("a number".into()),
            (Some(x), Some(lo), _) if x < lo => Some(format!("at least {lo}")),
            (Some(x), _, Some(hi)) if x > hi => Some(format!("at most {hi}")),
            _ => None,
        },
        Kind::Integer { min, max } => match v.as_i64() {
            Some(x) if (min..=max).contains(&x) => None,
            _ => Some(format!("a whole number in {min}..={max}")),
        },
        Kind::Text | Kind::Path { .. } | Kind::Ref(_) => match v.as_str() {
            Some(s) if f.required && s.trim().is_empty() => Some("required".into()),
            Some(_) => None,
            None => Some("text".into()),
        },
        Kind::Bool => (!v.is_boolean()).then(|| "true or false".into()),
        Kind::Choice(options) => match v.as_str() {
            Some(s) if options.iter().any(|(o, _)| *o == s) => None,
            _ => Some(format!("one of {}", options.iter().map(|(o, _)| *o).collect::<Vec<_>>().join(", "))),
        },
        Kind::Vector { labels, nullable, .. } => match v.as_array() {
            Some(a) if a.len() == labels.len() && a.iter().all(|x| x.as_f64().is_some_and(f64::is_finite) || (nullable && x.is_null())) => None,
            _ => Some(format!("{} numbers", labels.len())),
        },
        Kind::RefList(_) => (!v.as_array().is_some_and(|a| a.iter().all(Value::is_string))).then(|| "a list of names".into()),
        Kind::Group(fields) => match v.as_object() {
            Some(o) => {
                check(fields, o, path, out, depth + 1);
                None
            }
            None => Some("an object".into()),
        },
        Kind::Variants { tag, .. } => match v.as_object() {
            Some(o) => match o.get(tag).and_then(Value::as_str).and_then(|t| f.kind.variant(t)) {
                Some(var) => {
                    check(var.fields, o, path, out, depth + 1);
                    None
                }
                None => Some(format!("needs a known `{tag}`")),
            },
            None => Some("an object".into()),
        },
        Kind::Either(options) => {
            let ok = match v {
                Value::String(s) => options.iter().any(|o| o.key == s && matches!(o.kind, Kind::Unit)),
                Value::Object(o) if o.len() == 1 => o.iter().all(|(k, inner)| {
                    options.iter().find(|opt| opt.key == k).is_some_and(|opt| value_problem(opt, inner, path, out, depth + 1).is_none())
                }),
                _ => false,
            };
            (!ok).then(|| format!("one of {}", options.iter().map(|o| o.key).collect::<Vec<_>>().join(", ")))
        }
        Kind::Unit | Kind::Json => None,
        Kind::Rows { fields, max } => match v.as_array() {
            Some(a) if a.len() <= max => {
                for (i, row) in a.iter().enumerate() {
                    match row.as_object() {
                        Some(o) => check(fields, o, &format!("{path}/{i}"), out, depth + 1),
                        None => out.push(Problem { path: format!("{path}/{i}"), message: "an object".into() }),
                    }
                }
                None
            }
            _ => Some(format!("a list of at most {max} rows")),
        },
    }
}

// ---- JSON Schema ----

/// The fields as a JSON Schema (draft 2020-12) object schema, units and hints in descriptions.
pub fn json_schema(fields: &[Field]) -> Value {
    object_schema(fields, 0)
}

fn object_schema(fields: &[Field], depth: usize) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut one_of = None;
    for f in fields {
        if let Kind::Variants { tag, variants } = f.kind
            && f.key.is_empty()
        {
            one_of = Some(variant_schemas(tag, variants, depth));
            if f.required {
                required.push(json!(tag));
            }
            continue;
        }
        properties.insert(f.key.into(), field_schema(f, depth));
        if f.required {
            required.push(json!(f.key));
        }
    }
    let mut s = json!({"type": "object", "properties": properties});
    if !required.is_empty() {
        s["required"] = Value::Array(required);
    }
    if let Some(o) = one_of {
        s["oneOf"] = o;
    }
    s
}

fn variant_schemas(tag: &str, variants: &[Variant], depth: usize) -> Value {
    Value::Array(
        variants
            .iter()
            .map(|v| {
                let mut s = if depth > MAX_DEPTH { json!({"type": "object"}) } else { object_schema(v.fields, depth + 1) };
                s["properties"][tag] = json!({"const": v.tag, "title": v.label});
                let mut req = s.get("required").and_then(Value::as_array).cloned().unwrap_or_default();
                req.push(json!(tag));
                s["required"] = Value::Array(req);
                s
            })
            .collect(),
    )
}

fn field_schema(f: &Field, depth: usize) -> Value {
    let mut s = match f.kind {
        Kind::Number { dim, min, max } => {
            let mut s = json!({"type": "number"});
            if let Some(lo) = min {
                s["minimum"] = json!(lo);
            }
            if let Some(hi) = max {
                s["maximum"] = json!(hi);
            }
            if dim != Dim::None {
                s["x-dimension"] = json!(format!("{dim:?}").to_lowercase());
            }
            s
        }
        Kind::Integer { min, max } => json!({"type": "integer", "minimum": min, "maximum": max}),
        Kind::Text | Kind::Path { .. } => json!({"type": "string"}),
        Kind::Bool => json!({"type": "boolean"}),
        Kind::Choice(options) => json!({"enum": options.iter().map(|(o, _)| *o).collect::<Vec<_>>()}),
        Kind::Vector { labels, nullable, .. } => {
            let item = if nullable { json!({"type": ["number", "null"]}) } else { json!({"type": "number"}) };
            json!({"type": "array", "items": item, "minItems": labels.len(), "maxItems": labels.len()})
        }
        Kind::Ref(c) => json!({"type": "string", "x-references": c.label()}),
        Kind::RefList(c) => json!({"type": "array", "items": {"type": "string", "x-references": c.label()}}),
        Kind::Group(fields) if depth <= MAX_DEPTH => object_schema(fields, depth + 1),
        Kind::Variants { tag, variants } if depth <= MAX_DEPTH => json!({"oneOf": variant_schemas(tag, variants, depth + 1)}),
        Kind::Either(options) if depth <= MAX_DEPTH => json!({"oneOf": options.iter().map(|o| match o.kind {
            Kind::Unit => json!({"const": o.key}),
            _ => json!({"type": "object", "properties": {o.key: field_schema(o, depth + 1)}, "required": [o.key], "additionalProperties": false}),
        }).collect::<Vec<_>>()}),
        Kind::Rows { fields, max } if depth <= MAX_DEPTH => json!({"type": "array", "items": object_schema(fields, depth + 1), "maxItems": max}),
        Kind::Unit => json!({"const": f.key}),
        _ => json!({}),
    };
    s["title"] = json!(f.label);
    if !f.hint.is_empty() {
        s["description"] = json!(f.hint);
    }
    if let Some(d) = parse_default(f) {
        s["default"] = d;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_follow_the_system() {
        assert_eq!(Dim::Stress.unit(UnitSystem::MmTonneS).as_deref(), Some("MPa"));
        assert_eq!(Dim::Density.unit(UnitSystem::MmTonneS).as_deref(), Some("t/mm³"));
        assert_eq!(Dim::Moment.unit(UnitSystem::MKgS).as_deref(), Some("N·m"));
        assert_eq!(Dim::Density.unit(UnitSystem::InLbfS).as_deref(), Some("lbf·s²/in⁴"));
        assert_eq!(Dim::Stress.unit(UnitSystem::Unspecified), None);
        assert_eq!(Dim::Millimetre.unit(UnitSystem::Unspecified).as_deref(), Some("mm"));
        assert_eq!(Dim::None.unit(UnitSystem::MKgS), None);
    }

    const LOADISH: &[Field] = &[
        NAME,
        variants(
            "",
            "Type",
            &[
                variant(
                    "force",
                    "Force",
                    &[xyz("vector", "Vector", Dim::Force).req(), choice("distribution", "Spread", &[("total", "Total")]).def("\"total\"")],
                ),
                variant("pressure", "Pressure", &[num("magnitude", "Magnitude", Dim::Stress).req(), reference("set", "Set", Collection::Set).req()]),
            ],
        )
        .req()
        .def("\"force\""),
    ];

    #[test]
    fn prefill_switch_and_check() {
        let p = prefill(LOADISH, &json!({}));
        assert_eq!(p, json!({"type": "force", "distribution": "total"}));
        let problems = problems(LOADISH, &p);
        let paths: Vec<&str> = problems.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["name", "vector"]);
        let root = &LOADISH[1];
        let switched = switch_variant(
            root,
            LOADISH,
            &json!({"name": "a", "type": "force", "vector": [1, 2, 3]}).as_object().cloned().unwrap_or_default(),
            "pressure",
        );
        assert_eq!(Value::Object(switched), json!({"name": "a", "type": "pressure"}));
        assert!(super::problems(LOADISH, &json!({"name": "a", "type": "force", "vector": [1, "x", 3]})).iter().any(|p| p.path == "vector"));
        assert!(super::problems(LOADISH, &json!([1])).len() == 1);
    }

    #[test]
    fn json_schema_has_variants_and_requirements() {
        let s = json_schema(LOADISH);
        assert_eq!(s["required"], json!(["name", "type"]));
        assert_eq!(s["oneOf"][1]["properties"]["type"]["const"], "pressure");
        assert_eq!(s["oneOf"][1]["required"], json!(["magnitude", "set", "type"]));
    }
}
