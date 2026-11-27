//! Bounded `.npmrc` parsing, scoped registry routing, and HTTPS-only registry
//! authentication with lazy `${ENV_VAR}` resolution, redaction, and zeroization.
//!
//! Security invariants:
//! - Bounded input size ([`MAX_NPMRC_BYTES`] = 256 KiB).
//! - File loading uses `O_NOFOLLOW` on Unix and regular-file metadata checks,
//!   silently skipping missing `.npmrc` files while failing closed on symlinks
//!   or non-regular files.
//! - Only `https:` registries are accepted for `registry=` and `@scope:registry=`.
//! - [`NpmrcConfig::auth_for_url`] NEVER returns credentials for non-HTTPS URLs
//!   and fails closed with [`NpmrcError::InsecureAuthUrl`] if an auth rule
//!   matches a non-HTTPS URL.
//! - `${VAR_NAME}` interpolation allows only `[A-Za-z_][A-Za-z0-9_]*` names with
//!   zero shell evaluation, and fails closed lazily with
//!   [`NpmrcError::MissingAuthEnvVar`] only when a request matches that rule.
//! - Secrets are stored with [`ZeroizeOnDrop`] and custom `Debug`/`Display`
//!   implementations printing `<redacted>`. Raw credentials never appear in any
//!   error or `Debug` output.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;

use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::resolver::DEFAULT_REGISTRY_ORIGIN;

/// Maximum byte size accepted when reading or parsing a `.npmrc` file (256 KiB).
pub const MAX_NPMRC_BYTES: usize = 256 * 1024;

/// Maximum length of an npm scope name (matches `MAX_PACKAGE_NAME_BYTES`).
const MAX_SCOPE_BYTES: usize = 214;

/// Redacted, zeroize-on-drop wrapper for a registry credential secret.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedSecret {
    value: Zeroizing<String>,
}

impl RedactedSecret {
    /// Wraps a validated secret string.
    pub fn new(value: String) -> Self {
        Self {
            value: Zeroizing::new(value),
        }
    }

    /// Returns the underlying secret string for attaching to an outgoing HTTPS
    /// request header.
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

impl Zeroize for RedactedSecret {
    fn zeroize(&mut self) {
        self.value.zeroize();
    }
}

impl ZeroizeOnDrop for RedactedSecret {}

impl fmt::Debug for RedactedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for RedactedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Authentication credential resolved for a matching HTTPS registry URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryAuth {
    /// `Authorization: Bearer <token>` from `//...:_authToken=...`.
    Bearer(RedactedSecret),
    /// `Authorization: Basic <base64>` from `//...:_auth=...`.
    Basic(RedactedSecret),
}

impl RegistryAuth {
    /// Formats the complete `Authorization` HTTP header value inside a
    /// [`Zeroizing`] buffer.
    pub fn authorization_header_value(&self) -> Zeroizing<String> {
        match self {
            Self::Bearer(secret) => Zeroizing::new(format!("Bearer {}", secret.as_str())),
            Self::Basic(secret) => Zeroizing::new(format!("Basic {}", secret.as_str())),
        }
    }
}

/// Failure returned while reading, parsing, or resolving `.npmrc` configuration.
///
/// Error messages and `Debug` representations never contain secret tokens,
/// `_auth` credentials, or `Authorization` header values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NpmrcError {
    /// `.npmrc` content exceeded [`MAX_NPMRC_BYTES`].
    #[error(".npmrc exceeds {limit}-byte limit ({size} bytes)")]
    TooLarge { size: usize, limit: usize },

    /// Filesystem operation on `.npmrc` failed.
    #[error(".npmrc {operation} failed: {reason}")]
    Io {
        operation: &'static str,
        reason: String,
    },

    /// `.npmrc` path existed but was not a regular file.
    #[error(".npmrc is not a regular file")]
    NotRegularFile,

    /// `.npmrc` bytes were not valid UTF-8.
    #[error(".npmrc is not valid UTF-8")]
    InvalidUtf8,

    /// Line syntax (such as unclosed quotes) was malformed.
    #[error("invalid .npmrc syntax at line {line}: {reason}")]
    InvalidSyntax { line: usize, reason: String },

    /// `@scope:registry` contained an invalid npm scope name.
    #[error("invalid npm scope `{scope}` in .npmrc")]
    InvalidScope { scope: String },

    /// `registry=` or `@scope:registry=` used a non-HTTPS URL scheme.
    #[error("registry URL in .npmrc must use https: scheme, got `{scheme}:`")]
    InsecureRegistryUrl { scheme: String },

    /// `registry=` or `@scope:registry=` URL was malformed or contained
    /// disallowed components (credentials, query, fragment, missing host).
    #[error("invalid registry URL in .npmrc: {reason}")]
    InvalidRegistryUrl { reason: String },

    /// An auth rule matched a non-HTTPS URL.
    #[error("refusing to send registry credentials over non-HTTPS URL")]
    InsecureAuthUrl,

    /// A matching auth rule referenced an unset environment variable.
    #[error("required environment variable `{var_name}` for registry authentication is not set")]
    MissingAuthEnvVar { var_name: String },

    /// `${...}` environment variable reference syntax or identifier was invalid.
    #[error("invalid environment variable reference in .npmrc at line {line}")]
    InvalidEnvVarName { line: usize },

    /// Auth scope key (`//host[:port][/path/]:_authToken`) was malformed.
    #[error("invalid registry auth key in .npmrc at line {line}: {reason}")]
    InvalidAuthKey { line: usize, reason: String },

    /// Auth credential value was empty.
    #[error("empty auth credential in .npmrc at line {line}")]
    EmptyAuthValue { line: usize },

    /// Auth credential value contained disallowed characters (such as CR/LF/NUL).
    #[error("invalid auth credential in .npmrc at line {line}: {reason}")]
    InvalidAuthValue { line: usize, reason: String },

    /// Project-level `.npmrc` attempted to configure auth directives or `${ENV_VAR}` interpolation.
    #[error(
        "project .npmrc at line {line} cannot configure auth directives or env interpolation; move credentials to ~/.npmrc"
    )]
    ProjectAuthNotAllowed { line: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegistryTarget {
    url: Url,
    origin: String,
}

impl RegistryTarget {
    fn default_npmjs() -> Self {
        let url = Url::parse(&format!("{DEFAULT_REGISTRY_ORIGIN}/"))
            .expect("DEFAULT_REGISTRY_ORIGIN is a valid HTTPS URL");
        Self {
            url,
            origin: DEFAULT_REGISTRY_ORIGIN.to_owned(),
        }
    }

    fn from_validated_url(url: Url) -> Self {
        let origin = url.as_str().trim_end_matches('/').to_owned();
        Self { url, origin }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AuthScope {
    host: String,
    explicit_port: Option<u16>,
    path_prefix: String,
}

impl AuthScope {
    fn matches(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        if !host.eq_ignore_ascii_case(&self.host) {
            return false;
        }
        let port_matches = if url.scheme() == "https" {
            let req_port = url.port().filter(|&p| p != 443);
            req_port == self.explicit_port
        } else {
            let req_port = url.port();
            self.explicit_port == req_port
                || (self.explicit_port.is_none()
                    && (req_port.is_none() || (url.scheme() == "http" && req_port == Some(80))))
        };
        if !port_matches {
            return false;
        }
        if self.path_prefix == "/" {
            return true;
        }
        Self::path_prefix_matches(&self.path_prefix, url)
    }

    fn path_prefix_matches(path_prefix: &str, url: &Url) -> bool {
        if path_prefix == "/" {
            return true;
        }
        let req_path = url.path();
        req_path.starts_with(path_prefix) || req_path == &path_prefix[..path_prefix.len() - 1]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AuthEntry {
    Ready(RegistryAuth),
    MissingEnvVar { var_name: String },
}

/// Parsed `.npmrc` configuration for scoped registry routing and HTTPS-only
/// registry authentication.
#[derive(Clone, Debug)]
pub struct NpmrcConfig {
    default_registry: RegistryTarget,
    explicit_default_registry: bool,
    scoped_registries: BTreeMap<String, RegistryTarget>,
    auth_rules: BTreeMap<AuthScope, AuthEntry>,
}

impl Default for NpmrcConfig {
    fn default() -> Self {
        Self {
            default_registry: RegistryTarget::default_npmjs(),
            explicit_default_registry: false,
            scoped_registries: BTreeMap::new(),
            auth_rules: BTreeMap::new(),
        }
    }
}

impl NpmrcConfig {
    /// Parses `.npmrc` text using `env_lookup` for `${VAR_NAME}` substitution.
    ///
    /// Rejects payloads larger than [`MAX_NPMRC_BYTES`], ignores blank lines and
    /// `#` / `;` comments, supports quoted values, normalizes `@scope:registry`
    /// keys case-insensitively, validates HTTPS schemes for configured
    /// registries, and records missing `${ENV_VAR}` references lazily so
    /// unrelated public registries continue to work.
    pub fn parse(
        content: &str,
        env_lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, NpmrcError> {
        Self::parse_with_policy(content, env_lookup, true)
    }

    /// Parses a project-level `.npmrc` permitting registry routing (`registry=`,
    /// `@scope:registry=`) only.
    ///
    /// Rejects auth directives (`_authToken`, `_auth`, `always-auth`, `//...:`
    /// auth keys) and any `${ENV_VAR}` interpolation to prevent untrusted
    /// repositories from exfiltrating ambient environment variables.
    pub fn parse_project(content: &str) -> Result<Self, NpmrcError> {
        Self::parse_with_policy(content, &|_| None, false)
    }

    fn parse_with_policy(
        content: &str,
        env_lookup: &dyn Fn(&str) -> Option<String>,
        allow_auth: bool,
    ) -> Result<Self, NpmrcError> {
        if content.len() > MAX_NPMRC_BYTES {
            return Err(NpmrcError::TooLarge {
                size: content.len(),
                limit: MAX_NPMRC_BYTES,
            });
        }

        let mut config = Self::default();

        for (idx, raw_line) in content.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = raw_line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }

            let Some((raw_key, raw_val)) = trimmed.split_once('=') else {
                // Bare boolean flag (e.g. `save-exact`) if single token without quotes/whitespace.
                if trimmed
                    .chars()
                    .any(|c| c.is_whitespace() || c == '"' || c == '\'')
                {
                    return Err(NpmrcError::InvalidSyntax {
                        line: line_no,
                        reason: "expected `key=value` assignment".to_owned(),
                    });
                }
                if !allow_auth && is_auth_directive_key(trimmed) {
                    return Err(NpmrcError::ProjectAuthNotAllowed { line: line_no });
                }
                continue;
            };

            let key = raw_key.trim();
            if key.is_empty() {
                return Err(NpmrcError::InvalidSyntax {
                    line: line_no,
                    reason: "empty configuration key".to_owned(),
                });
            }

            let value = parse_value(raw_val, line_no)?;

            if !allow_auth && (is_auth_directive_key(key) || value.contains("${")) {
                return Err(NpmrcError::ProjectAuthNotAllowed { line: line_no });
            }

            if key == "registry" {
                let target = parse_registry_target(value.as_str())?;
                config.default_registry = target;
                config.explicit_default_registry = true;
                continue;
            }

            if let Some((scope_part, setting)) = key.split_once(':')
                && scope_part.starts_with('@')
                && setting == "registry"
            {
                let norm_scope = normalize_and_validate_scope(&scope_part[1..])?;
                let target = parse_registry_target(value.as_str())?;
                config.scoped_registries.insert(norm_scope, target);
                continue;
            }

            if allow_auth
                && key.starts_with("//")
                && let Some((prefix, setting)) = key.rsplit_once(':')
                && matches!(setting, "_authToken" | "_auth")
            {
                let scope = parse_auth_scope(prefix, line_no)?;
                let entry = build_auth_entry(setting, value.as_str(), line_no, env_lookup)?;
                config.auth_rules.insert(scope, entry);
            }
        }

        Ok(config)
    }

    /// Loads `~/.npmrc` (if `home_dir` is `Some`) and `<project_dir>/.npmrc`,
    /// with project settings overriding registry routing ONLY.
    ///
    /// Auth directives (`_authToken`, `_auth`, `always-auth`) and `${ENV_VAR}`
    /// interpolation are permitted only in `~/.npmrc` and rejected in project
    /// `.npmrc` files.
    ///
    /// Uses `O_NOFOLLOW` on Unix and regular-file bounded reads ([`MAX_NPMRC_BYTES`]).
    /// Missing `.npmrc` files are silently skipped; symlinks, non-regular files,
    /// oversized files, and invalid syntax fail closed.
    pub fn load_for_project(
        project_dir: &Path,
        home_dir: Option<&Path>,
        env_lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, NpmrcError> {
        Self::load_for_project_with_options(Some(project_dir), home_dir, env_lookup, false)
    }

    /// Loads `~/.npmrc` (if `home_dir` is `Some`) and optionally
    /// `<project_dir>/.npmrc` (if `project_dir` is `Some`).
    ///
    /// When `allow_project_auth` is `false` (the safe default), project `.npmrc`
    /// files may configure registry routing (`registry=`, `@scope:registry=`)
    /// only, and reject auth directives and `${ENV_VAR}` interpolation.
    /// Trusted embedders/tests may pass `allow_project_auth = true` to opt in
    /// to project-level auth directives.
    pub fn load_for_project_with_options(
        project_dir: Option<&Path>,
        home_dir: Option<&Path>,
        env_lookup: &dyn Fn(&str) -> Option<String>,
        allow_project_auth: bool,
    ) -> Result<Self, NpmrcError> {
        let mut merged = Self::default();

        if let Some(home) = home_dir
            && let Some(content) = read_optional_npmrc_file(&home.join(".npmrc"))?
        {
            let user_cfg = Self::parse(&content, env_lookup)?;
            merged.merge_from(user_cfg);
        }

        if let Some(proj) = project_dir
            && let Some(content) = read_optional_npmrc_file(&proj.join(".npmrc"))?
        {
            let project_cfg = if allow_project_auth {
                Self::parse_with_policy(&content, env_lookup, true)?
            } else {
                Self::parse_project(&content)?
            };
            merged.merge_from(project_cfg);
        }

        Ok(merged)
    }

    /// Merges `other` onto `self`, overriding default registry (if explicitly set
    /// in `other`), scoped registries, and matching auth rules.
    fn merge_from(&mut self, other: Self) {
        if other.explicit_default_registry {
            self.default_registry = other.default_registry;
            self.explicit_default_registry = true;
        }
        for (scope, target) in other.scoped_registries {
            self.scoped_registries.insert(scope, target);
        }
        for (scope, entry) in other.auth_rules {
            self.auth_rules.insert(scope, entry);
        }
    }

    /// Returns `true` if `registry=...` was explicitly configured in `.npmrc`.
    pub fn has_explicit_default_registry(&self) -> bool {
        self.explicit_default_registry
    }

    /// Returns the default registry URL (defaults to `https://registry.npmjs.org/`).
    pub fn default_registry(&self) -> &Url {
        &self.default_registry.url
    }

    /// Returns the default registry origin string without trailing `/`.
    pub fn default_registry_origin(&self) -> &str {
        &self.default_registry.origin
    }

    /// Resolves the registry URL for `package_name` (`@scope/pkg` or `pkg`),
    /// checking `@scope:registry` case-insensitively before falling back to the
    /// default registry.
    pub fn registry_for_package(&self, package_name: &str) -> &Url {
        self.target_for_package(package_name)
            .map_or(&self.default_registry.url, |t| &t.url)
    }

    /// Resolves the registry origin string (without trailing `/`) for
    /// `package_name`.
    pub fn registry_origin_for_package(&self, package_name: &str) -> &str {
        self.target_for_package(package_name)
            .map_or(self.default_registry.origin.as_str(), |t| t.origin.as_str())
    }

    /// Returns the explicit `@scope:registry` URL for `package_name`, if one
    /// is configured for its scope.
    pub fn scoped_registry_for_package(&self, package_name: &str) -> Option<&Url> {
        self.target_for_package(package_name).map(|t| &t.url)
    }

    /// Returns the explicit `@scope:registry` origin string (without trailing
    /// `/`) for `package_name`, if one is configured for its scope.
    pub fn scoped_registry_origin_for_package(&self, package_name: &str) -> Option<&str> {
        self.target_for_package(package_name)
            .map(|t| t.origin.as_str())
    }

    fn target_for_package(&self, package_name: &str) -> Option<&RegistryTarget> {
        let rest = package_name.strip_prefix('@')?;
        let (scope, _) = rest.split_once('/')?;
        let norm = scope.to_ascii_lowercase();
        self.scoped_registries.get(&norm)
    }

    /// Returns `true` if `url`'s path matches the normalized `path_prefix`
    /// on segment boundaries.
    pub fn path_prefix_matches(path_prefix: &str, url: &Url) -> bool {
        AuthScope::path_prefix_matches(path_prefix, url)
    }

    /// Resolves the authentication credential and matched normalized path prefix
    /// for `url`.
    ///
    /// Enforces HTTPS-only delivery, exact host + port matching, and longest
    /// normalized path-prefix matching.
    ///
    /// - Returns `Err(NpmrcError::InsecureAuthUrl)` if an auth rule matches a
    ///   non-HTTPS URL.
    /// - Returns `Err(NpmrcError::MissingAuthEnvVar { .. })` if the best-matching
    ///   HTTPS auth rule referenced an unset `${ENV_VAR}`.
    /// - Returns `Ok(None)` if no auth rule matches `url`.
    pub fn auth_with_scope_for_url(
        &self,
        url: &Url,
    ) -> Result<Option<(&RegistryAuth, &str)>, NpmrcError> {
        let mut best: Option<(&AuthScope, &AuthEntry)> = None;
        for (scope, entry) in &self.auth_rules {
            if !scope.matches(url) {
                continue;
            }
            let replace = match best {
                None => true,
                Some((cur_scope, _)) => scope.path_prefix.len() > cur_scope.path_prefix.len(),
            };
            if replace {
                best = Some((scope, entry));
            }
        }

        let Some((scope, entry)) = best else {
            return Ok(None);
        };

        if url.scheme() != "https" {
            return Err(NpmrcError::InsecureAuthUrl);
        }

        match entry {
            AuthEntry::Ready(auth) => Ok(Some((auth, scope.path_prefix.as_str()))),
            AuthEntry::MissingEnvVar { var_name } => Err(NpmrcError::MissingAuthEnvVar {
                var_name: var_name.clone(),
            }),
        }
    }

    /// Resolves the authentication credential for `url`.
    ///
    /// Enforces HTTPS-only delivery, exact host + port matching, and longest
    /// normalized path-prefix matching.
    ///
    /// - Returns `Err(NpmrcError::InsecureAuthUrl)` if an auth rule matches a
    ///   non-HTTPS URL.
    /// - Returns `Err(NpmrcError::MissingAuthEnvVar { .. })` if the best-matching
    ///   HTTPS auth rule referenced an unset `${ENV_VAR}`.
    /// - Returns `Ok(None)` if no auth rule matches `url`.
    pub fn auth_for_url(&self, url: &Url) -> Result<Option<&RegistryAuth>, NpmrcError> {
        self.auth_with_scope_for_url(url)
            .map(|opt| opt.map(|(auth, _)| auth))
    }
}

fn read_optional_npmrc_file(path: &Path) -> Result<Option<Zeroizing<String>>, NpmrcError> {
    let pre_meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(NpmrcError::Io {
                operation: "stat",
                reason: err.to_string(),
            });
        }
    };
    if pre_meta.file_type().is_symlink() || !pre_meta.is_file() {
        return Err(NpmrcError::NotRegularFile);
    }

    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = match opts.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(NpmrcError::Io {
                operation: "open",
                reason: err.to_string(),
            });
        }
    };

    let meta = file.metadata().map_err(|err| NpmrcError::Io {
        operation: "stat",
        reason: err.to_string(),
    })?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(NpmrcError::NotRegularFile);
    }
    if meta.len() > MAX_NPMRC_BYTES as u64 {
        return Err(NpmrcError::TooLarge {
            size: meta.len() as usize,
            limit: MAX_NPMRC_BYTES,
        });
    }

    let mut buf = Zeroizing::new(Vec::with_capacity(meta.len() as usize));
    let read_limit = (MAX_NPMRC_BYTES as u64).saturating_add(1);
    file.take(read_limit)
        .read_to_end(&mut buf)
        .map_err(|err| NpmrcError::Io {
            operation: "read",
            reason: err.to_string(),
        })?;
    if buf.len() > MAX_NPMRC_BYTES {
        return Err(NpmrcError::TooLarge {
            size: buf.len(),
            limit: MAX_NPMRC_BYTES,
        });
    }

    let text = String::from_utf8(std::mem::take(&mut *buf)).map_err(|_| NpmrcError::InvalidUtf8)?;
    Ok(Some(Zeroizing::new(text)))
}

fn parse_value(raw_val: &str, line_no: usize) -> Result<Zeroizing<String>, NpmrcError> {
    let trimmed = raw_val.trim_start();
    if let Some(first) = trimmed.chars().next()
        && (first == '"' || first == '\'')
    {
        let mut out = Zeroizing::new(String::new());
        let mut chars = trimmed[first.len_utf8()..].char_indices();
        let mut closed_at = None;
        while let Some((idx, ch)) = chars.next() {
            if ch == '\\' && first == '"' {
                let Some((_, esc)) = chars.next() else {
                    return Err(NpmrcError::InvalidSyntax {
                        line: line_no,
                        reason: "unterminated escape sequence in quoted value".to_owned(),
                    });
                };
                match esc {
                    '"' | '\\' => out.push(esc),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                }
                continue;
            }
            if ch == first {
                closed_at = Some(first.len_utf8() + idx + ch.len_utf8());
                break;
            }
            out.push(ch);
        }

        let Some(end_idx) = closed_at else {
            return Err(NpmrcError::InvalidSyntax {
                line: line_no,
                reason: "unclosed quoted value".to_owned(),
            });
        };
        let trailing = trimmed[end_idx..].trim_start();
        if !trailing.is_empty() && !trailing.starts_with('#') && !trailing.starts_with(';') {
            return Err(NpmrcError::InvalidSyntax {
                line: line_no,
                reason: "unexpected trailing characters after quoted value".to_owned(),
            });
        }
        return Ok(out);
    }

    // Unquoted value: strip trailing inline comment after whitespace (` #` / ` ;`).
    let mut end = trimmed.len();
    let bytes = trimmed.as_bytes();
    for i in 0..bytes.len() {
        if (bytes[i] == b'#' || bytes[i] == b';') && (i == 0 || bytes[i - 1].is_ascii_whitespace())
        {
            end = i;
            break;
        }
    }
    Ok(Zeroizing::new(trimmed[..end].trim_end().to_owned()))
}

fn normalize_and_validate_scope(raw_scope: &str) -> Result<String, NpmrcError> {
    let norm = raw_scope.to_ascii_lowercase();
    let valid_chars = !norm.is_empty()
        && norm.len() <= MAX_SCOPE_BYTES
        && norm
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && norm.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        });
    if !valid_chars {
        return Err(NpmrcError::InvalidScope {
            scope: raw_scope.to_owned(),
        });
    }
    Ok(norm)
}

fn parse_registry_target(raw: &str) -> Result<RegistryTarget, NpmrcError> {
    if raw.is_empty() {
        return Err(NpmrcError::InvalidRegistryUrl {
            reason: "registry URL is empty".to_owned(),
        });
    }
    let mut url = Url::parse(raw).map_err(|err| NpmrcError::InvalidRegistryUrl {
        reason: err.to_string(),
    })?;
    if url.scheme() != "https" {
        return Err(NpmrcError::InsecureRegistryUrl {
            scheme: url.scheme().to_owned(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NpmrcError::InvalidRegistryUrl {
            reason: "registry URL must not contain embedded credentials".to_owned(),
        });
    }
    if url.host_str().is_none() {
        return Err(NpmrcError::InvalidRegistryUrl {
            reason: "registry URL must have a host".to_owned(),
        });
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(NpmrcError::InvalidRegistryUrl {
            reason: "registry URL must not contain a query or fragment".to_owned(),
        });
    }
    if !url.path().ends_with('/') {
        let mut path = url.path().to_owned();
        path.push('/');
        url.set_path(&path);
    }
    Ok(RegistryTarget::from_validated_url(url))
}

fn parse_auth_scope(prefix: &str, line_no: usize) -> Result<AuthScope, NpmrcError> {
    if !prefix.starts_with("//") || prefix.len() <= 2 {
        return Err(NpmrcError::InvalidAuthKey {
            line: line_no,
            reason: "auth key must begin with `//host`".to_owned(),
        });
    }
    let synthetic = format!("https:{prefix}");
    let url = Url::parse(&synthetic).map_err(|err| NpmrcError::InvalidAuthKey {
        line: line_no,
        reason: err.to_string(),
    })?;
    if url.scheme() != "https" {
        return Err(NpmrcError::InvalidAuthKey {
            line: line_no,
            reason: "unexpected scheme in auth scope".to_owned(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NpmrcError::InvalidAuthKey {
            line: line_no,
            reason: "auth scope must not contain embedded URL credentials".to_owned(),
        });
    }
    let Some(host) = url.host_str() else {
        return Err(NpmrcError::InvalidAuthKey {
            line: line_no,
            reason: "auth scope must specify a host".to_owned(),
        });
    };
    if url.query().is_some() || url.fragment().is_some() {
        return Err(NpmrcError::InvalidAuthKey {
            line: line_no,
            reason: "auth scope must not contain a query or fragment".to_owned(),
        });
    }
    let explicit_port = url.port().filter(|&p| p != 443);
    let path_prefix = normalize_path_prefix(url.path());
    Ok(AuthScope {
        host: host.to_owned(),
        explicit_port,
        path_prefix,
    })
}

fn normalize_path_prefix(raw_path: &str) -> String {
    if raw_path.is_empty() || raw_path == "/" {
        return "/".to_owned();
    }
    let mut normalized = if raw_path.starts_with('/') {
        raw_path.to_owned()
    } else {
        format!("/{raw_path}")
    };
    if !normalized.ends_with('/') {
        normalized.push('/');
    }
    normalized
}

fn build_auth_entry(
    setting: &str,
    raw_value: &str,
    line_no: usize,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<AuthEntry, NpmrcError> {
    if raw_value.is_empty() {
        return Err(NpmrcError::EmptyAuthValue { line: line_no });
    }

    let mut out = Zeroizing::new(String::new());
    let mut missing: Option<String> = None;
    let mut rest = raw_value;

    while let Some(pos) = rest.find("${") {
        if missing.is_none() {
            out.push_str(&rest[..pos]);
        }
        let after = &rest[pos + 2..];
        let Some(end) = after.find('}') else {
            return Err(NpmrcError::InvalidEnvVarName { line: line_no });
        };
        let var_name = &after[..end];
        if !is_valid_env_var_name(var_name) {
            return Err(NpmrcError::InvalidEnvVarName { line: line_no });
        }
        match env_lookup(var_name) {
            Some(val) => {
                let val = Zeroizing::new(val);
                if missing.is_none() {
                    out.push_str(&val);
                }
            }
            None => {
                if missing.is_none() {
                    missing = Some(var_name.to_owned());
                }
            }
        }
        rest = &after[end + 1..];
    }

    if let Some(var_name) = missing {
        return Ok(AuthEntry::MissingEnvVar { var_name });
    }

    out.push_str(rest);

    if out.is_empty() {
        return Err(NpmrcError::EmptyAuthValue { line: line_no });
    }
    if out.bytes().any(|b| !b.is_ascii_graphic()) {
        return Err(NpmrcError::InvalidAuthValue {
            line: line_no,
            reason: "auth credential must contain only visible ASCII characters".to_owned(),
        });
    }

    let secret = RedactedSecret::new(std::mem::take(&mut *out));
    let auth = match setting {
        "_authToken" => RegistryAuth::Bearer(secret),
        "_auth" => RegistryAuth::Basic(secret),
        _ => unreachable!("caller checks setting"),
    };
    Ok(AuthEntry::Ready(auth))
}

fn is_auth_directive_key(key: &str) -> bool {
    if key.starts_with("//") {
        return true;
    }
    let setting = key.rsplit_once(':').map_or(key, |(_, s)| s);
    setting.eq_ignore_ascii_case("_authToken")
        || setting.eq_ignore_ascii_case("_auth")
        || setting.eq_ignore_ascii_case("always-auth")
        || setting.eq_ignore_ascii_case("_password")
        || setting.eq_ignore_ascii_case("username")
        || setting.eq_ignore_ascii_case("certfile")
        || setting.eq_ignore_ascii_case("keyfile")
}

fn is_valid_env_var_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_to_npmjs_registry_without_files() {
        let cfg = NpmrcConfig::default();
        assert!(!cfg.has_explicit_default_registry());
        assert_eq!(
            cfg.default_registry().as_str(),
            "https://registry.npmjs.org/"
        );
        assert_eq!(
            cfg.registry_for_package("lodash").as_str(),
            "https://registry.npmjs.org/"
        );
        assert_eq!(
            cfg.registry_for_package("@acme/pkg").as_str(),
            "https://registry.npmjs.org/"
        );
    }

    #[test]
    fn parses_comments_blank_lines_quotes_and_scoped_registries() {
        let raw = r#"
            # leading comment
            ; semicolon comment

            save-exact
            ignore-scripts = true
            registry = "https://mirror.example.com/npm" # inline comment
            @MyOrg:registry = 'https://org.example.com/registry/' ; inline comment
        "#;
        let cfg = NpmrcConfig::parse(raw, &no_env).unwrap();
        assert!(cfg.has_explicit_default_registry());
        assert_eq!(
            cfg.registry_for_package("lodash").as_str(),
            "https://mirror.example.com/npm/"
        );
        assert_eq!(
            cfg.registry_for_package("@myorg/pkg").as_str(),
            "https://org.example.com/registry/"
        );
        assert_eq!(
            cfg.registry_for_package("@MYORG/pkg").as_str(),
            "https://org.example.com/registry/"
        );
    }

    #[test]
    fn longest_path_prefix_and_exact_host_port_auth_matching() {
        let raw = r#"
            //registry.example.com/:_authToken=root_tok
            //registry.example.com/team/:_authToken=team_tok
            //registry.example.com:8443/team/:_auth=dXNlcjpwYXNz
        "#;
        let cfg = NpmrcConfig::parse(raw, &no_env).unwrap();

        let root_url = Url::parse("https://registry.example.com/public/pkg").unwrap();
        assert_eq!(
            cfg.auth_for_url(&root_url)
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Bearer root_tok"
        );

        let team_url = Url::parse("https://registry.example.com/team/@scope%2fpkg").unwrap();
        assert_eq!(
            cfg.auth_for_url(&team_url)
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Bearer team_tok"
        );

        // Path prefix must respect segment boundary: `/team-evil/` must NOT match `/team/`.
        let evil_prefix = Url::parse("https://registry.example.com/team-evil/pkg").unwrap();
        assert_eq!(
            cfg.auth_for_url(&evil_prefix)
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Bearer root_tok"
        );

        // Explicit port 8443 matches port 8443 only.
        let port_url = Url::parse("https://registry.example.com:8443/team/pkg").unwrap();
        assert_eq!(
            cfg.auth_for_url(&port_url)
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Basic dXNlcjpwYXNz"
        );

        // Host suffix spoofing must NEVER match.
        let spoofed = Url::parse("https://evil-registry.example.com/team/pkg").unwrap();
        assert!(cfg.auth_for_url(&spoofed).unwrap().is_none());
        let sub = Url::parse("https://sub.registry.example.com/team/pkg").unwrap();
        assert!(cfg.auth_for_url(&sub).unwrap().is_none());
    }

    #[test]
    fn env_interpolation_lazy_missing_and_validation() {
        let raw = r#"
            //private.example.com/:_authToken=${PRIVATE_TOKEN}
            //basic.example.com/:_auth=${BASIC_CREDS}
        "#;
        let cfg = NpmrcConfig::parse(raw, &|name| match name {
            "BASIC_CREDS" => Some("dXNlcjpwYXNz".to_owned()),
            _ => None,
        })
        .unwrap();

        // Unrelated public registry succeeds with Ok(None).
        let pub_url = Url::parse("https://registry.npmjs.org/lodash").unwrap();
        assert!(cfg.auth_for_url(&pub_url).unwrap().is_none());

        // Configured basic registry succeeds.
        let basic_url = Url::parse("https://basic.example.com/pkg").unwrap();
        assert_eq!(
            cfg.auth_for_url(&basic_url)
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Basic dXNlcjpwYXNz"
        );

        // Missing env var fails closed ONLY when matched.
        let priv_url = Url::parse("https://private.example.com/@priv%2fpkg").unwrap();
        let err = cfg.auth_for_url(&priv_url).unwrap_err();
        assert_eq!(
            err,
            NpmrcError::MissingAuthEnvVar {
                var_name: "PRIVATE_TOKEN".to_owned()
            }
        );

        // Invalid env var names are rejected at parse time (zero shell evaluation).
        for bad in [
            "//a.example.com/:_authToken=${1BAD}",
            "//a.example.com/:_authToken=${BAD-NAME}",
            "//a.example.com/:_authToken=${}",
            "//a.example.com/:_authToken=${UNCLOSED",
        ] {
            let err = NpmrcConfig::parse(bad, &no_env).unwrap_err();
            assert!(matches!(err, NpmrcError::InvalidEnvVarName { .. }), "{err}");
        }
    }

    #[test]
    fn rejects_plaintext_http_for_registries_and_auth() {
        let err =
            NpmrcConfig::parse("@priv:registry=http://registry.example.com/", &no_env).unwrap_err();
        assert!(
            matches!(err, NpmrcError::InsecureRegistryUrl { .. }),
            "{err}"
        );

        let err_default =
            NpmrcConfig::parse("registry=http://registry.example.com/", &no_env).unwrap_err();
        assert!(
            matches!(err_default, NpmrcError::InsecureRegistryUrl { .. }),
            "{err_default}"
        );

        let cfg =
            NpmrcConfig::parse("//registry.example.com/:_authToken=supersecret", &no_env).unwrap();
        let http_url = Url::parse("http://registry.example.com/@priv%2fpkg").unwrap();
        assert_eq!(
            cfg.auth_for_url(&http_url),
            Err(NpmrcError::InsecureAuthUrl)
        );

        // Unmatched http:// URL never returns credentials.
        let other_http = Url::parse("http://other.example.com/pkg").unwrap();
        assert_eq!(cfg.auth_for_url(&other_http), Ok(None));
    }

    #[test]
    fn redacts_secrets_in_debug_and_errors() {
        let secret = "npm_SuperSecretToken_987654321";
        let basic = "dXNlcjpzdXBlcnNlY3JldA==";
        let raw = format!("//a.example.com/:_authToken={secret}\n//b.example.com/:_auth={basic}\n");
        let cfg = NpmrcConfig::parse(&raw, &no_env).unwrap();
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains(secret), "secret leaked in NpmrcConfig Debug");
        assert!(!dbg.contains(basic), "basic leaked in NpmrcConfig Debug");
        assert!(dbg.contains("<redacted>"));

        let auth = cfg
            .auth_for_url(&Url::parse("https://a.example.com/x").unwrap())
            .unwrap()
            .unwrap();
        let auth_dbg = format!("{auth:?}");
        assert!(!auth_dbg.contains(secret));
        assert!(auth_dbg.contains("<redacted>"));

        // Invalid value with CR/LF, non-ASCII, or DEL must not echo secret in error.
        let crlf = format!("//a.example.com/:_authToken=\"{secret}\\r\\nX-Inject: 1\"");
        let err = NpmrcConfig::parse(&crlf, &no_env).unwrap_err();
        assert!(!err.to_string().contains(secret));
        assert!(!format!("{err:?}").contains(secret));

        for bad_val in [
            format!("//a.example.com/:_authToken=\"{secret}é\""),
            format!("//a.example.com/:_authToken=\"{secret}\x7f\""),
            format!("//a.example.com/:_authToken=\"{secret}\\t\""),
        ] {
            let err = NpmrcConfig::parse(&bad_val, &no_env).unwrap_err();
            assert!(matches!(err, NpmrcError::InvalidAuthValue { .. }), "{err}");
            assert!(!err.to_string().contains(secret));
        }
    }

    #[test]
    fn load_for_project_merges_user_and_project_and_rejects_symlinks() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();

        std::fs::write(
            home.path().join(".npmrc"),
            "registry=https://user-mirror.example.com/\n@user:registry=https://user.example.com/\n//user.example.com/:_authToken=${USER_TOK}\n",
        )
        .unwrap();
        // Project .npmrc may override routing only; auth must come from ~/.npmrc.
        std::fs::write(
            project.path().join(".npmrc"),
            "@user:registry=https://proj.example.com/\n",
        )
        .unwrap();

        let cfg = NpmrcConfig::load_for_project(project.path(), Some(home.path()), &|name| {
            (name == "USER_TOK").then(|| "user_tok".to_owned())
        })
        .unwrap();
        // Project without `registry=` preserves user's `registry=`.
        assert_eq!(
            cfg.registry_for_package("lodash").as_str(),
            "https://user-mirror.example.com/"
        );
        // Project overrides user's `@user:registry`, while user ~/.npmrc still provides auth.
        assert_eq!(
            cfg.registry_for_package("@user/pkg").as_str(),
            "https://proj.example.com/"
        );
        assert_eq!(
            cfg.auth_for_url(&Url::parse("https://user.example.com/x").unwrap())
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Bearer user_tok"
        );

        // Missing files silently skipped.
        let empty = tempfile::tempdir().unwrap();
        let default_cfg = NpmrcConfig::load_for_project(empty.path(), None, &no_env).unwrap();
        assert_eq!(
            default_cfg.default_registry().as_str(),
            "https://registry.npmjs.org/"
        );

        // Directory .npmrc fails closed.
        let dir_proj = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir_proj.path().join(".npmrc")).unwrap();
        let dir_err = NpmrcConfig::load_for_project(dir_proj.path(), None, &no_env).unwrap_err();
        assert_eq!(dir_err, NpmrcError::NotRegularFile);

        // Symlinked .npmrc fails closed via symlink_metadata + O_NOFOLLOW on Unix.
        #[cfg(unix)]
        {
            let sym_proj = tempfile::tempdir().unwrap();
            let target = sym_proj.path().join("real.npmrc");
            std::fs::write(&target, "registry=https://example.com/\n").unwrap();
            std::os::unix::fs::symlink(&target, sym_proj.path().join(".npmrc")).unwrap();
            let err = NpmrcConfig::load_for_project(sym_proj.path(), None, &no_env).unwrap_err();
            assert_eq!(err, NpmrcError::NotRegularFile);
        }
    }

    #[test]
    fn project_npmrc_rejects_auth_directives_and_env_interpolation_by_default() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".npmrc"),
            "//user.example.com/:_authToken=${USER_SECRET}\n",
        )
        .unwrap();

        for malicious in [
            "registry=https://evil.test/\n//evil.test/:_authToken=${SECRET}\n",
            "//evil.test/:_auth=${SECRET}\n",
            "_authToken=${SECRET}\n",
            "always-auth=true\n",
            "always-auth\n",
            "registry=https://${SECRET}.evil.test/\n",
        ] {
            let project = tempfile::tempdir().unwrap();
            std::fs::write(project.path().join(".npmrc"), malicious).unwrap();
            let secret_read = AtomicBool::new(false);
            let err = NpmrcConfig::load_for_project(project.path(), Some(home.path()), &|name| {
                if name == "SECRET" {
                    secret_read.store(true, Ordering::SeqCst);
                    return Some("super_secret_value".to_owned());
                }
                (name == "USER_SECRET").then(|| "user_ok".to_owned())
            })
            .unwrap_err();
            assert!(
                matches!(err, NpmrcError::ProjectAuthNotAllowed { .. }),
                "expected ProjectAuthNotAllowed for `{malicious}`, got {err}"
            );
            assert!(
                !secret_read.load(Ordering::SeqCst),
                "env_lookup must NEVER be called for project .npmrc: {malicious}"
            );
        }

        // Skipping project .npmrc (`None`) loads ~/.npmrc only.
        let skipped = NpmrcConfig::load_for_project_with_options(
            None,
            Some(home.path()),
            &|name| (name == "USER_SECRET").then(|| "user_ok".to_owned()),
            false,
        )
        .unwrap();
        assert_eq!(
            skipped
                .auth_for_url(&Url::parse("https://user.example.com/pkg").unwrap())
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Bearer user_ok"
        );

        // Explicit trusted opt-in allows project auth when requested.
        let opted_in_proj = tempfile::tempdir().unwrap();
        std::fs::write(
            opted_in_proj.path().join(".npmrc"),
            "//trusted.test/:_authToken=trusted_tok\n",
        )
        .unwrap();
        let opted_in = NpmrcConfig::load_for_project_with_options(
            Some(opted_in_proj.path()),
            None,
            &no_env,
            true,
        )
        .unwrap();
        assert_eq!(
            opted_in
                .auth_for_url(&Url::parse("https://trusted.test/pkg").unwrap())
                .unwrap()
                .unwrap()
                .authorization_header_value()
                .as_str(),
            "Bearer trusted_tok"
        );
    }
}
