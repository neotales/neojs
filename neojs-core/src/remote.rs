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
//! fresh. A stale entry is revalidated with `If-None-Match` and
//! `If-Modified-Since` when the server supplied validators; a `304` reuses the
//! cached body and refreshes its metadata. An entry with no freshness lifetime
//! is always revalidated. `Cache-Control: no-store` is not stored, and
//! `no-cache` forces revalidation. A cache read or write failure is ignored:
//! the module is fetched and the program continues. There is no lockfile and no
//! content hash, so the cache is not integrity protection and does not pin a
//! module version. Openness of the cache directory is the user's
//! responsibility.
//!
//! This is not SSRF protection. There is no host allowlist, no private-address
//! rejection, and no DNS pinning: neojs fetches whatever HTTPS URL the program
//! names, using the process's network authority. Remote code has the same
//! authority as local code once it runs; it is not a sandbox. The cache is an
//! internal part of module loading; neojs exposes no `fetch`-style API and no
//! way for JavaScript to read the cache.

use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use deno_core::url::Url;
use sha2::Digest;
use sha2::Sha256;

/// Default maximum size of one remote module body.
const DEFAULT_MAX_MODULE_BYTES: usize = 16 * 1024 * 1024;
/// Default maximum number of redirects followed for one module.
const DEFAULT_MAX_REDIRECTS: u32 = 5;
/// Default deadline for one module fetch, including redirects and the body.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// The first line of a cache entry.
const CACHE_MAGIC: &str = "neojs-remote-cache 1";
/// Largest metadata block accepted from a cache entry.
const MAX_META_BYTES: usize = 64 * 1024;

/// Limits and cache settings for remote module fetches.
///
/// Create one with [`RemoteOptions::new`] and adjust it with the `with_*`
/// methods. The defaults are a 30-second deadline, a 16 MiB body limit, five
/// redirects, and the user-profile package cache. These are resource bounds,
/// not permissions: enabling remote loading is the permission decision.
#[derive(Clone, Debug)]
pub struct RemoteOptions {
    timeout: Duration,
    max_module_bytes: usize,
    max_redirects: u32,
    cache_dir: Option<PathBuf>,
}

impl Default for RemoteOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_module_bytes: DEFAULT_MAX_MODULE_BYTES,
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
    /// The deadline covers every redirect and reading the body. A zero duration
    /// makes every fetch fail immediately. DNS resolution may exceed it because
    /// the resolver API cannot be interrupted.
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

/// A configured remote module client.
#[derive(Clone)]
pub(crate) struct RemoteClient {
    options: RemoteOptions,
    transport: Arc<dyn RemoteTransport>,
    cache: Option<Arc<Cache>>,
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
        }
    }

    /// Fetches a module body, following checked redirects and using the cache.
    ///
    /// The URL is validated first. The returned body and final URL are the
    /// final response; the final URL is what the module map aliases the
    /// requested URL to.
    pub(crate) fn fetch(&self, requested: &Url) -> Result<FetchedModule, String> {
        validate_url(requested)?;
        if self.options.timeout.is_zero() {
            return Err("the remote timeout is zero".to_string());
        }
        let deadline = Instant::now()
            .checked_add(self.options.timeout)
            .ok_or_else(|| "the remote timeout is too large".to_string())?;

        let mut current = request_url(requested);
        let mut redirects = 0;
        loop {
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
                return Ok(FetchedModule {
                    body: cached.body.clone(),
                    final_url: current,
                });
            }
            let etag = cached.as_ref().and_then(|entry| entry.etag.as_deref());
            let last_modified = cached
                .as_ref()
                .and_then(|entry| entry.last_modified.as_deref());
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

            if response.status == 304 {
                let Some(mut cached) = cached else {
                    return Err(format!(
                        "HTTP 304 from {} without a cached module",
                        display_url(&current)
                    ));
                };
                let directives = CacheDirectives::parse(response.cache_control.as_deref());
                cached.stored_at = now;
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
                if let Some(cache) = &self.cache {
                    if directives.no_store {
                        cache.remove(&current);
                    } else {
                        let _ = cache.store(&current, &cached);
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
            if let Some(length) = response.declared_length
                && length > self.options.max_module_bytes as u64
            {
                return Err(format!(
                    "module body from {} declares {length} bytes, above the {} byte limit",
                    display_url(&current),
                    self.options.max_module_bytes
                ));
            }
            let body = read_bounded(
                response.body,
                self.options.max_module_bytes,
                deadline,
                &current,
            )?;
            let directives = CacheDirectives::parse(response.cache_control.as_deref());
            if let Some(cache) = &self.cache {
                if directives.no_store {
                    cache.remove(&current);
                } else {
                    let entry = CachedModule {
                        body: body.clone(),
                        stored_at: now,
                        max_age: if directives.no_cache {
                            Some(0)
                        } else {
                            directives.max_age
                        },
                        etag: response.etag,
                        last_modified: response.last_modified,
                    };
                    let _ = cache.store(&current, &entry);
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
    let mut clean = url.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.set_query(None);
    clean.set_fragment(None);
    clean.to_string()
}

/// Reads at most `limit` bytes, checking the deadline between reads.
fn read_bounded(
    mut body: Box<dyn Read + Send>,
    limit: usize,
    deadline: Instant,
    url: &Url,
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
                "module body from {} is larger than the {limit} byte limit",
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
            if directive.eq_ignore_ascii_case("no-store") {
                directives.no_store = true;
            } else if directive.eq_ignore_ascii_case("no-cache") {
                directives.no_cache = true;
            } else if let Some(value) = directive
                .get(..8)
                .filter(|prefix| prefix.eq_ignore_ascii_case("max-age="))
                .map(|_| &directive[8..])
            {
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
struct CachedModule {
    body: Vec<u8>,
    stored_at: u64,
    /// `Cache-Control: max-age`, or `Some(0)` for `no-cache`.
    max_age: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
}

impl CachedModule {
    /// Returns whether the entry may be used without a request.
    fn is_fresh(&self, now: u64) -> bool {
        self.max_age
            .is_some_and(|max_age| now.saturating_sub(self.stored_at) < max_age)
    }

    /// Serializes metadata without a URL or any other request-derived secret.
    fn metadata(&self) -> String {
        let mut metadata = format!("stored={}\n", self.stored_at);
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
fn parse_metadata(metadata: &str) -> Option<CachedModule> {
    let mut stored_at = None;
    let mut max_age = None;
    let mut etag = None;
    let mut last_modified = None;
    for line in metadata.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        match name {
            "stored" => stored_at = value.parse().ok(),
            "max-age" => max_age = value.parse().ok(),
            "etag" => etag = hex_decode(value),
            "last-modified" => last_modified = hex_decode(value),
            _ => {}
        }
    }
    Some(CachedModule {
        body: Vec::new(),
        stored_at: stored_at?,
        max_age,
        etag,
        last_modified,
    })
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

/// The user-profile module cache.
///
/// One file per URL holds a short metadata block and the body. Writes go to a
/// unique temporary file in the same directory and are renamed into place, so a
/// reader sees either the old entry or the new one, never a partial file.
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

    /// Loads an entry, returning `None` when it is absent or malformed.
    fn load(&self, url: &Url, max_body: usize) -> Option<CachedModule> {
        let path = self.path(url);
        let metadata = std::fs::metadata(&path).ok()?;
        let max_file = (max_body as u64)
            .saturating_add(MAX_META_BYTES as u64)
            .saturating_add(128);
        if metadata.len() > max_file {
            return None;
        }
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
        let mut entry = parse_metadata(std::str::from_utf8(&metadata).ok()?)?;
        let mut body = Vec::new();
        reader
            .take(max_body.saturating_add(1) as u64)
            .read_to_end(&mut body)
            .ok()?;
        if body.len() > max_body {
            return None;
        }
        entry.body = body;
        Some(entry)
    }

    /// Removes an entry after a `no-store` response.
    fn remove(&self, url: &Url) {
        let _ = std::fs::remove_file(self.path(url));
    }

    /// Writes an entry atomically, returning a filesystem error on failure.
    fn store(&self, url: &Url, entry: &CachedModule) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
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
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("neojs/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent }
    }
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
            Err(error) => return Err(error.to_string()),
        };
        let status = response.status();
        let location = response.header("location").map(str::to_owned);
        let declared_length = response
            .header("content-length")
            .and_then(|value| value.parse().ok());
        let etag = response.header("etag").map(str::to_owned);
        let last_modified = response.header("last-modified").map(str::to_owned);
        let cache_control = response.header("cache-control").map(str::to_owned);
        Ok(RemoteResponse {
            status,
            location,
            declared_length,
            etag,
            last_modified,
            cache_control,
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
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

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
                body: Box::new(std::io::Cursor::new(b"export default 1;".to_vec())),
            },
            RemoteResponse {
                status: 304,
                location: None,
                declared_length: None,
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                cache_control: Some("max-age=0".to_string()),
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
                    max_age: Some(60),
                    etag: None,
                    last_modified: None,
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
    fn cache_metadata_never_contains_the_url() {
        let entry = super::CachedModule {
            body: Vec::new(),
            stored_at: 1,
            max_age: Some(60),
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
        };
        assert!(!entry.metadata().contains("https://"));
        assert!(!entry.metadata().contains("example"));
    }
}
