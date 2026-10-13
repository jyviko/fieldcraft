//! Toolbar icons: original artwork drawn with VectorCraft (`assets/icons`, `cargo xtask icons`),
//! embedded at build time and rasterised with resvg.
//!
//! Icons are drawn in four palette roles, each in one fixed colour (the dark theme's). Before
//! rasterising, every role colour is replaced by the theme token it stands for, so the same SVG
//! serves the dark and light themes and the greyed "coming soon" state. Textures are cached per
//! icon, theme, state and pixel size.

use std::collections::HashMap;

use egui::{Color32, TextureHandle};
use resvg::{tiny_skia, usvg};

use crate::theme::Tokens;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// The palette roles' colours in the SVG files. Keep in step with `PALETTE` in
/// `xtask/src/icons.rs`.
pub const PALETTE: [&str; 4] = ["#dde2ea", "#4f9ce8", "#2d4e70", "#f0b44a"];

/// Largest icon side rasterised (pixels).
const MAX_PX: u32 = 256;

/// The icon's SVG text.
pub fn svg(id: &str) -> Option<&'static str> {
    ICONS.binary_search_by(|(k, _)| (*k).cmp(id)).ok().and_then(|i| ICONS.get(i)).map(|(_, s)| *s)
}

/// The colours the palette roles (line, accent, soft, warm) take in a theme, greyed when the
/// entry is not available yet.
pub fn colours(t: &Tokens, enabled: bool) -> [Color32; 4] {
    if enabled { [t.text, t.accent, t.accent_soft, t.warm] } else { [t.text_dim, t.text_dim, t.panel_header, t.text_dim] }
}

/// The SVG with each palette colour replaced by its role's colour.
pub fn recolour(svg: &str, colours: [Color32; 4]) -> String {
    let mut out = svg.to_string();
    for (from, to) in PALETTE.iter().zip(colours) {
        let hex = format!("#{:02x}{:02x}{:02x}", to.r(), to.g(), to.b());
        out = out.replace(&format!("\"{from}\""), &format!("\"{hex}\""));
    }
    out
}

/// Rasterise an SVG to a `px` × `px` image.
pub fn rasterise(svg: &str, px: u32) -> Result<egui::ColorImage, String> {
    let px = px.clamp(1, MAX_PX);
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px).ok_or("empty icon")?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height()).max(f32::EPSILON);
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

/// Icon textures by (id, enabled) for one theme and pixel size; `None` when the icon is missing
/// or does not rasterise, so it is not tried again every frame. A change of theme or size
/// (zoom) drops the old textures.
#[derive(Default)]
pub struct IconCache {
    theme_px: Option<(bool, u32)>,
    textures: HashMap<(&'static str, bool), Option<TextureHandle>>,
}

impl IconCache {
    pub fn get(&mut self, ctx: &egui::Context, id: &'static str, dark: bool, enabled: bool, px: u32) -> Option<TextureHandle> {
        if self.theme_px != Some((dark, px)) {
            self.textures.clear();
            self.theme_px = Some((dark, px));
        }
        self.textures
            .entry((id, enabled))
            .or_insert_with(|| {
                let svg = svg(id)?;
                let image = rasterise(&recolour(svg, colours(&Tokens::get(dark), enabled)), px).map_err(|e| log::warn!("icon {id}: {e}")).ok()?;
                Some(ctx.load_texture(format!("icon:{id}:{dark}:{enabled}:{px}"), image, egui::TextureOptions::LINEAR))
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelcraft_engine::catalogue::CATALOGUE;

    #[test]
    fn every_catalogue_entry_has_an_icon_and_every_icon_an_entry() {
        for c in CATALOGUE {
            assert!(svg(c.id).is_some(), "no icon assets/icons/{}.svg", c.id);
        }
        for (id, _) in ICONS {
            assert!(CATALOGUE.iter().any(|c| c.id == *id), "assets/icons/{id}.svg belongs to no catalogue entry");
        }
        assert!(ICONS.windows(2).all(|w| w[0].0 < w[1].0), "sorted for the binary search");
    }

    /// Icons use only the palette, so recolouring reaches every colour, and they rasterise.
    #[test]
    fn icons_use_only_palette_colours_and_rasterise() {
        for (id, text) in ICONS {
            for part in text.split('"').skip(1).step_by(2) {
                if part.starts_with('#') {
                    assert!(PALETTE.contains(&part), "{id}: colour {part} is not in the palette");
                }
            }
            let image = rasterise(text, 24).unwrap();
            assert!(image.pixels.iter().any(|p| p.a() > 0), "{id} draws nothing");
        }
    }

    /// The SVG colours are the dark theme's own, so the dark theme draws icons as authored.
    #[test]
    fn the_palette_is_the_dark_themes_colours() {
        let hex: Vec<String> = colours(&Tokens::dark(), true).iter().map(|c| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())).collect();
        assert_eq!(hex, PALETTE);
    }

    #[test]
    fn recolouring_follows_the_theme_and_greys_planned_entries() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0 L24 24" stroke="#4f9ce8" fill="#2d4e70"/></svg>"##;
        let light = Tokens::light();
        let out = recolour(svg, colours(&light, true));
        assert!(out.contains("stroke=\"#1f6fc4\"") && out.contains("fill=\"#c6dcf4\""), "{out}");
        let grey = recolour(svg, colours(&light, false));
        let dim = light.text_dim;
        assert!(grey.contains(&format!("stroke=\"#{:02x}{:02x}{:02x}\"", dim.r(), dim.g(), dim.b())), "{grey}");
        assert!(rasterise("not svg", 24).is_err());
        assert_eq!(rasterise(svg, 100_000).unwrap().size, [MAX_PX as usize; 2]);
    }
}
