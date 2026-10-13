//! What the viewport shows over a results plot: the legend (field, bands, range, deformation
//! scale, a stale-result warning), the probed node, and the results panel whose controls run
//! the `results.*` commands.

use egui::{Align2, Color32, FontId, Rect, RichText, Stroke, pos2, vec2};
use modelcraft_engine::results::Field;
use serde_json::{Value, json};

use crate::ModelApp;
use crate::results::{Plot, number};
use crate::theme::Tokens;

/// Width of the results panel and of the legend's colour swatches (points).
const PANEL_WIDTH: f32 = 260.0;
/// Width of the panel's drop-down lists.
const COMBO_WIDTH: f32 = 160.0;
const SWATCH: egui::Vec2 = vec2(18.0, 15.0);

/// Draw the legend, the probe and the panel when a results plot is shown.
pub fn show(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let Some(plot) = app.plot() else { return };
    let painter = ui.painter_at(rect);
    legend(&plot, &painter, rect, t);
    probe(app, &plot, &painter, t);
    panel(app, &plot, ui, rect, t);
}

/// The legend, top left under the view buttons.
fn legend(plot: &Plot, painter: &egui::Painter, rect: Rect, t: &Tokens) {
    let mut lines: Vec<(String, Color32)> = Vec::new();
    let title = match (&plot.contour, plot.mode()) {
        (Some(l), _) => l.field.label().to_string(),
        (None, Some(_)) => "Mode shape".into(),
        (None, None) => "Deformed shape".into(),
    };
    let mut step = format!("Step {}", plot.view.step);
    if let Some((mode, hz)) = plot.mode() {
        step.push_str(&format!(", mode {mode}: {} Hz", number(hz)));
    }
    lines.push((step, t.overlay_text));
    if plot.scale > 0.0 {
        lines.push((format!("Deformation × {}", number(plot.scale)), t.text_dim));
    } else {
        lines.push(("Undeformed".into(), t.text_dim));
    }
    if plot.stale {
        lines.push(("Stale: the model changed since this step was solved".into(), t.warm));
    }
    let origin = rect.left_top() + vec2(12.0, 44.0);
    let font = FontId::proportional(12.0);
    let line_h = 16.0;
    let bands = plot.contour.as_ref().map_or(0, |l| l.bands.count) as f32;
    let width = 250.0;
    // Padding and title, the lines, then the swatches with the range lines under them.
    let height = 36.0 + line_h * lines.len() as f32 + if bands > 0.0 { 16.0 + bands * SWATCH.y + 2.0 * line_h } else { 0.0 };
    let frame = Rect::from_min_size(origin, vec2(width, height));
    painter.rect_filled(frame, 4.0, t.overlay);
    let mut y = origin.y + 8.0;
    painter.text(pos2(origin.x + 10.0, y), Align2::LEFT_TOP, title, FontId::proportional(14.0), t.overlay_text);
    y += 20.0;
    for (text, color) in &lines {
        painter.text(pos2(origin.x + 10.0, y), Align2::LEFT_TOP, text, font.clone(), *color);
        y += line_h;
    }
    let Some(l) = &plot.contour else { return };
    y += 6.0;
    let stops = t.contour_stops();
    // High values on top, each band's lower boundary written beside its swatch's bottom edge.
    for k in (0..l.bands.count).rev() {
        let row = (l.bands.count - 1 - k) as f32;
        let r = Rect::from_min_size(pos2(origin.x + 10.0, y + row * SWATCH.y), SWATCH);
        let c = modelcraft_render::contour::band_color(&stops, k, l.bands.count);
        painter.rect_filled(r, 0.0, Color32::from_rgb(c.0, c.1, c.2));
        if k + 1 == l.bands.count {
            painter.text(pos2(r.right() + 6.0, r.top()), Align2::LEFT_CENTER, number(l.bands.level(k + 1)), font.clone(), t.overlay_text);
        }
        painter.text(pos2(r.right() + 6.0, r.bottom()), Align2::LEFT_CENTER, number(l.bands.level(k)), font.clone(), t.overlay_text);
    }
    painter.rect_stroke(
        Rect::from_min_size(pos2(origin.x + 10.0, y), vec2(SWATCH.x, SWATCH.y * l.bands.count as f32)),
        0.0,
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Outside,
    );
    y += SWATCH.y * l.bands.count as f32 + 10.0;
    painter.text(pos2(origin.x + 10.0, y), Align2::LEFT_TOP, format!("Max {} at node {}", number(l.max.1), l.max.0), font.clone(), t.text_dim);
    painter.text(pos2(origin.x + 10.0, y + line_h), Align2::LEFT_TOP, format!("Min {} at node {}", number(l.min.1), l.min.0), font, t.text_dim);
}

/// The probed node: a marker on it and its values beside it.
fn probe(app: &ModelApp, plot: &Plot, painter: &egui::Painter, t: &Tokens) {
    let Some(p) = app.probe.as_ref().filter(|p| p.step == plot.view.step && p.mode == plot.view.mode) else { return };
    let Some(at) = plot.node_position(p.node).and_then(|w| crate::viewport::to_screen(app, w)) else { return };
    painter.circle(at, 5.0, t.accent, Stroke::new(2.0, t.overlay_text));
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(p.lines.join("\n"), font, t.overlay_text);
    let box_rect = Rect::from_min_size(at + vec2(12.0, -10.0), galley.size() + vec2(16.0, 8.0));
    painter.rect_filled(box_rect, 4.0, t.overlay);
    painter.galley(box_rect.left_top() + vec2(8.0, 4.0), galley, t.overlay_text);
}

/// The results panel, top right: step, contour field, deformation, modes, animation, reaction
/// sums. Every control runs a command.
fn panel(app: &mut ModelApp, plot: &Plot, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let area = Rect::from_min_size(pos2(rect.right() - PANEL_WIDTH - 12.0, rect.top() + 44.0), vec2(PANEL_WIDTH, rect.height() - 56.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
    let mut run: Vec<(&'static str, Value)> = Vec::new();
    let steps: Vec<(String, bool)> = app.session.result_steps().map(|(n, c)| (n.to_string(), c)).collect();
    egui::Frame::NONE.fill(t.overlay).corner_radius(4.0).inner_margin(egui::Margin::same(8)).show(&mut child, |ui| {
        ui.set_width(PANEL_WIDTH - 16.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Results").strong().color(t.overlay_text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Close").on_hover_text("Back to the model (results.clear)").clicked() {
                    run.push(("results.clear", json!({})));
                }
            });
        });
        egui::Grid::new("mc_results_grid").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
            ui.label("Step");
            egui::ComboBox::from_id_salt("mc_results_step").selected_text(&plot.view.step).width(COMBO_WIDTH).truncate().show_ui(ui, |ui| {
                for (name, current) in &steps {
                    let label = if *current { name.clone() } else { format!("{name} (stale)") };
                    if ui.selectable_label(*name == plot.view.step, label).clicked() && *name != plot.view.step {
                        run.push(("results.deformed", json!({"step": name})));
                    }
                }
            });
            ui.end_row();
            ui.label("Contours");
            let shown = plot.view.contour.map_or("None", Field::label);
            egui::ComboBox::from_id_salt("mc_results_field").selected_text(shown).width(COMBO_WIDTH).truncate().show_ui(ui, |ui| {
                if ui.selectable_label(plot.view.contour.is_none(), "None").clicked() {
                    run.push(("results.contour", json!({"field": "none"})));
                }
                for f in Field::available(&plot.result) {
                    if ui.selectable_label(plot.view.contour == Some(f), f.label()).clicked() {
                        run.push(("results.contour", json!({"field": f.id()})));
                    }
                }
            });
            ui.end_row();
            ui.label("Deformed");
            ui.horizontal(|ui| {
                let mut deformed = plot.view.deformed;
                if ui.checkbox(&mut deformed, "").changed() {
                    run.push(("results.deformed", json!({"show": deformed})));
                }
                let mut auto = plot.view.scale.is_none();
                if ui.checkbox(&mut auto, "Auto").on_hover_text("Scale the largest displacement to a tenth of the model").changed() {
                    run.push(("results.deformed", json!({"scale": if auto { json!("auto") } else { json!(plot.scale.max(f64::MIN_POSITIVE)) }})));
                }
                let mut scale = plot.scale;
                let drag = egui::DragValue::new(&mut scale).speed(plot.scale.abs().max(1e-6) * 0.01).range(1e-9..=1e12).prefix("× ");
                if ui.add_enabled(plot.view.deformed, drag).changed() {
                    run.push(("results.deformed", json!({"scale": scale})));
                }
            });
            ui.end_row();
            ui.label("Animate");
            let mut animate = plot.view.animate;
            if ui.checkbox(&mut animate, "").changed() {
                run.push(("results.animate", json!({"on": animate})));
            }
            ui.end_row();
        });
        if let modelcraft_solver::StepResult::Frequency(f) = plot.result.as_ref() {
            ui.separator();
            ui.label(RichText::new("Modes").color(t.text_dim));
            egui::ScrollArea::vertical().id_salt("mc_results_modes").max_height(150.0).show(ui, |ui| {
                for (i, m) in f.modes.iter().enumerate() {
                    let mode = i + 1;
                    if ui.selectable_label(plot.view.mode == mode, format!("{mode}   {} Hz", number(m.frequency))).clicked() {
                        run.push(("results.modes", json!({"mode": mode})));
                    }
                }
            });
        } else {
            ui.separator();
            if ui.button("Reaction sums").on_hover_text("Reaction force and moment per boundary-condition set (results.reactions)").clicked() {
                run.push(("results.reactions", json!({"step": plot.view.step})));
            }
            if let Some(r) = app.reactions.as_ref().filter(|r| r["step"] == plot.view.step.as_str()) {
                reactions_table(ui, r, t);
            }
        }
        ui.label(RichText::new("Click the model to probe a node").small().color(t.text_dim));
    });
    for (id, params) in run {
        let reply = app.run(id, params);
        if id == "results.reactions" {
            app.reactions = reply.ok();
        }
    }
}

fn reactions_table(ui: &mut egui::Ui, r: &Value, t: &Tokens) {
    let row = |ui: &mut egui::Ui, name: &str, v: &Value| {
        let f = |k: usize| v.pointer(&format!("/force/{k}")).and_then(Value::as_f64).map_or("—".into(), number);
        ui.label(name);
        ui.label(f(0));
        ui.label(f(1));
        ui.label(f(2));
        ui.end_row();
    };
    egui::Grid::new("mc_results_reactions").num_columns(4).spacing(vec2(6.0, 2.0)).show(ui, |ui| {
        for h in ["Set", "Fx", "Fy", "Fz"] {
            ui.label(RichText::new(h).small().color(t.text_dim));
        }
        ui.end_row();
        for s in r["sets"].as_array().into_iter().flatten() {
            row(ui, s["set"].as_str().unwrap_or("?"), s);
        }
        row(ui, "All", &r["total"]);
    });
}
