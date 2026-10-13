//! Bulk data entries in fixed format, and the numbers in them.
//!
//! An entry is a name and a stream of data fields. In small-field format each line holds the name
//! (or, on a continuation line, blanks) in columns 1–8 and eight data fields of eight columns; in
//! large-field format the name carries a `*`, the data fields are sixteen columns wide, four per
//! line, and each continuation line starts with `*`. Continuations are automatic: the next line
//! continues the entry when its first field is blank (small) or `*` (large), so no continuation
//! markers are written in field 10. A data stream is the same in both formats: field `k` of an
//! entry sits at the same place in the stream whichever width carries it.
//!
//! Entries that hold only integers and keywords use small fields (labels stop at 99 999 999, eight
//! digits). Entries that hold a real number use large fields, because eight columns keep at most
//! seven significant digits. A real is written exactly when its shortest round-trip form fits
//! sixteen columns (every hand-entered value and coordinates on decimal grids); otherwise it is
//! rounded to the nearest value that fits, which keeps at least ten significant digits and, for
//! magnitudes between 1e-5 and 1e9 without a sign, fourteen. Free-field format would not help:
//! readers round free-field reals to the same widths.

use std::fmt::Write as _;

use crate::{Error, Result};

/// Columns of a small field.
const SMALL: usize = 8;
/// Columns of a large field.
const LARGE: usize = 16;

/// One data field.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Field {
    Blank,
    Int(i64),
    Real(f64),
    Text(&'static str),
}

/// A bulk data entry: a name and its data fields in order (fields 2–9 of the first line, then
/// fields 2–9 of each continuation).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Card {
    name: &'static str,
    fields: Vec<Field>,
}

impl Card {
    pub(crate) fn new(name: &'static str) -> Card {
        Card { name, fields: Vec::new() }
    }

    pub(crate) fn int(mut self, v: impl Into<i64>) -> Card {
        self.fields.push(Field::Int(v.into()));
        self
    }

    pub(crate) fn ints<I: Into<i64>>(mut self, v: impl IntoIterator<Item = I>) -> Card {
        self.fields.extend(v.into_iter().map(|i| Field::Int(i.into())));
        self
    }

    pub(crate) fn real(mut self, v: f64) -> Card {
        self.fields.push(Field::Real(v));
        self
    }

    pub(crate) fn reals(mut self, v: impl IntoIterator<Item = f64>) -> Card {
        self.fields.extend(v.into_iter().map(Field::Real));
        self
    }

    pub(crate) fn text(mut self, v: &'static str) -> Card {
        self.fields.push(Field::Text(v));
        self
    }

    pub(crate) fn blank(mut self) -> Card {
        self.fields.push(Field::Blank);
        self
    }

    /// An optional integer: blank when absent.
    pub(crate) fn opt_int(self, v: Option<impl Into<i64>>) -> Card {
        match v {
            Some(v) => self.int(v),
            None => self.blank(),
        }
    }

    /// An optional real: blank when absent.
    pub(crate) fn opt_real(self, v: Option<f64>) -> Card {
        match v {
            Some(v) => self.real(v),
            None => self.blank(),
        }
    }

    /// An optional keyword: blank when absent.
    pub(crate) fn opt_text(self, v: Option<&'static str>) -> Card {
        match v {
            Some(v) => self.text(v),
            None => self.blank(),
        }
    }

    /// Blank fields up to the end of the current line, so the next field starts a continuation.
    pub(crate) fn end_line(mut self) -> Card {
        while !self.fields.len().is_multiple_of(8) {
            self.fields.push(Field::Blank);
        }
        self
    }

    /// Append the entry to `out`. Trailing blank fields are dropped, and so are trailing blanks on
    /// each line. Counts reals that had to be rounded in `rounded`.
    pub(crate) fn write(&self, out: &mut String, rounded: &mut usize) -> Result<()> {
        let large = self.fields.iter().any(|f| matches!(f, Field::Real(_)));
        let (width, per_line) = if large { (LARGE, 4) } else { (SMALL, 8) };
        let used = self.fields.iter().rposition(|f| *f != Field::Blank).map_or(0, |i| i + 1);
        let mut line = if large { format!("{}*", self.name) } else { self.name.to_string() };
        let mut fields = self.fields.iter().take(used).peekable();
        loop {
            pad(&mut line, SMALL);
            for f in fields.by_ref().take(per_line) {
                let text = match *f {
                    Field::Blank => String::new(),
                    Field::Int(v) => v.to_string(),
                    Field::Text(t) => t.to_string(),
                    Field::Real(v) => {
                        let (s, exact) = real(v, width).ok_or_else(|| Error::Limit(format!("{}: {v} is not a finite number", self.name)))?;
                        if !exact {
                            *rounded += 1;
                        }
                        s
                    }
                };
                if text.len() > width {
                    return Err(Error::Limit(format!("{}: `{text}` does not fit a {width}-column field", self.name)));
                }
                let _ = write!(line, "{text:>width$}");
            }
            out.push_str(line.trim_end());
            out.push('\n');
            if fields.peek().is_none() {
                return Ok(());
            }
            line = if large { "*".to_string() } else { String::new() };
        }
    }
}

fn pad(line: &mut String, to: usize) {
    while line.len() < to {
        line.push(' ');
    }
}

/// A finite real in at most `width` columns, and whether it is exact (reads back as the same
/// `f64`). `None` for NaN and infinities.
///
/// Forms, in order of preference: plain decimal with a leading digit (`0.3`, `210000.`) for
/// magnitudes from 1e-5 below 1e16; an exponent with `E` (`7.85E-9`); the exponent without `E` (`7.85-9`);
/// a plain decimal without its leading zero (`.0001`). When none fits the shortest round-trip
/// digits, the value is rounded to fewer significant digits until one does.
pub(crate) fn real(v: f64, width: usize) -> Option<(String, bool)> {
    if !v.is_finite() {
        return None;
    }
    if v == 0.0 {
        return Some(("0.".to_string(), true));
    }
    let (digits, exp) = shortest(v.abs());
    if let Some(s) = fit(v < 0.0, &digits, exp, width) {
        return Some((s, true));
    }
    for keep in (1..digits.len()).rev() {
        let rounded: f64 = format!("{:.*e}", keep - 1, v.abs()).parse().ok()?;
        let (digits, exp) = shortest(rounded);
        if let Some(s) = fit(v < 0.0, &digits, exp, width) {
            return Some((s, false));
        }
    }
    None
}

/// The shortest round-trip significant digits of a positive finite number, and the decimal
/// exponent of the first digit (`v = d.ddd × 10^exp`).
fn shortest(v: f64) -> (String, i32) {
    let s = format!("{v:e}");
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    (mantissa.replace('.', ""), exp.parse().unwrap_or(0))
}

/// The first form of `±d.ddd × 10^exp` that fits `width`.
fn fit(negative: bool, digits: &str, exp: i32, width: usize) -> Option<String> {
    let sign = if negative { "-" } else { "" };
    let (first, rest) = digits.split_at(1.min(digits.len()));
    let plain = |leading_zero: bool| -> String {
        let n = digits.len() as i32;
        if exp >= 0 {
            let int_len = exp + 1;
            if n <= int_len {
                format!("{sign}{digits}{}.", "0".repeat((int_len - n) as usize))
            } else {
                let (a, b) = digits.split_at(int_len as usize);
                format!("{sign}{a}.{b}")
            }
        } else {
            let zero = if leading_zero { "0" } else { "" };
            format!("{sign}{zero}.{}{digits}", "0".repeat((-exp - 1) as usize))
        }
    };
    let exp_sign = if exp < 0 { '-' } else { '+' };
    let candidates = [
        (-5..16).contains(&exp).then(|| plain(true)),
        Some(format!("{sign}{first}.{rest}E{exp_sign}{}", exp.abs())),
        Some(format!("{sign}{first}.{rest}{exp_sign}{}", exp.abs())),
        (exp < 0).then(|| plain(false)),
    ];
    candidates.into_iter().flatten().find(|c| c.len() <= width)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a real the way a Nastran reader does: an exponent may come without `E`.
    fn read(s: &str) -> f64 {
        let s = s.trim();
        let split = s.char_indices().skip(1).find(|(i, c)| (*c == '+' || *c == '-') && !s[..*i].ends_with(['E', 'e']));
        let normal = match split {
            Some((i, _)) => format!("{}E{}", &s[..i], &s[i..]),
            None => s.to_string(),
        };
        normal.parse().unwrap()
    }

    #[test]
    fn reals_take_the_plainest_form_that_fits() {
        let cases = [
            (0.0, "0."),
            (-0.0, "0."),
            (0.3, "0.3"),
            (-1.5, "-1.5"),
            (210000.0, "210000."),
            (7.85e-9, "7.85E-9"),
            (1e-5, "0.00001"),
            (1e20, "1.E+20"),
            (-2.5e-300, "-2.5E-300"),
            (123456789.12345, "123456789.12345"),
            (f64::MAX, "1.7976931348623157E+308"),
        ];
        for (v, want) in cases {
            let (s, exact) = real(v, 32).unwrap();
            assert_eq!(s, want, "{v}");
            assert!(exact);
            assert_eq!(read(&s), v);
        }
        assert_eq!(real(f64::NAN, 16), None);
        assert_eq!(real(f64::INFINITY, 16), None);
        // Tight widths fall back to the E-less exponent, then to fewer digits.
        assert_eq!(real(1.25e-12, 7).unwrap(), ("1.25-12".into(), true));
        assert_eq!(real(1.0 / 3.0, 8).unwrap(), (".3333333".into(), false));
    }

    #[test]
    fn sixteen_columns_round_trip_short_values_and_keep_ten_digits_of_any() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let v = f64::from_bits(next());
            if !v.is_finite() {
                continue;
            }
            let (s, exact) = real(v, LARGE).unwrap();
            assert!(s.len() <= LARGE, "{s}");
            let back = read(&s);
            if exact {
                assert_eq!(back.to_bits(), v.to_bits(), "{v} -> {s}");
            }
            assert!(((back - v) / v).abs() < 1e-9 || v == 0.0, "{v} -> {s}");
            // Coordinates and moduli in everyday ranges keep fourteen digits.
            let w = (next() as f64 / u64::MAX as f64) * 1e9;
            let (s, _) = real(w, LARGE).unwrap();
            assert!(((read(&s) - w) / w).abs() < 1e-13 || w == 0.0, "{w} -> {s}");
        }
        for v in [0.1 + 0.2, 1.0 / 3.0, std::f64::consts::PI * 1e-7, -2.0f64.sqrt() * 1e5] {
            let (s, _) = real(v, LARGE).unwrap();
            assert!(((read(&s) - v) / v).abs() < 1e-13, "{v} -> {s}");
        }
    }

    #[test]
    fn cards_wrap_and_trim() {
        let mut out = String::new();
        let mut rounded = 0;
        Card::new("CHEXA").ints(1..=22u32).write(&mut out, &mut rounded).unwrap();
        Card::new("GRID").int(7).blank().reals([1.0, -2.5, 1.0 / 3.0]).write(&mut out, &mut rounded).unwrap();
        Card::new("PSHELL").int(1).int(2).real(0.5).blank().blank().write(&mut out, &mut rounded).unwrap();
        let want = "\
CHEXA          1       2       3       4       5       6       7       8
               9      10      11      12      13      14      15      16
              17      18      19      20      21      22
GRID*                  7                              1.            -2.5
*       .333333333333333
PSHELL*                1               2             0.5
";
        assert_eq!(out, want);
        assert_eq!(rounded, 1);
        assert!(Card::new("X").int(123_456_789).write(&mut out, &mut rounded).is_err(), "nine digits do not fit");
        assert!(Card::new("X").real(f64::NAN).write(&mut out, &mut rounded).is_err());
    }
}
