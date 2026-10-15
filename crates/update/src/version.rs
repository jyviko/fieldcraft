//! Release versions: `MAJOR.MINOR.PATCH` with an optional pre-release suffix (`0.6.0-rc.1`),
//! ordered as Semantic Versioning 2.0.0 orders them (§11): a pre-release sorts before its
//! release, and pre-release identifiers compare numerically when both are numbers.

use std::cmp::Ordering;
use std::fmt;

/// The longest version string accepted (a release tag is short; anything longer is hostile).
const MAX_LEN: usize = 64;

/// A release version.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// The pre-release identifiers (`rc.1` is `["rc", "1"]`); empty for a release.
    pub pre: Vec<String>,
}

impl Version {
    /// Parse `0.5.8`, `v0.5.8` (a release tag) or `0.6.0-rc.1`. Build metadata (`+…`) is ignored.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_LEN {
            return None;
        }
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split_once('+').map_or(text, |(v, _)| v);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let mut number = || parts.next().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).and_then(|p| p.parse::<u64>().ok());
        let (major, minor, patch) = (number()?, number()?, number()?);
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => {
                let ids: Vec<String> = pre.split('.').map(str::to_string).collect();
                if ids.iter().any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                    return None;
                }
                ids
            }
        };
        Some(Version { major, minor, patch, pre })
    }

    /// Whether this is a pre-release (`-rc.1`).
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

/// One pre-release identifier against another (SemVer §11.4): numbers numerically and below
/// words, words by their ASCII text.
fn compare_id(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self
                    .pre
                    .iter()
                    .zip(&other.pre)
                    .map(|(a, b)| compare_id(a, b))
                    .find(|o| o.is_ne())
                    .unwrap_or_else(|| self.pre.len().cmp(&other.pre.len())),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_parse_and_print() {
        assert_eq!(v("0.5.8"), Version { major: 0, minor: 5, patch: 8, pre: vec![] });
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert_eq!(v(" 1.2.3+build.7 "), v("1.2.3"));
        assert_eq!(v("0.6.0-rc.1").pre, ["rc", "1"]);
        assert_eq!(v("0.6.0-rc.1").to_string(), "0.6.0-rc.1");
        assert!(v("0.6.0-rc.1").is_prerelease() && !v("0.6.0").is_prerelease());
        for bad in
            ["", "1", "1.2", "1.2.3.4", "1..3", "a.b.c", "1.2.-3", "1.2.3-", "1.2.3-rc..1", "1.2.3-r c", "99999999999999999999.0.0", &"1".repeat(100)]
        {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    /// The order SemVer §11 gives, the example from the specification included.
    #[test]
    fn versions_order_as_semver() {
        let order = [
            "0.5.8",
            "0.5.9",
            "0.5.10",
            "0.6.0-alpha",
            "0.6.0-alpha.1",
            "0.6.0-alpha.beta",
            "0.6.0-beta",
            "0.6.0-beta.2",
            "0.6.0-beta.11",
            "0.6.0-rc.1",
            "0.6.0",
            "1.0.0",
        ];
        for pair in order.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(v("1.0.0").cmp(&v("v1.0.0+x")), Ordering::Equal);
    }
}
