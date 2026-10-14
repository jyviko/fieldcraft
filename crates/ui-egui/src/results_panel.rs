//! What the viewport shows over a results plot: the legend (field, bands, range, deformation
//! scale, a stale-result warning) and the probed node; and the results controls of the context
//! panel (right), which run the `results.*` commands.

use egui::{Align2, Color32, FontId, Rect, RichText, Stroke, pos2, vec2};
use fieldcraft_engine::results::{Field, Kind};
use fieldcraft_explicit::{Energies, ExplicitResult};
use serde_json::{Value, json};

use crate::ModelApp;
use crate::results::{Plot, number};
use crate::theme::Tokens;

/// Width of the panel's drop-down lists.
const COMBO_WIDTH: f32 = 160.0;
const SWATCH: egui::Vec2 = vec2(18.0, 15.0);
/// Height of the energy plot (points), and the most points one of its lines draws.
const PLOT_HEIGHT: f32 = 90.0;
const MAX_PLOT_POINTS: usize = 400;

/// Draw the legend and the probe when a results plot is shown.
pub fn show(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let Some(plot) = app.plot() else { return };
    let painter = ui.painter_at(rect);
    legend(&plot, &painter, rect, t);
    probe(app, &plot, &painter, t);
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
    if let Some((k, time, n)) = plot.frame() {
        step.push_str(&format!(", frame {k} of {}: t = {}", n.saturating_sub(1), number(time)));
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
    let origin = rect.left_top() + vec2(12.0, 12.0);
    let font = FontId::proportional(12.0);
    let line_h = 16.0;
    let bands = plot.contour.as_ref().map_or(0, |l| l.bands.count) as f32;
    // As wide as its longest line (and the range labels beside the swatches).
    let widest = lines.iter().map(|(text, _)| painter.layout_no_wrap(text.clone(), font.clone(), t.overlay_text).size().x).fold(0.0_f32, f32::max);
    let title_w = painter.layout_no_wrap(title.clone(), FontId::proportional(14.0), t.overlay_text).size().x;
    let width = (widest.max(title_w) + 20.0).max(170.0);
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
        let c = fieldcraft_render::contour::band_color(&stops, k, l.bands.count);
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
    let Some(p) = app.probe.as_ref().filter(|p| p.step == plot.view.step && p.mode == plot.pick.mode && p.frame == plot.pick.frame) else { return };
    let Some(at) = plot.node_position(p.node).and_then(|w| crate::viewport::to_screen(app, w)) else { return };
    painter.circle(at, 5.0, t.accent, Stroke::new(2.0, t.overlay_text));
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(p.lines.join("\n"), font, t.overlay_text);
    let box_rect = Rect::from_min_size(at + vec2(12.0, -10.0), galley.size() + vec2(16.0, 8.0));
    painter.rect_filled(box_rect, 4.0, t.overlay);
    painter.galley(box_rect.left_top() + vec2(8.0, 4.0), galley, t.overlay_text);
}

/// The results controls (the context panel's Results section): step, contour field,
/// deformation, modes, animation, reaction sums. Every control runs a command.
pub fn controls(app: &mut ModelApp, plot: &Plot, ui: &mut egui::Ui, t: &Tokens) {
    let mut run: Vec<(&'static str, Value)> = Vec::new();
    let steps: Vec<(String, bool)> = app.session.result_steps().map(|(n, c)| (n.to_string(), c)).collect();
    egui::Grid::new("fc_results_grid").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
        ui.label("Step");
        egui::ComboBox::from_id_salt("fc_results_step").selected_text(&plot.view.step).width(COMBO_WIDTH).truncate().show_ui(ui, |ui| {
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
        egui::ComboBox::from_id_salt("fc_results_field").selected_text(shown).width(COMBO_WIDTH).truncate().show_ui(ui, |ui| {
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
    if let Kind::Explicit(e) = plot.result.kind() {
        ui.separator();
        explicit(ui, e, plot, t, &mut run);
    } else if let Kind::Frequency(f) = plot.result.kind() {
        ui.separator();
        ui.label(RichText::new("Modes").color(t.text_dim));
        egui::ScrollArea::vertical().id_salt("fc_results_modes").max_height(150.0).show(ui, |ui| {
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
    for (id, params) in run {
        let reply = app.run(id, params);
        if id == "results.reactions" {
            app.reactions = reply.ok();
        }
    }
}

/// An explicit step: a frame slider (the deformed shape and contours follow it) and the energy
/// histories, plotted and read at the frame's time.
fn explicit(ui: &mut egui::Ui, e: &ExplicitResult, plot: &Plot, t: &Tokens, run: &mut Vec<(&'static str, Value)>) {
    if let Some((k, time, n)) = plot.frame() {
        ui.horizontal(|ui| {
            ui.label("Frame");
            let mut frame = k;
            if ui.add(egui::Slider::new(&mut frame, 0..=n.saturating_sub(1))).changed() {
                run.push(("results.frame", json!({"frame": frame})));
            }
        });
        ui.label(RichText::new(format!("t = {}", number(time))).color(t.text_dim));
    }
    let h = &e.history;
    if h.time.is_empty() {
        return;
    }
    let series: [(&str, Color32, fn(&Energies) -> f64); 4] = [
        ("Kinetic", t.accent, |e| e.kinetic),
        ("Internal", t.ok, |e| e.internal),
        ("Hourglass", t.warm, |e| e.hourglass),
        ("External", t.error, |e| e.external),
    ];
    ui.label(RichText::new("Energies").color(t.text_dim));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), PLOT_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, t.field);
    let t_end = h.time.last().copied().unwrap_or(0.0).max(f64::MIN_POSITIVE);
    let top = h.energies.iter().flat_map(|e| series.iter().map(move |(_, _, f)| f(e).abs())).filter(|v| v.is_finite()).fold(0.0, f64::max);
    let top = if top > 0.0 { top } else { 1.0 };
    let at =
        |time: f64, v: f64| pos2(rect.left() + (time / t_end) as f32 * rect.width(), rect.bottom() - 2.0 - (v / top) as f32 * (rect.height() - 4.0));
    // At most a few hundred points per line, however long the history.
    let stride = (h.time.len() / MAX_PLOT_POINTS).max(1);
    for (_, color, f) in &series {
        let points: Vec<egui::Pos2> =
            h.time.iter().zip(&h.energies).step_by(stride).filter(|(_, e)| f(e).is_finite()).map(|(time, e)| at(*time, f(e))).collect();
        painter.add(egui::Shape::line(points, Stroke::new(1.5, *color)));
    }
    // The frame's time, and the sample nearest it.
    let now = plot.frame().map_or(t_end, |(_, time, _)| time);
    let x = at(now, 0.0).x;
    painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, t.text_dim));
    let i = h.time.iter().enumerate().min_by(|a, b| (a.1 - now).abs().total_cmp(&(b.1 - now).abs())).map_or(0, |(i, _)| i);
    let Some(at_now) = h.energies.get(i) else { return };
    egui::Grid::new("fc_results_energies").num_columns(2).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
        for (name, color, f) in &series {
            ui.label(RichText::new(*name).color(*color));
            ui.label(number(f(at_now)));
            ui.end_row();
        }
        ui.label(RichText::new("Balance").color(t.text_dim));
        ui.label(number(at_now.balance));
        ui.end_row();
    });
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
    egui::Grid::new("fc_results_reactions").num_columns(4).spacing(vec2(6.0, 2.0)).show(ui, |ui| {
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
