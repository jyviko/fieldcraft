//! The 3D viewport: the model drawn on the GPU (or on the CPU without one), camera navigation,
//! the standard-view buttons and an axis indicator.
//!
//! Navigation: left drag orbits, right or middle drag (or Shift + left drag) pans, the wheel or a
//! pinch zooms at the cursor, a double click or F frames the model.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use modelcraft_render::{StandardView, Vec3};

use crate::ModelApp;
use crate::gpu::{GpuScene, ViewportCallback};
use crate::theme::Tokens;

/// Wheel zoom per point scrolled.
const ZOOM_PER_POINT: f64 = 0.0015;

pub fn show(app: &mut ModelApp, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    app.viewport.rect = Some(rect);
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    navigate(app, ui, &response, rect);
    let t = Tokens::get(app.ui.dark);
    let painter = ui.painter_at(rect);
    gradient(&painter, rect, t.viewport_top, t.viewport_bottom);
    if app.viewport.gpu.is_some() {
        paint_gpu(app, ui, &painter, rect);
    } else {
        paint_cpu(app, ui, &painter, rect);
    }
    if app.session.model.nodes().is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "The model is empty.\nAdd nodes and elements from the Commands panel, or start with --sample.",
            egui::FontId::proportional(15.0),
            t.text_dim,
        );
    }
    axis_indicator(app, &painter, rect, &t);
    view_buttons(app, ui, rect, &t);
}

fn navigate(app: &mut ModelApp, ui: &egui::Ui, response: &egui::Response, rect: Rect) {
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
    let d = response.drag_delta();
    let (dx, dy) = (f64::from(d.x), f64::from(d.y));
    let shift = ui.input(|i| i.modifiers.shift);
    if response.dragged_by(egui::PointerButton::Primary) && !shift {
        app.cam.orbit(dx, dy);
    } else if response.dragged_by(egui::PointerButton::Secondary)
        || response.dragged_by(egui::PointerButton::Middle)
        || (response.dragged_by(egui::PointerButton::Primary) && shift)
    {
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
    let cb = ViewportCallback {
        key,
        slot: app.viewport.slot.clone(),
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
    let size = [((rect.width() * ppp) as usize).max(1), ((rect.height() * ppp) as usize).max(1)];
    let cam = app.cam;
    let key = app.scene().0;
    let fresh = app.viewport.cpu.as_ref().is_some_and(|(_, k, c, s)| *k == key && *c == cam && *s == size);
    if !fresh {
        let c = modelcraft_render::render(&app.scene.1, &cam, size[0], size[1]);
        let image = egui::ColorImage::from_rgba_unmultiplied([c.w, c.h], &c.rgba);
        let tex = match app.viewport.cpu.take() {
            Some((mut tex, ..)) => {
                tex.set(image, egui::TextureOptions::LINEAR);
                tex
            }
            None => ui.ctx().load_texture("mc_viewport_cpu", image, egui::TextureOptions::LINEAR),
        };
        app.viewport.cpu = Some((tex, key, cam, size));
    }
    if let Some((tex, ..)) = &app.viewport.cpu {
        painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// X, Y and Z drawn from the lower-left corner in the current view direction.
fn axis_indicator(app: &ModelApp, painter: &egui::Painter, rect: Rect, t: &Tokens) {
    let (r, u, b) = app.cam.basis();
    let origin = rect.left_bottom() + vec2(44.0, -44.0);
    let len = 30.0;
    let mut axes = [(Vec3::X, t.axis_x, "X"), (Vec3::Y, t.axis_y, "Y"), (Vec3::Z, t.axis_z, "Z")];
    // Farthest first, so the nearest axis is drawn on top.
    axes.sort_by(|a, b2| a.0.dot(b).total_cmp(&b2.0.dot(b)));
    for (axis, color, name) in axes {
        let tip = origin + Vec2::new(axis.dot(r) as f32, -axis.dot(u) as f32) * len;
        painter.line_segment([origin, tip], Stroke::new(2.0, color));
        painter.text(tip + (tip - origin).normalized() * 9.0, egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(12.0), color);
    }
}

/// The standard views and Fit, as text buttons along the viewport's top edge.
fn view_buttons(app: &mut ModelApp, ui: &mut egui::Ui, rect: Rect, t: &Tokens) {
    let area = Rect::from_min_size(rect.left_top() + vec2(8.0, 8.0), vec2(rect.width() - 16.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::left_to_right(egui::Align::Center)));
    egui::Frame::NONE.fill(t.overlay).corner_radius(4.0).inner_margin(egui::Margin::symmetric(4, 2)).show(&mut child, |ui| {
        ui.horizontal(|ui| {
            for v in StandardView::ALL {
                if view_button(ui, v.label(), t).clicked() {
                    app.set_view(v);
                }
            }
            ui.separator();
            if view_button(ui, "Fit", t).on_hover_text("Frame the whole model (F, or double-click)").clicked() {
                app.fit_view();
            }
        });
    });
}

fn view_button(ui: &mut egui::Ui, label: &str, t: &Tokens) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).color(t.overlay_text).size(12.0)).frame(false).min_size(vec2(0.0, 20.0)))
}

/// The screen point (points) of a world point in the viewport drawn last frame.
pub fn to_screen(app: &ModelApp, p: Vec3) -> Option<Pos2> {
    let rect = app.viewport.rect?;
    let radius = app.scene.1.radius.max(1.0);
    let (x, y, _) = app.cam.to_screen(p, f64::from(rect.width()), f64::from(rect.height()), radius)?;
    Some(pos2(rect.left() + x as f32, rect.top() + y as f32))
}
