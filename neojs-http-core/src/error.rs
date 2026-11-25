//! Typed error codes and error values for `neojs-http-core`.
//!
//! Error display messages describe the programmatic cause without echoing
//! caller-supplied filesystem paths, header values, or cookie values.

use std::fmt;
use std::io;

use thiserror::Error;

/// Stable machine-readable error codes for [`HttpCoreError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum HttpCoreErrorCode {
    /// Route pattern syntax is invalid or contains duplicate parameters.
    InvalidPattern,
    /// Route pattern conflicts with an already-registered route.
    RouteConflict,
    /// Requested static file or directory index was not found.
    NotFound,
    /// Supplied URL path or index filename failed safety validation.
    InvalidPath,
    /// HTTP header name or value failed validation.
    InvalidHeader,
    /// Cookie header, name, value, or attribute failed validation or exceeded bounds.
    InvalidCookie,
    /// Operating-system I/O error during static file resolution.
    Io,
}

impl HttpCoreErrorCode {
    /// Returns the stable `NEO_HTTP_*` error code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidPattern => "NEO_HTTP_INVALID_PATTERN",
            Self::RouteConflict => "NEO_HTTP_ROUTE_CONFLICT",
            Self::NotFound => "NEO_HTTP_NOT_FOUND",
            Self::InvalidPath => "NEO_HTTP_INVALID_PATH",
            Self::InvalidHeader => "NEO_HTTP_INVALID_HEADER",
            Self::InvalidCookie => "NEO_HTTP_INVALID_COOKIE",
            Self::Io => "NEO_HTTP_IO",
        }
    }
}

impl fmt::Display for HttpCoreErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed error returned by fallible operations in `neojs-http-core`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HttpCoreError {
    /// Route pattern failed syntax or parameter validation.
    #[error("invalid route pattern: {reason}")]
    InvalidPattern {
        /// Human-readable reason the pattern was rejected.
        reason: &'static str,
    },
    /// Route pattern conflicts with an already-registered route.
    #[error("conflicting route pattern: {reason}")]
    RouteConflict {
        /// Human-readable reason the route conflicted.
        reason: &'static str,
    },
    /// Requested static file or index file does not exist.
    #[error("static file not found")]
    NotFound,
    /// Static file URL path or index name failed security validation.
    #[error("invalid static file path: {reason}")]
    InvalidPath {
        /// Human-readable reason the path was rejected.
        reason: &'static str,
    },
    /// HTTP header name or value failed RFC validation.
    #[error("invalid HTTP header: {reason}")]
    InvalidHeader {
        /// Human-readable reason the header was rejected.
        reason: &'static str,
    },
    /// Cookie header, name, value, or attribute failed validation or count bounds.
    #[error("invalid HTTP cookie: {reason}")]
    InvalidCookie {
        /// Human-readable reason the cookie was rejected.
        reason: &'static str,
    },
    /// Operating-system I/O failure while resolving a static file.
    #[error("static file I/O error: {source}")]
    Io {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
}

impl HttpCoreError {
    /// Returns the stable [`HttpCoreErrorCode`] for this error.
    #[must_use]
    pub const fn code(&self) -> HttpCoreErrorCode {
        match self {
            Self::InvalidPattern { .. } => HttpCoreErrorCode::InvalidPattern,
            Self::RouteConflict { .. } => HttpCoreErrorCode::RouteConflict,
            Self::NotFound => HttpCoreErrorCode::NotFound,
            Self::InvalidPath { .. } => HttpCoreErrorCode::InvalidPath,
            Self::InvalidHeader { .. } => HttpCoreErrorCode::InvalidHeader,
            Self::InvalidCookie { .. } => HttpCoreErrorCode::InvalidCookie,
            Self::Io { .. } => HttpCoreErrorCode::Io,
        }
    }
}

pub(crate) fn map_io_error(err: io::Error) -> HttpCoreError {
    match err.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => HttpCoreError::NotFound,
        _ => HttpCoreError::Io { source: err },
    }
}
