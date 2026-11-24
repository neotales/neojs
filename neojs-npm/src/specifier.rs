//! Strict `npm:` specifier parsing, package name validation, and canonical
//! peer-context-isolated virtual URL helpers.
//!
//! Runtime JavaScript and TypeScript imports use `npm:[@scope/]name[@constraint][/subpath]`.
//! Resolved packages never materialize as `node_modules` directories; instead,
//! each resolved module receives an internal virtual URL of the form:
//!
//! ```text
//! npm://<registry_host>/<name>/<version>/<ctx>/<pkg_rel_path>
//! npm://<registry_host>/@<scope>/<name>/<version>/<ctx>/<pkg_rel_path>
//! ```

use std::fmt;

use url::Url;

use crate::semver::VersionConstraint;

/// Maximum bytes accepted in one `npm:` specifier.
pub const MAX_SPECIFIER_BYTES: usize = 2 * 1024;
/// Maximum bytes in a validated npm package name (including `@scope/`).
pub const MAX_PACKAGE_NAME_BYTES: usize = 214;
/// Maximum bytes in one version or range constraint.
pub const MAX_CONSTRAINT_BYTES: usize = 256;
/// Maximum bytes in one subpath.
pub const MAX_SUBPATH_BYTES: usize = 1024;

/// Failure returned when parsing or validating an `npm:` specifier, package
/// name, bare package specifier, or virtual `npm://` URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SpecifierError {
    /// The input was empty or exceeded a structural length bound.
    #[error("invalid npm specifier: {reason}")]
    InvalidFormat { reason: String },

    /// The package name violated npm naming rules.
    #[error("invalid npm package name: {reason}")]
    InvalidPackageName { reason: String },

    /// The version constraint after `@` was empty or invalid.
    #[error("invalid npm version constraint `{constraint}`: {reason}")]
    InvalidConstraint { constraint: String, reason: String },

    /// The subpath contained traversal, forbidden characters, or `node_modules`.
    #[error("invalid npm subpath `{subpath}`: {reason}")]
    InvalidSubpath { subpath: String, reason: String },

    /// The virtual `npm://` URL was malformed or failed structural checks.
    #[error("invalid virtual npm URL: {reason}")]
    InvalidVirtualUrl { reason: String },
}

/// A validated npm package name (`name` or `@scope/name`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageName {
    raw: String,
    slash_index: Option<usize>,
}

impl PackageName {
    /// Parses and validates an npm package name.
    ///
    /// Accepts lowercase ASCII letters, digits, `-`, `_`, and `.`, starting
    /// with a lowercase ASCII letter or digit, up to [`MAX_PACKAGE_NAME_BYTES`].
    /// Scoped names must have the form `@scope/name`.
    pub fn parse(raw: &str) -> Result<Self, SpecifierError> {
        if raw.is_empty() {
            return Err(SpecifierError::InvalidPackageName {
                reason: "package name is empty".to_owned(),
            });
        }
        if raw.len() > MAX_PACKAGE_NAME_BYTES {
            return Err(SpecifierError::InvalidPackageName {
                reason: format!("package name exceeds {MAX_PACKAGE_NAME_BYTES} bytes"),
            });
        }

        if let Some(after_at) = raw.strip_prefix('@') {
            let Some((scope, name)) = after_at.split_once('/') else {
                return Err(SpecifierError::InvalidPackageName {
                    reason: "scoped package name must be `@scope/name`".to_owned(),
                });
            };
            if name.contains('/') {
                return Err(SpecifierError::InvalidPackageName {
                    reason: "scoped package name may contain only one `/`".to_owned(),
                });
            }
            validate_name_part(scope, "scope")?;
            validate_name_part(name, "name")?;
            return Ok(Self {
                raw: raw.to_owned(),
                slash_index: Some(1 + scope.len()),
            });
        }

        if raw.contains('/') || raw.contains('@') {
            return Err(SpecifierError::InvalidPackageName {
                reason: "unscoped package name may not contain `/` or `@`".to_owned(),
            });
        }
        validate_name_part(raw, "name")?;
        Ok(Self {
            raw: raw.to_owned(),
            slash_index: None,
        })
    }

    /// Returns the full package name (including `@scope/` when scoped).
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Returns the scope without leading `@`, if this is a scoped package.
    pub fn scope(&self) -> Option<&str> {
        self.slash_index.map(|idx| &self.raw[1..idx])
    }

    /// Returns the unscoped package name segment.
    pub fn unscoped_name(&self) -> &str {
        match self.slash_index {
            Some(idx) => &self.raw[idx + 1..],
            None => &self.raw,
        }
    }

    /// Returns `true` if this package is scoped (`@scope/name`).
    pub fn is_scoped(&self) -> bool {
        self.slash_index.is_some()
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// A parsed runtime `npm:[@scope/]name[@constraint][/subpath]` specifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpmSpecifier {
    name: PackageName,
    constraint: Option<String>,
    subpath: Option<String>,
}

impl NpmSpecifier {
    /// Parses and validates an `npm:` import specifier.
    ///
    /// Rejects `npm://...` authority URLs, whitespace, control characters,
    /// `%`, `\`, `?`, `#`, empty `@` constraints, credentials, invalid package
    /// names, and subpath traversal or `node_modules` segments.
    pub fn parse(raw: &str) -> Result<Self, SpecifierError> {
        if raw.is_empty() {
            return Err(SpecifierError::InvalidFormat {
                reason: "specifier is empty".to_owned(),
            });
        }
        if raw.len() > MAX_SPECIFIER_BYTES {
            return Err(SpecifierError::InvalidFormat {
                reason: format!("specifier exceeds {MAX_SPECIFIER_BYTES}-byte limit"),
            });
        }
        if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(SpecifierError::InvalidFormat {
                reason: "specifier contains whitespace or control characters".to_owned(),
            });
        }
        if raw.contains('%') || raw.contains('\\') || raw.contains('?') || raw.contains('#') {
            return Err(SpecifierError::InvalidFormat {
                reason: "specifier contains forbidden character (`%`, `\\`, `?`, or `#`)"
                    .to_owned(),
            });
        }
        let Some(rest) = raw.strip_prefix("npm:") else {
            return Err(SpecifierError::InvalidFormat {
                reason: "specifier must start with `npm:`".to_owned(),
            });
        };
        if rest.starts_with("//") {
            return Err(SpecifierError::InvalidFormat {
                reason: "direct `npm://` URLs are not accepted as `npm:` specifiers".to_owned(),
            });
        }
        if rest.is_empty() {
            return Err(SpecifierError::InvalidFormat {
                reason: "specifier has no package name after `npm:`".to_owned(),
            });
        }

        let name_str = extract_name_prefix(rest)?;
        let name = PackageName::parse(name_str)?;
        let after_name = &rest[name_str.len()..];

        let (constraint, subpath) = if after_name.is_empty() {
            (None, None)
        } else if let Some(after_at) = after_name.strip_prefix('@') {
            let (constraint_raw, subpath_raw) = match after_at.split_once('/') {
                Some((c, s)) => (c, Some(s)),
                None => (after_at, None),
            };
            validate_constraint_token(constraint_raw)?;
            let subpath = subpath_raw.map(validate_subpath).transpose()?;
            (Some(constraint_raw.to_owned()), subpath)
        } else if let Some(subpath_raw) = after_name.strip_prefix('/') {
            let subpath = validate_subpath(subpath_raw)?;
            (None, Some(subpath))
        } else {
            return Err(SpecifierError::InvalidFormat {
                reason: "unexpected trailing characters after package name".to_owned(),
            });
        };

        Ok(Self {
            name,
            constraint,
            subpath,
        })
    }

    /// Returns the validated npm package name.
    pub fn name(&self) -> &PackageName {
        &self.name
    }

    /// Returns the explicit constraint string if present after `@`.
    pub fn constraint_str(&self) -> Option<&str> {
        self.constraint.as_deref()
    }

    /// Returns the explicit constraint or `"latest"` when omitted.
    pub fn effective_constraint_str(&self) -> &str {
        self.constraint.as_deref().unwrap_or("latest")
    }

    /// Parses the effective constraint (`"latest"` if omitted) into [`VersionConstraint`].
    pub fn constraint(&self) -> Result<VersionConstraint, SpecifierError> {
        let raw = self.effective_constraint_str();
        VersionConstraint::parse(raw).map_err(|err| SpecifierError::InvalidConstraint {
            constraint: raw.to_owned(),
            reason: err.to_string(),
        })
    }

    /// Returns the normalized subpath without a leading `/`, if any.
    pub fn subpath(&self) -> Option<&str> {
        self.subpath.as_deref()
    }

    /// Returns the canonical string form of this specifier.
    pub fn canonical(&self) -> String {
        match (&self.constraint, &self.subpath) {
            (Some(c), Some(s)) => format!("npm:{}@{c}/{s}", self.name),
            (Some(c), None) => format!("npm:{}@{c}", self.name),
            (None, Some(s)) => format!("npm:{}/{s}", self.name),
            (None, None) => format!("npm:{}", self.name),
        }
    }
}

impl fmt::Display for NpmSpecifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// Splits a bare package specifier (`pkg`, `pkg/sub`, `@scope/pkg`, `@scope/pkg/sub`)
/// into its validated [`PackageName`] and optional normalized subpath.
pub fn split_bare_package_specifier(
    raw: &str,
) -> Result<(PackageName, Option<String>), SpecifierError> {
    if raw.is_empty() {
        return Err(SpecifierError::InvalidFormat {
            reason: "bare specifier is empty".to_owned(),
        });
    }
    if raw.len() > MAX_SPECIFIER_BYTES {
        return Err(SpecifierError::InvalidFormat {
            reason: format!("bare specifier exceeds {MAX_SPECIFIER_BYTES}-byte limit"),
        });
    }
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(SpecifierError::InvalidFormat {
            reason: "bare specifier contains whitespace or control characters".to_owned(),
        });
    }
    if raw.contains('%')
        || raw.contains('\\')
        || raw.contains('?')
        || raw.contains('#')
        || raw.contains(':')
    {
        return Err(SpecifierError::InvalidFormat {
            reason: "bare specifier contains forbidden characters".to_owned(),
        });
    }
    if raw.starts_with('/') || raw.starts_with("./") || raw.starts_with("../") {
        return Err(SpecifierError::InvalidFormat {
            reason: "path specifiers are not bare package specifiers".to_owned(),
        });
    }

    let name_str = if let Some(after_at) = raw.strip_prefix('@') {
        let Some((scope, rest)) = after_at.split_once('/') else {
            return Err(SpecifierError::InvalidPackageName {
                reason: "scoped bare specifier must have `@scope/name`".to_owned(),
            });
        };
        let pkg_end = rest.find('/').unwrap_or(rest.len());
        let total = 1 + scope.len() + 1 + pkg_end;
        &raw[..total]
    } else {
        let pkg_end = raw.find('/').unwrap_or(raw.len());
        &raw[..pkg_end]
    };

    let name = PackageName::parse(name_str)?;
    let remainder = &raw[name_str.len()..];
    let subpath = if remainder.is_empty() {
        None
    } else if let Some(sub) = remainder.strip_prefix('/') {
        Some(validate_subpath(sub)?)
    } else {
        return Err(SpecifierError::InvalidFormat {
            reason: "malformed bare package specifier".to_owned(),
        });
    };
    Ok((name, subpath))
}

/// Decomposed components of a canonical virtual `npm://` module URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedVirtualUrl {
    /// Registry host (`registry.npmjs.org`).
    pub registry_host: String,
    /// Validated package name (`name` or `@scope/name`).
    pub name: PackageName,
    /// Resolved SemVer package version string.
    pub version: String,
    /// Peer-context token (`"0"` or `"p<32-hex>"`).
    pub ctx: String,
    /// Normalized package-relative file path without a leading `/`.
    pub rel_path: String,
}

impl ParsedVirtualUrl {
    /// Returns `true` if `other` belongs to the exact same resolved package instance
    /// `(registry_host, name, version, ctx)`.
    pub fn same_instance(&self, other: &Self) -> bool {
        self.registry_host == other.registry_host
            && self.name == other.name
            && self.version == other.version
            && self.ctx == other.ctx
    }

    /// Rebuilds the canonical `npm://` [`Url`] for this instance and path.
    pub fn to_url(&self) -> Result<Url, SpecifierError> {
        build_virtual_url(
            &self.registry_host,
            &self.name,
            &self.version,
            &self.ctx,
            &self.rel_path,
        )
    }
}

/// Constructs a canonical virtual URL:
/// `npm://<registry_host>/<pkg>/<version>/<ctx>/<rel_path>`.
pub fn build_virtual_url(
    registry_host: &str,
    name: &PackageName,
    version: &str,
    ctx: &str,
    rel_path: &str,
) -> Result<Url, SpecifierError> {
    if registry_host.is_empty()
        || registry_host.contains('/')
        || registry_host.contains('@')
        || registry_host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(SpecifierError::InvalidVirtualUrl {
            reason: "invalid registry host".to_owned(),
        });
    }
    validate_ctx_token(ctx)?;
    if version.is_empty()
        || version.contains('/')
        || version.contains('\\')
        || version.contains('?')
        || version.contains('#')
    {
        return Err(SpecifierError::InvalidVirtualUrl {
            reason: "invalid version segment".to_owned(),
        });
    }
    let clean_rel = validate_subpath(rel_path)?;
    let mut url = Url::parse(&format!("npm://{registry_host}/")).map_err(|_| {
        SpecifierError::InvalidVirtualUrl {
            reason: "failed to parse registry base URL".to_owned(),
        }
    })?;
    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|_| SpecifierError::InvalidVirtualUrl {
                reason: "virtual URL cannot be a base".to_owned(),
            })?;
        segs.pop_if_empty();
        if let Some(scope) = name.scope() {
            segs.push(&format!("@{scope}"));
            segs.push(name.unscoped_name());
        } else {
            segs.push(name.unscoped_name());
        }
        segs.push(version);
        segs.push(ctx);
        for segment in clean_rel.split('/') {
            segs.push(segment);
        }
    }
    Ok(url)
}

/// Parses and validates a canonical virtual `npm://` URL.
pub fn parse_virtual_url(url: &Url) -> Result<ParsedVirtualUrl, SpecifierError> {
    if url.scheme() != "npm" {
        return Err(SpecifierError::InvalidVirtualUrl {
            reason: format!("expected `npm:` scheme, got `{}`", url.scheme()),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SpecifierError::InvalidVirtualUrl {
            reason: "virtual npm URL must not contain credentials".to_owned(),
        });
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(SpecifierError::InvalidVirtualUrl {
            reason: "virtual npm URL must not contain query or fragment".to_owned(),
        });
    }
    let Some(host) = url.host_str() else {
        return Err(SpecifierError::InvalidVirtualUrl {
            reason: "virtual npm URL missing registry host".to_owned(),
        });
    };
    let segments: Vec<&str> = url
        .path_segments()
        .ok_or_else(|| SpecifierError::InvalidVirtualUrl {
            reason: "virtual npm URL has no path segments".to_owned(),
        })?
        .collect();
    if segments.is_empty() {
        return Err(SpecifierError::InvalidVirtualUrl {
            reason: "virtual npm URL missing package segments".to_owned(),
        });
    }
    let (name, idx) = if segments[0].starts_with('@') {
        if segments.len() < 5 {
            return Err(SpecifierError::InvalidVirtualUrl {
                reason: "scoped virtual npm URL missing version, ctx, or file path".to_owned(),
            });
        }
        let full = format!("{}/{}", segments[0], segments[1]);
        (PackageName::parse(&full)?, 2)
    } else {
        if segments.len() < 4 {
            return Err(SpecifierError::InvalidVirtualUrl {
                reason: "virtual npm URL missing version, ctx, or file path".to_owned(),
            });
        }
        (PackageName::parse(segments[0])?, 1)
    };
    let version = segments[idx];
    let ctx = segments[idx + 1];
    validate_ctx_token(ctx)?;
    let rel_raw = segments[idx + 2..].join("/");
    let rel_path = validate_subpath(&rel_raw)?;
    Ok(ParsedVirtualUrl {
        registry_host: host.to_owned(),
        name,
        version: version.to_owned(),
        ctx: ctx.to_owned(),
        rel_path,
    })
}

/// Validates a relative path inside a package (`rel_path`), rejecting empty
/// segments, `.`, `..`, backslashes, `%`, `?`, `#`, control chars, and
/// `node_modules` segments.
pub(crate) fn validate_subpath(raw: &str) -> Result<String, SpecifierError> {
    if raw.is_empty() {
        return Err(SpecifierError::InvalidSubpath {
            subpath: raw.to_owned(),
            reason: "subpath is empty".to_owned(),
        });
    }
    if raw.len() > MAX_SUBPATH_BYTES {
        return Err(SpecifierError::InvalidSubpath {
            subpath: raw.to_owned(),
            reason: format!("subpath exceeds {MAX_SUBPATH_BYTES}-byte limit"),
        });
    }
    if raw.contains('\\') || raw.contains('%') || raw.contains('?') || raw.contains('#') {
        return Err(SpecifierError::InvalidSubpath {
            subpath: raw.to_owned(),
            reason: "subpath contains forbidden character (`\\`, `%`, `?`, or `#`)".to_owned(),
        });
    }
    if raw.chars().any(|c| c.is_control() || c == '\0') {
        return Err(SpecifierError::InvalidSubpath {
            subpath: raw.to_owned(),
            reason: "subpath contains control characters".to_owned(),
        });
    }
    if has_windows_drive_prefix(raw) {
        return Err(SpecifierError::InvalidSubpath {
            subpath: raw.to_owned(),
            reason: "subpath may not contain a Windows drive prefix".to_owned(),
        });
    }
    for segment in raw.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(SpecifierError::InvalidSubpath {
                subpath: raw.to_owned(),
                reason: "subpath contains empty, `.`, or `..` segment".to_owned(),
            });
        }
        if has_windows_drive_prefix(segment) {
            return Err(SpecifierError::InvalidSubpath {
                subpath: raw.to_owned(),
                reason: "subpath may not contain a Windows drive prefix".to_owned(),
            });
        }
        if segment.eq_ignore_ascii_case("node_modules") {
            return Err(SpecifierError::InvalidSubpath {
                subpath: raw.to_owned(),
                reason: "`node_modules` path segments are forbidden".to_owned(),
            });
        }
    }
    Ok(raw.to_owned())
}

pub(crate) fn has_windows_drive_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn extract_name_prefix(rest: &str) -> Result<&str, SpecifierError> {
    if let Some(after_at) = rest.strip_prefix('@') {
        let Some((scope, remainder)) = after_at.split_once('/') else {
            return Err(SpecifierError::InvalidPackageName {
                reason: "scoped package name must be `@scope/name`".to_owned(),
            });
        };
        let boundary = remainder.find(['@', '/']).unwrap_or(remainder.len());
        let total = 1 + scope.len() + 1 + boundary;
        return Ok(&rest[..total]);
    }
    let boundary = rest.find(['@', '/']).unwrap_or(rest.len());
    Ok(&rest[..boundary])
}

fn validate_name_part(part: &str, kind: &str) -> Result<(), SpecifierError> {
    if part.is_empty() {
        return Err(SpecifierError::InvalidPackageName {
            reason: format!("package {kind} is empty"),
        });
    }
    if part == "." || part == ".." {
        return Err(SpecifierError::InvalidPackageName {
            reason: format!("package {kind} cannot be `.` or `..`"),
        });
    }
    if !part
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(SpecifierError::InvalidPackageName {
            reason: format!(
                "package {kind} `{part}` may only contain lowercase ASCII letters, digits, `-`, `_`, and `.`"
            ),
        });
    }
    if !part
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(SpecifierError::InvalidPackageName {
            reason: format!("package {kind} `{part}` must start with a lowercase letter or digit"),
        });
    }
    Ok(())
}

fn validate_constraint_token(raw: &str) -> Result<(), SpecifierError> {
    if raw.is_empty() {
        return Err(SpecifierError::InvalidConstraint {
            constraint: raw.to_owned(),
            reason: "constraint after `@` is empty".to_owned(),
        });
    }
    if raw.len() > MAX_CONSTRAINT_BYTES {
        return Err(SpecifierError::InvalidConstraint {
            constraint: raw.to_owned(),
            reason: format!("constraint exceeds {MAX_CONSTRAINT_BYTES}-byte limit"),
        });
    }
    if raw.contains('@') {
        return Err(SpecifierError::InvalidConstraint {
            constraint: raw.to_owned(),
            reason: "constraint may not contain `@`".to_owned(),
        });
    }
    VersionConstraint::parse(raw).map_err(|err| SpecifierError::InvalidConstraint {
        constraint: raw.to_owned(),
        reason: err.to_string(),
    })?;
    Ok(())
}

fn validate_ctx_token(ctx: &str) -> Result<(), SpecifierError> {
    if ctx == "0" {
        return Ok(());
    }
    if let Some(hex) = ctx.strip_prefix('p')
        && hex.len() == 32
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Ok(());
    }
    Err(SpecifierError::InvalidVirtualUrl {
        reason: format!("invalid peer-context token `{ctx}`"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test category: parser & boundary checks for NpmSpecifier and virtual URLs.
    #[test]
    fn parses_unscoped_scoped_ranges_and_subpaths() {
        let s1 = NpmSpecifier::parse("npm:lodash").unwrap();
        assert_eq!(s1.name().as_str(), "lodash");
        assert_eq!(s1.constraint_str(), None);
        assert_eq!(s1.effective_constraint_str(), "latest");
        assert_eq!(s1.subpath(), None);
        assert_eq!(s1.canonical(), "npm:lodash");

        let s2 = NpmSpecifier::parse("npm:@scope/pkg@^1.2.3/sub/file.mjs").unwrap();
        assert_eq!(s2.name().as_str(), "@scope/pkg");
        assert_eq!(s2.name().scope(), Some("scope"));
        assert_eq!(s2.name().unscoped_name(), "pkg");
        assert_eq!(s2.constraint_str(), Some("^1.2.3"));
        assert_eq!(s2.subpath(), Some("sub/file.mjs"));

        let s3 = NpmSpecifier::parse("npm:@scope/pkg/sub.mjs").unwrap();
        assert_eq!(s3.name().as_str(), "@scope/pkg");
        assert_eq!(s3.constraint_str(), None);
        assert_eq!(s3.subpath(), Some("sub.mjs"));

        let s4 = NpmSpecifier::parse("npm:pkg@>=1.2.0,<2.0.0").unwrap();
        assert_eq!(s4.constraint_str(), Some(">=1.2.0,<2.0.0"));
        assert!(s4.constraint().is_ok());
    }

    // Unit test category: parser & boundary checks rejecting unsafe inputs.
    #[test]
    fn rejects_invalid_specifiers_and_traversal() {
        for bad in [
            "",
            "lodash",
            "npm:",
            "npm://registry.npmjs.org/lodash/4.17.21/0/lodash.js",
            "npm:user:pass@lodash",
            "npm:lodash@",
            "npm:lodash@1.0.0/",
            "npm:lodash@1.0.0/../secret.js",
            "npm:lodash@1.0.0/./index.js",
            "npm:lodash@1.0.0/node_modules/evil.js",
            "npm:lodash@1.0.0?token=1",
            "npm:lodash@1.0.0#frag",
            "npm:lodash@1.0.0%2e%2e/a.js",
            "npm:lodash@1.0.0\\a.js",
            "npm:BadUpperCase",
            "npm:_leading_underscore",
            "npm:@scope",
            "npm:@scope/",
            "npm:lodash@1.2.3.4",
        ] {
            assert!(
                NpmSpecifier::parse(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    // Unit test category: parser round-trip for canonical virtual npm:// URLs.
    #[test]
    fn round_trips_virtual_urls() {
        let pkg = PackageName::parse("@scope/plug").unwrap();
        let url = build_virtual_url(
            "registry.npmjs.org",
            &pkg,
            "2.1.0",
            "p8f3a1c9b2e4d7a109c4e1b2f3a0d8e12",
            "dist/index.mjs",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "npm://registry.npmjs.org/@scope/plug/2.1.0/p8f3a1c9b2e4d7a109c4e1b2f3a0d8e12/dist/index.mjs"
        );
        let parsed = parse_virtual_url(&url).unwrap();
        assert_eq!(parsed.registry_host, "registry.npmjs.org");
        assert_eq!(parsed.name.as_str(), "@scope/plug");
        assert_eq!(parsed.version, "2.1.0");
        assert_eq!(parsed.ctx, "p8f3a1c9b2e4d7a109c4e1b2f3a0d8e12");
        assert_eq!(parsed.rel_path, "dist/index.mjs");
    }
}
