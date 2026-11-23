//! `jsr:` resolution and loading over the opt-in HTTPS transport.
//!
//! [`crate::Runtime::with_remote`] installs this loader; [`crate::Runtime::new`]
//! does not, so a default runtime rejects every `jsr:` import. Remote loading is
//! the explicit permission decision, and `neojs --allow-remote` is its
//! command-line form.
//!
//! A `jsr:@scope/name[@constraint][/subpath]` import is parsed by
//! [`crate::jsr`], resolved against the public registry's package metadata and
//! the selected version's metadata, and loaded as the canonical
//! `https://jsr.io/@scope/name/<version>/<file>` module. Every file's declared
//! length and SHA-256 checksum are verified before the TypeScript stripper or
//! V8 sees the bytes. That check proves the body matches the version metadata
//! that was retrieved; it does not authenticate the metadata or the publisher.
//! The runtime caches parsed package and version metadata for its own lifetime;
//! the HTTPS client caches responses by URL under its own freshness and
//! revalidation policy.
//!
//! # The registry and the transport
//!
//! [`RemoteRegistry`] maps the three registry documents (`meta.json`,
//! `<version>_meta.json`, and one manifest file) to ordinary HTTPS GETs through
//! [`crate::remote::RemoteClient`]. Redirects, TLS, deadlines, request and byte
//! budgets, and the user-profile cache are the remote client's policy; the
//! loader never follows a redirect itself and treats a
//! [`RegistryError::Redirect`] from another registry as a contract violation.
//! A file is verified after a cached read too, but that is not an authentication
//! boundary: the cache is local mutable state, and an attacker who consistently
//! rewrites both a cached manifest and a cached body defeats the checksum.
//!
//! [`deno_core::ModuleLoader::resolve`] is synchronous. The first import of a
//! package or version therefore performs its metadata GETs on the isolate
//! thread, using the same blocking transport as remote entry and module
//! loading. That is a latency limitation, not a trust boundary; avoiding it
//! needs a prefetch or an asynchronous resolution stage.
//!
//! # Direct HTTPS and the JSR graph
//!
//! Only a `jsr:` import, a relative import from a successfully loaded JSR
//! module, or an explicit `https://jsr.io` module URL imported by a successfully
//! loaded JSR module joins the verified JSR graph. An ordinary
//! `https://jsr.io/...` import from a local
//! or remote module keeps the HTTPS loader's semantics and is not checked
//! against registry metadata, so a prepared `run_url` entry is used as-is. A
//! canonical jsr.io URL first loaded as ordinary HTTPS cannot later be reused by
//! a `jsr:` import: V8 would share the unverified module identity, so the
//! `jsr:` resolution is refused instead.
//!
//! The final URL of every ordinary remote load is observed before V8 registers
//! it. A redirect that lands on an already verified JSR identity is refused
//! before registration; a redirect that lands on a canonical jsr.io URL is
//! recorded under that final URL, so a later `jsr:` import cannot reuse the
//! unverified redirect body. A prepared `run_url` entry goes through the same
//! observation.
//!
//! A `jsr:` resolution is pending, not loaded. Only a successful verified load
//! marks the identity as loaded and makes it a collision authority. An
//! `import.meta.resolve` that never loads therefore does not refuse an ordinary
//! redirect, and a failed load does not either. A pending or failed resolution
//! also does not classify a referrer as JSR: an ordinary module registered under
//! the same URL keeps its relative imports on the ordinary remote path. A
//! pending resolution still selects the verified load path if V8 later loads
//! the URL; the original specifier is not available at load time, and choosing
//! verification is the safe direction.
//!
//! # Graph boundary
//!
//! A registry module may import another registry module, a relative module
//! inside its exact package version, a built-in, or an absolute `https:` URL
//! when remote loading is enabled. A canonical `jsr.io` URL joins the verified
//! graph; another origin is an ordinary remote module. It may not import
//! `file:`: a registry module
//! cannot use an import to read the user's files. Relative imports are resolved
//! against the canonical URL and must stay inside the package version
//! directory; a module cannot use `..`, an absolute path, or a protocol-relative
//! URL to reach another package or version.
//!
//! The loader keeps aggregate file, byte, and metadata-document budgets for one
//! runtime. The remote client keeps its own module, byte, and request budgets.
//! None of this establishes publisher identity, and executing a module is not a
//! sandbox: the module has the host's authority through the built-ins it can
//! import.
//!
//! # Not implemented
//!
//! `npm:`, `node:`, and `gh:` imports, import maps and `neo.jsonc`, dist-tags,
//! lockfiles, offline or frozen policy, private registries, authentication,
//! custom origins, JSR package tarballs, and the resolver's optional module
//! graph hints are not implemented. A JSR package that imports any of those
//! fails at resolution.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;

use deno_core::ModuleLoadOptions;
use deno_core::ModuleLoadReferrer;
use deno_core::ModuleLoadResponse;
use deno_core::ModuleLoader;
use deno_core::ModuleResolveResponse;
use deno_core::ModuleSource;
use deno_core::ModuleSourceCode;
use deno_core::ModuleType;
use deno_core::ResolutionKind;
use deno_core::url::Url;
use deno_error::JsErrorBox;
use semver::Version;

use crate::jsr;
use crate::jsr::PackageId;
use crate::jsr::PackageMetadata;
use crate::jsr::PackageVersion;
use crate::jsr::ResolvedModule;
use crate::jsr::Specifier;
use crate::jsr::VersionMetadata;
use crate::loader::FileModuleLoader;
use crate::loader::ModuleFormat;
use crate::loader::module_format;
use crate::loader::unsupported_module_type;
use crate::remote::RemoteClient;
use crate::remote::display_url;
use crate::typescript;

/// The public registry origin used in canonical module URLs.
const REGISTRY_ORIGIN: &str = "https://jsr.io";

/// The public registry host.
const REGISTRY_HOST: &str = "jsr.io";

/// Resource bounds for one runtime's JSR module graph.
///
/// The defaults are deliberately modest. They bound one runtime's work; they
/// are not a network deadline, a CPU budget, or a sandbox.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Maximum declared and downloaded bytes in one file.
    pub(crate) max_file_bytes: usize,
    /// Maximum distinct files in one module graph.
    pub(crate) max_graph_files: usize,
    /// Maximum declared and downloaded bytes across one module graph.
    pub(crate) max_graph_bytes: usize,
    /// Maximum distinct package and version metadata documents fetched.
    pub(crate) max_metadata_documents: usize,
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

/// A failure a registry transport reports to the loader.
///
/// The loader adds the package, file, and importer context. A transport must
/// follow redirects itself and must not put credentials or other secrets in a
/// [`Self::Redirect`] location or an [`Self::Unavailable`] reason; both are
/// shown to the program and the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RegistryError {
    /// The registry has no such package, version, or file.
    NotFound,
    /// The transport returned a redirect without following it.
    ///
    /// [`RemoteRegistry`] lets the remote client follow checked redirects and
    /// never returns this. It remains part of the [`Registry`] contract so
    /// another registry can report a redirect the loader must not follow.
    #[cfg_attr(not(test), allow(dead_code))]
    Redirect(String),
    /// The transport could not complete the request for another reason.
    Unavailable(String),
}

/// A synchronous source of JSR registry metadata and file bytes.
///
/// Implementations are called from [`deno_core::ModuleLoader::resolve`] and
/// [`deno_core::ModuleLoader::load`]. They must not block on a network or a
/// long-running job: the calls run on the isolate thread. A real HTTPS
/// transport belongs behind a prefetch or an asynchronous resolution stage, not
/// behind a hidden blocking call here.
pub(crate) trait Registry {
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
pub(crate) struct RemoteRegistry {
    remote: RemoteClient,
}

impl RemoteRegistry {
    /// Creates a registry over an existing remote client.
    ///
    /// The client carries the runtime's request, module, byte, redirect, and
    /// deadline limits and its user-profile cache.
    pub(crate) fn new(remote: RemoteClient) -> Self {
        Self { remote }
    }

    /// Fetches one registry document and returns its body.
    fn fetch(&self, url: Url) -> Result<Vec<u8>, RegistryError> {
        match self.remote.fetch(&url) {
            Ok(fetched) => Ok(fetched.body),
            Err(reason) if is_missing(&reason) => Err(RegistryError::NotFound),
            Err(reason) => Err(RegistryError::Unavailable(reason)),
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

/// Returns whether a remote error means the registry has no such document.
///
/// [`crate::remote::RemoteClient`] reports an HTTP error status as
/// `HTTP <status> from <url>`. Only a 404 or 410 is a missing document; every
/// other failure stays an unavailable transport so the loader does not turn a
/// server fault into "not found".
fn is_missing(reason: &str) -> bool {
    reason.starts_with("HTTP 404 from ") || reason.starts_with("HTTP 410 from ")
}

fn package_metadata_url(package: &PackageId) -> Result<Url, RegistryError> {
    registry_url(
        format!("{REGISTRY_ORIGIN}/{}/meta.json", package),
        "package metadata",
        &package.to_string(),
    )
}

fn version_metadata_url(package: &PackageVersion) -> Result<Url, RegistryError> {
    registry_url(
        format!(
            "{REGISTRY_ORIGIN}/{}/{}_meta.json",
            package.package, package.version
        ),
        "version metadata",
        &package.to_string(),
    )
}

fn file_url(package: &PackageVersion, path: &str) -> Result<Url, RegistryError> {
    jsr::validate_manifest_path(path).map_err(RegistryError::Unavailable)?;
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

#[derive(Default)]
struct GraphState {
    loaded: BTreeSet<String>,
    bytes: usize,
    metadata_documents: usize,
}

/// A module loader that resolves and verifies `jsr:` modules from a [`Registry`].
///
/// It delegates every other specifier to its [`FileModuleLoader`], so built-in,
/// local-file, and non-registry HTTPS imports keep the same behaviour. A
/// resolved JSR module's identity is its canonical
/// `https://jsr.io/@scope/name/<version>/<file>` URL, which names the exact
/// version and suppresses a range from becoming a second module instance.
pub(crate) struct JsrModuleLoader {
    registry: Rc<dyn Registry>,
    limits: Limits,
    /// Delegate for built-in, local-file, and ordinary HTTPS specifiers.
    ///
    /// A normal runtime shares this with [`crate::Runtime`], so a prepared
    /// remote entry and the remote client are the same objects.
    files: Rc<FileModuleLoader>,
    packages: RefCell<BTreeMap<String, PackageMetadata>>,
    versions: RefCell<BTreeMap<String, VersionMetadata>>,
    graph: RefCell<GraphState>,
    /// Canonical URLs resolved from a `jsr:` specifier or a relative import
    /// from a resolved JSR module, and not yet loaded successfully.
    ///
    /// This set selects the verified load path. It is deliberately not a
    /// collision authority: `import.meta.resolve` can populate it without a
    /// later load, and a failed load must not refuse an ordinary redirect.
    jsr_pending: Rc<RefCell<BTreeSet<String>>>,
    /// Canonical URLs whose verified module source was handed to V8.
    ///
    /// This is the collision authority for ordinary HTTPS redirects. A source
    /// handed to V8 is treated as registered for identity purposes; a later
    /// compilation or evaluation failure does not clear it.
    jsr_loaded: Rc<RefCell<BTreeSet<String>>>,
    /// `jsr.io` URLs whose ordinary HTTPS module source was handed to V8.
    ///
    /// Both the requested and final identities are recorded, because V8
    /// registers under the final redirect URL and aliases the requested URL.
    /// A later `jsr:` import of either identity is refused. A later compilation
    /// or evaluation failure does not clear the record.
    ordinary_loaded: Rc<RefCell<BTreeSet<String>>>,
}

impl JsrModuleLoader {
    /// Creates a loader over a registry and a file-loader delegate.
    pub(crate) fn new(
        registry: Rc<dyn Registry>,
        files: Rc<FileModuleLoader>,
        limits: Limits,
    ) -> Self {
        let jsr_pending = Rc::new(RefCell::new(BTreeSet::new()));
        let jsr_loaded = Rc::new(RefCell::new(BTreeSet::new()));
        let ordinary_loaded = Rc::new(RefCell::new(BTreeSet::new()));
        let final_jsr_loaded = Rc::clone(&jsr_loaded);
        let final_ordinary = Rc::clone(&ordinary_loaded);
        files.set_final_url_observer(Rc::new(
            move |requested: &Url, final_url: &Url| {
                let requested_identities = registry_identities_if_registry(requested);
                let final_identities = registry_identities_if_registry(final_url);
                {
                    let loaded = final_jsr_loaded.borrow();
                    if requested_identities
                        .iter()
                        .chain(final_identities.iter())
                        .any(|identity| loaded.contains(identity))
                    {
                        return Err(format!(
                            "the final URL `{}` is already a verified JSR module; an ordinary HTTPS load cannot register a module under that identity",
                            display_url(final_url)
                        ));
                    }
                }
                let mut ordinary = final_ordinary.borrow_mut();
                for identity in requested_identities.into_iter().chain(final_identities) {
                    ordinary.insert(identity);
                }
                Ok(())
            },
        ));
        Self {
            registry,
            limits,
            files,
            packages: RefCell::new(BTreeMap::new()),
            versions: RefCell::new(BTreeMap::new()),
            graph: RefCell::new(GraphState::default()),
            jsr_pending,
            jsr_loaded,
            ordinary_loaded,
        }
    }

    /// Creates a loader with its own local-only file-loader delegate.
    ///
    /// The crate tests use this; a normal runtime passes a shared file loader
    /// so a prepared remote entry and the remote client are the same objects.
    #[cfg(test)]
    pub(crate) fn with_limits(registry: Rc<dyn Registry>, limits: Limits) -> Self {
        Self::new(registry, Rc::new(FileModuleLoader::new(None)), limits)
    }

    /// Resolves one `jsr:` specifier against the registry.
    ///
    /// The first import of a package or version performs its metadata requests
    /// here. [`deno_core::ModuleLoader::resolve`] is synchronous, so that work
    /// blocks the isolate thread with the same transport as remote loading.
    fn resolve_jsr(&self, input: &str, referrer: &str) -> ModuleResolveResponse {
        let result = (|| -> Result<Url, String> {
            let specifier = Specifier::parse(input).map_err(|error| error.to_string())?;
            let package = self
                .package_metadata(&specifier.package)
                .map_err(|reason| reason.to_string())?;
            let selected = package
                .select(&specifier.constraint)
                .map_err(|error| error.to_string())?;
            let package_version = PackageVersion {
                package: specifier.package.clone(),
                version: selected.version,
            };
            let version = self
                .version_metadata(&package_version)
                .map_err(|reason| reason.to_string())?;
            let resolved = jsr::resolve_module(&specifier, &package, &version)
                .map_err(|error| error.to_string())?;
            let url = canonical_module_url(&resolved)?;
            if self.is_ordinary_loaded(&url) {
                return Err(format!(
                    "`{url}` was already loaded as an ordinary HTTPS module; a `jsr:` import cannot add registry verification to that module. Use the `jsr:` import first, or a new Runtime."
                ));
            }
            self.pend(&url);
            Ok(url)
        })();
        result.map_err(|reason| resolve_error(input, referrer, reason))
    }

    /// Resolves a relative import from a previously resolved JSR module.
    ///
    /// The joined URL must stay on the registry origin and inside the exact
    /// package version directory. A module cannot use `..`, an absolute path,
    /// or a protocol-relative URL to reach another package or version.
    fn resolve_relative(&self, input: &str, referrer: &str) -> ModuleResolveResponse {
        let result = (|| -> Result<Url, String> {
            if let Some(character) = input.chars().find(|character| {
                matches!(character, '%' | '\\' | '?' | '#') || character.is_control()
            }) {
                return Err(format!(
                    "the import contains `{character}`, which is not accepted in a JSR relative import"
                ));
            }
            let base = Url::parse(referrer).map_err(|_| "the referrer is not a URL".to_owned())?;
            let Some(base_module) = parse_module_url(&base)? else {
                return Err("the referrer is not a JSR module URL".to_owned());
            };
            let joined = base
                .join(input)
                .map_err(|error| format!("the import is not a valid relative URL: {error}"))?;
            if joined.scheme() != "https" || joined.host_str() != Some(REGISTRY_HOST) {
                return Err("the relative import leaves the registry origin".to_owned());
            }
            let prefix = format!("/{}/{}/", base_module.id.package, base_module.id.version);
            if !joined.path().starts_with(&prefix) {
                return Err("the relative import leaves the package version directory".to_owned());
            }
            let Some(joined_module) = parse_module_url(&joined)? else {
                return Err("the relative import is not a canonical JSR module path".to_owned());
            };
            if joined_module.id != base_module.id {
                return Err("the relative import leaves the package version directory".to_owned());
            }
            // Normalize the encoding chosen by the URL parser so one file has
            // one module identity however the importer spelled it.
            let url = canonical_url_for(&joined_module)?;
            if self.is_ordinary_loaded(&url) {
                return Err(format!(
                    "`{url}` was already loaded as an ordinary HTTPS module; a JSR graph import cannot add registry verification to that module"
                ));
            }
            self.pend(&url);
            Ok(url)
        })();
        result.map_err(|reason| resolve_error(input, referrer, reason))
    }

    /// Records a canonical URL as resolved through the JSR path.
    ///
    /// This does not mark it loaded; only a successful `load_jsr` does that.
    fn pend(&self, url: &Url) {
        if !self.is_jsr_loaded(url) {
            self.jsr_pending.borrow_mut().insert(url.to_string());
        }
    }

    /// Returns whether a URL is pending or loaded as a JSR module.
    fn is_jsr_candidate(&self, url: &Url) -> bool {
        let identities = registry_identities(url);
        let pending = self.jsr_pending.borrow();
        let loaded = self.jsr_loaded.borrow();
        identities
            .iter()
            .any(|identity| pending.contains(identity) || loaded.contains(identity))
    }

    /// Returns whether a URL's verified source was handed to V8.
    fn is_jsr_loaded(&self, url: &Url) -> bool {
        let loaded = self.jsr_loaded.borrow();
        registry_identities(url)
            .iter()
            .any(|identity| loaded.contains(identity))
    }

    /// Marks a URL as loaded after its verified source is prepared.
    fn mark_jsr_loaded(&self, url: &Url) {
        let identities = registry_identities(url);
        {
            let mut pending = self.jsr_pending.borrow_mut();
            for identity in &identities {
                pending.remove(identity);
            }
        }
        let mut loaded = self.jsr_loaded.borrow_mut();
        for identity in identities {
            loaded.insert(identity);
        }
    }

    /// Returns whether a URL's ordinary source was handed to V8.
    fn is_ordinary_loaded(&self, url: &Url) -> bool {
        let loaded = self.ordinary_loaded.borrow();
        registry_identities(url)
            .iter()
            .any(|identity| loaded.contains(identity))
    }

    /// Handles an explicit `https://jsr.io` import from a successfully loaded
    /// JSR module.
    ///
    /// Returns `None` when the specifier is not a `jsr.io` URL, so the caller
    /// can delegate it. A canonical module URL joins the verified JSR graph; a
    /// `jsr.io` URL that is not canonical is refused instead of falling through
    /// to the ordinary remote loader, which would execute it without the
    /// version-manifest check.
    fn resolve_registry_https(&self, input: &str, referrer: &str) -> Option<ModuleResolveResponse> {
        let url = Url::parse(input).ok()?;
        if url.scheme() != "https" || url.host_str() != Some(REGISTRY_HOST) {
            return None;
        }
        let result = (|| -> Result<Url, String> {
            let Some(module) = parse_module_url(&url)? else {
                return Err(
                    "a JSR module can import only a canonical https://jsr.io module URL, with no credentials, query, or fragment"
                        .to_owned(),
                );
            };
            let canonical = canonical_url_for(&module)?;
            if self.is_ordinary_loaded(&canonical) {
                return Err(format!(
                    "`{canonical}` was already loaded as an ordinary HTTPS module; a JSR graph import cannot add registry verification to that module"
                ));
            }
            self.pend(&canonical);
            Ok(canonical)
        })();
        Some(result.map_err(|reason| resolve_error(input, referrer, reason)))
    }

    fn package_metadata(&self, package: &PackageId) -> Result<PackageMetadata, LoadFailure> {
        let key = package.to_string();
        if let Some(cached) = self.packages.borrow().get(&key) {
            return Ok(cached.clone());
        }
        self.charge_metadata()?;
        let json = self
            .registry
            .package_metadata(package)
            .map_err(|error| registry_failure("package metadata", &key, error))?;
        let metadata =
            PackageMetadata::parse(&json).map_err(|error| LoadFailure::new(error.to_string()))?;
        if metadata.package != *package {
            return Err(LoadFailure::new(format!(
                "the registry returned metadata for `{}` when `{key}` was requested",
                metadata.package
            )));
        }
        self.packages.borrow_mut().insert(key, metadata.clone());
        Ok(metadata)
    }

    fn version_metadata(&self, package: &PackageVersion) -> Result<VersionMetadata, LoadFailure> {
        let key = package.to_string();
        if let Some(cached) = self.versions.borrow().get(&key) {
            return Ok(cached.clone());
        }
        self.charge_metadata()?;
        let json = self
            .registry
            .version_metadata(package)
            .map_err(|error| registry_failure("version metadata", &key, error))?;
        let metadata = VersionMetadata::parse(&json, package.clone())
            .map_err(|error| LoadFailure::new(error.to_string()))?;
        self.versions.borrow_mut().insert(key, metadata.clone());
        Ok(metadata)
    }

    fn charge_metadata(&self) -> Result<(), LoadFailure> {
        let mut graph = self.graph.borrow_mut();
        if graph.metadata_documents >= self.limits.max_metadata_documents {
            return Err(LoadFailure::new(format!(
                "the JSR module graph already requested {} metadata documents, the limit",
                self.limits.max_metadata_documents
            )));
        }
        graph.metadata_documents += 1;
        Ok(())
    }

    /// Reserves a distinct module's declared bytes before fetching it.
    ///
    /// A URL that was already loaded is not charged again. A failed fetch keeps
    /// its reservation, so repeatedly catching a failing import cannot grow the
    /// graph without bound; the runtime is normally consumed by the first
    /// failure anyway.
    fn reserve(&self, url: &str, declared: u64) -> Result<(), LoadFailure> {
        let declared = usize::try_from(declared).map_err(|_| {
            LoadFailure::new(format!(
                "the declared size {declared} does not fit this platform's address space"
            ))
        })?;
        let mut graph = self.graph.borrow_mut();
        if graph.loaded.contains(url) {
            return Ok(());
        }
        if graph.loaded.len() >= self.limits.max_graph_files {
            return Err(LoadFailure::new(format!(
                "the JSR module graph already has {} files, the limit",
                self.limits.max_graph_files
            )));
        }
        let total = graph.bytes.checked_add(declared).ok_or_else(|| {
            LoadFailure::new("the JSR module graph byte count overflowed".to_owned())
        })?;
        if total > self.limits.max_graph_bytes {
            return Err(LoadFailure::new(format!(
                "loading it would bring the JSR module graph to {total} bytes, over the {}-byte limit",
                self.limits.max_graph_bytes
            )));
        }
        graph.loaded.insert(url.to_owned());
        graph.bytes = total;
        Ok(())
    }

    /// Loads one resolved JSR graph module after verifying it against the
    /// version manifest.
    fn load_jsr(&self, url: &Url, referrer: &str) -> Result<ModuleSource, LoadFailure> {
        if self.is_ordinary_loaded(url) {
            return Err(self.load_failure(
                url,
                referrer,
                "the URL is already loaded as an ordinary HTTPS module; a `jsr:` import cannot add registry verification to that module".to_owned(),
            ));
        }
        let Some(module) =
            parse_module_url(url).map_err(|reason| self.load_failure(url, referrer, reason))?
        else {
            return Err(self.load_failure(
                url,
                referrer,
                "the resolved JSR module URL is not a canonical registry module".to_owned(),
            ));
        };
        let format = module_format(Path::new(module.path.as_str()))
            .map_err(|reason| self.load_failure(url, referrer, reason))?;
        let metadata = self
            .version_metadata(&module.id)
            .map_err(|failure| failure.at(referrer))?;
        let declared = metadata
            .file_size(&module.path)
            .map_err(|error| self.load_failure(url, referrer, error.to_string()))?;
        if declared > self.limits.max_file_bytes as u64 {
            return Err(self.load_failure(
                url,
                referrer,
                format!(
                    "the file is {declared} bytes, over the {}-byte per-file limit",
                    self.limits.max_file_bytes
                ),
            ));
        }
        self.reserve(url.as_str(), declared)
            .map_err(|failure| failure.at(referrer))?;
        let bytes = self
            .registry
            .file(&module.id, &module.path)
            .map_err(|error| {
                self.load_failure(
                    url,
                    referrer,
                    registry_failure("file", &module.path, error).to_string(),
                )
            })?;
        metadata
            .verify_file(&module.path, &bytes)
            .map_err(|error| self.load_failure(url, referrer, error.to_string()))?;

        let code = match format {
            ModuleFormat::JavaScript => ModuleSourceCode::Bytes(bytes.into_boxed_slice().into()),
            ModuleFormat::TypeScript => {
                let text = String::from_utf8(bytes).map_err(|_| {
                    self.load_failure(
                        url,
                        referrer,
                        "the TypeScript file is not valid UTF-8".to_owned(),
                    )
                })?;
                let stripped = typescript::strip(url, text)
                    .map_err(|reason| self.load_failure(url, referrer, reason))?;
                ModuleSourceCode::String(stripped.into())
            }
        };
        Ok(ModuleSource::new(ModuleType::JavaScript, code, url, None))
    }

    fn load_failure(&self, url: &Url, referrer: &str, reason: String) -> LoadFailure {
        LoadFailure::new(format!("cannot load {url}: {reason}")).at(referrer)
    }
}

impl ModuleLoader for JsrModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> ModuleResolveResponse {
        if specifier.starts_with("jsr:") {
            return self.resolve_jsr(specifier, referrer);
        }
        // Only a successfully verified JSR module classifies its relative
        // imports as JSR. A pending resolution, or a failed one, does not: an
        // ordinary module may be registered under the same URL, and its
        // relative graph must stay on the ordinary remote path.
        let referrer_is_jsr = Url::parse(referrer).is_ok_and(|url| self.is_jsr_loaded(&url));
        if referrer_is_jsr {
            if is_relative_reference(specifier) {
                return self.resolve_relative(specifier, referrer);
            }
            if let Some(reason) = local_file_reason(specifier) {
                return Err(JsErrorBox::generic(format!(
                    "cannot resolve `{specifier}` (imported from {referrer}): {reason}"
                )));
            }
            if let Some(response) = self.resolve_registry_https(specifier, referrer) {
                return response;
            }
        }
        self.files.resolve(specifier, referrer, kind)
    }

    fn load(
        &self,
        module_specifier: &deno_core::ModuleSpecifier,
        maybe_referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        if let Some(reason) = unsupported_module_type(&options.requested_module_type) {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {module_specifier}: {reason}"
            ))));
        }
        let referrer = maybe_referrer
            .map(|referrer| referrer.specifier.as_str())
            .unwrap_or("");
        if self.is_jsr_candidate(module_specifier) {
            return match self.load_jsr(module_specifier, referrer) {
                Ok(source) => {
                    self.mark_jsr_loaded(module_specifier);
                    ModuleLoadResponse::Sync(Ok(source))
                }
                Err(failure) => {
                    ModuleLoadResponse::Sync(Err(JsErrorBox::generic(failure.to_string())))
                }
            };
        }
        self.files.load(module_specifier, maybe_referrer, options)
    }
}

/// One resolved module identity plus its manifest path.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ModuleKey {
    id: PackageVersion,
    path: String,
}

/// One loader failure, with the import location added where known.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LoadFailure {
    reason: String,
    referrer: Option<String>,
}

impl LoadFailure {
    fn new(reason: String) -> Self {
        Self {
            reason,
            referrer: None,
        }
    }

    fn at(mut self, referrer: &str) -> Self {
        if !referrer.is_empty() {
            self.referrer = Some(referrer.to_owned());
        }
        self
    }
}

impl From<String> for LoadFailure {
    fn from(reason: String) -> Self {
        Self::new(reason)
    }
}

impl std::fmt::Display for LoadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)?;
        if let Some(referrer) = &self.referrer {
            write!(formatter, " (imported from {referrer})")?;
        }
        Ok(())
    }
}

fn resolve_error(specifier: &str, referrer: &str, reason: String) -> JsErrorBox {
    let message = if referrer.is_empty() {
        format!("cannot resolve `{specifier}`: {reason}")
    } else {
        format!("cannot resolve `{specifier}` (imported from {referrer}): {reason}")
    };
    JsErrorBox::generic(message)
}

fn registry_failure(action: &str, subject: &str, error: RegistryError) -> LoadFailure {
    let reason = match error {
        RegistryError::NotFound => {
            format!("the registry has no {action} for `{subject}`")
        }
        RegistryError::Redirect(location) => format!(
            "the registry transport redirected the {action} for `{subject}` to `{location}`; redirects must be followed by the transport, not the loader"
        ),
        RegistryError::Unavailable(reason) => {
            format!("the registry could not provide the {action} for `{subject}`: {reason}")
        }
    };
    LoadFailure::new(reason)
}

/// Returns whether a specifier is a relative URL reference.
fn is_relative_reference(specifier: &str) -> bool {
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
/// A JSR module may import another registry module or a neojs built-in. It may
/// not reach the local filesystem through `file:`; that keeps the module graph
/// on the registry and avoids turning a registry package into a reader for the
/// user's files. This is a module-graph boundary, not a sandbox.
fn local_file_reason(specifier: &str) -> Option<&'static str> {
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
fn canonical_module_url(resolved: &ResolvedModule) -> Result<Url, String> {
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
fn canonical_url_for(module: &ModuleKey) -> Result<Url, String> {
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
fn registry_identities(url: &Url) -> Vec<String> {
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
fn registry_identities_if_registry(url: &Url) -> Vec<String> {
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
fn parse_module_url(url: &Url) -> Result<Option<ModuleKey>, String> {
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
    jsr::validate_manifest_path(&path)?;
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use deno_core::ModuleLoader;
    use sha2::Digest;
    use sha2::Sha256;

    use super::JsrModuleLoader;
    use super::Limits;
    use super::Registry;
    use super::RegistryError;
    use crate::Error;
    use crate::Runtime;
    use crate::jsr::PackageId;
    use crate::jsr::PackageVersion;

    const ZERO_CHECKSUM: &str =
        "sha256-0000000000000000000000000000000000000000000000000000000000000000";

    /// A deterministic in-memory registry used only by these tests.
    ///
    /// The builder computes manifest sizes and checksums from the file bytes it
    /// is given. A test that needs an inconsistent document sets raw metadata
    /// explicitly with [`Self::metadata`].
    struct MockJsrRegistry {
        packages: BTreeMap<String, MockPackage>,
        versions: BTreeMap<String, MockVersion>,
        files: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
        package_redirects: BTreeMap<String, String>,
        package_unavailable: BTreeMap<String, String>,
        version_redirects: BTreeMap<String, String>,
        file_redirects: BTreeMap<(String, String), String>,
    }

    #[derive(Default)]
    struct MockPackage {
        versions: BTreeMap<String, bool>,
    }

    #[derive(Default)]
    struct MockVersion {
        exports: BTreeMap<String, String>,
        metadata: Option<String>,
    }

    impl MockJsrRegistry {
        fn new() -> Self {
            Self {
                packages: BTreeMap::new(),
                versions: BTreeMap::new(),
                files: BTreeMap::new(),
                package_redirects: BTreeMap::new(),
                package_unavailable: BTreeMap::new(),
                version_redirects: BTreeMap::new(),
                file_redirects: BTreeMap::new(),
            }
        }

        fn package(mut self, scope: &str, name: &str, versions: &[(&str, bool)]) -> Self {
            let entry = self.packages.entry(package_key(scope, name)).or_default();
            for (version, yanked) in versions {
                entry.versions.insert((*version).to_owned(), *yanked);
            }
            self
        }

        fn version(
            mut self,
            scope: &str,
            name: &str,
            version: &str,
            exports: &[(&str, &str)],
        ) -> Self {
            self.ensure_version(scope, name, version);
            let entry = self
                .versions
                .entry(version_key(scope, name, version))
                .or_default();
            for (key, target) in exports {
                entry
                    .exports
                    .insert((*key).to_owned(), (*target).to_owned());
            }
            self
        }

        fn file(
            mut self,
            scope: &str,
            name: &str,
            version: &str,
            path: &str,
            source: &[u8],
        ) -> Self {
            self.ensure_version(scope, name, version);
            self.files
                .entry(version_key(scope, name, version))
                .or_default()
                .insert(path.to_owned(), source.to_vec());
            self
        }

        fn metadata(mut self, scope: &str, name: &str, version: &str, json: &str) -> Self {
            self.ensure_version(scope, name, version);
            self.versions
                .entry(version_key(scope, name, version))
                .or_default()
                .metadata = Some(json.to_owned());
            self
        }

        fn redirect_package(mut self, scope: &str, name: &str, location: &str) -> Self {
            self.package_redirects
                .insert(package_key(scope, name), location.to_owned());
            self
        }

        fn unavailable_package(mut self, scope: &str, name: &str, reason: &str) -> Self {
            self.package_unavailable
                .insert(package_key(scope, name), reason.to_owned());
            self
        }

        fn redirect_version(
            mut self,
            scope: &str,
            name: &str,
            version: &str,
            location: &str,
        ) -> Self {
            self.version_redirects
                .insert(version_key(scope, name, version), location.to_owned());
            self
        }

        fn redirect_file(
            mut self,
            scope: &str,
            name: &str,
            version: &str,
            path: &str,
            location: &str,
        ) -> Self {
            self.file_redirects.insert(
                (version_key(scope, name, version), path.to_owned()),
                location.to_owned(),
            );
            self
        }

        fn ensure_version(&mut self, scope: &str, name: &str, version: &str) {
            self.packages
                .entry(package_key(scope, name))
                .or_default()
                .versions
                .entry(version.to_owned())
                .or_insert(false);
        }
    }

    impl Registry for MockJsrRegistry {
        fn package_metadata(&self, package: &PackageId) -> Result<String, RegistryError> {
            let key = package.to_string();
            if let Some(location) = self.package_redirects.get(&key) {
                return Err(RegistryError::Redirect(location.clone()));
            }
            if let Some(reason) = self.package_unavailable.get(&key) {
                return Err(RegistryError::Unavailable(reason.clone()));
            }
            let Some(entry) = self.packages.get(&key) else {
                return Err(RegistryError::NotFound);
            };
            let mut versions = serde_json::Map::new();
            for (version, yanked) in &entry.versions {
                versions.insert(version.clone(), serde_json::json!({ "yanked": yanked }));
            }
            Ok(serde_json::json!({
                "scope": package.scope,
                "name": package.name,
                "versions": versions,
            })
            .to_string())
        }

        fn version_metadata(&self, package: &PackageVersion) -> Result<String, RegistryError> {
            let key = package.to_string();
            if let Some(location) = self.version_redirects.get(&key) {
                return Err(RegistryError::Redirect(location.clone()));
            }
            let Some(entry) = self.versions.get(&key) else {
                return Err(RegistryError::NotFound);
            };
            if let Some(metadata) = &entry.metadata {
                return Ok(metadata.clone());
            }
            let files = self.files.get(&key).cloned().unwrap_or_default();
            let mut manifest = serde_json::Map::new();
            for (path, bytes) in files {
                manifest.insert(
                    path,
                    serde_json::json!({
                        "size": bytes.len(),
                        "checksum": checksum(&bytes),
                    }),
                );
            }
            let mut document = serde_json::json!({ "manifest": manifest });
            if !entry.exports.is_empty() {
                document["exports"] = serde_json::json!(entry.exports);
            }
            Ok(document.to_string())
        }

        fn file(&self, package: &PackageVersion, path: &str) -> Result<Vec<u8>, RegistryError> {
            let key = package.to_string();
            if let Some(location) = self.file_redirects.get(&(key.clone(), path.to_owned())) {
                return Err(RegistryError::Redirect(location.clone()));
            }
            self.files
                .get(&key)
                .and_then(|files| files.get(path))
                .cloned()
                .ok_or(RegistryError::NotFound)
        }
    }

    fn package_key(scope: &str, name: &str) -> String {
        format!("@{scope}/{name}")
    }

    fn version_key(scope: &str, name: &str, version: &str) -> String {
        format!("@{scope}/{name}@{version}")
    }

    fn checksum(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let digest = Sha256::digest(bytes);
        let mut text = String::from("sha256-");
        for byte in digest {
            let _ = write!(text, "{byte:02x}");
        }
        text
    }

    fn run_with_jsr(registry: MockJsrRegistry, source: &str) -> Result<(), Error> {
        run_with_jsr_limits(registry, Limits::default(), source)
    }

    fn run_with_jsr_limits(
        registry: MockJsrRegistry,
        limits: Limits,
        source: &str,
    ) -> Result<(), Error> {
        let directory = tempfile::tempdir().expect("temp dir");
        let entry = directory.path().join("main.js");
        std::fs::write(&entry, source).expect("write entry");
        let registry: Rc<dyn Registry> = Rc::new(registry);
        let mut runtime = Runtime::with_jsr_registry_limits(registry, limits)?;
        runtime.run_file(&entry)
    }

    fn run_default(source: &str) -> Result<(), Error> {
        let directory = tempfile::tempdir().expect("temp dir");
        let entry = directory.path().join("main.js");
        std::fs::write(&entry, source).expect("write entry");
        let mut runtime = Runtime::new()?;
        runtime.run_file(&entry)
    }

    fn error_message(result: Result<(), Error>) -> String {
        match result {
            Ok(()) => panic!("expected an error"),
            Err(error) => error.to_string(),
        }
    }

    // Integration test through the actual runtime: static and dynamic imports,
    // scoped names, exports, erasable TypeScript, JavaScript, and a relative
    // dependency are all exercised together.
    #[test]
    fn loads_static_and_dynamic_scoped_modules() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.1.0", false)])
            .version(
                "scope",
                "pkg",
                "1.1.0",
                &[(".", "./mod.ts"), ("./join", "./join.js")],
            )
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/mod.ts",
                b"import { isDigit } from \"chars\";\nimport { helper } from \"./lib/helper.ts\";\ninterface Box { value: number }\nexport const value: number = helper();\nexport const label: string = \"typed\";\nexport const builtin: boolean = isDigit(55);\n",
            )
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/lib/helper.ts",
                b"export function helper(): number { return 7; }\n",
            )
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/join.js",
                b"export function join(a, b) { return a + b; }\n",
            );

        run_with_jsr(
            registry,
            r#"
import { value, label, builtin } from "jsr:@scope/pkg@1";
const join = await import("jsr:@scope/pkg@1/join");
if (value !== 7) throw new Error("static import failed");
if (label !== "typed") throw new Error("type stripping failed");
if (builtin !== true) throw new Error("built-in import failed");
if (join.join("a", "b") !== "ab") throw new Error("dynamic import failed");
"#,
        )
        .expect("static and dynamic JSR imports must run");
    }

    // Integration test through the actual runtime: a range and an exact version
    // resolve to one canonical URL, yanked versions are skipped by a range but
    // still selectable explicitly, and prereleases are not selected by `@1`.
    #[test]
    fn selects_versions_and_resolves_one_exact_identity() {
        let registry = MockJsrRegistry::new()
            .package(
                "scope",
                "pkg",
                &[
                    ("1.0.0", false),
                    ("1.0.1", true),
                    ("1.1.0", false),
                    ("2.0.0-beta.1", false),
                ],
            )
            .version("scope", "pkg", "1.0.0", &[(".", "./v1.js")])
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/v1.js",
                b"globalThis.__jsr_loads = (globalThis.__jsr_loads ?? 0) + 1;\nexport const version = \"1.0.0\";\nexport const loads = globalThis.__jsr_loads;\n",
            )
            .version("scope", "pkg", "1.0.1", &[(".", "./v101.js")])
            .file(
                "scope",
                "pkg",
                "1.0.1",
                "/v101.js",
                b"export const version = \"1.0.1\";\n",
            )
            .version("scope", "pkg", "1.1.0", &[(".", "./v11.js")])
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/v11.js",
                b"globalThis.__jsr_loads = (globalThis.__jsr_loads ?? 0) + 1;\nexport const version = \"1.1.0\";\nexport const loads = globalThis.__jsr_loads;\n",
            );

        run_with_jsr(
            registry,
            r#"
const ranged = await import("jsr:@scope/pkg@1");
const exact = await import("jsr:@scope/pkg@1.1.0");
const older = await import("jsr:@scope/pkg@1.0.0");
const yanked = await import("jsr:@scope/pkg@1.0.1");
if (ranged.version !== "1.1.0") throw new Error(`range picked ${ranged.version}`);
if (ranged !== exact) throw new Error("the range and exact version are different modules");
if (ranged.loads !== 1) throw new Error(`the module evaluated ${ranged.loads} times`);
if (older.version !== "1.0.0") throw new Error(`exact picked ${older.version}`);
if (yanked.version !== "1.0.1") throw new Error(`yanked picked ${yanked.version}`);
const resolved = import.meta.resolve("jsr:@scope/pkg@1");
if (resolved !== "https://jsr.io/@scope/pkg/1.1.0/v11.js") {
    throw new Error(`unexpected canonical URL ${resolved}`);
}
"#,
        )
        .expect("version selection and identity must hold");
    }

    // Integration test through the actual runtime: relative dependencies stay
    // inside the exact package version and resolve through the same loader.
    #[test]
    fn resolves_relative_dependencies_within_the_package_version() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.1.0", false)])
            .version("scope", "pkg", "1.1.0", &[(".", "./mod.ts")])
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/mod.ts",
                b"import { doubled } from \"./lib/double.ts\";\nimport { plus } from \"./lib/a+b.ts\";\nexport const value: number = doubled(3) + plus();\n",
            )
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/lib/double.ts",
                b"import { base } from \"../base.ts\";\nexport function doubled(value: number): number { return base(value) * 2; }\n",
            )
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/lib/a+b.ts",
                b"export function plus(): number { return 0; }\n",
            )
            .file(
                "scope",
                "pkg",
                "1.1.0",
                "/base.ts",
                b"export function base(value: number): number { return value + 1; }\n",
            );

        run_with_jsr(
            registry,
            r#"
import { value } from "jsr:@scope/pkg@1";
if (value !== 8) throw new Error(`relative dependency returned ${value}`);
"#,
        )
        .expect("relative dependencies must resolve within the version");
    }

    // Integration test through the actual runtime: a cycle through relative
    // imports loads normally.
    #[test]
    fn handles_a_module_cycle() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version("scope", "pkg", "1.0.0", &[(".", "./a.ts")])
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/a.ts",
                b"import { fromB } from \"./b.ts\";\nexport function fromA(): string { return \"A\"; }\nexport const viaB = (): string => fromB();\n",
            )
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/b.ts",
                b"import { fromA } from \"./a.ts\";\nexport function fromB(): string { return \"B\" + fromA(); }\n",
            );

        run_with_jsr(
            registry,
            r#"
import { viaB } from "jsr:@scope/pkg@1";
if (viaB() !== "BA") throw new Error("module cycle did not evaluate");
"#,
        )
        .expect("a module cycle must load");
    }

    // Integration test through the actual runtime: traversal out of the exact
    // package version is refused with a precise reason.
    #[test]
    fn rejects_relative_imports_that_leave_the_package_version() {
        let cases = [
            ("../../evil.js", "leaves the package version directory"),
            ("/evil.js", "leaves the package version directory"),
            ("//evil.test/evil.js", "leaves the registry origin"),
            ("file:///etc/hosts", "cannot import a local file"),
        ];
        for (specifier, expected) in cases {
            let registry = MockJsrRegistry::new()
                .package("scope", "pkg", &[("1.0.0", false)])
                .version("scope", "pkg", "1.0.0", &[(".", "./mod.js")])
                .file(
                    "scope",
                    "pkg",
                    "1.0.0",
                    "/mod.js",
                    format!("import \"{specifier}\";\n").as_bytes(),
                );
            let message = error_message(run_with_jsr(registry, "import \"jsr:@scope/pkg@1\";\n"));
            assert!(
                message.contains(expected),
                "`{specifier}` produced `{message}`, expected `{expected}`"
            );
        }
    }

    // Integration test through the actual runtime: a missing export is reported
    // before any file is fetched.
    #[test]
    fn rejects_a_missing_export() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version("scope", "pkg", "1.0.0", &[(".", "./mod.js")])
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/mod.js",
                b"export const value = 1;\n",
            );

        let message = error_message(run_with_jsr(
            registry,
            "import \"jsr:@scope/pkg@1/missing\";\n",
        ));
        assert!(
            message.contains("the package version has no export `./missing`"),
            "{message}"
        );
    }

    // Integration test through the actual runtime: a manifest length mismatch
    // and a checksum mismatch are both reported before the module executes.
    #[test]
    fn rejects_manifest_mismatches_before_execution() {
        let source = b"throw new Error(\"EXECUTED\");\n";
        let cases = [
            (
                "size",
                format!(
                    r#"{{"manifest":{{"/mod.js":{{"size":5,"checksum":"{ZERO_CHECKSUM}"}}}},"exports":{{".":"./mod.js"}}}}"#
                ),
                "has 29 bytes, expected 5",
            ),
            (
                "checksum",
                format!(
                    r#"{{"manifest":{{"/mod.js":{{"size":{},"checksum":"{ZERO_CHECKSUM}"}}}},"exports":{{".":"./mod.js"}}}}"#,
                    source.len()
                ),
                "expected sha256-0000",
            ),
        ];
        for (name, metadata, expected) in cases {
            let registry = MockJsrRegistry::new()
                .package("scope", "pkg", &[("1.0.0", false)])
                .version("scope", "pkg", "1.0.0", &[])
                .metadata("scope", "pkg", "1.0.0", &metadata)
                .file("scope", "pkg", "1.0.0", "/mod.js", source);
            let message = error_message(run_with_jsr(registry, "import \"jsr:@scope/pkg@1\";\n"));
            assert!(
                message.contains("JSR file verification failed"),
                "{name}: {message}"
            );
            assert!(message.contains(expected), "{name}: {message}");
            assert!(
                !message.contains("EXECUTED"),
                "{name}: verification failed after execution: {message}"
            );
        }
    }

    // Integration test through the actual runtime: a transport redirect is
    // surfaced, not followed.
    #[test]
    fn rejects_a_transport_redirect_without_following_it() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .redirect_package("scope", "pkg", "https://elsewhere.test/meta.json");

        let message = error_message(run_with_jsr(registry, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(
            message.contains("redirected the package metadata"),
            "{message}"
        );
        assert!(
            message.contains("https://elsewhere.test/meta.json"),
            "{message}"
        );
        assert!(
            message.contains("redirects must be followed by the transport"),
            "{message}"
        );
    }

    // Integration test through the actual runtime: version metadata and file
    // redirects are surfaced, not followed.
    #[test]
    fn rejects_version_and_file_redirects() {
        let version = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .redirect_version(
                "scope",
                "pkg",
                "1.0.0",
                "https://elsewhere.test/version.json",
            );
        let message = error_message(run_with_jsr(version, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(
            message.contains("redirected the version metadata"),
            "{message}"
        );

        let file = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version("scope", "pkg", "1.0.0", &[(".", "./mod.js")])
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/mod.js",
                b"export const value = 1;\n",
            )
            .redirect_file(
                "scope",
                "pkg",
                "1.0.0",
                "/mod.js",
                "https://elsewhere.test/mod.js",
            );
        let message = error_message(run_with_jsr(file, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(message.contains("redirected the file"), "{message}");
    }

    // Integration test through the actual runtime: an unavailable transport is
    // reported with its own reason.
    #[test]
    fn reports_an_unavailable_transport() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .unavailable_package("scope", "pkg", "connection refused");

        let message = error_message(run_with_jsr(registry, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(message.contains("connection refused"), "{message}");
        assert!(
            message.contains("could not provide the package metadata"),
            "{message}"
        );
    }

    // Integration test through the actual runtime: TypeScript that needs a
    // transform is rejected after verification and before execution.
    #[test]
    fn rejects_typescript_that_needs_a_transform() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version("scope", "pkg", "1.0.0", &[(".", "./mod.ts")])
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/mod.ts",
                b"enum Color { Red }\nexport const value = Color.Red;\n",
            );

        let message = error_message(run_with_jsr(registry, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(
            message.contains("enum is not supported in strip-only mode"),
            "{message}"
        );
    }

    // Integration test through the actual runtime: the default runtime still
    // refuses `jsr:` and names the missing remote opt-in.
    #[test]
    fn the_default_runtime_still_rejects_jsr() {
        let message = error_message(run_default("import \"jsr:@scope/pkg@1\";\n"));
        assert!(
            message.contains("jsr: modules require remote module loading"),
            "{message}"
        );
    }

    // Integration test through the actual runtime: a file larger than the
    // per-file limit is refused before its body runs.
    #[test]
    fn runtime_enforces_the_per_file_limit_before_execution() {
        let source = b"throw new Error(\"EXECUTED\");\n";
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version("scope", "pkg", "1.0.0", &[(".", "./mod.js")])
            .file("scope", "pkg", "1.0.0", "/mod.js", source);
        let limits = Limits {
            max_file_bytes: 8,
            max_graph_files: 8,
            max_graph_bytes: 1024,
            max_metadata_documents: 8,
        };
        let message = error_message(run_with_jsr_limits(
            registry,
            limits,
            "import \"jsr:@scope/pkg@1\";\n",
        ));
        assert!(
            message.contains(&format!(
                "the file is {} bytes, over the 8-byte per-file limit",
                source.len()
            )),
            "{message}"
        );
        assert!(!message.contains("EXECUTED"), "{message}");
    }

    // Integration test through the actual runtime: the second file crosses the
    // graph-file limit and neither module body executes.
    #[test]
    fn runtime_enforces_the_graph_file_limit_before_execution() {
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version("scope", "pkg", "1.0.0", &[(".", "./root.js")])
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/root.js",
                b"import \"./dep.js\";\nthrow new Error(\"ENTRY_EXECUTED\");\n",
            )
            .file(
                "scope",
                "pkg",
                "1.0.0",
                "/dep.js",
                b"throw new Error(\"DEP_EXECUTED\");\n",
            );
        let limits = Limits {
            max_file_bytes: 1024,
            max_graph_files: 1,
            max_graph_bytes: 1024 * 1024,
            max_metadata_documents: 8,
        };
        let message = error_message(run_with_jsr_limits(
            registry,
            limits,
            "import \"jsr:@scope/pkg@1\";\n",
        ));
        assert!(
            message.contains("the JSR module graph already has 1 files, the limit"),
            "{message}"
        );
        assert!(!message.contains("ENTRY_EXECUTED"), "{message}");
        assert!(!message.contains("DEP_EXECUTED"), "{message}");
    }

    // Integration test through the actual runtime: the second file crosses the
    // aggregate byte limit and neither module body executes.
    #[test]
    fn runtime_enforces_the_graph_byte_limit_before_execution() {
        let root = b"import \"./dep.js\";\nthrow new Error(\"ENTRY_EXECUTED\");\n";
        let dep = b"throw new Error(\"DEP_EXECUTED\");\n";
        let budget = root.len() + dep.len() - 1;
        let registry = MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version("scope", "pkg", "1.0.0", &[(".", "./root.js")])
            .file("scope", "pkg", "1.0.0", "/root.js", root)
            .file("scope", "pkg", "1.0.0", "/dep.js", dep);
        let limits = Limits {
            max_file_bytes: 1024,
            max_graph_files: 8,
            max_graph_bytes: budget,
            max_metadata_documents: 8,
        };
        let message = error_message(run_with_jsr_limits(
            registry,
            limits,
            "import \"jsr:@scope/pkg@1\";\n",
        ));
        assert!(
            message.contains(&format!(
                "loading it would bring the JSR module graph to {} bytes, over the {budget}-byte limit",
                root.len() + dep.len()
            )),
            "{message}"
        );
        assert!(!message.contains("ENTRY_EXECUTED"), "{message}");
        assert!(!message.contains("DEP_EXECUTED"), "{message}");
    }

    // Integration test through the actual runtime: the fourth metadata document
    // crosses the limit, and the second package's module body never executes.
    #[test]
    fn runtime_enforces_the_metadata_document_limit_before_execution() {
        let registry = MockJsrRegistry::new()
            .package("scope", "aa", &[("1.0.0", false)])
            .version("scope", "aa", "1.0.0", &[(".", "./mod.js")])
            .file("scope", "aa", "1.0.0", "/mod.js", b"export const aa = 1;\n")
            .package("scope", "bb", &[("1.0.0", false)])
            .version("scope", "bb", "1.0.0", &[(".", "./mod.js")])
            .file(
                "scope",
                "bb",
                "1.0.0",
                "/mod.js",
                b"throw new Error(\"B_EXECUTED\");\n",
            );
        let limits = Limits {
            max_file_bytes: 1024,
            max_graph_files: 8,
            max_graph_bytes: 1024 * 1024,
            max_metadata_documents: 3,
        };
        let message = error_message(run_with_jsr_limits(
            registry,
            limits,
            r#"
const first = await import("jsr:@scope/aa@1");
const second = await import("jsr:@scope/aa@^1.0.0");
if (first !== second) throw new Error("identity failed");
await import("jsr:@scope/bb@1");
"#,
        ));
        assert!(
            message.contains("already requested 3 metadata documents, the limit"),
            "{message}"
        );
        assert!(message.contains("jsr:@scope/bb@1"), "{message}");
        assert!(!message.contains("B_EXECUTED"), "{message}");
    }

    // Unit test: a bounds check that is not reachable through a public API; the
    // Runtime-driven tests above prove the same limits across a real graph.
    #[test]
    fn enforces_per_file_and_graph_limits() {
        let limits = Limits {
            max_file_bytes: 2,
            max_graph_files: 8,
            max_graph_bytes: 1024,
            max_metadata_documents: 8,
        };
        let registry = two_file_registry();
        let loader = JsrModuleLoader::with_limits(Rc::new(registry), limits);
        let error = load_file(&loader, "https://jsr.io/@scope/pkg/1.0.0/a.js")
            .expect_err("a file over the per-file limit must fail");
        assert!(error.contains("over the 2-byte per-file limit"), "{error}");

        let limits = Limits {
            max_file_bytes: 1024,
            max_graph_files: 1,
            max_graph_bytes: 1024,
            max_metadata_documents: 8,
        };
        let loader = JsrModuleLoader::with_limits(Rc::new(two_file_registry()), limits);
        load_file(&loader, "https://jsr.io/@scope/pkg/1.0.0/a.js")
            .expect("the first file must load");
        let error = load_file(&loader, "https://jsr.io/@scope/pkg/1.0.0/b.js")
            .expect_err("a second file over the graph-file limit must fail");
        assert!(error.contains("already has 1 files, the limit"), "{error}");

        let limits = Limits {
            max_file_bytes: 1024,
            max_graph_files: 8,
            max_graph_bytes: 4,
            max_metadata_documents: 8,
        };
        let loader = JsrModuleLoader::with_limits(Rc::new(two_file_registry()), limits);
        load_file(&loader, "https://jsr.io/@scope/pkg/1.0.0/a.js")
            .expect("the first file must fit the byte budget");
        let error = load_file(&loader, "https://jsr.io/@scope/pkg/1.0.0/b.js")
            .expect_err("a second file over the byte limit must fail");
        assert!(error.contains("over the 4-byte limit"), "{error}");
    }

    fn two_file_registry() -> MockJsrRegistry {
        MockJsrRegistry::new()
            .package("scope", "pkg", &[("1.0.0", false)])
            .version(
                "scope",
                "pkg",
                "1.0.0",
                &[(".", "./a.js"), ("./b", "./b.js")],
            )
            .file("scope", "pkg", "1.0.0", "/a.js", b"abc")
            .file("scope", "pkg", "1.0.0", "/b.js", b"def")
    }

    fn load_file(loader: &JsrModuleLoader, url: &str) -> Result<(), String> {
        let url = deno_core::url::Url::parse(url).expect("fixture URL");
        // A real `resolve` marks the canonical URL as a pending JSR module
        // before `load` sees it. This unit test calls `load` directly to
        // exercise the bounds.
        loader.pend(&url);
        let options = || deno_core::ModuleLoadOptions {
            is_dynamic_import: false,
            is_synchronous: false,
            requested_module_type: deno_core::RequestedModuleType::None,
        };
        match loader.load(&url, None, options()) {
            deno_core::ModuleLoadResponse::Sync(Ok(_)) => Ok(()),
            deno_core::ModuleLoadResponse::Sync(Err(error)) => Err(error.to_string()),
            deno_core::ModuleLoadResponse::Async(_) => {
                panic!("the JSR test loader never loads asynchronously")
            }
        }
    }
}
