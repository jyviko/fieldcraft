//! The application bar: the window's title bar. The app's mark and name, the quick actions (File
//! menu, save, undo, redo, Solve, command search, theme, help), the model's document tab and the
//! version; on Windows and Linux the caption buttons ([`crate::titlebar`]).
//!
//! Layout follows SolidCraft's `toolbar::app_bar` (MIT OR Apache-2.0), see ATTRIBUTION.md; the
//! quick-action buttons are craft-ui `IconButton`s and the document tab a craft-ui `Tab`.

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Vec2, pos2, vec2};
use serde_json::json;

use crate::ModelApp;
use crate::icons::{self, Look};
use crate::theme::Tokens;

/// The bar's height.
pub const HEIGHT: f32 = 34.0;
/// A quick-action button's side, and the room each takes.
const BUTTON: f32 = 26.0;
const STRIDE: f32 = 30.0;

/// The quick actions' look: flat on the dark bar, lighter under the pointer.
fn button_style(t: &Tokens) -> IconButtonStyle {
    let normal = ButtonVisuals { fill: Color32::TRANSPARENT, stroke: Stroke::NONE, icon: t.app_bar_text };
    IconButtonStyle {
        normal,
        hovered: ButtonVisuals { fill: Color32::from_white_alpha(30), ..normal },
        pressed: Some(ButtonVisuals { fill: Color32::from_white_alpha(18), ..normal }),
        selected: ButtonVisuals { fill: Color32::from_white_alpha(40), ..normal },
        corner_radius: CornerRadius::same(4),
        focus_stroke: Stroke::new(1.0, t.accent),
    }
}

/// ModelCraft's mark: the application icon's bracket (`assets/app-icon/icon-small.svg`) drawn in
/// code from the SVG's own coordinates, without its tile, each face in the middle of its gradient.
fn mark(p: &egui::Painter, c: egui::Pos2) {
    // The bracket spans y 141–883 in the SVG; draw it 22 points tall around its centre.
    let s = 22.0 / 742.0;
    let at = |x: f32, y: f32| c + vec2((x - 512.0) * s, (y - 512.0) * s);
    let face = |col: Color32, pts: &[(f32, f32)]| {
        p.add(egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| at(x, y)).collect(), col, Stroke::NONE));
    };
    let (top, front, end) = (Color32::from_rgb(0x69, 0xd4, 0xc2), Color32::from_rgb(0x19, 0xa0, 0x8d), Color32::from_rgb(0x0d, 0x66, 0x5d));
    face(top, &[(512.0, 141.0), (833.3, 326.5), (730.48, 385.86), (409.19, 200.36)]);
    face(front, &[(409.19, 200.36), (730.48, 385.86), (730.48, 638.14), (409.19, 452.64)]);
    face(top, &[(409.19, 452.64), (730.48, 638.14), (512.0, 764.28), (190.7, 578.78)]);
    face(front, &[(190.7, 578.78), (512.0, 764.28), (512.0, 883.0), (190.7, 697.5)]);
    face(end, &[(730.48, 385.86), (833.3, 326.5), (833.3, 697.5), (730.48, 638.14)]);
    face(end, &[(730.48, 638.14), (833.3, 697.5), (512.0, 883.0), (512.0, 764.28)]);
    // The bolt hole.
    let hole: Vec<egui::Pos2> = (0..24)
        .map(|k| {
            let a = k as f32 / 24.0 * std::f32::consts::TAU;
            at(460.6 + 86.0 * a.cos(), 608.5 + 49.5 * a.sin())
        })
        .collect();
    p.add(egui::Shape::convex_polygon(hole, Color32::from_rgb(0x08, 0x46, 0x3f), Stroke::NONE));
}

pub fn app_bar(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let style = button_style(&t);
    let mut anchors: Vec<(&'static str, Rect)> = Vec::new();
    egui::Panel::top("mc_appbar").exact_size(HEIGHT).show_separator_line(false).frame(egui::Frame::NONE.fill(t.app_bar)).show(ui, |ui| {
        let r = ui.max_rect();
        app.place("app_bar", r);
        if app.custom_titlebar || app.integrated_titlebar {
            crate::titlebar::drag_area(ui, r);
        }
        let left = r.left() + if app.integrated_titlebar { crate::titlebar::TRAFFIC_LIGHTS } else { 0.0 };
        let compact = r.right() - left < 760.0;
        mark(ui.painter(), pos2(left + 20.0, r.center().y));
        if !compact {
            ui.painter().text(pos2(left + 36.0, r.center().y), Align2::LEFT_CENTER, "ModelCraft", FontId::proportional(14.0), t.app_bar_text);
        }
        let mut x = left + if compact { 44.0 } else { 132.0 };
        let (undo, redo) = app.session.undo_depth();
        let busy = app.busy().is_some();
        let actions: [(&'static str, &'static str, String, bool); 4] = [
            ("file", "file.open", "File: open, save, decks, samples".to_string(), true),
            ("save", "file.save", format!("Save ({})", shortcut("S")), !busy),
            ("undo", "edit.undo", format!("Undo ({})", shortcut("Z")), undo > 0 && !busy),
            ("redo", "edit.redo", format!("Redo ({})", shortcut("Shift+Z")), redo > 0 && !busy),
        ];
        let mut clicked = None;
        for (key, icon, tip, enabled) in actions {
            let rect = Rect::from_center_size(pos2(x + BUTTON / 2.0, r.center().y), Vec2::splat(BUTTON));
            if quick_button(app, ui, &style, key, icon, rect, &tip, enabled, app.ui.menu.as_deref() == Some(key)) {
                clicked = Some(key);
            }
            anchors.push((key, rect));
            x += STRIDE;
        }
        // Solve: the one prominent action, a filled button with its label.
        x += 6.0;
        let solve = Rect::from_min_size(pos2(x, r.center().y - 12.0), vec2(if compact { 30.0 } else { 74.0 }, 24.0));
        let resp = ui.interact(solve, ui.id().with("mc_solve"), Sense::click());
        let can = !app.session.model.steps().is_empty() && !busy;
        let fill = if resp.hovered() && can {
            t.accent.gamma_multiply(1.15)
        } else if can {
            t.accent
        } else {
            Color32::from_white_alpha(24)
        };
        ui.painter().rect_filled(solve, 5.0, fill);
        // A white play mark: the accent fill is the button's colour.
        let c = pos2(solve.left() + 15.0, solve.center().y);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![c + vec2(-3.5, -5.5), c + vec2(5.5, 0.0), c + vec2(-3.5, 5.5)],
            Color32::WHITE,
            Stroke::NONE,
        ));
        if !compact {
            ui.painter().text(pos2(solve.left() + 28.0, solve.center().y), Align2::LEFT_CENTER, "Solve", FontId::proportional(13.0), Color32::WHITE);
        }
        let tip = if app.session.model.steps().is_empty() {
            "Solve: the model has no steps yet (try Help › Samples)"
        } else {
            "Solve every step and show the results"
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Solve"));
        if resp.on_hover_text(tip).clicked() {
            clicked = Some("solve");
        }
        anchors.push(("solve", solve));
        x = solve.right() + 10.0;
        let more: [(&'static str, &'static str, String); 3] = [
            ("search", "ui.search", format!("Search commands (S or {})", shortcut("K"))),
            ("theme", if app.ui.dark { "ui.sun" } else { "ui.moon" }, if app.ui.dark { "Light theme".into() } else { "Dark theme".into() }),
            ("help", "ui.help", "Help: samples, navigation, about".into()),
        ];
        for (key, icon, tip) in more {
            let rect = Rect::from_center_size(pos2(x + BUTTON / 2.0, r.center().y), Vec2::splat(BUTTON));
            let selected = (key == "help" && app.ui.menu.as_deref() == Some("help")) || (key == "search" && app.ui.palette);
            if quick_button(app, ui, &style, key, icon, rect, &tip, true, selected) {
                clicked = Some(key);
            }
            anchors.push((key, rect));
            x += STRIDE;
        }
        // The model's document tab, joined to the ribbon below it.
        let captions = if app.custom_titlebar { crate::titlebar::WIDTH } else { 0.0 };
        let version = format!("v{}", crate::VERSION);
        let version_w = ui.painter().layout_no_wrap(version.clone(), FontId::proportional(11.0), t.text_dim).size().x;
        let tabs_end = r.right() - captions - version_w - 24.0;
        let label = format!("{}{}", app.model_name(), if app.session.is_dirty() { " •" } else { "" });
        let text_w = ui.painter().layout_no_wrap(label.clone(), FontId::proportional(13.0), t.text).size().x;
        let tab_x = (x + 16.0).max(r.center().x - (text_w + 40.0) / 2.0);
        let tab_w = (text_w + 40.0).clamp(120.0, 260.0).min((tabs_end - tab_x).max(0.0));
        if tab_w > 40.0 {
            let tab = Rect::from_min_size(pos2(tab_x, r.top() + 5.0), vec2(tab_w, r.height() - 5.0));
            let path = app.session.path.clone();
            craft_ui::tabs::Tab::new(egui::Id::new("mc_document_tab"), &label, true).show_at(ui, tab, |ui, resp| {
                ui.painter().rect_filled(tab, CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, t.toolbar);
                let clip = tab.shrink2(vec2(10.0, 0.0));
                ui.painter().with_clip_rect(clip).text(
                    pos2(tab.left() + 14.0, tab.center().y),
                    Align2::LEFT_CENTER,
                    &label,
                    FontId::proportional(13.0),
                    t.text,
                );
                let _ = resp.clone().on_hover_text(path.as_deref().unwrap_or("Not saved yet"));
            });
            app.place("document_tab", tab);
        }
        ui.painter().text(
            pos2(r.right() - 12.0 - captions, r.center().y),
            Align2::RIGHT_CENTER,
            version,
            FontId::proportional(11.0),
            Color32::from_rgb(160, 168, 182),
        );
        if app.custom_titlebar {
            crate::titlebar::caption_buttons(app, ui, r);
        }
        match clicked {
            Some("file") => app.toggle_menu("file"),
            Some("help") => app.toggle_menu("help"),
            Some("save") => {
                app.save();
            }
            Some("undo") => {
                let _ = app.run("edit.undo", json!({}));
            }
            Some("redo") => {
                let _ = app.run("edit.redo", json!({}));
            }
            Some("solve") => app.solve_all(),
            Some("search") => {
                app.ui.palette = !app.ui.palette;
                app.ui.menu = None;
            }
            Some("theme") => app.ui.dark = !app.ui.dark,
            _ => {}
        }
        for (key, rect) in &anchors {
            app.place(
                match *key {
                    "file" => "app_bar:file",
                    "save" => "app_bar:save",
                    "undo" => "app_bar:undo",
                    "redo" => "app_bar:redo",
                    "solve" => "app_bar:solve",
                    "search" => "app_bar:search",
                    "theme" => "app_bar:theme",
                    _ => "app_bar:help",
                },
                *rect,
            );
        }
        let anchor = |key: &str| anchors.iter().find(|(k, _)| *k == key).map_or(r, |(_, a)| *a);
        crate::frame::menu(app, ui, "file", anchor("file"), crate::files::file_menu);
        crate::frame::menu(app, ui, "help", anchor("help"), help_menu);
    });
}

/// One quick action: its icon on the dark bar, dimmed when it cannot run now. `true` when clicked.
#[allow(clippy::too_many_arguments)]
fn quick_button(
    app: &mut ModelApp,
    ui: &mut egui::Ui,
    style: &IconButtonStyle,
    key: &str,
    icon: &'static str,
    rect: Rect,
    tip: &str,
    enabled: bool,
    selected: bool,
) -> bool {
    let resp = ui
        .scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(("mc_quick", key)), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                IconButton::new(tip, rect.size(), style).selected(selected).show(ui, |_, _, _| {}).on_hover_text(tip).on_disabled_hover_text(tip)
            })
            .inner
        })
        .inner;
    let tint = if enabled { Color32::WHITE } else { Color32::from_white_alpha(90) };
    icons::paint_tinted(app, ui, icon, rect.shrink(4.0), Look::AppBar, tint);
    enabled && resp.clicked()
}

/// The shortcut text with this platform's command key.
pub fn shortcut(key: &str) -> String {
    format!("{}+{key}", if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" })
}

/// Help: the samples (ready to solve), how to navigate, keys, and the version.
fn help_menu(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    ui.set_max_width(340.0);
    ui.label(egui::RichText::new("SAMPLES").size(11.0).color(t.text_dim));
    let mut open = None;
    crate::files::sample_rows(ui, &t, &mut open);
    ui.separator();
    ui.label(egui::RichText::new("NAVIGATION").size(11.0).color(t.text_dim));
    ui.label("Left drag orbits (or pans or zooms: the navigation bar), right or middle drag pans, the wheel zooms, F or a double click fits.");
    ui.label(format!("S or {} searches every command; the command line at the bottom runs `id {{json}}`.", shortcut("K")));
    ui.separator();
    ui.label(egui::RichText::new(format!("ModelCraft {}", crate::VERSION)).color(t.text_dim));
    if let Some(n) = open {
        app.ui.menu = None;
        app.open_sample(n);
    }
}
