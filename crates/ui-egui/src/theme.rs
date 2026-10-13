//! Colours and widget style: a dark theme (the default) and a light one. Every colour the UI
//! uses comes from [`Tokens`].
//!
//! Adapted from SolidCraft's `crates/ui-egui/src/theme.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use egui::{Color32, Stroke};
use modelcraft_model::ElementKind;
use modelcraft_render::Rgb;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub app_bar: Color32,
    pub panel: Color32,
    pub panel_header: Color32,
    pub border: Color32,
    pub field: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub hover: Color32,
    pub error: Color32,
    pub ok: Color32,
    /// Loads and heat in icons.
    pub warm: Color32,
    // ---- viewport ----
    /// Background gradient, top and bottom.
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
    /// Buttons drawn over the viewport.
    pub overlay: Color32,
    pub overlay_text: Color32,
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    // ---- elements ----
    pub hex: Color32,
    pub tet: Color32,
    pub wedge: Color32,
    pub shell: Color32,
    pub line_element: Color32,
    /// Element edges (the mesh wireframe).
    pub mesh_edge: Color32,
    pub node: Color32,
    // ---- geometry ----
    /// Faces of the open design and imported surfaces.
    pub geometry: Color32,
    /// Their face boundaries.
    pub geometry_edge: Color32,
    // ---- results ----
    /// The contour colour scale, low to high (the same in both themes).
    pub contour: [Color32; 5],
    /// Faces with no result at a node.
    pub no_result: Color32,
}

/// The contour colour scale: blue, cyan, green, yellow, red.
const CONTOUR: [Color32; 5] = [
    Color32::from_rgb(0x2b, 0x4f, 0xd8),
    Color32::from_rgb(0x2f, 0xb8, 0xe0),
    Color32::from_rgb(0x3c, 0xc8, 0x64),
    Color32::from_rgb(0xf0, 0xd2, 0x3c),
    Color32::from_rgb(0xe0, 0x40, 0x2e),
];

impl Tokens {
    pub fn get(dark: bool) -> Tokens {
        if dark { Tokens::dark() } else { Tokens::light() }
    }

    pub const fn dark() -> Tokens {
        Tokens {
            app_bar: Color32::from_rgb(0x23, 0x27, 0x2e),
            panel: Color32::from_rgb(0x2b, 0x30, 0x38),
            panel_header: Color32::from_rgb(0x34, 0x3a, 0x44),
            border: Color32::from_rgb(0x44, 0x4b, 0x57),
            field: Color32::from_rgb(0x1f, 0x23, 0x29),
            text: Color32::from_rgb(0xdd, 0xe2, 0xea),
            text_dim: Color32::from_rgb(0x96, 0x9f, 0xad),
            accent: Color32::from_rgb(0x4f, 0x9c, 0xe8),
            accent_soft: Color32::from_rgb(0x2d, 0x4e, 0x70),
            hover: Color32::from_rgb(0x3c, 0x43, 0x4f),
            error: Color32::from_rgb(0xf0, 0x6c, 0x64),
            ok: Color32::from_rgb(0x8c, 0xc8, 0x8c),
            warm: Color32::from_rgb(0xf0, 0xb4, 0x4a),
            viewport_top: Color32::from_rgb(0x3a, 0x41, 0x4d),
            viewport_bottom: Color32::from_rgb(0x1e, 0x22, 0x28),
            overlay: Color32::from_rgba_unmultiplied_const(0x1c, 0x20, 0x26, 0xd0),
            overlay_text: Color32::from_rgb(0xdd, 0xe2, 0xea),
            axis_x: Color32::from_rgb(0xe0, 0x5a, 0x50),
            axis_y: Color32::from_rgb(0x6c, 0xc0, 0x5a),
            axis_z: Color32::from_rgb(0x52, 0x8c, 0xe6),
            hex: Color32::from_rgb(0x82, 0xa6, 0xc8),
            tet: Color32::from_rgb(0x8c, 0xc0, 0x96),
            wedge: Color32::from_rgb(0xc8, 0xb4, 0x82),
            shell: Color32::from_rgb(0xc8, 0x96, 0x82),
            line_element: Color32::from_rgb(0xf0, 0xc0, 0x50),
            mesh_edge: Color32::from_rgb(0x14, 0x18, 0x1e),
            node: Color32::from_rgb(0xf2, 0xe6, 0x6e),
            geometry: Color32::from_rgb(0xa8, 0xae, 0xb8),
            geometry_edge: Color32::from_rgb(0xe8, 0xec, 0xf2),
            contour: CONTOUR,
            no_result: Color32::from_rgb(0x70, 0x76, 0x80),
        }
    }

    pub const fn light() -> Tokens {
        Tokens {
            app_bar: Color32::from_rgb(0xe4, 0xe8, 0xee),
            panel: Color32::from_rgb(0xf3, 0xf5, 0xf8),
            panel_header: Color32::from_rgb(0xe2, 0xe6, 0xec),
            border: Color32::from_rgb(0xc4, 0xcb, 0xd5),
            field: Color32::from_rgb(0xff, 0xff, 0xff),
            text: Color32::from_rgb(0x1e, 0x23, 0x2a),
            text_dim: Color32::from_rgb(0x5e, 0x67, 0x74),
            accent: Color32::from_rgb(0x1f, 0x6f, 0xc4),
            accent_soft: Color32::from_rgb(0xc6, 0xdc, 0xf4),
            hover: Color32::from_rgb(0xd8, 0xde, 0xe6),
            error: Color32::from_rgb(0xc0, 0x30, 0x28),
            ok: Color32::from_rgb(0x2c, 0x7a, 0x3a),
            warm: Color32::from_rgb(0xc0, 0x7a, 0x14),
            viewport_top: Color32::from_rgb(0xf0, 0xf3, 0xf7),
            viewport_bottom: Color32::from_rgb(0xc8, 0xd0, 0xdc),
            overlay: Color32::from_rgba_unmultiplied_const(0xea, 0xee, 0xf3, 0xe0),
            overlay_text: Color32::from_rgb(0x1e, 0x23, 0x2a),
            axis_x: Color32::from_rgb(0xc8, 0x3c, 0x32),
            axis_y: Color32::from_rgb(0x3c, 0x96, 0x32),
            axis_z: Color32::from_rgb(0x2c, 0x64, 0xc8),
            hex: Color32::from_rgb(0x78, 0x9c, 0xc0),
            tet: Color32::from_rgb(0x80, 0xb4, 0x8a),
            wedge: Color32::from_rgb(0xc0, 0xaa, 0x78),
            shell: Color32::from_rgb(0xc0, 0x8c, 0x78),
            line_element: Color32::from_rgb(0xc8, 0x8c, 0x1e),
            mesh_edge: Color32::from_rgb(0x1e, 0x23, 0x2a),
            node: Color32::from_rgb(0xb4, 0x3c, 0x8c),
            geometry: Color32::from_rgb(0xb4, 0xba, 0xc4),
            geometry_edge: Color32::from_rgb(0x2a, 0x30, 0x3a),
            contour: CONTOUR,
            no_result: Color32::from_rgb(0x9a, 0xa0, 0xaa),
        }
    }

    /// The contour colour scale for the renderer, low to high.
    pub fn contour_stops(&self) -> [Rgb; 5] {
        self.contour.map(rgb)
    }

    /// The colour elements of this kind are drawn in.
    pub fn element(&self, kind: ElementKind) -> Color32 {
        match kind {
            ElementKind::Hex8 | ElementKind::Hex20 => self.hex,
            ElementKind::Tet4 | ElementKind::Tet10 => self.tet,
            ElementKind::Wedge6 => self.wedge,
            ElementKind::Tri3 | ElementKind::Tri6 | ElementKind::Quad4 | ElementKind::Quad8 => self.shell,
            ElementKind::Rod2 | ElementKind::Beam2 => self.line_element,
        }
    }
}

/// A colour for the renderer.
pub fn rgb(c: Color32) -> Rgb {
    Rgb(c.r(), c.g(), c.b())
}

fn visuals(dark: bool) -> egui::Visuals {
    let t = Tokens::get(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.panel_header;
    v.override_text_color = Some(t.text);
    v.widgets.inactive.weak_bg_fill = t.panel_header;
    v.widgets.inactive.bg_fill = t.panel_header;
    v.widgets.active.weak_bg_fill = t.accent_soft;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.open.weak_bg_fill = t.panel_header;
    v.window_corner_radius = egui::CornerRadius::same(6);
    v
}

/// Style the context for the dark or light theme.
pub fn apply(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals(visuals(dark));
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 3.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every element kind has a colour, and edges stand out from the faces they lie on.
    #[test]
    fn element_colours_contrast_with_edges() {
        let luma = |c: Color32| u32::from(c.r()) * 3 + u32::from(c.g()) * 6 + u32::from(c.b());
        for dark in [true, false] {
            let t = Tokens::get(dark);
            for k in ElementKind::ALL {
                assert!(luma(t.element(k)) > luma(t.mesh_edge) + 600, "{k:?} dark={dark}");
            }
        }
    }
}
