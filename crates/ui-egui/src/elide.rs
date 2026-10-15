//! Text that does not fit: elided in the middle, so both ends of a name stay readable
//! (`F2:side:a5#[F2…]|F2:side:a6`), with the full text left to a hover.

use std::borrow::Cow;

use egui::{FontId, Ui};

/// The ellipsis put in place of the middle.
pub const ELLIPSIS: char = '…';

/// `text`, or as much of its start and end as fits in `max_width` around an [`ELLIPSIS`].
/// `width_of` measures a string as drawn. Ties keep one more character at the start. With no room
/// even for the ellipsis alone, the ellipsis.
pub fn middle<'a>(text: &'a str, max_width: f32, width_of: impl Fn(&str) -> f32) -> Cow<'a, str> {
    if width_of(text) <= max_width {
        return Cow::Borrowed(text);
    }
    let chars: Vec<char> = text.chars().collect();
    let joined = |kept: usize| -> String {
        let head = kept.div_ceil(2);
        let tail = kept - head;
        let mut s: String = chars.iter().take(head).collect();
        s.push(ELLIPSIS);
        s.extend(chars.iter().skip(chars.len() - tail));
        s
    };
    // The widest that fits: the width grows with the characters kept.
    let (mut lo, mut hi) = (0, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width_of(&joined(mid)) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Cow::Owned(joined(lo))
}

/// The width of `text` on one line in `font`.
pub fn width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font.clone(), egui::Color32::PLACEHOLDER).size().x
}

/// [`middle`] measured with `font` as `ui` draws it.
pub fn middle_in<'a>(ui: &Ui, text: &'a str, font: &FontId, max_width: f32) -> Cow<'a, str> {
    middle(text, max_width, |s| width(ui, s, font))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One unit per character.
    fn chars(s: &str) -> f32 {
        s.chars().count() as f32
    }

    #[test]
    fn fitting_text_is_kept_whole() {
        assert!(matches!(middle("Seeds", 5.0, chars), Cow::Borrowed("Seeds")));
    }

    #[test]
    fn long_text_keeps_both_ends() {
        let name = "F2:side:a5#[F2:side:a6]|F2:side:a6";
        let short = middle(name, 15.0, chars);
        assert_eq!(short, "F2:side…side:a6");
        assert!(chars(&short) <= 15.0);
        assert_eq!(middle(name, 16.0, chars), "F2:side:…side:a6");
    }

    #[test]
    fn no_room_leaves_the_ellipsis() {
        assert_eq!(middle("abcdef", 1.0, chars), "…");
        assert_eq!(middle("abcdef", 0.0, chars), "…");
        assert_eq!(middle("", -1.0, chars), "…");
    }

    #[test]
    fn multibyte_text_splits_on_characters() {
        assert_eq!(middle("αβγδεζ", 4.0, chars), "αβ…ζ");
    }
}
