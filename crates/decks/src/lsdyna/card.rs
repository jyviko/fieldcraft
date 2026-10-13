//! Cards in LS-DYNA's long keyword format.
//!
//! The deck is written in long format (`*KEYWORD long=y`): every numeric field is 20 columns
//! wide, on the same number of lines as the standard format. Standard format gives most reals
//! 10 columns (16 for node coordinates), which holds only about 5 to 9 significant digits; 20
//! columns hold the shortest decimal that reads back to the same `f64` for every value with up to
//! 17 significant digits and a magnitude from 0.01 to 1e17 (a leading `0` before the point is
//! dropped when that makes it fit: `-.01234…`), and for every value with up to 13 significant
//! digits at any exponent. A value that does not fit (17 digits and a large negative exponent,
//! say) is rounded to the most significant digits that fit, never fewer than
//! 13, and counted, so the writer reports how many values did not round-trip exactly.
//!
//! Fields are right-aligned in their columns; trailing blank fields are left out, so a card ends
//! at its last given value (blank fields take the keyword's defaults).

use std::fmt::Write as _;

/// Columns per field in long format.
pub(crate) const WIDTH: usize = 20;

/// One field of a card.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Field {
    Int(i64),
    Real(f64),
    /// Left empty: the keyword's default.
    Blank,
}

impl From<u32> for Field {
    fn from(v: u32) -> Field {
        Field::Int(i64::from(v))
    }
}

impl From<i32> for Field {
    fn from(v: i32) -> Field {
        Field::Int(i64::from(v))
    }
}

impl From<f64> for Field {
    fn from(v: f64) -> Field {
        Field::Real(v)
    }
}

/// A real as the shortest decimal that fits a field, and whether it reads back exactly.
pub(crate) fn real(v: f64) -> (String, bool) {
    if v == 0.0 {
        return ("0.0".into(), true);
    }
    let positional = with_point(format!("{v}"));
    let scientific = match format!("{v:e}").split_once('e') {
        Some((m, e)) => format!("{}e{e}", with_point(m.to_string())),
        None => positional.clone(),
    };
    // Without the leading zero (`-.0123`), which every Fortran reader takes, when that is needed.
    let bare = positional.replacen("0.", ".", 1);
    let bare = if v.abs() < 1.0 { bare } else { positional.clone() };
    // Positional reads best for everyday magnitudes; otherwise the shortest form.
    let everyday = (1e-4..1e15).contains(&v.abs());
    // Candidates in order of preference among equals: everyday magnitudes positional first,
    // others the shortest, scientific before positional; the bare form only when it is needed.
    let candidates = if everyday { [positional, bare, scientific] } else { [scientific, positional, bare] };
    let shortest = candidates
        .into_iter()
        .enumerate()
        .filter(|(_, s)| s.len() <= WIDTH)
        .min_by_key(|(i, s)| if everyday { (0, *i) } else { (s.len(), *i) })
        .map(|(_, s)| s)
        .unwrap_or_default();
    if !shortest.is_empty() {
        return (shortest, true);
    }
    // Round to the most significant digits that fit (13 always do: sign, 13 digits, the point
    // and a four-character exponent make 20).
    for digits in (1..=16usize).rev() {
        let s = trim_mantissa(&format!("{v:.*e}", digits - 1));
        if s.len() <= WIDTH {
            let exact = s.parse::<f64>().is_ok_and(|r| r == v);
            return (s, exact);
        }
    }
    (format!("{v:.0e}"), false)
}

/// Add `.0` to a number written without a decimal point (Fortran reads both, but an explicit
/// point keeps every real a real).
fn with_point(s: String) -> String {
    if s.contains('.') { s } else { format!("{s}.0") }
}

/// `1.2300e5` → `1.23e5` (keeping one digit after the point).
fn trim_mantissa(s: &str) -> String {
    match s.split_once('e') {
        Some((m, e)) => {
            let m = if m.contains('.') { m.trim_end_matches('0') } else { m };
            let m = if m.ends_with('.') { format!("{m}0") } else { m.to_string() };
            format!("{m}e{e}")
        }
        None => s.to_string(),
    }
}

/// A keyword deck being written.
#[derive(Debug, Default)]
pub(crate) struct Deck {
    pub(crate) text: String,
    /// Reals that did not fit their field exactly and were rounded.
    pub(crate) rounded: usize,
}

impl Deck {
    pub(crate) fn keyword(&mut self, keyword: &str) {
        self.text.push_str(keyword);
        self.text.push('\n');
    }

    /// A comment line (`$` in column 1).
    pub(crate) fn comment(&mut self, text: &str) {
        self.text.push('$');
        if !text.is_empty() {
            self.text.push(' ');
            self.text.push_str(text);
        }
        self.text.push('\n');
    }

    /// A comment naming the fields of the cards that follow, each over its column.
    pub(crate) fn header(&mut self, names: &[&str]) {
        let mut line = String::from("$");
        for (i, name) in names.iter().enumerate() {
            let width = if i == 0 { WIDTH - 1 } else { WIDTH };
            let _ = write!(line, "{name:>width$}");
        }
        self.text.push_str(line.trim_end());
        self.text.push('\n');
    }

    /// A card of fields.
    pub(crate) fn card(&mut self, fields: &[Field]) {
        let mut line = String::new();
        for f in fields {
            match f {
                Field::Int(v) => {
                    let _ = write!(line, "{v:>WIDTH$}");
                }
                Field::Real(v) => {
                    let (s, exact) = real(*v);
                    if !exact {
                        self.rounded += 1;
                    }
                    let _ = write!(line, "{s:>WIDTH$}");
                }
                Field::Blank => line.push_str(&" ".repeat(WIDTH)),
            }
        }
        self.text.push_str(line.trim_end());
        self.text.push('\n');
    }

    /// A text card (a title or heading), written as given.
    pub(crate) fn text(&mut self, text: &str) {
        self.text.push_str(text);
        self.text.push('\n');
    }

    /// Cards of up to eight integers each (set member lists).
    pub(crate) fn list(&mut self, values: &[u32]) {
        for chunk in values.chunks(8) {
            let fields: Vec<Field> = chunk.iter().map(|v| Field::from(*v)).collect();
            self.card(&fields);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_are_the_shortest_exact_form_that_fits() {
        for (v, s) in [
            (0.0, "0.0"),
            (-0.0, "0.0"),
            (1.0, "1.0"),
            (-2.5, "-2.5"),
            (210000.0, "210000.0"),
            (7.85e-9, "7.85e-9"),
            (1e21, "1.0e21"),
            (0.1 + 0.2, "0.30000000000000004"),
            (-9810.0, "-9810.0"),
            (6.02214076e23, "6.02214076e23"),
            (-1.5e-300, "-1.5e-300"),
        ] {
            let (got, exact) = real(v);
            assert_eq!(got, s, "{v:e}");
            assert!(exact && got.len() <= WIDTH, "{v:e}");
            assert_eq!(got.parse::<f64>().unwrap(), if v == 0.0 { 0.0 } else { v });
        }
    }

    #[test]
    fn values_that_cannot_fit_are_rounded_to_at_least_13_digits() {
        let mut count = 0;
        let mut x = 0.123_456_789_012_345_67_f64;
        for i in 0..2000 {
            // Wide exponent range, full 17-digit mantissas, both signs.
            let v = if i % 2 == 0 { x } else { -x } * 10f64.powi((i % 600) - 300);
            let (s, exact) = real(v);
            assert!(s.len() <= WIDTH, "{s}");
            let back: f64 = s.parse().unwrap();
            if exact {
                assert_eq!(back, v);
            } else {
                count += 1;
                assert!(((back - v) / v).abs() < 5e-13, "{v:e} -> {s}");
            }
            x = (x * 7.000_000_000_000_1).fract() + 0.1;
        }
        assert!(count > 0, "the sample includes values that need rounding");
        // Model coordinates (|x| <= 1e9) with up to 15 significant digits always fit.
        for v in [123_456_789.123_456_7_f64, -0.000_123_456_789_012_3, 1e-9, -987_654_321.0] {
            assert!(real(v).1, "{v:e}");
        }
    }

    #[test]
    fn cards_are_right_aligned_and_trimmed() {
        let mut d = Deck::default();
        d.card(&[Field::from(1u32), Field::Blank, Field::from(2.5), Field::Blank]);
        assert_eq!(d.text, format!("{:>20}{:20}{:>20}\n", 1, "", "2.5"));
        d.list(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(d.text.lines().count(), 3);
        assert_eq!(d.rounded, 0);
    }
}
