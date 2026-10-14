//! The 3D viewport: the model drawn on the GPU (or on the CPU without one), camera navigation,
//! the navigation bar (bottom centre) and the orientation gizmo (top right), as in SolidCraft.
//!
//! Navigation: left drag orbits (or pans or zooms: the navigation bar's modes), right or middle
//! drag (or Shift + left drag) pans, the wheel or a pinch zooms at the cursor, a double click or F
//! frames the model. The gizmo's axis tips turn the view to look along them. The bar and the gizmo
//! follow SolidCraft's `viewport.rs` (MIT OR Apache-2.0), see ATTRIBUTION.md.
//!
//! Selection ([`crate::selection`]): what a click would pick is highlighted under the pointer; a
//! click selects it (Shift adds, Cmd/Ctrl adds or removes, a click on nothing clears), Escape
//! clears. The selection bar (top centre) chooses what is picked and turns on box selection,
//! where a left drag selects the visible nodes, element faces or elements inside the box. A
//! right click offers a set made from the selection.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use fieldcraft_render::{StandardView, Vec3};

use crate::gpu::{GpuScene, ViewportCallback};
use crate::icons::{self, Look};
use crate::selection::{Filter, Mode};
use crate::theme::Tokens;
use crate::{ModelApp, NavMode, Style};

/// Wheel zoom per point scrolled.
const ZOOM_PER_POINT: f64 = 0.0015;

pub fn show(app: &mut ModelApp, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    app.viewport.rect = Some(rect);
    app.place("viewport", rect);
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let boxing = app.pick.box_mode && app.pick.filter.boxes();
    navigate(app, ui, &response, rect, boxing);
    if boxing {
        box_drag(app, ui, &response);
    }
    if response.clicked()
        && let Some(at) = response.interact_pointer_pos()
    {
        let _ = app.select_at(at, mode(ui.input(|i| i.modifiers)));
        probe_at(app, at, rect);
    }
    if !app.capturing {
        hover(app, &response);
    }
    context_menu(app, &response);
    let t = Tokens::get(app.ui.dark);
    let painter = ui.painter_at(rect);
    gradient(&painter, rect, t.viewport_top, t.viewport_bottom);
    if app.viewport.gpu.is_some() {
        paint_gpu(app, ui, &painter, rect);
        labels(app, &painter, &t);
    } else {
        paint_cpu(app, ui, &painter, rect);
    }
    sampled_note(app, &painter, rect, &t);
    hover_label(app, &painter, &t);
    if let Some((a, b)) = app.pick.dragging {
        let r = Rect::from_two_pos(a, b);
        painter.rect(r, 0.0, t.pick_selected.gamma_multiply(0.12), Stroke::new(1.0, t.pick_selected), egui::StrokeKind::Inside);
    }
    if app.display().bounds.is_empty() && app.busy().is_none() {
        start_screen(app, ui, rect, &t);
    } else {
        selection_bar(app, ui, rect, &t);
    }
    crate::results_panel::show(app, ui, rect, &t);
    axis_view(app, ui, rect, &t);
    nav_bar(app, ui, rect, &t);
}

/// The empty viewport: open a file or a sample (the samples are ready to solve).
fn start_screen(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let mut open = None;
    let mut pick = false;
    let area = Rect::from_center_size(rect.center(), vec2(440.0_f32.min(rect.width() - 24.0), 300.0_f32.min(rect.height() - 24.0)));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::Frame::NONE.fill(t.overlay).corner_radius(6.0).inner_margin(egui::Margin::same(14)).show(&mut child, |ui| {
        ui.label(egui::RichText::new("Start").size(16.0).color(t.overlay_text));
        ui.label(egui::RichText::new("Open a model, a SolidCraft design or a tagged mesh — or drop one here.").color(t.text_dim));
        ui.add_space(4.0);
        pick = ui.button("Open…").clicked();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Samples").color(t.overlay_text));
        crate::files::sample_rows(ui, t, &mut open);
    });
    if pick {
        app.open_dialog(None);
    } else if let Some(n) = open {
        app.open_sample(n);
    }
}

/// A click on a results plot probes the node nearest to the face under the pointer.
fn probe_at(app: &mut ModelApp, at: Pos2, rect: Rect) {
    let Some(plot) = app.plot() else { return };
    let (origin, dir) = app.cam.ray(f64::from(at.x - rect.left()), f64::from(at.y - rect.top()), f64::from(rect.width()), f64::from(rect.height()));
    if let Some((node, _)) = plot.display.pick(origin, dir) {
        // A failure (a stale result) shows in the status bar.
        let _ = app.probe_node(node);
    }
}

/// How a click with these modifier keys changes the selection: Cmd/Ctrl adds or removes, Shift
/// adds, a plain click replaces.
fn mode(m: egui::Modifiers) -> Mode {
    if m.command || m.ctrl {
        Mode::Toggle
    } else if m.shift {
        Mode::Add
    } else {
        Mode::Replace
    }
}

/// Box selection: a left drag draws the box; on release, what is visible inside it is selected
/// (with the release's modifier keys).
fn box_drag(app: &mut ModelApp, ui: &egui::Ui, response: &egui::Response) {
    let pointer = response.interact_pointer_pos();
    if response.drag_started_by(egui::PointerButton::Primary) {
        app.pick.dragging = pointer.map(|p| (p, p));
    } else if let (Some((a, _)), Some(p)) = (app.pick.dragging, pointer)
        && response.dragged_by(egui::PointerButton::Primary)
    {
        app.pick.dragging = Some((a, p));
    }
    if response.drag_stopped_by(egui::PointerButton::Primary)
        && let Some((a, b)) = app.pick.dragging.take()
        && a.distance(b) > 3.0
    {
        app.select_box(a, b, mode(ui.input(|i| i.modifiers)));
    }
}

/// What a click would pick under the pointer, worked out again when the pointer, the camera, the
/// scene or the filter change.
fn hover(app: &mut ModelApp, response: &egui::Response) {
    let at = response.hover_pos().filter(|_| !response.dragged() && app.pick.dragging.is_none());
    let key = (at, app.cam, app.scene_key(), app.pick.filter, app.viewport.rect);
    if app.pick.hover_key == Some(key) {
        return;
    }
    app.pick.hover_key = Some(key);
    app.pick.hover_at = at;
    let item = at.and_then(|p| app.pick_at(p));
    app.set_hover(item);
}

/// The hovered item's name beside the pointer.
fn hover_label(app: &ModelApp, painter: &egui::Painter, t: &Tokens) {
    let (Some(item), Some(at)) = (&app.pick.hover, app.pick.hover_at) else { return };
    if !painter.clip_rect().contains(at) {
        return;
    }
    let galley = painter.layout_no_wrap(item.describe(), egui::FontId::proportional(12.0), t.overlay_text);
    let r = Rect::from_min_size(at + vec2(14.0, 14.0), galley.size()).expand2(vec2(5.0, 2.0));
    painter.rect_filled(r, 3.0, t.overlay);
    painter.galley(r.min + vec2(5.0, 2.0), galley, t.overlay_text);
}

/// The right-click menu: a set from the selection, clearing it, and the filter.
fn context_menu(app: &mut ModelApp, response: &egui::Response) {
    response.context_menu(|ui| {
        let has = !app.pick.selection.is_empty();
        if ui
            .add_enabled(has, egui::Button::new("Create set from selection…"))
            .on_hover_text("model.set.create with the selection as members")
            .clicked()
        {
            let _ = app.create_set_from_selection();
            ui.close();
        }
        if ui.add_enabled(has, egui::Button::new("Clear selection")).on_hover_text("Esc").clicked() {
            app.clear_selection();
            ui.close();
        }
        ui.separator();
        for f in Filter::ALL {
            if ui.radio(app.pick.filter == f, format!("Select {}", f.label().to_lowercase())).clicked() {
                app.pick.filter = f;
                ui.close();
            }
        }
    });
}

/// The selection bar, top centre: what a click picks, box selection, and what is selected.
fn selection_bar(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let area = egui::Area::new(egui::Id::new("fc_selection_bar"))
        .fixed_pos(pos2(rect.center().x, rect.top() + 10.0))
        .pivot(egui::Align2::CENTER_TOP)
        .constrain_to(rect)
        .order(egui::Order::Middle);
    let shown = area.show(ui.ctx(), |ui| {
        egui::Frame::NONE.fill(t.overlay).stroke(Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 4)).show(
            ui,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Select").color(t.text_dim));
                    egui::ComboBox::from_id_salt("fc_selection_filter").selected_text(app.pick.filter.label()).width(110.0).show_ui(ui, |ui| {
                        for (i, f) in Filter::ALL.into_iter().enumerate() {
                            // Geometry, then the mesh.
                            if i > 0 && Filter::ALL.get(i - 1).is_some_and(|p| p.geometry() != f.geometry()) {
                                ui.separator();
                            }
                            if ui.selectable_label(app.pick.filter == f, f.label()).clicked() {
                                app.pick.filter = f;
                            }
                        }
                    });
                    let boxes = app.pick.filter.boxes();
                    let tip = if boxes {
                        "Box selection: a left drag selects the visible items inside the box (right or middle drag pans)"
                    } else {
                        "Box selection picks nodes, element faces and elements"
                    };
                    if ui.add_enabled(boxes, egui::Button::selectable(app.pick.box_mode && boxes, "Box")).on_hover_text(tip).clicked() {
                        app.pick.box_mode = !app.pick.box_mode;
                    }
                    if !app.pick.selection.is_empty() {
                        ui.separator();
                        let mut text = app.pick.selection.summary();
                        if app.pick.selection.capped() {
                            text.push_str(" (capped)");
                        }
                        ui.label(egui::RichText::new(text).color(t.pick_selected));
                        if ui.small_button("×").on_hover_text("Clear the selection (Esc)").clicked() {
                            app.clear_selection();
                        }
                    }
                });
            },
        );
    });
    app.place("selection_bar", shown.response.rect);
}

fn navigate(app: &mut ModelApp, ui: &egui::Ui, response: &egui::Response, rect: Rect, boxing: bool) {
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
    let d = response.drag_delta();
    let (dx, dy) = (f64::from(d.x), f64::from(d.y));
    let shift = ui.input(|i| i.modifiers.shift);
    // In box selection a left drag draws the box.
    let left = response.dragged_by(egui::PointerButton::Primary) && !boxing;
    if left && !shift && app.nav == NavMode::Orbit {
        app.cam.orbit(dx, dy);
    } else if left && !shift && app.nav == NavMode::Zoom {
        app.cam.zoom_at((dy * ZOOM_PER_POINT * 4.0).exp(), None);
    } else if response.dragged_by(egui::PointerButton::Secondary) || response.dragged_by(egui::PointerButton::Middle) || left {
        app.cam.pan(dx, dy, h);
    }
    if response.double_clicked() || (response.hovered() && ui.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.is_none())) {
        app.fit_view();
    }
    if response.hovered() {
        let (scroll, pinch, at) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
        let factor = (-f64::from(scroll) * ZOOM_PER_POINT).exp() / f64::from(pinch).max(1e-3);
        if (factor - 1.0).abs() > 1e-9 {
            let anchor = at.and_then(|p| app.cam.target_plane_point(f64::from(p.x - rect.left()), f64::from(p.y - rect.top()), w, h));
            app.cam.zoom_at(factor, anchor);
        }
    }
}

/// A vertical gradient behind the model.
fn gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_gpu(app: &mut ModelApp, ui: &egui::Ui, painter: &egui::Painter, rect: Rect) {
    let ppp = ui.ctx().pixels_per_point();
    let key = app.scene().0;
    let radius = app.scene.1.radius;
    if app.viewport.uploaded != Some(key) {
        let g = GpuScene::from_scene(&app.scene.1, ppp);
        if let Ok(mut slot) = app.viewport.slot.lock() {
            *slot = Some(g);
            app.viewport.uploaded = Some(key);
        }
    }
    let cam = app.cam;
    let aspect = f64::from(rect.width()) / f64::from(rect.height()).max(1.0);
    let eye = cam.eye();
    let (overlay_key, overlay) = app.overlay();
    if app.viewport.overlay_uploaded != Some(overlay_key)
        && let Ok(mut slot) = app.viewport.overlay_slot.lock()
    {
        *slot = Some(GpuScene::from_scene(&overlay, ppp));
        app.viewport.overlay_uploaded = Some(overlay_key);
    }
    let cb = ViewportCallback {
        key,
        slot: app.viewport.slot.clone(),
        overlay_key,
        overlay_slot: app.viewport.overlay_slot.clone(),
        view_proj: cam.view_proj(aspect, radius).to_f32(),
        back: cam.back().to_f32(),
        eye: [eye.x as f32, eye.y as f32, eye.z as f32, if cam.fov > 0.0 { 1.0 } else { 0.0 }],
        size_px: [rect.width() * ppp, rect.height() * ppp],
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, cb));
}

/// Without a GPU: the CPU rasterizer into a texture, redrawn when the scene, camera or size
/// changes.
fn paint_cpu(app: &mut ModelApp, ui: &egui::Ui, painter: &egui::Painter, rect: Rect) {
    let ppp = ui.ctx().pixels_per_point();
    // A window larger than the renderer's limit is drawn smaller and scaled up.
    let (w, h) = fieldcraft_render::fit_size((rect.width() * ppp) as usize, (rect.height() * ppp) as usize);
    let size = [w, h];
    let cam = app.cam;
    let (overlay_key, overlay) = app.overlay();
    let key = (app.scene().0, overlay_key);
    let fresh = app.viewport.cpu.as_ref().is_some_and(|(_, k, c, s)| *k == key && *c == cam && *s == size);
    // A scene the renderer refused is reported once and not retried until it changes.
    if !fresh && app.viewport.cpu_failed != Some(key.0) {
        let c = match fieldcraft_render::render_layers(&app.scene.1, Some(&overlay), &cam, w, h) {
            Ok(c) => c,
            Err(e) => {
                app.viewport.cpu = None;
                app.viewport.cpu_failed = Some(key.0);
                app.set_status(format!("Viewport: {e}"), true);
                return;
            }
        };
        app.viewport.cpu_failed = None;
        let image = egui::ColorImage::from_rgba_unmultiplied([c.w, c.h], &c.rgba);
        let tex = match app.viewport.cpu.take() {
            Some((mut tex, ..)) => {
                tex.set(image, egui::TextureOptions::LINEAR);
                tex
            }
            None => ui.ctx().load_texture("fc_viewport_cpu", image, egui::TextureOptions::LINEAR),
        };
        app.viewport.cpu = Some((tex, key, cam, size));
    }
    if let Some((tex, ..)) = &app.viewport.cpu {
        painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The scene's labels (reference point names) beside their points, with the interface's font.
/// The CPU renderer draws them into its image.
fn labels(app: &mut ModelApp, painter: &egui::Painter, t: &Tokens) {
    let labels: Vec<(Vec3, String)> = app.scene().1.labels.iter().map(|l| (l.p, l.text.clone())).collect();
    let offset = fieldcraft_render::LABEL_OFFSET as f32;
    for (p, text) in labels {
        if let Some(at) = to_screen(app, p) {
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), t.overlay_text);
            let r = Rect::from_min_size(at + vec2(offset, -offset - galley.size().y), galley.size()).expand2(vec2(4.0, 1.0));
            painter.rect_filled(r, 3.0, t.overlay);
            painter.galley(r.min + vec2(4.0, 1.0), galley, t.overlay_text);
        }
    }
}

/// Bottom left: the definitions whose symbols are a sample of their nodes or faces.
fn sampled_note(app: &ModelApp, painter: &egui::Painter, rect: Rect, t: &Tokens) {
    let show = app.ui.show();
    let notes = &app.glyph_notes();
    if notes.is_empty() || !(show.loads || show.interactions) {
        return;
    }
    let text = format!("Symbols sampled: {}", notes.join("; "));
    painter.text(rect.left_bottom() + vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(11.0), t.text_dim);
}

/// The orientation gizmo's arm length and tip radius (points).
const AXIS_LENGTH: f32 = 38.0;
const AXIS_RADIUS: f32 = 11.0;

/// The standard view looking along `d` from outside (the one whose eye side is nearest `d`).
fn view_along(d: Vec3) -> StandardView {
    let mut best = (StandardView::Front, f64::NEG_INFINITY);
    for v in StandardView::ALL.into_iter().filter(|v| *v != StandardView::Iso) {
        let mut c = fieldcraft_render::Camera::default();
        c.set_view(v);
        let score = c.back().dot(d);
        if score > best.1 {
            best = (v, score);
        }
    }
    best.0
}

/// The orientation gizmo (top right): the six signed axes in the current view, back to front;
/// clicking a tip looks along it, the house returns to the home view.
fn axis_view(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let area = Rect::from_center_size(pos2(rect.right() - 68.0, rect.top() + 68.0), Vec2::splat(116.0));
    if !rect.contains_rect(area) {
        return;
    }
    app.place("axis_view", area);
    let c = area.center();
    let (r, u, b) = app.cam.basis();
    let mut tips: Vec<(&str, Vec3, Pos2, f64, Color32)> = [
        ("X", Vec3::X, t.axis_x),
        ("-X", Vec3::new(-1.0, 0.0, 0.0), t.axis_x),
        ("Y", Vec3::Y, t.axis_y),
        ("-Y", Vec3::new(0.0, -1.0, 0.0), t.axis_y),
        ("Z", Vec3::Z, t.axis_z),
        ("-Z", Vec3::new(0.0, 0.0, -1.0), t.axis_z),
    ]
    .into_iter()
    .map(|(name, d, col)| (name, d, c + vec2(d.dot(r) as f32 * AXIS_LENGTH, -(d.dot(u) as f32) * AXIS_LENGTH), d.dot(b), col))
    .collect();
    tips.sort_by(|a, b2| a.3.total_cmp(&b2.3));
    let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| area.contains(*p));
    let hovered = hover.and_then(|p| tips.iter().rposition(|(_, _, at, _, _)| at.distance(p) <= AXIS_RADIUS));
    let painter = ui.painter_at(rect);
    if hover.is_some() {
        painter.circle_filled(c, 55.0, t.overlay);
    }
    for (i, (name, _, at, depth, col)) in tips.iter().enumerate() {
        let positive = !name.starts_with('-');
        let ink = if *depth < -0.01 { col.gamma_multiply(0.55) } else { *col };
        painter.line_segment([c, *at], Stroke::new(if positive { 2.0 } else { 1.0 }, ink));
        let hot = hovered == Some(i);
        painter.circle_filled(*at, AXIS_RADIUS, if hot { t.accent_soft } else { t.overlay });
        painter.circle_stroke(*at, AXIS_RADIUS, Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { t.accent } else { ink }));
        painter.text(*at, egui::Align2::CENTER_CENTER, *name, egui::FontId::proportional(11.0), ink);
    }
    let resp = ui.interact(area, ui.id().with("fc_axis_view"), Sense::click());
    if let Some(i) = hovered
        && let Some((name, d, ..)) = tips.get(i)
    {
        let view = view_along(*d);
        let resp = resp.clone().on_hover_text(format!("Look from {name} ({})", view.label()));
        if resp.clicked() {
            app.set_view(view);
        }
    }
    let home = Rect::from_center_size(pos2(c.x - 44.0, c.y - 44.0), Vec2::splat(18.0));
    let hr = ui.interact(home, ui.id().with("fc_axis_home"), Sense::click());
    if hr.hovered() {
        painter.rect_filled(home.expand(2.0), 3.0, t.hover);
    }
    icons::paint(app, ui, "ui.home", home, Look::Normal);
    if hr.on_hover_text("Home view").clicked() {
        app.home_view();
    }
}

/// The navigation bar's buttons: (key, icon, tooltip).
const NAV: [(&str, &str, &str); 7] = [
    ("orbit", "ui.orbit", "Orbit: left drag turns the view"),
    ("pan", "ui.pan", "Pan: left drag moves the view (or right or middle drag)"),
    ("zoom", "ui.zoom", "Zoom: left drag up and down (or the wheel)"),
    ("fit", "ui.fit", "Fit the model (F, or a double click)"),
    ("home", "ui.home", "Home view"),
    ("perspective", "ui.perspective", "Perspective or orthographic"),
    ("style", "ui.style", "Visual style and what is shown"),
];

/// The navigation bar, bottom centre: navigation modes, fit, home, projection, and the display
/// menu (style, geometry, mesh, nodes).
fn nav_bar(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let w = NAV.len() as f32 * 30.0 + 10.0;
    let bar = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 22.0), vec2(w, 30.0));
    if !rect.contains_rect(bar) {
        return;
    }
    app.place("nav_bar", bar);
    ui.painter().rect(bar, 6.0, t.overlay, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let mut style_button = None;
    for (i, (key, icon, tip)) in NAV.iter().enumerate() {
        let br = Rect::from_min_size(pos2(bar.left() + 5.0 + i as f32 * 30.0, bar.top() + 2.0), Vec2::splat(26.0));
        let resp = ui.interact(br, ui.id().with(("fc_nav", *key)), Sense::click());
        let active = match *key {
            "orbit" => app.nav == NavMode::Orbit,
            "pan" => app.nav == NavMode::Pan,
            "zoom" => app.nav == NavMode::Zoom,
            "perspective" => app.ui.perspective,
            "style" => app.ui.menu.as_deref() == Some("view"),
            _ => false,
        };
        if active || resp.hovered() {
            ui.painter().rect_filled(br, 4.0, if active { t.accent_soft } else { t.hover });
        }
        icons::paint(app, ui, icon, br.shrink(4.0), Look::Normal);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, *tip));
        if resp.on_hover_text(*tip).clicked() {
            match *key {
                "orbit" => app.nav = NavMode::Orbit,
                "pan" => app.nav = NavMode::Pan,
                "zoom" => app.nav = NavMode::Zoom,
                "fit" => app.fit_view(),
                "home" => {
                    app.home_view();
                }
                "perspective" => app.ui.perspective = !app.ui.perspective,
                _ => app.toggle_menu("view"),
            }
        }
        if *key == "style" {
            style_button = Some(br);
        }
    }
    if let Some(br) = style_button {
        crate::frame::settings_menu(app, ui, "view", br, |app, ui| {
            let t = Tokens::get(app.ui.dark);
            crate::frame::menu_heading(ui, &t, "STYLE");
            for s in Style::ALL {
                if ui.radio(app.ui.style == s, s.label()).clicked() {
                    app.ui.style = s;
                }
            }
            ui.separator();
            crate::frame::menu_heading(ui, &t, "SHOW");
            ui.checkbox(&mut app.ui.show_geometry, "Geometry");
            ui.checkbox(&mut app.ui.show_mesh, "Mesh");
            ui.checkbox(&mut app.ui.show_nodes, "Nodes");
            ui.checkbox(&mut app.ui.show_loads, "Loads & BCs");
            ui.checkbox(&mut app.ui.show_interactions, "Interactions");
            ui.separator();
            ui.checkbox(&mut app.ui.show_tree, "Browser");
            ui.checkbox(&mut app.ui.show_timeline, "Timeline");
        });
    }
}

/// The screen point (points) of a world point in the viewport drawn last frame.
pub fn to_screen(app: &ModelApp, p: Vec3) -> Option<Pos2> {
    let rect = app.viewport.rect?;
    // The projection picking uses ([`ModelApp::projector`]).
    let (x, y, _) = app.cam.to_screen(p, f64::from(rect.width()), f64::from(rect.height()), app.scene.1.radius)?;
    Some(pos2(rect.left() + x as f32, rect.top() + y as f32))
}
