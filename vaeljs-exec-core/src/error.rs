//! Typed error codes and error values for `vael:exec`.
//!
//! Error messages describe the programmatic and human cause without echoing
//! caller-supplied paths, command arguments, or environment variable values.

use std::fmt;
use std::io;
use std::time::Duration;

use thiserror::Error;

/// Stable machine-readable error codes for [`ExecError`].
///
/// Bindings expose [`ExecErrorCode::as_str`] as the JavaScript error's `code`
/// property so callers match on codes rather than parsing error strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum ExecErrorCode {
    /// Target executable or lock file path was not found.
    NotFound,
    /// Operating system denied permission for the requested operation.
    PermissionDenied,
    /// Caller supplied an empty, NUL-containing, out-of-range, or invalid argument.
    InvalidArgument,
    /// Child process or wait operation exceeded its configured timeout.
    Timeout,
    /// Captured child output exceeded the configured byte limit.
    OutputTooLarge,
    /// Requested signal cannot be delivered on the current platform.
    UnsupportedSignal,
    /// Non-blocking file lock acquisition failed because a conflicting lock is held.
    LockWouldBlock,
    /// Bounded file lock acquisition timed out before the lock became available.
    LockTimeout,
    /// Underlying operating-system I/O error.
    Io,
}

impl ExecErrorCode {
    /// Returns the stable `VAEL_EXEC_*` error code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "VAEL_EXEC_NOT_FOUND",
            Self::PermissionDenied => "VAEL_EXEC_PERMISSION_DENIED",
            Self::InvalidArgument => "VAEL_EXEC_INVALID_ARGUMENT",
            Self::Timeout => "VAEL_EXEC_TIMEOUT",
            Self::OutputTooLarge => "VAEL_EXEC_OUTPUT_TOO_LARGE",
            Self::UnsupportedSignal => "VAEL_EXEC_UNSUPPORTED_SIGNAL",
            Self::LockWouldBlock => "VAEL_EXEC_LOCK_WOULD_BLOCK",
            Self::LockTimeout => "VAEL_EXEC_LOCK_TIMEOUT",
            Self::Io => "VAEL_EXEC_IO",
        }
    }
}

impl fmt::Display for ExecErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed error returned by every fallible operation in [`crate`].
///
/// Each variant maps deterministically to a stable [`ExecErrorCode`] via
/// [`ExecError::code`] and never echoes raw paths, arguments, or secrets.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExecError {
    /// Target executable or path was not found.
    #[error("executable or path not found: {source}")]
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
    /// Caller supplied an invalid argument or option.
    #[error("invalid argument: {reason}")]
    InvalidArgument {
        /// Static description of the validation failure.
        reason: &'static str,
    },
    /// Operation timed out after the configured duration.
    #[error("operation timed out after {elapsed:?}")]
    Timeout {
        /// Configured timeout duration.
        elapsed: Duration,
    },
    /// Child output exceeded the configured byte limit.
    #[error("{stream} output exceeded limit of {limit} bytes")]
    OutputTooLarge {
        /// Configured byte limit.
        limit: usize,
        /// Stream name (`"stdout"` or `"stderr"`).
        stream: &'static str,
    },
    /// Signal is unsupported on the current platform.
    #[error("unsupported signal on this platform: {signal}")]
    UnsupportedSignal {
        /// Canonical signal name.
        signal: &'static str,
    },
    /// Non-blocking file lock failed because a conflicting lock is held.
    #[error("file lock is held by another lock owner")]
    LockWouldBlock,
    /// Timed file lock acquisition expired without acquiring the lock.
    #[error("timed out waiting for file lock")]
    LockTimeout,
    /// Operating system I/O failure.
    #[error("exec I/O error during {context}: {source}")]
    Io {
        /// Static operation context.
        context: &'static str,
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
}

impl ExecError {
    /// Returns the stable machine-readable [`ExecErrorCode`] for this error.
    #[must_use]
    pub const fn code(&self) -> ExecErrorCode {
        match self {
            Self::NotFound { .. } => ExecErrorCode::NotFound,
            Self::PermissionDenied { .. } => ExecErrorCode::PermissionDenied,
            Self::InvalidArgument { .. } => ExecErrorCode::InvalidArgument,
            Self::Timeout { .. } => ExecErrorCode::Timeout,
            Self::OutputTooLarge { .. } => ExecErrorCode::OutputTooLarge,
            Self::UnsupportedSignal { .. } => ExecErrorCode::UnsupportedSignal,
            Self::LockWouldBlock => ExecErrorCode::LockWouldBlock,
            Self::LockTimeout => ExecErrorCode::LockTimeout,
            Self::Io { .. } => ExecErrorCode::Io,
        }
    }
}

pub(crate) fn map_io_error(context: &'static str, err: io::Error) -> ExecError {
    match err.kind() {
        io::ErrorKind::NotFound => ExecError::NotFound { source: err },
        io::ErrorKind::PermissionDenied => ExecError::PermissionDenied { source: err },
        _ => ExecError::Io {
            context,
            source: err,
        },
    }
}
