//! The setup checklist at the top of the browser: the analysis stages from geometry to results
//! ([`fieldcraft_engine::progress`]), each ticked when done, the next one highlighted, and
//! optional ones dashed. A click on a stage starts its command (as its toolbar button would);
//! a blocked stage's hover says what it waits for. It shows once the model has geometry or a
//! mesh (before that the viewport's Start card is the entry), and the × hides it until the
//! View menu shows it again; the choice is remembered per user.

use egui::{FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};
use fieldcraft_engine::progress::{Stage, StageState, Status};

use crate::ModelApp;
use crate::theme::Tokens;

const ROW_H: f32 = 20.0;

/// Where a stage's row was drawn (`ui.inspect`'s `layout`).
pub fn region(stage: Stage) -> &'static str {
    match stage {
        Stage::Geometry => "setup/geometry",
        Stage::Prepare => "setup/prepare",
        Stage::Mesh => "setup/mesh",
        Stage::Material => "setup/material",
        Stage::Section => "setup/section",
        Stage::Sets => "setup/sets",
        Stage::BoundaryConditions => "setup/boundary_conditions",
        Stage::Loads => "setup/loads",
        Stage::Step => "setup/step",
        Stage::Solve => "setup/solve",
        Stage::Results => "setup/results",
    }
}

pub fn card(app: &mut ModelApp, ui: &mut egui::Ui) {
    if !app.ui.show_setup {
        return;
    }
    let stages = app.setup_stages();
    if stages.first().is_none_or(|g| g.status != Status::Done) {
        return;
    }
    let t = Tokens::get(app.ui.dark);
    let mut start = None;
    let frame = egui::Frame::NONE.fill(t.dialog_bg).stroke(Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(6));
    let r = frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("SETUP").font(crate::theme::label_font(ui.ctx(), 10.5)).color(t.text_dim));
            let done = stages.iter().filter(|s| matches!(s.status, Status::Done)).count();
            ui.label(egui::RichText::new(format!("{done} of {}", stages.len())).size(10.5).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.add(egui::Button::new(egui::RichText::new("×").size(13.0).color(t.text_dim)).frame(false));
                if close.on_hover_text("Hide the setup checklist (View › Setup checklist shows it again)").clicked() {
                    app.ui.show_setup = false;
                }
            });
        });
        ui.add_space(2.0);
        for s in stages.iter() {
            if row(app, ui, &t, s) {
                start = Some(s.command);
            }
        }
    });
    app.place("setup", r.response.rect);
    ui.add_space(6.0);
    if let Some(command) = start {
        // A command that cannot run yet says why in the status bar.
        let _ = app.start(command);
    }
}

/// One stage; true when clicked.
fn row(app: &mut ModelApp, ui: &mut egui::Ui, t: &Tokens, s: &StageState) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    app.place(region(s.stage), r);
    let next = s.status == Status::Next;
    if next {
        ui.painter().rect_filled(r, 3.0, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    let mark = Rect::from_center_size(pos2(r.left() + 10.0, r.center().y), vec2(10.0, 10.0));
    paint_mark(ui.painter(), mark, s.status, t);
    let colour = match s.status {
        Status::Done | Status::Next => t.text,
        Status::Todo | Status::Blocked | Status::Optional => t.text_dim,
    };
    let x = r.left() + 22.0;
    ui.painter().with_clip_rect(r).text(pos2(x, r.center().y), egui::Align2::LEFT_CENTER, s.label, FontId::proportional(12.5), colour);
    let note = match s.status {
        Status::Next => Some("next"),
        Status::Optional => Some("optional"),
        _ => None,
    };
    if let Some(n) = note {
        let c = if next { t.accent } else { t.text_dim };
        ui.painter().text(pos2(r.right() - 4.0, r.center().y), egui::Align2::RIGHT_CENTER, n, FontId::proportional(11.0), c);
    }
    let tip = match &s.detail {
        Some(why) => format!("{}\n\n{why}", s.place),
        None => s.place.clone(),
    };
    let resp = resp.on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s.label));
    resp.clicked()
}

/// ✓ done, → next, – optional, an open circle for the rest (struck through when blocked).
fn paint_mark(p: &egui::Painter, r: Rect, status: Status, t: &Tokens) {
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * r.width(), r.top() + y * r.height());
    match status {
        Status::Done => {
            p.line_segment([at(0.05, 0.55), at(0.4, 0.88)], Stroke::new(1.8, t.ok));
            p.line_segment([at(0.4, 0.88), at(0.95, 0.15)], Stroke::new(1.8, t.ok));
        }
        Status::Next => {
            p.line_segment([at(0.0, 0.5), at(0.95, 0.5)], Stroke::new(1.6, t.accent));
            p.line_segment([at(0.55, 0.15), at(0.95, 0.5)], Stroke::new(1.6, t.accent));
            p.line_segment([at(0.55, 0.85), at(0.95, 0.5)], Stroke::new(1.6, t.accent));
        }
        Status::Optional => {
            p.line_segment([at(0.15, 0.5), at(0.85, 0.5)], Stroke::new(1.4, t.text_dim));
        }
        Status::Todo => {
            p.circle_stroke(r.center(), r.width() * 0.4, Stroke::new(1.2, t.text_dim));
        }
        Status::Blocked => {
            p.circle_stroke(r.center(), r.width() * 0.4, Stroke::new(1.2, t.text_dim));
            p.line_segment([at(0.2, 0.8), at(0.8, 0.2)], Stroke::new(1.2, t.text_dim));
        }
    }
}
