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

/// The title bar's quick actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quick {
    File,
    Save,
    Undo,
    Redo,
    Solve,
    Search,
    Theme,
    Help,
}

impl Quick {
    /// Its region in `ui.inspect`'s layout.
    fn region(self) -> &'static str {
        match self {
            Quick::File => "app_bar:file",
            Quick::Save => "app_bar:save",
            Quick::Undo => "app_bar:undo",
            Quick::Redo => "app_bar:redo",
            Quick::Solve => "app_bar:solve",
            Quick::Search => "app_bar:search",
            Quick::Theme => "app_bar:theme",
            Quick::Help => "app_bar:help",
        }
    }
}

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

/// Where the title bar's parts go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarLayout {
    /// The mark's centre, and where the app's name starts (none when compact).
    pub mark: egui::Pos2,
    pub name: Option<egui::Pos2>,
    /// File, Save, Undo, Redo: after the mark, at the left.
    pub quick: [Rect; 4],
    /// Solve, then Search, Theme and Help: at the right end, before the version.
    pub solve: Rect,
    pub tools: [Rect; 3],
    /// The document tab's room, between the two groups.
    pub tab_room: (f32, f32),
    /// The version's right edge.
    pub version_right: f32,
    /// Too narrow for labels: no name, and Solve without its word.
    pub compact: bool,
}

/// The title bar's layout in `bar`. `traffic_lights`: the macOS window buttons sit over its left
/// end (an integrated title bar out of full screen, where there are none). `captions`: the width
/// of the custom caption buttons at its right end; `version_w`: the version text's width.
pub fn bar_layout(bar: Rect, traffic_lights: bool, captions: f32, version_w: f32) -> BarLayout {
    let left = bar.left() + if traffic_lights { crate::titlebar::TRAFFIC_LIGHTS } else { 0.0 };
    let compact = bar.right() - left < 760.0;
    let cy = bar.center().y;
    let button = |x: f32| Rect::from_center_size(pos2(x + BUTTON / 2.0, cy), Vec2::splat(BUTTON));
    let x0 = left + if compact { 44.0 } else { 132.0 };
    let quick = [0.0, 1.0, 2.0, 3.0].map(|i| button(x0 + i * STRIDE));
    let version_right = bar.right() - 12.0 - captions;
    let tools_end = version_right - version_w - 14.0;
    let tools = [3.0, 2.0, 1.0].map(|i| button(tools_end - i * STRIDE));
    let solve_w = if compact { 30.0 } else { 74.0 };
    let solve = Rect::from_min_size(pos2(tools[0].left() - 10.0 - solve_w, cy - 12.0), vec2(solve_w, 24.0));
    BarLayout {
        mark: pos2(left + 20.0, cy),
        name: (!compact).then(|| pos2(left + 36.0, cy)),
        quick,
        solve,
        tools,
        tab_room: (quick[3].right() + 16.0, solve.left() - 16.0),
        version_right,
        compact,
    }
}

/// Whether the window is in full screen (no traffic lights to make room for).
fn fullscreen(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
}

pub fn app_bar(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let style = button_style(&t);
    let mut anchors: Vec<(Quick, Rect)> = Vec::new();
    egui::Panel::top("mc_appbar").exact_size(HEIGHT).show_separator_line(false).frame(egui::Frame::NONE.fill(t.app_bar)).show(ui, |ui| {
        let r = ui.max_rect();
        app.place("app_bar", r);
        if app.custom_titlebar || app.integrated_titlebar {
            crate::titlebar::drag_area(ui, r);
        }
        let captions = if app.custom_titlebar { crate::titlebar::WIDTH } else { 0.0 };
        let version = format!("v{}", crate::VERSION);
        let version_w = ui.painter().layout_no_wrap(version.clone(), FontId::proportional(11.0), t.text_dim).size().x;
        let lay = bar_layout(r, app.integrated_titlebar && !fullscreen(ui.ctx()), captions, version_w);
        mark(ui.painter(), lay.mark);
        if let Some(at) = lay.name {
            ui.painter().text(at, Align2::LEFT_CENTER, "ModelCraft", FontId::proportional(14.0), t.app_bar_text);
        }
        let (undo, redo) = app.session.undo_depth();
        let busy = app.busy().is_some();
        let file_open = app.ui.menu.as_deref() == Some("file");
        let actions: [(Quick, &'static str, String, bool, bool); 4] = [
            (Quick::File, "file.open", "File: open, save, decks, samples".to_string(), true, file_open),
            (Quick::Save, "file.save", format!("Save ({})", shortcut("S")), !busy, false),
            (Quick::Undo, "edit.undo", format!("Undo ({})", shortcut("Z")), undo > 0 && !busy, false),
            (Quick::Redo, "edit.redo", format!("Redo ({})", shortcut("Shift+Z")), redo > 0 && !busy, false),
        ];
        let tools: [(Quick, &'static str, String, bool, bool); 3] = [
            (Quick::Search, "ui.search", format!("Search commands (S or {})", shortcut("K")), true, app.ui.palette),
            (
                Quick::Theme,
                if app.ui.dark { "ui.sun" } else { "ui.moon" },
                if app.ui.dark { "Light theme".into() } else { "Dark theme".into() },
                true,
                false,
            ),
            (Quick::Help, "ui.help", "Help: samples, navigation, about".into(), true, app.ui.menu.as_deref() == Some("help")),
        ];
        let mut clicked = None;
        for ((q, icon, tip, enabled, selected), rect) in actions.into_iter().zip(lay.quick).chain(tools.into_iter().zip(lay.tools)) {
            if quick_button(app, ui, &style, q.region(), icon, rect, &tip, enabled, selected) {
                clicked = Some(q);
            }
            anchors.push((q, rect));
        }
        // Solve: the one prominent action, a filled button with its label.
        let solve = lay.solve;
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
        if !lay.compact {
            ui.painter().text(pos2(solve.left() + 28.0, solve.center().y), Align2::LEFT_CENTER, "Solve", FontId::proportional(13.0), Color32::WHITE);
        }
        let tip = if app.session.model.steps().is_empty() {
            "Solve: the model has no steps yet (try Help › Samples)"
        } else {
            "Solve every step and show the results"
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Solve"));
        if resp.on_hover_text(tip).clicked() {
            clicked = Some(Quick::Solve);
        }
        anchors.push((Quick::Solve, solve));
        // The model's document tab, joined to the ribbon below it, centred in its room.
        let label = format!("{}{}", app.model_name(), if app.session.is_dirty() { " •" } else { "" });
        let text_w = ui.painter().layout_no_wrap(label.clone(), FontId::proportional(13.0), t.text).size().x;
        let (room_l, room_r) = lay.tab_room;
        let tab_w = (text_w + 40.0).clamp(120.0, 260.0).min((room_r - room_l).max(0.0));
        let tab_x = (r.center().x - tab_w / 2.0).clamp(room_l, (room_r - tab_w).max(room_l));
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
            pos2(lay.version_right, r.center().y),
            Align2::RIGHT_CENTER,
            version,
            FontId::proportional(11.0),
            Color32::from_rgb(160, 168, 182),
        );
        if app.custom_titlebar {
            crate::titlebar::caption_buttons(app, ui, r);
        }
        match clicked {
            Some(Quick::File) => app.toggle_menu("file"),
            Some(Quick::Help) => app.toggle_menu("help"),
            Some(Quick::Save) => {
                app.save();
            }
            Some(Quick::Undo) => {
                let _ = app.run("edit.undo", json!({}));
            }
            Some(Quick::Redo) => {
                let _ = app.run("edit.redo", json!({}));
            }
            Some(Quick::Solve) => app.solve_all(),
            Some(Quick::Search) => {
                app.ui.palette = !app.ui.palette;
                app.ui.menu = None;
            }
            Some(Quick::Theme) => app.ui.dark = !app.ui.dark,
            None => {}
        }
        for (q, rect) in &anchors {
            app.place(q.region(), *rect);
        }
        let anchor = |q: Quick| anchors.iter().find(|(k, _)| *k == q).map_or(r, |(_, a)| *a);
        crate::frame::menu(app, ui, "file", anchor(Quick::File), crate::files::file_menu);
        crate::frame::menu(app, ui, "help", anchor(Quick::Help), help_menu);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Solve and the tools sit at the right end; the left group starts after the traffic lights,
    /// at the very left in full screen (where there are none).
    #[test]
    fn the_title_bar_keeps_solve_right_and_drops_the_traffic_light_inset_in_full_screen() {
        let bar = Rect::from_min_size(egui::Pos2::ZERO, vec2(1400.0, HEIGHT));
        let windowed = bar_layout(bar, true, 0.0, 30.0);
        let full = bar_layout(bar, false, 0.0, 30.0);
        assert_eq!(windowed.mark.x - full.mark.x, crate::titlebar::TRAFFIC_LIGHTS);
        assert!(full.mark.x < 30.0 && full.quick[0].left() < 150.0, "{full:?}");
        for l in [windowed, full] {
            // Left to right: mark, quick actions, (tab), Solve, search, theme, help, version.
            assert!(l.quick.windows(2).all(|w| w[0].right() <= w[1].left()));
            assert!(l.quick[3].right() < l.tab_room.0 && l.tab_room.1 < l.solve.left());
            assert!(l.solve.right() <= l.tools[0].left() && l.tools.windows(2).all(|w| w[0].right() <= w[1].left()));
            assert!(l.tools[2].right() < l.version_right - 30.0 && l.version_right <= 1400.0);
            assert!(l.solve.left() > 1100.0, "Solve is at the right end: {:?}", l.solve);
        }
        // The right group does not move with full screen; custom caption buttons push it left.
        assert_eq!(windowed.solve, full.solve);
        let captions = bar_layout(bar, false, crate::titlebar::WIDTH, 30.0);
        assert!((full.tools[2].right() - captions.tools[2].right() - crate::titlebar::WIDTH).abs() < 0.01);
        // Narrow: compact, still in order.
        let narrow = bar_layout(Rect::from_min_size(egui::Pos2::ZERO, vec2(700.0, HEIGHT)), true, 0.0, 30.0);
        assert!(narrow.compact && narrow.name.is_none() && narrow.quick[3].right() < narrow.solve.left());
    }
}
