//! What the viewport hides: `view.hide`, `view.show`, `view.isolate` and `view.show_all`.
//!
//! The hidden items (bodies, imported surfaces, datums, boundary conditions, loads, reference
//! points, couplings) are view state ([`fieldcraft_io::View`]): saved with the model and marking
//! it unsaved, but not a definition, so they are neither journalled nor undone and they leave
//! results current. A hidden item is not drawn, so it cannot be picked either. An item that goes
//! away (deleted, or a body cut into cells) is forgotten.

use std::collections::BTreeSet;

use fieldcraft_io::{Item, ItemKind, MAX_HIDDEN};
use serde_json::Value;

use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const HIDE: CommandSpec = CommandSpec {
    id: "view.hide",
    label: "Hide",
    params: concat!(
        r#"{"items": [{"kind": "body" | "surface" | "datum" | "boundary_condition" | "load" | "reference_point" | "coupling", "name": "Sym"}]}"#,
        " — hides the items in the viewport (and from picking); view state, saved with the model"
    ),
    form: Some(super::forms::VIEW_ITEMS),
    kind: CommandKind::Session,
    requires: &[],
    run: |s, p| change(s, p, Change::Hide),
};

pub const SHOW: CommandSpec = CommandSpec {
    id: "view.show",
    label: "Show",
    params: concat!(
        r#"{"items": [{"kind": "body" | "surface" | "datum" | "boundary_condition" | "load" | "reference_point" | "coupling", "name": "Sym"}]}"#,
        " — shows hidden items again"
    ),
    form: Some(super::forms::VIEW_ITEMS),
    kind: CommandKind::Session,
    requires: &[],
    run: |s, p| change(s, p, Change::Show),
};

pub const ISOLATE: CommandSpec = CommandSpec {
    id: "view.isolate",
    label: "Isolate",
    params: concat!(
        r#"{"items": [{"kind": "body" | "surface" | "datum" | "boundary_condition" | "load" | "reference_point" | "coupling", "name": "Sym"}]}"#,
        " — shows only these items: every other one is hidden (view.show_all brings them back)"
    ),
    form: Some(super::forms::VIEW_ITEMS),
    kind: CommandKind::Session,
    requires: &[],
    run: |s, p| change(s, p, Change::Isolate),
};

pub const SHOW_ALL: CommandSpec = CommandSpec {
    id: "view.show_all",
    label: "Show All",
    params: "{}",
    form: Some(&[]),
    kind: CommandKind::Session,
    requires: &[],
    run: |s, _| {
        s.view.hidden.clear();
        Ok(Value::Null)
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Hide,
    Show,
    Isolate,
}

/// Every item of `kind` the session has, by name.
pub(crate) fn names(s: &Session, kind: ItemKind) -> Vec<String> {
    let m = &s.model;
    match kind {
        ItemKind::Body => s.design.iter().flat_map(|d| d.bodies.iter().map(|b| b.name.clone())).collect(),
        ItemKind::Surface => s.surfaces.iter().map(|x| x.name.clone()).collect(),
        ItemKind::Datum => m.datums().keys().cloned().collect(),
        ItemKind::BoundaryCondition => m.boundary_conditions().keys().cloned().collect(),
        ItemKind::Load => m.loads().keys().cloned().collect(),
        ItemKind::ReferencePoint => m.reference_points().keys().cloned().collect(),
        ItemKind::Coupling => m.couplings().keys().cloned().collect(),
    }
}

fn exists(s: &Session, item: &Item) -> bool {
    let m = &s.model;
    match item.kind {
        ItemKind::Body => s.design.as_ref().is_some_and(|d| d.body(&item.name).is_some()),
        ItemKind::Surface => s.surfaces.iter().any(|x| x.name == item.name),
        ItemKind::Datum => m.datum(&item.name).is_some(),
        ItemKind::BoundaryCondition => m.boundary_conditions().contains_key(&item.name),
        ItemKind::Load => m.loads().contains_key(&item.name),
        ItemKind::ReferencePoint => m.reference_points().contains_key(&item.name),
        ItemKind::Coupling => m.couplings().contains_key(&item.name),
    }
}

/// Forget hidden items the session no longer has.
pub(crate) fn forget_missing(s: &mut Session) {
    if s.view.hidden.is_empty() {
        return;
    }
    let gone: Vec<Item> = s.view.hidden.iter().filter(|i| !exists(s, i)).cloned().collect();
    for i in gone {
        s.view.hidden.remove(&i);
    }
}

impl Session {
    /// The analysis setup's symbols, without the definitions the view hides.
    pub fn glyphs(&self) -> fieldcraft_render::glyphs::Glyphs {
        self.glyphs_of(None)
    }

    /// The symbols of what solved step `step` used, over its results: its own boundary
    /// conditions and loads (none for a frequency step, which has no loads) and its own initial
    /// velocities and rigid walls; the interactions as always. Without the view's hidden items.
    pub fn step_glyphs(&self, step: &str) -> fieldcraft_render::glyphs::Glyphs {
        self.glyphs_of(Some(step))
    }

    fn glyphs_of(&self, step: Option<&str>) -> fieldcraft_render::glyphs::Glyphs {
        use fieldcraft_model::Procedure;
        use fieldcraft_render::glyphs::Definition;
        let step = step.map(|name| self.model.step(name));
        let frequency = matches!(step, Some(Some(s)) if matches!(s.procedure, Procedure::Frequency { .. }));
        fieldcraft_render::glyphs::Glyphs::from_model_hiding(&self.model, &|definition, name| {
            let (kind, used) = match definition {
                Definition::BoundaryCondition => (ItemKind::BoundaryCondition, step.map(|s| s.is_some_and(|s| s.boundary_conditions.contains(name)))),
                Definition::Load => (ItemKind::Load, step.map(|s| !frequency && s.is_some_and(|s| s.loads.contains(name)))),
                Definition::ReferencePoint => (ItemKind::ReferencePoint, None),
                Definition::Coupling => (ItemKind::Coupling, None),
                Definition::Step => return step.is_some_and(|s| s.is_none_or(|s| s.name != name)),
                Definition::Contact | Definition::Tie => return false,
            };
            used == Some(false) || self.view.is_hidden(kind, name)
        })
    }

    /// The elements of the bodies the view hides: hiding a body hides its mesh and its results.
    pub fn hidden_elements(&self) -> std::collections::BTreeSet<u32> {
        self.view.hidden.iter().filter(|i| i.kind == ItemKind::Body).flat_map(|i| self.model.body_elements(&i.name)).collect()
    }

    /// Whether the view shows item `name` of `kind`.
    pub fn shows(&self, kind: ItemKind, name: &str) -> bool {
        !self.view.is_hidden(kind, name)
    }
}

fn change(s: &mut Session, p: &Value, how: Change) -> Result<Value> {
    let items: Vec<Item> = super::req(p, "items")?;
    if items.len() > MAX_HIDDEN {
        return Err(EngineError::Params(format!("at most {MAX_HIDDEN} items")));
    }
    if let Some(i) = items.iter().find(|i| !exists(s, i)) {
        let kind = serde_json::to_value(i.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        return Err(EngineError::Params(format!("no {} `{}`", kind.replace('_', " "), i.name.chars().take(100).collect::<String>())));
    }
    match how {
        Change::Hide => s.view.hidden.extend(items),
        Change::Show => {
            for i in &items {
                s.view.hidden.remove(i);
            }
        }
        Change::Isolate => {
            let keep: BTreeSet<Item> = items.into_iter().collect();
            let all = ItemKind::ALL.into_iter().flat_map(|kind| names(s, kind).into_iter().map(move |name| Item { kind, name }));
            let rest: BTreeSet<Item> = all.filter(|i| !keep.contains(i)).take(MAX_HIDDEN).collect();
            s.view.hidden = rest;
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cmd::datum::tests::plate;

    fn item(kind: ItemKind, name: &str) -> Value {
        json!({"kind": kind, "name": name})
    }

    #[test]
    fn hiding_is_saved_view_state_not_an_edit() {
        let (mut s, dir) = plate();
        s.execute("geometry.datum", &json!({"name": "Sym", "by": {"type": "offset", "plane": "zx"}})).unwrap();
        s.execute("geometry.datum.point", &json!({"name": "P", "by": {"type": "coordinates", "point": [1, 2, 3]}})).unwrap();
        let file = dir.join("hide.fieldcraft");
        s.execute("file.save_as", &json!({"path": file.to_str().unwrap()})).unwrap();
        let (journal, revision) = (s.journal.len(), s.revision());
        assert!(!s.is_dirty());

        s.execute("view.hide", &json!({"items": [item(ItemKind::Datum, "Sym")]})).unwrap();
        assert_eq!(s.view.hidden.iter().collect::<Vec<_>>(), [&Item { kind: ItemKind::Datum, name: "Sym".into() }]);
        // Unsaved, but no edit: nothing journalled, the revision (and so any result) unchanged.
        assert!(s.is_dirty());
        assert_eq!((s.journal.len(), s.revision()), (journal, revision));
        // Showing it again is back to the saved state.
        s.execute("view.show", &json!({"items": [item(ItemKind::Datum, "Sym")]})).unwrap();
        assert!(!s.is_dirty());

        // Hidden state survives save and open.
        s.execute("view.hide", &json!({"items": [item(ItemKind::Datum, "Sym")]})).unwrap();
        s.execute("file.save", &json!({})).unwrap();
        let mut t = Session::default();
        t.execute("file.open", &json!({"path": file.to_str().unwrap()})).unwrap();
        assert!(t.view.is_hidden(ItemKind::Datum, "Sym") && !t.view.is_hidden(ItemKind::Datum, "P"));
        assert!(!t.is_dirty());

        // Isolate hides every other item; show_all brings them back.
        let body = names(&t, ItemKind::Body).remove(0);
        t.execute("view.isolate", &json!({"items": [item(ItemKind::Datum, "P")]})).unwrap();
        assert!(t.view.is_hidden(ItemKind::Body, &body) && t.view.is_hidden(ItemKind::Datum, "Sym") && !t.view.is_hidden(ItemKind::Datum, "P"));
        t.execute("view.show_all", &json!({})).unwrap();
        assert!(t.view.is_empty());

        // An item deleted is forgotten.
        t.execute("view.hide", &json!({"items": [item(ItemKind::Datum, "P")]})).unwrap();
        t.execute("geometry.datum.delete", &json!({"name": "P"})).unwrap();
        assert!(t.view.is_empty());

        // Unknown items and hostile parameters change nothing.
        for p in [json!({}), json!({"items": "x"}), json!({"items": [item(ItemKind::Load, "Nope")]}), json!({"items": [{"kind": "x", "name": "A"}]})]
        {
            assert!(t.execute("view.hide", &p).is_err(), "{p}");
        }
        assert!(t.view.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
