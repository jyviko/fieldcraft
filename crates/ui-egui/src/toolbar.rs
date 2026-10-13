//! The toolbar: one tab per stage of the analyst's workflow, each a row of panels of icon buttons,
//! all from the capability catalogue (`modelcraft_engine::catalogue`).
//!
//! When the window is too narrow for labelled buttons, every panel goes icon-only (the label is
//! in the tooltip), then panels collapse from the right into one drop-down button each.
//!
//! A ready entry starts its command (the same as the Commands panel: a command without
//! parameters runs, one with parameters opens its form); its related commands (edit, delete…)
//! are in the drop-down beside it. A planned entry is drawn greyed with its milestone and does
//! nothing; its tooltip says when it comes. Structure follows SolidCraft's
//! `crates/ui-egui/src/toolbar.rs` (MIT OR Apache-2.0), see ATTRIBUTION.md.

use std::sync::Arc;

use egui::{Align2, FontId, Galley, Rect, RichText, Sense, Stroke, vec2};
use modelcraft_engine::catalogue::{self, Capability, Tab};
use serde::Serialize;

use crate::ModelApp;
use crate::theme::Tokens;

/// Icon side (points).
const ICON: f32 = 24.0;
/// Button height with its label, and the width the label wraps at (points).
const BUTTON_H: f32 = 60.0;
const LABEL_W: f32 = 64.0;
/// A button without its label (compact panels).
const COMPACT: f32 = ICON + 12.0;
/// The drop-down strip beside a button with related commands.
const ARROW_W: f32 = 12.0;
/// A collapsed panel: one button that opens the list of its entries.
const COLLAPSED_W: f32 = COMPACT + ARROW_W;
/// Room a panel separator takes.
const PANEL_GAP: f32 = 13.0;
/// Room between buttons of a panel.
const BUTTON_GAP: f32 = 2.0;

/// How a panel is drawn on a toolbar that may be too narrow for every labelled button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Icon and label.
    Full,
    /// Icon only; the label is in the tooltip.
    Compact,
    /// One button that lists the panel's entries.
    Collapsed,
}

/// Fit panels into `room`: every panel in full when they all fit; otherwise every panel compact,
/// then panels wider than a collapsed one collapse from the right until the row fits. Nothing is
/// dropped.
pub fn fit(full: &[f32], compact: &[f32], room: f32) -> Vec<Fit> {
    let total = |widths: &mut dyn Iterator<Item = f32>| -> f32 { widths.map(|w| w + PANEL_GAP).sum() };
    if total(&mut full.iter().copied()) <= room {
        return vec![Fit::Full; full.len()];
    }
    let mut fits = vec![Fit::Compact; compact.len()];
    for i in (0..fits.len()).rev() {
        let width = total(&mut compact.iter().zip(&fits).map(|(w, f)| if *f == Fit::Collapsed { COLLAPSED_W } else { *w }));
        if width <= room {
            break;
        }
        if let (Some(f), Some(w)) = (fits.get_mut(i), compact.get(i))
            && *w > COLLAPSED_W
        {
            *f = Fit::Collapsed;
        }
    }
    fits
}

/// An entry's label laid out under its icon (at most two lines).
fn label(ui: &egui::Ui, t: &Tokens, c: &Capability) -> Arc<Galley> {
    let colour = if c.command().is_some() { t.text } else { t.text_dim };
    let mut job = egui::text::LayoutJob::simple(c.label.to_string(), FontId::proportional(11.0), colour, LABEL_W);
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    ui.painter().layout_job(job)
}

fn arrow_w(c: &Capability) -> f32 {
    if c.command().is_some() && !c.more.is_empty() { ARROW_W } else { 0.0 }
}

fn panel_width(widths: impl Iterator<Item = f32>) -> f32 {
    let (sum, n) = widths.fold((0.0, 0usize), |(s, n), w| (s + w, n + 1));
    sum + BUTTON_GAP * n.saturating_sub(1) as f32
}

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
        let panels: Vec<&'static [Capability]> = catalogue::panels(app.ui.tab).collect();
        let labels: Vec<Vec<Arc<Galley>>> = panels.iter().map(|p| p.iter().map(|c| label(ui, &t, c)).collect()).collect();
        let full: Vec<f32> = panels
            .iter()
            .zip(&labels)
            .map(|(p, l)| panel_width(p.iter().zip(l).map(|(c, g)| g.size().x.max(ICON + 12.0) + 8.0 + arrow_w(c))))
            .collect();
        let compact: Vec<f32> = panels.iter().map(|p| panel_width(p.iter().map(|c| COMPACT + arrow_w(c)))).collect();
        let fits = fit(&full, &compact, ui.available_width());
        ui.horizontal(|ui| {
            for (i, ((entries, galleys), fit)) in panels.iter().zip(labels).zip(&fits).enumerate() {
                let Some(panel) = entries.first().map(|c| c.panel) else { continue };
                if i > 0 {
                    ui.separator();
                }
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = BUTTON_GAP;
                        let clicked = match fit {
                            Fit::Collapsed => collapsed(app, ui, &t, panel, entries),
                            Fit::Full | Fit::Compact => {
                                let mut clicked = None;
                                for (c, galley) in entries.iter().zip(galleys) {
                                    let galley = (*fit == Fit::Full).then_some(galley);
                                    clicked = button(app, ui, &t, c, galley).or(clicked);
                                }
                                clicked
                            }
                        };
                        action = clicked.or(action);
                    });
                    let width = ui.min_rect().width();
                    ui.allocate_ui(vec2(width, 14.0), |ui| {
                        ui.centered_and_justified(|ui| ui.add(egui::Label::new(RichText::new(panel).small().color(t.text_dim)).truncate()));
                    });
                });
            }
        });
    });
    if let Some(id) = action {
        // Errors already show in the status bar.
        let _ = app.start(id);
    }
}

fn icon(app: &mut ModelApp, ui: &egui::Ui, t: &Tokens, c: &'static Capability, rect: Rect) {
    let px = (ICON * ui.ctx().pixels_per_point()).round() as u32;
    let p = ui.painter();
    match app.icons.get(ui.ctx(), c.id, app.ui.dark, c.command().is_some(), px) {
        Some(tex) => p.image(tex.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE),
        None => p.rect_stroke(rect.shrink(3.0), 3.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside),
    };
}

/// One entry's button, with its label when `label` is given; returns the id to start when it
/// (or its drop-down) was clicked.
fn button(app: &mut ModelApp, ui: &mut egui::Ui, t: &Tokens, c: &'static Capability, label: Option<Arc<Galley>>) -> Option<&'static str> {
    let enabled = c.command().is_some();
    let size = match &label {
        Some(g) => vec2(g.size().x.max(ICON + 12.0) + 8.0, BUTTON_H),
        None => vec2(COMPACT, COMPACT),
    };
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    app.toolbar_rects.push((c.id, rect, if label.is_some() { Fit::Full } else { Fit::Compact }));
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let icon_rect = Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 6.0 + ICON / 2.0), vec2(ICON, ICON));
    icon(app, ui, t, c, icon_rect);
    let p = ui.painter();
    if let Some(g) = label {
        p.galley(egui::pos2(rect.center().x, icon_rect.bottom() + 4.0), g, t.text);
    }
    if let Some(m) = c.milestone() {
        let text = p.layout_no_wrap(m.to_string(), FontId::proportional(9.0), t.text);
        let badge = Rect::from_min_size(egui::pos2(rect.right() - text.size().x - 4.0, rect.top()), text.size() + vec2(4.0, 0.0));
        p.rect_filled(badge, 3.0, t.panel_header);
        p.rect_stroke(badge, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        p.galley(badge.min + vec2(2.0, 0.0), text, t.text);
    }
    let resp = resp.on_hover_ui(|ui| {
        ui.label(c.tooltip());
    });
    let mut action = resp.clicked().then_some(c.id);
    if enabled && !c.more.is_empty() {
        let (arrow, arrow_resp) = ui.allocate_exact_size(vec2(ARROW_W, size.y), Sense::click());
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

/// A collapsed panel: its first entry's icon and a drop-down listing every entry (and their
/// related commands), coming-soon ones greyed with their milestone.
fn collapsed(app: &mut ModelApp, ui: &mut egui::Ui, t: &Tokens, panel: &str, entries: &'static [Capability]) -> Option<&'static str> {
    let first = entries.first()?;
    let (rect, resp) = ui.allocate_exact_size(vec2(COLLAPSED_W, COMPACT), Sense::click());
    for c in entries {
        app.toolbar_rects.push((c.id, rect, Fit::Collapsed));
    }
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icon(app, ui, t, first, Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(ICON, ICON)));
    ui.painter().text(egui::pos2(rect.right() - ARROW_W / 2.0, rect.center().y), Align2::CENTER_CENTER, "▾", FontId::proportional(11.0), t.text_dim);
    let resp = resp.on_hover_text(panel);
    let mut action = None;
    egui::Popup::menu(&resp).show(|ui| {
        for c in entries {
            match c.milestone() {
                Some(m) => {
                    ui.add_enabled(false, egui::Button::new(format!("{} — coming soon ({m})", c.label)));
                }
                None => {
                    if ui.button(c.label).on_hover_text(c.tip).clicked() {
                        action = Some(c.id);
                    }
                    for id in c.more {
                        if ui.button(format!("    {}", crate::command_label(id))).clicked() {
                            action = Some(*id);
                        }
                    }
                }
            }
        }
    });
    action
}

/// The toolbar as the control channel reports it: the tabs, the current tab, and every entry
/// with its status and, on the current tab, its button (screen points, last frame).
pub fn describe(app: &ModelApp) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = catalogue::CATALOGUE
        .iter()
        .map(|c| {
            let shown = app.toolbar_rects.iter().find(|(id, ..)| *id == c.id);
            let rect = shown.map(|(_, r, _)| [r.left(), r.top(), r.width(), r.height()]);
            serde_json::json!({
                "id": c.id,
                "label": c.label,
                "tab": c.tab,
                "panel": c.panel,
                "command": c.command(),
                "more": c.more,
                "coming_soon": c.milestone(),
                "rect": rect,
                "fit": shown.map(|(.., f)| f),
            })
        })
        .collect();
    serde_json::json!({"tab": app.ui.tab, "tabs": Tab::ALL, "entries": entries})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_go_compact_then_collapse_from_the_right() {
        let full = [200.0, 300.0, 100.0];
        let compact = [80.0, 120.0, 40.0];
        assert_eq!(fit(&full, &compact, 700.0), vec![Fit::Full; 3]);
        assert_eq!(fit(&full, &compact, 400.0), vec![Fit::Compact; 3]);
        // The last panel is narrower compact than collapsed, so the middle one goes first.
        assert_eq!(fit(&full, &compact, 230.0), vec![Fit::Compact, Fit::Collapsed, Fit::Compact]);
        assert_eq!(fit(&full, &compact, 10.0), vec![Fit::Collapsed, Fit::Collapsed, Fit::Compact], "nothing is dropped");
        assert!(fit(&[], &[], 0.0).is_empty());
    }
}
