//! Typed filesystem error codes and error values for `vael:fs`.
//!
//! Error messages describe the programmatic and human cause without echoing
//! caller-supplied filesystem paths, keeping internal directory layouts out of
//! logs and propagated error strings.

use std::fmt;
use std::io;

use thiserror::Error;

/// Stable machine-readable error codes for [`FsError`].
///
/// Bindings expose [`FsErrorCode::as_str`] as the JavaScript error's `code`
/// property. Callers match on the variant or code string rather than parsing
/// human-readable error messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum FsErrorCode {
    /// Target file, directory, or path component does not exist.
    NotFound,
    /// Target entry already exists when exclusive creation was requested.
    AlreadyExists,
    /// Operating system denied permission for the operation.
    PermissionDenied,
    /// Target is a directory where a non-directory was required.
    IsADirectory,
    /// A path component is not a directory where a directory was required.
    NotADirectory,
    /// Directory is not empty and recursive removal was not enabled.
    DirectoryNotEmpty,
    /// Supplied path is empty, contains a NUL byte, or is otherwise invalid.
    InvalidPath,
    /// Supplied options or arguments are invalid or contradictory.
    InvalidArgument,
    /// File contents or a directory entry name are not valid UTF-8.
    InvalidUtf8,
    /// File size exceeds the configured `max_bytes` read limit.
    TooLarge,
    /// Other operating-system I/O or CSPRNG failure.
    Io,
}

impl FsErrorCode {
    /// Returns the stable `VAEL_FS_*` error code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "VAEL_FS_NOT_FOUND",
            Self::AlreadyExists => "VAEL_FS_ALREADY_EXISTS",
            Self::PermissionDenied => "VAEL_FS_PERMISSION_DENIED",
            Self::IsADirectory => "VAEL_FS_IS_A_DIRECTORY",
            Self::NotADirectory => "VAEL_FS_NOT_A_DIRECTORY",
            Self::DirectoryNotEmpty => "VAEL_FS_DIRECTORY_NOT_EMPTY",
            Self::InvalidPath => "VAEL_FS_INVALID_PATH",
            Self::InvalidArgument => "VAEL_FS_INVALID_ARGUMENT",
            Self::InvalidUtf8 => "VAEL_FS_INVALID_UTF8",
            Self::TooLarge => "VAEL_FS_TOO_LARGE",
            Self::Io => "VAEL_FS_IO",
        }
    }
}

impl fmt::Display for FsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed error returned by every fallible operation in [`crate`].
///
/// Each variant maps deterministically to a stable [`FsErrorCode`] via
/// [`FsError::code`], preserves any underlying [`std::io::Error`] via
/// [`std::error::Error::source`], and avoids echoing raw paths in its display
/// string.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FsError {
    /// Target file, directory, or parent path was not found.
    #[error("filesystem entry not found: {source}")]
    NotFound {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// Target path already exists and overwriting was not permitted.
    #[error("filesystem entry already exists: {source}")]
    AlreadyExists {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// Operating system denied permission for the requested operation.
    #[error("permission denied: {source}")]
    PermissionDenied {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// Target is a directory where a file or non-directory was expected.
    #[error("target path is a directory: {source}")]
    IsADirectory {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// A path component is not a directory where a directory was expected.
    #[error("path component is not a directory: {source}")]
    NotADirectory {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// Directory is not empty and `recursive` was `false`.
    #[error("directory is not empty: {source}")]
    DirectoryNotEmpty {
        /// Underlying OS error.
        #[source]
        source: io::Error,
    },
    /// Input path failed validation or was rejected by the OS as invalid.
    #[error("invalid filesystem path: {reason}")]
    InvalidPath {
        /// Human-readable reason the path was rejected.
        reason: &'static str,
        /// Underlying OS error when reported by the operating system.
        #[source]
        source: Option<io::Error>,
    },
    /// Options or arguments passed to an operation are invalid or contradictory.
    #[error("invalid argument: {reason}")]
    InvalidArgument {
        /// Human-readable reason the argument combination was rejected.
        reason: &'static str,
    },
    /// File bytes or a directory entry name could not be decoded as valid UTF-8.
    #[error("invalid UTF-8: {reason}")]
    InvalidUtf8 {
        /// Human-readable description of the UTF-8 validation failure.
        reason: &'static str,
    },
    /// File size exceeds the configured `max_bytes` limit.
    #[error("file exceeds maximum size limit of {limit} bytes")]
    TooLarge {
        /// Configured byte limit.
        limit: u64,
        /// Observed size from file metadata when available before streaming.
        actual: Option<u64>,
    },
    /// Any other operating-system I/O or entropy failure.
    #[error("filesystem I/O error: {reason}")]
    Io {
        /// Human-readable failure description.
        reason: String,
        /// Underlying OS error when available.
        #[source]
        source: Option<io::Error>,
    },
}

impl FsError {
    /// Returns the stable [`FsErrorCode`] for this error.
    #[must_use]
    pub const fn code(&self) -> FsErrorCode {
        match self {
            Self::NotFound { .. } => FsErrorCode::NotFound,
            Self::AlreadyExists { .. } => FsErrorCode::AlreadyExists,
            Self::PermissionDenied { .. } => FsErrorCode::PermissionDenied,
            Self::IsADirectory { .. } => FsErrorCode::IsADirectory,
            Self::NotADirectory { .. } => FsErrorCode::NotADirectory,
            Self::DirectoryNotEmpty { .. } => FsErrorCode::DirectoryNotEmpty,
            Self::InvalidPath { .. } => FsErrorCode::InvalidPath,
            Self::InvalidArgument { .. } => FsErrorCode::InvalidArgument,
            Self::InvalidUtf8 { .. } => FsErrorCode::InvalidUtf8,
            Self::TooLarge { .. } => FsErrorCode::TooLarge,
            Self::Io { .. } => FsErrorCode::Io,
        }
    }
}

pub(crate) fn map_io_error(err: io::Error) -> FsError {
    match err.kind() {
        io::ErrorKind::NotFound => FsError::NotFound { source: err },
        io::ErrorKind::AlreadyExists => FsError::AlreadyExists { source: err },
        io::ErrorKind::PermissionDenied => FsError::PermissionDenied { source: err },
        io::ErrorKind::IsADirectory => FsError::IsADirectory { source: err },
        io::ErrorKind::NotADirectory => FsError::NotADirectory { source: err },
        io::ErrorKind::DirectoryNotEmpty => FsError::DirectoryNotEmpty { source: err },
        io::ErrorKind::InvalidFilename => FsError::InvalidPath {
            reason: "invalid filename or path component",
            source: Some(err),
        },
        _ => FsError::Io {
            reason: err.to_string(),
            source: Some(err),
        },
    }
}

pub(crate) fn map_getrandom_error(err: getrandom::Error) -> FsError {
    let reason = if let Some(errno) = err.raw_os_error() {
        format!("CSPRNG failed while generating temporary file name (OS error {errno})")
    } else if err == getrandom::Error::UNSUPPORTED {
        "CSPRNG is unsupported on this platform".to_owned()
    } else if err == getrandom::Error::ERRNO_NOT_POSITIVE {
        "CSPRNG failed with non-positive OS error".to_owned()
    } else if err == getrandom::Error::UNEXPECTED {
        "CSPRNG failed unexpectedly".to_owned()
    } else {
        "CSPRNG failed with internal error".to_owned()
    };

    FsError::Io {
        reason,
        source: None,
    }
}
