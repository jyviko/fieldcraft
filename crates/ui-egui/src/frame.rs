//! The one place windows and panel headings are made, so every prompt and panel shares the same
//! width limits and heading bands.
//!
//! Adapted from SolidCraft's `crates/ui-egui/src/frame.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use egui::{Align2, Ui, WidgetText, vec2};

/// Floating windows: prompts and panels, and those holding a table.
pub const WINDOW_MAX_W: f32 = 460.0;
pub const WINDOW_WIDE_MAX_W: f32 = 760.0;

/// How wide a window may get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Normal,
    Wide,
}

/// A floating window, centred on the screen, shrink-wrapped to its content up to its width limit.
pub fn window<'a>(ctx: &egui::Context, title: impl Into<WidgetText>, width: Width) -> egui::Window<'a> {
    let max = match width {
        Width::Normal => WINDOW_MAX_W,
        Width::Wide => WINDOW_WIDE_MAX_W,
    };
    let c = ctx.content_rect().center();
    egui::Window::new(title).collapsible(false).resizable(false).pivot(Align2::CENTER_CENTER).default_pos(c - vec2(0.0, 60.0)).max_width(max)
}

/// Heading levels inside a panel: its title, and the sections below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The panel's title row (`Tokens::heading_bg`).
    Title,
    /// A section heading (`Tokens::subheading_bg`).
    Section,
}

/// Horizontal inner margin of a heading band, on each side.
pub const HEADING_MARGIN: f32 = 4.0;

/// A heading row drawn on a band as wide as the panel; `add` lays out the row left to right.
pub fn heading<R>(ui: &mut Ui, t: &crate::theme::Tokens, level: Level, add: impl FnOnce(&mut Ui) -> R) -> R {
    let fill = match level {
        Level::Title => t.heading_bg,
        Level::Section => t.subheading_bg,
    };
    let inner = (ui.available_width() - 2.0 * HEADING_MARGIN).max(0.0);
    egui::Frame::new()
        .fill(fill)
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(HEADING_MARGIN as i8, 2))
        .show(ui, |ui| {
            ui.set_width(inner);
            ui.horizontal(add).inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    /// Windows are made in this file only, so none can skip the width limits.
    #[test]
    fn windows_come_from_the_frame_helper() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut bad = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "rs") && p.file_name().is_some_and(|n| n != "frame.rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                if text.contains(concat!("Window", "::new(")) {
                    bad.push(p.display().to_string());
                }
            }
        }
        assert!(bad.is_empty(), "use crate::frame::window instead of egui::Window::new in {bad:?}");
    }
}
