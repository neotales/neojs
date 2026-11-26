//! Remote (`https:`, `gh:`/`github:`, and `jsr:`) module resolution,
//! verification, loading, and diagnostic redaction for QuickJS.
//!
//! Compiled only when the `remote` Cargo feature is enabled.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;

use url::Url;
use vaeljs_module_policy::{ModuleFormat, module_format};
use vaeljs_remote::jsr::{
    Limits, PackageId, PackageMetadata, PackageVersion, REGISTRY_HOST, Registry, RegistryError,
    RemoteRegistry, Specifier, VersionMetadata, canonical_module_url, canonical_url_for,
    is_relative_reference, parse_module_url, registry_identities, registry_identities_if_registry,
    resolve_module,
};
use vaeljs_remote::{
    Redactions, RemoteClient, diagnostic_url, display_url, github_url, validate_url,
};

use crate::Error;
use crate::loader::display_specifier;

#[derive(Default)]
struct JsrGraphState {
    loaded: BTreeSet<String>,
    bytes: usize,
    metadata_documents: usize,
}

/// Prepared remote entry module ready to be evaluated by QuickJS.
pub(crate) struct PreparedRemoteEntry {
    pub(crate) module_name: String,
    pub(crate) source: Vec<u8>,
}

/// Shared remote & JSR loader state for one [`crate::Runtime`].
pub(crate) struct RemoteState {
    client: RemoteClient,
    jsr_registry: Rc<dyn Registry>,
    jsr_limits: Limits,
    jsr_packages: RefCell<BTreeMap<String, PackageMetadata>>,
    jsr_versions: RefCell<BTreeMap<String, VersionMetadata>>,
    jsr_graph: RefCell<JsrGraphState>,
    jsr_pending: RefCell<BTreeSet<String>>,
    jsr_loaded: RefCell<BTreeSet<String>>,
    ordinary_loaded: RefCell<BTreeSet<String>>,
    redirect_aliases: RefCell<BTreeMap<String, String>>,
    prefetched_https: RefCell<BTreeMap<String, Vec<u8>>>,
    loaded_https_urls: RefCell<BTreeSet<String>>,
}

impl RemoteState {
    pub(crate) fn new(client: RemoteClient) -> Self {
        let jsr_registry: Rc<dyn Registry> = Rc::new(RemoteRegistry::new(client.clone()));
        Self {
            client,
            jsr_registry,
            jsr_limits: Limits::default(),
            jsr_packages: RefCell::new(BTreeMap::new()),
            jsr_versions: RefCell::new(BTreeMap::new()),
            jsr_graph: RefCell::new(JsrGraphState::default()),
            jsr_pending: RefCell::new(BTreeSet::new()),
            jsr_loaded: RefCell::new(BTreeSet::new()),
            ordinary_loaded: RefCell::new(BTreeSet::new()),
            redirect_aliases: RefCell::new(BTreeMap::new()),
            prefetched_https: RefCell::new(BTreeMap::new()),
            loaded_https_urls: RefCell::new(BTreeSet::new()),
        }
    }

    pub(crate) fn client(&self) -> &RemoteClient {
        &self.client
    }

    pub(crate) fn redactions(&self) -> Redactions {
        self.client.redactions()
    }

    /// Fetches, verifies, and prepares a remote entry (`https://...`,
    /// `gh:...`, `github:...`, or `jsr:...`) before marking the runtime
    /// consumed.
    pub(crate) fn prepare_url_entry(&self, input: &str) -> Result<PreparedRemoteEntry, Error> {
        if input
            .get(..4)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("jsr:"))
        {
            return self.prepare_jsr_entry(input);
        }

        let requested = match github_url(input) {
            Some(Ok(url)) => url,
            Some(Err(reason)) => {
                return Err(Error::RemoteEntry {
                    url: "<invalid GitHub specifier>".to_string(),
                    reason,
                });
            }
            None => Url::parse(input).map_err(|error| Error::RemoteEntry {
                url: "<invalid URL>".to_string(),
                reason: format!("invalid URL: {error}"),
            })?,
        };
        validate_url(&requested).map_err(|reason| Error::RemoteEntry {
            url: display_url(&requested),
            reason: reason.to_string(),
        })?;

        let fetched = self
            .client
            .fetch_entry(&requested)
            .map_err(|reason| Error::RemoteEntry {
                url: display_url(&requested),
                reason: reason.to_string(),
            })?;
        let body_len = fetched.body.len();
        let module_charged = fetched.module_charged;

        let source = match prepare_https_source(&fetched.final_url, fetched.body) {
            Ok(bytes) => bytes,
            Err(PrepareSourceError::FormatOrUtf8(reason)) => {
                self.client
                    .refund_entry(&requested, body_len, module_charged);
                return Err(Error::RemoteEntry {
                    url: display_url(&requested),
                    reason,
                });
            }
            Err(PrepareSourceError::TypeScript(reason)) => {
                self.client
                    .refund_entry(&requested, body_len, module_charged);
                return Err(Error::RemoteTypeScript {
                    url: display_url(&requested),
                    reason,
                });
            }
        };

        if let Err(reason) = self.observe_final_url(&requested, &fetched.final_url) {
            self.client
                .refund_entry(&requested, body_len, module_charged);
            return Err(Error::RemoteEntry {
                url: display_url(&requested),
                reason,
            });
        }

        if fetched.final_url != requested {
            self.redirect_aliases
                .borrow_mut()
                .insert(requested.to_string(), fetched.final_url.to_string());
        }
        self.loaded_https_urls
            .borrow_mut()
            .insert(fetched.final_url.to_string());

        Ok(PreparedRemoteEntry {
            module_name: fetched.final_url.to_string(),
            source,
        })
    }

    fn prepare_jsr_entry(&self, input: &str) -> Result<PreparedRemoteEntry, Error> {
        let display = display_specifier(input);
        let canonical = self
            .resolve_jsr_url(input)
            .map_err(|reason| Error::RemoteEntry {
                url: display.clone(),
                reason,
            })?;
        let source = match self.load_jsr_prepared(&canonical) {
            Ok(bytes) => bytes,
            Err(PrepareSourceError::FormatOrUtf8(reason)) => {
                return Err(Error::RemoteEntry {
                    url: canonical.to_string(),
                    reason,
                });
            }
            Err(PrepareSourceError::TypeScript(reason)) => {
                return Err(Error::RemoteTypeScript {
                    url: canonical.to_string(),
                    reason,
                });
            }
        };
        self.mark_jsr_loaded(&canonical);
        Ok(PreparedRemoteEntry {
            module_name: canonical.to_string(),
            source,
        })
    }

    /// Resolves a `jsr:` specifier to its canonical `https://jsr.io/...` URL
    /// and marks it pending for verified loading.
    pub(crate) fn resolve_jsr(&self, input: &str) -> Result<String, String> {
        let url = self.resolve_jsr_url(input)?;
        self.pend(&url);
        Ok(url.to_string())
    }

    fn resolve_jsr_url(&self, input: &str) -> Result<Url, String> {
        let specifier = Specifier::parse(input).map_err(|error| error.to_string())?;
        let package = self.package_metadata(&specifier.package)?;
        let selected = package
            .select(&specifier.constraint)
            .map_err(|error| error.to_string())?;
        let package_version = PackageVersion {
            package: specifier.package.clone(),
            version: selected.version,
        };
        let version = self.version_metadata(&package_version)?;
        let resolved =
            resolve_module(&specifier, &package, &version).map_err(|error| error.to_string())?;
        let url = canonical_module_url(&resolved)?;
        if self.is_ordinary_loaded(&url) {
            return Err(format!(
                "`{url}` was already loaded as an ordinary HTTPS module; a `jsr:` import cannot add registry verification to that module. Use the `jsr:` import first, or a new Runtime."
            ));
        }
        Ok(url)
    }

    /// Resolves a relative import from a loaded JSR module, enforcing that it
    /// remains inside the exact package version directory.
    pub(crate) fn resolve_jsr_relative(
        &self,
        input: &str,
        referrer: &str,
    ) -> Result<String, String> {
        if let Some(character) = input
            .chars()
            .find(|character| matches!(character, '%' | '\\' | '?' | '#') || character.is_control())
        {
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
        let url = canonical_url_for(&joined_module)?;
        if self.is_ordinary_loaded(&url) {
            return Err(format!(
                "`{url}` was already loaded as an ordinary HTTPS module; a JSR graph import cannot add registry verification to that module"
            ));
        }
        self.pend(&url);
        Ok(url.to_string())
    }

    /// Handles an explicit `https://jsr.io/...` import from a verified JSR module.
    pub(crate) fn resolve_jsr_registry_https(&self, input: &str) -> Option<Result<String, String>> {
        let url = Url::parse(input).ok()?;
        if url.scheme() != "https" || url.host_str() != Some(REGISTRY_HOST) {
            return None;
        }
        Some((|| {
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
            Ok(canonical.to_string())
        })())
    }

    pub(crate) fn is_jsr_loaded_str(&self, referrer: &str) -> bool {
        Url::parse(referrer).is_ok_and(|url| self.is_jsr_loaded(&url))
    }

    pub(crate) fn is_jsr_candidate_str(&self, name: &str) -> bool {
        Url::parse(name).is_ok_and(|url| self.is_jsr_candidate(&url))
    }

    pub(crate) fn is_jsr_relative_reference(specifier: &str) -> bool {
        is_relative_reference(specifier)
    }

    /// Loads, verifies, and strips a pending/loaded JSR module.
    pub(crate) fn load_jsr(&self, name: &str) -> Result<Vec<u8>, String> {
        let url = Url::parse(name).map_err(|error| format!("invalid JSR module URL: {error}"))?;
        let source = self
            .load_jsr_prepared(&url)
            .map_err(|err| err.into_message())?;
        self.mark_jsr_loaded(&url);
        Ok(source)
    }

    /// Resolves an ordinary `https:` module URL, following redirects and
    /// caching its prepared source so QuickJS registers it under its final URL.
    pub(crate) fn resolve_https(&self, url: &Url) -> Result<String, String> {
        validate_url(url).map_err(|error| error.to_string())?;
        if self.is_jsr_loaded(url) {
            return Ok(url.to_string());
        }
        if let Some(aliased) = self.redirect_aliases.borrow().get(url.as_str()).cloned() {
            return Ok(aliased);
        }
        if self.loaded_https_urls.borrow().contains(url.as_str()) {
            return Ok(url.to_string());
        }

        let fetched = self.client.fetch(url).map_err(|error| error.to_string())?;
        let source = prepare_https_source(&fetched.final_url, fetched.body)
            .map_err(|err| err.into_message())?;
        self.observe_final_url(url, &fetched.final_url)?;

        if fetched.final_url != *url {
            self.redirect_aliases
                .borrow_mut()
                .insert(url.to_string(), fetched.final_url.to_string());
        }
        if !self
            .loaded_https_urls
            .borrow()
            .contains(fetched.final_url.as_str())
        {
            self.prefetched_https
                .borrow_mut()
                .insert(fetched.final_url.to_string(), source);
        }
        Ok(fetched.final_url.to_string())
    }

    /// Takes a prefetched ordinary HTTPS module source and marks its final URL loaded.
    pub(crate) fn take_prefetched_https(&self, name: &str) -> Option<Vec<u8>> {
        let bytes = self.prefetched_https.borrow_mut().remove(name)?;
        self.loaded_https_urls.borrow_mut().insert(name.to_owned());
        Some(bytes)
    }

    /// Loads an ordinary HTTPS module directly if it was not already prefetched.
    pub(crate) fn load_https_direct(&self, url: &Url) -> Result<Vec<u8>, String> {
        validate_url(url).map_err(|error| error.to_string())?;
        let fetched = self.client.fetch(url).map_err(|error| error.to_string())?;
        let source = prepare_https_source(&fetched.final_url, fetched.body)
            .map_err(|err| err.into_message())?;
        self.observe_final_url(url, &fetched.final_url)?;
        self.loaded_https_urls
            .borrow_mut()
            .insert(fetched.final_url.to_string());
        Ok(source)
    }

    fn observe_final_url(&self, requested: &Url, final_url: &Url) -> Result<(), String> {
        let requested_identities = registry_identities_if_registry(requested);
        let final_identities = registry_identities_if_registry(final_url);
        {
            let loaded = self.jsr_loaded.borrow();
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
        let mut ordinary = self.ordinary_loaded.borrow_mut();
        for identity in requested_identities.into_iter().chain(final_identities) {
            ordinary.insert(identity);
        }
        Ok(())
    }

    fn pend(&self, url: &Url) {
        if !self.is_jsr_loaded(url) {
            self.jsr_pending.borrow_mut().insert(url.to_string());
        }
    }

    fn is_jsr_candidate(&self, url: &Url) -> bool {
        let identities = registry_identities(url);
        let pending = self.jsr_pending.borrow();
        let loaded = self.jsr_loaded.borrow();
        identities
            .iter()
            .any(|identity| pending.contains(identity) || loaded.contains(identity))
    }

    fn is_jsr_loaded(&self, url: &Url) -> bool {
        let loaded = self.jsr_loaded.borrow();
        registry_identities(url)
            .iter()
            .any(|identity| loaded.contains(identity))
    }

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

    fn is_ordinary_loaded(&self, url: &Url) -> bool {
        let loaded = self.ordinary_loaded.borrow();
        registry_identities(url)
            .iter()
            .any(|identity| loaded.contains(identity))
    }

    fn package_metadata(&self, package: &PackageId) -> Result<PackageMetadata, String> {
        let key = package.to_string();
        if let Some(cached) = self.jsr_packages.borrow().get(&key) {
            return Ok(cached.clone());
        }
        self.charge_metadata()?;
        let json = self
            .jsr_registry
            .package_metadata(package)
            .map_err(|error| registry_failure("package metadata", &key, error))?;
        let metadata = PackageMetadata::parse(&json).map_err(|error| error.to_string())?;
        if metadata.package != *package {
            return Err(format!(
                "the registry returned metadata for `{}` when `{key}` was requested",
                metadata.package
            ));
        }
        self.jsr_packages.borrow_mut().insert(key, metadata.clone());
        Ok(metadata)
    }

    fn version_metadata(&self, package: &PackageVersion) -> Result<VersionMetadata, String> {
        let key = package.to_string();
        if let Some(cached) = self.jsr_versions.borrow().get(&key) {
            return Ok(cached.clone());
        }
        self.charge_metadata()?;
        let json = self
            .jsr_registry
            .version_metadata(package)
            .map_err(|error| registry_failure("version metadata", &key, error))?;
        let metadata =
            VersionMetadata::parse(&json, package.clone()).map_err(|error| error.to_string())?;
        self.jsr_versions.borrow_mut().insert(key, metadata.clone());
        Ok(metadata)
    }

    fn charge_metadata(&self) -> Result<(), String> {
        let mut graph = self.jsr_graph.borrow_mut();
        if graph.metadata_documents >= self.jsr_limits.max_metadata_documents {
            return Err(format!(
                "the JSR module graph already requested {} metadata documents, the limit",
                self.jsr_limits.max_metadata_documents
            ));
        }
        graph.metadata_documents += 1;
        Ok(())
    }

    fn reserve_jsr(&self, url: &str, declared: u64) -> Result<(), String> {
        let declared = usize::try_from(declared).map_err(|_| {
            format!("the declared size {declared} does not fit this platform's address space")
        })?;
        let mut graph = self.jsr_graph.borrow_mut();
        if graph.loaded.contains(url) {
            return Ok(());
        }
        if graph.loaded.len() >= self.jsr_limits.max_graph_files {
            return Err(format!(
                "the JSR module graph already has {} files, the limit",
                self.jsr_limits.max_graph_files
            ));
        }
        let total = graph
            .bytes
            .checked_add(declared)
            .ok_or_else(|| "the JSR module graph byte count overflowed".to_owned())?;
        if total > self.jsr_limits.max_graph_bytes {
            return Err(format!(
                "loading it would bring the JSR module graph to {total} bytes, over the {}-byte limit",
                self.jsr_limits.max_graph_bytes
            ));
        }
        graph.loaded.insert(url.to_owned());
        graph.bytes = total;
        Ok(())
    }

    fn load_jsr_prepared(&self, url: &Url) -> Result<Vec<u8>, PrepareSourceError> {
        if self.is_ordinary_loaded(url) {
            return Err(PrepareSourceError::FormatOrUtf8(
                "the URL is already loaded as an ordinary HTTPS module; a `jsr:` import cannot add registry verification to that module".to_owned(),
            ));
        }
        let Some(module) = parse_module_url(url).map_err(PrepareSourceError::FormatOrUtf8)? else {
            return Err(PrepareSourceError::FormatOrUtf8(
                "the resolved JSR module URL is not a canonical registry module".to_owned(),
            ));
        };
        let format = module_format(Path::new(module.path.as_str()))
            .map_err(PrepareSourceError::FormatOrUtf8)?;
        let metadata = self
            .version_metadata(&module.id)
            .map_err(PrepareSourceError::FormatOrUtf8)?;
        let declared = metadata
            .file_size(&module.path)
            .map_err(|error| PrepareSourceError::FormatOrUtf8(error.to_string()))?;
        if declared > self.jsr_limits.max_file_bytes as u64 {
            return Err(PrepareSourceError::FormatOrUtf8(format!(
                "the file is {declared} bytes, over the {}-byte per-file limit",
                self.jsr_limits.max_file_bytes
            )));
        }
        self.reserve_jsr(url.as_str(), declared)
            .map_err(PrepareSourceError::FormatOrUtf8)?;
        let bytes = self
            .jsr_registry
            .file(&module.id, &module.path)
            .map_err(|error| {
                PrepareSourceError::FormatOrUtf8(registry_failure("file", &module.path, error))
            })?;
        metadata
            .verify_file(&module.path, &bytes)
            .map_err(|error| PrepareSourceError::FormatOrUtf8(error.to_string()))?;

        prepare_module_bytes(url, format, bytes)
    }
}

enum PrepareSourceError {
    FormatOrUtf8(String),
    TypeScript(String),
}

impl PrepareSourceError {
    fn into_message(self) -> String {
        match self {
            Self::FormatOrUtf8(msg) | Self::TypeScript(msg) => msg,
        }
    }
}

fn prepare_https_source(final_url: &Url, body: Vec<u8>) -> Result<Vec<u8>, PrepareSourceError> {
    let format =
        module_format(Path::new(final_url.path())).map_err(PrepareSourceError::FormatOrUtf8)?;
    let diag = diagnostic_url(final_url);
    prepare_module_bytes(&diag, format, body)
}

fn prepare_module_bytes(
    diagnostic: &Url,
    format: ModuleFormat,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, PrepareSourceError> {
    match format {
        ModuleFormat::JavaScript => Ok(bytes),
        ModuleFormat::TypeScript => {
            #[cfg(not(feature = "typescript"))]
            {
                let _ = (diagnostic, bytes);
                Err(PrepareSourceError::FormatOrUtf8(
                    "TypeScript is not enabled in this build; enable the `typescript` feature or use a .js or .mjs file"
                        .to_owned(),
                ))
            }
            #[cfg(feature = "typescript")]
            {
                let text = String::from_utf8(bytes).map_err(|_| {
                    PrepareSourceError::FormatOrUtf8(
                        "the TypeScript file is not valid UTF-8".to_owned(),
                    )
                })?;
                vaeljs_module_policy::typescript::strip(diagnostic, text)
                    .map(String::into_bytes)
                    .map_err(PrepareSourceError::TypeScript)
            }
        }
    }
}

fn registry_failure(action: &str, subject: &str, error: RegistryError) -> String {
    match error {
        RegistryError::NotFound => format!("the registry has no {action} for `{subject}`"),
        RegistryError::Redirect(location) => format!(
            "the registry transport redirected the {action} for `{subject}` to `{location}`; redirects must be followed by the transport, not the loader"
        ),
        RegistryError::Unavailable(reason) => {
            format!("the registry could not provide the {action} for `{subject}`: {reason}")
        }
        other => format!("the registry could not provide the {action} for `{subject}`: {other}"),
    }
}

/// Redacts query/userinfo from a module identity string for diagnostic display.
pub(crate) fn display_module_identity(name: &str) -> String {
    match Url::parse(name) {
        Ok(url) if matches!(url.scheme(), "https" | "http") => display_url(&url),
        _ => name.to_owned(),
    }
}

/// Redacts known remote URLs from QuickJS stack traces and generated
/// syntax/link/loader diagnostics while preserving user-thrown `Error`
/// messages.
pub(crate) fn redact_known_urls(text: &str, redactions: &Redactions) -> String {
    let urls = redactions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if urls.is_empty() {
        return text.to_owned();
    }
    let mut sorted = urls.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|url| std::cmp::Reverse(url.len()));
    let mut redacted = text.to_owned();
    for url in sorted {
        if redacted.contains(url)
            && let Ok(parsed) = Url::parse(url)
        {
            redacted = redacted.replace(url, diagnostic_url(&parsed).as_ref());
        }
    }
    redacted
}

/// Returns whether a QuickJS `Error` message line is an engine-generated
/// syntax, link, or loader diagnostic rather than a user-thrown message.
pub(crate) fn is_generated_diagnostic(message: &str) -> bool {
    message.starts_with("Error resolving module ")
        || message.starts_with("Error loading module ")
        || (message.starts_with("Could not find export ") && message.contains(" in module '"))
        || (message.starts_with("SyntaxError: ") && message.contains(" at "))
}
