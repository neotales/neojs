//! Typed error codes and error values for `neojs-node-core`.
//!
//! Error messages describe the programmatic cause without echoing
//! caller-supplied paths, environment values, URLs, or raw buffers.

use std::fmt;
use std::io;

use neojs_fs_core::{FsError, FsErrorCode};
use thiserror::Error;

/// Stable machine-readable error codes for [`NodeCoreError`].
///
/// Runtime bindings expose [`NodeCoreErrorCode::as_str`] on JavaScript errors
/// so callers match on stable codes rather than parsing message strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum NodeCoreErrorCode {
    /// Caller supplied an empty, NUL-containing, or structurally invalid argument.
    InvalidInput,
    /// Input exceeded a configured size or aggregate length bound.
    InputTooLarge,
    /// Supplied path is invalid or lacks a required root/base.
    InvalidPath,
    /// Supplied URL is malformed, uses a non-`file:` scheme, or violates host/separator rules.
    InvalidUrl,
    /// Input string or byte sequence could not be decoded under the requested encoding.
    InvalidEncoding,
    /// Target file, directory, or executable was not found.
    NotFound,
    /// Operating system denied permission for the operation.
    PermissionDenied,
    /// Underlying operating-system I/O error.
    Io,
    /// Process spawning, waiting, or stdio piping failed.
    ProcessError,
    /// Value or host property is not valid UTF-8.
    InvalidUtf8,
    /// Requested host property is unavailable in the current environment.
    Unavailable,
    /// Underlying [`FsError`] forwarded from `neojs-fs-core`.
    Fs,
}

impl NodeCoreErrorCode {
    /// Returns the stable `NEO_NODE_*` error code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "NEO_NODE_INVALID_INPUT",
            Self::InputTooLarge => "NEO_NODE_INPUT_TOO_LARGE",
            Self::InvalidPath => "NEO_NODE_INVALID_PATH",
            Self::InvalidUrl => "NEO_NODE_INVALID_URL",
            Self::InvalidEncoding => "NEO_NODE_INVALID_ENCODING",
            Self::NotFound => "NEO_NODE_NOT_FOUND",
            Self::PermissionDenied => "NEO_NODE_PERMISSION_DENIED",
            Self::Io => "NEO_NODE_IO",
            Self::ProcessError => "NEO_NODE_PROCESS_ERROR",
            Self::InvalidUtf8 => "NEO_NODE_INVALID_UTF8",
            Self::Unavailable => "NEO_NODE_UNAVAILABLE",
            Self::Fs => "NEO_NODE_FS",
        }
    }
}

impl fmt::Display for NodeCoreErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed error returned by fallible operations in `neojs-node-core`.
///
/// Each variant maps deterministically to a [`NodeCoreErrorCode`] via
/// [`NodeCoreError::code`] and avoids echoing raw paths, URLs, or secrets.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NodeCoreError {
    /// Caller supplied an invalid argument.
    #[error("invalid input: {reason}")]
    InvalidInput {
        /// Human-readable reason the input was rejected.
        reason: &'static str,
    },
    /// Input exceeded a safety size limit.
    #[error("input exceeds maximum length of {limit} bytes")]
    InputTooLarge {
        /// Maximum allowed byte length.
        limit: usize,
        /// Observed byte length when known.
        actual: Option<usize>,
    },
    /// Supplied path is invalid for the operation.
    #[error("invalid path: {reason}")]
    InvalidPath {
        /// Human-readable reason the path was rejected.
        reason: &'static str,
    },
    /// Supplied URL is invalid or disallowed.
    #[error("invalid file URL: {reason}")]
    InvalidUrl {
        /// Human-readable reason the URL was rejected.
        reason: &'static str,
    },
    /// Supplied string could not be decoded with the requested buffer encoding.
    #[error("invalid encoded data: {reason}")]
    InvalidEncoding {
        /// Human-readable reason decoding failed.
        reason: &'static str,
    },
    /// Target entry or executable was not found.
    #[error("target not found: {source}")]
    NotFound {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// Operating system denied permission.
    #[error("permission denied: {source}")]
    PermissionDenied {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// General operating-system I/O error.
    #[error("I/O error ({context}): {source}")]
    Io {
        /// Short operation context without raw paths or secrets.
        context: &'static str,
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// Child process error not directly represented by `NotFound` or `PermissionDenied`.
    #[error("process error ({context}): {reason}")]
    ProcessError {
        /// Short operation context.
        context: &'static str,
        /// Failure detail.
        reason: String,
        /// Underlying OS error if available.
        #[source]
        source: Option<io::Error>,
    },
    /// Host string or environment value is not valid UTF-8.
    #[error("invalid UTF-8: {reason}")]
    InvalidUtf8 {
        /// Human-readable description.
        reason: &'static str,
    },
    /// Requested host metadata could not be determined.
    #[error("host information unavailable: {reason}")]
    Unavailable {
        /// Human-readable description.
        reason: &'static str,
    },
    /// Error forwarded from `neojs-fs-core`.
    #[error(transparent)]
    Fs(#[from] FsError),
}

impl NodeCoreError {
    /// Returns the stable [`NodeCoreErrorCode`] for this error.
    #[must_use]
    pub const fn code(&self) -> NodeCoreErrorCode {
        match self {
            Self::InvalidInput { .. } => NodeCoreErrorCode::InvalidInput,
            Self::InputTooLarge { .. } => NodeCoreErrorCode::InputTooLarge,
            Self::InvalidPath { .. } => NodeCoreErrorCode::InvalidPath,
            Self::InvalidUrl { .. } => NodeCoreErrorCode::InvalidUrl,
            Self::InvalidEncoding { .. } => NodeCoreErrorCode::InvalidEncoding,
            Self::NotFound { .. } => NodeCoreErrorCode::NotFound,
            Self::PermissionDenied { .. } => NodeCoreErrorCode::PermissionDenied,
            Self::Io { .. } => NodeCoreErrorCode::Io,
            Self::ProcessError { .. } => NodeCoreErrorCode::ProcessError,
            Self::InvalidUtf8 { .. } => NodeCoreErrorCode::InvalidUtf8,
            Self::Unavailable { .. } => NodeCoreErrorCode::Unavailable,
            Self::Fs(err) => match err.code() {
                FsErrorCode::NotFound => NodeCoreErrorCode::NotFound,
                FsErrorCode::PermissionDenied => NodeCoreErrorCode::PermissionDenied,
                FsErrorCode::InvalidPath => NodeCoreErrorCode::InvalidPath,
                FsErrorCode::InvalidArgument => NodeCoreErrorCode::InvalidInput,
                FsErrorCode::InvalidUtf8 => NodeCoreErrorCode::InvalidUtf8,
                FsErrorCode::TooLarge => NodeCoreErrorCode::InputTooLarge,
                _ => NodeCoreErrorCode::Fs,
            },
        }
    }
}

pub(crate) fn map_io_error(context: &'static str, err: io::Error) -> NodeCoreError {
    match err.kind() {
        io::ErrorKind::NotFound => NodeCoreError::NotFound { source: err },
        io::ErrorKind::PermissionDenied => NodeCoreError::PermissionDenied { source: err },
        _ => NodeCoreError::Io {
            context,
            source: err,
        },
    }
}
