//! Colours and widget style: the Craft family's dark theme (the default, blue-grey chrome around a
//! #2e3440 viewport) and its light one, with the same values as SolidCraft's tokens so the apps
//! look alike side by side. Every colour the UI uses comes from [`Tokens`].
//!
//! Adapted from SolidCraft's `crates/ui-egui/src/theme.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md. Widget states are coloured with craft-ui's `WidgetPalette`.

use egui::{Color32, Stroke};
use modelcraft_model::ElementKind;
use modelcraft_render::Rgb;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// The title bar (dark in both themes) and its text.
    pub app_bar: Color32,
    pub app_bar_text: Color32,
    /// The ribbon.
    pub toolbar: Color32,
    pub panel: Color32,
    pub panel_header: Color32,
    /// Command forms and the context panel.
    pub dialog_bg: Color32,
    /// Band behind a panel's heading and, lighter, behind its section headings.
    pub heading_bg: Color32,
    pub subheading_bg: Color32,
    pub border: Color32,
    pub field: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub hover: Color32,
    pub error: Color32,
    pub ok: Color32,
    /// Warnings, loads and heat in icons.
    pub warm: Color32,
    /// Icon line work and fills.
    pub icon: Color32,
    pub icon_fill: Color32,
    /// The journal timeline: its strip, its items and the current-position marker.
    pub timeline: Color32,
    pub timeline_item: Color32,
    pub timeline_marker: Color32,
    // ---- viewport ----
    /// Background gradient, top and bottom (equal for a flat fill).
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
    /// Chips drawn over the viewport (navigation bar, legend, start screen).
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
    /// Faces of the open design and imported surfaces (SolidCraft's body colour).
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

    pub const fn light() -> Tokens {
        Tokens {
            app_bar: Color32::from_rgb(52, 56, 64),
            app_bar_text: Color32::from_rgb(230, 234, 242),
            toolbar: Color32::from_rgb(246, 247, 249),
            panel: Color32::from_rgb(250, 251, 252),
            panel_header: Color32::from_rgb(236, 238, 242),
            dialog_bg: Color32::from_rgb(250, 251, 252),
            heading_bg: Color32::from_rgb(218, 223, 231),
            subheading_bg: Color32::from_rgb(234, 237, 242),
            border: Color32::from_rgb(208, 212, 220),
            field: Color32::WHITE,
            text: Color32::from_rgb(38, 42, 50),
            text_dim: Color32::from_rgb(110, 116, 128),
            accent: Color32::from_rgb(38, 120, 218),
            accent_soft: Color32::from_rgb(214, 230, 250),
            hover: Color32::from_rgb(226, 234, 246),
            error: Color32::from_rgb(214, 54, 54),
            ok: Color32::from_rgb(44, 122, 58),
            warm: Color32::from_rgb(222, 150, 30),
            icon: Color32::from_rgb(70, 78, 92),
            icon_fill: Color32::from_rgb(178, 196, 222),
            timeline: Color32::from_rgb(232, 235, 240),
            timeline_item: Color32::WHITE,
            timeline_marker: Color32::from_rgb(70, 76, 88),
            viewport_top: Color32::from_rgb(240, 243, 247),
            viewport_bottom: Color32::from_rgb(203, 210, 221),
            overlay: Color32::from_rgba_unmultiplied_const(255, 255, 255, 235),
            overlay_text: Color32::from_rgb(38, 42, 50),
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
            geometry: Color32::from_rgb(176, 186, 198),
            geometry_edge: Color32::from_rgb(40, 44, 52),
            contour: CONTOUR,
            no_result: Color32::from_rgb(0x9a, 0xa0, 0xaa),
        }
    }

    /// Dark blue-grey: the viewport #2e3440, the chrome a little darker, light text, a blue
    /// accent.
    pub const fn dark() -> Tokens {
        let mut t = Tokens::light();
        t.app_bar = Color32::from_rgb(28, 31, 38);
        t.toolbar = Color32::from_rgb(37, 41, 49);
        t.panel = Color32::from_rgb(39, 43, 52);
        t.panel_header = Color32::from_rgb(47, 52, 62);
        t.dialog_bg = Color32::from_rgb(59, 68, 83);
        t.heading_bg = Color32::from_rgb(38, 44, 55);
        t.subheading_bg = Color32::from_rgb(48, 55, 68);
        t.border = Color32::from_rgb(63, 69, 82);
        t.field = Color32::from_rgb(30, 34, 41);
        t.text = Color32::from_rgb(222, 226, 232);
        t.text_dim = Color32::from_rgb(148, 155, 168);
        t.accent = Color32::from_rgb(58, 136, 238);
        t.accent_soft = Color32::from_rgb(42, 70, 110);
        t.hover = Color32::from_rgb(54, 61, 74);
        t.error = Color32::from_rgb(240, 100, 96);
        t.ok = Color32::from_rgb(0x8c, 0xc8, 0x8c);
        t.warm = Color32::from_rgb(236, 170, 70);
        t.icon = Color32::from_rgb(208, 214, 226);
        t.icon_fill = Color32::from_rgb(86, 112, 150);
        t.timeline = Color32::from_rgb(33, 37, 45);
        t.timeline_item = Color32::from_rgb(52, 58, 70);
        t.timeline_marker = Color32::from_rgb(170, 178, 192);
        t.viewport_top = Color32::from_rgb(46, 52, 64);
        t.viewport_bottom = Color32::from_rgb(46, 52, 64);
        t.overlay = Color32::from_rgba_unmultiplied_const(34, 38, 46, 240);
        t.overlay_text = Color32::from_rgb(222, 226, 232);
        t.axis_x = Color32::from_rgb(0xe0, 0x5a, 0x50);
        t.axis_y = Color32::from_rgb(0x6c, 0xc0, 0x5a);
        t.axis_z = Color32::from_rgb(0x52, 0x8c, 0xe6);
        t.hex = Color32::from_rgb(0x82, 0xa6, 0xc8);
        t.tet = Color32::from_rgb(0x8c, 0xc0, 0x96);
        t.wedge = Color32::from_rgb(0xc8, 0xb4, 0x82);
        t.shell = Color32::from_rgb(0xc8, 0x96, 0x82);
        t.line_element = Color32::from_rgb(0xf0, 0xc0, 0x50);
        t.mesh_edge = Color32::from_rgb(0x14, 0x18, 0x1e);
        t.node = Color32::from_rgb(0xf2, 0xe6, 0x6e);
        t.geometry = Color32::from_rgb(152, 156, 162);
        t.geometry_edge = Color32::from_rgb(16, 16, 18);
        t.no_result = Color32::from_rgb(0x70, 0x76, 0x80);
        t
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
            ElementKind::Rod2 | ElementKind::Rod3 | ElementKind::Beam2 => self.line_element,
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
    craft_ui::theme::WidgetPalette {
        noninteractive: v.widgets.noninteractive.bg_fill,
        inactive: t.panel_header,
        hovered: t.hover,
        active: t.accent_soft,
        open: t.panel_header,
    }
    .apply_to(&mut v.widgets, egui::CornerRadius::same(3), Stroke::new(1.0, t.text));
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = egui::CornerRadius::same(6);
    v.menu_corner_radius = egui::CornerRadius::same(6);
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

    /// Title and section bands stand out from a panel, the title more than its sections.
    #[test]
    fn heading_bands_are_darker_than_panels_in_order() {
        let luma = |c: Color32| u32::from(c.r()) * 3 + u32::from(c.g()) * 6 + u32::from(c.b());
        for t in [Tokens::light(), Tokens::dark()] {
            assert!(luma(t.heading_bg) < luma(t.subheading_bg), "{t:?}");
            assert!(luma(t.subheading_bg) < luma(t.dialog_bg), "{t:?}");
        }
    }

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
