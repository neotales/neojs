//! Packument parsing, `NpmRegistry` trait, peer-context-isolated
//! `NpmGraphResolver`, verified module loader, and `ProjectPackageResolver`
//! for bare imports from `package.json` without `node_modules`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use url::Url;
use vaeljs_module_policy::{ModuleFormat, PackageType, package_module_format};

use crate::cas::{CasError, CasStore, sha256_hex};
use crate::exports::{ExportsError, ExportsResolver};
use crate::lockfile::{LockedPackage, Lockfile, LockfileError};
use crate::npmrc::{NpmrcConfig, NpmrcError};
use crate::package_json::{
    MAX_PACKAGE_JSON_BYTES, PackageJson, PackageJsonError, PeerDependencyMeta, ScriptAudit,
    TargetPlatform,
};
use crate::semver::{NpmRange, NpmVersion, SemverError, VersionConstraint};
use crate::specifier::{
    NpmSpecifier, PackageName, SpecifierError, build_virtual_url, parse_virtual_url,
    split_bare_package_specifier, validate_subpath,
};
use crate::tarball::{NpmLimits, PackageTree, TarballError, TarballIngestor, parse_sha512_sri};

/// Default public npm registry origin.
pub const DEFAULT_REGISTRY_ORIGIN: &str = "https://registry.npmjs.org";

/// Maximum directory depth walked upward when discovering a project `package.json`.
pub const MAX_DISCOVERY_DEPTH: usize = 64;

/// Failure returned by registry, graph resolution, module loading, or project
/// `package.json` bare-import resolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResolverError {
    /// Specifier parsing or virtual URL validation failed.
    #[error("{0}")]
    Specifier(#[from] SpecifierError),

    /// SemVer constraint or version parsing failed.
    #[error("{0}")]
    Semver(#[from] SemverError),

    /// `package.json` parsing or validation failed.
    #[error("{0}")]
    PackageJson(#[from] PackageJsonError),

    /// `exports`, `#imports`, or strict ESM classification failed.
    #[error("{0}")]
    Exports(#[from] ExportsError),

    /// Tarball verification, extraction, or `.tree` error.
    #[error("{0}")]
    Tarball(#[from] TarballError),

    /// CAS verification or I/O error.
    #[error("{0}")]
    Cas(#[from] CasError),

    /// Registry fetch failed (network, HTTP status, or malformed packument).
    #[error("registry error for `{package}`: {reason}")]
    Registry { package: String, reason: String },

    /// No published version satisfied the requested SemVer range or dist-tag.
    #[error("no version of `{package}` matches `{constraint}`")]
    NoMatchingVersion { package: String, constraint: String },

    /// Required dependency did not support the target OS/CPU/libc.
    #[error("package `{package}@{version}` does not support target platform ({os}/{cpu})")]
    UnsupportedPlatform {
        package: String,
        version: String,
        os: String,
        cpu: String,
    },

    /// Required peer dependency was absent from the importing context.
    #[error("package `{package}@{version}` requires peer `{peer}@{range}`, which is missing")]
    MissingPeerDependency {
        package: String,
        version: String,
        peer: String,
        range: String,
    },

    /// Available peer version did not satisfy the package's `peerDependencies` range.
    #[error(
        "package `{package}@{version}` requires peer `{peer}@{range}`, found incompatible `{found_version}`"
    )]
    PeerDependencyMismatch {
        package: String,
        version: String,
        peer: String,
        range: String,
        found_version: String,
    },

    /// Resolved package tarball `package.json` `"name"` did not match the requested package.
    #[error("tarball package.json name mismatch: expected `{expected}`, got `{actual:?}`")]
    PackageIdentityMismatch {
        expected: String,
        actual: Option<String>,
    },

    /// Resolved package tarball `package.json` `"version"` did not match the locked version.
    #[error(
        "tarball package.json version mismatch for `{package}`: expected `{expected}`, got `{actual}`"
    )]
    PackageVersionMismatch {
        package: String,
        expected: String,
        actual: String,
    },

    /// Graph exceeded [`NpmLimits::max_graph_packages`].
    #[error("npm dependency graph exceeded {limit}-package limit")]
    GraphLimitExceeded { limit: usize },

    /// Referrer tried to import an undeclared bare package or disallowed scheme.
    #[error("disallowed import `{specifier}` from `{referrer}`: {reason}")]
    DisallowedImport {
        referrer: String,
        specifier: String,
        reason: String,
    },

    /// A resolved package instance or file was not found in [`ResolvedNpmGraph`].
    #[error("unresolved npm graph target: {reason}")]
    UnresolvedTarget { reason: String },

    /// Project `package.json` discovery or read error.
    #[error("project package.json error: {reason}")]
    ProjectManifest { reason: String },

    /// Lockfile parse, read, write, or integrity verification error.
    #[error("{0}")]
    Lockfile(#[from] LockfileError),

    /// `.npmrc` configuration or registry authentication error.
    #[error("{0}")]
    Npmrc(#[from] NpmrcError),

    /// Resolution refused because `--frozen-lockfile` requires an existing lockfile entry.
    #[error("frozen lockfile violation: {reason}")]
    FrozenLockfile { reason: String },

    /// Resolution refused because offline/non-remote mode cannot fetch missing packages.
    #[error("offline npm resolution failed: {reason}")]
    Offline { reason: String },
}

/// Distribution metadata for one version inside an npm packument.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PackumentDist {
    /// Tarball download URL.
    pub tarball: String,
    /// Subresource integrity digest (must include `sha512-...`).
    pub integrity: Option<String>,
    /// Legacy SHA-1 hex digest (never sufficient by itself).
    pub shasum: Option<String>,
}

/// One version entry inside an abbreviated or full npm packument.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackumentVersion {
    /// Package version string.
    pub version: String,
    /// Distribution tarball URL and integrity.
    pub dist: PackumentDist,
    /// Regular dependencies.
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    /// Optional dependencies.
    #[serde(default)]
    pub optional_dependencies: BTreeMap<String, String>,
    /// Peer dependencies.
    #[serde(default)]
    pub peer_dependencies: BTreeMap<String, String>,
    /// Optionality metadata for peer dependencies.
    #[serde(default)]
    pub peer_dependencies_meta: BTreeMap<String, PeerDependencyMeta>,
    /// Supported OS list.
    #[serde(default)]
    pub os: Option<Vec<String>>,
    /// Supported CPU list.
    #[serde(default)]
    pub cpu: Option<Vec<String>>,
    /// Supported libc list.
    #[serde(default)]
    pub libc: Option<Vec<String>>,
}

/// Parsed abbreviated npm packument (`application/vnd.npm.install-v1+json`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Packument {
    /// Package name.
    pub name: String,
    /// Dist-tags map (`"latest" -> "1.2.3"`).
    #[serde(rename = "dist-tags", default)]
    pub dist_tags: BTreeMap<String, String>,
    /// Map of version string -> version metadata.
    #[serde(default)]
    pub versions: BTreeMap<String, PackumentVersion>,
}

impl Packument {
    /// Parses a packument JSON payload bounded by `max_bytes`.
    pub fn parse_bytes(bytes: &[u8], max_bytes: usize) -> Result<Self, ResolverError> {
        if bytes.len() > max_bytes {
            return Err(ResolverError::Registry {
                package: "<packument>".to_owned(),
                reason: format!("packument exceeds {max_bytes}-byte limit"),
            });
        }
        serde_json::from_slice(bytes).map_err(|err| ResolverError::Registry {
            package: "<packument>".to_owned(),
            reason: format!("invalid packument JSON: {err}"),
        })
    }

    /// Selects the matching version for `constraint` from this packument.
    pub fn select_version<'a>(
        &'a self,
        package: &PackageName,
        constraint: &VersionConstraint,
    ) -> Result<(&'a PackumentVersion, NpmVersion), ResolverError> {
        match constraint {
            VersionConstraint::DistTag(tag) => {
                let ver_str =
                    self.dist_tags
                        .get(tag)
                        .ok_or_else(|| ResolverError::NoMatchingVersion {
                            package: package.to_string(),
                            constraint: tag.clone(),
                        })?;
                let meta =
                    self.versions
                        .get(ver_str)
                        .ok_or_else(|| ResolverError::NoMatchingVersion {
                            package: package.to_string(),
                            constraint: tag.clone(),
                        })?;
                let parsed = NpmVersion::parse(&meta.version)?;
                Ok((meta, parsed))
            }
            VersionConstraint::Range(range) => {
                let mut best: Option<(&PackumentVersion, NpmVersion)> = None;
                for meta in self.versions.values() {
                    let Ok(parsed) = NpmVersion::parse(&meta.version) else {
                        continue;
                    };
                    if range.matches(&parsed) {
                        let replace = match &best {
                            None => true,
                            Some((_, cur)) => &parsed > cur,
                        };
                        if replace {
                            best = Some((meta, parsed));
                        }
                    }
                }
                best.ok_or_else(|| ResolverError::NoMatchingVersion {
                    package: package.to_string(),
                    constraint: range.to_string(),
                })
            }
        }
    }
}

/// Engine-neutral registry backend used by [`NpmGraphResolver`].
///
/// Implementations may read from an offline cache, a test mock, or (when the
/// `network` feature is enabled) [`RemoteNpmRegistry`].
pub trait NpmRegistry {
    /// Returns the registry origin URL (e.g. `"https://registry.npmjs.org"`).
    fn registry_origin(&self) -> &str;

    /// Returns the registry origin URL (without trailing `/`) used for
    /// `package_name`. Defaults to [`Self::registry_origin`].
    fn registry_origin_for_package(&self, _package_name: &PackageName) -> &str {
        self.registry_origin()
    }

    /// Fetches the abbreviated packument for `package_name`.
    fn fetch_packument(&self, package_name: &PackageName) -> Result<Packument, ResolverError>;

    /// Fetches raw `.tgz` bytes for `tarball_url` (expected to match `integrity`).
    fn fetch_tarball(&self, tarball_url: &str, integrity: &str) -> Result<Vec<u8>, ResolverError>;
}

/// [`NpmRegistry`] implementation backed by [`vaeljs_remote::RemoteClient`]
/// and optional `.npmrc` scoped registry / authentication rules.
///
/// Available via [`RemoteNpmRegistry::with_client`] in all builds, and via
/// `RemoteNpmRegistry::new` when the `network` feature is enabled.
#[derive(Clone)]
pub struct RemoteNpmRegistry {
    origin: String,
    origin_overridden: bool,
    client: vaeljs_remote::RemoteClient,
    limits: NpmLimits,
    npmrc: NpmrcConfig,
}

impl RemoteNpmRegistry {
    /// Creates a live registry client against `https://registry.npmjs.org`.
    #[cfg(feature = "network")]
    pub fn new(options: vaeljs_remote::RemoteOptions) -> Self {
        Self::with_client(
            DEFAULT_REGISTRY_ORIGIN,
            vaeljs_remote::RemoteClient::new(options),
        )
    }

    /// Creates a registry client backed by a caller-provided [`vaeljs_remote::RemoteClient`].
    pub fn with_client(origin: impl Into<String>, client: vaeljs_remote::RemoteClient) -> Self {
        let trimmed = origin.into().trim_end_matches('/').to_owned();
        let origin_overridden = trimmed != DEFAULT_REGISTRY_ORIGIN;
        Self {
            origin: trimmed,
            origin_overridden,
            client,
            limits: NpmLimits::default(),
            npmrc: NpmrcConfig::default(),
        }
    }

    /// Creates a registry client configured with `.npmrc` routing and
    /// HTTPS-only authentication rules.
    ///
    /// When `origin_overridden` is `false` and `npmrc` specifies `registry=...`,
    /// the `.npmrc` default registry replaces `origin`.
    pub fn with_client_and_npmrc(
        origin: impl Into<String>,
        client: vaeljs_remote::RemoteClient,
        npmrc: NpmrcConfig,
        origin_overridden: bool,
    ) -> Self {
        let trimmed = origin.into().trim_end_matches('/').to_owned();
        let effective_origin = if !origin_overridden && npmrc.has_explicit_default_registry() {
            npmrc.default_registry_origin().to_owned()
        } else {
            trimmed
        };
        Self {
            origin: effective_origin,
            origin_overridden,
            client,
            limits: NpmLimits::default(),
            npmrc,
        }
    }

    /// Applies `.npmrc` scoped registry routing and HTTPS-only auth rules.
    pub fn with_npmrc(mut self, npmrc: NpmrcConfig) -> Self {
        if !self.origin_overridden && npmrc.has_explicit_default_registry() {
            self.origin = npmrc.default_registry_origin().to_owned();
        }
        self.npmrc = npmrc;
        self
    }

    /// Returns the attached [`NpmrcConfig`].
    pub fn npmrc(&self) -> &NpmrcConfig {
        &self.npmrc
    }

    /// Overrides resource limits used when parsing packuments.
    pub fn with_limits(mut self, limits: NpmLimits) -> Self {
        self.limits = limits;
        self
    }

    fn base_registry_url_for_package(
        &self,
        package_name: &PackageName,
    ) -> Result<Url, ResolverError> {
        if let Some(scoped) = self
            .npmrc
            .scoped_registry_for_package(package_name.as_str())
        {
            return Ok(scoped.clone());
        }
        if !self.origin_overridden && self.npmrc.has_explicit_default_registry() {
            return Ok(self.npmrc.default_registry().clone());
        }
        let base = format!("{}/", self.origin.trim_end_matches('/'));
        Url::parse(&base).map_err(|err| ResolverError::Registry {
            package: package_name.to_string(),
            reason: err.to_string(),
        })
    }
}

impl NpmRegistry for RemoteNpmRegistry {
    fn registry_origin(&self) -> &str {
        &self.origin
    }

    fn registry_origin_for_package(&self, package_name: &PackageName) -> &str {
        if let Some(scoped) = self
            .npmrc
            .scoped_registry_origin_for_package(package_name.as_str())
        {
            return scoped;
        }
        if !self.origin_overridden && self.npmrc.has_explicit_default_registry() {
            return self.npmrc.default_registry_origin();
        }
        &self.origin
    }

    fn fetch_packument(&self, package_name: &PackageName) -> Result<Packument, ResolverError> {
        let encoded_name = if let Some(scope) = package_name.scope() {
            format!("@{scope}%2f{}", package_name.unscoped_name())
        } else {
            package_name.as_str().to_owned()
        };
        let base_url = self.base_registry_url_for_package(package_name)?;
        let url = base_url
            .join(&encoded_name)
            .map_err(|err| ResolverError::Registry {
                package: package_name.to_string(),
                reason: err.to_string(),
            })?;
        let matched = self.npmrc.auth_with_scope_for_url(&url)?;
        let auth_header = matched.map(|(auth, _)| auth.authorization_header_value());
        let matched_prefix = matched.map(|(_, prefix)| prefix);
        let fetched = match (auth_header.as_deref(), matched_prefix) {
            (Some(header), Some(prefix)) => {
                self.client
                    .fetch_with_scoped_authorization(&url, Some(header.as_str()), &|next| {
                        NpmrcConfig::path_prefix_matches(prefix, next)
                    })
            }
            _ => self.client.fetch_with_authorization(&url, None),
        }
        .map_err(|err| ResolverError::Registry {
            package: package_name.to_string(),
            reason: err.to_string(),
        })?;
        Packument::parse_bytes(&fetched.body, self.limits.max_packument_bytes)
    }

    fn fetch_tarball(&self, tarball_url: &str, _integrity: &str) -> Result<Vec<u8>, ResolverError> {
        let url = Url::parse(tarball_url).map_err(|err| ResolverError::Registry {
            package: redact_tarball_url_str(tarball_url),
            reason: err.to_string(),
        })?;
        let display = vaeljs_remote::display_url(&url);
        let matched = self.npmrc.auth_with_scope_for_url(&url)?;
        let auth_header = matched.map(|(auth, _)| auth.authorization_header_value());
        let matched_prefix = matched.map(|(_, prefix)| prefix);
        let fetched = match (auth_header.as_deref(), matched_prefix) {
            (Some(header), Some(prefix)) => {
                self.client
                    .fetch_with_scoped_authorization(&url, Some(header.as_str()), &|next| {
                        NpmrcConfig::path_prefix_matches(prefix, next)
                    })
            }
            _ => self.client.fetch_with_authorization(&url, None),
        }
        .map_err(|err| ResolverError::Registry {
            package: display,
            reason: err.to_string(),
        })?;
        Ok(fetched.body)
    }
}

fn redact_tarball_url_str(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(url) => vaeljs_remote::display_url(&url),
        Err(_) => "<invalid tarball URL>".to_owned(),
    }
}

/// One root package dependency request passed to [`NpmGraphResolver::resolve_requests`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpmRootRequest {
    /// Target package name.
    pub name: PackageName,
    /// SemVer range or dist-tag constraint.
    pub constraint: VersionConstraint,
    /// Whether omission is allowed on platform incompatibility.
    pub optional: bool,
}

impl NpmRootRequest {
    /// Builds a required root request from a parsed [`NpmSpecifier`].
    pub fn from_specifier(spec: &NpmSpecifier) -> Result<Self, ResolverError> {
        Ok(Self {
            name: spec.name().clone(),
            constraint: spec.constraint()?,
            optional: false,
        })
    }
}

/// One resolved, peer-context-isolated npm package instance in [`ResolvedNpmGraph`].
#[derive(Clone, Debug)]
pub struct ResolvedNpmPackage {
    /// Canonical instance identifier `<name>@<version>~<ctx>`.
    pub instance_id: String,
    /// Registry host (`registry.npmjs.org`).
    pub registry_host: String,
    /// Registry origin (`https://registry.npmjs.org`).
    pub registry_origin: String,
    /// Validated package name.
    pub name: PackageName,
    /// Selected SemVer version.
    pub version: String,
    /// Peer-context token (`"0"` or `"p<32-hex>"`).
    pub ctx: String,
    /// Tarball download URL.
    pub tarball_url: String,
    /// Verified tarball `sha512-...` SRI string.
    pub integrity: String,
    /// Canonical `.tree` manifest SHA-256 hex digest.
    pub tree_sha256: String,
    /// Verified `.tree` file manifest.
    pub tree: PackageTree,
    /// Verified root `package.json`.
    pub package_json: PackageJson,
    /// Resolved `dependencies` (`dep_name -> instance_id`).
    pub dependencies: BTreeMap<PackageName, String>,
    /// Resolved `optionalDependencies` (`dep_name -> Some(instance_id)` or `None` if omitted).
    pub optional_dependencies: BTreeMap<PackageName, Option<String>>,
    /// Bound `peerDependencies` (`peer_name -> Some(instance_id)` or `None` if optional omitted).
    pub peer_dependencies: BTreeMap<PackageName, Option<String>>,
    /// Lifecycle script and native binary audit metadata (scripts are never executed).
    pub script_audit: ScriptAudit,
}

impl ResolvedNpmPackage {
    /// Looks up the target `instance_id` for a declared dependency, optional dependency, or peer.
    pub fn resolve_declared_dep(&self, name: &PackageName) -> Option<&str> {
        if let Some(id) = self.dependencies.get(name) {
            return Some(id.as_str());
        }
        if let Some(Some(id)) = self.optional_dependencies.get(name) {
            return Some(id.as_str());
        }
        if let Some(Some(id)) = self.peer_dependencies.get(name) {
            return Some(id.as_str());
        }
        None
    }

    /// Builds the canonical virtual `npm://` [`Url`] for `rel_path` inside this instance.
    pub fn virtual_url(&self, rel_path: &str) -> Result<Url, ResolverError> {
        Ok(build_virtual_url(
            &self.registry_host,
            &self.name,
            &self.version,
            &self.ctx,
            rel_path,
        )?)
    }
}

/// Verified module source bytes and ESM classification returned by
/// [`ResolvedNpmGraph::load_module_verified`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedNpmModule {
    /// Canonical virtual `npm://` URL.
    pub url: Url,
    /// Exact verified UTF-8/raw file bytes from [`CasStore`].
    pub bytes: Vec<u8>,
    /// Classified format (`JavaScript` or `TypeScript`).
    pub format: ModuleFormat,
    /// Nearest `package.json` `"type"`.
    pub package_type: PackageType,
}

/// Outcome of resolving an import specifier from inside an `npm://` referrer module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NpmReferrerResolution {
    /// Resolved to another virtual `npm://` module inside the graph.
    NpmUrl(Url),
    /// A canonical `vael:*` built-in specifier (never intercepted by npm).
    VaelBuiltin(String),
    /// A canonical `node:*` built-in specifier (handled only if `node-compat` is enabled).
    NodeBuiltin(String),
}

/// Resolved, peer-context-isolated npm package graph backed by [`CasStore`].
#[derive(Clone, Debug)]
pub struct ResolvedNpmGraph {
    registry_origin: String,
    registry_host: String,
    cas: CasStore,
    /// Maps root request `(name, constraint_str)` -> resolved `instance_id` (or `None` if omitted optional root).
    roots: BTreeMap<(PackageName, String), Option<String>>,
    /// All canonical package instances keyed by `instance_id` (`name@version~ctx`).
    instances: BTreeMap<String, ResolvedNpmPackage>,
    /// Quick lookup `(registry_host, name, version, ctx)` -> `instance_id`.
    by_url_identity: BTreeMap<(String, PackageName, String, String), String>,
}

impl ResolvedNpmGraph {
    /// Creates an empty [`ResolvedNpmGraph`] for `registry_origin` and `cas`.
    pub fn new(registry_origin: impl Into<String>, cas: CasStore) -> Result<Self, ResolverError> {
        let origin = registry_origin.into().trim_end_matches('/').to_owned();
        let origin_url = Url::parse(&origin).map_err(|err| ResolverError::Registry {
            package: "<registry>".to_owned(),
            reason: err.to_string(),
        })?;
        let registry_host = origin_url
            .host_str()
            .ok_or_else(|| ResolverError::Registry {
                package: "<registry>".to_owned(),
                reason: "registry origin has no host".to_owned(),
            })?
            .to_owned();
        Ok(Self {
            registry_origin: origin,
            registry_host,
            cas,
            roots: BTreeMap::new(),
            instances: BTreeMap::new(),
            by_url_identity: BTreeMap::new(),
        })
    }

    /// Returns the registry origin URL used for this graph.
    pub fn registry_origin(&self) -> &str {
        &self.registry_origin
    }

    /// Returns the registry host used in virtual `npm://` URLs.
    pub fn registry_host(&self) -> &str {
        &self.registry_host
    }

    /// Returns all root request mappings `(name, constraint_str) -> Option<instance_id>`.
    pub fn roots(&self) -> &BTreeMap<(PackageName, String), Option<String>> {
        &self.roots
    }

    /// Returns all canonical resolved package instances keyed by `instance_id`.
    pub fn instances(&self) -> &BTreeMap<String, ResolvedNpmPackage> {
        &self.instances
    }

    /// Merges all roots and package instances from `other` into `self`,
    /// preserving any already-resolved instances.
    pub fn merge(&mut self, other: Self) {
        for (k, v) in other.roots {
            self.roots.entry(k).or_insert(v);
        }
        for (k, v) in other.by_url_identity {
            self.by_url_identity.entry(k).or_insert(v);
        }
        for (k, v) in other.instances {
            self.instances.entry(k).or_insert(v);
        }
    }

    /// Returns the collected [`ScriptAudit`] records across all resolved instances.
    pub fn script_audits(&self) -> Vec<&ScriptAudit> {
        self.instances.values().map(|p| &p.script_audit).collect()
    }

    /// Looks up the root `instance_id` resolved for `(name, constraint_str)`.
    pub fn root_instance_id(&self, name: &PackageName, constraint_str: &str) -> Option<&str> {
        self.roots
            .get(&(name.clone(), constraint_str.to_owned()))
            .and_then(|opt| opt.as_deref())
    }

    /// Resolves a root [`NpmSpecifier`] to its canonical virtual `npm://` [`Url`],
    /// validating `exports` / `main` and strict ESM classification.
    pub fn resolve_root_specifier(&self, spec: &NpmSpecifier) -> Result<Url, ResolverError> {
        let constraint = spec.effective_constraint_str();
        let instance_id = self
            .root_instance_id(spec.name(), constraint)
            .ok_or_else(|| ResolverError::UnresolvedTarget {
                reason: format!("root `{}` not present in resolved graph", spec.canonical()),
            })?;
        self.resolve_instance_entry(instance_id, spec.subpath())
    }

    /// Resolves a package `instance_id` and optional `subpath` to a virtual `npm://` URL.
    pub fn resolve_instance_entry(
        &self,
        instance_id: &str,
        subpath: Option<&str>,
    ) -> Result<Url, ResolverError> {
        let pkg =
            self.instances
                .get(instance_id)
                .ok_or_else(|| ResolverError::UnresolvedTarget {
                    reason: format!("unknown package instance `{instance_id}`"),
                })?;
        let file = ExportsResolver::resolve_package_entry(
            &pkg.tree,
            &self.cas,
            &pkg.package_json,
            pkg.name.as_str(),
            subpath,
        )?;
        pkg.virtual_url(&file.rel_path)
    }

    /// Resolves an import `specifier` issued from an existing `npm://` `referrer` URL.
    ///
    /// Enforces strict graph boundaries:
    /// - `./...` / `../...` relative paths constrained inside `<pkg>/<version>/<ctx>/`
    /// - `#...` internal/declared imports via `"imports"`
    /// - Self-reference (when `"exports"` is present) or declared dependencies/peers
    /// - Same-instance `npm://...` URLs (supports `new URL("./sub.mjs", import.meta.url).href`)
    /// - `vael:*` and `node:*` returned unintercepted as built-ins
    /// - Refuses `file:`, `http:`, `https:`, `gh:`, `github:`, `jsr:`, `data:`, `blob:`,
    ///   cross-instance `npm://`, and undeclared bare/npm specifiers.
    pub fn resolve_from_npm_referrer(
        &self,
        referrer: &Url,
        specifier: &str,
    ) -> Result<NpmReferrerResolution, ResolverError> {
        if has_scheme_prefix_ci(specifier, "vael:") {
            return Ok(NpmReferrerResolution::VaelBuiltin(specifier.to_owned()));
        }
        if has_scheme_prefix_ci(specifier, "node:") {
            return Ok(NpmReferrerResolution::NodeBuiltin(specifier.to_owned()));
        }

        let parsed_ref = parse_virtual_url(referrer)?;
        let ref_instance_id = self
            .by_url_identity
            .get(&(
                parsed_ref.registry_host.clone(),
                parsed_ref.name.clone(),
                parsed_ref.version.clone(),
                parsed_ref.ctx.clone(),
            ))
            .ok_or_else(|| ResolverError::UnresolvedTarget {
                reason: format!("referrer `{referrer}` is not in resolved graph"),
            })?;
        let ref_pkg = &self.instances[ref_instance_id];

        // Relative import inside the same package instance.
        if specifier.starts_with("./") || specifier.starts_with("../") {
            let joined =
                referrer
                    .join(specifier)
                    .map_err(|err| ResolverError::DisallowedImport {
                        referrer: referrer.to_string(),
                        specifier: specifier.to_owned(),
                        reason: err.to_string(),
                    })?;
            let parsed_target =
                parse_virtual_url(&joined).map_err(|err| ResolverError::DisallowedImport {
                    referrer: referrer.to_string(),
                    specifier: specifier.to_owned(),
                    reason: err.to_string(),
                })?;
            if !parsed_ref.same_instance(&parsed_target) {
                return Err(ResolverError::DisallowedImport {
                    referrer: referrer.to_string(),
                    specifier: specifier.to_owned(),
                    reason: "relative import escaped package root".to_owned(),
                });
            }
            let file = ExportsResolver::classify_file(
                &ref_pkg.tree,
                &self.cas,
                ref_pkg.name.as_str(),
                &parsed_target.rel_path,
            )?;
            return Ok(NpmReferrerResolution::NpmUrl(
                ref_pkg.virtual_url(&file.rel_path)?,
            ));
        }

        // Same-instance `npm://` URL (e.g. `import(new URL("./sub.mjs", import.meta.url).href)`).
        if specifier.starts_with("npm://") {
            let target_url =
                Url::parse(specifier).map_err(|err| ResolverError::DisallowedImport {
                    referrer: referrer.to_string(),
                    specifier: specifier.to_owned(),
                    reason: err.to_string(),
                })?;
            let parsed_target = parse_virtual_url(&target_url)?;
            if !parsed_ref.same_instance(&parsed_target) {
                return Err(ResolverError::DisallowedImport {
                    referrer: referrer.to_string(),
                    specifier: specifier.to_owned(),
                    reason: "cross-package `npm://` URL imports are forbidden".to_owned(),
                });
            }
            let file = ExportsResolver::classify_file(
                &ref_pkg.tree,
                &self.cas,
                ref_pkg.name.as_str(),
                &parsed_target.rel_path,
            )?;
            return Ok(NpmReferrerResolution::NpmUrl(
                ref_pkg.virtual_url(&file.rel_path)?,
            ));
        }

        // `#...` subpath import against this package's `"imports"`.
        if specifier.starts_with('#') {
            let resolved = ExportsResolver::resolve_package_import(
                &ref_pkg.tree,
                &self.cas,
                &ref_pkg.package_json,
                ref_pkg.name.as_str(),
                specifier,
            )?;
            match resolved {
                crate::exports::PackageImportTarget::InternalFile(file) => {
                    return Ok(NpmReferrerResolution::NpmUrl(
                        ref_pkg.virtual_url(&file.rel_path)?,
                    ));
                }
                crate::exports::PackageImportTarget::ExternalPackage { name, subpath } => {
                    let target_id = ref_pkg.resolve_declared_dep(&name).ok_or_else(|| {
                        ResolverError::DisallowedImport {
                            referrer: referrer.to_string(),
                            specifier: specifier.to_owned(),
                            reason: format!(
                                "external `#imports` target `{name}` was omitted or unresolved"
                            ),
                        }
                    })?;
                    let url = self.resolve_instance_entry(target_id, subpath.as_deref())?;
                    return Ok(NpmReferrerResolution::NpmUrl(url));
                }
            }
        }

        // Reject all disallowed schemes and filesystem paths inside npm modules.
        if specifier.starts_with('/') || specifier.contains(':') || specifier.contains('\\') {
            return Err(ResolverError::DisallowedImport {
                referrer: referrer.to_string(),
                specifier: specifier.to_owned(),
                reason: "scheme or absolute path is not allowed inside npm: packages".to_owned(),
            });
        }

        // Bare specifier: self-reference or declared dependency/optional/peer.
        let (dep_name, subpath) = split_bare_package_specifier(specifier)?;
        if dep_name == ref_pkg.name && ref_pkg.package_json.exports.is_some() {
            let url = self.resolve_instance_entry(ref_instance_id, subpath.as_deref())?;
            return Ok(NpmReferrerResolution::NpmUrl(url));
        }

        let target_id = ref_pkg.resolve_declared_dep(&dep_name).ok_or_else(|| {
            ResolverError::DisallowedImport {
                referrer: referrer.to_string(),
                specifier: specifier.to_owned(),
                reason: format!(
                    "package `{dep_name}` is not declared in dependencies, optionalDependencies, or peerDependencies of `{}`",
                    ref_pkg.name
                ),
            }
        })?;
        let url = self.resolve_instance_entry(target_id, subpath.as_deref())?;
        Ok(NpmReferrerResolution::NpmUrl(url))
    }

    /// Reads and content-verifies a resolved virtual `npm://` URL directly from
    /// [`CasStore`], returning owned bytes and strict ESM classification.
    pub fn load_module_verified(&self, url: &Url) -> Result<LoadedNpmModule, ResolverError> {
        let parsed = parse_virtual_url(url)?;
        let instance_id = self
            .by_url_identity
            .get(&(
                parsed.registry_host,
                parsed.name,
                parsed.version,
                parsed.ctx,
            ))
            .ok_or_else(|| ResolverError::UnresolvedTarget {
                reason: format!("module URL `{url}` does not belong to resolved graph"),
            })?;
        let pkg = &self.instances[instance_id];

        // Verify .tree on disk matches expected tree_sha256 before trusting file metadata.
        let _verified_tree = PackageTree::load_verified(
            self.cas.cache_root(),
            &pkg.registry_origin,
            &pkg.integrity,
            Some(&pkg.tree_sha256),
        )?
        .ok_or_else(|| ResolverError::UnresolvedTarget {
            reason: format!("missing .tree manifest for `{instance_id}`"),
        })?;

        let classified = ExportsResolver::classify_file(
            &pkg.tree,
            &self.cas,
            pkg.name.as_str(),
            &parsed.rel_path,
        )?;
        let entry =
            pkg.tree
                .get_file(&parsed.rel_path)
                .ok_or_else(|| ExportsError::MissingTargetFile {
                    package: pkg.name.to_string(),
                    target: parsed.rel_path.clone(),
                })?;
        let bytes = self.cas.read_verified(&entry.sha256, entry.size)?;
        let canonical_url = pkg.virtual_url(&classified.rel_path)?;
        Ok(LoadedNpmModule {
            url: canonical_url,
            bytes,
            format: classified.format,
            package_type: classified.package_type,
        })
    }
}

/// Engine-neutral peer-context-isolated npm dependency graph resolver.
pub struct NpmGraphResolver<'a, R: NpmRegistry + ?Sized> {
    registry: &'a R,
    ingestor: TarballIngestor,
    limits: NpmLimits,
    platform: TargetPlatform,
}

impl<'a, R: NpmRegistry + ?Sized> NpmGraphResolver<'a, R> {
    /// Creates a resolver backed by `registry` and `cas` using default limits
    /// and [`TargetPlatform::current`].
    pub fn new(registry: &'a R, cas: CasStore) -> Self {
        let limits = NpmLimits::default();
        Self {
            registry,
            ingestor: TarballIngestor::new(cas, limits),
            limits,
            platform: TargetPlatform::current(),
        }
    }

    /// Overrides resource limits.
    pub fn with_limits(mut self, limits: NpmLimits) -> Self {
        self.ingestor = TarballIngestor::new(self.ingestor.cas().clone(), limits);
        self.limits = limits;
        self
    }

    /// Overrides the target platform used for `os`/`cpu`/`libc` checks.
    pub fn with_platform(mut self, platform: TargetPlatform) -> Self {
        self.platform = platform;
        self
    }

    /// Resolves a list of `npm:` specifiers with mutual sibling peer visibility.
    pub fn resolve_specifiers(
        &self,
        specifiers: &[NpmSpecifier],
    ) -> Result<ResolvedNpmGraph, ResolverError> {
        let requests: Vec<NpmRootRequest> = specifiers
            .iter()
            .map(NpmRootRequest::from_specifier)
            .collect::<Result<_, _>>()?;
        self.resolve_requests(&requests)
    }

    /// Resolves a list of [`NpmRootRequest`]s with mutual sibling peer visibility.
    pub fn resolve_requests(
        &self,
        requests: &[NpmRootRequest],
    ) -> Result<ResolvedNpmGraph, ResolverError> {
        let origin = self
            .registry
            .registry_origin()
            .trim_end_matches('/')
            .to_owned();
        let origin_url = Url::parse(&origin).map_err(|err| ResolverError::Registry {
            package: "<registry>".to_owned(),
            reason: err.to_string(),
        })?;
        let registry_host = origin_url
            .host_str()
            .ok_or_else(|| ResolverError::Registry {
                package: "<registry>".to_owned(),
                reason: "registry origin has no host".to_owned(),
            })?
            .to_owned();

        let mut state = ResolutionState {
            resolver: self,
            packuments: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            nodes: Vec::new(),
            active_stack: Vec::new(),
        };

        // Step 1: Select and load root artifacts upfront so sibling roots can see
        // each other regardless of import order (e.g. `react-dom` before `react`).
        struct PreparedRoot {
            request: NpmRootRequest,
            artifact_key: Option<String>,
            pass: u8,
        }
        let mut prepared = Vec::with_capacity(requests.len());

        for req in requests {
            let (meta, _) = state.select_version(&req.name, &req.constraint)?;
            let art = state.load_artifact(&req.name, &meta)?;
            if !self.platform.matches(
                art.package_json.os.as_deref(),
                art.package_json.cpu.as_deref(),
                art.package_json.libc.as_deref(),
            ) {
                if req.optional {
                    prepared.push(PreparedRoot {
                        request: req.clone(),
                        artifact_key: None,
                        pass: 0,
                    });
                    continue;
                }
                return Err(ResolverError::UnsupportedPlatform {
                    package: req.name.to_string(),
                    version: meta.version.clone(),
                    os: self.platform.os.clone(),
                    cpu: self.platform.cpu.clone(),
                });
            }
            let art_key = format!("{}@{}", req.name, meta.version);
            let pass = if !art.package_json.peer_dependencies.is_empty() {
                2
            } else if !art.package_json.dependencies.is_empty()
                || !art.package_json.optional_dependencies.is_empty()
            {
                1
            } else {
                0
            };
            prepared.push(PreparedRoot {
                request: req.clone(),
                artifact_key: Some(art_key),
                pass,
            });
        }

        // Resolve leaf roots first, then non-peer roots, then peer-requiring roots
        // so sibling peer visibility works regardless of specifier order.
        let mut root_peer_scope: BTreeMap<PackageName, usize> = BTreeMap::new();
        let mut root_node_ids: Vec<Option<usize>> = vec![None; prepared.len()];

        for pass in 0u8..=2 {
            for (idx, item) in prepared.iter().enumerate() {
                if item.pass != pass {
                    continue;
                }
                let Some(art_key) = &item.artifact_key else {
                    continue;
                };
                let node_id =
                    state.resolve_artifact_node(&item.request.name, art_key, &root_peer_scope)?;
                root_node_ids[idx] = Some(node_id);
                root_peer_scope
                    .entry(item.request.name.clone())
                    .or_insert(node_id);
            }
        }

        // Step 2: Fixed-point computation of transitive peer bindings and `<ctx>`.
        compute_contexts(&mut state.nodes);

        // Step 3: Canonical deduplication by `<name>@<version>~<ctx>`.
        let mut instances = BTreeMap::new();
        let mut node_to_instance = Vec::with_capacity(state.nodes.len());
        for node in &state.nodes {
            node_to_instance.push(format!("{}@{}~{}", node.name, node.version, node.ctx));
        }

        let mut by_url_identity = BTreeMap::new();
        for (idx, node) in state.nodes.iter().enumerate() {
            let instance_id = node_to_instance[idx].clone();
            if instances.contains_key(&instance_id) {
                continue;
            }
            let mut deps = BTreeMap::new();
            for (d_name, d_node) in &node.dependencies {
                deps.insert(d_name.clone(), node_to_instance[*d_node].clone());
            }
            let mut opt_deps = BTreeMap::new();
            for (d_name, d_opt) in &node.optional_dependencies {
                opt_deps.insert(
                    d_name.clone(),
                    d_opt.map(|nid| node_to_instance[nid].clone()),
                );
            }
            let mut peer_deps = BTreeMap::new();
            for (p_name, p_opt) in &node.direct_peers {
                peer_deps.insert(
                    p_name.clone(),
                    p_opt.map(|nid| node_to_instance[nid].clone()),
                );
            }

            let pkg = ResolvedNpmPackage {
                instance_id: instance_id.clone(),
                registry_host: node.registry_host.clone(),
                registry_origin: node.registry_origin.clone(),
                name: node.name.clone(),
                version: node.version.clone(),
                ctx: node.ctx.clone(),
                tarball_url: node.tarball_url.clone(),
                integrity: node.integrity.clone(),
                tree_sha256: node.tree.tree_sha256.clone(),
                tree: node.tree.clone(),
                package_json: node.package_json.clone(),
                dependencies: deps,
                optional_dependencies: opt_deps,
                peer_dependencies: peer_deps,
                script_audit: node.script_audit.clone(),
            };
            by_url_identity.insert(
                (
                    node.registry_host.clone(),
                    node.name.clone(),
                    node.version.clone(),
                    node.ctx.clone(),
                ),
                instance_id.clone(),
            );
            instances.insert(instance_id, pkg);
        }

        let mut roots = BTreeMap::new();
        for (idx, item) in prepared.iter().enumerate() {
            let key = (
                item.request.name.clone(),
                item.request.constraint.to_string(),
            );
            let mapped = root_node_ids[idx].map(|nid| node_to_instance[nid].clone());
            roots.insert(key, mapped);
        }

        Ok(ResolvedNpmGraph {
            registry_origin: origin,
            registry_host,
            cas: self.ingestor.cas().clone(),
            roots,
            instances,
            by_url_identity,
        })
    }
}

struct LoadedArtifact {
    registry_origin: String,
    registry_host: String,
    version: String,
    tarball_url: String,
    integrity: String,
    tree: PackageTree,
    package_json: PackageJson,
    script_audit: ScriptAudit,
}

struct WorkingNode {
    registry_origin: String,
    registry_host: String,
    name: PackageName,
    version: String,
    tarball_url: String,
    integrity: String,
    tree: PackageTree,
    package_json: PackageJson,
    script_audit: ScriptAudit,
    dependencies: BTreeMap<PackageName, usize>,
    optional_dependencies: BTreeMap<PackageName, Option<usize>>,
    direct_peers: BTreeMap<PackageName, Option<usize>>,
    transitive_peers: BTreeMap<PackageName, Option<usize>>,
    ctx: String,
}

struct StackFrame {
    name: PackageName,
    version: String,
    direct_peers: BTreeMap<PackageName, Option<usize>>,
    node_id: usize,
}

struct ResolutionState<'a, R: NpmRegistry + ?Sized> {
    resolver: &'a NpmGraphResolver<'a, R>,
    packuments: BTreeMap<PackageName, Packument>,
    artifacts: BTreeMap<String, LoadedArtifact>,
    nodes: Vec<WorkingNode>,
    active_stack: Vec<StackFrame>,
}

impl<'a, R: NpmRegistry + ?Sized> ResolutionState<'a, R> {
    fn get_packument(&mut self, name: &PackageName) -> Result<&Packument, ResolverError> {
        if !self.packuments.contains_key(name) {
            let pack = self.resolver.registry.fetch_packument(name)?;
            self.packuments.insert(name.clone(), pack);
        }
        Ok(&self.packuments[name])
    }

    fn select_version(
        &mut self,
        name: &PackageName,
        constraint: &VersionConstraint,
    ) -> Result<(PackumentVersion, NpmVersion), ResolverError> {
        let pack = self.get_packument(name)?;
        let (meta, parsed) = pack.select_version(name, constraint)?;
        Ok((meta.clone(), parsed))
    }

    fn load_artifact(
        &mut self,
        name: &PackageName,
        meta: &PackumentVersion,
    ) -> Result<&LoadedArtifact, ResolverError> {
        let art_key = format!("{name}@{}", meta.version);
        if self.artifacts.contains_key(&art_key) {
            return Ok(&self.artifacts[&art_key]);
        }

        let pkg_origin = self
            .resolver
            .registry
            .registry_origin_for_package(name)
            .trim_end_matches('/')
            .to_owned();
        let pkg_origin_url = Url::parse(&pkg_origin).map_err(|err| ResolverError::Registry {
            package: name.to_string(),
            reason: err.to_string(),
        })?;
        let pkg_host = pkg_origin_url
            .host_str()
            .ok_or_else(|| ResolverError::Registry {
                package: name.to_string(),
                reason: "registry origin has no host".to_owned(),
            })?
            .to_owned();

        let integrity = meta
            .dist
            .integrity
            .as_deref()
            .ok_or(TarballError::MissingIntegrity)?;
        // Validate SRI algorithm before touching cache or network (rejects sha1-only).
        let _ = parse_sha512_sri(integrity)?;

        let cas = self.resolver.ingestor.cas();
        let tree = if let Some(cached_tree) =
            PackageTree::load_verified(cas.cache_root(), &pkg_origin, integrity, None)?
        {
            cached_tree
        } else {
            let tgz = self
                .resolver
                .registry
                .fetch_tarball(&meta.dist.tarball, integrity)?;
            self.resolver
                .ingestor
                .ingest(&pkg_origin, integrity, &tgz)?
        };

        let pkg_entry = tree
            .get_file("package.json")
            .ok_or(TarballError::MissingPackageJson)?;
        let pkg_bytes = cas.read_verified(&pkg_entry.sha256, pkg_entry.size)?;
        let package_json = PackageJson::parse_bytes(&pkg_bytes)?;

        if package_json.name.as_deref() != Some(name.as_str()) {
            return Err(ResolverError::PackageIdentityMismatch {
                expected: name.to_string(),
                actual: package_json.name.clone(),
            });
        }

        let script_audit = package_json.build_script_audit(
            name.as_str(),
            &meta.version,
            tree.has_binding_gyp,
            tree.skipped_native.clone(),
        );

        self.artifacts.insert(
            art_key.clone(),
            LoadedArtifact {
                registry_origin: pkg_origin,
                registry_host: pkg_host,
                version: meta.version.clone(),
                tarball_url: meta.dist.tarball.clone(),
                integrity: tree.integrity.clone(),
                tree,
                package_json,
                script_audit,
            },
        );
        Ok(&self.artifacts[&art_key])
    }

    fn resolve_artifact_node(
        &mut self,
        name: &PackageName,
        art_key: &str,
        peer_scope: &BTreeMap<PackageName, usize>,
    ) -> Result<usize, ResolverError> {
        let art = &self.artifacts[art_key];
        let registry_origin = art.registry_origin.clone();
        let registry_host = art.registry_host.clone();
        let version = art.version.clone();
        let tarball_url = art.tarball_url.clone();
        let integrity = art.integrity.clone();
        let tree = art.tree.clone();
        let package_json = art.package_json.clone();
        let script_audit = art.script_audit.clone();

        // Bind direct peerDependencies from `peer_scope`.
        let mut direct_peers = BTreeMap::new();
        for (peer_name_str, range_str) in &package_json.peer_dependencies {
            let peer_name = PackageName::parse(peer_name_str)?;
            let range = NpmRange::parse(range_str)?;
            let is_optional = package_json
                .peer_dependencies_meta
                .get(peer_name_str)
                .is_some_and(|m| m.optional);

            if let Some(&target_id) = peer_scope.get(&peer_name) {
                let target_ver_str = &self.nodes[target_id].version;
                let target_ver = NpmVersion::parse(target_ver_str)?;
                if !range.matches(&target_ver) {
                    return Err(ResolverError::PeerDependencyMismatch {
                        package: name.to_string(),
                        version,
                        peer: peer_name.to_string(),
                        range: range_str.clone(),
                        found_version: target_ver_str.clone(),
                    });
                }
                direct_peers.insert(peer_name, Some(target_id));
            } else if is_optional {
                direct_peers.insert(peer_name, None);
            } else {
                return Err(ResolverError::MissingPeerDependency {
                    package: name.to_string(),
                    version,
                    peer: peer_name.to_string(),
                    range: range_str.clone(),
                });
            }
        }

        // Cycle check on active resolution stack.
        if let Some(frame) = self
            .active_stack
            .iter()
            .find(|f| &f.name == name && f.version == version && f.direct_peers == direct_peers)
        {
            return Ok(frame.node_id);
        }

        if self.nodes.len() >= self.resolver.limits.max_graph_packages {
            return Err(ResolverError::GraphLimitExceeded {
                limit: self.resolver.limits.max_graph_packages,
            });
        }

        let node_id = self.nodes.len();
        self.nodes.push(WorkingNode {
            registry_origin,
            registry_host,
            name: name.clone(),
            version: version.clone(),
            tarball_url,
            integrity,
            tree,
            package_json: package_json.clone(),
            script_audit,
            dependencies: BTreeMap::new(),
            optional_dependencies: BTreeMap::new(),
            direct_peers: direct_peers.clone(),
            transitive_peers: BTreeMap::new(),
            ctx: "0".to_owned(),
        });
        self.active_stack.push(StackFrame {
            name: name.clone(),
            version,
            direct_peers,
            node_id,
        });

        // Pre-prepare all dependencies & optionalDependencies so siblings with peers
        // are ordered after siblings they peer-depend on regardless of alphabetical order.
        struct ChildPrep {
            name: PackageName,
            art_key: Option<String>,
            is_optional: bool,
            peer_names: BTreeSet<PackageName>,
        }
        impl HasPeerNames for ChildPrep {
            fn pkg_name(&self) -> &PackageName {
                &self.name
            }
            fn peers(&self) -> &BTreeSet<PackageName> {
                &self.peer_names
            }
        }
        let mut children = Vec::new();

        for (dep_str, range_str) in &package_json.dependencies {
            let dep_name = PackageName::parse(dep_str)?;
            let constraint = VersionConstraint::parse(range_str)?;
            let (meta, _) = self.select_version(&dep_name, &constraint)?;
            let platform = self.resolver.platform.clone();
            let art = self.load_artifact(&dep_name, &meta)?;
            if !platform.matches(
                art.package_json.os.as_deref(),
                art.package_json.cpu.as_deref(),
                art.package_json.libc.as_deref(),
            ) {
                self.active_stack.pop();
                return Err(ResolverError::UnsupportedPlatform {
                    package: dep_name.to_string(),
                    version: meta.version,
                    os: platform.os,
                    cpu: platform.cpu,
                });
            }
            let peer_names = art
                .package_json
                .peer_dependencies
                .keys()
                .map(|s| PackageName::parse(s))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let key = format!("{dep_name}@{}", meta.version);
            children.push(ChildPrep {
                name: dep_name,
                art_key: Some(key),
                is_optional: false,
                peer_names,
            });
        }

        for (dep_str, range_str) in &package_json.optional_dependencies {
            if package_json.dependencies.contains_key(dep_str) {
                continue;
            }
            let dep_name = PackageName::parse(dep_str)?;
            let constraint = VersionConstraint::parse(range_str)?;
            let meta = match self.select_version(&dep_name, &constraint) {
                Ok((m, _)) => m,
                Err(ResolverError::NoMatchingVersion { .. }) => {
                    children.push(ChildPrep {
                        name: dep_name,
                        art_key: None,
                        is_optional: true,
                        peer_names: BTreeSet::new(),
                    });
                    continue;
                }
                Err(e) => {
                    self.active_stack.pop();
                    return Err(e);
                }
            };
            let platform = self.resolver.platform.clone();
            let art = self.load_artifact(&dep_name, &meta)?;
            if !platform.matches(
                art.package_json.os.as_deref(),
                art.package_json.cpu.as_deref(),
                art.package_json.libc.as_deref(),
            ) {
                children.push(ChildPrep {
                    name: dep_name,
                    art_key: None,
                    is_optional: true,
                    peer_names: BTreeSet::new(),
                });
                continue;
            }
            let peer_names = art
                .package_json
                .peer_dependencies
                .keys()
                .map(|s| PackageName::parse(s))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let key = format!("{dep_name}@{}", meta.version);
            children.push(ChildPrep {
                name: dep_name,
                art_key: Some(key),
                is_optional: true,
                peer_names,
            });
        }

        // Order children so packages without peer dependencies (or not peering on siblings)
        // resolve before siblings that peer-depend on them.
        let child_order = topo_order_children(&children);
        let mut child_peer_scope = peer_scope.clone();
        child_peer_scope.insert(name.clone(), node_id);

        let mut resolved_deps = BTreeMap::new();
        let mut resolved_opt_deps = BTreeMap::new();

        for idx in child_order {
            let child = &children[idx];
            match &child.art_key {
                None => {
                    resolved_opt_deps.insert(child.name.clone(), None);
                }
                Some(key) => {
                    let child_res = self.resolve_artifact_node(&child.name, key, &child_peer_scope);
                    match child_res {
                        Ok(child_id) => {
                            child_peer_scope.insert(child.name.clone(), child_id);
                            if child.is_optional {
                                resolved_opt_deps.insert(child.name.clone(), Some(child_id));
                            } else {
                                resolved_deps.insert(child.name.clone(), child_id);
                            }
                        }
                        Err(e) => {
                            self.active_stack.pop();
                            return Err(e);
                        }
                    }
                }
            }
        }

        self.active_stack.pop();
        self.nodes[node_id].dependencies = resolved_deps;
        self.nodes[node_id].optional_dependencies = resolved_opt_deps;
        Ok(node_id)
    }
}

fn topo_order_children<T>(children: &[T]) -> Vec<usize>
where
    T: HasPeerNames,
{
    let n = children.len();
    let mut visited = vec![false; n];
    let mut visiting = vec![false; n];
    let mut order = Vec::with_capacity(n);

    fn dfs<T: HasPeerNames>(
        idx: usize,
        children: &[T],
        visited: &mut [bool],
        visiting: &mut [bool],
        order: &mut Vec<usize>,
    ) {
        if visited[idx] || visiting[idx] {
            return;
        }
        visiting[idx] = true;
        for (j, other) in children.iter().enumerate() {
            if j != idx && children[idx].peers().contains(other.pkg_name()) {
                dfs(j, children, visited, visiting, order);
            }
        }
        visiting[idx] = false;
        visited[idx] = true;
        order.push(idx);
    }

    for i in 0..n {
        dfs(i, children, &mut visited, &mut visiting, &mut order);
    }
    order
}

trait HasPeerNames {
    fn pkg_name(&self) -> &PackageName;
    fn peers(&self) -> &BTreeSet<PackageName>;
}

fn compute_contexts(nodes: &mut [WorkingNode]) {
    let n = nodes.len();
    for node in nodes.iter_mut() {
        node.transitive_peers = node.direct_peers.clone();
    }

    // Fixed-point propagation of transitive external peers + ctx hashes across cycles.
    for _ in 0..=(n * 2) {
        let mut changed = false;

        // 1. Propagate child transitive_peers (excluding peers satisfied directly by `i` itself).
        for i in 0..n {
            let owned_children: BTreeSet<usize> = nodes[i]
                .dependencies
                .values()
                .copied()
                .chain(nodes[i].optional_dependencies.values().flatten().copied())
                .collect();

            let mut next_peers = nodes[i].direct_peers.clone();
            for &child_id in &owned_children {
                let child_peers = nodes[child_id].transitive_peers.clone();
                for (p_name, p_target) in child_peers {
                    let is_owned =
                        p_target.is_some_and(|tid| tid == i || owned_children.contains(&tid));
                    if !is_owned {
                        next_peers.entry(p_name).or_insert(p_target);
                    }
                }
            }
            if next_peers != nodes[i].transitive_peers {
                nodes[i].transitive_peers = next_peers;
                changed = true;
            }
        }

        // 2. Recompute ctx tokens from transitive_peers.
        for i in 0..n {
            let new_ctx = if nodes[i].transitive_peers.is_empty() {
                "0".to_owned()
            } else {
                let mut canonical = String::new();
                for (p_name, p_target) in &nodes[i].transitive_peers {
                    use std::fmt::Write as _;
                    match p_target {
                        Some(tid) => {
                            let t = &nodes[*tid];
                            let _ = writeln!(
                                canonical,
                                "{}={}@{}~{}",
                                p_name, t.name, t.version, t.ctx
                            );
                        }
                        None => {
                            let _ = writeln!(canonical, "{p_name}=<omitted>");
                        }
                    }
                }
                let hash = sha256_hex(canonical.as_bytes());
                format!("p{}", &hash[..32])
            };
            if new_ctx != nodes[i].ctx {
                nodes[i].ctx = new_ctx;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }
}

/// Resolution result for a specifier evaluated by [`ProjectPackageResolver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectResolution {
    /// Specifier uses a built-in/explicit scheme (`vael:*`, `node:*`, `npm:*`, `https:`, etc.)
    /// or path (`./`, `../`, `/`) and must NOT be intercepted by project `package.json`.
    NotIntercepted,
    /// Resolved to a virtual `npm://` URL inside [`ResolvedNpmGraph`].
    NpmModule(Url),
    /// Resolved to a local file inside the project root (from `#imports` or self-reference).
    LocalFile {
        /// Absolute local path inside the project root.
        path: PathBuf,
        /// Classified module format (`JavaScript` or `TypeScript`).
        format: ModuleFormat,
    },
}

/// Project `package.json` bare-import and `#imports` resolver (without `node_modules`).
///
/// Maps `dependencies`, `optionalDependencies`, `#imports`, and package
/// self-reference into `npm:` graph roots or local project files while never
/// intercepting `vael:` or `node:` built-ins.
#[derive(Clone, Debug)]
pub struct ProjectPackageResolver {
    root_dir: PathBuf,
    package_json: PackageJson,
}

impl ProjectPackageResolver {
    /// Discovers the nearest `package.json` walking upward from `start_dir`
    /// (bounded by [`MAX_DISCOVERY_DEPTH`] and canonical symlink-loop detection).
    pub fn discover(start_dir: &Path) -> Result<Option<Self>, ResolverError> {
        use std::io::Read as _;

        let mut current =
            fs::canonicalize(start_dir).map_err(|err| ResolverError::ProjectManifest {
                reason: err.to_string(),
            })?;
        let mut visited = BTreeSet::new();
        for _ in 0..MAX_DISCOVERY_DEPTH {
            if let Ok(canon) = fs::canonicalize(&current)
                && !visited.insert(canon)
            {
                break;
            }
            let candidate = current.join("package.json");
            if candidate.is_file() {
                let file =
                    fs::File::open(&candidate).map_err(|err| ResolverError::ProjectManifest {
                        reason: err.to_string(),
                    })?;
                let mut bytes = Vec::new();
                file.take((MAX_PACKAGE_JSON_BYTES as u64) + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|err| ResolverError::ProjectManifest {
                        reason: err.to_string(),
                    })?;
                if bytes.len() > MAX_PACKAGE_JSON_BYTES {
                    return Err(PackageJsonError::TooLarge { size: bytes.len() }.into());
                }
                let package_json = PackageJson::parse_bytes(&bytes)?;
                return Ok(Some(Self {
                    root_dir: current,
                    package_json,
                }));
            }
            if !current.pop() {
                break;
            }
        }
        Ok(None)
    }

    /// Creates a resolver from an explicit `root_dir` and parsed [`PackageJson`].
    pub fn from_package_json(root_dir: PathBuf, package_json: PackageJson) -> Self {
        Self {
            root_dir,
            package_json,
        }
    }

    /// Returns the project directory containing `package.json`.
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// Returns the parsed project [`PackageJson`].
    pub fn package_json(&self) -> &PackageJson {
        &self.package_json
    }

    /// Classifies a local file path inside the project using the project
    /// `package.json` `"type"` via [`vaeljs_module_policy::package_module_format`].
    pub fn classify_local_file(&self, path: &Path) -> Result<ModuleFormat, String> {
        package_module_format(path, self.package_json.package_type)
    }

    /// Converts declared `dependencies`, `optionalDependencies`, and
    /// `devDependencies` into [`NpmRootRequest`]s for preflight graph resolution.
    ///
    /// Lifecycle `scripts` are never executed.
    pub fn root_requests(&self) -> Result<Vec<NpmRootRequest>, ResolverError> {
        let mut requests = Vec::new();
        for (name_str, range_str) in &self.package_json.dependencies {
            requests.push(NpmRootRequest {
                name: PackageName::parse(name_str)?,
                constraint: VersionConstraint::parse(range_str)?,
                optional: false,
            });
        }
        for (name_str, range_str) in &self.package_json.optional_dependencies {
            if self.package_json.dependencies.contains_key(name_str) {
                continue;
            }
            requests.push(NpmRootRequest {
                name: PackageName::parse(name_str)?,
                constraint: VersionConstraint::parse(range_str)?,
                optional: true,
            });
        }
        for (name_str, range_str) in &self.package_json.dev_dependencies {
            if self.package_json.dependencies.contains_key(name_str)
                || self
                    .package_json
                    .optional_dependencies
                    .contains_key(name_str)
            {
                continue;
            }
            requests.push(NpmRootRequest {
                name: PackageName::parse(name_str)?,
                constraint: VersionConstraint::parse(range_str)?,
                optional: false,
            });
        }
        Ok(requests)
    }

    /// Returns `(range_str, optional)` if `name` is declared in `dependencies`,
    /// `optionalDependencies`, or `devDependencies` of this project `package.json`.
    pub fn dependency_constraint(&self, name: &PackageName) -> Option<(&str, bool)> {
        if let Some(range) = self.package_json.dependencies.get(name.as_str()) {
            return Some((range.as_str(), false));
        }
        if let Some(range) = self.package_json.optional_dependencies.get(name.as_str()) {
            return Some((range.as_str(), true));
        }
        if let Some(range) = self.package_json.dev_dependencies.get(name.as_str()) {
            return Some((range.as_str(), false));
        }
        None
    }

    /// Resolves a bare or `#...` import specifier against the project's
    /// `package.json` and a pre-resolved [`ResolvedNpmGraph`].
    ///
    /// Never intercepts `vael:`, `node:`, `npm:`, `jsr:`, `gh:`, `https:`, `file:`,
    /// or relative/absolute paths (returning [`ProjectResolution::NotIntercepted`]).
    pub fn resolve_specifier(
        &self,
        specifier: &str,
        graph: &ResolvedNpmGraph,
    ) -> Result<ProjectResolution, ResolverError> {
        if has_scheme_prefix_ci(specifier, "vael:")
            || has_scheme_prefix_ci(specifier, "node:")
            || specifier.starts_with("./")
            || specifier.starts_with("../")
            || specifier.starts_with('/')
            || specifier.contains(':')
        {
            return Ok(ProjectResolution::NotIntercepted);
        }

        let proj_name = self.package_json.name.as_deref().unwrap_or("<project>");

        if specifier.starts_with('#') {
            let resolved = ExportsResolver::resolve_project_import(
                &self.package_json,
                &self.root_dir,
                proj_name,
                specifier,
            )?;
            match resolved {
                crate::exports::PackageImportTarget::InternalFile(file) => {
                    return self.resolve_local_rel(&file.rel_path);
                }
                crate::exports::PackageImportTarget::ExternalPackage { name, subpath } => {
                    return self.resolve_declared_npm(&name, subpath.as_deref(), graph);
                }
            }
        }

        let (pkg_name, subpath) = split_bare_package_specifier(specifier)?;
        if self.package_json.name.as_deref() == Some(pkg_name.as_str())
            && let Some(exports) = &self.package_json.exports
        {
            let rel =
                ExportsResolver::resolve_exports_target(exports, proj_name, subpath.as_deref())?;
            return self.resolve_local_rel(&rel);
        }

        self.resolve_declared_npm(&pkg_name, subpath.as_deref(), graph)
    }

    /// Resolves local-only project targets (`#imports` internal files and
    /// package self-references) without consulting a [`ResolvedNpmGraph`] or
    /// requiring a [`CasStore`].
    ///
    /// Returns `Ok(Some(ProjectResolution::NotIntercepted))` for non-project
    /// schemes/paths, `Ok(Some(ProjectResolution::LocalFile { .. }))` for local
    /// `#imports` / self-references, and `Ok(None)` when the specifier targets
    /// an external npm dependency.
    pub fn resolve_local_specifier(
        &self,
        specifier: &str,
    ) -> Result<Option<ProjectResolution>, ResolverError> {
        if has_scheme_prefix_ci(specifier, "vael:")
            || has_scheme_prefix_ci(specifier, "node:")
            || specifier.starts_with("./")
            || specifier.starts_with("../")
            || specifier.starts_with('/')
            || specifier.contains(':')
        {
            return Ok(Some(ProjectResolution::NotIntercepted));
        }

        let proj_name = self.package_json.name.as_deref().unwrap_or("<project>");
        if specifier.starts_with('#') {
            let resolved = ExportsResolver::resolve_project_import(
                &self.package_json,
                &self.root_dir,
                proj_name,
                specifier,
            )?;
            return match resolved {
                crate::exports::PackageImportTarget::InternalFile(file) => {
                    self.resolve_local_rel(&file.rel_path).map(Some)
                }
                crate::exports::PackageImportTarget::ExternalPackage { .. } => Ok(None),
            };
        }

        let (pkg_name, subpath) = split_bare_package_specifier(specifier)?;
        if self.package_json.name.as_deref() == Some(pkg_name.as_str())
            && let Some(exports) = &self.package_json.exports
        {
            let rel =
                ExportsResolver::resolve_exports_target(exports, proj_name, subpath.as_deref())?;
            return self.resolve_local_rel(&rel).map(Some);
        }

        Ok(None)
    }

    fn resolve_local_rel(&self, rel_path: &str) -> Result<ProjectResolution, ResolverError> {
        use std::path::Component;

        let clean = validate_subpath(rel_path)?;
        if !Path::new(&clean)
            .components()
            .all(|comp| matches!(comp, Component::Normal(_)))
        {
            return Err(SpecifierError::InvalidSubpath {
                subpath: rel_path.to_owned(),
                reason: "local project subpath must consist strictly of normal path components"
                    .to_owned(),
            }
            .into());
        }
        let full = self.root_dir.join(&clean);
        let format = self.classify_local_file(&full).map_err(|reason| {
            ExportsError::UnsupportedModuleFormat {
                package: self
                    .package_json
                    .name
                    .clone()
                    .unwrap_or_else(|| "<project>".to_owned()),
                path: clean,
                reason,
            }
        })?;
        Ok(ProjectResolution::LocalFile { path: full, format })
    }

    fn resolve_declared_npm(
        &self,
        name: &PackageName,
        subpath: Option<&str>,
        graph: &ResolvedNpmGraph,
    ) -> Result<ProjectResolution, ResolverError> {
        let range_str = self
            .package_json
            .dependencies
            .get(name.as_str())
            .or_else(|| self.package_json.optional_dependencies.get(name.as_str()))
            .or_else(|| self.package_json.dev_dependencies.get(name.as_str()))
            .ok_or_else(|| ResolverError::DisallowedImport {
                referrer: self
                    .package_json
                    .name
                    .clone()
                    .unwrap_or_else(|| "<project>".to_owned()),
                specifier: match subpath {
                    Some(s) => format!("{name}/{s}"),
                    None => name.to_string(),
                },
                reason: format!(
                    "package `{name}` is not declared in project package.json dependencies"
                ),
            })?;
        let constraint = VersionConstraint::parse(range_str)?;
        let instance_id = graph
            .root_instance_id(name, &constraint.to_string())
            .ok_or_else(|| ResolverError::UnresolvedTarget {
                reason: format!("project dependency `{name}@{constraint}` missing from graph"),
            })?;
        let url = graph.resolve_instance_entry(instance_id, subpath)?;
        Ok(ProjectResolution::NpmModule(url))
    }
}

/// Resolves `requests` against `lockfile` and `cas`, consulting `registry` only
/// when permitted (`!offline` and `registry.is_some()`).
///
/// Returns `(graph, lockfile_modified)`:
/// - In `frozen_lockfile` mode, any specifier or dependency missing from
///   `lockfile`, any mismatched range/version, or any integrity mismatch fails
///   closed without consulting `registry` for new versions.
/// - In `offline` mode (or when `registry` is `None`), missing `.tree`/CAS
///   entries or unlocked specifiers fail closed without network I/O.
#[allow(clippy::too_many_arguments)]
pub fn resolve_requests_with_lockfile<R: NpmRegistry + ?Sized>(
    requests: &[NpmRootRequest],
    lockfile: &mut Lockfile,
    cas: &CasStore,
    registry: Option<&R>,
    registry_origin: &str,
    frozen_lockfile: bool,
    offline: bool,
    limits: NpmLimits,
    platform: &TargetPlatform,
) -> Result<(ResolvedNpmGraph, bool), ResolverError> {
    resolve_requests_with_lockfile_and_npmrc(
        requests,
        lockfile,
        cas,
        registry,
        registry_origin,
        frozen_lockfile,
        offline,
        limits,
        platform,
        None,
        false,
    )
}

/// Resolves `requests` against `lockfile` and `cas` with optional `.npmrc`
/// scoped-registry origin routing for both live and offline/locked runs.
#[allow(clippy::too_many_arguments)]
pub fn resolve_requests_with_lockfile_and_npmrc<R: NpmRegistry + ?Sized>(
    requests: &[NpmRootRequest],
    lockfile: &mut Lockfile,
    cas: &CasStore,
    registry: Option<&R>,
    registry_origin: &str,
    frozen_lockfile: bool,
    offline: bool,
    limits: NpmLimits,
    platform: &TargetPlatform,
    npmrc: Option<&NpmrcConfig>,
    registry_origin_overridden: bool,
) -> Result<(ResolvedNpmGraph, bool), ResolverError> {
    let origin = registry_origin.trim_end_matches('/').to_owned();
    let mut graph = ResolvedNpmGraph::new(&origin, cas.clone())?;
    let registry_host = graph.registry_host.clone();
    let ingestor = TarballIngestor::new(cas.clone(), limits);
    let mut lockfile_modified = false;
    let mut visited_keys: BTreeMap<String, String> = BTreeMap::new();

    let mut locked_siblings: Vec<(PackageName, NpmVersion)> = Vec::new();
    let mut unlocked_reqs: Vec<NpmRootRequest> = Vec::new();

    for req in requests {
        let spec_key = format!("{}@{}", req.name, req.constraint);
        if let Some(pkg_key) = lockfile.specifiers.get(&spec_key).cloned() {
            let locked =
                lockfile
                    .packages
                    .get(&pkg_key)
                    .ok_or_else(|| LockfileError::PackageNotFound {
                        pkg_key: pkg_key.clone(),
                    })?;
            let parsed_ver = NpmVersion::parse(&locked.version)?;
            if let VersionConstraint::Range(range) = &req.constraint
                && !range.matches(&parsed_ver)
            {
                return Err(ResolverError::FrozenLockfile {
                    reason: format!("locked version `{pkg_key}` does not satisfy `{spec_key}`"),
                });
            }

            let mut ctx = LockedContext {
                lockfile,
                cas,
                ingestor: &ingestor,
                registry: if offline { None } else { registry },
                registry_origin: &origin,
                registry_host: &registry_host,
                npmrc,
                registry_origin_overridden,
                frozen_lockfile,
                offline,
                limits,
                platform,
                visited_keys: &mut visited_keys,
                graph: &mut graph,
            };
            let instance_id = load_locked_package(&req.name, &pkg_key, req.optional, &mut ctx)?;
            if instance_id.is_some() {
                locked_siblings.push((req.name.clone(), parsed_ver));
            }
            graph
                .roots
                .insert((req.name.clone(), req.constraint.to_string()), instance_id);
        } else {
            if frozen_lockfile {
                return Err(ResolverError::FrozenLockfile {
                    reason: format!("specifier `{spec_key}` is not pinned in vael.lock"),
                });
            }
            if offline {
                return Err(ResolverError::Offline {
                    reason: format!(
                        "specifier `{spec_key}` is not in vael.lock and --offline disables network fetches"
                    ),
                });
            }
            if registry.is_none() {
                return Err(ResolverError::Offline {
                    reason: format!(
                        "specifier `{spec_key}` is not in vael.lock / offline CAS cache and remote loading is disabled (--allow-remote required)"
                    ),
                });
            }
            unlocked_reqs.push(req.clone());
        }
    }

    if !unlocked_reqs.is_empty() {
        let reg = registry.ok_or_else(|| ResolverError::Offline {
            reason: "remote loading is disabled (--allow-remote required)".to_owned(),
        })?;
        let resolver = NpmGraphResolver::new(reg, cas.clone())
            .with_limits(limits)
            .with_platform(platform.clone());

        let mut combined_reqs = unlocked_reqs.clone();
        let mut synthetic_roots: Vec<(PackageName, String)> = Vec::new();
        let mut sub_graph = loop {
            match resolver.resolve_requests(&combined_reqs) {
                Ok(g) => break g,
                Err(ResolverError::MissingPeerDependency {
                    package,
                    version,
                    peer,
                    range,
                }) => {
                    if let Some((_, ver)) = locked_siblings
                        .iter()
                        .find(|(name, _)| name.as_str() == peer.as_str())
                    {
                        let peer_name = PackageName::parse(&peer)?;
                        let constraint = VersionConstraint::parse(&ver.to_string())?;
                        let root_key = (peer_name.clone(), constraint.to_string());
                        if !synthetic_roots.contains(&root_key) {
                            synthetic_roots.push(root_key);
                            combined_reqs.push(NpmRootRequest {
                                name: peer_name,
                                constraint,
                                optional: false,
                            });
                            continue;
                        }
                    }
                    return Err(ResolverError::MissingPeerDependency {
                        package,
                        version,
                        peer,
                        range,
                    });
                }
                Err(err) => return Err(err),
            }
        };
        for root_key in &synthetic_roots {
            sub_graph.roots.remove(root_key);
        }
        if lockfile.record_graph(&sub_graph) {
            lockfile_modified = true;
        }
        graph.merge(sub_graph);
    }

    Ok((graph, lockfile_modified))
}

struct LockedContext<'a, R: NpmRegistry + ?Sized> {
    lockfile: &'a Lockfile,
    cas: &'a CasStore,
    ingestor: &'a TarballIngestor,
    registry: Option<&'a R>,
    registry_origin: &'a str,
    registry_host: &'a str,
    npmrc: Option<&'a NpmrcConfig>,
    registry_origin_overridden: bool,
    frozen_lockfile: bool,
    offline: bool,
    limits: NpmLimits,
    platform: &'a TargetPlatform,
    visited_keys: &'a mut BTreeMap<String, String>,
    graph: &'a mut ResolvedNpmGraph,
}

fn locked_package_origin_and_host<R: NpmRegistry + ?Sized>(
    expected_name: &PackageName,
    ctx: &LockedContext<'_, R>,
) -> Result<(String, String), ResolverError> {
    let pkg_origin = if let Some(reg) = ctx.registry {
        reg.registry_origin_for_package(expected_name)
            .trim_end_matches('/')
            .to_owned()
    } else if let Some(cfg) = ctx.npmrc {
        if !ctx.registry_origin_overridden {
            cfg.registry_origin_for_package(expected_name.as_str())
                .to_owned()
        } else {
            cfg.scoped_registry_origin_for_package(expected_name.as_str())
                .unwrap_or(ctx.registry_origin)
                .to_owned()
        }
    } else {
        ctx.registry_origin.to_owned()
    };

    if pkg_origin == ctx.registry_origin {
        return Ok((pkg_origin, ctx.registry_host.to_owned()));
    }
    let origin_url = Url::parse(&pkg_origin).map_err(|err| ResolverError::Registry {
        package: expected_name.to_string(),
        reason: err.to_string(),
    })?;
    let host = origin_url
        .host_str()
        .ok_or_else(|| ResolverError::Registry {
            package: expected_name.to_string(),
            reason: "registry origin has no host".to_owned(),
        })?
        .to_owned();
    Ok((pkg_origin, host))
}

fn load_locked_package<R: NpmRegistry + ?Sized>(
    expected_name: &PackageName,
    pkg_key: &str,
    optional: bool,
    ctx: &mut LockedContext<'_, R>,
) -> Result<Option<String>, ResolverError> {
    if let Some(existing_id) = ctx.visited_keys.get(pkg_key) {
        return Ok(Some(existing_id.clone()));
    }
    if ctx.graph.instances.len() >= ctx.limits.max_graph_packages {
        return Err(ResolverError::GraphLimitExceeded {
            limit: ctx.limits.max_graph_packages,
        });
    }

    let locked: &LockedPackage =
        ctx.lockfile
            .packages
            .get(pkg_key)
            .ok_or_else(|| LockfileError::PackageNotFound {
                pkg_key: pkg_key.to_owned(),
            })?;
    let _ = NpmVersion::parse(&locked.version)?;

    let pkg_ctx = extract_lock_ctx(expected_name, &locked.version, pkg_key);
    let instance_id = format!("{expected_name}@{}~{pkg_ctx}", locked.version);

    let (pkg_origin, pkg_host) = locked_package_origin_and_host(expected_name, ctx)?;

    // Load and verify `.tree` from CAS (or fetch tarball when permitted).
    let _ = parse_sha512_sri(&locked.integrity)?;
    if ctx.frozen_lockfile && locked.tree_sha256.is_none() {
        return Err(ResolverError::FrozenLockfile {
            reason: format!(
                "locked package `{pkg_key}` is missing required `treeSha256` under --frozen-lockfile"
            ),
        });
    }
    let tree = match PackageTree::load_verified(
        ctx.cas.cache_root(),
        &pkg_origin,
        &locked.integrity,
        locked.tree_sha256.as_deref(),
    )? {
        Some(cached) => cached,
        None => {
            if ctx.offline {
                return Err(ResolverError::Offline {
                    reason: format!(
                        "package `{pkg_key}` is pinned in vael.lock but missing from the offline CAS cache"
                    ),
                });
            }
            let Some(reg) = ctx.registry else {
                return Err(ResolverError::Offline {
                    reason: format!(
                        "package `{pkg_key}` is pinned in vael.lock but missing from the CAS cache and remote loading is disabled (--allow-remote required)"
                    ),
                });
            };
            let tarball_url = if !locked.tarball_url.is_empty() {
                locked.tarball_url.clone()
            } else {
                let packument = reg.fetch_packument(expected_name)?;
                let meta = packument.versions.get(&locked.version).ok_or_else(|| {
                    ResolverError::NoMatchingVersion {
                        package: expected_name.to_string(),
                        constraint: locked.version.clone(),
                    }
                })?;
                if let Some(pack_integrity) = meta.dist.integrity.as_deref() {
                    ctx.lockfile.verify_integrity(pkg_key, pack_integrity)?;
                }
                meta.dist.tarball.clone()
            };
            let tgz = reg.fetch_tarball(&tarball_url, &locked.integrity)?;
            let tree = ctx.ingestor.ingest(&pkg_origin, &locked.integrity, &tgz)?;
            if let Some(expected_tree_sha256) = &locked.tree_sha256
                && expected_tree_sha256 != &tree.tree_sha256
            {
                return Err(CasError::IntegrityMismatch {
                    expected: expected_tree_sha256.clone(),
                    actual: tree.tree_sha256,
                }
                .into());
            }
            tree
        }
    };

    let pkg_entry = tree
        .get_file("package.json")
        .ok_or(TarballError::MissingPackageJson)?;
    let pkg_bytes = ctx.cas.read_verified(&pkg_entry.sha256, pkg_entry.size)?;
    let package_json = PackageJson::parse_bytes(&pkg_bytes)?;

    if package_json.name.as_deref() != Some(expected_name.as_str()) {
        return Err(ResolverError::PackageIdentityMismatch {
            expected: expected_name.to_string(),
            actual: package_json.name.clone(),
        });
    }
    if let Some(pkg_ver) = &package_json.version
        && pkg_ver != &locked.version
    {
        return Err(ResolverError::PackageVersionMismatch {
            package: expected_name.to_string(),
            expected: locked.version.clone(),
            actual: pkg_ver.clone(),
        });
    }
    if !ctx.platform.matches(
        package_json.os.as_deref(),
        package_json.cpu.as_deref(),
        package_json.libc.as_deref(),
    ) {
        if optional {
            return Ok(None);
        }
        return Err(ResolverError::UnsupportedPlatform {
            package: expected_name.to_string(),
            version: locked.version.clone(),
            os: ctx.platform.os.clone(),
            cpu: ctx.platform.cpu.clone(),
        });
    }

    if ctx.frozen_lockfile {
        for dep_name in package_json.dependencies.keys() {
            if !locked.dependencies.contains_key(dep_name) {
                return Err(ResolverError::FrozenLockfile {
                    reason: format!(
                        "locked package `{pkg_key}` is missing required dependency `{dep_name}`"
                    ),
                });
            }
        }
        for peer_name in package_json.peer_dependencies.keys() {
            let is_optional_peer = package_json
                .peer_dependencies_meta
                .get(peer_name)
                .is_some_and(|m| m.optional);
            if !is_optional_peer && !locked.dependencies.contains_key(peer_name) {
                return Err(ResolverError::FrozenLockfile {
                    reason: format!(
                        "locked package `{pkg_key}` is missing required peer dependency `{peer_name}`"
                    ),
                });
            }
        }
    }

    // Record visited before recursing so dependency cycles terminate cleanly.
    ctx.visited_keys
        .insert(pkg_key.to_owned(), instance_id.clone());

    let mut deps = BTreeMap::new();
    let mut opt_deps = BTreeMap::new();
    let mut peer_deps = BTreeMap::new();

    for (dep_name_str, dep_pkg_key) in &locked.dependencies {
        if !package_json.is_declared_dependency(dep_name_str) {
            return Err(ResolverError::DisallowedImport {
                referrer: pkg_key.to_owned(),
                specifier: dep_name_str.to_owned(),
                reason: format!(
                    "locked dependency `{dep_name_str}` is not declared in dependencies, optionalDependencies, or peerDependencies of `{expected_name}`"
                ),
            });
        }
        let declared_range = package_json
            .dependencies
            .get(dep_name_str)
            .or_else(|| package_json.optional_dependencies.get(dep_name_str))
            .or_else(|| package_json.peer_dependencies.get(dep_name_str))
            .expect("checked by is_declared_dependency");
        let declared_constraint = VersionConstraint::parse(declared_range)?;
        let child_locked = ctx.lockfile.packages.get(dep_pkg_key).ok_or_else(|| {
            LockfileError::PackageNotFound {
                pkg_key: dep_pkg_key.clone(),
            }
        })?;
        let child_ver = NpmVersion::parse(&child_locked.version)?;
        if let VersionConstraint::Range(range) = &declared_constraint
            && !range.matches(&child_ver)
        {
            return Err(ResolverError::FrozenLockfile {
                reason: format!(
                    "locked dependency `{dep_pkg_key}` of `{pkg_key}` does not satisfy `{dep_name_str}@{declared_range}`"
                ),
            });
        }

        let dep_name = PackageName::parse(dep_name_str)?;
        let is_opt = package_json
            .optional_dependencies
            .contains_key(dep_name_str)
            && !package_json.dependencies.contains_key(dep_name_str);
        let is_peer = package_json.peer_dependencies.contains_key(dep_name_str)
            && !package_json.dependencies.contains_key(dep_name_str)
            && !package_json
                .optional_dependencies
                .contains_key(dep_name_str);
        let child_id = load_locked_package(&dep_name, dep_pkg_key, is_opt, ctx)?;
        if is_opt {
            opt_deps.insert(dep_name, child_id);
        } else if is_peer {
            peer_deps.insert(dep_name, child_id);
        } else if let Some(id) = child_id {
            deps.insert(dep_name, id);
        }
    }

    let script_audit = package_json.build_script_audit(
        expected_name.as_str(),
        &locked.version,
        tree.has_binding_gyp,
        tree.skipped_native.clone(),
    );

    let resolved = ResolvedNpmPackage {
        instance_id: instance_id.clone(),
        registry_host: pkg_host.clone(),
        registry_origin: pkg_origin,
        name: expected_name.clone(),
        version: locked.version.clone(),
        ctx: pkg_ctx,
        tarball_url: locked.tarball_url.clone(),
        integrity: locked.integrity.clone(),
        tree_sha256: tree.tree_sha256.clone(),
        tree,
        package_json,
        dependencies: deps,
        optional_dependencies: opt_deps,
        peer_dependencies: peer_deps,
        script_audit,
    };

    ctx.graph.by_url_identity.insert(
        (
            pkg_host,
            resolved.name.clone(),
            resolved.version.clone(),
            resolved.ctx.clone(),
        ),
        instance_id.clone(),
    );
    ctx.graph.instances.insert(instance_id.clone(), resolved);

    Ok(Some(instance_id))
}

fn extract_lock_ctx(name: &PackageName, version: &str, pkg_key: &str) -> String {
    let base = format!("{name}@{version}");
    if let Some(rest) = pkg_key.strip_prefix(&base)
        && let Some(ctx) = rest.strip_prefix('~')
        && !ctx.is_empty()
    {
        return ctx.to_owned();
    }
    "0".to_owned()
}

fn has_scheme_prefix_ci(specifier: &str, prefix_lower: &str) -> bool {
    specifier
        .get(..prefix_lower.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix_lower))
}
