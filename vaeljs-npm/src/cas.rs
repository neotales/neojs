//! Content-Addressed Store (`cas/sha256/`) with atomic `0444` writes and
//! `O_NOFOLLOW` + exact-size + SHA-256 verification on every read.
//!
//! All extracted npm package files live in `<cache_root>/cas/sha256/<aa>/<bb>/<sha256_hex>`
//! and are read back into owned `Vec<u8>` buffers only after verifying both
//! exact byte length and SHA-256 digest on the open file descriptor.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Default maximum byte length accepted for one CAS blob (16 MiB).
pub const DEFAULT_MAX_CAS_FILE_BYTES: usize = 16 * 1024 * 1024;

/// Version marker stored at `<cache_root>/cache.version`.
pub const CACHE_LAYOUT_VERSION: &str = "vaeljs-cache-layout 1\n";

/// Failure returned by [`CasStore`] reads, writes, or integrity checks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CasError {
    /// Expected SHA-256 digest was not 64 lowercase hexadecimal characters.
    #[error("invalid SHA-256 digest `{digest}`")]
    InvalidDigest { digest: String },

    /// The file size exceeded the configured limit.
    #[error("CAS object size {size} bytes exceeds {limit}-byte limit")]
    TooLarge { size: u64, limit: u64 },

    /// The expected CAS object does not exist.
    #[error("CAS object `sha256:{digest}` not found")]
    NotFound { digest: String },

    /// The file on disk was not a regular file (for example, a symlink or directory).
    #[error("CAS object failed regular-file check")]
    NotRegularFile,

    /// The observed file size did not match the expected manifest size.
    #[error("CAS size mismatch: expected {expected} bytes, got {actual} bytes")]
    SizeMismatch { expected: u64, actual: u64 },

    /// The SHA-256 digest of the bytes read did not match the expected digest.
    #[error("CAS SHA-256 integrity mismatch: expected {expected}, got {actual}")]
    IntegrityMismatch { expected: String, actual: String },

    /// An underlying filesystem operation failed.
    #[error("CAS {operation} failed: {reason}")]
    Io {
        operation: &'static str,
        reason: String,
    },
}

/// Verified digest and byte size of a stored CAS object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CasDigest {
    /// Lowercase 64-character hex SHA-256 digest.
    pub sha256: String,
    /// Exact byte length.
    pub size: u64,
}

/// Content-Addressed Store rooted at `<cache_root>/cas/sha256/`.
#[derive(Clone, Debug)]
pub struct CasStore {
    cache_root: PathBuf,
    max_file_bytes: usize,
}

impl CasStore {
    /// Creates a [`CasStore`] rooted under `cache_root`.
    pub fn new(cache_root: impl Into<PathBuf>) -> Self {
        Self {
            cache_root: cache_root.into(),
            max_file_bytes: DEFAULT_MAX_CAS_FILE_BYTES,
        }
    }

    /// Overrides the maximum file size accepted on CAS reads and writes.
    pub fn with_max_file_bytes(mut self, max_file_bytes: usize) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }

    /// Returns the base cache root directory.
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// Returns `<cache_root>/tmp`.
    pub fn tmp_dir(&self) -> PathBuf {
        self.cache_root.join("tmp")
    }

    /// Returns `<cache_root>/cas/sha256`.
    pub fn cas_dir(&self) -> PathBuf {
        self.cache_root.join("cas").join("sha256")
    }

    /// Returns `<cache_root>/npm/<origin_sha256_hex>`.
    pub fn npm_origin_dir(&self, registry_origin: &str) -> PathBuf {
        let origin_hash = sha256_hex(registry_origin.as_bytes());
        self.cache_root.join("npm").join(origin_hash)
    }

    /// Ensures the partitioned cache root, `tmp/`, `cas/sha256/`, and `cache.version` exist.
    pub fn ensure_layout(&self) -> Result<(), CasError> {
        create_dir_mode(&self.cache_root, 0o700)?;
        create_dir_mode(&self.tmp_dir(), 0o700)?;
        create_dir_mode(&self.cache_root.join("locks"), 0o700)?;
        create_dir_mode(&self.cache_root.join("cas"), 0o755)?;
        create_dir_mode(&self.cas_dir(), 0o755)?;

        let version_path = self.cache_root.join("cache.version");
        if !version_path.exists() {
            let _ = atomic_write_file(
                &self.tmp_dir(),
                &version_path,
                CACHE_LAYOUT_VERSION.as_bytes(),
                0o644,
            );
        }
        Ok(())
    }

    /// Computes the CAS storage path `cas/sha256/<aa>/<bb>/<digest>` for `sha256_hex`.
    pub fn blob_path(&self, sha256_hex: &str) -> Result<PathBuf, CasError> {
        validate_sha256_hex(sha256_hex)?;
        let aa = &sha256_hex[0..2];
        let bb = &sha256_hex[2..4];
        Ok(self.cas_dir().join(aa).join(bb).join(sha256_hex))
    }

    /// Stores `bytes` atomically (`0444` on Unix) in `cas/sha256/<aa>/<bb>/<digest>`.
    ///
    /// If the target CAS blob already exists, its on-disk size and SHA-256
    /// digest are verified before accepting it; corrupted pre-existing CAS
    /// blobs fail closed with [`CasError::IntegrityMismatch`] or
    /// [`CasError::SizeMismatch`].
    pub fn write_verified(&self, bytes: &[u8]) -> Result<CasDigest, CasError> {
        if bytes.len() > self.max_file_bytes {
            return Err(CasError::TooLarge {
                size: bytes.len() as u64,
                limit: self.max_file_bytes as u64,
            });
        }
        self.ensure_layout()?;
        let digest = sha256_hex(bytes);
        let size = bytes.len() as u64;
        let target = self.blob_path(&digest)?;

        if target.exists() {
            // Verify existing blob instead of trusting or blindly overwriting corruption.
            let _existing = read_verified_file(&target, &digest, Some(size), self.max_file_bytes)?;
            return Ok(CasDigest {
                sha256: digest,
                size,
            });
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| CasError::Io {
                operation: "create CAS bucket directory",
                reason: err.to_string(),
            })?;
        }
        atomic_write_file(&self.tmp_dir(), &target, bytes, 0o444)?;

        Ok(CasDigest {
            sha256: digest,
            size,
        })
    }

    /// Reads a CAS object by `expected_sha256_hex` and `expected_size`, opening
    /// with `O_NOFOLLOW` on Unix and verifying regular-file metadata, exact
    /// size, and SHA-256 digest in memory before returning owned bytes.
    pub fn read_verified(
        &self,
        expected_sha256_hex: &str,
        expected_size: u64,
    ) -> Result<Vec<u8>, CasError> {
        validate_sha256_hex(expected_sha256_hex)?;
        if expected_size > self.max_file_bytes as u64 {
            return Err(CasError::TooLarge {
                size: expected_size,
                limit: self.max_file_bytes as u64,
            });
        }
        let path = self.blob_path(expected_sha256_hex)?;
        match read_verified_file(
            &path,
            expected_sha256_hex,
            Some(expected_size),
            self.max_file_bytes,
        ) {
            Err(CasError::Io {
                operation: "open", ..
            }) if matches!(
                fs::symlink_metadata(&path),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound
            ) =>
            {
                Err(CasError::NotFound {
                    digest: expected_sha256_hex.to_owned(),
                })
            }
            other => other,
        }
    }
}

/// Computes lowercase hex SHA-256 digest of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_lower(&digest)
}

/// Formats arbitrary digest bytes as lowercase hexadecimal.
pub fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Validates that `digest` is 64 lowercase ASCII hex digits.
pub fn validate_sha256_hex(digest: &str) -> Result<(), CasError> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(CasError::InvalidDigest {
            digest: digest.to_owned(),
        })
    }
}

/// Reads a regular file at `path` using `O_NOFOLLOW` (on Unix), `fstat`
/// regular-file + size check, and a single bounded read up to `max_bytes`.
pub fn read_bounded_regular_file(
    path: &Path,
    expected_size: Option<u64>,
    max_bytes: usize,
) -> Result<Vec<u8>, CasError> {
    let file = open_nofollow(path)?;
    let meta = file.metadata().map_err(|err| CasError::Io {
        operation: "stat",
        reason: err.to_string(),
    })?;
    if is_symlink_or_reparse(&meta) || !meta.is_file() {
        return Err(CasError::NotRegularFile);
    }
    let actual_len = meta.len();
    if let Some(exp) = expected_size
        && actual_len != exp
    {
        return Err(CasError::SizeMismatch {
            expected: exp,
            actual: actual_len,
        });
    }
    if actual_len > max_bytes as u64 {
        return Err(CasError::TooLarge {
            size: actual_len,
            limit: max_bytes as u64,
        });
    }

    let cap = expected_size.unwrap_or(actual_len) as usize;
    let mut buf = Vec::with_capacity(cap);
    let read_limit = (max_bytes as u64).saturating_add(1);
    file.take(read_limit)
        .read_to_end(&mut buf)
        .map_err(|err| CasError::Io {
            operation: "read",
            reason: err.to_string(),
        })?;

    if buf.len() > max_bytes {
        return Err(CasError::TooLarge {
            size: buf.len() as u64,
            limit: max_bytes as u64,
        });
    }
    if let Some(exp) = expected_size
        && buf.len() as u64 != exp
    {
        return Err(CasError::SizeMismatch {
            expected: exp,
            actual: buf.len() as u64,
        });
    }
    Ok(buf)
}

/// Reads and verifies a regular file at `path` using `O_NOFOLLOW` (on Unix),
/// `fstat` regular-file + size check, bounded single read, and SHA-256 verification.
pub fn read_verified_file(
    path: &Path,
    expected_sha256_hex: &str,
    expected_size: Option<u64>,
    max_bytes: usize,
) -> Result<Vec<u8>, CasError> {
    validate_sha256_hex(expected_sha256_hex)?;
    let buf = read_bounded_regular_file(path, expected_size, max_bytes)?;
    let actual_digest = sha256_hex(&buf);
    if actual_digest != expected_sha256_hex {
        return Err(CasError::IntegrityMismatch {
            expected: expected_sha256_hex.to_owned(),
            actual: actual_digest,
        });
    }
    Ok(buf)
}

/// Atomically writes `bytes` into `target` via a private temporary file in
/// `tmp_dir`, setting `mode` (e.g., `0o444`) on Unix before renaming.
pub(crate) fn atomic_write_file(
    tmp_dir: &Path,
    target: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), CasError> {
    create_dir_mode(tmp_dir, 0o700)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| CasError::Io {
            operation: "create target parent directory",
            reason: err.to_string(),
        })?;
    }

    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let tmp_path = tmp_dir.join(format!("tmp-{pid}-{seq}.part"));

    let write_res = (|| -> Result<(), CasError> {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp_path).map_err(|err| CasError::Io {
            operation: "create temp file",
            reason: err.to_string(),
        })?;
        file.write_all(bytes).map_err(|err| CasError::Io {
            operation: "write temp file",
            reason: err.to_string(),
        })?;
        file.sync_data().map_err(|err| CasError::Io {
            operation: "sync temp file",
            reason: err.to_string(),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode))
                .map_err(|err| CasError::Io {
                    operation: "chmod temp file",
                    reason: err.to_string(),
                })?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        drop(file);

        fs::rename(&tmp_path, target).map_err(|err| CasError::Io {
            operation: "rename temp file",
            reason: err.to_string(),
        })?;
        Ok(())
    })();

    if write_res.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_res
}

fn is_symlink_or_reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

pub(crate) fn open_nofollow(path: &Path) -> Result<File, CasError> {
    let pre_meta = fs::symlink_metadata(path).map_err(|err| CasError::Io {
        operation: "open",
        reason: err.to_string(),
    })?;
    if is_symlink_or_reparse(&pre_meta) || !pre_meta.is_file() {
        return Err(CasError::NotRegularFile);
    }

    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = opts.open(path).map_err(|err| CasError::Io {
        operation: "open",
        reason: err.to_string(),
    })?;
    let post_meta = file.metadata().map_err(|err| CasError::Io {
        operation: "stat",
        reason: err.to_string(),
    })?;
    if is_symlink_or_reparse(&post_meta) || !post_meta.is_file() {
        return Err(CasError::NotRegularFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if (pre_meta.dev(), pre_meta.ino()) != (post_meta.dev(), post_meta.ino()) {
            return Err(CasError::NotRegularFile);
        }
    }
    Ok(file)
}

fn create_dir_mode(dir: &Path, mode: u32) -> Result<(), CasError> {
    fs::create_dir_all(dir).map_err(|err| CasError::Io {
        operation: "create directory",
        reason: err.to_string(),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(mode)).map_err(|err| CasError::Io {
            operation: "set directory permissions",
            reason: err.to_string(),
        })?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test category: checksum and tamper-detection boundary for CasStore.
    #[test]
    fn cas_writes_readonly_and_rejects_tampering() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = CasStore::new(tmp.path());

        let body = b"export const answer = 42;\n";
        let digest = cas.write_verified(body).unwrap();
        assert_eq!(digest.size, body.len() as u64);
        assert_eq!(
            cas.read_verified(&digest.sha256, digest.size).unwrap(),
            body
        );

        let blob_path = cas.blob_path(&digest.sha256).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&blob_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o444);
            fs::set_permissions(&blob_path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        // Tamper with a single byte while keeping length identical.
        let mut tampered = body.to_vec();
        tampered[0] = b'X';
        fs::write(&blob_path, &tampered).unwrap();

        assert!(matches!(
            cas.read_verified(&digest.sha256, digest.size),
            Err(CasError::IntegrityMismatch { .. })
        ));
    }
}
