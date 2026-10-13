//! The panels around the viewport: menu bar, model tree, commands, status bar and the command
//! parameter form. They read engine state and act through commands.

use egui::RichText;
use modelcraft_render::StandardView;

use crate::theme::Tokens;
use crate::{ModelApp, Style, VERSION};

pub fn menu_bar(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    egui::Panel::top("mc_menu").frame(egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.label(RichText::new("ModelCraft").strong());
            ui.add_space(8.0);
            ui.menu_button("View", |ui| {
                for s in Style::ALL {
                    ui.radio_value(&mut app.ui.style, s, s.label());
                }
                ui.separator();
                ui.checkbox(&mut app.ui.show_nodes, "Nodes");
                ui.checkbox(&mut app.ui.perspective, "Perspective");
                ui.checkbox(&mut app.ui.dark, "Dark theme");
                ui.separator();
                for v in StandardView::ALL {
                    if ui.button(v.label()).clicked() {
                        app.set_view(v);
                    }
                }
                if ui.button("Fit").clicked() {
                    app.fit_view();
                }
            });
            ui.menu_button("Window", |ui| {
                ui.checkbox(&mut app.ui.show_tree, "Model tree");
                ui.checkbox(&mut app.ui.show_commands, "Commands");
            });
            ui.menu_button("Help", |ui| {
                ui.label(format!("ModelCraft {VERSION}"));
                ui.label(RichText::new("Left drag orbits, right or middle drag pans,\nthe wheel zooms, F or a double click fits.").color(t.text_dim));
            });
        });
    });
}

pub fn status_bar(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let (nodes, elements) = (app.session.model.nodes().len(), app.session.model.elements().len());
    egui::Panel::bottom("mc_status").exact_size(24.0).frame(egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin::symmetric(8, 3))).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                match &app.status {
                    Some(s) => {
                        ui.label(RichText::new(&s.text).color(if s.error { t.error } else { t.text }));
                    }
                    None => {
                        ui.label(RichText::new("Ready").color(t.text_dim));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(app.gpu_info.as_deref().unwrap_or("CPU renderer")).color(t.text_dim));
                    ui.separator();
                    ui.label(RichText::new(if app.ui.perspective { "Perspective" } else { "Orthographic" }).color(t.text_dim));
                    ui.separator();
                    ui.label(format!("{nodes} nodes  {elements} elements"));
                });
            });
        },
    );
}

pub fn model_tree(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let model = &app.session.model;
    egui::Panel::left("mc_tree").resizable(true).default_size(220.0).frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::same(8))).show(
        ui,
        |ui| {
            ui.label(RichText::new("Model").strong());
            ui.separator();
            egui::ScrollArea::vertical().id_salt("mc_tree_scroll").show(ui, |ui| {
                ui.label(format!("Nodes ({})", model.nodes().len()));
                egui::CollapsingHeader::new(format!("Elements ({})", model.elements().len())).id_salt("mc_tree_elements").default_open(true).show(
                    ui,
                    |ui| {
                        let counts = model.element_counts();
                        if counts.is_empty() {
                            ui.label(RichText::new("none").color(t.text_dim));
                        }
                        for (kind, n) in counts {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                                ui.painter().rect_filled(r, 2.0, t.element(kind));
                                ui.label(format!("{} ({n})", kind.name()));
                            });
                        }
                    },
                );
                if let Some(d) = &app.session.design {
                    ui.label(format!("Design {} ({} bodies)", d.name, d.bodies.len()));
                }
                if !app.session.surfaces.is_empty() {
                    ui.label(format!("Surfaces ({})", app.session.surfaces.len()));
                }
                ui.label(format!("History ({} commands)", app.session.journal.len()));
            });
        },
    );
}

/// The command registry by area (the id's first part), each command a button that starts it.
pub fn commands(app: &mut ModelApp, ui: &mut egui::Ui) {
    let t = Tokens::get(app.ui.dark);
    let mut start = None;
    egui::Panel::right("mc_commands")
        .resizable(true)
        .default_size(220.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::same(8)))
        .show(ui, |ui| {
            ui.label(RichText::new("Commands").strong());
            ui.separator();
            egui::ScrollArea::vertical().id_salt("mc_commands_scroll").show(ui, |ui| {
                let all = modelcraft_engine::commands();
                let mut areas: Vec<&str> = Vec::new();
                for c in all {
                    if !areas.contains(&area(c.id)) {
                        areas.push(area(c.id));
                    }
                }
                for a in areas {
                    ui.label(RichText::new(title(a)).color(t.text_dim).small());
                    for c in all.iter().filter(|c| area(c.id) == a) {
                        let button = egui::Button::new(c.label).min_size(egui::vec2(ui.available_width(), 0.0));
                        if ui.add(button).on_hover_text(format!("{}\n{}", c.id, c.params)).clicked() {
                            start = Some(c.id);
                        }
                    }
                    ui.add_space(6.0);
                }
            });
        });
    if let Some(id) = start {
        // Errors already show in the status bar.
        let _ = app.start(id);
    }
}

fn area(id: &str) -> &str {
    id.split('.').next().unwrap_or(id)
}

fn title(area: &str) -> String {
    let mut c = area.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The open command's parameters as JSON, with the command's parameter documentation.
pub fn params_dialog(app: &mut ModelApp, ctx: &egui::Context) {
    let t = Tokens::get(app.ui.dark);
    let Some(d) = app.dialog.as_mut() else { return };
    let Some(spec) = modelcraft_engine::find_command(d.command) else {
        app.dialog = None;
        return;
    };
    let (mut run, mut cancel) = (false, false);
    egui::Window::new(spec.label).id(egui::Id::new("mc_params")).collapsible(false).resizable(true).default_width(360.0).show(ctx, |ui| {
        ui.label(RichText::new(spec.id).color(t.text_dim).small());
        ui.label(RichText::new(spec.params).color(t.text_dim));
        ui.add(egui::TextEdit::multiline(&mut d.text).code_editor().desired_rows(4).desired_width(f32::INFINITY));
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).color(t.error));
        }
        ui.horizontal(|ui| {
            run = ui.button("Run").clicked();
            cancel = ui.button("Cancel").clicked();
        });
    });
    if cancel {
        app.dialog = None;
    } else if run {
        let _ = app.run_dialog();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn areas_are_titled() {
        assert_eq!(super::area("mesh.node.add"), "mesh");
        assert_eq!(super::title("mesh"), "Mesh");
        assert_eq!(super::title(""), "");
    }
}
