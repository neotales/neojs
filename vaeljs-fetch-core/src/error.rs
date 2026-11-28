//! Structured error codes and redacted error types for `vaeljs-fetch-core`.
//!
//! Error messages and `Debug` representations never include URL query strings,
//! URL userinfo, bearer tokens, `Authorization` values, `Cookie` values, or
//! caller-supplied filesystem paths.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Machine-readable error classification for [`FetchError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum FetchErrorCode {
    /// The supplied URL or redirect target is syntactically invalid, uses an
    /// unsupported scheme, carries embedded userinfo, or lacks a host.
    InvalidUrl,
    /// One or more request or download options failed validation.
    InvalidOption,
    /// The target host or resolved IP address is blocked by the SSRF guard.
    SsrfBlocked,
    /// DNS resolution exceeded the bounded lookup timeout.
    DnsTimeout,
    /// The overall fetch or download deadline expired.
    Timeout,
    /// Redirect traversal exceeded `max_redirects`.
    TooManyRedirects,
    /// Response or download body exceeded the configured byte budget.
    BodyTooLarge,
    /// Download failed because the server returned a non-2xx HTTP status.
    HttpStatus,
    /// Downloaded bytes did not match `expected_sha256`.
    ChecksumMismatch,
    /// Destination path already exists and `overwrite` is `false`.
    AlreadyExists,
    /// Transport-level network failure (connection refused, TLS, DNS, etc.).
    Network,
    /// Local filesystem I/O failure during download.
    Io,
    /// Response body is not valid UTF-8.
    InvalidUtf8,
    /// Response body is not valid JSON.
    InvalidJson,
}

impl FetchErrorCode {
    /// Returns the stable `VAEL_FETCH_*` error code identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "VAEL_FETCH_INVALID_URL",
            Self::InvalidOption => "VAEL_FETCH_INVALID_OPTION",
            Self::SsrfBlocked => "VAEL_FETCH_SSRF_BLOCKED",
            Self::DnsTimeout => "VAEL_FETCH_DNS_TIMEOUT",
            Self::Timeout => "VAEL_FETCH_TIMEOUT",
            Self::TooManyRedirects => "VAEL_FETCH_TOO_MANY_REDIRECTS",
            Self::BodyTooLarge => "VAEL_FETCH_BODY_TOO_LARGE",
            Self::HttpStatus => "VAEL_FETCH_HTTP_STATUS",
            Self::ChecksumMismatch => "VAEL_FETCH_CHECKSUM_MISMATCH",
            Self::AlreadyExists => "VAEL_FETCH_ALREADY_EXISTS",
            Self::Network => "VAEL_FETCH_NETWORK",
            Self::Io => "VAEL_FETCH_IO",
            Self::InvalidUtf8 => "VAEL_FETCH_INVALID_UTF8",
            Self::InvalidJson => "VAEL_FETCH_INVALID_JSON",
        }
    }
}

impl fmt::Display for FetchErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Redacted, typed error returned by `vaeljs-fetch-core` operations.
///
/// Every `url` stored inside an error variant is sanitized via URL redaction
/// (userinfo, query string, and fragment removed) before constructing the error.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[non_exhaustive]
pub enum FetchError {
    /// URL parsing or structural validation failed.
    #[error("invalid URL: {reason}")]
    InvalidUrl {
        /// Safe description of why the URL was rejected.
        reason: String,
    },

    /// Request or download option failed validation.
    #[error("invalid option: {reason}")]
    InvalidOption {
        /// Safe description of the invalid option.
        reason: &'static str,
    },

    /// Connection blocked by the SSRF guard.
    #[error("connection to {url} blocked by SSRF policy")]
    SsrfBlocked {
        /// Redacted target URL (`scheme://host[:port]/path`).
        url: String,
    },

    /// DNS resolution exceeded the bounded timeout.
    #[error("DNS resolution timed out for {url}")]
    DnsTimeout {
        /// Redacted target URL.
        url: String,
    },

    /// The request or download deadline expired.
    #[error("request timed out for {url}")]
    Timeout {
        /// Redacted target URL.
        url: String,
    },

    /// Redirect chain exceeded `max_redirects`.
    #[error("too many redirects for {url} (limit is {limit})")]
    TooManyRedirects {
        /// Redacted target URL.
        url: String,
        /// Configured redirect limit.
        limit: usize,
    },

    /// Declared or streamed body exceeded the configured byte limit.
    #[error("response body from {url} exceeded the {limit} byte limit")]
    BodyTooLarge {
        /// Redacted target URL.
        url: String,
        /// Configured byte limit.
        limit: u64,
    },

    /// Server returned a non-2xx status code during `download_sync`.
    #[error("HTTP {status} from {url}")]
    HttpStatus {
        /// HTTP status code returned by the server.
        status: u16,
        /// Redacted target URL.
        url: String,
    },

    /// Downloaded payload SHA-256 digest did not match `expected_sha256`.
    #[error("SHA-256 checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Normalized lowercase hex expected digest.
        expected: String,
        /// Computed lowercase hex digest.
        actual: String,
    },

    /// Destination path already exists and `overwrite` is `false`.
    #[error("destination path already exists")]
    AlreadyExists,

    /// Network or HTTP transport failure.
    #[error("network error for {url}: {reason}")]
    Network {
        /// Redacted target URL.
        url: String,
        /// Fixed transport error category.
        reason: &'static str,
    },

    /// Local filesystem I/O error.
    #[error("I/O error: {reason}")]
    Io {
        /// Safe OS error description.
        reason: String,
    },

    /// Response body was not valid UTF-8.
    #[error("response body is not valid UTF-8")]
    InvalidUtf8,

    /// Response body could not be parsed as JSON.
    #[error("response body is not valid JSON: {reason}")]
    InvalidJson {
        /// JSON parser diagnostic.
        reason: String,
    },
}

impl FetchError {
    /// Returns the machine-readable [`FetchErrorCode`] for this error.
    #[must_use]
    pub const fn code(&self) -> FetchErrorCode {
        match self {
            Self::InvalidUrl { .. } => FetchErrorCode::InvalidUrl,
            Self::InvalidOption { .. } => FetchErrorCode::InvalidOption,
            Self::SsrfBlocked { .. } => FetchErrorCode::SsrfBlocked,
            Self::DnsTimeout { .. } => FetchErrorCode::DnsTimeout,
            Self::Timeout { .. } => FetchErrorCode::Timeout,
            Self::TooManyRedirects { .. } => FetchErrorCode::TooManyRedirects,
            Self::BodyTooLarge { .. } => FetchErrorCode::BodyTooLarge,
            Self::HttpStatus { .. } => FetchErrorCode::HttpStatus,
            Self::ChecksumMismatch { .. } => FetchErrorCode::ChecksumMismatch,
            Self::AlreadyExists => FetchErrorCode::AlreadyExists,
            Self::Network { .. } => FetchErrorCode::Network,
            Self::Io { .. } => FetchErrorCode::Io,
            Self::InvalidUtf8 => FetchErrorCode::InvalidUtf8,
            Self::InvalidJson { .. } => FetchErrorCode::InvalidJson,
        }
    }

    /// Returns the redacted target URL associated with this error, if applicable.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        match self {
            Self::SsrfBlocked { url }
            | Self::DnsTimeout { url }
            | Self::Timeout { url }
            | Self::TooManyRedirects { url, .. }
            | Self::BodyTooLarge { url, .. }
            | Self::HttpStatus { url, .. }
            | Self::Network { url, .. } => Some(url.as_str()),
            Self::InvalidUrl { .. }
            | Self::InvalidOption { .. }
            | Self::ChecksumMismatch { .. }
            | Self::AlreadyExists
            | Self::Io { .. }
            | Self::InvalidUtf8
            | Self::InvalidJson { .. } => None,
        }
    }
}
