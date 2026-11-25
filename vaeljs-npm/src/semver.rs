//! Bounded `node-semver`-compatible version and range parser and matcher.
//!
//! Rust's `semver` crate implements Cargo's caret-by-default rules and rejects
//! npm syntax (`1.x`, `1.2.3 - 2.3.4`, `>=1.0.0 <2.0.0`, `||`, `v1.2.3`,
//! dist-tags). This module implements bounded npm SemVer semantics directly:
//!
//! - Exact versions (`1.2.3`, `=1.2.3`, `v1.2.3`)
//! - Primitive comparators (`<`, `<=`, `>`, `>=`, `=`)
//! - Caret (`^`, including `^0.x` and `^0.0.x`) and tilde (`~`) ranges
//! - X-ranges (`*`, `x`, `X`, `1.x`, `1.2.x`, partial `1` and `1.2`)
//! - Hyphen ranges (`1.2.3 - 2.3.4`, `1.2 - 2.3.4`)
//! - Space- or comma-separated AND comparator sets and `||` OR unions
//! - npm prerelease isolation (prerelease versions match a comparator set only
//!   when at least one comparator in that set names a prerelease on the same
//!   `[major, minor, patch]` tuple)
//! - Bounded input length ([`MAX_CONSTRAINT_BYTES`]), OR clauses
//!   ([`MAX_OR_CLAUSES`]), and comparators per clause ([`MAX_COMPARATORS_PER_CLAUSE`])

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Maximum bytes in a version or range constraint string.
pub const MAX_CONSTRAINT_BYTES: usize = 256;
/// Maximum `||` union clauses in one range.
pub const MAX_OR_CLAUSES: usize = 32;
/// Maximum primitive comparators inside one AND clause.
pub const MAX_COMPARATORS_PER_CLAUSE: usize = 16;
/// Maximum bytes in one parsed SemVer version string.
pub const MAX_VERSION_BYTES: usize = 128;

/// Failure returned when parsing an npm SemVer version, range, or dist-tag.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SemverError {
    /// The version string was malformed or exceeded [`MAX_VERSION_BYTES`].
    #[error("invalid npm SemVer version `{version}`: {reason}")]
    InvalidVersion { version: String, reason: String },

    /// The range string was malformed or exceeded structural bounds.
    #[error("invalid npm SemVer range `{range}`: {reason}")]
    InvalidRange { range: String, reason: String },

    /// The constraint exceeded a configured complexity bound.
    #[error("npm SemVer constraint exceeds bound: {reason}")]
    LimitExceeded { reason: String },
}

/// One dot-separated prerelease identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PrereleaseId {
    /// Numeric identifier without leading zeroes (except `"0"` itself).
    Numeric(u64),
    /// Non-numeric ASCII alphanumeric / hyphen identifier.
    Ident(String),
}

impl Ord for PrereleaseId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Ident(_)) => Ordering::Less,
            (Self::Ident(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Ident(a), Self::Ident(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PrereleaseId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for PrereleaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Numeric(n) => write!(f, "{n}"),
            Self::Ident(s) => f.write_str(s),
        }
    }
}

/// A parsed npm SemVer 2.0.0 version (`MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`).
///
/// Build metadata is preserved for display but excluded from equality, hashing,
/// and ordering per SemVer 2.0.0.
#[derive(Clone, Debug)]
pub struct NpmVersion {
    /// Major version component.
    pub major: u64,
    /// Minor version component.
    pub minor: u64,
    /// Patch version component.
    pub patch: u64,
    /// Prerelease identifiers (empty for stable releases).
    pub prerelease: Vec<PrereleaseId>,
    /// Build metadata identifiers.
    pub build: Vec<String>,
}

impl NpmVersion {
    /// Parses a SemVer version string (allowing an optional leading `v`).
    pub fn parse(raw: &str) -> Result<Self, SemverError> {
        if raw.is_empty() || raw.len() > MAX_VERSION_BYTES {
            return Err(SemverError::InvalidVersion {
                version: raw.to_owned(),
                reason: format!("version must be 1..={MAX_VERSION_BYTES} bytes"),
            });
        }
        let s = raw.strip_prefix('v').unwrap_or(raw);
        let (main_and_pre, build) = match s.split_once('+') {
            Some((left, b)) => {
                if b.is_empty() {
                    return Err(SemverError::InvalidVersion {
                        version: raw.to_owned(),
                        reason: "empty build metadata after `+`".to_owned(),
                    });
                }
                let parts: Vec<String> = b
                    .split('.')
                    .map(|p| {
                        if !is_valid_build_id(p) {
                            return Err(SemverError::InvalidVersion {
                                version: raw.to_owned(),
                                reason: format!("invalid build identifier `{p}`"),
                            });
                        }
                        Ok(p.to_owned())
                    })
                    .collect::<Result<_, _>>()?;
                (left, parts)
            }
            None => (s, Vec::new()),
        };

        let (core, prerelease) = match main_and_pre.split_once('-') {
            Some((c, pre)) => (c, parse_prerelease_list(pre, raw)?),
            None => (main_and_pre, Vec::new()),
        };

        let mut nums = core.split('.');
        let major_s = nums.next().unwrap_or("");
        let minor_s = nums.next().ok_or_else(|| SemverError::InvalidVersion {
            version: raw.to_owned(),
            reason: "missing minor version".to_owned(),
        })?;
        let patch_s = nums.next().ok_or_else(|| SemverError::InvalidVersion {
            version: raw.to_owned(),
            reason: "missing patch version".to_owned(),
        })?;
        if nums.next().is_some() {
            return Err(SemverError::InvalidVersion {
                version: raw.to_owned(),
                reason: "too many dot-separated numeric components".to_owned(),
            });
        }

        let major = parse_semver_u64(major_s, raw)?;
        let minor = parse_semver_u64(minor_s, raw)?;
        let patch = parse_semver_u64(patch_s, raw)?;

        Ok(Self {
            major,
            minor,
            patch,
            prerelease,
            build,
        })
    }

    /// Returns `true` if this version carries prerelease identifiers.
    pub fn is_prerelease(&self) -> bool {
        !self.prerelease.is_empty()
    }

    fn cmp_core(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
    }
}

impl PartialEq for NpmVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for NpmVersion {}

impl Hash for NpmVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.major.hash(state);
        self.minor.hash(state);
        self.patch.hash(state);
        self.prerelease.hash(state);
    }
}

impl PartialOrd for NpmVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NpmVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let core = self.cmp_core(other);
        if core != Ordering::Equal {
            return core;
        }
        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.prerelease.iter().zip(other.prerelease.iter()) {
                    let c = a.cmp(b);
                    if c != Ordering::Equal {
                        return c;
                    }
                }
                self.prerelease.len().cmp(&other.prerelease.len())
            }
        }
    }
}

impl fmt::Display for NpmVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.prerelease.is_empty() {
            f.write_str("-")?;
            for (idx, id) in self.prerelease.iter().enumerate() {
                if idx > 0 {
                    f.write_str(".")?;
                }
                write!(f, "{id}")?;
            }
        }
        if !self.build.is_empty() {
            f.write_str("+")?;
            for (idx, id) in self.build.iter().enumerate() {
                if idx > 0 {
                    f.write_str(".")?;
                }
                f.write_str(id)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Exact,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Comparator {
    op: Op,
    version: NpmVersion,
}

impl Comparator {
    fn matches_op(&self, v: &NpmVersion) -> bool {
        match self.op {
            Op::Exact => v == &self.version,
            Op::Lt => v < &self.version,
            Op::Le => v <= &self.version,
            Op::Gt => v > &self.version,
            Op::Ge => v >= &self.version,
        }
    }
}

/// A parsed npm SemVer range (`||` union of AND comparator sets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpmRange {
    raw: String,
    clauses: Vec<Vec<Comparator>>,
}

impl NpmRange {
    /// Parses a bounded npm SemVer range expression.
    ///
    /// An empty string `""` is accepted as `*` (`>=0.0.0`), matching npm's
    /// `package.json` dependency behavior.
    pub fn parse(raw: &str) -> Result<Self, SemverError> {
        if raw.len() > MAX_CONSTRAINT_BYTES {
            return Err(SemverError::LimitExceeded {
                reason: format!("range exceeds {MAX_CONSTRAINT_BYTES} bytes"),
            });
        }
        let or_parts: Vec<&str> = raw.split("||").collect();
        if or_parts.len() > MAX_OR_CLAUSES {
            return Err(SemverError::LimitExceeded {
                reason: format!("range exceeds {MAX_OR_CLAUSES} `||` clauses"),
            });
        }

        let mut clauses = Vec::with_capacity(or_parts.len());
        for clause_raw in or_parts {
            let comps = parse_clause(clause_raw, raw)?;
            if comps.len() > MAX_COMPARATORS_PER_CLAUSE {
                return Err(SemverError::LimitExceeded {
                    reason: format!(
                        "range clause exceeds {MAX_COMPARATORS_PER_CLAUSE} comparators"
                    ),
                });
            }
            clauses.push(comps);
        }

        Ok(Self {
            raw: raw.to_owned(),
            clauses,
        })
    }

    /// Returns the original range text.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Returns `true` if `version` satisfies this range under npm SemVer rules.
    pub fn matches(&self, version: &NpmVersion) -> bool {
        self.clauses
            .iter()
            .any(|clause| clause_matches(clause, version))
    }

    /// Selects the highest matching version from `candidates`.
    pub fn select_highest<'a>(
        &self,
        candidates: impl IntoIterator<Item = &'a NpmVersion>,
    ) -> Option<&'a NpmVersion> {
        candidates.into_iter().filter(|v| self.matches(v)).max()
    }
}

impl fmt::Display for NpmRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// A parsed npm version constraint: either a SemVer range or a dist-tag (`latest`, `next`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionConstraint {
    /// A SemVer version or range expression.
    Range(NpmRange),
    /// A registry dist-tag such as `"latest"` or `"next"`.
    DistTag(String),
}

impl VersionConstraint {
    /// Parses `raw` as an [`NpmRange`] if valid SemVer syntax, or as a valid npm
    /// dist-tag when it cannot be mistaken for a malformed SemVer string.
    pub fn parse(raw: &str) -> Result<Self, SemverError> {
        if raw.is_empty() {
            return Err(SemverError::InvalidRange {
                range: raw.to_owned(),
                reason: "constraint is empty".to_owned(),
            });
        }
        if raw.len() > MAX_CONSTRAINT_BYTES {
            return Err(SemverError::LimitExceeded {
                reason: format!("constraint exceeds {MAX_CONSTRAINT_BYTES} bytes"),
            });
        }
        if let Ok(range) = NpmRange::parse(raw) {
            return Ok(Self::Range(range));
        }
        if is_valid_dist_tag(raw) {
            return Ok(Self::DistTag(raw.to_owned()));
        }
        Err(SemverError::InvalidRange {
            range: raw.to_owned(),
            reason: "not a valid npm SemVer range or dist-tag".to_owned(),
        })
    }
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Range(r) => write!(f, "{r}"),
            Self::DistTag(t) => f.write_str(t),
        }
    }
}

fn is_valid_dist_tag(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > MAX_VERSION_BYTES {
        return false;
    }
    // Reject anything starting with a digit or `v` + digit so malformed versions
    // like `1.2.3.4` or `v1.2.bad` never silently turn into dist-tags.
    let bytes = raw.as_bytes();
    if bytes[0].is_ascii_digit() {
        return false;
    }
    if bytes[0] == b'v' && bytes.get(1).is_some_and(u8::is_ascii_digit) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn clause_matches(clause: &[Comparator], version: &NpmVersion) -> bool {
    if !clause.iter().all(|c| c.matches_op(version)) {
        return false;
    }
    if version.prerelease.is_empty() {
        return true;
    }
    clause.iter().any(|c| {
        !c.version.prerelease.is_empty()
            && c.version.major == version.major
            && c.version.minor == version.minor
            && c.version.patch == version.patch
    })
}

fn parse_clause(clause_raw: &str, full_range: &str) -> Result<Vec<Comparator>, SemverError> {
    let trimmed = clause_raw.trim();
    if trimmed.is_empty() {
        return Ok(vec![Comparator {
            op: Op::Ge,
            version: zero_version(),
        }]);
    }

    // Check hyphen range (`A - B`) where `-` is whitespace-separated.
    if let Some((left, right)) = split_hyphen_range(trimmed) {
        return expand_hyphen_range(left, right, full_range);
    }

    let tokens = tokenize_comparators(trimmed, full_range)?;
    if tokens.is_empty() {
        return Ok(vec![Comparator {
            op: Op::Ge,
            version: zero_version(),
        }]);
    }

    let mut out = Vec::new();
    for tok in tokens {
        expand_comparator(&tok, full_range, &mut out)?;
    }
    Ok(out)
}

fn split_hyphen_range(clause: &str) -> Option<(&str, &str)> {
    let idx = clause.find(" - ")?;
    let left = clause[..idx].trim();
    let right = clause[idx + 3..].trim();
    Some((left, right))
}

fn tokenize_comparators(clause: &str, full_range: &str) -> Result<Vec<String>, SemverError> {
    // Normalize commas to spaces for space-free npm:pkg@>=1.2.0,<2.0.0 ranges,
    // then join separated operators like `>= 1.0.0` into `>=1.0.0`.
    let replaced = clause.replace(',', " ");
    let raw_parts: Vec<&str> = replaced.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw_parts.len() {
        let part = raw_parts[i];
        if matches!(part, "=" | "<" | "<=" | ">" | ">=" | "~" | "^") {
            let next = raw_parts
                .get(i + 1)
                .ok_or_else(|| SemverError::InvalidRange {
                    range: full_range.to_owned(),
                    reason: format!("dangling operator `{part}`"),
                })?;
            out.push(format!("{part}{next}"));
            i += 2;
        } else if part == "-" {
            return Err(SemverError::InvalidRange {
                range: full_range.to_owned(),
                reason: "malformed hyphen range".to_owned(),
            });
        } else {
            out.push(part.to_owned());
            i += 1;
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
struct PartialVersion {
    major: Option<u64>,
    minor: Option<u64>,
    patch: Option<u64>,
    prerelease: Vec<PrereleaseId>,
}

fn parse_partial(raw: &str, full_range: &str) -> Result<PartialVersion, SemverError> {
    let s = raw.strip_prefix('v').unwrap_or(raw);
    if s.is_empty() {
        return Err(SemverError::InvalidRange {
            range: full_range.to_owned(),
            reason: "empty comparator version".to_owned(),
        });
    }
    let (main_and_pre, _build) = match s.split_once('+') {
        Some((left, b)) => {
            if b.is_empty() || !b.split('.').all(is_valid_build_id) {
                return Err(SemverError::InvalidRange {
                    range: full_range.to_owned(),
                    reason: format!("invalid build metadata in `{raw}`"),
                });
            }
            (left, Some(b))
        }
        None => (s, None),
    };
    let (core, prerelease) = match main_and_pre.split_once('-') {
        Some((c, pre)) => (c, parse_prerelease_list(pre, raw)?),
        None => (main_and_pre, Vec::new()),
    };

    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() > 3 || parts.is_empty() {
        return Err(SemverError::InvalidRange {
            range: full_range.to_owned(),
            reason: format!("invalid version `{raw}`"),
        });
    }

    let parse_opt = |s: &str| -> Result<Option<u64>, SemverError> {
        if matches!(s, "*" | "x" | "X") {
            Ok(None)
        } else {
            Ok(Some(parse_semver_u64(s, raw)?))
        }
    };

    let major = parse_opt(parts[0])?;
    let minor = match parts.get(1) {
        Some(p) => parse_opt(p)?,
        None => None,
    };
    let patch = match parts.get(2) {
        Some(p) => parse_opt(p)?,
        None => None,
    };

    // Wildcards cannot precede concrete components (`*.1.0` or `1.*.0`),
    // and prerelease tags are only valid on concrete `M.m.p`.
    if (major.is_none() && (minor.is_some() || patch.is_some()))
        || (minor.is_none() && patch.is_some())
    {
        return Err(SemverError::InvalidRange {
            range: full_range.to_owned(),
            reason: format!("numeric component cannot follow wildcard in `{raw}`"),
        });
    }
    if !prerelease.is_empty() && (major.is_none() || minor.is_none() || patch.is_none()) {
        return Err(SemverError::InvalidRange {
            range: full_range.to_owned(),
            reason: format!("prerelease cannot be attached to wildcard version `{raw}`"),
        });
    }

    Ok(PartialVersion {
        major,
        minor,
        patch,
        prerelease,
    })
}

fn expand_hyphen_range(
    left_raw: &str,
    right_raw: &str,
    full_range: &str,
) -> Result<Vec<Comparator>, SemverError> {
    if left_raw.is_empty() || right_raw.is_empty() || right_raw.contains(" - ") {
        return Err(SemverError::InvalidRange {
            range: full_range.to_owned(),
            reason: "malformed hyphen range".to_owned(),
        });
    }
    let left = parse_partial(left_raw, full_range)?;
    let right = parse_partial(right_raw, full_range)?;
    let mut out = Vec::new();

    match (left.major, left.minor, left.patch) {
        (None, _, _) => {}
        (Some(m), None, None) => out.push(Comparator {
            op: Op::Ge,
            version: ver(m, 0, 0),
        }),
        (Some(m), Some(n), None) => out.push(Comparator {
            op: Op::Ge,
            version: ver(m, n, 0),
        }),
        (Some(m), Some(n), Some(p)) => out.push(Comparator {
            op: Op::Ge,
            version: NpmVersion {
                major: m,
                minor: n,
                patch: p,
                prerelease: left.prerelease,
                build: Vec::new(),
            },
        }),
        _ => unreachable!(),
    }

    match (right.major, right.minor, right.patch) {
        (None, _, _) => {
            if out.is_empty() {
                out.push(Comparator {
                    op: Op::Ge,
                    version: zero_version(),
                });
            }
        }
        (Some(m), None, None) => out.push(Comparator {
            op: Op::Lt,
            version: ver_pre0(next_component(m, full_range)?, 0, 0),
        }),
        (Some(m), Some(n), None) => out.push(Comparator {
            op: Op::Lt,
            version: ver_pre0(m, next_component(n, full_range)?, 0),
        }),
        (Some(m), Some(n), Some(p)) => out.push(Comparator {
            op: Op::Le,
            version: NpmVersion {
                major: m,
                minor: n,
                patch: p,
                prerelease: right.prerelease,
                build: Vec::new(),
            },
        }),
        _ => unreachable!(),
    }
    Ok(out)
}

fn expand_comparator(
    tok: &str,
    full_range: &str,
    out: &mut Vec<Comparator>,
) -> Result<(), SemverError> {
    if let Some(rest) = tok.strip_prefix('^') {
        let p = parse_partial(rest, full_range)?;
        return expand_caret(p, full_range, out);
    }
    if let Some(rest) = tok.strip_prefix('~') {
        let p = parse_partial(rest, full_range)?;
        return expand_tilde(p, full_range, out);
    }
    let (op, rest) = if let Some(r) = tok.strip_prefix(">=") {
        (Some(Op::Ge), r)
    } else if let Some(r) = tok.strip_prefix("<=") {
        (Some(Op::Le), r)
    } else if let Some(r) = tok.strip_prefix('>') {
        (Some(Op::Gt), r)
    } else if let Some(r) = tok.strip_prefix('<') {
        (Some(Op::Lt), r)
    } else if let Some(r) = tok.strip_prefix('=') {
        (Some(Op::Exact), r)
    } else {
        (None, tok)
    };

    let p = parse_partial(rest, full_range)?;
    match (op, p.major, p.minor, p.patch) {
        (None | Some(Op::Exact), None, _, _) => {
            out.push(Comparator {
                op: Op::Ge,
                version: zero_version(),
            });
        }
        (None | Some(Op::Exact), Some(m), None, None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, 0, 0),
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(next_component(m, full_range)?, 0, 0),
            });
        }
        (None | Some(Op::Exact), Some(m), Some(n), None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, n, 0),
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(m, next_component(n, full_range)?, 0),
            });
        }
        (None | Some(Op::Exact), Some(m), Some(n), Some(patch)) => {
            out.push(Comparator {
                op: Op::Exact,
                version: NpmVersion {
                    major: m,
                    minor: n,
                    patch,
                    prerelease: p.prerelease,
                    build: Vec::new(),
                },
            });
        }
        (Some(Op::Gt), None, _, _) => {
            // `>*` matches nothing in node-semver.
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(0, 0, 0),
            });
        }
        (Some(Op::Gt), Some(m), None, None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(next_component(m, full_range)?, 0, 0),
            });
        }
        (Some(Op::Gt), Some(m), Some(n), None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, next_component(n, full_range)?, 0),
            });
        }
        (Some(Op::Ge), None, _, _) => {
            out.push(Comparator {
                op: Op::Ge,
                version: zero_version(),
            });
        }
        (Some(Op::Ge), Some(m), None, None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, 0, 0),
            });
        }
        (Some(Op::Ge), Some(m), Some(n), None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, n, 0),
            });
        }
        (Some(Op::Lt), None, _, _) => {
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(0, 0, 0),
            });
        }
        (Some(Op::Lt), Some(m), None, None) => {
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(m, 0, 0),
            });
        }
        (Some(Op::Lt), Some(m), Some(n), None) => {
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(m, n, 0),
            });
        }
        (Some(Op::Le), None, _, _) => {
            out.push(Comparator {
                op: Op::Ge,
                version: zero_version(),
            });
        }
        (Some(Op::Le), Some(m), None, None) => {
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(next_component(m, full_range)?, 0, 0),
            });
        }
        (Some(Op::Le), Some(m), Some(n), None) => {
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(m, next_component(n, full_range)?, 0),
            });
        }
        (Some(prim_op), Some(m), Some(n), Some(patch)) => {
            out.push(Comparator {
                op: prim_op,
                version: NpmVersion {
                    major: m,
                    minor: n,
                    patch,
                    prerelease: p.prerelease,
                    build: Vec::new(),
                },
            });
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn expand_tilde(
    p: PartialVersion,
    full_range: &str,
    out: &mut Vec<Comparator>,
) -> Result<(), SemverError> {
    match (p.major, p.minor, p.patch) {
        (None, _, _) => out.push(Comparator {
            op: Op::Ge,
            version: zero_version(),
        }),
        (Some(m), None, None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, 0, 0),
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(next_component(m, full_range)?, 0, 0),
            });
        }
        (Some(m), Some(n), None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, n, 0),
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(m, next_component(n, full_range)?, 0),
            });
        }
        (Some(m), Some(n), Some(patch)) => {
            out.push(Comparator {
                op: Op::Ge,
                version: NpmVersion {
                    major: m,
                    minor: n,
                    patch,
                    prerelease: p.prerelease,
                    build: Vec::new(),
                },
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(m, next_component(n, full_range)?, 0),
            });
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn expand_caret(
    p: PartialVersion,
    full_range: &str,
    out: &mut Vec<Comparator>,
) -> Result<(), SemverError> {
    match (p.major, p.minor, p.patch) {
        (None, _, _) => out.push(Comparator {
            op: Op::Ge,
            version: zero_version(),
        }),
        (Some(m), None, None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, 0, 0),
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(next_component(m, full_range)?, 0, 0),
            });
        }
        (Some(0), Some(n), None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(0, n, 0),
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(0, next_component(n, full_range)?, 0),
            });
        }
        (Some(m), Some(n), None) => {
            out.push(Comparator {
                op: Op::Ge,
                version: ver(m, n, 0),
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(next_component(m, full_range)?, 0, 0),
            });
        }
        (Some(0), Some(0), Some(patch)) => {
            out.push(Comparator {
                op: Op::Ge,
                version: NpmVersion {
                    major: 0,
                    minor: 0,
                    patch,
                    prerelease: p.prerelease,
                    build: Vec::new(),
                },
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(0, 0, next_component(patch, full_range)?),
            });
        }
        (Some(0), Some(n), Some(patch)) => {
            out.push(Comparator {
                op: Op::Ge,
                version: NpmVersion {
                    major: 0,
                    minor: n,
                    patch,
                    prerelease: p.prerelease,
                    build: Vec::new(),
                },
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(0, next_component(n, full_range)?, 0),
            });
        }
        (Some(m), Some(n), Some(patch)) => {
            out.push(Comparator {
                op: Op::Ge,
                version: NpmVersion {
                    major: m,
                    minor: n,
                    patch,
                    prerelease: p.prerelease,
                    build: Vec::new(),
                },
            });
            out.push(Comparator {
                op: Op::Lt,
                version: ver_pre0(next_component(m, full_range)?, 0, 0),
            });
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn next_component(val: u64, full_range: &str) -> Result<u64, SemverError> {
    val.checked_add(1).ok_or_else(|| SemverError::InvalidRange {
        range: full_range.to_owned(),
        reason: "SemVer upper bound overflow".to_owned(),
    })
}

fn zero_version() -> NpmVersion {
    ver(0, 0, 0)
}

fn ver(major: u64, minor: u64, patch: u64) -> NpmVersion {
    NpmVersion {
        major,
        minor,
        patch,
        prerelease: Vec::new(),
        build: Vec::new(),
    }
}

fn ver_pre0(major: u64, minor: u64, patch: u64) -> NpmVersion {
    NpmVersion {
        major,
        minor,
        patch,
        prerelease: vec![PrereleaseId::Numeric(0)],
        build: Vec::new(),
    }
}

fn parse_semver_u64(s: &str, context: &str) -> Result<u64, SemverError> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SemverError::InvalidVersion {
            version: context.to_owned(),
            reason: format!("expected numeric SemVer component, got `{s}`"),
        });
    }
    if s.len() > 1 && s.starts_with('0') {
        return Err(SemverError::InvalidVersion {
            version: context.to_owned(),
            reason: format!("leading zero in SemVer component `{s}`"),
        });
    }
    s.parse::<u64>().map_err(|_| SemverError::InvalidVersion {
        version: context.to_owned(),
        reason: format!("numeric overflow in `{s}`"),
    })
}

fn parse_prerelease_list(raw: &str, context: &str) -> Result<Vec<PrereleaseId>, SemverError> {
    if raw.is_empty() {
        return Err(SemverError::InvalidVersion {
            version: context.to_owned(),
            reason: "empty prerelease after `-`".to_owned(),
        });
    }
    let mut out = Vec::new();
    for part in raw.split('.') {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(SemverError::InvalidVersion {
                version: context.to_owned(),
                reason: format!("invalid prerelease identifier `{part}`"),
            });
        }
        if part.bytes().all(|b| b.is_ascii_digit()) {
            if part.len() > 1 && part.starts_with('0') {
                return Err(SemverError::InvalidVersion {
                    version: context.to_owned(),
                    reason: format!("leading zero in numeric prerelease `{part}`"),
                });
            }
            let num = part
                .parse::<u64>()
                .map_err(|_| SemverError::InvalidVersion {
                    version: context.to_owned(),
                    reason: format!("numeric overflow in prerelease `{part}`"),
                })?;
            out.push(PrereleaseId::Numeric(num));
        } else {
            out.push(PrereleaseId::Ident(part.to_owned()));
        }
    }
    Ok(out)
}

fn is_valid_build_id(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(range: &str, version: &str) -> bool {
        let r = NpmRange::parse(range).expect(range);
        let v = NpmVersion::parse(version).expect(version);
        r.matches(&v)
    }

    // Unit test category: critical parser/matcher contract for node-semver compatibility.
    #[test]
    fn exact_caret_tilde_xranges_and_unions() {
        // Exact (unlike Cargo's semver crate, bare 1.2.3 is exact, not ^1.2.3!)
        assert!(matches("1.2.3", "1.2.3"));
        assert!(!matches("1.2.3", "1.2.4"));
        assert!(matches("=v1.2.3", "1.2.3"));

        // Caret semantics including 0.x and 0.0.x
        assert!(matches("^1.2.3", "1.9.9"));
        assert!(!matches("^1.2.3", "2.0.0"));
        assert!(!matches("^1.2.3", "2.0.0-alpha.1"));
        assert!(matches("^0.2.3", "0.2.9"));
        assert!(!matches("^0.2.3", "0.3.0"));
        assert!(matches("^0.0.3", "0.0.3"));
        assert!(!matches("^0.0.3", "0.0.4"));

        // Tilde semantics
        assert!(matches("~1.2.3", "1.2.9"));
        assert!(!matches("~1.2.3", "1.3.0"));
        assert!(matches("~1.2", "1.2.9"));
        assert!(!matches("~1.2", "1.3.0"));
        assert!(matches("~1", "1.9.9"));
        assert!(!matches("~1", "2.0.0"));

        // X-ranges & partials
        assert!(matches("1.x", "1.4.2"));
        assert!(!matches("1.x", "2.0.0"));
        assert!(matches("1.2.X", "1.2.9"));
        assert!(!matches("1.2.X", "1.3.0"));
        assert!(matches("*", "5.0.0"));
        assert!(!matches("*", "5.0.0-alpha.1"));

        // Hyphen ranges & unions
        assert!(matches("1.2.3 - 2.3.4", "1.2.3"));
        assert!(matches("1.2.3 - 2.3.4", "2.3.4"));
        assert!(!matches("1.2.3 - 2.3.4", "2.3.5"));
        assert!(matches("1.2 - 2.3", "2.3.9"));
        assert!(!matches("1.2 - 2.3", "2.4.0"));
        assert!(matches("^1.0.0 || ^3.0.0", "3.2.1"));
        assert!(!matches("^1.0.0 || ^3.0.0", "2.0.0"));
        assert!(matches(">=1.2.0,<2.0.0", "1.5.0"));
        assert!(!matches(">=1.2.0,<2.0.0", "2.0.0"));
    }

    // Unit test category: critical matcher contract for npm prerelease isolation.
    #[test]
    fn enforces_npm_prerelease_isolation() {
        assert!(matches("^1.2.3-beta.2", "1.2.3-beta.3"));
        assert!(matches("^1.2.3-beta.2", "1.2.3"));
        assert!(matches("^1.2.3-beta.2", "1.5.0"));
        assert!(!matches("^1.2.3-beta.2", "1.5.0-alpha.1"));
        assert!(!matches("^1.2.3-beta.2", "2.0.0-alpha.1"));
    }

    // Unit test category: bounds enforcement on comparator and clause counts.
    #[test]
    fn enforces_complexity_bounds_and_dist_tags() {
        let too_many_or = vec!["1.0.0"; MAX_OR_CLAUSES + 1].join(" || ");
        assert!(NpmRange::parse(&too_many_or).is_err());

        let too_many_and = vec![">=1.0.0"; MAX_COMPARATORS_PER_CLAUSE + 1].join(" ");
        assert!(NpmRange::parse(&too_many_and).is_err());

        assert_eq!(
            VersionConstraint::parse("latest").unwrap(),
            VersionConstraint::DistTag("latest".to_owned())
        );
        assert!(VersionConstraint::parse("1.2.3.4").is_err());
        assert!(NpmRange::parse("^18446744073709551615.0.0").is_err());
        assert!(NpmRange::parse("~18446744073709551615").is_err());
    }
}
