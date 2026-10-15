//! What the viewport shows over a results plot: the legend (field and unit, bands, range,
//! deformation scale, a stale-result warning), markers at the contour's largest and smallest
//! value, and the probed node; and the results controls of the context panel (right), which run
//! the `results.*` commands. Numbers go through one formatter ([`crate::results::number`]) with
//! the units of the model's unit system ([`Quantity`]).

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, pos2, vec2};
use fieldcraft_engine::results::{Field, Kind, Quantity, true_scale, with_unit as titled};
use fieldcraft_explicit::{Energies, ExplicitResult};
use serde_json::{Value, json};

use crate::ModelApp;
use crate::results::{Plot, components, count, number, number_in, with_unit};
use crate::theme::Tokens;

/// Width of the panel's drop-down lists.
const COMBO_WIDTH: f32 = 160.0;
const SWATCH: egui::Vec2 = vec2(18.0, 15.0);
/// Height of the energy plot's drawing area (points), the room its axis labels take, and the
/// most points one of its lines draws.
const PLOT_HEIGHT: f32 = 110.0;
const AXIS_LEFT: f32 = 48.0;
const AXIS_BOTTOM: f32 = 16.0;
const MAX_PLOT_POINTS: usize = 400;
/// Gap between a marker and its label, and the label's padding (points).
const LABEL_GAP: f32 = 12.0;
const LABEL_PAD: egui::Vec2 = vec2(8.0, 4.0);

/// Draw the legend, the extremes' markers and the probe when a results plot is shown. A click
/// on the legend's Max or Min line turns the view to that node and probes it.
pub fn show(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let Some(plot) = app.plot() else { return };
    let painter = ui.painter_at(rect);
    let extremes = legend(&plot, &painter, rect, t);
    if app.ui.result_extremes {
        markers(app, &plot, &painter, rect, t);
    }
    probe(app, &plot, &painter, rect, t);
    let mut focus = None;
    for (i, (line, node)) in extremes.into_iter().enumerate() {
        let r = ui.interact(line, ui.id().with(("fc_legend_extreme", i)), egui::Sense::click()).on_hover_text(format!("Show and probe node {node}"));
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.clicked() {
            focus = Some(node);
        }
    }
    if let Some(node) = focus {
        focus_node(app, &plot, node);
    }
}

/// Turn the view about node `node` of the plot (keeping the zoom) and probe it.
fn focus_node(app: &mut ModelApp, plot: &Plot, node: u32) {
    if let Some(at) = plot.node_position(node).filter(|p| p.is_finite()) {
        app.cam.target = at;
    }
    // A failure (a stale result) shows in the status bar.
    let _ = app.probe_node(node);
}

/// What the legend shows and where: its frame, title and text lines.
struct Legend {
    frame: Rect,
    title: String,
    lines: Vec<(String, Color32)>,
}

const LEGEND_LINE: f32 = 16.0;

/// Where the legend of `plot` goes in the viewport `rect` (a fit leaves it free).
pub fn legend_rect(plot: &Plot, painter: &egui::Painter, rect: Rect, t: &Tokens) -> Rect {
    legend_layout(plot, painter, rect, t).frame
}

/// The legend's title: the field with its unit (a mode shape normalised), or the shape drawn.
fn title(plot: &Plot) -> String {
    match (&plot.contour, plot.mode()) {
        (Some(l), _) => l.field.title(&plot.result, plot.units),
        (None, Some(_)) => "Mode shape (normalised)".into(),
        (None, None) => "Deformed shape".into(),
    }
}

/// The deformation line: its scale, and whether it is automatic or true.
fn deformation(plot: &Plot) -> String {
    if !plot.view.deformed {
        return "Undeformed".into();
    }
    let how = match plot.view.scale {
        _ if plot.scale == 1.0 && true_scale(&plot.result) => " (true scale)",
        None => " (auto)",
        Some(_) => "",
    };
    format!("Deformation × {}{how}", number(plot.scale))
}

/// The contour's Max and Min lines.
fn extreme_lines(plot: &Plot) -> Option<[(String, u32); 2]> {
    let l = plot.contour.as_ref()?;
    let unit = l.field.unit(&plot.result, plot.units);
    let scale = l.max.1.abs().max(l.min.1.abs());
    let line = |what: &str, (node, v): (u32, f64)| (format!("{what} {} at node {node}", with_unit(number_in(v, scale), unit.as_deref())), node);
    Some([line("Max", l.max), line("Min", l.min)])
}

/// The legend's text and frame: top left, as wide as its longest line and as tall as its lines
/// and bands.
fn legend_layout(plot: &Plot, painter: &egui::Painter, rect: Rect, t: &Tokens) -> Legend {
    let mut lines: Vec<(String, Color32)> = Vec::new();
    let title = title(plot);
    let mut step = format!("Step {}", plot.view.step);
    if let Some((mode, value)) = plot.mode() {
        step.push_str(&format!(", mode {mode}: {value}"));
    }
    if let Some((k, at, n)) = plot.frame() {
        let last = n.saturating_sub(1);
        match plot.result.kind() {
            // A nonlinear static step's frames are its increments, at their load factors.
            Kind::Static(_) => step.push_str(&format!(", increment {k} of {last}: load factor {}", number(at))),
            _ => step.push_str(&format!(", frame {k} of {last}: t = {}", with_unit(number(at), Quantity::Time.unit(plot.units).as_deref()))),
        }
    }
    lines.push((step, t.overlay_text));
    lines.push((deformation(plot), t.text_dim));
    if let Some(l) = plot.contour.as_ref().filter(|_| plot.view.range_min.is_some() || plot.view.range_max.is_some()) {
        let scale = l.max.1.abs().max(l.min.1.abs());
        lines.push((format!("Legend clamped to {} … {}", number_in(l.bands.min, scale), number_in(l.bands.max, scale)), t.warm));
    }
    if plot.stale {
        lines.push(("Stale: the model changed since this step was solved".into(), t.warm));
    }
    let origin = rect.left_top() + vec2(12.0, 12.0);
    let font = FontId::proportional(12.0);
    let line_h = LEGEND_LINE;
    let bands = plot.contour.as_ref().map_or(0, |l| swatches(&l.bands)) as f32;
    // As wide as its longest line (and the extremes' lines under the swatches).
    let extremes = extreme_lines(plot).into_iter().flatten().map(|(text, _)| text);
    let widest = lines
        .iter()
        .map(|(text, _)| text.clone())
        .chain(extremes)
        .map(|text| painter.layout_no_wrap(text, font.clone(), t.overlay_text).size().x)
        .fold(0.0_f32, f32::max);
    let title_w = painter.layout_no_wrap(title.clone(), FontId::proportional(14.0), t.overlay_text).size().x;
    let width = (widest.max(title_w) + 20.0).max(170.0);
    // Padding and title, the lines, then the swatches with the range lines under them.
    let height = 36.0 + line_h * lines.len() as f32 + if bands > 0.0 { 16.0 + bands * SWATCH.y + 2.0 * line_h } else { 0.0 };
    Legend { frame: Rect::from_min_size(origin, vec2(width, height)), title, lines }
}

/// The legend's swatches: the bands, and the "over" band above a clamped maximum.
fn swatches(b: &fieldcraft_render::contour::Bands) -> u32 {
    b.count + u32::from(b.over)
}

/// The legend, top left under the view buttons. Returns the Max and Min lines' rects with their
/// nodes.
fn legend(plot: &Plot, painter: &egui::Painter, rect: Rect, t: &Tokens) -> Vec<(Rect, u32)> {
    let Legend { frame, title, lines } = legend_layout(plot, painter, rect, t);
    let origin = frame.min;
    let (font, line_h) = (FontId::proportional(12.0), LEGEND_LINE);
    painter.rect_filled(frame, 4.0, t.overlay);
    let mut y = origin.y + 8.0;
    painter.text(pos2(origin.x + 10.0, y), Align2::LEFT_TOP, title, FontId::proportional(14.0), t.overlay_text);
    y += 20.0;
    for (text, color) in &lines {
        painter.text(pos2(origin.x + 10.0, y), Align2::LEFT_TOP, text, font.clone(), *color);
        y += line_h;
    }
    let (Some(l), Some(extremes)) = (&plot.contour, extreme_lines(plot)) else { return Vec::new() };
    y += 6.0;
    let stops = t.contour_stops();
    // Round-off among the levels (a range from -1e-15 to 3) reads as 0.
    let scale = l.max.1.abs().max(l.min.1.abs());
    // High values on top, each band's lower boundary written beside its swatch's bottom edge;
    // over a clamped maximum, the "over" band runs up to the field's true maximum.
    let rows = swatches(&l.bands);
    let over = crate::theme::rgb(t.contour_over);
    for k in (0..rows).rev() {
        let row = (rows - 1 - k) as f32;
        let r = Rect::from_min_size(pos2(origin.x + 10.0, y + row * SWATCH.y), SWATCH);
        let c = fieldcraft_render::contour::bands_color(&stops, over, &l.bands, k);
        painter.rect_filled(r, 0.0, Color32::from_rgb(c.0, c.1, c.2));
        if k + 1 == rows {
            let top = if l.bands.is_over(k) { l.max.1 } else { l.bands.level(k + 1) };
            painter.text(pos2(r.right() + 6.0, r.top()), Align2::LEFT_CENTER, number_in(top, scale), font.clone(), t.overlay_text);
        }
        painter.text(pos2(r.right() + 6.0, r.bottom()), Align2::LEFT_CENTER, number_in(l.bands.level(k), scale), font.clone(), t.overlay_text);
    }
    painter.rect_stroke(
        Rect::from_min_size(pos2(origin.x + 10.0, y), vec2(SWATCH.x, SWATCH.y * rows as f32)),
        0.0,
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Outside,
    );
    y += SWATCH.y * rows as f32 + 10.0;
    let mut out = Vec::new();
    for (i, (text, node)) in extremes.into_iter().enumerate() {
        let r = painter.text(pos2(origin.x + 10.0, y + line_h * i as f32), Align2::LEFT_TOP, text, font.clone(), t.text_dim);
        out.push((r.expand(2.0), node));
    }
    out
}

/// A box of `text` beside the screen point `at`, kept inside `bounds`: right of the point, or left
/// of it when it would run off the right edge; held inside top and bottom.
fn label_beside(painter: &egui::Painter, at: Pos2, text: String, bounds: Rect, t: &Tokens) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), t.overlay_text);
    let size = galley.size() + 2.0 * LABEL_PAD;
    let right = at + vec2(LABEL_GAP, -size.y / 2.0);
    let x = if right.x + size.x > bounds.right() { at.x - LABEL_GAP - size.x } else { right.x };
    let y = right.y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    let box_rect = Rect::from_min_size(pos2(x.max(bounds.left()), y), size);
    painter.rect_filled(box_rect, 4.0, t.overlay);
    painter.galley(box_rect.left_top() + LABEL_PAD, galley, t.overlay_text);
}

/// Markers with their values at the contour's largest and smallest value.
fn markers(app: &ModelApp, plot: &Plot, painter: &egui::Painter, rect: Rect, t: &Tokens) {
    let Some(l) = &plot.contour else { return };
    let unit = l.field.unit(&plot.result, plot.units);
    let scale = l.max.1.abs().max(l.min.1.abs());
    let (high, low) = (t.contour[4], t.contour[0]);
    for (what, (node, v), color) in [("Max", l.max, high), ("Min", l.min, low)] {
        let Some(at) = plot.node_position(node).and_then(|w| crate::viewport::to_screen(app, w)) else { continue };
        if !rect.contains(at) {
            continue;
        }
        painter.circle(at, 5.0, color, Stroke::new(2.0, t.overlay_text));
        label_beside(painter, at, format!("{what} {}", with_unit(number_in(v, scale), unit.as_deref())), rect, t);
    }
}

/// `results.contour`'s parameters that change one end of the legend's range (`key` `min` or
/// `max`), keeping the field and the other end.
fn legend_params(plot: &Plot, key: &str, value: Value) -> Value {
    let field = plot.view.contour.map_or("none", Field::id);
    let mut range = json!({"min": plot.view.range_min, "max": plot.view.range_max});
    range[key] = value;
    json!({"step": plot.view.step, "field": field, "range": range})
}

/// The probed node: a marker on it and its values beside it, inside the viewport.
fn probe(app: &ModelApp, plot: &Plot, painter: &egui::Painter, rect: Rect, t: &Tokens) {
    let Some(p) = app.probe.as_ref().filter(|p| p.step == plot.view.step && p.mode == plot.pick.mode && p.frame == plot.pick.frame) else { return };
    let Some(at) = plot.node_position(p.node).and_then(|w| crate::viewport::to_screen(app, w)) else { return };
    painter.circle(at, 5.0, t.accent, Stroke::new(2.0, t.overlay_text));
    label_beside(painter, at, p.lines.join("\n"), rect, t);
}

/// The results controls (the context panel's Results section): step, contour field,
/// deformation, modes, animation, what the viewport marks, reaction sums and the probed node.
/// Every control runs a command or a view setting.
pub fn controls(app: &mut ModelApp, plot: &Plot, ui: &mut egui::Ui, t: &Tokens) {
    let mut run: Vec<(&'static str, Value)> = Vec::new();
    let mut export: Option<Value> = None;
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
        // The legend's range: each end follows the field (Auto) or is clamped, e.g. below a
        // singular peak (#191).
        if let Some(l) = &plot.contour {
            for (label, key, set, value) in [("Legend max", "max", plot.view.range_max, l.max.1), ("Legend min", "min", plot.view.range_min, l.min.1)]
            {
                ui.label(label);
                ui.horizontal(|ui| {
                    let mut auto = set.is_none();
                    let tip = format!("Follow the field's {}; untick to clamp the legend", if key == "max" { "maximum" } else { "minimum" });
                    if ui.checkbox(&mut auto, "Auto").on_hover_text(tip).changed() {
                        run.push(("results.contour", legend_params(plot, key, if auto { json!("auto") } else { json!(value) })));
                    }
                    let mut v = set.unwrap_or(value);
                    let speed = (l.max.1 - l.min.1).abs().max(1e-9) * 0.005;
                    let drag = egui::DragValue::new(&mut v)
                        .speed(speed)
                        .custom_formatter(|v, _| number(v))
                        .custom_parser(|s| s.replace(',', "").trim().parse::<f64>().ok());
                    if ui.add_enabled(set.is_some(), drag).changed() {
                        run.push(("results.contour", legend_params(plot, key, json!(v))));
                    }
                });
                ui.end_row();
            }
        }
        ui.label("Deformed");
        ui.horizontal(|ui| {
            let mut deformed = plot.view.deformed;
            if ui.checkbox(&mut deformed, "").changed() {
                run.push(("results.deformed", json!({"show": deformed})));
            }
            let mut auto = plot.view.scale.is_none();
            let tip = if true_scale(&plot.result) {
                "Draw the displacements at true scale (they are large already)"
            } else {
                "Scale the largest displacement to a tenth of the model"
            };
            if ui.checkbox(&mut auto, "Auto").on_hover_text(tip).changed() {
                run.push(("results.deformed", json!({"scale": if auto { json!("auto") } else { json!(plot.scale) }})));
            }
            // The scale ticking Deformed draws with, also while it is off.
            let mut scale = plot.scale;
            let drag = egui::DragValue::new(&mut scale)
                .speed(plot.scale.abs().max(1e-6) * 0.01)
                .range(0.0..=1e12)
                .prefix("× ")
                .custom_formatter(|v, _| number(v))
                .custom_parser(|s| s.replace(',', "").trim().parse::<f64>().ok());
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
        ui.label("Show");
        ui.horizontal(|ui| {
            if plot.contour.is_some() {
                ui.checkbox(&mut app.ui.result_extremes, "Max/Min").on_hover_text("Mark the nodes of the largest and smallest value");
            }
            ui.checkbox(&mut app.ui.result_symbols, "Loads & BCs").on_hover_text("The symbols of the step's boundary conditions and loads");
        });
        ui.end_row();
    });
    if let Kind::Explicit(e) = plot.result.kind() {
        ui.separator();
        explicit(ui, e, plot, t, &mut run);
    } else if let Some((k, factor, n)) = plot.frame().filter(|_| matches!(plot.result.kind(), Kind::Static(_))) {
        // A nonlinear static step: its increments, the deformed shape following the slider.
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Increment");
            let mut frame = k;
            if ui.add(egui::Slider::new(&mut frame, 0..=n.saturating_sub(1))).changed() {
                run.push(("results.frame", json!({"frame": frame})));
            }
        });
        ui.label(RichText::new(format!("load factor {}", number(factor))).color(t.text_dim));
    } else if let Kind::Frequency(f) = plot.result.kind() {
        ui.separator();
        ui.label(RichText::new("Modes").color(t.text_dim));
        egui::ScrollArea::vertical().id_salt("fc_results_modes").max_height(150.0).show(ui, |ui| {
            for (i, m) in f.modes.iter().enumerate() {
                let mode = i + 1;
                if ui
                    .selectable_label(
                        plot.view.mode == mode,
                        format!("{mode}   {}", crate::results::mode_value(f.buckling.is_some(), m.eigenvalue, m.frequency)),
                    )
                    .clicked()
                {
                    run.push(("results.modes", json!({"mode": mode})));
                }
            }
        });
    } else if let Kind::Heat(h) = plot.result.kind() {
        ui.separator();
        heat_flows(ui, &fieldcraft_engine::results::heat_flows(h), t);
    }
    if matches!(plot.result.kind(), Kind::Static(_)) {
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Reaction sums").on_hover_text("Reaction force and moment per boundary-condition set (results.reactions)").clicked() {
                run.push(("results.reactions", json!({"step": plot.view.step})));
            }
            if ui.button("Export CSV…").on_hover_text("Write the reaction sums to a CSV file (results.export.csv)").clicked() {
                export = Some(json!({"what": "reactions", "step": plot.view.step}));
            }
        });
        if let Some(r) = app.reactions.as_ref().filter(|r| r["step"] == plot.view.step.as_str()) {
            reactions_table(ui, r, plot, t);
        }
    }
    ui.separator();
    if let Some(p) = probed(app, plot, ui, t) {
        export = Some(p);
    }
    if let Some(p) = export {
        app.export_csv(p);
    }
    for (id, params) in run {
        // The reaction sums land in `app.reactions` (ModelApp::run_form).
        let _ = app.run(id, params);
    }
}

/// The probed node's values, one row each, or how to probe one; the export's parameters when
/// its Export CSV… is clicked.
fn probed(app: &ModelApp, plot: &Plot, ui: &mut egui::Ui, t: &Tokens) -> Option<Value> {
    let Some(p) = app.probe.as_ref().filter(|p| p.step == plot.view.step && p.mode == plot.pick.mode && p.frame == plot.pick.frame) else {
        ui.label(RichText::new("Click the model to probe a node").small().color(t.text_dim));
        return None;
    };
    ui.label(RichText::new(format!("Node {}", p.node)).strong());
    egui::Grid::new("fc_results_probe").num_columns(2).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
        for (what, value) in &p.rows {
            ui.label(RichText::new(what).color(t.text_dim));
            ui.add(egui::Label::new(value).extend());
            ui.end_row();
        }
    });
    ui.button("Export CSV…").on_hover_text("Write the node's results to a CSV file (results.export.csv)").clicked().then(|| {
        let mut params = json!({"what": "probe", "step": p.step, "nodes": [p.node]});
        if matches!(plot.result.kind(), Kind::Frequency(_)) {
            params["mode"] = json!(p.mode);
        }
        if let Some(f) = p.frame {
            params["frame"] = json!(f);
        }
        params
    })
}

/// An explicit step: a frame slider (the deformed shape and contours follow it) and the energy
/// histories, plotted against time with a cursor at the frame shown (a click or drag moves it)
/// and read there.
fn explicit(ui: &mut egui::Ui, e: &ExplicitResult, plot: &Plot, t: &Tokens, run: &mut Vec<(&'static str, Value)>) {
    let time_unit = Quantity::Time.unit(plot.units);
    if let Some((k, time, n)) = plot.frame() {
        ui.horizontal(|ui| {
            ui.label("Frame");
            let mut frame = k;
            if ui.add(egui::Slider::new(&mut frame, 0..=n.saturating_sub(1))).changed() {
                run.push(("results.frame", json!({"frame": frame})));
            }
        });
        ui.label(RichText::new(format!("t = {}", with_unit(number(time), time_unit.as_deref()))).color(t.text_dim));
    }
    let h = &e.history;
    if h.time.is_empty() {
        return;
    }
    let series: [(&str, Color32, fn(&Energies) -> f64); 4] = [
        ("Kinetic", t.accent, |e| e.kinetic),
        ("Internal", t.ok, |e| e.internal),
        ("Hourglass", t.warm, |e| e.hourglass),
        ("External work", t.error, |e| e.external),
    ];
    let energy_unit = Quantity::Energy.unit(plot.units);
    ui.label(RichText::new(titled("Energies", energy_unit.as_deref())).color(t.text_dim));
    let (outer, response) = ui.allocate_exact_size(vec2(ui.available_width(), PLOT_HEIGHT + AXIS_BOTTOM), egui::Sense::click_and_drag());
    let rect = Rect::from_min_max(outer.min + vec2(AXIS_LEFT, 0.0), outer.max - vec2(0.0, AXIS_BOTTOM));
    let painter = ui.painter_at(outer);
    painter.rect_filled(rect, 2.0, t.field);
    let t_end = h.time.last().copied().unwrap_or(0.0).max(f64::MIN_POSITIVE);
    // The value axis spans every curve and 0 (a curve below 0 stays in the frame).
    let values = || h.energies.iter().flat_map(|e| series.iter().map(move |(_, _, f)| f(e))).filter(|v| v.is_finite());
    let hi = values().fold(0.0, f64::max);
    let lo = values().fold(0.0, f64::min);
    let span = if hi > lo { hi - lo } else { 1.0 };
    let at = |time: f64, v: f64| {
        pos2(rect.left() + (time / t_end) as f32 * rect.width(), rect.bottom() - 2.0 - ((v - lo) / span) as f32 * (rect.height() - 4.0))
    };
    // Axes: the value range left, the time range under it.
    let small = FontId::proportional(10.0);
    let scale = hi.abs().max(lo.abs());
    painter.text(pos2(rect.left() - 4.0, rect.top()), Align2::RIGHT_TOP, number_in(hi, scale), small.clone(), t.text_dim);
    painter.text(pos2(rect.left() - 4.0, rect.bottom()), Align2::RIGHT_BOTTOM, number_in(lo, scale), small.clone(), t.text_dim);
    if lo < 0.0 && hi > 0.0 {
        let zero = at(0.0, 0.0).y;
        painter.line_segment([pos2(rect.left(), zero), pos2(rect.right(), zero)], Stroke::new(1.0, t.border));
    }
    painter.text(pos2(rect.left(), rect.bottom() + 2.0), Align2::LEFT_TOP, "0", small.clone(), t.text_dim);
    let end = format!("t {}", with_unit(number(t_end), time_unit.as_deref()));
    painter.text(pos2(rect.right(), rect.bottom() + 2.0), Align2::RIGHT_TOP, end, small, t.text_dim);
    // At most a few hundred points per line, however long the history.
    let stride = (h.time.len() / MAX_PLOT_POINTS).max(1);
    for (_, color, f) in &series {
        let points: Vec<egui::Pos2> =
            h.time.iter().zip(&h.energies).step_by(stride).filter(|(_, e)| f(e).is_finite()).map(|(time, e)| at(*time, f(e))).collect();
        painter.add(egui::Shape::line(points, Stroke::new(1.5, *color)));
    }
    let nearest = |time: f64| h.time.iter().enumerate().min_by(|a, b| (a.1 - time).abs().total_cmp(&(b.1 - time).abs())).map_or(0, |(i, _)| i);
    let time_at = |x: f32| f64::from(((x - rect.left()) / rect.width()).clamp(0.0, 1.0)) * t_end;
    // A click or drag on the plot shows the frame nearest that time.
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        run.push(("results.frame", json!({"time": time_at(p.x)})));
    }
    // Hover reads the curves at the pointer's time.
    if let Some(p) = response.hover_pos().filter(|p| rect.contains(*p)) {
        let x = p.x;
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, t.border));
        if let Some(e) = h.energies.get(nearest(time_at(x))) {
            let mut lines = vec![format!("t = {}", with_unit(number(time_at(x)), time_unit.as_deref()))];
            lines.extend(series.iter().map(|(name, _, f)| format!("{name} {}", number_in(f(e), scale))));
            response.clone().on_hover_text(lines.join("\n"));
        }
    }
    // The frame's time: the cursor, and the values there.
    let now = plot.frame().map_or(t_end, |(_, time, _)| time);
    let x = at(now, 0.0).x;
    painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, t.text));
    let Some(at_now) = h.energies.get(nearest(now)) else { return };
    let frame = plot.frame().map_or_else(String::new, |(k, ..)| format!(", frame {k}"));
    ui.label(RichText::new(format!("At t = {}{frame}", with_unit(number(now), time_unit.as_deref()))).small().color(t.text_dim));
    egui::Grid::new("fc_results_energies").num_columns(2).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
        for (name, color, f) in &series {
            ui.label(RichText::new(*name).color(*color));
            ui.label(with_unit(number_in(f(at_now), scale), energy_unit.as_deref()));
            ui.end_row();
        }
        ui.label(RichText::new("Balance").color(t.text_dim)).on_hover_text("The energy balance's relative error");
        ui.label(number(at_now.balance));
        ui.end_row();
    });
}

/// A heat step's balance: the heat each heat load carries in (negative: out), the prescribed
/// temperatures' total, and what is left over (about 0 when steady).
fn heat_flows(ui: &mut egui::Ui, flows: &Value, t: &Tokens) {
    ui.label(RichText::new("Heat flow into the body").color(t.text_dim));
    egui::Grid::new("fc_results_heat").num_columns(2).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, name: &str, v: Option<f64>| {
            ui.label(name);
            ui.label(v.map_or("—".into(), number));
            ui.end_row();
        };
        for l in flows["loads"].as_array().into_iter().flatten() {
            row(ui, &format!("{} ({})", l["name"].as_str().unwrap_or("?"), l["type"].as_str().unwrap_or("").replace('_', " ")), l["heat"].as_f64());
        }
        row(ui, "Prescribed temperatures", flows["reactions"].as_f64());
        row(ui, "Imbalance", flows["imbalance"].as_f64());
    });
}

/// The reaction sums: per set and over the model, the force and the moment about the point, and
/// the applied total beside the reacted one (they balance in equilibrium). Round-off below a
/// billionth of the largest force (moment) is 0.
fn reactions_table(ui: &mut egui::Ui, r: &Value, plot: &Plot, t: &Tokens) {
    let vector = |v: &Value| -> Option<[f64; 3]> { serde_json::from_value(v.clone()).ok() };
    let rows: Vec<(String, Option<[f64; 3]>, Option<[f64; 3]>)> = r["sets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            let nodes = s["nodes"].as_u64().unwrap_or(0);
            (format!("{} ({} nodes)", s["set"].as_str().unwrap_or("?"), count(nodes)), vector(&s["force"]), vector(&s["moment"]))
        })
        .chain(std::iter::once(("All nodes".to_string(), vector(&r["total"]["force"]), vector(&r["total"]["moment"]))))
        .collect();
    let applied = vector(&r["applied"]);
    let largest = |vs: &mut dyn Iterator<Item = [f64; 3]>| vs.flatten().map(f64::abs).filter(|x| x.is_finite()).fold(0.0, f64::max);
    let force_scale = largest(&mut rows.iter().filter_map(|r| r.1).chain(applied));
    let moment_scale = largest(&mut rows.iter().filter_map(|r| r.2));
    let (force, moment) = (Quantity::Force.unit(plot.units), Quantity::Moment.unit(plot.units));
    let about = vector(&r["about"]).unwrap_or_default();
    ui.label(RichText::new(format!("Moments about ({})", components(&about, 0.0))).small().color(t.text_dim));
    let cells = |ui: &mut egui::Ui, v: Option<[f64; 3]>, scale: f64| {
        for k in 0..3 {
            ui.add(egui::Label::new(v.and_then(|v| v.get(k).copied()).map_or("—".into(), |x| number_in(x, scale))).extend());
        }
    };
    egui::Grid::new("fc_results_reactions").num_columns(4).spacing(vec2(8.0, 2.0)).striped(true).show(ui, |ui| {
        for h in ["", "X", "Y", "Z"] {
            ui.label(RichText::new(h).small().color(t.text_dim));
        }
        ui.end_row();
        for (name, f, m) in &rows {
            ui.label(RichText::new(name).strong());
            ui.end_row();
            ui.label(RichText::new(titled("F", force.as_deref())).color(t.text_dim));
            cells(ui, *f, force_scale);
            ui.end_row();
            ui.label(RichText::new(titled("M", moment.as_deref())).color(t.text_dim));
            cells(ui, *m, moment_scale);
            ui.end_row();
        }
        if applied.is_some() {
            ui.label(RichText::new(titled("Applied F", force.as_deref())).color(t.text_dim));
            cells(ui, applied, force_scale);
            ui.end_row();
        }
    });
    // Equilibrium: the reactions over the whole model balance the applied loads.
    if let (Some(total), Some(applied)) = (rows.last().and_then(|r| r.1), applied) {
        let residual = (0..3).map(|k| (total[k] + applied[k]).abs()).fold(0.0, f64::max);
        let balanced = residual <= 1e-6 * force_scale.max(f64::MIN_POSITIVE);
        let (text, color) = if balanced {
            ("✔ Reactions balance the applied loads".to_string(), t.ok)
        } else {
            (format!("Reactions and applied loads differ by {}", with_unit(number(residual), force.as_deref())), t.warm)
        };
        ui.label(RichText::new(text).small().color(color));
    }
}
