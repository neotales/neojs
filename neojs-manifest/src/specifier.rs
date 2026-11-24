//! Normalization of import specifiers used by a manifest.
//!
//! This module parses the small specifier surface a `neo.jsonc` import map may
//! contain and turns it into one canonical spelling:
//!
//! | Input | Canonical form |
//! | --- | --- |
//! | `gh:owner/repo@ref/path` | `https://raw.githubusercontent.com/owner/repo/ref/path` |
//! | `github:owner/repo@ref/path` | the same raw URL as the `gh:` spelling |
//! | `gh:owner/repo/path/@ref` (legacy) | `https://raw.githubusercontent.com/owner/repo/ref/path` |
//! | `npm:[@scope/]name@version[/subpath]` | the same `npm:` spelling, with a validated name |
//! | `jsr:@scope/name@version[/subpath]` | the same `jsr:` spelling, with a validated name |
//! | `node:name[/subpath]` | the same `node:` spelling |
//! | `https://...` | the URL after `url` crate normalization |
//! | `file://...` | the file URL after normalization |
//! | `./path` | the same relative path, validated |
//!
//! Normalization is not loading. [`Support`] states whether the project has a
//! transport for the canonical form. npm, JSR, and Node are always
//! [`Support::Unsupported`]. Local paths, HTTPS, and GitHub are
//! [`Support::NormalizedOnly`] because this crate does not install a manifest
//! import map into either runtime; a GitHub coordinate is loadable as a direct
//! runtime specifier only when remote loading is enabled.
//!
//! The parser is strict about ambiguity:
//!
//! - Specifiers contain no whitespace or control characters.
//! - `gh:` and `github:` accept `owner/repo@ref/path`. The reference ends at
//!   the first `/`, so a branch name that contains `/` cannot be expressed;
//!   use a tag or a full commit SHA. A path may contain `@`, including in a
//!   segment after the reference.
//! - The legacy `gh:owner/repo/path/@ref` spelling is still accepted. It uses
//!   the **last** `/@` as the reference separator, so
//!   `gh:o/r/dir/@scope/file/@v1` is rejected rather than guessed. An earlier
//!   path segment may contain `@`.
//! - The two spellings cannot be confused: `@` immediately after the
//!   repository selects the new form, and `/` after the repository selects the
//!   legacy form. A specifier with neither is rejected.
//! - The scheme itself is matched case-sensitively here (`GH:` is an unknown
//!   scheme), matching the rest of this parser.
//! - npm and JSR require an explicit version. Omission and dist-tags are left
//!   to a future transport instead of being guessed here.
//! - npm and JSR version text is preserved as one token. The parser does not
//!   implement npm range or JSR SemVer semantics.
//! - Relative targets may not contain `.` or `..` segments, so a manifest
//!   cannot name a file outside its own directory through the import map.
//! - Absolute filesystem paths are rejected. Use a path relative to the
//!   manifest, or a `file:` URL where a URL is genuinely required.

use std::fmt;

use url::Url;

use crate::Error;

/// Maximum bytes in one specifier.
pub const MAX_SPECIFIER_BYTES: usize = 8 * 1024;

const MAX_VERSION_BYTES: usize = 128;
pub(crate) const MAX_SUBPATH_BYTES: usize = 1024;
const MAX_GH_REFERENCE_BYTES: usize = 256;

const NPM_REASON: &str =
    "npm package resolution, artifact download, extraction, and cache policy are not implemented";
const JSR_REASON: &str =
    "JSR metadata resolution, artifact download, and cache policy are not implemented";
const NODE_REASON: &str =
    "Node built-in modules and a Node compatibility transform are not implemented";
const BARE_REASON: &str = "bare import-map targets are not implemented";

/// The kind of canonical specifier produced by [`normalize_specifier`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecifierKind {
    /// A `./`-prefixed module path.
    Path,
    /// A `file:` URL.
    FileUrl,
    /// An `https:` URL.
    Https,
    /// An `npm:` package specifier.
    Npm,
    /// A `jsr:` package specifier.
    Jsr,
    /// A `gh:` specifier, normalized to a GitHub raw HTTPS URL.
    Gh,
    /// A `node:` built-in or subpath.
    Node,
    /// A bare specifier such as `react`.
    Bare,
}

/// Whether the normalized specifier can be loaded by this project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// The spelling is normalized, but `neo.jsonc` does not install an import
    /// map in a runtime. A caller must not treat the mapping as loadable.
    NormalizedOnly,
    /// No transport for this kind is implemented.
    Unsupported(&'static str),
}

impl Support {
    /// Returns whether this crate implements a load path for the specifier.
    ///
    /// This is always `false` today. It is a separate method so callers do not
    /// have to inspect the reason text.
    pub fn is_supported(self) -> bool {
        false
    }
}

/// A specifier after parsing and canonicalization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormalizedSpecifier {
    /// A relative module path, preserved as written after validation.
    Path(String),
    /// A local `file:` URL.
    FileUrl(String),
    /// An `https:` URL after `url` crate normalization.
    Https(String),
    /// An npm package and optional subpath.
    Npm(NpmSpecifier),
    /// A JSR package and optional subpath.
    Jsr(JsrSpecifier),
    /// A GitHub coordinate and its raw HTTPS URL.
    Gh(GhSpecifier),
    /// A `node:` specifier, preserved after validation.
    Node(String),
    /// A bare specifier, preserved after validation.
    Bare(String),
}

impl NormalizedSpecifier {
    /// Returns the kind of this specifier.
    pub fn kind(&self) -> SpecifierKind {
        match self {
            Self::Path(_) => SpecifierKind::Path,
            Self::FileUrl(_) => SpecifierKind::FileUrl,
            Self::Https(_) => SpecifierKind::Https,
            Self::Npm(_) => SpecifierKind::Npm,
            Self::Jsr(_) => SpecifierKind::Jsr,
            Self::Gh(_) => SpecifierKind::Gh,
            Self::Node(_) => SpecifierKind::Node,
            Self::Bare(_) => SpecifierKind::Bare,
        }
    }

    /// Returns the one canonical spelling of this specifier.
    pub fn canonical(&self) -> String {
        match self {
            Self::Path(value)
            | Self::FileUrl(value)
            | Self::Https(value)
            | Self::Node(value)
            | Self::Bare(value) => value.clone(),
            Self::Npm(specifier) => specifier.canonical(),
            Self::Jsr(specifier) => specifier.canonical(),
            Self::Gh(specifier) => specifier.url.clone(),
        }
    }

    /// Returns the transport status for this specifier.
    pub fn support(&self) -> Support {
        match self {
            Self::Path(_) | Self::FileUrl(_) | Self::Https(_) | Self::Gh(_) => {
                Support::NormalizedOnly
            }
            Self::Npm(_) => Support::Unsupported(NPM_REASON),
            Self::Jsr(_) => Support::Unsupported(JSR_REASON),
            Self::Node(_) => Support::Unsupported(NODE_REASON),
            Self::Bare(_) => Support::Unsupported(BARE_REASON),
        }
    }
}

impl fmt::Display for NormalizedSpecifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

/// A validated npm package reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpmSpecifier {
    name: String,
    version: String,
    subpath: Option<String>,
    trailing_slash: bool,
}

impl NpmSpecifier {
    /// Returns the package name, including the scope when one is present.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the version text exactly as it appeared after `@`.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the subpath without a leading `/`, when one is present.
    pub fn subpath(&self) -> Option<&str> {
        self.subpath.as_deref()
    }

    /// Returns `npm:name@version` plus the optional subpath.
    pub fn canonical(&self) -> String {
        match (&self.subpath, self.trailing_slash) {
            (Some(subpath), _) => format!("npm:{}@{}/{subpath}", self.name, self.version),
            (None, true) => format!("npm:{}@{}/", self.name, self.version),
            (None, false) => format!("npm:{}@{}", self.name, self.version),
        }
    }
}

impl fmt::Display for NpmSpecifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

/// A validated JSR package reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsrSpecifier {
    scope: String,
    name: String,
    version: String,
    subpath: Option<String>,
    trailing_slash: bool,
}

impl JsrSpecifier {
    /// Returns the scope without its leading `@`.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Returns the package name without the scope.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the version text exactly as it appeared after `@`.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the subpath without a leading `/`, when one is present.
    pub fn subpath(&self) -> Option<&str> {
        self.subpath.as_deref()
    }

    /// Returns `jsr:@scope/name@version` plus the optional subpath.
    pub fn canonical(&self) -> String {
        match (&self.subpath, self.trailing_slash) {
            (Some(subpath), _) => format!(
                "jsr:@{}/{}@{}/{subpath}",
                self.scope, self.name, self.version
            ),
            (None, true) => format!("jsr:@{}/{}@{}/", self.scope, self.name, self.version),
            (None, false) => format!("jsr:@{}/{}@{}", self.scope, self.name, self.version),
        }
    }
}

impl fmt::Display for JsrSpecifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

/// A validated GitHub coordinate and the raw URL it normalizes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhSpecifier {
    owner: String,
    repo: String,
    reference: String,
    path: String,
    url: String,
}

impl GhSpecifier {
    /// Returns the GitHub owner or organisation.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Returns the repository name.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// Returns the tag, branch, or commit reference.
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Returns the repository-relative file path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the normalized `https://raw.githubusercontent.com/...` URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns [`Self::url`].
    pub fn canonical(&self) -> String {
        self.url.clone()
    }
}

impl fmt::Display for GhSpecifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.url)
    }
}

/// Parses and canonicalizes one import specifier.
///
/// The accepted grammar is documented on the module. Scheme names are matched
/// case-sensitively; `NPM:` is an unknown scheme, not `npm:`.
///
/// # Errors
///
/// Returns [`Error::InvalidSpecifier`] for a malformed specifier, and
/// [`Error::UnsupportedTransport`] for a syntactically valid scheme with no
/// implemented transport (`http:`, `data:`, `blob:`, and any other unknown
/// scheme). `npm:`, `jsr:`, and `node:` parse successfully and report
/// [`Support::Unsupported`] from [`NormalizedSpecifier::support`]. `gh:` and
/// `github:` parse successfully and report [`Support::NormalizedOnly`]: this
/// crate normalizes the coordinate but installs no import map.
pub fn normalize_specifier(raw: &str) -> Result<NormalizedSpecifier, Error> {
    normalize_specifier_with_mode(raw, false)
}

/// Parses and canonicalizes a prefix import-map target ending in `/`.
pub(crate) fn normalize_prefix_specifier(raw: &str) -> Result<NormalizedSpecifier, Error> {
    if !raw.ends_with('/') {
        return Err(invalid("a prefix import-map target must end with `/`"));
    }
    normalize_specifier_with_mode(raw, true)
}

fn normalize_specifier_with_mode(
    raw: &str,
    allow_prefix_slash: bool,
) -> Result<NormalizedSpecifier, Error> {
    validate_input(raw)?;

    if let Some(rest) = raw.strip_prefix("gh:") {
        return parse_gh(rest, allow_prefix_slash);
    }
    if let Some(rest) = raw.strip_prefix("github:") {
        return parse_gh(rest, allow_prefix_slash);
    }
    if let Some(rest) = raw.strip_prefix("npm:") {
        return parse_npm(rest, allow_prefix_slash);
    }
    if let Some(rest) = raw.strip_prefix("jsr:") {
        return parse_jsr(rest, allow_prefix_slash);
    }
    if let Some(rest) = raw.strip_prefix("node:") {
        return parse_node(rest, allow_prefix_slash);
    }
    if raw.starts_with("https:") {
        return parse_https(raw, allow_prefix_slash);
    }
    if raw.starts_with("http:") {
        return Err(Error::UnsupportedTransport {
            reason: "only https: module URLs are supported",
        });
    }
    if raw.starts_with("file:") {
        return parse_file_url(raw);
    }
    if raw.starts_with("data:") || raw.starts_with("blob:") {
        return Err(Error::UnsupportedTransport {
            reason: "data: and blob: modules are not implemented",
        });
    }

    if is_windows_drive_path(raw) {
        return Err(Error::InvalidSpecifier {
            reason: "absolute filesystem paths are not accepted in a manifest; use a relative path"
                .to_owned(),
        });
    }
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err(Error::InvalidSpecifier {
            reason: "absolute filesystem paths are not accepted in a manifest; use a relative path"
                .to_owned(),
        });
    }
    if is_relative_path(raw) {
        return parse_relative_path(raw, allow_prefix_slash);
    }
    if scheme_of(raw).is_some() {
        return Err(Error::UnsupportedTransport {
            reason: "only https:, file:, npm:, jsr:, gh:, node:, and relative paths are recognized",
        });
    }
    parse_bare(raw)
}

fn validate_input(raw: &str) -> Result<(), Error> {
    if raw.is_empty() {
        return Err(Error::InvalidSpecifier {
            reason: "the specifier is empty".to_owned(),
        });
    }
    if raw.len() > MAX_SPECIFIER_BYTES {
        return Err(Error::InvalidSpecifier {
            reason: format!("the specifier is over the {MAX_SPECIFIER_BYTES}-byte limit"),
        });
    }
    if raw.chars().any(char::is_control) {
        return Err(Error::InvalidSpecifier {
            reason: "the specifier contains a control character".to_owned(),
        });
    }
    if raw.chars().any(char::is_whitespace) {
        return Err(Error::InvalidSpecifier {
            reason: "the specifier contains whitespace".to_owned(),
        });
    }
    Ok(())
}

fn parse_gh(rest: &str, allow_prefix_slash: bool) -> Result<NormalizedSpecifier, Error> {
    let Some((owner, remainder)) = rest.split_once('/') else {
        return Err(invalid(
            "a gh: or github: specifier must name `owner/repo@reference/path`",
        ));
    };
    let repo_end = remainder.find(['@', '/']).unwrap_or(remainder.len());
    let repo = &remainder[..repo_end];
    let after_repo = &remainder[repo_end..];
    let (reference, path) = if let Some(after_at) = after_repo.strip_prefix('@') {
        parse_gh_new_reference(after_at)?
    } else if let Some(after_slash) = after_repo.strip_prefix('/') {
        parse_gh_legacy_reference(after_slash)?
    } else {
        return Err(invalid(
            "a gh: or github: specifier must include a repository, a tag or commit reference after `@`, and a file path",
        ));
    };

    validate_gh_owner(owner)?;
    validate_gh_repo(repo)?;
    validate_gh_reference(reference)?;
    let trimmed_path = if allow_prefix_slash {
        let trimmed = path.strip_suffix('/').unwrap_or(path);
        if !trimmed.is_empty() {
            validate_module_path(trimmed, MAX_SUBPATH_BYTES, "the GitHub path")?;
        }
        trimmed
    } else {
        validate_module_path(path, MAX_SUBPATH_BYTES, "the GitHub path")?;
        path
    };

    let mut url = Url::parse("https://raw.githubusercontent.com/").map_err(|_| {
        invalid("could not build the GitHub raw URL; this is an internal constant error")
    })?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| invalid("could not build the GitHub raw URL"))?;
        segments.pop_if_empty();
        segments.push(owner).push(repo).push(reference);
        if !trimmed_path.is_empty() {
            for segment in trimmed_path.split('/') {
                segments.push(segment);
            }
        }
        if allow_prefix_slash {
            segments.push("");
        }
    }

    Ok(NormalizedSpecifier::Gh(GhSpecifier {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        reference: reference.to_owned(),
        path: path.to_owned(),
        url: url.to_string(),
    }))
}

/// Splits the text after `owner/repo@` into a reference and a file path.
///
/// The reference is one segment: it ends at the first `/`. That makes the
/// spelling deterministic, but it also means a branch whose name contains `/`
/// cannot be written; use a tag or a full commit SHA. The path may itself
/// contain `@`.
fn parse_gh_new_reference(after_at: &str) -> Result<(&str, &str), Error> {
    let Some((reference, path)) = after_at.split_once('/') else {
        return Err(invalid(
            "a gh: or github: specifier must include a file path after `owner/repo@reference/`",
        ));
    };
    if reference.is_empty() {
        return Err(invalid(
            "a gh: or github: specifier must name a tag or commit reference after `@`",
        ));
    }
    Ok((reference, path))
}

/// Splits the legacy `owner/repo/path/@reference` form at its last `/@`.
///
/// A reference may not contain `/`, and the last separator wins, so
/// `owner/repo/dir/@scope/file/@v1` is rejected instead of guessed.
fn parse_gh_legacy_reference(after_slash: &str) -> Result<(&str, &str), Error> {
    let Some(separator) = after_slash.rfind("/@") else {
        return Err(invalid(
            "a gh: specifier without `@` after the repository must end the path with `/@` and a tag or commit reference",
        ));
    };
    let path = &after_slash[..separator];
    let reference = &after_slash[separator + 2..];
    if reference.contains('/') {
        return Err(invalid(
            "a gh: reference may not contain `/`; use a tag or a full commit SHA",
        ));
    }
    Ok((reference, path))
}

fn parse_npm(rest: &str, allow_prefix_slash: bool) -> Result<NormalizedSpecifier, Error> {
    let Some(name) = parse_npm_name(rest)? else {
        return Err(invalid("an npm: package name is empty"));
    };
    let Some(after_name) = rest.get(name.len()..) else {
        return Err(invalid("the npm: package name is malformed"));
    };
    let Some(version_and_path) = after_name.strip_prefix('@') else {
        return Err(invalid(
            "an npm: specifier must include an explicit version, as `npm:name@version`",
        ));
    };
    let (version, subpath) = match version_and_path.split_once('/') {
        Some((version, subpath)) => (version, Some(subpath)),
        None => (version_and_path, None),
    };
    validate_version(version)?;
    let (subpath, trailing_slash) =
        parse_subpath_with_mode(subpath, allow_prefix_slash, "the npm: subpath")?;

    Ok(NormalizedSpecifier::Npm(NpmSpecifier {
        name: name.to_owned(),
        version: version.to_owned(),
        subpath,
        trailing_slash,
    }))
}

/// Returns the package name at the start of `rest`, or why it is invalid.
fn parse_npm_name(rest: &str) -> Result<Option<&str>, Error> {
    if let Some(after_scope) = rest.strip_prefix('@') {
        let Some((scope, remainder)) = after_scope.split_once('/') else {
            return Err(invalid("an npm: scoped name must be written `@scope/name`"));
        };
        validate_npm_part(scope, "scope")?;
        let boundary = remainder.find(['@', '/']).unwrap_or(remainder.len());
        let name = &remainder[..boundary];
        if name.is_empty() {
            return Err(invalid("an npm: package name is empty"));
        }
        validate_npm_part(name, "name")?;
        let total = 2 + scope.len() + name.len();
        if total > 214 {
            return Err(invalid("the npm: package name is over the 214-byte limit"));
        }
        // `rest` starts with `@`, so the slice includes the scope separator.
        return Ok(Some(&rest[..total]));
    }

    let boundary = rest.find(['@', '/']).unwrap_or(rest.len());
    let name = &rest[..boundary];
    if name.is_empty() {
        return Ok(None);
    }
    validate_npm_part(name, "name")?;
    Ok(Some(name))
}

fn parse_jsr(rest: &str, allow_prefix_slash: bool) -> Result<NormalizedSpecifier, Error> {
    let Some(after_scope) = rest.strip_prefix('@') else {
        return Err(invalid(
            "a jsr: package must be scoped, as `jsr:@scope/name@version`",
        ));
    };
    let Some((scope, remainder)) = after_scope.split_once('/') else {
        return Err(invalid(
            "a jsr: package must include both a scope and a name",
        ));
    };
    validate_jsr_part(scope, "scope", 2, 20)?;
    let boundary = remainder.find(['@', '/']).unwrap_or(remainder.len());
    let name = &remainder[..boundary];
    validate_jsr_part(name, "name", 2, 58)?;

    let suffix = &remainder[boundary..];
    let Some(version_and_path) = suffix.strip_prefix('@') else {
        return Err(invalid(
            "a jsr: specifier must include an explicit version, as `jsr:@scope/name@version`",
        ));
    };
    let (version, subpath) = match version_and_path.split_once('/') {
        Some((version, subpath)) => (version, Some(subpath)),
        None => (version_and_path, None),
    };
    validate_version(version)?;
    let (subpath, trailing_slash) =
        parse_subpath_with_mode(subpath, allow_prefix_slash, "the jsr: subpath")?;

    Ok(NormalizedSpecifier::Jsr(JsrSpecifier {
        scope: scope.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        subpath,
        trailing_slash,
    }))
}

fn parse_subpath_with_mode(
    subpath: Option<&str>,
    allow_prefix_slash: bool,
    subject: &str,
) -> Result<(Option<String>, bool), Error> {
    let Some(subpath) = subpath else {
        return Ok((None, false));
    };
    if allow_prefix_slash {
        let trimmed = subpath.strip_suffix('/').unwrap_or(subpath);
        if trimmed.is_empty() {
            return Ok((None, true));
        }
        validate_module_path(trimmed, MAX_SUBPATH_BYTES, subject)?;
        return Ok((Some(format!("{trimmed}/")), true));
    }
    validate_module_path(subpath, MAX_SUBPATH_BYTES, subject)?;
    Ok((Some(subpath.to_owned()), false))
}

fn parse_node(rest: &str, allow_prefix_slash: bool) -> Result<NormalizedSpecifier, Error> {
    let check = if allow_prefix_slash {
        rest.strip_suffix('/').unwrap_or(rest)
    } else {
        rest
    };
    validate_node_name(check)?;
    Ok(NormalizedSpecifier::Node(format!("node:{rest}")))
}

fn parse_https(raw: &str, allow_prefix_slash: bool) -> Result<NormalizedSpecifier, Error> {
    let url = Url::parse(raw).map_err(|_| invalid("the https: URL is malformed"))?;
    if url.scheme() != "https" {
        return Err(invalid("the URL scheme must be `https`"));
    }
    if url.host_str().is_none() {
        return Err(invalid("the https: URL must have a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "an https: URL with a username or password is not supported",
        ));
    }
    if allow_prefix_slash && (url.query().is_some() || url.fragment().is_some()) {
        return Err(invalid(
            "a prefix https: URL may not contain a query or fragment",
        ));
    }
    Ok(NormalizedSpecifier::Https(url.to_string()))
}

fn parse_file_url(raw: &str) -> Result<NormalizedSpecifier, Error> {
    let url = Url::parse(raw).map_err(|_| invalid("the file: URL is malformed"))?;
    if url.scheme() != "file" {
        return Err(invalid("the URL scheme must be `file`"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "a file: URL with a username or password is not supported",
        ));
    }
    if url.host_str().is_some() {
        return Err(invalid(
            "a file: URL must not name a remote host; use a local path",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("a file: URL may not contain a query or fragment"));
    }
    if url.to_file_path().is_err() {
        return Err(invalid("the file: URL does not name a local file path"));
    }
    Ok(NormalizedSpecifier::FileUrl(url.to_string()))
}

fn parse_relative_path(raw: &str, allow_prefix_slash: bool) -> Result<NormalizedSpecifier, Error> {
    if allow_prefix_slash && raw == "./" {
        return Ok(NormalizedSpecifier::Path("./".to_owned()));
    }
    let path = raw.strip_prefix("./").unwrap_or(raw);
    let path = if allow_prefix_slash {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    validate_module_path(path, MAX_SUBPATH_BYTES, "the relative path")?;
    Ok(NormalizedSpecifier::Path(raw.to_owned()))
}

fn parse_bare(raw: &str) -> Result<NormalizedSpecifier, Error> {
    if raw.contains(':') {
        return Err(invalid(
            "a bare specifier cannot contain `:`; a colon is only valid in a URL scheme",
        ));
    }
    if raw.len() > 1024 {
        return Err(invalid("the bare specifier is over the 1024-byte limit"));
    }
    if raw
        .bytes()
        .any(|byte| byte == b'\\' || byte.is_ascii_control())
    {
        return Err(invalid(
            "the bare specifier contains a disallowed character",
        ));
    }
    Ok(NormalizedSpecifier::Bare(raw.to_owned()))
}

fn validate_npm_part(part: &str, kind: &str) -> Result<(), Error> {
    if part.is_empty() {
        return Err(invalid(&format!("the npm: {kind} is empty")));
    }
    if part == "." || part == ".." {
        return Err(invalid(&format!("the npm: {kind} cannot be `.` or `..`")));
    }
    if part.len() > 214 {
        return Err(invalid(&format!(
            "the npm: {kind} is over the 214-byte limit"
        )));
    }
    if !part.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
    }) {
        return Err(invalid(&format!(
            "the npm: {kind} may contain only lowercase letters, digits, `-`, `_`, and `.`"
        )));
    }
    if !part
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(invalid(&format!(
            "the npm: {kind} must start with a letter or digit"
        )));
    }
    Ok(())
}

fn validate_jsr_part(part: &str, kind: &str, min: usize, max: usize) -> Result<(), Error> {
    if part.len() < min || part.len() > max {
        return Err(invalid(&format!(
            "the jsr: {kind} must be {min} to {max} bytes"
        )));
    }
    if !part
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(invalid(&format!(
            "the jsr: {kind} may contain only lowercase letters, digits, and `-`"
        )));
    }
    if part.starts_with('-') || part.ends_with('-') {
        return Err(invalid(&format!(
            "the jsr: {kind} may not start or end with `-`"
        )));
    }
    Ok(())
}

fn validate_gh_owner(owner: &str) -> Result<(), Error> {
    if owner.is_empty() || owner.len() > 39 {
        return Err(invalid("the GitHub owner must be 1 to 39 bytes"));
    }
    if !owner
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(invalid(
            "the GitHub owner may contain only ASCII letters, digits, and `-`",
        ));
    }
    if owner.starts_with('-') || owner.ends_with('-') {
        return Err(invalid("the GitHub owner may not start or end with `-`"));
    }
    Ok(())
}

fn validate_gh_repo(repo: &str) -> Result<(), Error> {
    if repo.is_empty() || repo.len() > 100 {
        return Err(invalid("the GitHub repository must be 1 to 100 bytes"));
    }
    if repo == "." || repo == ".." || repo.eq_ignore_ascii_case(".git") {
        return Err(invalid(
            "the GitHub repository cannot be `.`, `..`, or `.git`",
        ));
    }
    if !repo
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid(
            "the GitHub repository may contain only ASCII letters, digits, `.`, `_`, and `-`",
        ));
    }
    Ok(())
}

fn validate_gh_reference(reference: &str) -> Result<(), Error> {
    if reference.is_empty() || reference.len() > MAX_GH_REFERENCE_BYTES {
        return Err(invalid(&format!(
            "the GitHub reference must be 1 to {MAX_GH_REFERENCE_BYTES} bytes"
        )));
    }
    if reference.contains("..") {
        return Err(invalid("the GitHub reference may not contain `..`"));
    }
    if !reference.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-' | b'@')
    }) {
        return Err(invalid(
            "the GitHub reference may contain only ASCII letters, digits, `.`, `_`, `+`, `-`, and `@`",
        ));
    }
    if !reference
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(invalid(
            "the GitHub reference must start with a letter or digit",
        ));
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<(), Error> {
    if version.is_empty() {
        return Err(invalid("the version after `@` is empty"));
    }
    if version.len() > MAX_VERSION_BYTES {
        return Err(invalid(&format!(
            "the version is over the {MAX_VERSION_BYTES}-byte limit"
        )));
    }
    if !version
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'/' | b'@' | b'\\' | b'?' | b'#'))
    {
        return Err(invalid(
            "the version must be one token without `/`, `@`, `\\`, `?`, or `#`",
        ));
    }
    Ok(())
}

fn validate_node_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.len() > 256 {
        return Err(invalid("a node: specifier must name a module"));
    }
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(invalid(
                "the node: module path has an empty or traversal segment",
            ));
        }
        if !segment.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
        }) {
            return Err(invalid(
                "the node: module path may contain only lowercase letters, digits, `_`, `.`, and `-`",
            ));
        }
    }
    Ok(())
}

/// Validates a slash-separated module path with no empty, `.`, or `..` segment.
pub(crate) fn validate_module_path(
    path: &str,
    max_bytes: usize,
    subject: &str,
) -> Result<(), Error> {
    if path.is_empty() {
        return Err(invalid(&format!("{subject} is empty")));
    }
    if path.len() > max_bytes {
        return Err(invalid(&format!(
            "{subject} is over the {max_bytes}-byte limit"
        )));
    }
    if path.contains('\\') || path.contains('%') || path.contains('?') || path.contains('#') {
        return Err(invalid(&format!(
            "{subject} contains `\\`, `%`, `?`, or `#`"
        )));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(invalid(&format!(
                "{subject} has an empty, `.`, or `..` segment"
            )));
        }
        if !segment.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'.' | b'_'
                        | b'-'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                )
        }) {
            return Err(invalid(&format!(
                "{subject} contains a character outside the safe module-path set"
            )));
        }
    }
    Ok(())
}

fn invalid(reason: &str) -> Error {
    Error::InvalidSpecifier {
        reason: reason.to_owned(),
    }
}

fn is_relative_path(raw: &str) -> bool {
    raw.starts_with("./") || raw.starts_with("../")
}

pub(crate) fn is_windows_drive_path(raw: &str) -> bool {
    raw.len() >= 2 && raw.as_bytes()[0].is_ascii_alphabetic() && raw.as_bytes()[1] == b':'
}

fn scheme_of(raw: &str) -> Option<&str> {
    let (scheme, _) = raw.split_once(':')?;
    if scheme.is_empty() {
        return None;
    }
    let mut bytes = scheme.bytes();
    let first = bytes.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')) {
        Some(scheme)
    } else {
        None
    }
}
