//! Mouse navigation presets: which mouse button, with which keys held, orbits, pans or zooms.
//!
//! The presets after FieldCraft's own follow SolidCraft's `crates/ui-egui/src/prefs.rs`
//! (`NavPreset`, `NavBinding`, `NAV_PRESETS`; MIT OR Apache-2.0), see ATTRIBUTION.md. In every
//! preset a left drag the preset does not bind does what the navigation bar's mode says (orbit
//! by default), and a Ctrl (Cmd) + left drag that starts on empty space orbits, unless the
//! preset binds Ctrl + left itself (SolidCraft #471).

use crate::NavMode;

/// A mouse button that can drag the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavButton {
    Left,
    Middle,
    Right,
}

/// Modifier keys a binding needs held (others may be held too).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NavKeys {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl NavKeys {
    const NONE: NavKeys = NavKeys { shift: false, ctrl: false, alt: false };
    const SHIFT: NavKeys = NavKeys { shift: true, ctrl: false, alt: false };
    const CTRL: NavKeys = NavKeys { shift: false, ctrl: true, alt: false };
    const SHIFT_ALT: NavKeys = NavKeys { shift: true, ctrl: false, alt: true };

    /// All of these keys are held (Ctrl is Cmd on macOS).
    fn held(self, m: egui::Modifiers) -> bool {
        (!self.shift || m.shift) && (!self.ctrl || m.ctrl || m.command) && (!self.alt || m.alt)
    }
}

/// One camera binding: dragging `button` with `keys` held does `mode`.
#[derive(Clone, Copy, Debug)]
pub struct NavBinding {
    pub button: NavButton,
    pub keys: NavKeys,
    pub mode: NavMode,
}

const fn bind(button: NavButton, keys: NavKeys, mode: NavMode) -> NavBinding {
    NavBinding { button, keys, mode }
}

/// A mouse mapping: its bindings, most specific first (the first one whose button is dragged
/// with all its keys held wins).
pub struct NavPreset {
    pub id: &'static str,
    pub label: &'static str,
    /// What the mouse does, for the settings menu.
    pub help: &'static str,
    /// Choosing the preset turns the wheel the other way.
    pub reverse_zoom: bool,
    pub bindings: &'static [NavBinding],
}

impl NavPreset {
    /// What dragging `button` with `mods` held does, if the preset binds it.
    pub fn mode(&self, button: NavButton, mods: egui::Modifiers) -> Option<NavMode> {
        self.bindings.iter().find(|b| b.button == button && b.keys.held(mods)).map(|b| b.mode)
    }
}

use NavButton::{Left, Middle, Right};
use NavMode::{Orbit, Pan, Zoom};

/// The id of the default preset.
pub const DEFAULT_PRESET: &str = "fieldcraft";

/// The mouse mappings. The first is the default.
pub const NAV_PRESETS: [NavPreset; 7] = [
    NavPreset {
        id: DEFAULT_PRESET,
        label: "FieldCraft",
        help: "Left drag does the navigation bar's mode (orbit, pan or zoom); right, middle or Shift+left drag pans",
        reverse_zoom: false,
        bindings: &[bind(Left, NavKeys::SHIFT, Pan), bind(Right, NavKeys::NONE, Pan), bind(Middle, NavKeys::NONE, Pan)],
    },
    NavPreset {
        id: "fusion",
        label: "Fusion",
        help: "Middle drag pans, Shift+middle orbits, right drag orbits",
        reverse_zoom: false,
        bindings: &[bind(Middle, NavKeys::SHIFT, Orbit), bind(Middle, NavKeys::NONE, Pan), bind(Right, NavKeys::NONE, Orbit)],
    },
    NavPreset {
        id: "solidworks",
        label: "SolidWorks",
        help: "Middle drag orbits, Ctrl+middle pans, Shift+middle zooms, right drag orbits",
        reverse_zoom: true,
        bindings: &[
            bind(Middle, NavKeys::CTRL, Pan),
            bind(Middle, NavKeys::SHIFT, Zoom),
            bind(Middle, NavKeys::NONE, Orbit),
            bind(Right, NavKeys::NONE, Orbit),
        ],
    },
    NavPreset {
        id: "inventor",
        label: "Inventor",
        help: "Middle drag pans, Shift+middle orbits, Ctrl+middle zooms, right drag orbits",
        reverse_zoom: false,
        bindings: &[
            bind(Middle, NavKeys::SHIFT, Orbit),
            bind(Middle, NavKeys::CTRL, Zoom),
            bind(Middle, NavKeys::NONE, Pan),
            bind(Right, NavKeys::NONE, Orbit),
        ],
    },
    NavPreset {
        id: "alias",
        label: "Alias",
        help: "Shift+Alt+left drag orbits, Shift+Alt+middle pans, Shift+Alt+right zooms; middle drag also pans and right drag orbits",
        reverse_zoom: false,
        bindings: &[
            bind(Left, NavKeys::SHIFT_ALT, Orbit),
            bind(Middle, NavKeys::SHIFT_ALT, Pan),
            bind(Right, NavKeys::SHIFT_ALT, Zoom),
            bind(Middle, NavKeys::CTRL, Zoom),
            bind(Middle, NavKeys::NONE, Pan),
            bind(Right, NavKeys::NONE, Orbit),
        ],
    },
    NavPreset {
        id: "tinkercad",
        label: "Tinkercad",
        help: "Right drag orbits, middle or Shift+right drag pans",
        reverse_zoom: false,
        bindings: &[bind(Right, NavKeys::SHIFT, Pan), bind(Right, NavKeys::NONE, Orbit), bind(Middle, NavKeys::NONE, Pan)],
    },
    NavPreset {
        id: "openscad",
        label: "OpenSCAD",
        help: "Left drag orbits; right or middle drag pans, Shift+right or Shift+middle drag zooms",
        reverse_zoom: false,
        bindings: &[
            bind(Left, NavKeys::NONE, Orbit),
            bind(Right, NavKeys::SHIFT, Zoom),
            bind(Middle, NavKeys::SHIFT, Zoom),
            bind(Right, NavKeys::NONE, Pan),
            bind(Middle, NavKeys::NONE, Pan),
        ],
    },
];

/// The preset with this id (the default for an unknown one).
pub fn preset(id: &str) -> &'static NavPreset {
    NAV_PRESETS.iter().find(|p| p.id == id).unwrap_or(&NAV_PRESETS[0])
}

/// What a drag of `button` with `mods` held does under `preset`: the preset's binding; else, for
/// the left button, an orbit when only Ctrl (Cmd) is held and the drag starts off the model,
/// nothing in box selection (the drag draws the box), and the navigation bar's mode `bar`
/// otherwise. `None`: the drag does not move the camera.
pub fn drag_mode(preset: &NavPreset, button: NavButton, mods: egui::Modifiers, bar: NavMode, boxing: bool, on_model: bool) -> Option<NavMode> {
    if let Some(m) = preset.mode(button, mods) {
        return Some(m);
    }
    if button != Left {
        return None;
    }
    let ctrl_only = NavKeys::CTRL.held(mods) && !mods.shift && !mods.alt;
    if ctrl_only && !on_model {
        Some(Orbit)
    } else if boxing {
        None
    } else {
        Some(bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;

    const SHIFT: Modifiers = Modifiers::SHIFT;
    const CTRL: Modifiers = Modifiers::CTRL;
    const NONE: Modifiers = Modifiers::NONE;

    /// Each preset maps its buttons and keys as its help says.
    #[test]
    fn presets_bind_as_listed() {
        let shift_alt = Modifiers { shift: true, alt: true, ..NONE };
        let cases: [(&str, &[(NavButton, Modifiers, Option<NavMode>)]); 7] = [
            ("fieldcraft", &[(Left, SHIFT, Some(Pan)), (Right, NONE, Some(Pan)), (Middle, NONE, Some(Pan)), (Left, NONE, None)]),
            ("fusion", &[(Middle, NONE, Some(Pan)), (Middle, SHIFT, Some(Orbit)), (Right, NONE, Some(Orbit)), (Left, NONE, None)]),
            ("solidworks", &[(Middle, NONE, Some(Orbit)), (Middle, CTRL, Some(Pan)), (Middle, SHIFT, Some(Zoom)), (Right, NONE, Some(Orbit))]),
            ("inventor", &[(Middle, NONE, Some(Pan)), (Middle, SHIFT, Some(Orbit)), (Middle, CTRL, Some(Zoom)), (Right, NONE, Some(Orbit))]),
            ("alias", &[(Left, shift_alt, Some(Orbit)), (Middle, shift_alt, Some(Pan)), (Right, shift_alt, Some(Zoom)), (Middle, NONE, Some(Pan))]),
            ("tinkercad", &[(Right, NONE, Some(Orbit)), (Right, SHIFT, Some(Pan)), (Middle, NONE, Some(Pan)), (Left, NONE, None)]),
            ("openscad", &[(Left, NONE, Some(Orbit)), (Right, NONE, Some(Pan)), (Right, SHIFT, Some(Zoom)), (Middle, SHIFT, Some(Zoom))]),
        ];
        for (id, binds) in cases {
            let p = preset(id);
            assert_eq!(p.id, id);
            for &(button, mods, want) in binds {
                assert_eq!(p.mode(button, mods), want, "{id}: {button:?} {mods:?}");
            }
        }
        assert_eq!(preset("no such preset").id, DEFAULT_PRESET);
        assert!(preset("solidworks").reverse_zoom);
    }

    /// An unbound left drag does the bar's mode; Ctrl+left off the model orbits in every preset
    /// (in box selection too); box selection keeps the plain left drag for the box.
    #[test]
    fn left_drag_follows_the_bar_and_ctrl_orbits_from_empty_space() {
        for p in &NAV_PRESETS {
            assert_eq!(drag_mode(p, Left, CTRL, Pan, true, false), Some(Orbit), "{}", p.id);
            assert_eq!(drag_mode(p, Left, CTRL, Pan, false, true), Some(p.mode(Left, CTRL).unwrap_or(Pan)), "{}", p.id);
        }
        let fc = preset(DEFAULT_PRESET);
        assert_eq!(drag_mode(fc, Left, NONE, Zoom, false, true), Some(Zoom));
        assert_eq!(drag_mode(fc, Left, NONE, Orbit, true, false), None);
        assert_eq!(drag_mode(fc, Left, SHIFT, Orbit, false, false), Some(Pan));
        assert_eq!(drag_mode(preset("fusion"), Right, CTRL, Orbit, false, false), Some(Orbit));
    }
}
