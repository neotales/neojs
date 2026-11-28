//! Public configuration, request, response, and download types for `vaeljs-fetch-core`.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::FetchError;
use crate::security::is_sensitive_header_name;

/// Default request and download deadline (`30s`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum permitted request or download deadline (`600s`).
pub const MAX_TIMEOUT: Duration = Duration::from_secs(600);

/// Maximum bounded DNS resolution wait per connection (`10s`).
pub const MAX_DNS_TIMEOUT: Duration = Duration::from_secs(10);

/// Default maximum buffered response body size (`16 MiB`).
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum permitted buffered response body limit (`256 MiB`).
pub const MAX_ALLOWED_RESPONSE_BYTES: usize = 256 * 1024 * 1024;

/// Default maximum number of redirect hops followed.
pub const DEFAULT_MAX_REDIRECTS: usize = 10;

/// Maximum permitted redirect limit (`20`).
pub const MAX_ALLOWED_REDIRECTS: usize = 20;

/// Default maximum streaming download size (`256 MiB`).
pub const DEFAULT_DOWNLOAD_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum permitted streaming download limit (`4 GiB`).
pub const MAX_ALLOWED_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Chunk size used when streaming downloads to disk (`64 KiB`).
pub const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// Default `User-Agent` header value sent when no override is specified.
pub const DEFAULT_USER_AGENT: &str = concat!("vaeljs/", env!("CARGO_PKG_VERSION"));

/// Request body payload for [`crate::fetch_sync`].
#[derive(Clone, Debug, Default, PartialEq)]
pub enum FetchBody {
    /// No request body.
    #[default]
    None,
    /// Raw binary body bytes.
    Bytes(Vec<u8>),
    /// UTF-8 text body.
    Text(String),
    /// JSON value serialized with `serde_json`; sets
    /// `Content-Type: application/json; charset=utf-8` if absent.
    Json(serde_json::Value),
}

/// Options controlling [`crate::fetch_sync`].
#[derive(Clone)]
pub struct FetchOptions {
    /// HTTP method (`GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`).
    /// Case-normalized to uppercase during validation. Default: `"GET"`.
    pub method: String,
    /// Additional HTTP request headers as `(name, value)` pairs.
    pub headers: Vec<(String, String)>,
    /// Query parameters safely encoded and appended via `url::Url::query_pairs_mut()`.
    pub query: Vec<(String, String)>,
    /// Optional bearer token sent as `Authorization: Bearer <token>` if no
    /// case-insensitive `authorization` header is already present.
    /// Redacted in `Debug` output and zeroized on drop.
    pub bearer_token: Option<Zeroizing<String>>,
    /// Request body payload. Default: [`FetchBody::None`].
    pub body: FetchBody,
    /// Overall request deadline across redirects and body read.
    /// Default: `30s`; must be `> 0` and `<= 600s`.
    pub timeout: Duration,
    /// Maximum response body size in bytes.
    /// Default: `16 MiB`; must be `> 0` and `<= 256 MiB`.
    pub max_response_bytes: usize,
    /// Maximum number of HTTP redirects to follow. Default: `10`; max: `20`.
    pub max_redirects: usize,
    /// Allow connections to private, loopback, link-local, or metadata IPs.
    /// Default: `false` (SSRF protection active).
    pub allow_private_ips: bool,
    /// Optional `User-Agent` header. Default: `Some("vaeljs/0.0.0-alpha.1")`.
    pub user_agent: Option<String>,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            method: "GET".to_owned(),
            headers: Vec::new(),
            query: Vec::new(),
            bearer_token: None,
            body: FetchBody::None,
            timeout: DEFAULT_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            allow_private_ips: false,
            user_agent: Some(DEFAULT_USER_AGENT.to_owned()),
        }
    }
}

impl FetchOptions {
    /// Creates a new [`FetchOptions`] with secure defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for FetchOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchOptions")
            .field("method", &self.method)
            .field("headers", &RedactedHeaders(&self.headers))
            .field("query", &RedactedQuery(&self.query))
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("body", &self.body)
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_redirects", &self.max_redirects)
            .field("allow_private_ips", &self.allow_private_ips)
            .field("user_agent", &self.user_agent)
            .finish()
    }
}

/// Bundled target URL and [`FetchOptions`] for [`crate::fetch_request_sync`].
#[derive(Clone, Debug)]
pub struct FetchRequest {
    /// Target URL (`http:` or `https:` without userinfo).
    pub url: String,
    /// Request options.
    pub options: FetchOptions,
}

impl FetchRequest {
    /// Creates a new [`FetchRequest`] for `url` and `options`.
    #[must_use]
    pub fn new(url: impl Into<String>, options: FetchOptions) -> Self {
        Self {
            url: url.into(),
            options,
        }
    }
}

/// Response returned by [`crate::fetch_sync`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FetchResponse {
    /// HTTP status code.
    pub status: u16,
    /// HTTP status reason phrase.
    pub status_text: String,
    /// `true` when `status` is in `200..=299`.
    pub ok: bool,
    /// Final URL after following any redirects (without fragment).
    pub url: String,
    /// `true` if at least one redirect hop was followed.
    pub redirected: bool,
    /// Response headers as `(name, value)` pairs.
    pub headers: Vec<(String, String)>,
    /// Response body bytes (bounded by `max_response_bytes`).
    pub body: Vec<u8>,
}

impl FetchResponse {
    /// Returns the first header value matching `name` case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Returns all header values matching `name` case-insensitively.
    #[must_use]
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// Decodes [`Self::body`] as UTF-8 text.
    ///
    /// # Errors
    ///
    /// Returns [`FetchError::InvalidUtf8`] if the body contains invalid UTF-8.
    pub fn text(&self) -> Result<&str, FetchError> {
        std::str::from_utf8(&self.body).map_err(|_| FetchError::InvalidUtf8)
    }

    /// Parses [`Self::body`] as JSON into [`serde_json::Value`].
    ///
    /// # Errors
    ///
    /// Returns [`FetchError::InvalidJson`] if the body is not valid JSON.
    pub fn json_value(&self) -> Result<serde_json::Value, FetchError> {
        serde_json::from_slice(&self.body).map_err(|err| FetchError::InvalidJson {
            reason: err.to_string(),
        })
    }
}

/// Options controlling [`crate::download_sync`].
#[derive(Clone)]
pub struct DownloadOptions {
    /// Additional HTTP request headers as `(name, value)` pairs.
    pub headers: Vec<(String, String)>,
    /// Query parameters safely encoded and appended via `url::Url::query_pairs_mut()`.
    pub query: Vec<(String, String)>,
    /// Optional bearer token sent as `Authorization: Bearer <token>` if no
    /// case-insensitive `authorization` header is already present.
    /// Redacted in `Debug` output and zeroized on drop.
    pub bearer_token: Option<Zeroizing<String>>,
    /// Overall download deadline across redirects and disk streaming.
    /// Default: `30s`; must be `> 0` and `<= 600s`.
    pub timeout: Duration,
    /// Maximum number of HTTP redirects to follow. Default: `10`; max: `20`.
    pub max_redirects: usize,
    /// Allow connections to private, loopback, link-local, or metadata IPs.
    /// Default: `false` (SSRF protection active).
    pub allow_private_ips: bool,
    /// Optional `User-Agent` header. Default: `Some("vaeljs/0.0.0-alpha.1")`.
    pub user_agent: Option<String>,
    /// Maximum bytes allowed to be written to disk.
    /// Default: `256 MiB`; must be `> 0` and `<= 4 GiB`.
    pub max_bytes: u64,
    /// Optional expected SHA-256 hex digest (64 ASCII hex chars, validated up front).
    pub expected_sha256: Option<String>,
    /// Whether an existing destination file may be atomically replaced.
    /// Default: `false` (fails with [`crate::FetchErrorCode::AlreadyExists`]).
    pub overwrite: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            headers: Vec::new(),
            query: Vec::new(),
            bearer_token: None,
            timeout: DEFAULT_TIMEOUT,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            allow_private_ips: false,
            user_agent: Some(DEFAULT_USER_AGENT.to_owned()),
            max_bytes: DEFAULT_DOWNLOAD_MAX_BYTES,
            expected_sha256: None,
            overwrite: false,
        }
    }
}

impl DownloadOptions {
    /// Creates a new [`DownloadOptions`] with secure defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for DownloadOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloadOptions")
            .field("headers", &RedactedHeaders(&self.headers))
            .field("query", &RedactedQuery(&self.query))
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("timeout", &self.timeout)
            .field("max_redirects", &self.max_redirects)
            .field("allow_private_ips", &self.allow_private_ips)
            .field("user_agent", &self.user_agent)
            .field("max_bytes", &self.max_bytes)
            .field("expected_sha256", &self.expected_sha256)
            .field("overwrite", &self.overwrite)
            .finish()
    }
}

/// Incremental progress snapshot passed to the [`crate::download_sync`] callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadProgress {
    /// Total bytes written to the temporary file so far.
    pub bytes_written: u64,
    /// Declared `Content-Length` from the server, if present and valid.
    pub total_bytes: Option<u64>,
}

/// Metadata returned upon completion of [`crate::download_sync`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadResult {
    /// Destination path written atomically.
    pub path: PathBuf,
    /// HTTP status code (`200..=299`).
    pub status: u16,
    /// Final URL after following any redirects (without fragment).
    pub url: String,
    /// Total bytes written to `path`.
    pub bytes_written: u64,
    /// Lowercase hexadecimal SHA-256 digest of the downloaded bytes.
    pub sha256: String,
    /// Response `Content-Type` header, if present.
    pub content_type: Option<String>,
    /// Response `ETag` header, if present.
    pub etag: Option<String>,
}

struct RedactedHeaders<'a>(&'a [(String, String)]);

impl fmt::Debug for RedactedHeaders<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();
        for (name, value) in self.0 {
            if is_sensitive_header_name(name) {
                list.entry(&(name.as_str(), "[REDACTED]"));
            } else {
                list.entry(&(name.as_str(), value.as_str()));
            }
        }
        list.finish()
    }
}

struct RedactedQuery<'a>(&'a [(String, String)]);

impl fmt::Debug for RedactedQuery<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();
        for (key, _) in self.0 {
            list.entry(&(key.as_str(), "[REDACTED]"));
        }
        list.finish()
    }
}
