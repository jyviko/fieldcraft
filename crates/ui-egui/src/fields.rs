//! Form field widgets: a labelled row, a number field that keeps what is being typed until it
//! is a number, a drop-down, a text field. They know nothing of FieldCraft (colours and widths
//! come in a [`Style`]), so they can move to the shared Craft UI crate.
//!
//! The row layout (a label column of fixed width, clamped for long labels, then the field) is
//! adapted from SolidCraft's dialog rows (`crates/ui-egui/src/dialogs.rs`, MIT OR Apache-2.0).

use egui::{Color32, Id, RichText, Ui, vec2};

/// Widths and colours of a form.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    /// The label column.
    pub label_width: f32,
    /// A field (number, text, drop-down) without its unit.
    pub field_width: f32,
    pub dim: Color32,
    pub error: Color32,
    /// The mark of a required field.
    pub required: Color32,
}

/// Row height.
pub const ROW_H: f32 = 22.0;

/// A row: `label` (with a mark when `required`) in the label column, then `add`.
pub fn row<R>(ui: &mut Ui, s: &Style, label: &str, required: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(s.label_width, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(s.label_width);
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add(egui::Label::new(RichText::new(label).color(s.dim)).truncate()).on_hover_text(label);
            if required {
                ui.label(RichText::new("*").color(s.required)).on_hover_text("Required");
            }
        });
        add(ui)
    })
    .inner
}

/// A message under a field, aligned with the fields.
pub fn note(ui: &mut Ui, s: &Style, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.add_space(s.label_width + ui.spacing().item_spacing.x);
        ui.add(egui::Label::new(RichText::new(text).small().color(color)).wrap());
    });
}

/// What a number field did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumberEdit {
    Unchanged,
    /// A new value; `None` when the field was emptied.
    Set(Option<f64>),
    /// What is typed is not a number (yet); the value is unchanged.
    Invalid,
}

/// The typed text of a field and the value it was typed over: kept while the value stays the
/// same, so a half-typed `1e-` survives frames and an outside change (undo, the JSON view)
/// replaces it.
#[derive(Clone, Debug, Default)]
struct Draft {
    text: String,
    over: Option<f64>,
}

/// A number field: shows `value` (or the typed text), greys `hint` when empty, and reports a new
/// value only when the text is a finite number (or empty). `integer` refuses fractions. `id` is the
/// field's own (unique in the form, and differing between forms): it keys both the typed text and
/// the widget, so focus and typing never move to another form's field at the same place.
pub fn number(ui: &mut Ui, s: &Style, id: Id, value: Option<f64>, hint: &str, integer: bool, width: f32) -> NumberEdit {
    let shown = |v: Option<f64>| v.map(format_number).unwrap_or_default();
    let mut draft = ui.data(|d| d.get_temp::<Draft>(id)).filter(|d| d.over == value).unwrap_or_else(|| Draft { text: shown(value), over: value });
    let parse = |t: &str| -> Option<Option<f64>> {
        let t = t.trim();
        if t.is_empty() {
            return Some(None);
        }
        let x = t.parse::<f64>().ok().filter(|x| x.is_finite())?;
        (!integer || x.fract() == 0.0).then_some(Some(x))
    };
    let bad = parse(&draft.text).is_none();
    let mut edit = egui::TextEdit::singleline(&mut draft.text).id(id).desired_width(width).hint_text(RichText::new(hint).italics().color(s.dim));
    if bad {
        edit = edit.text_color(s.error);
    }
    let r = ui.add(edit);
    let out = if r.changed() {
        match parse(&draft.text) {
            Some(v) => {
                draft.over = v;
                NumberEdit::Set(v)
            }
            None => NumberEdit::Invalid,
        }
    } else if bad {
        NumberEdit::Invalid
    } else {
        NumberEdit::Unchanged
    };
    ui.data_mut(|d| d.insert_temp(id, draft));
    out
}

/// A number as typed: integers without a fraction, others in their shortest round-trip form.
pub fn format_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        serde_json::Number::from_f64(x).map_or_else(|| x.to_string(), |n| n.to_string())
    }
}

/// A drop-down of `options` (labels); returns the index picked this frame.
pub fn combo(ui: &mut Ui, id: Id, selected: &str, options: &[&str], width: f32) -> Option<usize> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(id).selected_text(selected).width(width).truncate().show_ui(ui, |ui| {
        for (i, o) in options.iter().enumerate() {
            if ui.selectable_label(*o == selected, *o).clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// A one-line text field with a greyed hint, keyed by `id` (see [`number`]); true when it changed.
pub fn text(ui: &mut Ui, s: &Style, id: Id, value: &mut String, hint: &str, width: f32) -> bool {
    ui.add(egui::TextEdit::singleline(value).id(id).desired_width(width).hint_text(RichText::new(hint).italics().color(s.dim))).changed()
}

/// A unit after a field.
pub fn unit(ui: &mut Ui, s: &Style, unit: Option<&str>) {
    if let Some(u) = unit {
        ui.label(RichText::new(u).small().color(s.dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_typed() {
        assert_eq!(format_number(210000.0), "210000");
        assert_eq!(format_number(0.3), "0.3");
        assert_eq!(format_number(7.85e-9), "7.85e-9");
        assert_eq!(format_number(-2.0), "-2");
    }
}
