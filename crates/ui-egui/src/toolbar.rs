//! The toolbar: one tab per stage of the analyst's workflow, each a row of panels of icon buttons,
//! all from the capability catalogue (`modelcraft_engine::catalogue`).
//!
//! A ready entry starts its command (the same as the Commands panel: a command without
//! parameters runs, one with parameters opens its form); its related commands (edit, delete…)
//! are in the drop-down beside it. A planned entry is drawn greyed with its milestone and does
//! nothing; its tooltip says when it comes. Structure follows SolidCraft's
//! `crates/ui-egui/src/toolbar.rs` (MIT OR Apache-2.0), see ATTRIBUTION.md.

use egui::{Align2, FontId, Rect, RichText, Sense, Stroke, vec2};
use modelcraft_engine::catalogue::{self, Capability, Tab};

use crate::ModelApp;
use crate::theme::Tokens;

/// Icon side (points).
pub const ICON: f32 = 24.0;
/// Button height and the width its label wraps at (points).
const BUTTON_H: f32 = 60.0;
const LABEL_W: f32 = 64.0;
/// The drop-down strip beside a button with related commands.
const ARROW_W: f32 = 12.0;

pub fn toolbar(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let mut action = None;
    egui::Panel::top("mc_toolbar").frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| {
        ui.horizontal(|ui| {
            for tab in Tab::ALL {
                let text = RichText::new(tab.label()).color(if app.ui.tab == tab { t.accent } else { t.text });
                if ui.selectable_label(app.ui.tab == tab, text).clicked() {
                    app.ui.tab = tab;
                }
            }
        });
        ui.add_space(2.0);
        app.toolbar_rects.clear();
        egui::ScrollArea::horizontal().id_salt("mc_toolbar_scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, panel) in catalogue::panels(app.ui.tab).into_iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for c in catalogue::entries(app.ui.tab, panel) {
                                if let Some(a) = button(app, ui, &t, c) {
                                    action = Some(a);
                                }
                            }
                        });
                        let width = ui.min_rect().width();
                        ui.allocate_ui(vec2(width, 14.0), |ui| {
                            ui.centered_and_justified(|ui| ui.label(RichText::new(panel).small().color(t.text_dim)));
                        });
                    });
                }
            });
        });
    });
    if let Some(id) = action {
        // Errors already show in the status bar.
        let _ = app.start(id);
    }
}

/// One entry's button; returns the id to start when it (or its drop-down) was clicked.
fn button(app: &mut ModelApp, ui: &mut egui::Ui, t: &Tokens, c: &'static Capability) -> Option<&'static str> {
    let enabled = c.command().is_some();
    let mut job = egui::text::LayoutJob::simple(c.label.to_string(), FontId::proportional(11.0), if enabled { t.text } else { t.text_dim }, LABEL_W);
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    let width = galley.size().x.max(ICON + 12.0) + 8.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, BUTTON_H), sense);
    app.toolbar_rects.push((c.id, rect));
    let p = ui.painter();
    if resp.hovered() && enabled {
        p.rect_filled(rect, 4.0, t.hover);
    }
    let icon_rect = Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 6.0 + ICON / 2.0), vec2(ICON, ICON));
    let px = (ICON * ui.ctx().pixels_per_point()).round() as u32;
    match app.icons.get(ui.ctx(), c.id, app.ui.dark, enabled, px) {
        Some(tex) => {
            p.image(tex.id(), icon_rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        None => {
            p.rect_stroke(icon_rect.shrink(3.0), 3.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
        }
    }
    p.galley(egui::pos2(rect.center().x, icon_rect.bottom() + 4.0), galley, t.text);
    if let Some(m) = c.milestone() {
        let font = FontId::proportional(9.0);
        let text = p.layout_no_wrap(m.to_string(), font, t.text);
        let badge = Rect::from_min_size(egui::pos2(rect.right() - text.size().x - 6.0, rect.top() + 1.0), text.size() + vec2(4.0, 0.0));
        p.rect_filled(badge, 3.0, t.panel_header);
        p.rect_stroke(badge, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        p.galley(badge.min + vec2(2.0, 0.0), text, t.text);
    }
    let tip = match c.milestone() {
        Some(m) => format!("{} — coming soon ({m})\n{}", c.label, c.tip),
        None => format!("{}\n{}\n{}", c.label, c.tip, c.command().unwrap_or_default()),
    };
    let resp = resp.on_hover_text(tip);
    let mut action = resp.clicked().then_some(c.id);
    if enabled && !c.more.is_empty() {
        let (arrow, arrow_resp) = ui.allocate_exact_size(vec2(ARROW_W, BUTTON_H), Sense::click());
        if arrow_resp.hovered() {
            ui.painter().rect_filled(arrow, 4.0, t.hover);
        }
        ui.painter().text(arrow.center(), Align2::CENTER_CENTER, "▾", FontId::proportional(11.0), t.text_dim);
        let arrow_resp = arrow_resp.on_hover_text(format!("More {} commands", c.label));
        egui::Popup::menu(&arrow_resp).show(|ui| {
            for id in c.more {
                if ui.button(crate::command_label(id)).clicked() {
                    action = Some(*id);
                }
            }
        });
    }
    action
}

/// The toolbar as the control channel reports it: the tabs, the current tab, and every entry
/// with its status and, on the current tab, its button (screen points, last frame).
pub fn describe(app: &ModelApp) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = catalogue::CATALOGUE
        .iter()
        .map(|c| {
            let rect = app.toolbar_rects.iter().find(|(id, _)| *id == c.id).map(|(_, r)| [r.left(), r.top(), r.width(), r.height()]);
            serde_json::json!({
                "id": c.id,
                "label": c.label,
                "tab": c.tab,
                "panel": c.panel,
                "command": c.command(),
                "more": c.more,
                "coming_soon": c.milestone(),
                "rect": rect,
            })
        })
        .collect();
    serde_json::json!({"tab": app.ui.tab, "tabs": Tab::ALL, "entries": entries})
}
