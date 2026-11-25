//! Native `jsr:` module resolution, registry contract, and offline cache
//! registry.
//!
//! This module is the engine-neutral core of `jsr:` support: strict specifier
//! parsing, registry-metadata parsing, version selection, entrypoint
//! resolution, per-file size and checksum verification, canonical `jsr.io` URL
//! helpers, the [`Registry`] trait, [`RemoteRegistry`], and the offline
//! [`CacheRegistry`]. It has no JavaScript engine dependency; `vaeljs-core`
//! wires it to `deno_core::ModuleLoader` in its `jsr_loader` adapter.
//!
//! # Scope
//!
//! The parser accepts exactly `jsr:@scope/name[@constraint][/subpath]`.
//! Scopes are 2 to 20 characters and package names 2 to 58 characters, each
//! lowercase ASCII letters, digits, or hyphens, and neither may start with a
//! hyphen. These are the registry's published rules.
//!
//! Subpaths and export targets are registry paths, not filesystem paths. The
//! parser rejects percent escapes, backslashes, query and fragment markers,
//! control characters, empty segments, `.`, `..`, and `*`. It never decodes a
//! percent escape: a `%` is rejected instead of being interpreted, so `%2e%2e`
//! cannot become `..`.
//!
//! # SemVer
//!
//! A bare full version such as `1.2.3` is an exact version, as the JSR docs
//! promise. That differs from Rust's `semver` crate requirement grammar, where
//! `1.2.3` is caret-compatible, so this module parses a bare `Version` first.
//! `1`, `1.0`, `^1.2.3`, `~1.2.3`, `*`, and the other `semver` requirements
//! are ranges. A bare `1.2` is the registry's minor-line range
//! (`>=1.2.0, <1.3.0`), not Rust `semver`'s caret-compatible default, so a
//! bare partial minor version is normalized to an explicit tilde requirement.
//! Prereleases are excluded by range matching unless the requirement names a
//! compatible prerelease; an exact prerelease is accepted. No dist-tags are
//! supported because the registry API documents no tag endpoint, so `latest`
//! is rejected as a non-SemVer requirement.
//!
//! Yanked versions are skipped by default and range resolution. An exact
//! version is still selectable when it is yanked, matching the documented rule
//! that an explicit import can use a yanked version; the result reports
//! `yanked` so a later caller can warn. Default resolution, with no constraint,
//! selects the greatest non-yanked stable version.
//!
//! # Metadata and integrity
//!
//! Package metadata (`meta.json`) supplies published versions and yank state.
//! Version metadata (`<version>_meta.json`) supplies the file manifest and
//! normalized exports. The document has no package or version field, so it
//! cannot prove its own identity. [`VersionMetadata::parse`] therefore takes
//! the [`PackageVersion`] the caller fetched it for and stores that binding;
//! [`resolve_module`] rejects metadata whose binding does not match the version
//! selected from package metadata.
//!
//! [`VersionMetadata::verify_file`] checks a file's byte length and
//! `sha256-<lowercase hex>` checksum against the manifest before a later loader
//! executes those bytes. That verifies downloaded content against the retrieved
//! metadata; it does not establish publisher identity and is not an
//! authentication boundary for the local cache (an attacker who consistently
//! rewrites both a cached manifest and a cached body can keep them matching).
//!
//! # Limits
//!
//! One specifier is at most [`MAX_SPECIFIER_BYTES`], one constraint at most
//! [`MAX_CONSTRAINT_BYTES`] and [`MAX_COMPARATORS`] comparators, one metadata
//! document at most [`MAX_METADATA_BYTES`], and one package at most
//! [`MAX_VERSIONS`], [`MAX_MANIFEST_ENTRIES`], and [`MAX_EXPORTS`] entries.
//! Path text after its required prefix is at most [`MAX_PATH_BYTES`].

use std::collections::BTreeMap;
use std::fmt;

use semver::{Version, VersionReq};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::remote::{Cache, DEFAULT_MAX_MODULE_BYTES, RemoteClient, RemoteError};

/// The public registry origin used in canonical module URLs.
pub const REGISTRY_ORIGIN: &str = "https://jsr.io";

/// The public registry host.
pub const REGISTRY_HOST: &str = "jsr.io";

/// Maximum bytes in one `jsr:` specifier, including the scheme.
pub const MAX_SPECIFIER_BYTES: usize = 2 * 1024;

/// Maximum bytes in one version constraint.
pub const MAX_CONSTRAINT_BYTES: usize = 128;

/// Maximum comma-separated comparators in one version constraint.
pub const MAX_COMPARATORS: usize = 8;

/// Minimum characters in a scope name.
pub const MIN_SCOPE_CHARS: usize = 2;

/// Maximum characters in a scope name.
pub const MAX_SCOPE_CHARS: usize = 20;

/// Minimum characters in a package name.
pub const MIN_NAME_CHARS: usize = 2;

/// Maximum characters in a package name.
pub const MAX_NAME_CHARS: usize = 58;

/// Maximum bytes in one subpath.
pub const MAX_SUBPATH_BYTES: usize = 512;

/// Maximum bytes in one metadata JSON document.
pub const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;

/// Maximum versions listed by one package metadata document.
pub const MAX_VERSIONS: usize = 10_000;

/// Maximum files listed by one version manifest.
pub const MAX_MANIFEST_ENTRIES: usize = 10_000;

/// Maximum entrypoints listed by one version metadata document.
pub const MAX_EXPORTS: usize = 1_000;

/// Maximum bytes in a manifest path, export key, or export target after its
/// required prefix.
pub const MAX_PATH_BYTES: usize = 1024;

/// Resource bounds for one runtime's JSR module graph.
///
/// The defaults are deliberately modest. They bound one runtime's work; they
/// are not a network deadline, a CPU budget, or a sandbox.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum declared and downloaded bytes in one file.
    pub max_file_bytes: usize,
    /// Maximum distinct files in one module graph.
    pub max_graph_files: usize,
    /// Maximum declared and downloaded bytes across one module graph.
    pub max_graph_bytes: usize,
    /// Maximum distinct package and version metadata documents fetched.
    pub max_metadata_documents: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_graph_files: 256,
            max_graph_bytes: 32 * 1024 * 1024,
            max_metadata_documents: 128,
        }
    }
}

/// A failure while parsing or resolving JSR metadata.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A `jsr:` specifier is malformed.
    #[error("invalid jsr: specifier: {0}")]
    Specifier(String),
    /// A version constraint is not a supported SemVer requirement.
    #[error("invalid jsr: version constraint: {0}")]
    Constraint(String),
    /// Package or version metadata is malformed.
    #[error("invalid JSR metadata: {0}")]
    Metadata(String),
    /// No published version can satisfy the request.
    #[error("cannot resolve JSR version: {0}")]
    Version(String),
    /// The requested entrypoint does not exist.
    #[error("cannot resolve JSR entrypoint: {0}")]
    Export(String),
    /// A file is not listed in the version manifest.
    #[error("invalid JSR manifest: {0}")]
    Manifest(String),
    /// A file's length or checksum does not match its manifest entry.
    #[error("JSR file verification failed: {0}")]
    Integrity(String),
}

/// A failure a registry transport or cache reports to a JSR loader.
///
/// The loader adds the package, file, and importer context. A transport must
/// follow redirects itself and must not put credentials or other secrets in a
/// [`Self::Redirect`] location or an [`Self::Unavailable`] reason; both are
/// shown to the program and the user.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// The registry (or offline cache) has no such package, version, or file.
    #[error("the registry document was not found")]
    NotFound,
    /// The transport returned a redirect without following it.
    ///
    /// [`RemoteRegistry`] lets the remote client follow checked redirects and
    /// never returns this. It remains part of the [`Registry`] contract so
    /// another registry can report a redirect the loader must not follow.
    #[error("the registry transport redirected to `{0}`")]
    Redirect(String),
    /// The transport or cache could not complete the request for another reason.
    #[error("{0}")]
    Unavailable(String),
}

/// A synchronous source of JSR registry metadata and file bytes.
pub trait Registry {
    /// Returns the `meta.json` document for a package.
    fn package_metadata(&self, package: &PackageId) -> Result<String, RegistryError>;

    /// Returns the `<version>_meta.json` document for an exact package version.
    fn version_metadata(&self, package: &PackageVersion) -> Result<String, RegistryError>;

    /// Returns one manifest file's bytes.
    fn file(&self, package: &PackageVersion, path: &str) -> Result<Vec<u8>, RegistryError>;
}

/// A [`Registry`] backed by the opt-in HTTPS transport and cache.
///
/// The registry builds every URL from a validated [`PackageId`],
/// [`PackageVersion`], or manifest path. It never accepts a URL or a path from
/// JavaScript, so a `jsr:` specifier's subpath cannot select another origin or
/// another package version. The remote client owns redirects, TLS, deadlines,
/// credentials policy, and the cache.
pub struct RemoteRegistry {
    remote: RemoteClient,
}

impl RemoteRegistry {
    /// Creates a registry over an existing remote client.
    ///
    /// The client carries the runtime's request, module, byte, redirect, and
    /// deadline limits and its user-profile cache.
    pub fn new(remote: RemoteClient) -> Self {
        Self { remote }
    }

    /// Fetches one registry document and returns its body.
    fn fetch(&self, url: Url) -> Result<Vec<u8>, RegistryError> {
        match self.remote.fetch(&url) {
            Ok(fetched) => Ok(fetched.body),
            Err(RemoteError::HttpStatus {
                status: 404 | 410, ..
            }) => Err(RegistryError::NotFound),
            Err(error) => Err(RegistryError::Unavailable(error.to_string())),
        }
    }
}

impl Registry for RemoteRegistry {
    fn package_metadata(&self, package: &PackageId) -> Result<String, RegistryError> {
        let subject = package.to_string();
        let url = package_metadata_url(package)?;
        let bytes = self.fetch(url)?;
        decode_text(bytes, "package metadata", &subject)
    }

    fn version_metadata(&self, package: &PackageVersion) -> Result<String, RegistryError> {
        let subject = package.to_string();
        let url = version_metadata_url(package)?;
        let bytes = self.fetch(url)?;
        decode_text(bytes, "version metadata", &subject)
    }

    fn file(&self, package: &PackageVersion, path: &str) -> Result<Vec<u8>, RegistryError> {
        let url = file_url(package, path)?;
        self.fetch(url)
    }
}

/// An offline [`Registry`] backed solely by the disk [`Cache`].
///
/// Reads use [`Cache::load_offline_with_limit`], which takes no write locks,
/// ignores `max-age` expiry, verifies SHA-256 checksums, honors `.tomb`
/// tombstones, and follows bounded `.redir` redirect pointers. It performs zero
/// network requests and compiles without the `network` feature.
#[derive(Clone, Debug)]
pub struct CacheRegistry {
    cache: Cache,
    max_file_bytes: usize,
}

impl CacheRegistry {
    /// Creates an offline registry over `cache` with a per-file cap of
    /// [`DEFAULT_MAX_MODULE_BYTES`] (16 MiB).
    ///
    /// Absent, tombstoned, corrupt, or oversized entries return
    /// [`RegistryError::NotFound`]. Invalid URLs, redirect loops, and excess
    /// redirect hops return [`RegistryError::Unavailable`].
    pub fn new(cache: Cache) -> Self {
        Self {
            cache,
            max_file_bytes: DEFAULT_MAX_MODULE_BYTES,
        }
    }

    /// Creates an offline registry over `cache` with an explicit
    /// `max_file_bytes` cap in bytes.
    ///
    /// Cached metadata documents or files larger than `max_file_bytes` bytes,
    /// as well as absent, tombstoned, or corrupt entries, return
    /// [`RegistryError::NotFound`]. Redirect loops and excess hops return
    /// [`RegistryError::Unavailable`].
    pub fn with_max_file_bytes(cache: Cache, max_file_bytes: usize) -> Self {
        Self {
            cache,
            max_file_bytes,
        }
    }

    fn load(&self, url: Url) -> Result<Vec<u8>, RegistryError> {
        match self
            .cache
            .load_offline_with_limit(&url, self.max_file_bytes)
        {
            Ok(Some(hit)) => Ok(hit.body),
            Ok(None) => Err(RegistryError::NotFound),
            Err(error) => Err(RegistryError::Unavailable(error.to_string())),
        }
    }
}

impl Registry for CacheRegistry {
    fn package_metadata(&self, package: &PackageId) -> Result<String, RegistryError> {
        let subject = package.to_string();
        let url = package_metadata_url(package)?;
        let bytes = self.load(url)?;
        decode_text(bytes, "package metadata", &subject)
    }

    fn version_metadata(&self, package: &PackageVersion) -> Result<String, RegistryError> {
        let subject = package.to_string();
        let url = version_metadata_url(package)?;
        let bytes = self.load(url)?;
        decode_text(bytes, "version metadata", &subject)
    }

    fn file(&self, package: &PackageVersion, path: &str) -> Result<Vec<u8>, RegistryError> {
        let url = file_url(package, path)?;
        self.load(url)
    }
}

/// Builds the `meta.json` URL for a validated [`PackageId`].
pub fn package_metadata_url(package: &PackageId) -> Result<Url, RegistryError> {
    registry_url(
        format!("{REGISTRY_ORIGIN}/{package}/meta.json"),
        "package metadata",
        &package.to_string(),
    )
}

/// Builds the `<version>_meta.json` URL for a validated [`PackageVersion`].
pub fn version_metadata_url(package: &PackageVersion) -> Result<Url, RegistryError> {
    registry_url(
        format!(
            "{REGISTRY_ORIGIN}/{}/{}_meta.json",
            package.package, package.version
        ),
        "version metadata",
        &package.to_string(),
    )
}

/// Builds the canonical file URL for a validated [`PackageVersion`] and
/// manifest path.
pub fn file_url(package: &PackageVersion, path: &str) -> Result<Url, RegistryError> {
    validate_manifest_path(path).map_err(RegistryError::Unavailable)?;
    registry_url(
        format!(
            "{REGISTRY_ORIGIN}/{}/{}{}",
            package.package,
            package.version,
            encode_path(path)
        ),
        "file",
        &format!("{package}:{path}"),
    )
}

fn registry_url(text: String, action: &str, subject: &str) -> Result<Url, RegistryError> {
    Url::parse(&text).map_err(|error| {
        RegistryError::Unavailable(format!(
            "cannot build the {action} URL for `{subject}`: {error}"
        ))
    })
}

fn decode_text(bytes: Vec<u8>, action: &str, subject: &str) -> Result<String, RegistryError> {
    String::from_utf8(bytes).map_err(|_| {
        RegistryError::Unavailable(format!("the {action} for `{subject}` is not valid UTF-8"))
    })
}

/// One resolved module identity plus its manifest path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleKey {
    /// The package and exact version.
    pub id: PackageVersion,
    /// The leading-`/` manifest path.
    pub path: String,
}

/// Returns whether a specifier is a relative URL reference.
pub fn is_relative_reference(specifier: &str) -> bool {
    specifier == "."
        || specifier == ".."
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
        || specifier.starts_with('\\')
        || specifier.starts_with(".\\")
        || specifier.starts_with("..\\")
}

/// Returns why a registry module cannot import a local-file URL.
///
/// A JSR module may import another registry module or a vaeljs built-in. It may
/// not reach the local filesystem through `file:`; that keeps the module graph
/// on the registry and avoids turning a registry package into a reader for the
/// user's files. This is a module-graph boundary, not a sandbox.
pub fn local_file_reason(specifier: &str) -> Option<&'static str> {
    let Ok(url) = Url::parse(specifier) else {
        return None;
    };
    if url.scheme() == "file" {
        return Some(
            "a JSR module cannot import a local file; only registry modules and built-in modules are allowed",
        );
    }
    None
}

/// Builds the canonical URL for one resolved module.
pub fn canonical_module_url(resolved: &ResolvedModule) -> Result<Url, String> {
    let file = resolved
        .target
        .path
        .strip_prefix('/')
        .ok_or_else(|| "the manifest path has no leading `/`".to_owned())?;
    let text = format!(
        "{REGISTRY_ORIGIN}/{}/{}/{}",
        resolved.package,
        resolved.version,
        encode_path(file)
    );
    let url = Url::parse(&text)
        .map_err(|error| format!("cannot build the module URL for `{resolved}`: {error}"))?;
    let key = canonical_url_for(&ModuleKey {
        id: PackageVersion {
            package: resolved.package.clone(),
            version: resolved.version.clone(),
        },
        path: resolved.target.path.clone(),
    })?;
    if key != url {
        return Err(format!(
            "the module URL for `{resolved}` did not round-trip through its identity"
        ));
    }
    Ok(url)
}

/// Builds the canonical URL for a parsed module identity.
pub fn canonical_url_for(module: &ModuleKey) -> Result<Url, String> {
    let file = module
        .path
        .strip_prefix('/')
        .ok_or_else(|| "the manifest path has no leading `/`".to_owned())?;
    Url::parse(&format!(
        "{REGISTRY_ORIGIN}/{}/{}/{}",
        module.id.package,
        module.id.version,
        encode_path(file)
    ))
    .map_err(|error| format!("cannot build the canonical module URL: {error}"))
}

/// Returns the identities a registry URL can be registered under.
///
/// A canonical module URL is represented by its parsed string and its
/// re-encoded canonical form. Checking both prevents an ordinary redirect and a
/// `jsr:` resolution from disagreeing only because they encoded the path
/// differently.
pub fn registry_identities(url: &Url) -> Vec<String> {
    let mut identities = vec![url.to_string()];
    if let Ok(Some(module)) = parse_module_url(url)
        && let Ok(canonical) = canonical_url_for(&module)
    {
        let canonical = canonical.to_string();
        if canonical != identities[0] {
            identities.push(canonical);
        }
    }
    identities
}

/// Returns registry identities for a URL only when it is on the public origin.
pub fn registry_identities_if_registry(url: &Url) -> Vec<String> {
    if url.scheme() == "https" && url.host_str() == Some(REGISTRY_HOST) {
        registry_identities(url)
    } else {
        Vec::new()
    }
}

/// Parses and validates one canonical registry URL.
///
/// Returns `None` when the URL is not on the registry origin, so the caller can
/// delegate it. A malformed module path on the registry origin is an error,
/// never a silent fallthrough.
pub fn parse_module_url(url: &Url) -> Result<Option<ModuleKey>, String> {
    if url.scheme() != "https"
        || url.host_str() != Some(REGISTRY_HOST)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Ok(None);
    }
    let Some(rest) = url.path().strip_prefix('/') else {
        return Ok(None);
    };
    let mut segments = rest.split('/');
    let Some(scope_segment) = segments.next() else {
        return Ok(None);
    };
    let Some(scope) = scope_segment.strip_prefix('@') else {
        return Ok(None);
    };
    let Some(name) = segments.next() else {
        return Ok(None);
    };
    let Some(version_text) = segments.next() else {
        return Ok(None);
    };
    let encoded_file = segments.collect::<Vec<_>>().join("/");
    if encoded_file.is_empty() {
        return Err("the module URL names no file".to_owned());
    }
    let package = PackageId::new(scope, name)?;
    let version = Version::parse(version_text)
        .map_err(|_| format!("`{version_text}` is not a valid version"))?;
    if version.to_string() != version_text {
        return Err(format!("`{version_text}` is not canonical SemVer"));
    }
    let file = decode_path(&encoded_file)?;
    let path = format!("/{file}");
    validate_manifest_path(&path)?;
    Ok(Some(ModuleKey {
        id: PackageVersion { package, version },
        path,
    }))
}

/// Encodes a registry path for a URL, preserving `/` as a separator.
///
/// The encoder is deliberately small and paired with [`decode_path`]. It does
/// not decode arbitrary percent escapes from user input: a `%` in a specifier
/// or a relative import is rejected before this point.
fn encode_path(value: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                encoded.push(char::from(byte))
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

/// Decodes the encoder's output back to a UTF-8 registry path.
fn decode_path(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(high) = bytes.get(index + 1).and_then(|byte| hex_value(*byte)) else {
                return Err("the module path has a malformed percent escape".to_owned());
            };
            let Some(low) = bytes.get(index + 2).and_then(|byte| hex_value(*byte)) else {
                return Err("the module path has a malformed percent escape".to_owned());
            };
            let value = high << 4 | low;
            if matches!(value, b'/' | b'\\') {
                return Err("the module path percent-encodes a path separator".to_owned());
            }
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "the module path is not valid UTF-8".to_owned())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// A validated JSR package identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageId {
    /// The scope without the leading `@`.
    pub scope: String,
    /// The package name without a scope prefix.
    pub name: String,
}

impl PackageId {
    /// Validates a scope and package name and builds an identity.
    ///
    /// The names use the registry's published rules: lowercase ASCII letters,
    /// digits, and hyphens, with the documented length limits and no leading
    /// hyphen. A caller that already has a validated [`PackageId`] must not
    /// re-parse it through untrusted text.
    pub fn new(scope: &str, name: &str) -> Result<Self, String> {
        validate_scope(scope)?;
        validate_package_name(name)?;
        Ok(Self {
            scope: scope.to_owned(),
            name: name.to_owned(),
        })
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "@{}/{}", self.scope, self.name)
    }
}

/// A validated package identity at one exact version.
///
/// This is the identity that binds version metadata to what was fetched. A
/// `PackageVersion` is constructed from a validated [`PackageId`] and an exact
/// parsed [`Version`]; it is not parsed from untrusted text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageVersion {
    /// The package.
    pub package: PackageId,
    /// The exact version.
    pub version: Version,
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.package, self.version)
    }
}

/// A validated package subpath, stored without its leading `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subpath(String);

impl Subpath {
    /// Returns the subpath text without its leading `/`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the package export key for this subpath.
    pub fn export_key(&self) -> String {
        format!("./{}", self.0)
    }

    fn parse(raw: &str) -> Result<Self, Error> {
        if raw.is_empty() {
            return Err(Error::Specifier(
                "the subpath after `/` is empty".to_owned(),
            ));
        }
        if raw.len() > MAX_SUBPATH_BYTES {
            return Err(Error::Specifier(format!(
                "the subpath is {} bytes, over the {MAX_SUBPATH_BYTES}-byte limit",
                raw.len()
            )));
        }
        validate_path_text(raw, "subpath").map_err(Error::Specifier)?;
        Ok(Self(raw.to_owned()))
    }
}

/// A parsed and validated `jsr:@scope/name[@constraint][/subpath]` specifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Specifier {
    /// The package the specifier names.
    pub package: PackageId,
    /// The version constraint, or no constraint when one was omitted.
    pub constraint: VersionConstraint,
    /// The optional entrypoint subpath.
    pub subpath: Option<Subpath>,
}

impl Specifier {
    /// Parses one `jsr:` specifier.
    ///
    /// The scheme is matched case-sensitively and must be `jsr:`. The package
    /// must be scoped. A constraint starts at the first `@` after the package
    /// name and ends at the next `/`; a subpath is everything after that
    /// separator. No query, fragment, whitespace trimming, or percent decoding
    /// is performed.
    pub fn parse(input: &str) -> Result<Self, Error> {
        if input.len() > MAX_SPECIFIER_BYTES {
            return Err(Error::Specifier(format!(
                "the specifier is {} bytes, over the {MAX_SPECIFIER_BYTES}-byte limit",
                input.len()
            )));
        }
        let Some(after_scheme) = input.strip_prefix("jsr:") else {
            return Err(Error::Specifier(
                "the specifier must start with `jsr:`".to_owned(),
            ));
        };
        if after_scheme.is_empty() {
            return Err(Error::Specifier(
                "the specifier names no package".to_owned(),
            ));
        }
        let Some(package_rest) = after_scheme.strip_prefix('@') else {
            return Err(Error::Specifier(
                "the package must be scoped, as `jsr:@scope/name`".to_owned(),
            ));
        };
        let Some((scope, name_and_rest)) = package_rest.split_once('/') else {
            return Err(Error::Specifier(
                "the package must include both a scope and a name, as `@scope/name`".to_owned(),
            ));
        };
        validate_scope(scope).map_err(Error::Specifier)?;

        let boundary = name_and_rest
            .find(['@', '/'])
            .unwrap_or(name_and_rest.len());
        let name = &name_and_rest[..boundary];
        validate_package_name(name).map_err(Error::Specifier)?;

        let suffix = &name_and_rest[boundary..];
        let (constraint, subpath) = if suffix.is_empty() {
            (VersionConstraint::unconstrained(), None)
        } else if let Some(constraint_and_path) = suffix.strip_prefix('@') {
            let (raw, path) = match constraint_and_path.split_once('/') {
                Some((raw, path)) => (raw, Some(path)),
                None => (constraint_and_path, None),
            };
            if raw.is_empty() {
                return Err(Error::Specifier(
                    "the version constraint after `@` is empty".to_owned(),
                ));
            }
            let constraint = VersionConstraint::parse(raw)?;
            let subpath = path.map(Subpath::parse).transpose()?;
            (constraint, subpath)
        } else if let Some(path) = suffix.strip_prefix('/') {
            (
                VersionConstraint::unconstrained(),
                Some(Subpath::parse(path)?),
            )
        } else {
            return Err(Error::Specifier(
                "the package specifier has an unexpected suffix".to_owned(),
            ));
        };

        Ok(Self {
            package: PackageId {
                scope: scope.to_owned(),
                name: name.to_owned(),
            },
            constraint,
            subpath,
        })
    }

    /// Returns the entrypoint key: `.` for the default entrypoint, otherwise
    /// the `./`-prefixed subpath.
    pub fn export_key(&self) -> String {
        match &self.subpath {
            Some(subpath) => subpath.export_key(),
            None => ".".to_owned(),
        }
    }
}

/// A validated SemVer constraint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionConstraint {
    /// The original text, or `None` when the specifier omitted a constraint.
    pub raw: Option<String>,
    /// The requirement used for range matching. An exact constraint is
    /// selected through [`Self::exact`] and does not use this requirement.
    pub req: VersionReq,
    /// The exact version when the constraint names one.
    pub exact: Option<Version>,
}

impl VersionConstraint {
    /// Returns an unconstrained requirement matching the latest non-yanked
    /// stable version.
    pub fn unconstrained() -> Self {
        Self {
            raw: None,
            req: VersionReq::STAR,
            exact: None,
        }
    }

    /// Parses one SemVer version or requirement string.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        if raw.len() > MAX_CONSTRAINT_BYTES {
            return Err(Error::Constraint(format!(
                "the constraint is {} bytes, over the {MAX_CONSTRAINT_BYTES}-byte limit",
                raw.len()
            )));
        }
        let comparators = raw.matches(',').count() + 1;
        if comparators > MAX_COMPARATORS {
            return Err(Error::Constraint(format!(
                "the constraint has {comparators} comparators, over the {MAX_COMPARATORS}-comparator limit"
            )));
        }
        let exact = parse_exact_version(raw);
        let req = VersionReq::parse(&normalize_bare_partial(raw)).map_err(|_| {
            Error::Constraint(format!(
                "`{raw}` is not a SemVer version or requirement; dist-tags such as `latest` are not supported"
            ))
        })?;
        Ok(Self {
            raw: Some(raw.to_owned()),
            req,
            exact,
        })
    }

    fn describe(&self) -> String {
        self.raw
            .clone()
            .unwrap_or_else(|| "latest stable".to_owned())
    }
}

/// Selects a bare version or an `=`-prefixed version as exact.
fn parse_exact_version(raw: &str) -> Option<Version> {
    if let Ok(version) = Version::parse(raw) {
        return Some(version);
    }
    raw.strip_prefix('=')
        .and_then(|rest| Version::parse(rest).ok())
}

/// Rewrites a bare `major.minor` version to the registry's minor-line range.
///
/// Rust `semver` treats a bare `1.2` as caret-compatible (`>=1.2.0, <2.0.0`),
/// but JSR documents `1.2` as a specific minor version (`>=1.2.0, <1.3.0`).
/// Bare full versions are handled by [`parse_exact_version`] before this runs.
fn normalize_bare_partial(raw: &str) -> String {
    let mut parts = raw.split('.');
    let (Some(major), Some(minor), None) = (parts.next(), parts.next(), parts.next()) else {
        return raw.to_owned();
    };
    if major.is_empty()
        || minor.is_empty()
        || !major.bytes().all(|byte| byte.is_ascii_digit())
        || !minor.bytes().all(|byte| byte.is_ascii_digit())
    {
        return raw.to_owned();
    }
    format!("~{raw}")
}

/// A published version selected from package metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedVersion {
    /// The exact published version.
    pub version: Version,
    /// Whether the registry marks this version yanked.
    pub yanked: bool,
}

/// Validated `meta.json` package metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageMetadata {
    /// The package this metadata belongs to.
    pub package: PackageId,
    versions: BTreeMap<Version, bool>,
}

#[derive(Deserialize)]
struct RawPackageMetadata {
    scope: String,
    name: String,
    versions: BTreeMap<String, RawPackageVersion>,
}

#[derive(Deserialize)]
struct RawPackageVersion {
    #[serde(default)]
    yanked: bool,
}

impl PackageMetadata {
    /// Parses and validates one `meta.json` document.
    ///
    /// Scope and package names are revalidated, version keys must be canonical
    /// SemVer, and at least one version must be listed. Unknown fields are
    /// ignored for forward compatibility.
    pub fn parse(json: &str) -> Result<Self, Error> {
        if json.len() > MAX_METADATA_BYTES {
            return Err(Error::Metadata(format!(
                "the package metadata is {} bytes, over the {MAX_METADATA_BYTES}-byte limit",
                json.len()
            )));
        }
        let raw: RawPackageMetadata = serde_json::from_str(json).map_err(|error| {
            Error::Metadata(format!("the package metadata is not valid JSON: {error}"))
        })?;
        validate_scope(&raw.scope).map_err(Error::Metadata)?;
        validate_package_name(&raw.name).map_err(Error::Metadata)?;
        if raw.versions.is_empty() {
            return Err(Error::Metadata(format!(
                "package `@{}/{}` lists no versions",
                raw.scope, raw.name
            )));
        }
        if raw.versions.len() > MAX_VERSIONS {
            return Err(Error::Metadata(format!(
                "package `@{}/{}` lists {} versions, over the {MAX_VERSIONS}-version limit",
                raw.scope,
                raw.name,
                raw.versions.len()
            )));
        }

        let mut versions = BTreeMap::new();
        for (text, info) in raw.versions {
            let version = Version::parse(&text).map_err(|_| {
                Error::Metadata(format!("version key `{text}` is not valid SemVer"))
            })?;
            if version.to_string() != text {
                return Err(Error::Metadata(format!(
                    "version key `{text}` is not canonical SemVer; expected `{version}`"
                )));
            }
            versions.insert(version, info.yanked);
        }

        Ok(Self {
            package: PackageId {
                scope: raw.scope,
                name: raw.name,
            },
            versions,
        })
    }

    /// Selects one published version for a constraint.
    ///
    /// An exact constraint selects that version even when it is yanked. A
    /// default or range constraint ignores yanked versions and prereleases
    /// unless the requirement names a compatible prerelease. When several
    /// versions remain, the greatest SemVer precedence wins; `Version`'s total
    /// order breaks any equal-precedence build-metadata tie deterministically.
    pub fn select(&self, constraint: &VersionConstraint) -> Result<SelectedVersion, Error> {
        if let Some(exact) = &constraint.exact {
            let Some(yanked) = self.versions.get(exact) else {
                return Err(Error::Version(format!(
                    "package `{}` has no published version `{exact}`",
                    self.package
                )));
            };
            return Ok(SelectedVersion {
                version: exact.clone(),
                yanked: *yanked,
            });
        }

        let mut best: Option<&Version> = None;
        let mut yanked_match = false;
        for (version, yanked) in &self.versions {
            if !constraint.req.matches(version) {
                continue;
            }
            if *yanked {
                yanked_match = true;
                continue;
            }
            if best.is_none_or(|candidate| version > candidate) {
                best = Some(version);
            }
        }

        match best {
            Some(version) => Ok(SelectedVersion {
                version: version.clone(),
                yanked: false,
            }),
            None if yanked_match => Err(Error::Version(format!(
                "every version of `{}` matching `{}` is yanked; specify an exact version to use one",
                self.package,
                constraint.describe()
            ))),
            None => Err(Error::Version(format!(
                "no published version of `{}` matches `{}`",
                self.package,
                constraint.describe()
            ))),
        }
    }
}

/// Validated `<version>_meta.json` version metadata bound to one package
/// version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionMetadata {
    package_version: PackageVersion,
    manifest: BTreeMap<String, ManifestEntry>,
    exports: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManifestEntry {
    size: u64,
    checksum: [u8; 32],
}

#[derive(Deserialize)]
struct RawVersionMetadata {
    manifest: BTreeMap<String, RawManifestEntry>,
    #[serde(default)]
    exports: Option<RawExports>,
}

#[derive(Deserialize)]
struct RawManifestEntry {
    size: u64,
    checksum: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawExports {
    Single(String),
    Map(BTreeMap<String, String>),
}

/// One resolved entrypoint and its manifest entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportTarget {
    /// The export key matched: `.` or a `./`-prefixed key.
    pub key: String,
    /// The manifest path, with a leading `/`.
    pub path: String,
    /// The declared file length in bytes.
    pub size: u64,
    /// The declared SHA-256 digest.
    pub checksum: [u8; 32],
}

impl ExportTarget {
    /// Verifies downloaded bytes against this entry's declared length and
    /// SHA-256 checksum.
    pub fn verify(&self, bytes: &[u8]) -> Result<(), Error> {
        verify_bytes(&self.path, self.size, &self.checksum, bytes)
    }
}

impl VersionMetadata {
    /// Parses and validates one `<version>_meta.json` document for a known
    /// [`PackageVersion`].
    ///
    /// The document itself has no package or version field, so the binding is
    /// the caller's assertion about what was fetched. [`resolve_module`] checks
    /// that binding against the package metadata before resolving an
    /// entrypoint.
    ///
    /// Manifest paths, export keys, and export targets are validated, and each
    /// checksum must be `sha256-` followed by 64 lowercase hex characters.
    /// Export targets are not required to appear in the manifest here;
    /// [`Self::resolve_export`] reports that inconsistency when an entrypoint is
    /// actually requested.
    pub fn parse(json: &str, package_version: PackageVersion) -> Result<Self, Error> {
        if json.len() > MAX_METADATA_BYTES {
            return Err(Error::Metadata(format!(
                "the version metadata is {} bytes, over the {MAX_METADATA_BYTES}-byte limit",
                json.len()
            )));
        }
        let raw: RawVersionMetadata = serde_json::from_str(json).map_err(|error| {
            Error::Metadata(format!("the version metadata is not valid JSON: {error}"))
        })?;
        if raw.manifest.is_empty() {
            return Err(Error::Metadata(
                "the version metadata lists no files".to_owned(),
            ));
        }
        if raw.manifest.len() > MAX_MANIFEST_ENTRIES {
            return Err(Error::Metadata(format!(
                "the version metadata lists {} files, over the {MAX_MANIFEST_ENTRIES}-file limit",
                raw.manifest.len()
            )));
        }

        let mut manifest = BTreeMap::new();
        for (path, entry) in raw.manifest {
            validate_manifest_path(&path).map_err(Error::Metadata)?;
            let checksum = parse_checksum(&entry.checksum).map_err(Error::Metadata)?;
            manifest.insert(
                path,
                ManifestEntry {
                    size: entry.size,
                    checksum,
                },
            );
        }

        let exports = match raw.exports {
            None => None,
            Some(RawExports::Single(target)) => {
                let target = normalize_export_target(&target).map_err(Error::Metadata)?;
                let mut exports = BTreeMap::new();
                exports.insert(".".to_owned(), target);
                Some(exports)
            }
            Some(RawExports::Map(exports)) => {
                if exports.is_empty() {
                    return Err(Error::Metadata(
                        "the version metadata lists no exports".to_owned(),
                    ));
                }
                if exports.len() > MAX_EXPORTS {
                    return Err(Error::Metadata(format!(
                        "the version metadata lists {} exports, over the {MAX_EXPORTS}-export limit",
                        exports.len()
                    )));
                }
                let mut normalized = BTreeMap::new();
                for (key, target) in exports {
                    validate_export_key(&key).map_err(Error::Metadata)?;
                    let target = normalize_export_target(&target).map_err(Error::Metadata)?;
                    normalized.insert(key, target);
                }
                Some(normalized)
            }
        };

        Ok(Self {
            package_version,
            manifest,
            exports,
        })
    }

    /// Returns the package version this metadata was parsed for.
    pub fn package_version(&self) -> &PackageVersion {
        &self.package_version
    }

    /// Resolves an export key to its manifest entry.
    ///
    /// `key` is `.` for the default entrypoint or a `./`-prefixed key. A
    /// missing exports map, a missing key, and an export whose target is absent
    /// from the manifest are distinct errors.
    pub fn resolve_export(&self, key: &str) -> Result<ExportTarget, Error> {
        validate_export_key(key).map_err(Error::Export)?;
        let exports = self.exports.as_ref().ok_or_else(|| {
            Error::Export(format!(
                "the package version has no exports; cannot import `{key}`"
            ))
        })?;
        let target = exports
            .get(key)
            .ok_or_else(|| Error::Export(format!("the package version has no export `{key}`")))?;
        let rest = target.strip_prefix("./").ok_or_else(|| {
            Error::Export(format!(
                "export `{key}` has invalid target `{target}` after metadata validation"
            ))
        })?;
        let path = format!("/{rest}");
        let entry = self.manifest.get(&path).ok_or_else(|| {
            Error::Export(format!(
                "export `{key}` points to `{path}`, which is not listed in the manifest"
            ))
        })?;
        Ok(ExportTarget {
            key: key.to_owned(),
            path,
            size: entry.size,
            checksum: entry.checksum,
        })
    }

    /// Returns the declared byte length of one manifest file.
    ///
    /// The path must be a manifest key with a leading `/`. A missing entry is a
    /// [`Error::Manifest`]. A caller uses this to reject an oversized download
    /// before fetching it; the length is checked again by
    /// [`Self::verify_file`] against the actual bytes.
    pub fn file_size(&self, path: &str) -> Result<u64, Error> {
        validate_manifest_path(path).map_err(Error::Manifest)?;
        self.manifest
            .get(path)
            .map(|entry| entry.size)
            .ok_or_else(|| Error::Manifest(format!("`{path}` is not listed in the manifest")))
    }

    /// Verifies downloaded bytes for one manifest path.
    ///
    /// The path must be a manifest key with a leading `/`. A missing entry is a
    /// [`Error::Manifest`]; a length or checksum mismatch is
    /// [`Error::Integrity`].
    pub fn verify_file(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        validate_manifest_path(path).map_err(Error::Manifest)?;
        let entry = self
            .manifest
            .get(path)
            .ok_or_else(|| Error::Manifest(format!("`{path}` is not listed in the manifest")))?;
        verify_bytes(path, entry.size, &entry.checksum, bytes)
    }
}

/// A module resolved to one exact version and one manifest file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModule {
    /// The resolved package.
    pub package: PackageId,
    /// The exact resolved version.
    pub version: Version,
    /// Whether the exact version is yanked. Only an explicit exact import can
    /// produce `true`.
    pub yanked: bool,
    /// The matched entrypoint and its manifest entry.
    pub target: ExportTarget,
}

impl ResolvedModule {
    /// Returns the canonical `jsr:` specifier for the exact resolution.
    ///
    /// The default entrypoint has no suffix. Other entrypoints use the export
    /// key without its `./` prefix, so `./join` becomes `/join`.
    pub fn canonical_specifier(&self) -> String {
        let mut text = format!("jsr:{}@{}", self.package, self.version);
        if self.target.key != "." {
            text.push('/');
            text.push_str(
                self.target
                    .key
                    .strip_prefix("./")
                    .unwrap_or(&self.target.key),
            );
        }
        text
    }

    /// Returns the registry-relative module path, without the `https://jsr.io/`
    /// origin: `@scope/name/version/file`.
    ///
    /// The path is not URL-encoded.
    pub fn registry_path(&self) -> String {
        let file = self
            .target
            .path
            .strip_prefix('/')
            .unwrap_or(&self.target.path);
        format!("{}/{}/{}", self.package, self.version, file)
    }

    /// Verifies downloaded bytes against the resolved manifest entry.
    pub fn verify(&self, bytes: &[u8]) -> Result<(), Error> {
        self.target.verify(bytes)
    }
}

impl fmt::Display for ResolvedModule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical_specifier())
    }
}

/// Resolves a parsed specifier against package and version metadata.
///
/// The package metadata must describe the specifier's package. The version
/// metadata must be bound to the exact package version selected from that
/// package metadata; a mismatch is rejected before any entrypoint is resolved.
pub fn resolve_module(
    specifier: &Specifier,
    package: &PackageMetadata,
    version_metadata: &VersionMetadata,
) -> Result<ResolvedModule, Error> {
    if package.package != specifier.package {
        return Err(Error::Metadata(format!(
            "package metadata is for `{}`, but the specifier names `{}`",
            package.package, specifier.package
        )));
    }
    let selected = package.select(&specifier.constraint)?;
    let expected = PackageVersion {
        package: package.package.clone(),
        version: selected.version,
    };
    if version_metadata.package_version() != &expected {
        return Err(Error::Metadata(format!(
            "version metadata is for `{}`, but the specifier resolves to `{expected}`",
            version_metadata.package_version()
        )));
    }
    let target = version_metadata.resolve_export(&specifier.export_key())?;
    Ok(ResolvedModule {
        package: expected.package,
        version: expected.version,
        yanked: selected.yanked,
        target,
    })
}

fn validate_scope(scope: &str) -> Result<(), String> {
    validate_name_like(scope, "scope", MIN_SCOPE_CHARS, MAX_SCOPE_CHARS)
}

fn validate_package_name(name: &str) -> Result<(), String> {
    validate_name_like(name, "package name", MIN_NAME_CHARS, MAX_NAME_CHARS)
}

fn validate_name_like(
    value: &str,
    label: &str,
    minimum: usize,
    maximum: usize,
) -> Result<(), String> {
    let characters = value.chars().count();
    if !(minimum..=maximum).contains(&characters) {
        return Err(format!(
            "the {label} must be {minimum} to {maximum} characters, got {characters}"
        ));
    }
    if value.starts_with('-') {
        return Err(format!("the {label} must not start with a hyphen"));
    }
    if !value.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err(format!(
            "the {label} may contain only lowercase ASCII letters, digits, and hyphens"
        ));
    }
    Ok(())
}

fn validate_path_text(value: &str, label: &str) -> Result<(), String> {
    if value.len() > MAX_PATH_BYTES {
        return Err(format!(
            "the {label} is {} bytes, over the {MAX_PATH_BYTES}-byte limit",
            value.len()
        ));
    }
    for character in value.chars() {
        let reason = match character {
            '\\' => Some("contains a backslash"),
            '%' => Some("contains a percent escape; percent decoding is not performed"),
            '?' => Some("contains a query marker"),
            '#' => Some("contains a fragment marker"),
            '*' => Some("contains a wildcard"),
            character if character.is_control() => Some("contains a control character"),
            _ => None,
        };
        if let Some(reason) = reason {
            return Err(format!("the {label} {reason}"));
        }
    }
    for segment in value.split('/') {
        if segment.is_empty() {
            return Err(format!("the {label} contains an empty segment"));
        }
        if segment == "." {
            return Err(format!("the {label} contains a `.` segment"));
        }
        if segment == ".." {
            return Err(format!("the {label} contains a `..` segment"));
        }
    }
    Ok(())
}

fn validate_export_key(key: &str) -> Result<(), String> {
    if key == "." {
        return Ok(());
    }
    let Some(rest) = key.strip_prefix("./") else {
        return Err(format!(
            "the export key `{key}` must be `.` or start with `./`"
        ));
    };
    if rest.is_empty() {
        return Err("the export key is empty after `./`".to_owned());
    }
    validate_path_text(rest, "export key")
}

fn normalize_export_target(target: &str) -> Result<String, String> {
    let Some(rest) = target.strip_prefix("./") else {
        return Err(format!(
            "the export target `{target}` must be a relative path starting with `./`"
        ));
    };
    if rest.is_empty() {
        return Err("the export target is empty after `./`".to_owned());
    }
    validate_path_text(rest, "export target")?;
    Ok(format!("./{rest}"))
}

/// Validates a leading-`/` manifest path.
pub fn validate_manifest_path(path: &str) -> Result<&str, String> {
    let Some(rest) = path.strip_prefix('/') else {
        return Err(format!("the manifest path `{path}` must start with `/`"));
    };
    if rest.is_empty() {
        return Err("the manifest path names no file after `/`".to_owned());
    }
    validate_path_text(rest, "manifest path")?;
    Ok(rest)
}

fn parse_checksum(value: &str) -> Result<[u8; 32], String> {
    let Some(hex) = value.strip_prefix("sha256-") else {
        return Err("the checksum must use the `sha256-` prefix".to_owned());
    };
    if hex.len() != 64 {
        return Err(format!(
            "the SHA-256 checksum must have 64 hex characters, got {}",
            hex.len()
        ));
    }
    if !hex.is_ascii() {
        return Err("the SHA-256 checksum must be 64 lowercase hexadecimal characters".to_owned());
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        let start = index * 2;
        let pair = &hex[start..start + 2];
        if !pair
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
        {
            return Err(
                "the SHA-256 checksum must be 64 lowercase hexadecimal characters".to_owned(),
            );
        }
        *byte = u8::from_str_radix(pair, 16)
            .map_err(|_| "the SHA-256 checksum is not valid hexadecimal".to_owned())?;
    }
    Ok(digest)
}

fn verify_bytes(
    path: &str,
    expected_size: u64,
    expected_checksum: &[u8; 32],
    bytes: &[u8],
) -> Result<(), Error> {
    if bytes.len() as u64 != expected_size {
        return Err(Error::Integrity(format!(
            "`{path}` has {} bytes, expected {expected_size}",
            bytes.len()
        )));
    }
    let actual_checksum = Sha256::digest(bytes);
    if actual_checksum.as_slice() != expected_checksum.as_slice() {
        return Err(Error::Integrity(format!(
            "`{path}` has checksum sha256-{}, expected sha256-{}",
            encode_hex(actual_checksum.as_slice()),
            encode_hex(expected_checksum)
        )));
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::remote::{RemoteOptions, RemoteTransport, TestRemoteResponse};

    const MOD_FILE: &[u8] = b"export const value = 1;\n";
    const MOD_SHA: &str = "sha256-5d8f65d2774e206bc9f7a7a4ad39ca2dc563b5c31e46ab57ef4874961237ce29";
    const JOIN_FILE: &[u8] = b"export function join(a, b) { return a + b; }\n";
    const JOIN_SHA: &str =
        "sha256-54823f4581bd517efd2e49948c23990faf3d49794d54d9275fe911e8f4a12156";

    const VERSION_JSON: &str = r#"{
        "manifest": {
            "/mod.ts": { "size": 24, "checksum": "sha256-5d8f65d2774e206bc9f7a7a4ad39ca2dc563b5c31e46ab57ef4874961237ce29" },
            "/join.ts": { "size": 45, "checksum": "sha256-54823f4581bd517efd2e49948c23990faf3d49794d54d9275fe911e8f4a12156" }
        },
        "exports": { ".": "./mod.ts", "./join": "./join.ts" }
    }"#;

    fn package_metadata(versions: &[(&str, bool)]) -> PackageMetadata {
        let entries = versions
            .iter()
            .map(|(version, yanked)| {
                if *yanked {
                    format!("\"{version}\":{{\"yanked\":true}}")
                } else {
                    format!("\"{version}\":{{}}")
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(r#"{{"scope":"scope","name":"pkg","versions":{{{entries}}}}}"#);
        PackageMetadata::parse(&json).expect("package metadata fixture")
    }

    fn default_package_metadata() -> PackageMetadata {
        package_metadata(&[
            ("1.0.0", false),
            ("1.0.1", true),
            ("1.1.0", false),
            ("2.0.0-beta.1", false),
        ])
    }

    fn specifier(input: &str) -> Specifier {
        Specifier::parse(input).expect("specifier fixture")
    }

    fn package_version(version: &str) -> PackageVersion {
        PackageVersion {
            package: PackageId {
                scope: "scope".to_owned(),
                name: "pkg".to_owned(),
            },
            version: Version::parse(version).expect("version fixture"),
        }
    }

    fn version_metadata_for(version: &str) -> VersionMetadata {
        VersionMetadata::parse(VERSION_JSON, package_version(version))
            .expect("version metadata fixture")
    }

    fn version_metadata() -> VersionMetadata {
        version_metadata_for("1.1.0")
    }

    fn select(
        versions: &[(&str, bool)],
        constraint: Option<&str>,
    ) -> Result<SelectedVersion, Error> {
        let package = package_metadata(versions);
        let constraint = match constraint {
            Some(raw) => VersionConstraint::parse(raw)?,
            None => VersionConstraint::unconstrained(),
        };
        package.select(&constraint)
    }

    fn selected(versions: &[(&str, bool)], constraint: Option<&str>) -> String {
        select(versions, constraint)
            .expect("version selection fixture")
            .version
            .to_string()
    }

    fn error_text(result: Result<impl std::fmt::Debug, Error>) -> String {
        match result {
            Ok(value) => panic!("expected an error, got {value:?}"),
            Err(error) => error.to_string(),
        }
    }

    // Unit test: private parser; exact assertions on accepted grammar.
    #[test]
    fn parses_valid_specifiers() {
        let plain = specifier("jsr:@luca/cases");
        assert_eq!(plain.package.to_string(), "@luca/cases");
        assert_eq!(plain.constraint.raw, None);
        assert_eq!(plain.export_key(), ".");
        assert_eq!(plain.subpath, None);

        let ranged = specifier("jsr:@std/path@^1.0.0/join");
        assert_eq!(ranged.package.to_string(), "@std/path");
        assert_eq!(ranged.constraint.raw.as_deref(), Some("^1.0.0"));
        assert_eq!(ranged.constraint.exact, None);
        assert_eq!(ranged.export_key(), "./join");
        assert_eq!(ranged.subpath.as_ref().map(Subpath::as_str), Some("join"));

        let nested = specifier("jsr:@scope/name@~1.2.3/sub/dir");
        assert_eq!(nested.constraint.raw.as_deref(), Some("~1.2.3"));
        assert_eq!(nested.export_key(), "./sub/dir");
    }

    // Unit test: private parser; rejection cases are exact assertions.
    #[test]
    fn rejects_malformed_specifiers() {
        let cases = vec![
            ("", "must start with `jsr:`"),
            ("jsr", "must start with `jsr:`"),
            ("JSR:@scope/name", "must start with `jsr:`"),
            ("jsr:", "names no package"),
            ("jsr:luca/cases", "must be scoped"),
            ("jsr:@/name", "scope must be 2 to 20"),
            ("jsr:@scope/", "package name must be 2 to 58"),
            ("jsr:@scope/name@", "constraint after `@` is empty"),
            (
                "jsr:@scope/name@nope",
                "not a SemVer version or requirement",
            ),
            ("jsr:@scope/name@1.0.0 - 2.0.0", "not a SemVer"),
            ("jsr:@scope/name/", "subpath after `/` is empty"),
            ("jsr:@scope/name//x", "empty segment"),
            ("jsr:@scope/name/../x", "`..` segment"),
            ("jsr:@scope/name/./x", "`.` segment"),
            ("jsr:@scope/name/a/../../b", "`..` segment"),
            ("jsr:@scope/name/a\\b", "backslash"),
            ("jsr:@scope/name/a/b\\c", "backslash"),
            ("jsr:@scope/name/%2e%2e", "percent escape"),
            ("jsr:@scope/name/%2F", "percent escape"),
            ("jsr:@scope/name/%5c", "percent escape"),
            ("jsr:@scope/name/a%20b", "percent escape"),
            ("jsr:@scope/name/a?b", "query marker"),
            ("jsr:@scope/name/a#b", "fragment marker"),
            ("jsr:@scope/name/a*b", "wildcard"),
        ];
        for (input, expected) in cases {
            let error = error_text(Specifier::parse(input));
            assert!(
                error.contains(expected),
                "`{input}` produced `{error}`, expected `{expected}`"
            );
        }

        let control = format!("jsr:@scope/name/a{}b", '\u{7f}');
        let error = error_text(Specifier::parse(&control));
        assert!(error.contains("control character"), "{error}");

        let long_subpath = format!("jsr:@scope/name/{}", "a".repeat(MAX_SUBPATH_BYTES + 1));
        let error = error_text(Specifier::parse(&long_subpath));
        assert!(error.contains("over the 512-byte limit"), "{error}");

        let long_specifier = format!("jsr:@scope/name/{}", "a".repeat(MAX_SPECIFIER_BYTES));
        let error = error_text(Specifier::parse(&long_specifier));
        assert!(error.contains("over the 2048-byte limit"), "{error}");
    }

    // Unit test: private name validator; exact assertions on registry rules.
    #[test]
    fn rejects_invalid_scope_and_package_names() {
        let long_scope = "a".repeat(MAX_SCOPE_CHARS + 1);
        let long_name = "a".repeat(MAX_NAME_CHARS + 1);
        let cases = vec![
            ("jsr:@a/pkg".to_owned(), "scope must be 2 to 20"),
            (format!("jsr:@{long_scope}/pkg"), "scope must be 2 to 20"),
            ("jsr:@-scope/pkg".to_owned(), "must not start with a hyphen"),
            ("jsr:@scope/-pkg".to_owned(), "must not start with a hyphen"),
            ("jsr:@scope/pkg_name".to_owned(), "lowercase ASCII"),
            ("jsr:@scope/PKG".to_owned(), "lowercase ASCII"),
            ("jsr:@scope/pkü".to_owned(), "lowercase ASCII"),
            ("jsr:@sc_ope/pkg".to_owned(), "lowercase ASCII"),
            (
                format!("jsr:@scope/{long_name}"),
                "package name must be 2 to 58",
            ),
            ("jsr:@scope/p".to_owned(), "package name must be 2 to 58"),
        ];
        for (input, expected) in cases {
            let error = error_text(Specifier::parse(&input));
            assert!(
                error.contains(expected),
                "`{input}` produced `{error}`, expected `{expected}`"
            );
        }
    }

    // Unit test: semver matching is a critical parser path; exact assertion.
    #[test]
    fn bare_full_versions_are_exact_not_caret_ranges() {
        let versions = &[("1.2.3", false), ("1.2.4", false), ("1.3.0", false)];
        assert_eq!(selected(versions, Some("1.2.3")), "1.2.3");
        assert_eq!(selected(versions, Some("=1.2.3")), "1.2.3");
    }

    // Unit test: semver range matching; exact assertions on documented forms.
    #[test]
    fn major_minor_caret_tilde_and_star_are_ranges() {
        let versions = &[
            ("1.0.0", false),
            ("1.0.1", false),
            ("1.1.0", false),
            ("1.9.0", false),
            ("2.0.0", false),
        ];
        assert_eq!(selected(versions, Some("1")), "1.9.0");
        assert_eq!(selected(versions, Some("1.0")), "1.0.1");
        assert_eq!(selected(versions, Some("^1.0.0")), "1.9.0");
        assert_eq!(selected(versions, Some("~1.0.0")), "1.0.1");
        assert_eq!(selected(versions, Some("*")), "2.0.0");
        assert_eq!(selected(versions, None), "2.0.0");
    }

    // Unit test: prerelease policy; exact assertions on documented exclusion.
    #[test]
    fn prereleases_need_an_explicit_compatible_requirement() {
        let versions = &[("1.0.0", false), ("1.1.0-beta.1", false)];
        assert_eq!(selected(versions, None), "1.0.0");
        assert_eq!(selected(versions, Some("^1.0.0")), "1.0.0");
        assert_eq!(selected(versions, Some(">=1.0.0")), "1.0.0");
        assert_eq!(selected(versions, Some("1.1.0-beta.1")), "1.1.0-beta.1");
    }

    // Unit test: yanked policy; exact assertions on the documented exception.
    #[test]
    fn yanked_versions_are_skipped_unless_the_request_is_exact() {
        let versions = &[("1.0.0", false), ("1.0.1", true), ("1.0.2", true)];
        assert_eq!(selected(versions, None), "1.0.0");
        assert_eq!(selected(versions, Some("1")), "1.0.0");

        let explicit = select(versions, Some("1.0.1")).expect("explicit yanked version");
        assert_eq!(explicit.version.to_string(), "1.0.1");
        assert!(explicit.yanked);

        let error = error_text(select(&[("1.0.0", false), ("2.0.0", true)], Some("^2")));
        assert!(error.contains("every version"), "{error}");
        assert!(error.contains("yanked"), "{error}");
    }

    // Unit test: version-selection errors; exact assertions on the reasons.
    #[test]
    fn reports_missing_versions_and_invalid_constraints() {
        let error = error_text(select(&[("1.0.0", false)], Some("^3")));
        assert!(error.contains("no published version"), "{error}");
        let error = error_text(select(&[("1.0.0", false)], Some("2.0.0")));
        assert!(
            error.contains("has no published version `2.0.0`"),
            "{error}"
        );

        for constraint in ["latest", "next", "1.0.0 - 2.0.0", ">=1.0.0 <2.0.0"] {
            let error = error_text(select(&[("1.0.0", false)], Some(constraint)));
            assert!(
                error.contains("not a SemVer version or requirement"),
                "`{constraint}` produced `{error}`"
            );
        }
    }

    // Unit test: constraint limits; exact assertions on the documented bounds.
    #[test]
    fn enforces_constraint_limits() {
        let too_long = "1".repeat(MAX_CONSTRAINT_BYTES + 1);
        let error = error_text(select(&[("1.0.0", false)], Some(&too_long)));
        assert!(error.contains("over the 128-byte limit"), "{error}");

        let many = ["1.0.0"; MAX_COMPARATORS + 1].join(", ");
        let error = error_text(select(&[("1.0.0", false)], Some(&many)));
        assert!(error.contains("over the 8-comparator limit"), "{error}");
    }

    // Unit test: metadata validation; exact assertions on version identity.
    #[test]
    fn package_metadata_requires_canonical_versions() {
        let invalid = r#"{"scope":"scope","name":"pkg","versions":{"01.0.0":{}}}"#;
        let error = error_text(PackageMetadata::parse(invalid));
        assert!(error.contains("not valid SemVer"), "{error}");

        let empty = r#"{"scope":"scope","name":"pkg","versions":{}}"#;
        let error = error_text(PackageMetadata::parse(empty));
        assert!(error.contains("lists no versions"), "{error}");

        let bad_scope = r#"{"scope":"SCOPE","name":"pkg","versions":{"1.0.0":{}}}"#;
        let error = error_text(PackageMetadata::parse(bad_scope));
        assert!(error.contains("lowercase ASCII"), "{error}");
    }

    // Unit test: duplicate fixed JSON fields; exact assertion on serde rejection.
    #[test]
    fn rejects_duplicate_fixed_json_fields() {
        let duplicate_scope =
            r#"{"scope":"scope","scope":"other","name":"pkg","versions":{"1.0.0":{}}}"#;
        let error = error_text(PackageMetadata::parse(duplicate_scope));
        assert!(error.contains("duplicate field `scope`"), "{error}");

        let duplicate_checksum = format!(
            r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}","checksum":"{JOIN_SHA}"}}}},"exports":{{".":"./mod.ts"}}}}"#
        );
        let error = error_text(VersionMetadata::parse(
            &duplicate_checksum,
            package_version("1.1.0"),
        ));
        assert!(error.contains("duplicate field `checksum`"), "{error}");
    }

    // Unit test: duplicate map entries; exact assertions on last-value wins.
    #[test]
    fn duplicate_map_entries_use_the_last_value() {
        let duplicate_versions =
            r#"{"scope":"scope","name":"pkg","versions":{"1.0.0":{},"1.0.0":{"yanked":true}}}"#;
        let package = PackageMetadata::parse(duplicate_versions).expect("last version entry wins");
        let selected = package
            .select(&VersionConstraint::parse("1.0.0").expect("constraint"))
            .expect("selected version");
        assert!(selected.yanked);

        let duplicate_manifest = format!(
            r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{JOIN_SHA}"}},"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./mod.ts"}}}}"#
        );
        let metadata = VersionMetadata::parse(&duplicate_manifest, package_version("1.1.0"))
            .expect("last manifest entry wins");
        metadata
            .verify_file("/mod.ts", MOD_FILE)
            .expect("last checksum wins");

        let duplicate_exports = format!(
            r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}},"/join.ts":{{"size":45,"checksum":"{JOIN_SHA}"}}}},"exports":{{".":"./mod.ts",".":"./join.ts"}}}}"#
        );
        let metadata = VersionMetadata::parse(&duplicate_exports, package_version("1.1.0"))
            .expect("last export entry wins");
        assert_eq!(
            metadata.resolve_export(".").expect("default export").path,
            "/join.ts"
        );
    }

    // Unit test: export resolution and exact version identity.
    #[test]
    fn resolves_entrypoints_and_builds_exact_identities() {
        let package = default_package_metadata();
        let version = version_metadata();

        let root = resolve_module(&specifier("jsr:@scope/pkg@1"), &package, &version)
            .expect("default entrypoint");
        assert_eq!(root.version.to_string(), "1.1.0");
        assert_eq!(root.target.key, ".");
        assert_eq!(root.target.path, "/mod.ts");
        assert_eq!(root.canonical_specifier(), "jsr:@scope/pkg@1.1.0");
        assert_eq!(root.registry_path(), "@scope/pkg/1.1.0/mod.ts");
        assert_eq!(root.to_string(), "jsr:@scope/pkg@1.1.0");
        assert!(!root.yanked);

        let join = resolve_module(&specifier("jsr:@scope/pkg@^1.0.0/join"), &package, &version)
            .expect("named entrypoint");
        assert_eq!(join.target.key, "./join");
        assert_eq!(join.target.path, "/join.ts");
        assert_eq!(join.canonical_specifier(), "jsr:@scope/pkg@1.1.0/join");
        assert_eq!(join.registry_path(), "@scope/pkg/1.1.0/join.ts");

        let yanked = resolve_module(
            &specifier("jsr:@scope/pkg@1.0.1"),
            &package,
            &version_metadata_for("1.0.1"),
        )
        .expect("explicit yanked version");
        assert!(yanked.yanked);
        assert_eq!(yanked.version.to_string(), "1.0.1");

        let equivalent = resolve_module(&specifier("jsr:@scope/pkg@1"), &package, &version)
            .expect("equivalent resolution");
        assert_eq!(root, equivalent);
        let other = resolve_module(
            &specifier("jsr:@scope/pkg@1.0.0"),
            &package,
            &version_metadata_for("1.0.0"),
        )
        .expect("other exact version");
        assert_ne!(root, other);
    }

    // Unit test: missing exports and missing manifest targets; exact assertions.
    #[test]
    fn reports_missing_exports_and_manifest_targets() {
        let package = default_package_metadata();

        let without_exports = VersionMetadata::parse(
            &format!(r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}}}}"#),
            package_version("1.1.0"),
        )
        .expect("manifest-only metadata");
        let error = error_text(resolve_module(
            &specifier("jsr:@scope/pkg@1"),
            &package,
            &without_exports,
        ));
        assert!(error.contains("has no exports"), "{error}");

        let version = version_metadata();
        let error = error_text(version.resolve_export("./missing"));
        assert!(error.contains("has no export `./missing`"), "{error}");

        let missing_target = VersionMetadata::parse(
            &format!(
                r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./missing.ts"}}}}"#
            ),
            package_version("1.1.0"),
        )
        .expect("metadata with a dangling export target");
        let error = error_text(missing_target.resolve_export("."));
        assert!(error.contains("not listed in the manifest"), "{error}");
    }

    // Unit test: metadata path safety; exact assertions on traversal rejection.
    #[test]
    fn rejects_traversal_in_metadata_paths() {
        let cases = vec![
            (
                format!(
                    r#"{{"manifest":{{"/../mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./mod.ts"}}}}"#
                ),
                "`..` segment",
            ),
            (
                format!(
                    r#"{{"manifest":{{"mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./mod.ts"}}}}"#
                ),
                "must start with `/`",
            ),
            (
                format!(
                    r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{"./../x":"./mod.ts"}}}}"#
                ),
                "`..` segment",
            ),
            (
                format!(
                    r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./../mod.ts"}}}}"#
                ),
                "`..` segment",
            ),
            (
                format!(
                    r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./%2e%2e/mod.ts"}}}}"#
                ),
                "percent escape",
            ),
            (
                format!(
                    r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./a\\b.ts"}}}}"#
                ),
                "backslash",
            ),
            (
                format!(
                    r#"{{"manifest":{{"/a//b.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./a//b.ts"}}}}"#
                ),
                "empty segment",
            ),
            (
                format!(
                    r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}},"exports":{{"./*":"./mod.ts"}}}}"#
                ),
                "wildcard",
            ),
        ];
        for (json, expected) in cases {
            let error = error_text(VersionMetadata::parse(&json, package_version("1.1.0")));
            assert!(
                error.contains(expected),
                "fixture produced `{error}`, expected `{expected}`"
            );
        }
    }

    // Unit test: checksum codec and verification; exact assertions on integrity.
    #[test]
    fn verifies_manifest_lengths_and_checksums() {
        let version = version_metadata();
        version
            .verify_file("/mod.ts", MOD_FILE)
            .expect("mod checksum");
        version
            .verify_file("/join.ts", JOIN_FILE)
            .expect("join checksum");

        let error = error_text(version.verify_file("/mod.ts", b"short"));
        assert!(error.contains("has 5 bytes, expected 24"), "{error}");
        let error = error_text(version.verify_file("/mod.ts", b"export const value = 2;\n"));
        assert!(error.contains("expected sha256-5d8f"), "{error}");
        let error = error_text(version.verify_file("/missing.ts", MOD_FILE));
        assert!(error.contains("not listed in the manifest"), "{error}");

        let resolved = resolve_module(
            &specifier("jsr:@scope/pkg@1/join"),
            &default_package_metadata(),
            &version,
        )
        .expect("resolved join");
        resolved.verify(JOIN_FILE).expect("resolved checksum");
    }

    // Unit test: checksum format; exact assertions on the accepted encoding.
    #[test]
    fn rejects_noncanonical_checksums() {
        let cases = vec![
            "sha512-0000000000000000000000000000000000000000000000000000000000000000",
            "sha256-0000",
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "sha256-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ];
        for checksum in cases {
            let json = format!(
                r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{checksum}"}}}},"exports":{{".":"./mod.ts"}}}}"#
            );
            let error = error_text(VersionMetadata::parse(&json, package_version("1.1.0")));
            assert!(
                error.contains("checksum") || error.contains("hex"),
                "`{checksum}` produced `{error}`"
            );
        }

        // A 64-byte value containing a multi-byte character must be rejected
        // without slicing at a non-character boundary.
        let multibyte = format!("sha256-{}{}a", "a".repeat(61), 'é');
        let json = format!(
            r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{multibyte}"}}}},"exports":{{".":"./mod.ts"}}}}"#
        );
        let error = error_text(VersionMetadata::parse(&json, package_version("1.1.0")));
        assert!(error.contains("lowercase hexadecimal"), "{error}");
    }

    // Unit test: metadata identity check; exact assertion on mismatch.
    #[test]
    fn rejects_metadata_for_another_package() {
        let other =
            PackageMetadata::parse(r#"{"scope":"other","name":"pkg","versions":{"1.0.0":{}}}"#)
                .expect("other package metadata");
        let error = error_text(resolve_module(
            &specifier("jsr:@scope/pkg@1"),
            &other,
            &version_metadata(),
        ));
        assert!(error.contains("metadata is for `@other/pkg`"), "{error}");
        assert!(error.contains("names `@scope/pkg`"), "{error}");
    }

    // Unit test: version metadata binding; exact assertion on wrong package.
    #[test]
    fn rejects_version_metadata_bound_to_another_package() {
        let metadata = VersionMetadata::parse(
            VERSION_JSON,
            PackageVersion {
                package: PackageId {
                    scope: "other".to_owned(),
                    name: "pkg".to_owned(),
                },
                version: Version::parse("1.1.0").expect("version"),
            },
        )
        .expect("version metadata fixture");
        let error = error_text(resolve_module(
            &specifier("jsr:@scope/pkg@1"),
            &default_package_metadata(),
            &metadata,
        ));
        assert!(
            error.contains("version metadata is for `@other/pkg@1.1.0`"),
            "{error}"
        );
        assert!(
            error.contains("specifier resolves to `@scope/pkg@1.1.0`"),
            "{error}"
        );
    }

    // Unit test: version metadata binding; wrong version is rejected before exports.
    #[test]
    fn rejects_version_metadata_bound_to_another_version_before_resolving_exports() {
        let manifest_only =
            format!(r#"{{"manifest":{{"/mod.ts":{{"size":24,"checksum":"{MOD_SHA}"}}}}}}"#);
        let metadata = VersionMetadata::parse(&manifest_only, package_version("1.0.0"))
            .expect("manifest-only metadata");
        let error = error_text(resolve_module(
            &specifier("jsr:@scope/pkg@1"),
            &default_package_metadata(),
            &metadata,
        ));
        assert!(
            error.contains("version metadata is for `@scope/pkg@1.0.0`"),
            "{error}"
        );
        assert!(
            error.contains("specifier resolves to `@scope/pkg@1.1.0`"),
            "{error}"
        );
        assert!(!error.contains("has no exports"), "{error}");
    }

    // Unit test: size limit; exact assertion on the metadata bound.
    #[test]
    fn rejects_oversized_metadata() {
        let json = format!(
            r#"{{"scope":"scope","name":"pkg","versions":{{}}}}{}"#,
            " ".repeat(MAX_METADATA_BYTES)
        );
        let error = error_text(PackageMetadata::parse(&json));
        assert!(error.contains("over the 4194304-byte limit"), "{error}");
    }

    #[test]
    fn cache_registry_resolves_cached_jsr_package_offline_even_when_stale() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pkg_json = r#"{"scope":"scope","name":"pkg","versions":{"1.1.0":{}}}"#;

        let transport: Arc<dyn RemoteTransport> =
            crate::remote::test_transport(move |url, _, _, _| {
                let (status, location, body) = match url {
                    "https://jsr.io/@scope/pkg/meta.json" => {
                        (200, None, pkg_json.as_bytes().to_vec())
                    }
                    "https://jsr.io/@scope/pkg/1.1.0_meta.json" => (
                        302,
                        Some("https://cdn.example.test/jsr/1.1.0_meta.json"),
                        Vec::new(),
                    ),
                    "https://cdn.example.test/jsr/1.1.0_meta.json" => {
                        (200, None, VERSION_JSON.as_bytes().to_vec())
                    }
                    "https://jsr.io/@scope/pkg/1.1.0/mod.ts" => (200, None, MOD_FILE.to_vec()),
                    other => return Err(format!("unexpected URL: {other}")),
                };
                Ok(TestRemoteResponse {
                    status,
                    location: location.map(str::to_owned),
                    declared_length: None,
                    etag: None,
                    last_modified: None,
                    // Cacheable (non-no-store) but immediately stale (`max-age=0`).
                    cache_control: Some("max-age=0".to_string()),
                    date: None,
                    age: None,
                    body,
                })
            });

        let client = RemoteClient::with_transport(
            RemoteOptions::new().with_cache_dir(dir.path()),
            transport,
        );
        let remote_registry = RemoteRegistry::new(client);
        let spec = Specifier::parse("jsr:@scope/pkg@1").expect("spec");

        // Warm the cache via the live RemoteRegistry (including redirect on version metadata).
        let pkg_meta = PackageMetadata::parse(
            &remote_registry
                .package_metadata(&spec.package)
                .expect("live pkg meta"),
        )
        .expect("parse pkg");
        let sel = pkg_meta.select(&spec.constraint).expect("select");
        let pv = PackageVersion {
            package: spec.package.clone(),
            version: sel.version,
        };
        let ver_meta = VersionMetadata::parse(
            &remote_registry
                .version_metadata(&pv)
                .expect("live ver meta"),
            pv.clone(),
        )
        .expect("parse ver");
        let resolved = resolve_module(&spec, &pkg_meta, &ver_meta).expect("resolve");
        let live_bytes = remote_registry
            .file(&pv, &resolved.target.path)
            .expect("live file");
        ver_meta
            .verify_file(&resolved.target.path, &live_bytes)
            .expect("verify live");

        // Now resolve completely offline via CacheRegistry.
        let offline_registry = CacheRegistry::new(Cache::new(dir.path()));
        let off_pkg = PackageMetadata::parse(
            &offline_registry
                .package_metadata(&spec.package)
                .expect("offline pkg meta"),
        )
        .expect("parse offline pkg");
        let off_ver = VersionMetadata::parse(
            &offline_registry
                .version_metadata(&pv)
                .expect("offline ver meta"),
            pv.clone(),
        )
        .expect("parse offline ver");
        let off_resolved = resolve_module(&spec, &off_pkg, &off_ver).expect("offline resolve");
        let off_bytes = offline_registry
            .file(&pv, &off_resolved.target.path)
            .expect("offline file");
        off_ver
            .verify_file(&off_resolved.target.path, &off_bytes)
            .expect("verify offline");
        assert_eq!(off_bytes, MOD_FILE);

        // Missing package returns RegistryError::NotFound.
        let missing = PackageId::new("missing", "pkg").expect("id");
        assert_eq!(
            offline_registry.package_metadata(&missing),
            Err(RegistryError::NotFound)
        );
    }

    #[test]
    fn cache_registry_enforces_max_file_bytes_tombstones_yanked_and_integrity() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pkg_json =
            r#"{"scope":"scope","name":"pkg","versions":{"1.0.0":{},"1.0.1":{"yanked":true}}}"#;
        let yanked_ver_json = format!(
            r#"{{"manifest":{{"/mod.ts":{{"size":{},"checksum":"{MOD_SHA}"}}}},"exports":{{".":"./mod.ts"}}}}"#,
            MOD_FILE.len()
        );
        let tampered_bytes = b"export const value = 9;\n".to_vec();

        let transport: Arc<dyn RemoteTransport> =
            crate::remote::test_transport(move |url, _, _, _| {
                let (status, cache_control, body) = match url {
                    "https://jsr.io/@scope/pkg/meta.json" => {
                        (200, Some("max-age=60"), pkg_json.as_bytes().to_vec())
                    }
                    "https://jsr.io/@scope/pkg/1.0.0_meta.json" => {
                        (200, Some("max-age=60"), VERSION_JSON.as_bytes().to_vec())
                    }
                    "https://jsr.io/@scope/pkg/1.0.1_meta.json" => {
                        (200, Some("max-age=60"), yanked_ver_json.as_bytes().to_vec())
                    }
                    "https://jsr.io/@scope/pkg/1.0.0/mod.ts" => {
                        (200, Some("max-age=60"), MOD_FILE.to_vec())
                    }
                    "https://jsr.io/@scope/pkg/1.0.1/mod.ts" => {
                        (200, Some("max-age=60"), tampered_bytes.clone())
                    }
                    "https://jsr.io/@scope/tomb/meta.json" => {
                        (200, Some("no-store"), pkg_json.as_bytes().to_vec())
                    }
                    other => return Err(format!("unexpected URL: {other}")),
                };
                Ok(TestRemoteResponse {
                    status,
                    cache_control: cache_control.map(str::to_owned),
                    body,
                    ..Default::default()
                })
            });

        let client = RemoteClient::with_transport(
            RemoteOptions::new().with_cache_dir(dir.path()),
            transport,
        );
        let remote_registry = RemoteRegistry::new(client);
        let pkg = PackageId::new("scope", "pkg").expect("pkg");
        let v100 = PackageVersion {
            package: pkg.clone(),
            version: Version::parse("1.0.0").expect("v100"),
        };
        let v101 = PackageVersion {
            package: pkg.clone(),
            version: Version::parse("1.0.1").expect("v101"),
        };
        let tomb_pkg = PackageId::new("scope", "tomb").expect("tomb");

        // Warm all entries (including a `no-store` tombstone on `@scope/tomb`).
        remote_registry.package_metadata(&pkg).expect("pkg");
        remote_registry.version_metadata(&v100).expect("v100 meta");
        remote_registry.version_metadata(&v101).expect("v101 meta");
        remote_registry.file(&v100, "/mod.ts").expect("v100 file");
        remote_registry.file(&v101, "/mod.ts").expect("v101 file");
        remote_registry.package_metadata(&tomb_pkg).expect("tomb");

        let offline = CacheRegistry::new(Cache::new(dir.path()));

        // Range skips yanked `1.0.1` and picks `1.0.0`; exact `1.0.1` selects yanked.
        let off_pkg = PackageMetadata::parse(&offline.package_metadata(&pkg).expect("off pkg"))
            .expect("parse off pkg");
        let ranged = off_pkg
            .select(
                &Specifier::parse("jsr:@scope/pkg@1")
                    .expect("spec")
                    .constraint,
            )
            .expect("ranged");
        assert_eq!(ranged.version.to_string(), "1.0.0");
        assert!(!ranged.yanked);
        let exact_yanked = off_pkg
            .select(
                &Specifier::parse("jsr:@scope/pkg@1.0.1")
                    .expect("spec")
                    .constraint,
            )
            .expect("exact yanked");
        assert_eq!(exact_yanked.version.to_string(), "1.0.1");
        assert!(exact_yanked.yanked);

        // Tombstoned entry returns `RegistryError::NotFound`.
        assert_eq!(
            offline.package_metadata(&tomb_pkg),
            Err(RegistryError::NotFound)
        );

        // Tampered cached file fails `VersionMetadata::verify_file`.
        let off_v101_meta = VersionMetadata::parse(
            &offline.version_metadata(&v101).expect("off v101 meta"),
            v101.clone(),
        )
        .expect("parse v101");
        let bad_bytes = offline.file(&v101, "/mod.ts").expect("load tampered bytes");
        let verify_err = off_v101_meta
            .verify_file("/mod.ts", &bad_bytes)
            .expect_err("checksum mismatch must fail");
        assert!(matches!(verify_err, Error::Integrity(_)), "{verify_err}");

        // `with_max_file_bytes` rejects an oversized cached file as `NotFound`.
        let small = CacheRegistry::with_max_file_bytes(Cache::new(dir.path()), 8);
        assert_eq!(small.file(&v100, "/mod.ts"), Err(RegistryError::NotFound));
    }

    #[test]
    fn remote_registry_maps_404_and_410_to_not_found() {
        for status in [404u16, 410] {
            let transport = crate::remote::test_transport(move |_, _, _, _| {
                Ok(TestRemoteResponse {
                    status,
                    cache_control: Some("no-store".to_string()),
                    ..Default::default()
                })
            });
            let client =
                RemoteClient::with_transport(RemoteOptions::new().without_cache(), transport);
            let registry = RemoteRegistry::new(client);
            let pkg = PackageId::new("scope", "missing").expect("pkg");
            assert_eq!(
                registry.package_metadata(&pkg),
                Err(RegistryError::NotFound)
            );
        }

        let transport = crate::remote::test_transport(|_, _, _, _| {
            Ok(TestRemoteResponse {
                status: 503,
                cache_control: Some("no-store".to_string()),
                ..Default::default()
            })
        });
        let client = RemoteClient::with_transport(RemoteOptions::new().without_cache(), transport);
        let registry = RemoteRegistry::new(client);
        let pkg = PackageId::new("scope", "down").expect("pkg");
        let err = registry.package_metadata(&pkg).unwrap_err();
        assert!(matches!(err, RegistryError::Unavailable(_)), "{err:?}");
    }
}
