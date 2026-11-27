//! Shared cross-runtime helpers and embedded JS module factory for `node:*` wiring.

use std::fs;
use std::io;
use std::path::PathBuf;

use vaeljs_crypto_core::envelope::random_bytes;
use vaeljs_fs_core::{FsError, FsErrorCode};

use crate::error::{NodeCoreError, NodeCoreErrorCode, map_io_error};

/// Shared ECMAScript factory source constructing all `node:*` built-in module
/// objects from engine-supplied host primitives (`host`).
///
/// Evaluated once per runtime context in both `vaeljs-core` (V8) and
/// `vaeljs-lite-core` (QuickJS) when `node-compat` is enabled. Installs nothing
/// onto `globalThis` or `Vael`.
pub const NODE_COMPAT_FACTORY_SOURCE: &str = include_str!("node_compat.js");

/// Default maximum file size read by `node:fs` `readFileSync` / `promises.readFile` (`64 MiB`).
pub const DEFAULT_MAX_FS_READ_BYTES: u64 = 64 * 1024 * 1024;

const MKDTEMP_ATTEMPTS: usize = 16;
const MKDTEMP_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Creates a unique temporary directory whose path starts with `prefix` + 6 random
/// ASCII characters, matching `fs.mkdtempSync(prefix)`.
///
/// Requires the parent directory of `prefix` to exist. On Unix, creates the
/// directory with `0o700` permissions.
pub fn mkdtemp(prefix: &str) -> Result<PathBuf, NodeCoreError> {
    if prefix.is_empty() {
        return Err(NodeCoreError::InvalidPath {
            reason: "mkdtemp prefix must not be empty",
        });
    }
    if prefix.as_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidPath {
            reason: "mkdtemp prefix must not contain a NUL byte",
        });
    }

    for _ in 0..MKDTEMP_ATTEMPTS {
        let entropy = random_bytes(6).map_err(|_| NodeCoreError::Unavailable {
            reason: "operating system random number generator failed",
        })?;
        let mut candidate = String::with_capacity(prefix.len() + 6);
        candidate.push_str(prefix);
        for byte in entropy {
            let idx = usize::from(byte) % MKDTEMP_ALPHABET.len();
            candidate.push(char::from(MKDTEMP_ALPHABET[idx]));
        }
        let path = PathBuf::from(&candidate);
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(map_io_error("create temporary directory", err)),
        }
    }

    Err(NodeCoreError::Io {
        context: "create temporary directory",
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "failed to allocate a unique temporary directory",
        ),
    })
}

/// Generates a lowercase RFC 9562 version 4 UUID from the OS CSPRNG.
pub fn random_uuid() -> Result<String, NodeCoreError> {
    let mut bytes = random_bytes(16).map_err(|_| NodeCoreError::Unavailable {
        reason: "operating system random number generator failed",
    })?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(36);
    for (idx, &b) in bytes.iter().enumerate() {
        if matches!(idx, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    Ok(out)
}

/// Compares two equal-length byte slices in constant time.
///
/// Returns `Err(NodeCoreError::InvalidInput)` if `a.len() != b.len()` (bindings
/// map this to `RangeError` to match Node.js `crypto.timingSafeEqual`).
pub fn timing_safe_equal(a: &[u8], b: &[u8]) -> Result<bool, NodeCoreError> {
    if a.len() != b.len() {
        return Err(NodeCoreError::InvalidInput {
            reason: "Input buffers must have the same byte length",
        });
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    Ok(std::hint::black_box(diff) == 0)
}

/// Maps a [`FsErrorCode`] to a standard Node.js / POSIX error code string.
#[must_use]
pub const fn fs_error_code_str(err: &FsError) -> &'static str {
    match err.code() {
        FsErrorCode::NotFound => "ENOENT",
        FsErrorCode::AlreadyExists => "EEXIST",
        FsErrorCode::PermissionDenied => "EACCES",
        FsErrorCode::IsADirectory => "EISDIR",
        FsErrorCode::NotADirectory => "ENOTDIR",
        FsErrorCode::DirectoryNotEmpty => "ENOTEMPTY",
        FsErrorCode::InvalidPath | FsErrorCode::InvalidArgument | FsErrorCode::InvalidUtf8 => {
            "EINVAL"
        }
        FsErrorCode::TooLarge => "ERR_FS_FILE_TOO_LARGE",
        _ => "EIO",
    }
}

/// Maps a [`NodeCoreErrorCode`] to a standard Node.js / POSIX error code string.
#[must_use]
pub const fn node_error_code_str(err: &NodeCoreError) -> &'static str {
    match err {
        NodeCoreError::Fs(fs_err) => fs_error_code_str(fs_err),
        _ => match err.code() {
            NodeCoreErrorCode::NotFound => "ENOENT",
            NodeCoreErrorCode::PermissionDenied => "EACCES",
            NodeCoreErrorCode::InvalidUrl => "ERR_INVALID_URL",
            NodeCoreErrorCode::InvalidInput
            | NodeCoreErrorCode::InvalidPath
            | NodeCoreErrorCode::InvalidEncoding
            | NodeCoreErrorCode::InvalidUtf8 => "EINVAL",
            NodeCoreErrorCode::InputTooLarge => "ERR_OUT_OF_RANGE",
            NodeCoreErrorCode::Unavailable => "ERR_UNAVAILABLE",
            NodeCoreErrorCode::Io | NodeCoreErrorCode::ProcessError | NodeCoreErrorCode::Fs => {
                "EIO"
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mkdtemp_creates_unique_dir_and_validates_prefix() {
        assert_eq!(
            mkdtemp("").unwrap_err().code(),
            NodeCoreErrorCode::InvalidPath
        );
        assert_eq!(
            mkdtemp("bad\0prefix").unwrap_err().code(),
            NodeCoreErrorCode::InvalidPath
        );
        let base = tempfile::tempdir().unwrap();
        let prefix = base.path().join("node-tmp-");
        let created = mkdtemp(prefix.to_str().unwrap()).unwrap();
        assert!(created.is_dir());
        assert!(
            created
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("node-tmp-")
        );
    }

    #[test]
    fn random_uuid_has_v4_format_and_timing_safe_equal_works() {
        let u1 = random_uuid().unwrap();
        let u2 = random_uuid().unwrap();
        assert_eq!(u1.len(), 36);
        assert_ne!(u1, u2);
        assert_eq!(&u1[14..15], "4");
        assert!(matches!(&u1[19..20], "8" | "9" | "a" | "b"));

        assert!(timing_safe_equal(b"secret", b"secret").unwrap());
        assert!(!timing_safe_equal(b"secret", b"secreT").unwrap());
        assert!(timing_safe_equal(b"", b"").unwrap());
        assert_eq!(
            timing_safe_equal(b"abc", b"ab").unwrap_err().code(),
            NodeCoreErrorCode::InvalidInput
        );
    }
}
