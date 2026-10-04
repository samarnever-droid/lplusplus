//! A minimal, correct-enough semver for the resolver.
//!
//! Supports `major.minor.patch` versions and the requirement forms Keel uses:
//! `*`, a bare/caret version (`1` or `^1.2.3`), tilde (`~1.2.3`), exact
//! (`=1.2.3`), and simple comparators (`>`, `>=`, `<`, `<=`).

/// A semantic version (major, minor, patch). Pre-release/build tags are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse `1`, `1.2`, or `1.2.3` (a leading `v` and any `-pre`/`+build` are ignored).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let core = s.split(|c| c == '-' || c == '+').next()?;
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() > 3 {
            return None;
        }
        for p in &parts {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
        }
        let major = parts[0].parse().ok()?;
        let minor = parts.get(1).copied().unwrap_or("0").parse().ok()?;
        let patch = parts.get(2).copied().unwrap_or("0").parse().ok()?;
        Some(Self::new(major, minor, patch))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A version requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Req {
    /// `*` — any version.
    Any,
    /// `^1.2.3` (also the meaning of a bare `1.2.3`).
    Caret(Version),
    /// `~1.2.3`.
    Tilde(Version),
    /// `=1.2.3`.
    Exact(Version),
    Gt(Version),
    Gte(Version),
    Lt(Version),
    Lte(Version),
}

impl Req {
    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim();
        if t.is_empty() || t == "*" {
            return Some(Req::Any);
        }
        let (op, rest) = if let Some(r) = t.strip_prefix(">=") {
            (">=", r)
        } else if let Some(r) = t.strip_prefix("<=") {
            ("<=", r)
        } else if let Some(r) = t.strip_prefix('>') {
            (">", r)
        } else if let Some(r) = t.strip_prefix('<') {
            ("<", r)
        } else if let Some(r) = t.strip_prefix('=') {
            ("=", r)
        } else if let Some(r) = t.strip_prefix('^') {
            ("^", r)
        } else if let Some(r) = t.strip_prefix('~') {
            ("~", r)
        } else {
            ("", t)
        };
        let v = Version::parse(rest)?;
        Some(match op {
            "" => Req::Caret(v),
            ">" => Req::Gt(v),
            ">=" => Req::Gte(v),
            "<" => Req::Lt(v),
            "<=" => Req::Lte(v),
            "=" => Req::Exact(v),
            "^" => Req::Caret(v),
            "~" => Req::Tilde(v),
            _ => return None,
        })
    }

    pub fn matches(&self, v: &Version) -> bool {
        match self {
            Req::Any => true,
            Req::Caret(r) => v >= r && caret_upper(r, v),
            Req::Tilde(r) => v >= r && v.major == r.major && v.minor == r.minor,
            Req::Exact(r) => v == r,
            Req::Gt(r) => v > r,
            Req::Gte(r) => v >= r,
            Req::Lt(r) => v < r,
            Req::Lte(r) => v <= r,
        }
    }
}

/// Caret upper bound: bump the leftmost non-zero component (semver caret rule).
fn caret_upper(r: &Version, v: &Version) -> bool {
    if r.major > 0 {
        v.major == r.major
    } else if r.minor > 0 {
        v.major == 0 && v.minor == r.minor
    } else {
        v.major == 0 && v.minor == 0 && v.patch == r.patch
    }
}

impl std::fmt::Display for Req {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Req::Any => write!(f, "*"),
            Req::Caret(v) => write!(f, "^{v}"),
            Req::Tilde(v) => write!(f, "~{v}"),
            Req::Exact(v) => write!(f, "={v}"),
            Req::Gt(v) => write!(f, ">{v}"),
            Req::Gte(v) => write!(f, ">={v}"),
            Req::Lt(v) => write!(f, "<{v}"),
            Req::Lte(v) => write!(f, "<={v}"),
        }
    }
}
