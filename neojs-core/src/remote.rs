//! HTTPS module transport and the user-profile module cache.
//!
//! Remote loading is opt-in per [`crate::Runtime`]; a default runtime never
//! performs a network request. When enabled, only `https:` URLs are accepted.
//! `http:`, `data:`, `blob:`, and `file:` are not remote transports, and a
//! remote module cannot import a `file:` module.
//!
//! The client is deliberately small:
//!
//! - TLS is provided by the reviewed `rustls` backend through `ureq`, with the
//!   Mozilla root store bundled by `webpki-roots`. No native or user root store
//!   is consulted.
//! - No cookies, authorization header, referrer, proxy from the environment, or
//!   client certificate is sent. A URL with a username or password is rejected
//!   before a request is made, so a credential cannot be forwarded by accident.
//! - Redirects are followed by neojs, not by the HTTP client, so every hop is
//!   checked. The target must be `https:`, credential-free, and within the
//!   configured redirect limit. Cross-origin HTTPS redirects are allowed
//!   because no credential, cookie, or referrer is sent.
//! - The body is streamed with a byte limit and the whole fetch, including every
//!   redirect and the body, is bounded by one deadline. A declared
//!   `Content-Length` above the limit fails before the body is read.
//!
//! Downloaded modules are cached by default in one user-profile package
//! directory, `$XDG_CACHE_HOME/neojs/packages` or `$HOME/.cache/neojs/packages`
//! (`%USERPROFILE%\.cache\neojs\packages` on Windows). The cache
//! key is the SHA-256 of the canonical URL including its query, so a URL query
//! never appears as a file name and two URLs that differ by query are cached
//! separately. A body is written to a temporary file in the cache directory and
//! atomically renamed into place, so concurrent processes cannot publish a
//! partial entry.
//!
//! Cached responses are reused while `Cache-Control: max-age` says they are
//! fresh. Freshness includes the upstream `Age` and `Date` values, so a
//! response that was already old when it arrived is stale immediately. A stale
//! entry is revalidated with `If-None-Match` and `If-Modified-Since` when the
//! server supplied validators; a `304` reuses the cached body and recomputes
//! its age from the `304` headers. An entry with no freshness lifetime is
//! always revalidated. `Cache-Control: no-store` is not stored, and `no-cache`
//! forces revalidation.
//!
//! Each entry records the body length and SHA-256 digest; a mismatch or a
//! truncation is a cache miss, not an executable body. Publications are
//! serialized by a per-key lock file, and every entry carries an exact
//! generation. A `304` may only publish when that generation is still current,
//! so an older revalidation cannot replace a newer `200` or undo its
//! `no-store`. A stale lock left by a crash can delay a cache write until the
//! lock times out; it never blocks loading, which falls back to the network.
//! This detects corruption, torn writes, and lost-update races, not an adversary
//! who can write both the metadata and the body in the user's cache directory.
//! There is no signature or content-addressed store, so the cache does not pin a
//! version or provide integrity beyond transport TLS and those consistency
//! checks. A cache read or write failure is ignored: the module is fetched and
//! the program continues. Openness of the cache directory is the user's
//! responsibility.
//!
//! One runtime's remote graph is bounded by a per-module body limit, an
//! aggregate byte limit, and a maximum number of remote modules. A module and
//! its bytes are admitted before the network or cache is consulted, a body is
//! read only up to the remaining aggregate allowance, and a rejected module is
//! neither fetched nor published to the cache. The deadline is per module, not
//! a wall-clock bound on the whole graph: JavaScript can keep running and
//! dynamically import more modules, each with its own deadline, until the
//! aggregate module or byte limit is reached. Module execution and compilation
//! are trusted; these are resource bounds, not a sandbox or a bound on CPU or
//! V8 heap used by a program.
//!
//! Generated V8 errors are redacted as they leave the runtime: known remote
//! module URLs are removed from stack-frame locations and from the generated
//! ` at <URL>:line:column` suffix, without changing the module identity or
//! `import.meta.url`, which remain the final URL. Script-supplied exception
//! messages and custom stacks are preserved, so code that puts a URL (or any
//! other value) in its own error still exposes it. Module specifiers are not
//! secret storage.
//!
//! This is not SSRF protection. There is no host allowlist, no private-address
//! rejection, and no DNS pinning: neojs fetches whatever HTTPS URL the program
//! names, using the process's network authority. Remote code has the same
//! authority as local code once it runs; it is not a sandbox. The cache is an
//! internal part of module loading; neojs exposes no `fetch`-style API and no
//! way for JavaScript to read the cache.

use std::collections::BTreeSet;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use deno_core::error::CoreError;
use deno_core::error::CoreErrorKind;
use deno_core::error::JsError;
use deno_core::url::Url;
use deno_error::JsErrorBox;
use sha2::Digest;
use sha2::Sha256;

/// Default maximum size of one remote module body.
const DEFAULT_MAX_MODULE_BYTES: usize = 16 * 1024 * 1024;
/// Default maximum number of redirects followed for one module.
const DEFAULT_MAX_REDIRECTS: u32 = 5;
/// Default deadline for one module fetch, including redirects and the body.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default maximum number of distinct modules fetched by one runtime.
const DEFAULT_MAX_REMOTE_MODULES: usize = 256;
/// Default maximum aggregate remote body bytes loaded by one runtime.
const DEFAULT_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// The first line of a cache entry.
const CACHE_MAGIC: &str = "neojs-remote-cache 3";
/// Largest metadata block accepted from a cache entry.
const MAX_META_BYTES: usize = 64 * 1024;

/// Limits and cache settings for remote module fetches.
///
/// Create one with [`RemoteOptions::new`] and adjust it with the `with_*`
/// methods. The defaults are a 30-second per-module deadline, a 16 MiB body
/// limit, a 64 MiB aggregate byte limit, 256 remote modules, five redirects, and
/// the user-profile package cache. These are resource bounds, not permissions:
/// enabling remote loading is the permission decision. There is no aggregate
/// wall-clock deadline; each module fetch has its own.
#[derive(Clone, Debug)]
pub struct RemoteOptions {
    timeout: Duration,
    max_module_bytes: usize,
    max_total_bytes: usize,
    max_remote_modules: usize,
    max_redirects: u32,
    cache_dir: Option<PathBuf>,
}

impl Default for RemoteOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_module_bytes: DEFAULT_MAX_MODULE_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_remote_modules: DEFAULT_MAX_REMOTE_MODULES,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            cache_dir: default_cache_dir(),
        }
    }
}

impl RemoteOptions {
    /// Returns the default limits and the user-profile cache directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the deadline for one module fetch.
    ///
    /// The deadline covers every redirect and reading one module's body. It is
    /// not a deadline for the whole module graph: code can keep running and
    /// dynamically import more modules, each with its own deadline, until the
    /// aggregate module or byte limit is reached. A zero duration makes every
    /// fetch fail immediately. DNS resolution may exceed it because the resolver
    /// API cannot be interrupted.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the largest accepted module body, in bytes.
    ///
    /// A body exactly this size is accepted; one byte more is rejected. A zero
    /// limit rejects every non-empty module.
    pub fn with_max_module_bytes(mut self, max_module_bytes: usize) -> Self {
        self.max_module_bytes = max_module_bytes;
        self
    }

    /// Sets the aggregate remote body limit for one runtime, in bytes.
    ///
    /// Every distinct remote module body loaded by the runtime counts once,
    /// including a body served from the cache. The entry module counts. A zero
    /// limit rejects the first non-empty module.
    pub fn with_max_total_bytes(mut self, max_total_bytes: usize) -> Self {
        self.max_total_bytes = max_total_bytes;
        self
    }

    /// Sets how many distinct remote modules one runtime may load.
    ///
    /// The entry module counts. A zero limit rejects the remote entry before
    /// V8 loads it.
    pub fn with_max_remote_modules(mut self, max_remote_modules: usize) -> Self {
        self.max_remote_modules = max_remote_modules;
        self
    }

    /// Sets how many redirects one module fetch may follow.
    ///
    /// Zero refuses every redirect. The limit counts redirect *responses*, so a
    /// value of five permits a chain of five and rejects a sixth.
    pub fn with_max_redirects(mut self, max_redirects: u32) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    /// Uses `dir` for the module cache.
    ///
    /// The directory is created on the first successful download. Point it at a
    /// private location. Relative paths are used as given and depend on the
    /// process working directory. An empty path is not special.
    pub fn with_cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(dir.into());
        self
    }

    /// Disables the module cache.
    ///
    /// Every remote fetch then uses the network, and no cache directory is read
    /// or written.
    pub fn without_cache(mut self) -> Self {
        self.cache_dir = None;
        self
    }

    /// Returns the deadline for one module fetch.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Returns the largest accepted module body, in bytes.
    pub fn max_module_bytes(&self) -> usize {
        self.max_module_bytes
    }

    /// Returns the aggregate remote body limit, in bytes.
    pub fn max_total_bytes(&self) -> usize {
        self.max_total_bytes
    }

    /// Returns how many distinct remote modules one runtime may load.
    pub fn max_remote_modules(&self) -> usize {
        self.max_remote_modules
    }

    /// Returns how many redirects one module fetch may follow.
    pub fn max_redirects(&self) -> u32 {
        self.max_redirects
    }

    /// Returns the cache directory, or `None` when caching is disabled.
    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }
}

/// Returns the default cache directory for this user profile.
///
/// It honors `XDG_CACHE_HOME` when set to an absolute path, giving
/// `$XDG_CACHE_HOME/neojs/packages`. Otherwise it is
/// `$HOME/.cache/neojs/packages`, or `%USERPROFILE%\.cache\neojs\packages` when
/// `HOME` is absent. Without any of these variables there is no default cache
/// directory and remote loading proceeds without a cache.
fn default_cache_dir() -> Option<PathBuf> {
    cache_dir_from(
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from),
    )
}

/// Resolves the cache directory from the two environment values.
///
/// A relative `XDG_CACHE_HOME` is ignored, as the XDG base-directory
/// specification requires. This is separate from [`default_cache_dir`] so the
/// rule can be tested without changing the process environment.
fn cache_dir_from(cache_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(cache_home) = cache_home
        && cache_home.is_absolute()
    {
        return Some(cache_home.join("neojs").join("packages"));
    }
    home.map(|home| home.join(".cache").join("neojs").join("packages"))
}

/// A fetched module body and the URL it was finally read from.
#[derive(Debug)]
pub(crate) struct FetchedModule {
    /// The response body bytes.
    pub(crate) body: Vec<u8>,
    /// The URL of the final response, after any redirects and without a
    /// fragment. It differs from the requested URL when a redirect was
    /// followed.
    pub(crate) final_url: Url,
}

/// One HTTP response, with an unread body.
pub(crate) struct RemoteResponse {
    /// The HTTP status code.
    pub(crate) status: u16,
    /// The `Location` header, when present.
    pub(crate) location: Option<String>,
    /// The declared `Content-Length`, when present and parseable.
    pub(crate) declared_length: Option<u64>,
    /// The `ETag` header, when present.
    pub(crate) etag: Option<String>,
    /// The `Last-Modified` header, when present.
    pub(crate) last_modified: Option<String>,
    /// The `Cache-Control` header, when present.
    pub(crate) cache_control: Option<String>,
    /// The `Date` header, when present.
    pub(crate) date: Option<String>,
    /// The `Age` header, when present.
    pub(crate) age: Option<String>,
    /// The response body, read lazily so the caller can bound it.
    pub(crate) body: Box<dyn Read + Send>,
}

/// One request, including the validators for a conditional GET.
pub(crate) struct RemoteRequest<'a> {
    /// The URL to request, without a fragment.
    pub(crate) url: &'a Url,
    /// The time left before the fetch deadline.
    pub(crate) timeout: Duration,
    /// An `If-None-Match` value, when a stale cache entry has an ETag.
    pub(crate) if_none_match: Option<&'a str>,
    /// An `If-Modified-Since` value, when a stale cache entry has one.
    pub(crate) if_modified_since: Option<&'a str>,
}

/// Performs one request without following redirects.
pub(crate) trait RemoteTransport: Send + Sync {
    /// Requests `request.url`, returning headers and an unread body.
    ///
    /// An HTTP error status is returned as a response, not as an error, so the
    /// caller can report it uniformly.
    fn get(&self, request: &RemoteRequest<'_>) -> Result<RemoteResponse, String>;
}

/// The aggregate limits shared by all clones of one runtime's remote client.
#[derive(Default)]
struct RemoteBudget {
    modules: AtomicUsize,
    bytes: AtomicUsize,
}

impl RemoteBudget {
    /// Admits one module before any network or cache work.
    ///
    /// The module slot is held by the returned [`Admission`] and is released if
    /// the fetch fails or the admission is dropped without a commit. Bytes are
    /// only charged by `commit`, after the body is known.
    fn begin<'a>(&'a self, options: &'a RemoteOptions) -> Result<Admission<'a>, String> {
        let modules = self.modules.fetch_add(1, Ordering::SeqCst) + 1;
        if modules > options.max_remote_modules {
            self.modules.fetch_sub(1, Ordering::SeqCst);
            return Err(format!(
                "the remote module graph exceeds the {} module limit",
                options.max_remote_modules
            ));
        }
        Ok(Admission {
            budget: self,
            options,
            committed: false,
        })
    }
}

/// A module admitted into the graph but not yet charged for its body.
struct Admission<'a> {
    budget: &'a RemoteBudget,
    options: &'a RemoteOptions,
    committed: bool,
}

impl Admission<'_> {
    /// Returns how many more body bytes the graph may accept.
    fn remaining_bytes(&self) -> usize {
        self.options
            .max_total_bytes
            .saturating_sub(self.budget.bytes.load(Ordering::SeqCst))
    }

    /// Charges the body bytes. On failure the admission is rolled back and the
    /// caller must not publish the cache entry.
    fn commit(&mut self, bytes: usize) -> Result<(), String> {
        let total = self
            .budget
            .bytes
            .fetch_add(bytes, Ordering::SeqCst)
            .saturating_add(bytes);
        if total > self.options.max_total_bytes {
            self.budget.bytes.fetch_sub(bytes, Ordering::SeqCst);
            return Err(format!(
                "the remote module graph exceeds the {} byte total limit",
                self.options.max_total_bytes
            ));
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for Admission<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.budget.modules.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Remote module URLs that may appear in generated V8 diagnostics.
pub(crate) type Redactions = Arc<Mutex<BTreeSet<String>>>;

/// A configured remote module client.
#[derive(Clone)]
pub(crate) struct RemoteClient {
    options: RemoteOptions,
    transport: Arc<dyn RemoteTransport>,
    cache: Option<Arc<Cache>>,
    budget: Arc<RemoteBudget>,
    redactions: Redactions,
}

impl RemoteClient {
    /// Creates a client using the rustls-backed HTTPS transport.
    pub(crate) fn new(options: RemoteOptions) -> Self {
        let cache = options
            .cache_dir
            .as_ref()
            .map(|dir| Arc::new(Cache::new(dir.clone())));
        Self {
            options,
            transport: Arc::new(UreqTransport::new()),
            cache,
            budget: Arc::new(RemoteBudget::default()),
            redactions: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Creates a client over an injected transport.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_transport(
        options: RemoteOptions,
        transport: Arc<dyn RemoteTransport>,
    ) -> Self {
        let cache = options
            .cache_dir
            .as_ref()
            .map(|dir| Arc::new(Cache::new(dir.clone())));
        Self {
            options,
            transport,
            cache,
            budget: Arc::new(RemoteBudget::default()),
            redactions: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Refunds one committed module after a preflight failure.
    ///
    /// The runtime is still unused, so an entry that was fetched but rejected
    /// for its format, UTF-8, or TypeScript must not spend a module slot or
    /// bytes from a later retry.
    pub(crate) fn refund(&self, bytes: usize) {
        self.budget.modules.fetch_sub(1, Ordering::SeqCst);
        self.budget.bytes.fetch_sub(bytes, Ordering::SeqCst);
    }

    /// Returns the shared set of module URLs to redact from V8 diagnostics.
    pub(crate) fn redactions(&self) -> Redactions {
        Arc::clone(&self.redactions)
    }

    /// Records a URL whose query must not appear in a generated diagnostic.
    fn record_url(&self, url: &Url) {
        self.redactions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(url.to_string());
    }

    /// Fetches a module body, following checked redirects and using the cache.
    ///
    /// The URL is validated first. The returned body and final URL are the
    /// final response; the final URL is what the module map aliases the
    /// requested URL to.
    pub(crate) fn fetch(&self, requested: &Url) -> Result<FetchedModule, String> {
        validate_url(requested)?;
        self.record_url(requested);
        if self.options.timeout.is_zero() {
            return Err("the remote timeout is zero".to_string());
        }
        let deadline = Instant::now()
            .checked_add(self.options.timeout)
            .ok_or_else(|| "the remote timeout is too large".to_string())?;
        let mut admission = self.budget.begin(&self.options)?;

        let mut current = request_url(requested);
        let mut redirects = 0;
        loop {
            self.record_url(&current);
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("timed out fetching {}", display_url(&current)));
            }

            let now = unix_now();
            let cached = self
                .cache
                .as_ref()
                .and_then(|cache| cache.load(&current, self.options.max_module_bytes));
            if let Some(cached) = &cached
                && cached.is_fresh(now)
            {
                admission.commit(cached.body.len())?;
                return Ok(FetchedModule {
                    body: cached.body.clone(),
                    final_url: current,
                });
            }
            let etag = cached.as_ref().and_then(|entry| entry.etag.as_deref());
            let last_modified = cached
                .as_ref()
                .and_then(|entry| entry.last_modified.as_deref());
            let request_started = Instant::now();
            let response = self
                .transport
                .get(&RemoteRequest {
                    url: &current,
                    timeout: remaining,
                    if_none_match: etag,
                    if_modified_since: last_modified,
                })
                .map_err(|reason| {
                    format!("request for {} failed: {reason}", display_url(&current))
                })?;
            let received_at = unix_now();
            let response_delay = duration_secs_ceil(request_started.elapsed());

            if response.status == 304 {
                let Some(mut cached) = cached else {
                    return Err(format!(
                        "HTTP 304 from {} without a cached module",
                        display_url(&current)
                    ));
                };
                let directives = CacheDirectives::parse(response.cache_control.as_deref());
                let previous_age = cached
                    .initial_age
                    .saturating_add(received_at.saturating_sub(cached.stored_at));
                let upstream_age = response_age(
                    response.date.as_deref(),
                    response.age.as_deref(),
                    received_at,
                    response_delay,
                );
                // A successful revalidation replaces the age with the `304`
                // headers. When the `304` omits both `Date` and `Age`, keeping
                // the previous age is the conservative choice.
                cached.initial_age = if response.date.is_some() || response.age.is_some() {
                    upstream_age
                } else {
                    previous_age
                };
                cached.stored_at = received_at;
                if directives.no_cache {
                    cached.max_age = Some(0);
                } else if let Some(max_age) = directives.max_age {
                    cached.max_age = Some(max_age);
                }
                if response.etag.is_some() {
                    cached.etag = response.etag;
                }
                if response.last_modified.is_some() {
                    cached.last_modified = response.last_modified;
                }
                admission.commit(cached.body.len())?;
                if let Some(cache) = &self.cache {
                    if directives.no_store {
                        let _ = cache.remove_if_generation(&current, Some(&cached.generation));
                    } else {
                        let _ =
                            cache.store_if_generation(&current, &cached, Some(&cached.generation));
                    }
                }
                return Ok(FetchedModule {
                    body: cached.body,
                    final_url: current,
                });
            }

            if (300..=399).contains(&response.status) {
                let Some(location) = response.location.as_deref() else {
                    return Err(format!(
                        "HTTP {} from {} has no Location header",
                        response.status,
                        display_url(&current)
                    ));
                };
                if redirects >= self.options.max_redirects {
                    return Err(format!(
                        "too many redirects fetching {}; the limit is {}",
                        display_url(requested),
                        self.options.max_redirects
                    ));
                }
                let target = current.join(location).map_err(|error| {
                    format!(
                        "cannot resolve the redirect location from {}: {error}",
                        display_url(&current)
                    )
                })?;
                validate_redirect(&target)?;
                current = request_url(&target);
                redirects += 1;
                continue;
            }

            if !(200..=299).contains(&response.status) {
                return Err(format!(
                    "HTTP {} from {}",
                    response.status,
                    display_url(&current)
                ));
            }
            let remaining_bytes = admission.remaining_bytes();
            let limit = self.options.max_module_bytes.min(remaining_bytes);
            let budget_label = if remaining_bytes < self.options.max_module_bytes {
                "remaining graph budget"
            } else {
                "limit"
            };
            if let Some(length) = response.declared_length
                && length > limit as u64
            {
                return Err(format!(
                    "module body from {} declares {length} bytes, above the {limit} byte {budget_label}",
                    display_url(&current)
                ));
            }
            let body = read_bounded(response.body, limit, deadline, &current, budget_label)?;
            admission.commit(body.len())?;
            let directives = CacheDirectives::parse(response.cache_control.as_deref());
            let expected_generation = cached.as_ref().map(|entry| entry.generation.clone());
            if let Some(cache) = &self.cache {
                if directives.no_store {
                    if let Some(expected) = expected_generation.as_deref() {
                        let _ = cache.remove_if_generation(&current, Some(expected));
                    }
                } else {
                    let entry = CachedModule {
                        body: body.clone(),
                        stored_at: received_at,
                        initial_age: response_age(
                            response.date.as_deref(),
                            response.age.as_deref(),
                            received_at,
                            response_delay,
                        ),
                        max_age: if directives.no_cache {
                            Some(0)
                        } else {
                            directives.max_age
                        },
                        etag: response.etag,
                        last_modified: response.last_modified,
                        generation: new_generation(),
                    };
                    if let Some(expected) = expected_generation.as_deref() {
                        let _ = cache.store_if_generation(&current, &entry, Some(expected));
                    } else {
                        let _ = cache.store(&current, &entry);
                    }
                }
            }
            return Ok(FetchedModule {
                body,
                final_url: current,
            });
        }
    }
}

/// Rejects a URL that is not a credential-free HTTPS URL.
pub(crate) fn validate_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err(format!(
            "only https: module URLs are supported; `{}` is not",
            url.scheme()
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("module URLs with a username or password are not supported".to_string());
    }
    if url.host_str().is_none() {
        return Err("the module URL has no host".to_string());
    }
    Ok(())
}

/// Rejects a redirect target that is not a credential-free HTTPS URL.
///
/// The message names the source URL but not the target, so a credential in a
/// malformed `Location` header is not echoed.
fn validate_redirect(target: &Url) -> Result<(), String> {
    if target.scheme() != "https" {
        return Err(format!(
            "refusing a redirect to a `{}` URL; remote modules require https:",
            target.scheme()
        ));
    }
    if !target.username().is_empty() || target.password().is_some() {
        return Err("refusing a redirect URL with a username or password".to_string());
    }
    if target.host_str().is_none() {
        return Err("refusing a redirect URL with no host".to_string());
    }
    Ok(())
}

/// Returns the URL to request, without a fragment.
///
/// A fragment is part of module identity but is never sent on the wire.
fn request_url(url: &Url) -> Url {
    let mut request = url.clone();
    request.set_fragment(None);
    request
}

/// Formats a URL for an error without credentials, a query, or a fragment.
///
/// A query can contain a signed token, so it is not written to an error or a
/// log line even though it is part of the cache key.
pub(crate) fn display_url(url: &Url) -> String {
    diagnostic_url(url).to_string()
}

/// Returns a URL safe to pass to a diagnostic producer.
///
/// Credentials, the query, and the fragment are removed, so a type-stripping
/// error cannot echo a signed query even though it names the module URL.
pub(crate) fn diagnostic_url(url: &Url) -> Url {
    let mut clean = url.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.set_query(None);
    clean.set_fragment(None);
    clean
}

/// Redacts known remote module URLs from a V8 diagnostic without touching the
/// script's exception value.
///
/// Only URLs recorded by this client are replaced, and only in generated
/// locations: stack-frame file names, frame lines after the first, and the
/// generated ` at <URL>:line:column` suffix. `exception_message` and
/// `source_line` are otherwise preserved, and a custom stack supplied by the
/// script is kept as-is.
pub(crate) fn redact_core_error(error: CoreError, redactions: &Redactions) -> CoreError {
    let urls = redactions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if urls.is_empty() {
        return error;
    }
    let CoreError(kind) = error;
    let kind = match *kind {
        CoreErrorKind::Js(js) => CoreErrorKind::Js(Box::new(redact_js_error(*js, &urls))),
        other => {
            let original = CoreError(Box::new(other));
            let text = original.to_string();
            let redacted = redact_generated_mentions(&text, &urls);
            if redacted == text {
                return original;
            }
            CoreErrorKind::JsBox(JsErrorBox::generic(redacted))
        }
    };
    CoreError(Box::new(kind))
}

fn redact_js_error(mut error: JsError, urls: &BTreeSet<String>) -> JsError {
    for frame in &mut error.frames {
        if let Some(file_name) = frame.file_name.take() {
            frame.file_name = Some(redact_known_urls(&file_name, urls));
        }
        if let Some(eval_origin) = frame.eval_origin.take() {
            frame.eval_origin = Some(redact_known_urls(&eval_origin, urls));
        }
    }
    if !error.stack_is_custom {
        if let Some(stack) = error.stack.take() {
            error.stack = Some(redact_stack(&stack, urls));
        }
        // V8 puts generated locations in syntax, link, and resolution errors.
        // Only known module URLs in those generated positions are replaced, so
        // an ordinary thrown value that merely contains a URL is preserved.
        error.exception_message = redact_generated_mentions(&error.exception_message, urls);
        error.message = error
            .message
            .take()
            .map(|message| redact_generated_mentions(&message, urls));
    }
    error.cause = error
        .cause
        .take()
        .map(|cause| Box::new(redact_js_error(*cause, urls)));
    error.aggregated = error.aggregated.take().map(|aggregated| {
        aggregated
            .into_iter()
            .map(|error| redact_js_error(error, urls))
            .collect()
    });
    error
}

/// Redacts generated frame lines but leaves the first stack line (the
/// exception value) untouched.
fn redact_stack(stack: &str, urls: &BTreeSet<String>) -> String {
    let mut lines = stack.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut redacted = String::from(first);
    for line in lines {
        redacted.push('\n');
        redacted.push_str(&redact_known_urls(line, urls));
    }
    redacted
}

/// Replaces known module URLs in generated diagnostic positions.
///
/// It handles ` at <URL>:line:column` and a URL quoted as a module specifier in
/// a link or resolution error. Other mentions, including an exception value that
/// carries an unrelated URL, are preserved.
fn redact_generated_mentions(message: &str, urls: &BTreeSet<String>) -> String {
    let mut redacted = message.to_string();
    for url in urls {
        let replacement = redact_url_string(url);
        redacted = redacted.replace(&format!(" at {url}:"), &format!(" at {replacement}:"));
        redacted = redacted.replace(&format!("'{url}'"), &format!("'{replacement}'"));
        redacted = redacted.replace(&format!("\"{url}\""), &format!("\"{replacement}\""));
    }
    redact_quoted_specifier_queries(&redacted)
}

/// Removes the query from a quoted relative module specifier in a generated
/// message, such as `'./module.js?token=secret'` in a link error. Other quoted
/// text is preserved.
fn redact_quoted_specifier_queries(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let quote = bytes[index];
        if (quote == b'\'' || quote == b'"')
            && let Some(offset) = bytes[index + 1..].iter().position(|byte| *byte == quote)
        {
            let end = index + 1 + offset;
            let inner = &text[index + 1..end];
            if is_relative_specifier_with_query(inner) {
                output.push(quote);
                output
                    .extend_from_slice(inner.split(['?', '#']).next().unwrap_or(inner).as_bytes());
                output.push(quote);
                index = end + 1;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| text.to_string())
}

fn is_relative_specifier_with_query(specifier: &str) -> bool {
    let looks_relative = specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
        || specifier.starts_with("https://")
        || specifier.starts_with("http://");
    looks_relative && specifier.contains('?') && !specifier.chars().any(char::is_whitespace)
}

/// Replaces exact known module URLs, longest first so one URL cannot corrupt a
/// longer one that shares its prefix.
fn redact_known_urls(text: &str, urls: &BTreeSet<String>) -> String {
    let mut urls = urls.iter().collect::<Vec<_>>();
    urls.sort_by_key(|url| std::cmp::Reverse(url.len()));
    let mut redacted = text.to_string();
    for url in urls {
        if redacted.contains(url) {
            redacted = redacted.replace(url, &redact_url_string(url));
        }
    }
    redacted
}

fn redact_url_string(url: &str) -> String {
    match Url::parse(url) {
        Ok(url) => diagnostic_url(&url).to_string(),
        Err(_) => url.to_string(),
    }
}

/// Reads at most `limit` bytes, checking the deadline between reads.
fn read_bounded(
    mut body: Box<dyn Read + Send>,
    limit: usize,
    deadline: Instant,
    url: &Url,
    budget_label: &str,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out reading the module body from {}",
                display_url(url)
            ));
        }
        let read = body.read(&mut buffer).map_err(|error| {
            format!(
                "cannot read the module body from {}: {error}",
                display_url(url)
            )
        })?;
        if read == 0 {
            return Ok(bytes);
        }
        if bytes.len() + read > limit {
            return Err(format!(
                "module body from {} is larger than the {limit} byte {budget_label}",
                display_url(url)
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
}

/// Cache directives neojs understands.
#[derive(Clone, Copy, Debug, Default)]
struct CacheDirectives {
    no_store: bool,
    no_cache: bool,
    max_age: Option<u64>,
}

impl CacheDirectives {
    /// Parses the directives neojs implements; unknown directives are ignored.
    fn parse(header: Option<&str>) -> Self {
        let mut directives = Self::default();
        let Some(header) = header else {
            return directives;
        };
        for directive in header.split(',') {
            let directive = directive.trim();
            let (name, value) = directive.split_once('=').unwrap_or((directive, ""));
            let name = name.trim();
            let value = value.trim();
            if name.eq_ignore_ascii_case("no-store") {
                directives.no_store = true;
            } else if name.eq_ignore_ascii_case("no-cache") {
                // A qualified `no-cache="field"` still requires revalidation.
                directives.no_cache = true;
            } else if name.eq_ignore_ascii_case("max-age") {
                let value = value.trim_matches('"');
                if let Ok(seconds) = value.parse() {
                    directives.max_age = Some(seconds);
                }
            }
        }
        directives
    }
}

/// A cached module body and the metadata needed to reuse or revalidate it.
#[derive(Clone)]
struct CachedModule {
    body: Vec<u8>,
    stored_at: u64,
    /// Upstream age at the time the response arrived, from `Date` and `Age`.
    initial_age: u64,
    /// `Cache-Control: max-age`, or `Some(0)` for `no-cache`.
    max_age: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
    /// Exact publication generation. A `304` may only replace this generation.
    generation: String,
}

impl CachedModule {
    /// Returns whether the entry may be used without a request.
    ///
    /// The age is the upstream age at storage plus time resident locally. A
    /// server `Age` or an old `Date` therefore makes an entry stale even while
    /// it is young in this process.
    fn is_fresh(&self, now: u64) -> bool {
        let age = self
            .initial_age
            .saturating_add(now.saturating_sub(self.stored_at));
        self.max_age.is_some_and(|max_age| age < max_age)
    }

    /// Serializes metadata without a URL or any other request-derived secret.
    fn metadata(&self) -> String {
        let mut metadata = format!(
            "stored={}\ninitial-age={}\nbody-length={}\nbody-sha256={}\ngeneration={}\n",
            self.stored_at,
            self.initial_age,
            self.body.len(),
            hex_encode(&Sha256::digest(&self.body)),
            self.generation
        );
        if let Some(max_age) = self.max_age {
            metadata.push_str(&format!("max-age={max_age}\n"));
        }
        if let Some(etag) = &self.etag {
            metadata.push_str(&format!("etag={}\n", hex_encode(etag.as_bytes())));
        }
        if let Some(last_modified) = &self.last_modified {
            metadata.push_str(&format!(
                "last-modified={}\n",
                hex_encode(last_modified.as_bytes())
            ));
        }
        metadata
    }
}

/// Parses the metadata block of a cache entry.
///
/// The body length and digest are returned separately so `load` can verify
/// them against the body bytes before using an entry.
fn parse_metadata(metadata: &str) -> Option<(CachedModule, usize, String)> {
    let mut stored_at = None;
    let mut initial_age = None;
    let mut body_length = None;
    let mut body_sha256 = None;
    let mut generation = None;
    let mut max_age = None;
    let mut etag = None;
    let mut last_modified = None;
    for line in metadata.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        match name {
            "stored" => stored_at = value.parse().ok(),
            "initial-age" => initial_age = value.parse().ok(),
            "body-length" => body_length = value.parse().ok(),
            "body-sha256" => body_sha256 = Some(value.to_string()),
            "generation" => generation = Some(value.to_string()),
            "max-age" => max_age = value.parse().ok(),
            "etag" => etag = hex_decode(value),
            "last-modified" => last_modified = hex_decode(value),
            _ => {}
        }
    }
    Some((
        CachedModule {
            body: Vec::new(),
            stored_at: stored_at?,
            initial_age: initial_age?,
            max_age,
            etag,
            last_modified,
            generation: generation?,
        },
        body_length?,
        body_sha256?,
    ))
}

/// Encode bytes as lowercase hexadecimal.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0x0f) as usize] as char);
    }
    text
}

/// Decode lowercase or uppercase hexadecimal into a UTF-8 string.
fn hex_decode(text: &str) -> Option<String> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let text = text.as_bytes();
    for index in (0..text.len()).step_by(2) {
        let high = hex_digit(text[index])?;
        let low = hex_digit(text[index + 1])?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Seconds since the Unix epoch, or zero if the clock is before it.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Rounds a duration up to whole seconds, so an age is never understated.
fn duration_secs_ceil(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

/// Returns the response's age when it arrived, from `Date` and `Age`.
///
/// This follows the conservative part of RFC 9111's age calculation: the
/// apparent age from `Date` and the corrected `Age` value (server age plus the
/// request delay) are both considered, and the larger is used. An `Age` above
/// the freshness lifetime makes an entry stale immediately. A missing or
/// malformed `Date` contributes no apparent age, and a malformed `Age` is
/// ignored.
fn response_age(date: Option<&str>, age: Option<&str>, now: u64, response_delay: u64) -> u64 {
    let apparent = date
        .and_then(|date| httpdate::parse_http_date(date).ok())
        .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |date| now.saturating_sub(date.as_secs()));
    let age_value = age
        .and_then(|age| age.trim().parse::<u64>().ok())
        .unwrap_or(0);
    apparent.max(age_value.saturating_add(response_delay))
}

/// Makes cache generations unique within and across processes.
static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns a new opaque cache generation.
fn new_generation() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        nanos,
        GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst)
    )
}

/// The user-profile module cache.
///
/// One file per URL holds a short metadata block and the body. Writes go to a
/// unique temporary file in the same directory and are renamed into place, so a
/// reader sees either the old entry or the new one, never a partial file. A
/// per-key lock file serializes publication across processes so an older `304`
/// cannot replace a newer `200`.
struct Cache {
    dir: PathBuf,
}

impl Cache {
    fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Returns the canonical cache key: the URL without a fragment.
    fn key(url: &Url) -> String {
        let mut key = url.clone();
        let _ = key.set_username("");
        let _ = key.set_password(None);
        key.set_fragment(None);
        key.to_string()
    }

    /// Returns the hashed file name for a URL.
    ///
    /// Hashing keeps the host, path, and query out of the file name. The key is
    /// the canonical URL including its query.
    fn path(&self, url: &Url) -> PathBuf {
        let digest = Sha256::digest(Self::key(url).as_bytes());
        self.dir.join(hex_encode(&digest))
    }

    /// Returns the metadata for an entry, without reading the body.
    fn read_metadata(&self, url: &Url) -> Option<(CachedModule, usize, String)> {
        let path = self.path(url);
        let file = File::open(path).ok()?;
        let mut reader = BufReader::new(file);
        let mut magic = String::new();
        reader.read_line(&mut magic).ok()?;
        if magic.trim_end() != CACHE_MAGIC {
            return None;
        }
        let mut length_line = String::new();
        reader.read_line(&mut length_line).ok()?;
        let metadata_length: usize = length_line.trim().parse().ok()?;
        if metadata_length > MAX_META_BYTES {
            return None;
        }
        let mut metadata = vec![0u8; metadata_length];
        reader.read_exact(&mut metadata).ok()?;
        parse_metadata(std::str::from_utf8(&metadata).ok()?)
    }

    /// Loads an entry, returning `None` when it is absent or malformed.
    fn load(&self, url: &Url, max_body: usize) -> Option<CachedModule> {
        let (mut entry, expected_length, expected_digest) = self.read_metadata(url)?;
        if expected_length > max_body {
            return None;
        }
        let path = self.path(url);
        let file = File::open(path).ok()?;
        let mut reader = BufReader::new(file);
        // Skip the already-validated header and metadata block.
        let mut magic = String::new();
        reader.read_line(&mut magic).ok()?;
        let mut length_line = String::new();
        reader.read_line(&mut length_line).ok()?;
        let metadata_length: usize = length_line.trim().parse().ok()?;
        let mut metadata = vec![0u8; metadata_length];
        reader.read_exact(&mut metadata).ok()?;
        let mut body = Vec::new();
        reader
            .take(max_body.saturating_add(1) as u64)
            .read_to_end(&mut body)
            .ok()?;
        if body.len() != expected_length
            || !expected_digest.eq_ignore_ascii_case(&hex_encode(&Sha256::digest(&body)))
        {
            return None;
        }
        entry.body = body;
        Some(entry)
    }

    /// Acquires the per-key publication lock.
    fn lock(&self, url: &Url) -> std::io::Result<CacheLock> {
        std::fs::create_dir_all(&self.dir)?;
        let mut lock_path = self.path(url).into_os_string();
        lock_path.push(".lock");
        let lock_path = PathBuf::from(lock_path);
        for _ in 0..400 {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_file) => return Ok(CacheLock { path: lock_path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "cache publication lock timed out",
        ))
    }

    /// Removes an entry only when it still has `expected` generation.
    fn remove_if_generation(&self, url: &Url, expected: Option<&str>) -> std::io::Result<bool> {
        let _lock = self.lock(url)?;
        if let Some(expected) = expected {
            match self.read_metadata(url) {
                Some((entry, _, _)) if entry.generation == expected => {}
                _ => return Ok(false),
            }
        }
        match std::fs::remove_file(self.path(url)) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Writes an entry atomically, returning a filesystem error on failure.
    fn store(&self, url: &Url, entry: &CachedModule) -> std::io::Result<()> {
        let _lock = self.lock(url)?;
        self.write_entry(url, entry)
    }

    /// Writes an entry only when the current generation matches `expected`.
    ///
    /// A `None` expected generation is an unconditional publication for a new
    /// `200`. Returns `false` when a newer generation won the race.
    fn store_if_generation(
        &self,
        url: &Url,
        entry: &CachedModule,
        expected: Option<&str>,
    ) -> std::io::Result<bool> {
        let _lock = self.lock(url)?;
        if let Some(expected) = expected {
            match self.read_metadata(url) {
                Some((current, _, _)) if current.generation == expected => {}
                _ => return Ok(false),
            }
        }
        self.write_entry(url, entry)?;
        Ok(true)
    }

    fn write_entry(&self, url: &Url, entry: &CachedModule) -> std::io::Result<()> {
        let metadata = entry.metadata();
        let path = self.path(url);
        let temporary = self.dir.join(format!(
            ".tmp-{}-{}-{}",
            std::process::id(),
            unix_now(),
            TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            write!(file, "{CACHE_MAGIC}\n{}\n{}", metadata.len(), metadata)?;
            file.write_all(&entry.body)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

/// Holds a per-key cache publication lock until dropped.
struct CacheLock {
    path: PathBuf,
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Makes temporary cache file names unique within one process.
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The production transport: blocking HTTPS through `ureq` and `rustls`.
struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    fn new() -> Self {
        // `https_only` and `redirects(0)` mirror the policy in `RemoteClient`;
        // they are defence in depth, because every hop is already checked there.
        // Environment proxy detection is not enabled, and no cookie store or
        // authorization header exists on this agent.
        let agent = ureq::AgentBuilder::new()
            .https_only(true)
            .redirects(0)
            // Explicit even though the `proxy-from-env` feature is off: no
            // ambient proxy URL or proxy credential is ever read.
            .try_proxy_from_env(false)
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("neojs/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent }
    }

    /// Wraps an agent built with a test-only trust store.
    #[cfg(test)]
    fn with_agent(agent: ureq::Agent) -> Self {
        Self { agent }
    }
}

/// Returns a fixed classification for a transport error.
///
/// Neither `ureq`'s message nor its URL is used: a URL can carry a signed query
/// and the transport message can include it. The category is enough to act on.
fn ureq_error_reason(error: ureq::Error) -> String {
    let Some(error) = error.into_transport() else {
        return "HTTP transport error".to_string();
    };
    let reason = match error.kind() {
        ureq::ErrorKind::InvalidUrl => "invalid URL",
        ureq::ErrorKind::UnknownScheme => "unsupported URL scheme",
        ureq::ErrorKind::Dns => "DNS resolution failed",
        ureq::ErrorKind::InsecureRequestHttpsOnly => "non-HTTPS request refused",
        ureq::ErrorKind::ConnectionFailed => "connection failed",
        ureq::ErrorKind::TooManyRedirects => "too many redirects",
        ureq::ErrorKind::BadStatus => "invalid HTTP status line",
        ureq::ErrorKind::BadHeader => "invalid HTTP header",
        ureq::ErrorKind::Io => "network I/O error",
        ureq::ErrorKind::InvalidProxyUrl => "invalid proxy URL",
        ureq::ErrorKind::ProxyConnect => "proxy connection failed",
        ureq::ErrorKind::ProxyUnauthorized => "proxy authentication failed",
        ureq::ErrorKind::HTTP => "HTTP transport error",
    };
    reason.to_string()
}

impl RemoteTransport for UreqTransport {
    fn get(&self, request: &RemoteRequest<'_>) -> Result<RemoteResponse, String> {
        let mut call = self
            .agent
            .get(request.url.as_str())
            .timeout(request.timeout);
        if let Some(etag) = request.if_none_match {
            call = call.set("If-None-Match", etag);
        }
        if let Some(last_modified) = request.if_modified_since {
            call = call.set("If-Modified-Since", last_modified);
        }
        let response = match call.call() {
            Ok(response) => response,
            // A 4xx or 5xx response still carries a status, headers, and a
            // body. Treat it as a response so the policy reports it uniformly.
            Err(ureq::Error::Status(_, response)) => response,
            Err(error) => return Err(ureq_error_reason(error)),
        };
        let status = response.status();
        let location = response.header("location").map(str::to_owned);
        let declared_length = response
            .header("content-length")
            .and_then(|value| value.parse().ok());
        let etag = response.header("etag").map(str::to_owned);
        let last_modified = response.header("last-modified").map(str::to_owned);
        let cache_control = response.header("cache-control").map(str::to_owned);
        let date = response.header("date").map(str::to_owned);
        let age = response.header("age").map(str::to_owned);
        Ok(RemoteResponse {
            status,
            location,
            declared_length,
            etag,
            last_modified,
            cache_control,
            date,
            age,
            body: response.into_reader(),
        })
    }
}

/// A scripted response for the test-only transport hook.
#[cfg(feature = "test-support")]
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct TestRemoteResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The `Location` header, if any.
    pub location: Option<String>,
    /// The declared `Content-Length`, if any.
    pub declared_length: Option<u64>,
    /// The `ETag` header, if any.
    pub etag: Option<String>,
    /// The `Last-Modified` header, if any.
    pub last_modified: Option<String>,
    /// The `Cache-Control` header, if any.
    pub cache_control: Option<String>,
    /// The `Date` header, if any.
    pub date: Option<String>,
    /// The `Age` header, if any.
    pub age: Option<String>,
    /// The response body.
    pub body: Vec<u8>,
}

#[cfg(feature = "test-support")]
struct ClosureTransport<F>(F);

#[cfg(feature = "test-support")]
impl<F> RemoteTransport for ClosureTransport<F>
where
    F: Fn(&str, Duration, Option<&str>, Option<&str>) -> Result<TestRemoteResponse, String>
        + Send
        + Sync,
{
    fn get(&self, request: &RemoteRequest<'_>) -> Result<RemoteResponse, String> {
        let reply = (self.0)(
            request.url.as_str(),
            request.timeout,
            request.if_none_match,
            request.if_modified_since,
        )?;
        Ok(RemoteResponse {
            status: reply.status,
            location: reply.location,
            declared_length: reply.declared_length,
            etag: reply.etag,
            last_modified: reply.last_modified,
            cache_control: reply.cache_control,
            date: reply.date,
            age: reply.age,
            body: Box::new(std::io::Cursor::new(reply.body)),
        })
    }
}

/// Wraps a closure as an injected transport.
#[cfg(feature = "test-support")]
pub(crate) fn test_transport<F>(fetch: F) -> Arc<dyn RemoteTransport>
where
    F: Fn(&str, Duration, Option<&str>, Option<&str>) -> Result<TestRemoteResponse, String>
        + Send
        + Sync
        + 'static,
{
    Arc::new(ClosureTransport(fetch))
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::io::Write;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Stdio;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use std::time::SystemTime;

    use deno_core::url::Url;

    use super::RemoteClient;
    use super::RemoteOptions;
    use super::RemoteRequest;
    use super::RemoteResponse;
    use super::RemoteTransport;

    type SeenRequest = (String, Option<String>, Option<String>);

    /// Records requested URLs and replies from a queue of responses.
    struct ScriptedTransport {
        responses: Mutex<Vec<RemoteResponse>>,
        seen: Mutex<Vec<SeenRequest>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<RemoteResponse>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl RemoteTransport for ScriptedTransport {
        fn get(&self, request: &RemoteRequest<'_>) -> Result<RemoteResponse, String> {
            self.seen.lock().expect("seen lock").push((
                request.url.to_string(),
                request.if_none_match.map(str::to_owned),
                request.if_modified_since.map(str::to_owned),
            ));
            if self.responses.lock().expect("responses lock").is_empty() {
                return Err("no scripted response".to_string());
            }
            Ok(self.responses.lock().expect("responses lock").remove(0))
        }
    }

    fn response(status: u16, location: Option<&str>, body: &str) -> RemoteResponse {
        RemoteResponse {
            status,
            location: location.map(str::to_owned),
            declared_length: None,
            etag: None,
            last_modified: None,
            cache_control: Some("no-store".to_string()),
            date: None,
            age: None,
            body: Box::new(std::io::Cursor::new(body.as_bytes().to_vec())),
        }
    }

    fn client(options: RemoteOptions, transport: Arc<ScriptedTransport>) -> RemoteClient {
        RemoteClient::with_transport(options, transport)
    }

    /// A client with caching disabled, so these policy tests touch no disk.
    fn options() -> RemoteOptions {
        RemoteOptions::new().without_cache()
    }

    #[test]
    fn relative_and_absolute_redirects_are_followed_and_recorded() {
        let transport = ScriptedTransport::new(vec![
            response(302, Some("/next.js"), ""),
            response(200, None, "export default 1;\n"),
        ]);
        let saw = Arc::clone(&transport);
        let client = client(options(), transport);

        let url = Url::parse("https://example.test/start.js").expect("url");
        let fetched = client.fetch(&url).expect("fetch");
        assert_eq!(fetched.body, b"export default 1;\n");
        assert_eq!(
            fetched.final_url,
            Url::parse("https://example.test/next.js").unwrap()
        );
        assert_eq!(
            *saw.seen.lock().expect("seen"),
            [
                ("https://example.test/start.js".to_string(), None, None),
                ("https://example.test/next.js".to_string(), None, None)
            ]
        );
    }

    #[test]
    fn cross_origin_https_redirects_are_allowed() {
        let transport = ScriptedTransport::new(vec![
            response(301, Some("https://cdn.example.test/mod.js"), ""),
            response(200, None, "export default 2;\n"),
        ]);
        let client = client(options(), transport);
        let url = Url::parse("https://example.test/mod.js").expect("url");

        assert_eq!(
            client.fetch(&url).expect("fetch").body,
            b"export default 2;\n"
        );
    }

    #[test]
    fn downgrade_redirect_is_rejected_without_requesting_the_target() {
        let transport =
            ScriptedTransport::new(vec![response(302, Some("http://example.test/mod.js"), "")]);
        let saw = Arc::clone(&transport);
        let client = client(options(), transport);
        let url = Url::parse("https://example.test/mod.js").expect("url");

        let error = client.fetch(&url).expect_err("downgrade must fail");
        assert!(error.contains("https"), "{error}");
        assert_eq!(saw.seen.lock().expect("seen").len(), 1);
    }

    #[test]
    fn file_redirect_target_is_rejected() {
        let transport = ScriptedTransport::new(vec![response(302, Some("file:///etc/passwd"), "")]);
        let client = client(options(), transport);
        let url = Url::parse("https://example.test/mod.js").expect("url");

        let error = client.fetch(&url).expect_err("file target must fail");
        assert!(error.contains("file"), "{error}");
    }

    #[test]
    fn redirect_limit_is_enforced() {
        let transport = ScriptedTransport::new(vec![
            response(302, Some("/1.js"), ""),
            response(302, Some("/2.js"), ""),
        ]);
        let client = client(options().with_max_redirects(1), Arc::clone(&transport));
        let url = Url::parse("https://example.test/mod.js").expect("url");

        let error = client.fetch(&url).expect_err("second redirect must fail");
        assert!(error.contains("too many redirects"), "{error}");
    }

    #[test]
    fn zero_redirects_refuses_even_one() {
        let transport = ScriptedTransport::new(vec![response(302, Some("/next.js"), "")]);
        let client = client(options().with_max_redirects(0), transport);
        let url = Url::parse("https://example.test/mod.js").expect("url");

        let error = client.fetch(&url).expect_err("redirect must fail");
        assert!(error.contains("too many redirects"), "{error}");
    }

    #[test]
    fn credential_bearing_urls_are_rejected_before_any_request() {
        let transport = ScriptedTransport::new(Vec::new());
        let saw = Arc::clone(&transport);
        let client = client(options(), transport);
        let url = Url::parse("https://user:secret@example.test/mod.js").expect("url");

        let error = client.fetch(&url).expect_err("credentials must fail");
        assert!(error.contains("username or password"), "{error}");
        assert!(!error.contains("secret"), "{error}");
        assert!(saw.seen.lock().expect("seen").is_empty());
    }

    #[test]
    fn queries_are_not_written_to_errors() {
        let transport = ScriptedTransport::new(Vec::new());
        let client = client(options(), transport);
        let url = Url::parse("https://example.test/mod.js?token=secret").expect("url");

        let error = client.fetch(&url).expect_err("missing response must fail");
        assert!(!error.contains("token"), "{error}");
        assert!(!error.contains("secret"), "{error}");
    }

    #[test]
    fn declared_length_above_the_limit_fails_without_reading_the_body() {
        let transport = ScriptedTransport::new(vec![RemoteResponse {
            status: 200,
            location: None,
            declared_length: Some(11),
            etag: None,
            last_modified: None,
            cache_control: None,
            date: None,
            age: None,
            body: Box::new(std::io::Cursor::new(b"0123456789".to_vec())),
        }]);
        let client = client(options().with_max_module_bytes(10), transport);
        let url = Url::parse("https://example.test/mod.js").expect("url");

        let error = client.fetch(&url).expect_err("length must fail");
        assert!(error.contains("10 byte limit"), "{error}");
    }

    #[test]
    fn streamed_body_above_the_limit_fails() {
        let transport = ScriptedTransport::new(vec![RemoteResponse {
            status: 200,
            location: None,
            declared_length: None,
            etag: None,
            last_modified: None,
            cache_control: None,
            date: None,
            age: None,
            body: Box::new(std::io::Cursor::new(b"0123456789".to_vec())),
        }]);
        let client = client(options().with_max_module_bytes(9), transport);
        let url = Url::parse("https://example.test/mod.js").expect("url");

        let error = client.fetch(&url).expect_err("body must fail");
        assert!(error.contains("larger than the 9 byte limit"), "{error}");
    }

    #[test]
    fn http_and_other_schemes_are_rejected() {
        for (specifier, scheme) in [
            ("http://example.test/mod.js", "http"),
            ("data:text/javascript,1", "data"),
            ("blob:https://example.test/id", "blob"),
        ] {
            let client = client(options(), ScriptedTransport::new(Vec::new()));
            let url = Url::parse(specifier).expect("url");
            let error = client.fetch(&url).expect_err("scheme must fail");
            assert!(error.contains(scheme), "{specifier}: {error}");
        }
    }

    #[test]
    fn fragments_are_not_sent_but_queries_are() {
        let transport = ScriptedTransport::new(vec![response(200, None, "")]);
        let saw = Arc::clone(&transport);
        let client = client(options(), transport);
        let url = Url::parse("https://example.test/mod.js?v=1#part").expect("url");
        client.fetch(&url).expect("fetch");
        assert_eq!(
            saw.seen.lock().expect("seen")[0].0,
            "https://example.test/mod.js?v=1"
        );
        assert_eq!(url.fragment(), Some("part"));
    }

    #[test]
    fn zero_timeout_fails_before_a_request() {
        let transport = ScriptedTransport::new(Vec::new());
        let saw = Arc::clone(&transport);
        let client = client(options().with_timeout(Duration::ZERO), transport);
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let error = client.fetch(&url).expect_err("zero timeout must fail");
        assert!(error.contains("timeout"), "{error}");
        assert!(saw.seen.lock().expect("seen").is_empty());
    }

    #[test]
    fn a_fresh_entry_is_reused_without_a_request() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![RemoteResponse {
            status: 200,
            location: None,
            declared_length: None,
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            cache_control: Some("max-age=60".to_string()),
            date: None,
            age: None,
            body: Box::new(std::io::Cursor::new(b"export default 1;".to_vec())),
        }]);
        let saw = Arc::clone(&transport);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js?v=1").expect("url");

        let first = client(options.clone(), Arc::clone(&transport));
        assert_eq!(first.fetch(&url).expect("first").body, b"export default 1;");
        // A second client over the same directory sees the entry.
        let second = client(options, Arc::clone(&transport));
        assert_eq!(
            second.fetch(&url).expect("second").body,
            b"export default 1;"
        );
        assert_eq!(saw.seen.lock().expect("seen").len(), 1);
    }

    #[test]
    fn a_stale_entry_revalidates_with_its_validators() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".to_string()),
                cache_control: Some("max-age=0".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"export default 1;".to_vec())),
            },
            RemoteResponse {
                status: 304,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=0".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(Vec::new())),
            },
        ]);
        let saw = Arc::clone(&transport);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");

        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("first");
        let second = client(options, Arc::clone(&transport))
            .fetch(&url)
            .expect("second");
        assert_eq!(second.body, b"export default 1;");
        let seen = saw.seen.lock().expect("seen");
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].1.as_deref(), Some("\"v1\""));
        assert_eq!(seen[1].2.as_deref(), Some("Wed, 21 Oct 2015 07:28:00 GMT"));
    }

    #[test]
    fn a_server_age_counts_toward_freshness() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: None,
                age: Some("120".to_string()),
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: None,
                age: Some("0".to_string()),
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
        ]);
        let saw = Arc::clone(&transport);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");

        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("first");
        client(options, Arc::clone(&transport))
            .fetch(&url)
            .expect("second");
        assert_eq!(saw.seen.lock().expect("seen").len(), 2);
    }

    #[test]
    fn an_old_date_counts_toward_freshness() {
        let dir = tempfile::tempdir().expect("temp dir");
        let date = httpdate::fmt_http_date(
            SystemTime::now()
                .checked_sub(Duration::from_secs(120))
                .expect("past time"),
        );
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: Some(date.clone()),
                age: None,
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: Some(httpdate::fmt_http_date(SystemTime::now())),
                age: None,
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
        ]);
        let saw = Arc::clone(&transport);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");

        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("first");
        client(options, Arc::clone(&transport))
            .fetch(&url)
            .expect("second");
        assert_eq!(saw.seen.lock().expect("seen").len(), 2);
    }

    #[test]
    fn a_304_with_a_server_age_does_not_refresh_the_entry() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=0".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
            RemoteResponse {
                status: 304,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: None,
                age: Some("120".to_string()),
                body: Box::new(std::io::Cursor::new(Vec::new())),
            },
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=0".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
        ]);
        let saw = Arc::clone(&transport);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");

        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("first");
        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("revalidate");
        client(options, Arc::clone(&transport))
            .fetch(&url)
            .expect("third");
        let seen = saw.seen.lock().expect("seen");
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[2].1.as_deref(), Some("\"v1\""));
    }

    #[test]
    fn a_fresh_304_metadata_update_allows_reuse() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=0".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
            RemoteResponse {
                status: 304,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: Some(httpdate::fmt_http_date(SystemTime::now())),
                age: Some("0".to_string()),
                body: Box::new(std::io::Cursor::new(Vec::new())),
            },
        ]);
        let saw = Arc::clone(&transport);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");

        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("first");
        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("revalidate");
        let third = client(options, Arc::clone(&transport))
            .fetch(&url)
            .expect("a fresh 304 must make the entry reusable");
        assert_eq!(third.body, b"body");
        assert_eq!(saw.seen.lock().expect("seen").len(), 2);
    }

    #[test]
    fn no_store_is_not_written() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![response(200, None, "body")]);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");
        client(options, Arc::clone(&transport))
            .fetch(&url)
            .expect("fetch");
        assert_eq!(std::fs::read_dir(dir.path()).expect("cache dir").count(), 0);
    }

    #[test]
    fn no_store_removes_a_stale_entry() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=0".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"body".to_vec())),
            },
            response(200, None, "body"),
        ]);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");
        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("first");
        client(options, transport).fetch(&url).expect("second");
        assert_eq!(std::fs::read_dir(dir.path()).expect("cache dir").count(), 0);
    }

    #[test]
    fn cache_file_names_do_not_contain_the_url_or_query() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![RemoteResponse {
            status: 200,
            location: None,
            declared_length: None,
            etag: None,
            last_modified: None,
            cache_control: Some("max-age=60".to_string()),
            date: None,
            age: None,
            body: Box::new(std::io::Cursor::new(b"body".to_vec())),
        }]);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js?token=secret").expect("url");
        client(options, transport).fetch(&url).expect("fetch");

        for entry in std::fs::read_dir(dir.path()).expect("cache dir") {
            let name = entry.expect("entry").file_name();
            let name = name.to_string_lossy();
            assert!(!name.contains("example"), "{name}");
            assert!(!name.contains("token"), "{name}");
            assert!(!name.contains("secret"), "{name}");
        }
    }

    #[test]
    fn concurrent_writers_publish_one_complete_entry() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = Arc::new(super::Cache::new(dir.path().to_path_buf()));
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let mut threads = Vec::new();
        for length in 1..=4 {
            let cache = Arc::clone(&cache);
            let url = url.clone();
            threads.push(std::thread::spawn(move || {
                let entry = super::CachedModule {
                    body: vec![b'x'; length * 1024],
                    stored_at: 0,
                    initial_age: 0,
                    max_age: Some(60),
                    etag: None,
                    last_modified: None,
                    generation: "test".to_string(),
                };
                cache.store(&url, &entry).expect("store");
            }));
        }
        for thread in threads {
            thread.join().expect("writer");
        }
        let entry = cache.load(&url, 8192).expect("complete entry");
        assert!(matches!(entry.body.len(), 1024 | 2048 | 3072 | 4096));
    }

    #[test]
    fn the_cache_directory_honors_xdg_cache_home_when_absolute() {
        assert_eq!(
            super::cache_dir_from(
                Some(std::path::PathBuf::from("/tmp/xdg-cache")),
                Some(std::path::PathBuf::from("/home/user")),
            ),
            Some(std::path::PathBuf::from("/tmp/xdg-cache/neojs/packages"))
        );
        assert_eq!(
            super::cache_dir_from(
                Some(std::path::PathBuf::from("relative-cache")),
                Some(std::path::PathBuf::from("/home/user")),
            ),
            Some(std::path::PathBuf::from("/home/user/.cache/neojs/packages"))
        );
        assert_eq!(
            super::cache_dir_from(None, Some(std::path::PathBuf::from("/home/user"))),
            Some(std::path::PathBuf::from("/home/user/.cache/neojs/packages"))
        );
        assert_eq!(super::cache_dir_from(None, None), None);
    }

    #[test]
    fn the_module_and_total_byte_budgets_are_enforced() {
        let transport = ScriptedTransport::new(vec![
            response(200, None, "aaaa"),
            response(200, None, "bbbb"),
        ]);
        let module_limited = client(options().with_max_remote_modules(1), Arc::clone(&transport));
        let one = Url::parse("https://example.test/one.js").expect("url");
        let two = Url::parse("https://example.test/two.js").expect("url");
        module_limited.fetch(&one).expect("first module");
        let error = module_limited.fetch(&two).expect_err("module limit");
        assert!(error.contains("1 module limit"), "{error}");

        let transport = ScriptedTransport::new(vec![
            response(200, None, "aaaa"),
            response(200, None, "bbbb"),
        ]);
        let byte_limited = client(options().with_max_total_bytes(7), Arc::clone(&transport));
        byte_limited.fetch(&one).expect("first bytes");
        let error = byte_limited.fetch(&two).expect_err("byte limit");
        assert!(
            error.contains("remaining graph budget") || error.contains("byte total limit"),
            "{error}"
        );
    }

    #[test]
    fn a_rejected_graph_admission_does_not_publish_a_cache_entry() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"aa".to_vec())),
            },
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"bb".to_vec())),
            },
        ]);
        let client = client(
            RemoteOptions::new()
                .with_max_total_bytes(3)
                .with_cache_dir(dir.path()),
            transport,
        );
        let one = Url::parse("https://example.test/one.js").expect("url");
        let two = Url::parse("https://example.test/two.js").expect("url");
        client.fetch(&one).expect("first body fits");
        let error = client
            .fetch(&two)
            .expect_err("remaining budget is one byte");
        assert!(error.contains("remaining graph budget"), "{error}");
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("cache dir")
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
                })
                .count(),
            1,
            "the rejected module must not be cached"
        );
    }

    #[test]
    fn cache_rejects_a_modified_or_truncated_body() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = super::Cache::new(dir.path().to_path_buf());
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let entry = super::CachedModule {
            body: b"export default 1;\n".to_vec(),
            stored_at: 1,
            initial_age: 0,
            max_age: Some(60),
            etag: None,
            last_modified: None,
            generation: "test".to_string(),
        };
        cache.store(&url, &entry).expect("store");
        assert!(cache.load(&url, 1024).is_some());

        let path = cache.path(&url);
        let original = std::fs::read(&path).expect("read cache");
        let needle = b"export default 1;\n";
        let replacement = b"export default 2;\n";
        let offset = original
            .windows(needle.len())
            .position(|window| window == needle)
            .expect("body in cache file");
        let mut tampered = original.clone();
        tampered[offset..offset + needle.len()].copy_from_slice(replacement);
        std::fs::write(&path, &tampered).expect("write tampered");
        assert!(
            cache.load(&url, 1024).is_none(),
            "a modified body must miss"
        );

        std::fs::write(&path, &original[..original.len() - 2]).expect("truncate");
        assert!(
            cache.load(&url, 1024).is_none(),
            "a truncated body must miss"
        );

        std::fs::write(&path, &original).expect("restore");
        assert_eq!(cache.load(&url, 1024).expect("restored").body, entry.body);
    }

    #[test]
    fn a_corrupt_cache_entry_is_refetched_not_executed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let transport = ScriptedTransport::new(vec![
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"export default 1;".to_vec())),
            },
            RemoteResponse {
                status: 200,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: Some("max-age=60".to_string()),
                date: None,
                age: None,
                body: Box::new(std::io::Cursor::new(b"export default 2;".to_vec())),
            },
        ]);
        let saw = Arc::clone(&transport);
        let options = RemoteOptions::new().with_cache_dir(dir.path());
        let url = Url::parse("https://example.test/mod.js").expect("url");
        client(options.clone(), Arc::clone(&transport))
            .fetch(&url)
            .expect("first fetch");

        let cache = super::Cache::new(dir.path().to_path_buf());
        let path = cache.path(&url);
        let original = std::fs::read(&path).expect("cache file");
        let mut tampered = original.clone();
        let needle = b"export default 1;";
        let replacement = b"export default 9;";
        let offset = tampered
            .windows(needle.len())
            .position(|window| window == needle)
            .expect("body");
        tampered[offset..offset + needle.len()].copy_from_slice(replacement);
        std::fs::write(&path, tampered).expect("tamper");

        let fetched = client(options, transport).fetch(&url).expect("refetch");
        assert_eq!(fetched.body, b"export default 2;");
        assert_eq!(saw.seen.lock().expect("seen").len(), 2);
    }

    #[test]
    fn concurrent_readers_never_observe_a_partial_entry() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = Arc::new(super::Cache::new(dir.path().to_path_buf()));
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut readers = Vec::new();
        for _ in 0..2 {
            let cache = Arc::clone(&cache);
            let url = url.clone();
            let stop = Arc::clone(&stop);
            readers.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Some(entry) = cache.load(&url, 8192) {
                        assert_eq!(entry.body.len(), 1024);
                        assert!(matches!(entry.body[0], b'a'..=b'd'));
                    }
                    std::thread::yield_now();
                }
            }));
        }
        let mut writers = Vec::new();
        for index in 0..4u8 {
            let cache = Arc::clone(&cache);
            let url = url.clone();
            writers.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    let entry = super::CachedModule {
                        body: vec![b'a' + index; 1024],
                        stored_at: 1,
                        initial_age: 0,
                        max_age: Some(60),
                        etag: None,
                        last_modified: None,
                        generation: "test".to_string(),
                    };
                    cache.store(&url, &entry).expect("store");
                }
            }));
        }
        for writer in writers {
            writer.join().expect("writer");
        }
        stop.store(true, Ordering::Relaxed);
        for reader in readers {
            reader.join().expect("reader");
        }
        assert_eq!(cache.load(&url, 8192).expect("final").body.len(), 1024);
    }

    fn certificate() -> (Vec<u8>, Vec<u8>) {
        let certified = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()])
            .expect("self-signed certificate");
        (
            certified.cert.der().to_vec(),
            certified.signing_key.serialize_der(),
        )
    }

    fn spawn_tls_server(cert_der: Vec<u8>, key_der: Vec<u8>) -> (u16, std::thread::JoinHandle<()>) {
        use rustls::pki_types::CertificateDer;
        use rustls::pki_types::PrivateKeyDer;
        use rustls::pki_types::PrivatePkcs8KeyDer;

        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert_der)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
            )
            .expect("server config");
        let config = Arc::new(config);
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handle = std::thread::spawn(move || {
            let Ok((tcp, _)) = listener.accept() else {
                return;
            };
            let Ok(connection) = rustls::ServerConnection::new(config) else {
                return;
            };
            let mut stream = rustls::StreamOwned::new(connection, tcp);
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = b"export default 1;\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        });
        (port, handle)
    }

    fn trusted_agent(cert_der: &[u8]) -> ureq::Agent {
        use rustls::pki_types::CertificateDer;

        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(cert_der.to_vec()))
            .expect("trust test certificate");
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        ureq::AgentBuilder::new()
            .https_only(true)
            .redirects(0)
            .try_proxy_from_env(false)
            .timeout(Duration::from_secs(5))
            .tls_config(Arc::new(config))
            .build()
    }

    #[test]
    fn a_real_transport_error_never_echoes_the_query() {
        let url = Url::parse("https://127.0.0.1:1/mod.js?token=sentinel").expect("url");
        let error = super::UreqTransport::new()
            .get(&super::RemoteRequest {
                url: &url,
                timeout: Duration::from_secs(1),
                if_none_match: None,
                if_modified_since: None,
            })
            .err()
            .expect("connection refused");
        assert!(!error.contains("sentinel"), "{error}");
        assert!(!error.contains("token"), "{error}");
    }

    #[test]
    fn the_production_transport_negotiates_tls_and_rejects_an_untrusted_certificate() {
        let (cert_der, key_der) = certificate();

        let (port, handle) = spawn_tls_server(cert_der.clone(), key_der.clone());
        let url =
            Url::parse(&format!("https://127.0.0.1:{port}/mod.js?token=sentinel")).expect("url");
        let untrusted = super::UreqTransport::new();
        let error = untrusted
            .get(&super::RemoteRequest {
                url: &url,
                timeout: Duration::from_secs(5),
                if_none_match: None,
                if_modified_since: None,
            })
            .err()
            .expect("a self-signed certificate is untrusted");
        assert!(!error.contains("sentinel"), "{error}");
        assert!(!error.contains(&port.to_string()), "{error}");
        handle.join().expect("server");

        let (port, handle) = spawn_tls_server(cert_der.clone(), key_der);
        let url = Url::parse(&format!("https://127.0.0.1:{port}/mod.js")).expect("url");
        let transport = super::UreqTransport::with_agent(trusted_agent(&cert_der));
        let response = transport
            .get(&super::RemoteRequest {
                url: &url,
                timeout: Duration::from_secs(5),
                if_none_match: None,
                if_modified_since: None,
            })
            .expect("trusted TLS response");
        assert_eq!(response.status, 200);
        let mut body = Vec::new();
        response
            .body
            .take(1024)
            .read_to_end(&mut body)
            .expect("body");
        assert_eq!(body, b"export default 1;\n");
        handle.join().expect("server");
    }

    /// Child half of [`production_transport_does_not_use_an_ambient_proxy`].
    #[test]
    fn ambient_proxy_child() {
        let Some(destination) = std::env::var_os("NEOJS_DIRECT_DEST") else {
            return;
        };
        let Some(marker) = std::env::var_os("NEOJS_PROXY_MARKER") else {
            return;
        };
        let url = Url::parse(&format!(
            "https://127.0.0.1:{}/mod.js?token=sentinel",
            destination.to_string_lossy()
        ))
        .expect("url");
        let transport = super::UreqTransport::new();
        let _ = transport.get(&super::RemoteRequest {
            url: &url,
            timeout: Duration::from_secs(2),
            if_none_match: None,
            if_modified_since: None,
        });
        std::fs::write(marker, "completed").expect("completion marker");
    }

    #[test]
    fn production_transport_does_not_use_an_ambient_proxy() {
        if std::env::var_os("NEOJS_PROXY_CHILD").is_some() {
            return;
        }
        let proxy_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind proxy observer");
        proxy_listener
            .set_nonblocking(true)
            .expect("nonblocking proxy listener");
        let proxy_port = proxy_listener.local_addr().expect("proxy addr").port();
        let direct_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind direct observer");
        direct_listener
            .set_nonblocking(true)
            .expect("nonblocking direct listener");
        let direct_port = direct_listener.local_addr().expect("direct addr").port();
        let dir = tempfile::tempdir().expect("marker dir");
        let marker = dir.path().join("completed");
        let proxy = format!("http://user:secret@127.0.0.1:{proxy_port}");
        let mut child = Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "remote::tests::ambient_proxy_child",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("HTTPS_PROXY", &proxy)
            .env("HTTP_PROXY", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("NEOJS_PROXY_CHILD", "1")
            .env("NEOJS_DIRECT_DEST", direct_port.to_string())
            .env("NEOJS_PROXY_MARKER", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child test");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut direct_reached = false;
        loop {
            if let Ok((_stream, _)) = proxy_listener.accept() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("production transport contacted an ambient proxy");
            }
            if let Ok((_stream, _)) = direct_listener.accept() {
                direct_reached = true;
            }
            if marker.exists() {
                assert!(
                    direct_reached,
                    "the direct synthetic destination was not reached"
                );
                let status = child.wait().expect("child status");
                assert!(status.success(), "child test failed: {status}");
                return;
            }
            if let Some(status) = child.try_wait().expect("child status") {
                if !direct_reached {
                    direct_reached = direct_listener.accept().is_ok();
                }
                assert!(status.success(), "child test failed: {status}");
                assert!(
                    direct_reached,
                    "the direct synthetic destination was not reached"
                );
                assert!(
                    marker.exists(),
                    "the child did not write its completion marker"
                );
                return;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child test did not finish");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn an_older_304_cannot_replace_a_newer_200() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = super::Cache::new(dir.path().to_path_buf());
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let old = super::CachedModule {
            body: b"old".to_vec(),
            stored_at: 1,
            initial_age: 0,
            max_age: Some(60),
            etag: Some("\"old\"".to_string()),
            last_modified: None,
            generation: "old-generation".to_string(),
        };
        cache.store(&url, &old).expect("old store");
        let loaded = cache.load(&url, 1024).expect("old entry");
        let new = super::CachedModule {
            body: b"new".to_vec(),
            stored_at: 2,
            initial_age: 0,
            max_age: Some(60),
            etag: Some("\"new\"".to_string()),
            last_modified: None,
            generation: "new-generation".to_string(),
        };
        cache.store(&url, &new).expect("new store");

        let published = cache
            .store_if_generation(&url, &loaded, Some(&loaded.generation))
            .expect("conditional store");
        assert!(!published, "the old generation must lose");
        assert_eq!(cache.load(&url, 1024).expect("current").body, b"new");
    }

    #[test]
    fn an_older_no_store_304_cannot_remove_a_newer_200() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = super::Cache::new(dir.path().to_path_buf());
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let old = super::CachedModule {
            body: b"old".to_vec(),
            stored_at: 1,
            initial_age: 0,
            max_age: Some(60),
            etag: None,
            last_modified: None,
            generation: "old".to_string(),
        };
        cache.store(&url, &old).expect("old store");
        let new = super::CachedModule {
            body: b"new".to_vec(),
            stored_at: 2,
            initial_age: 0,
            max_age: Some(60),
            etag: None,
            last_modified: None,
            generation: "new".to_string(),
        };
        cache.store(&url, &new).expect("new store");
        assert!(
            !cache
                .remove_if_generation(&url, Some("old"))
                .expect("conditional remove")
        );
        assert_eq!(cache.load(&url, 1024).expect("current").body, b"new");
    }

    #[test]
    fn a_304_publishes_when_its_generation_is_current() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = super::Cache::new(dir.path().to_path_buf());
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let entry = super::CachedModule {
            body: b"body".to_vec(),
            stored_at: 1,
            initial_age: 0,
            max_age: Some(0),
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            generation: "generation-1".to_string(),
        };
        cache.store(&url, &entry).expect("store");
        let mut revalidated = cache.load(&url, 1024).expect("entry");
        revalidated.max_age = Some(60);
        revalidated.stored_at = 2;
        assert!(
            cache
                .store_if_generation(&url, &revalidated, Some(&revalidated.generation))
                .expect("conditional store")
        );
        assert_eq!(cache.load(&url, 1024).expect("entry").max_age, Some(60));
    }

    #[test]
    fn concurrent_generations_settle_on_the_newer_publication() {
        let dir = tempfile::tempdir().expect("temp dir");
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let old_cache = Arc::new(super::Cache::new(dir.path().to_path_buf()));
        let old = super::CachedModule {
            body: b"old".to_vec(),
            stored_at: 1,
            initial_age: 0,
            max_age: Some(60),
            etag: None,
            last_modified: None,
            generation: "old".to_string(),
        };
        old_cache.store(&url, &old).expect("old store");
        let loaded = old_cache.load(&url, 1024).expect("old entry");

        for _ in 0..20 {
            let new_cache = Arc::new(super::Cache::new(dir.path().to_path_buf()));
            let right_cache = Arc::clone(&old_cache);
            let url_for_old = url.clone();
            let old_entry = loaded.clone();
            let new = super::CachedModule {
                body: b"new".to_vec(),
                stored_at: 2,
                initial_age: 0,
                max_age: Some(60),
                etag: None,
                last_modified: None,
                generation: super::new_generation(),
            };
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let left = {
                let barrier = Arc::clone(&barrier);
                let cache = Arc::clone(&new_cache);
                let url = url.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    cache.store(&url, &new).expect("new store");
                })
            };
            let right = {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    right_cache
                        .store_if_generation(&url_for_old, &old_entry, Some(&old_entry.generation))
                        .expect("old conditional store");
                })
            };
            left.join().expect("new writer");
            right.join().expect("old writer");
            assert_eq!(
                old_cache.load(&url, 1024).expect("current").body,
                b"new",
                "an older generation replaced the newer body"
            );
        }
    }

    /// Child half of [`an_older_304_loses_to_a_newer_200_across_processes`].
    #[test]
    fn cache_generation_child() {
        let Some(role) = std::env::var_os("NEOJS_CACHE_CHILD") else {
            return;
        };
        let dir = std::env::var_os("NEOJS_CACHE_DIR").expect("cache dir");
        let url = Url::parse(&std::env::var("NEOJS_CACHE_URL").expect("cache url")).expect("url");
        let cache = super::Cache::new(PathBuf::from(dir));
        let entry = super::CachedModule {
            body: if role == "new" {
                b"new".to_vec()
            } else {
                b"old".to_vec()
            },
            stored_at: 1,
            initial_age: 0,
            max_age: Some(60),
            etag: None,
            last_modified: None,
            generation: if role == "new" {
                "new-generation".to_string()
            } else {
                std::env::var("NEOJS_CACHE_OLD_GENERATION").expect("old generation")
            },
        };
        if role == "new" {
            cache.store(&url, &entry).expect("new store");
        } else {
            let expected = std::env::var("NEOJS_CACHE_OLD_GENERATION").expect("old generation");
            cache
                .store_if_generation(&url, &entry, Some(&expected))
                .expect("old conditional store");
        }
    }

    #[test]
    fn an_older_304_loses_to_a_newer_200_across_processes() {
        if std::env::var_os("NEOJS_CACHE_CHILD").is_some() {
            return;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let url = Url::parse("https://example.test/mod.js").expect("url");
        let cache = super::Cache::new(dir.path().to_path_buf());
        let old = super::CachedModule {
            body: b"old".to_vec(),
            stored_at: 1,
            initial_age: 0,
            max_age: Some(60),
            etag: None,
            last_modified: None,
            generation: "old-generation".to_string(),
        };
        cache.store(&url, &old).expect("old store");

        let mut children = Vec::new();
        for role in ["new", "old"] {
            children.push(
                Command::new(std::env::current_exe().expect("test binary"))
                    .args([
                        "remote::tests::cache_generation_child",
                        "--exact",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("NEOJS_CACHE_CHILD", role)
                    .env("NEOJS_CACHE_DIR", dir.path())
                    .env("NEOJS_CACHE_URL", url.as_str())
                    .env("NEOJS_CACHE_OLD_GENERATION", "old-generation")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("spawn cache child"),
            );
        }
        for mut child in children {
            let status = child.wait().expect("cache child status");
            assert!(status.success(), "cache child failed: {status}");
        }
        assert_eq!(
            cache.load(&url, 1024).expect("current entry").body,
            b"new",
            "an older 304 replaced a newer 200 across processes"
        );
    }

    #[test]
    fn cache_metadata_never_contains_the_url() {
        let entry = super::CachedModule {
            body: Vec::new(),
            stored_at: 1,
            initial_age: 0,
            max_age: Some(60),
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            generation: "test".to_string(),
        };
        assert!(!entry.metadata().contains("https://"));
        assert!(!entry.metadata().contains("example"));
    }
}
