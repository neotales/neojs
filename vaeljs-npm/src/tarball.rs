//! Bounded `.tgz` verification, zero-disk-unpack POSIX/ustar/PAX tar reader,
//! and canonical `.tree` manifest publication into [`CasStore`].
//!
//! Security invariants:
//! - Verifies `sha512-...` SRI integrity BEFORE opening the gzip/tar stream.
//! - Rejects missing `integrity`, weak algorithms (`sha1`, `md5`), and mismatches.
//! - Streams through [`flate2::read::GzDecoder`] with a strict uncompressed byte
//!   ceiling (stopping gzip bombs immediately).
//! - Parses tar headers strictly in memory with zero disk unpack (`ustar`,
//!   `PAX` `x`/`g`, and GNU `L` longnames), rejecting symlinks, hardlinks,
//!   devices, FIFOs, sparse entries, traversal (`..`), backslashes, absolute
//!   paths, `node_modules/` segments, non-uniform top-level directories, and
//!   duplicate paths.
//! - Excludes native binaries (`.node`, `.dll`, `.so`, `.dylib`) from CAS and
//!   records them in `skipped_native` for audit reporting.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

use crate::cas::{
    CasError, CasStore, atomic_write_file, hex_lower, read_bounded_regular_file,
    read_verified_file, sha256_hex,
};
use crate::package_json::{MAX_PACKAGE_JSON_BYTES, PackageJson, PackageJsonError};

const DEFAULT_MAX_COMPRESSED_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_TARBALL_FILES: usize = 8_192;
const DEFAULT_MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_PATH_BYTES: usize = 512;
const DEFAULT_MAX_PACKUMENT_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_GRAPH_PACKAGES: usize = 2_048;
const MAX_PAX_HEADER_BYTES: usize = 64 * 1024;

/// Resource and security bounds applied during npm packument, tarball, and graph processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NpmLimits {
    /// Maximum compressed `.tgz` size in bytes (default 32 MiB).
    pub max_tarball_compressed_bytes: usize,
    /// Maximum decompressed tar stream size in bytes (default 64 MiB).
    pub max_tarball_uncompressed_bytes: usize,
    /// Maximum total entries in one tarball (default 8,192).
    pub max_tarball_files: usize,
    /// Maximum individual file size in bytes (default 16 MiB).
    pub max_file_bytes: usize,
    /// Maximum tar entry path length in bytes (default 512).
    pub max_path_bytes: usize,
    /// Maximum `package.json` size in bytes (default 1 MiB).
    pub max_package_json_bytes: usize,
    /// Maximum packument JSON size in bytes (default 16 MiB).
    pub max_packument_bytes: usize,
    /// Maximum resolved package instances in one graph (default 2,048).
    pub max_graph_packages: usize,
}

impl Default for NpmLimits {
    fn default() -> Self {
        Self {
            max_tarball_compressed_bytes: DEFAULT_MAX_COMPRESSED_BYTES,
            max_tarball_uncompressed_bytes: DEFAULT_MAX_UNCOMPRESSED_BYTES,
            max_tarball_files: DEFAULT_MAX_TARBALL_FILES,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_path_bytes: DEFAULT_MAX_PATH_BYTES,
            max_package_json_bytes: MAX_PACKAGE_JSON_BYTES,
            max_packument_bytes: DEFAULT_MAX_PACKUMENT_BYTES,
            max_graph_packages: DEFAULT_MAX_GRAPH_PACKAGES,
        }
    }
}

/// Failure returned during tarball SRI verification, decompression, or tar parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TarballError {
    /// Tarball `dist.integrity` was absent or empty.
    #[error("npm tarball missing required sha512 SRI integrity digest")]
    MissingIntegrity,

    /// Tarball only provided a weak or unsupported SRI algorithm (such as `sha1`).
    #[error("unsupported or weak tarball integrity `{algorithm}` (sha512 required)")]
    WeakIntegrity { algorithm: String },

    /// Tarball SRI format or base64 payload was malformed.
    #[error("malformed tarball SRI integrity `{integrity}`")]
    MalformedIntegrity { integrity: String },

    /// Tarball SHA-512 digest did not match the declared SRI digest.
    #[error("tarball SRI mismatch: expected {expected}, got {actual}")]
    IntegrityMismatch { expected: String, actual: String },

    /// Compressed tarball exceeded [`NpmLimits::max_tarball_compressed_bytes`].
    #[error("tarball compressed size {size} bytes exceeds {limit}-byte limit")]
    CompressedLimitExceeded { size: usize, limit: usize },

    /// Decompressed stream exceeded [`NpmLimits::max_tarball_uncompressed_bytes`].
    #[error("tarball uncompressed stream exceeds {limit}-byte limit")]
    UncompressedLimitExceeded { limit: usize },

    /// Individual entry size exceeded [`NpmLimits::max_file_bytes`].
    #[error("tarball file `{path}` size {size} bytes exceeds {limit}-byte limit")]
    FileLimitExceeded {
        path: String,
        size: usize,
        limit: usize,
    },

    /// Entry count exceeded [`NpmLimits::max_tarball_files`].
    #[error("tarball entry count exceeds {limit}-entry limit")]
    EntryCountLimitExceeded { limit: usize },

    /// Tar header, checksum, or gzip stream was malformed or truncated.
    #[error("malformed tar archive: {reason}")]
    MalformedTar { reason: String },

    /// Entry path failed traversal, encoding, character, or `node_modules` checks.
    #[error("unsafe tarball path `{path}`: {reason}")]
    UnsafeEntryPath { path: String, reason: String },

    /// Entry had a disallowed tar type (symlink, hardlink, device, FIFO, etc.).
    #[error("disallowed tar entry type `{typeflag}` at `{path}`")]
    UnsupportedEntryType { path: String, typeflag: char },

    /// Two entries resolved to the same normalized package-relative path.
    #[error("duplicate path `{path}` in tarball")]
    DuplicatePath { path: String },

    /// Archive entries did not share a single uniform top-level directory.
    #[error("tarball top-level directory mismatch: expected `{expected}`, got `{actual}`")]
    InconsistentTopDirectory { expected: String, actual: String },

    /// Tarball contained no root `package.json`.
    #[error("tarball does not contain a root package.json")]
    MissingPackageJson,

    /// Root `package.json` failed bounded parsing.
    #[error("invalid root package.json in tarball: {0}")]
    PackageJson(#[from] PackageJsonError),

    /// CAS or `.tree` persistence failed.
    #[error("CAS error during tarball ingestion: {0}")]
    Cas(#[from] CasError),
}

/// One file entry recorded inside a canonical `.tree` manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeFileEntry {
    /// Lowercase hex SHA-256 of the verified file bytes in `cas/sha256/`.
    pub sha256: String,
    /// Exact byte length of the file.
    pub size: u64,
}

/// Canonical package tree manifest stored at `npm/<origin>/trees/sha512-<hex>.tree`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageTree {
    /// Format version (`1`).
    pub version: u32,
    /// Verified tarball `sha512-...` SRI string.
    pub integrity: String,
    /// Sorted map of normalized package-relative path -> CAS blob digest/size.
    pub files: BTreeMap<String, TreeFileEntry>,
    /// Native binaries excluded from CAS during ingestion.
    pub skipped_native: Vec<String>,
    /// Whether `binding.gyp` exists at package root.
    pub has_binding_gyp: bool,
    /// Lowercase hex SHA-256 of the canonical `.tree` JSON bytes.
    #[serde(skip)]
    pub tree_sha256: String,
}

impl PackageTree {
    /// Looks up a regular file in the tree by normalized package-relative path.
    pub fn get_file(&self, rel_path: &str) -> Option<&TreeFileEntry> {
        self.files.get(rel_path)
    }

    /// Returns `true` if `rel_path` was excluded as a native binary.
    pub fn is_skipped_native(&self, rel_path: &str) -> bool {
        self.skipped_native.iter().any(|p| p == rel_path)
    }

    /// Reads and verifies a cached `.tree` file for `(registry_origin, integrity)`
    /// against `expected_tree_sha256` (when pinned by `vael.lock`), or returns
    /// `Ok(None)` if not present on disk.
    pub fn load_verified(
        cache_root: &Path,
        registry_origin: &str,
        integrity: &str,
        expected_tree_sha256: Option<&str>,
    ) -> Result<Option<Self>, TarballError> {
        let expected_sha512 = parse_sha512_sri(integrity)?;
        let sha512_hex = hex_lower(&expected_sha512);
        let path = tree_path(cache_root, registry_origin, &sha512_hex);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = match expected_tree_sha256 {
            Some(tree_hash) => read_verified_file(&path, tree_hash, None, 16 * 1024 * 1024)?,
            None => read_bounded_regular_file(&path, None, 16 * 1024 * 1024)?,
        };
        let tree_sha256 = sha256_hex(&bytes);
        let mut tree: Self =
            serde_json::from_slice(&bytes).map_err(|err| TarballError::MalformedTar {
                reason: format!("corrupt .tree manifest: {err}"),
            })?;
        if tree.version != 1 {
            return Err(TarballError::MalformedTar {
                reason: format!("unsupported .tree version {}", tree.version),
            });
        }
        let tree_sri = parse_sha512_sri(&tree.integrity)?;
        if tree_sri != expected_sha512 {
            return Err(TarballError::IntegrityMismatch {
                expected: integrity.to_owned(),
                actual: tree.integrity.clone(),
            });
        }
        tree.tree_sha256 = tree_sha256;
        Ok(Some(tree))
    }
}

/// Verified tarball ingestor writing extracted regular files into [`CasStore`]
/// and publishing `.tgz` + `.tree` manifests atomically.
#[derive(Clone, Debug)]
pub struct TarballIngestor {
    cas: CasStore,
    limits: NpmLimits,
}

impl TarballIngestor {
    /// Creates an ingestor backed by `cas` and bounded by `limits`.
    pub fn new(cas: CasStore, limits: NpmLimits) -> Self {
        Self {
            cas: cas.with_max_file_bytes(limits.max_file_bytes),
            limits,
        }
    }

    /// Returns a reference to the underlying [`CasStore`].
    pub fn cas(&self) -> &CasStore {
        &self.cas
    }

    /// Verifies `tgz_bytes` against `integrity` (`sha512-...`), streams and
    /// validates tar entries in memory, stores regular non-native files in
    /// [`CasStore`], and atomically writes `.tgz` and `.tree`.
    pub fn ingest(
        &self,
        registry_origin: &str,
        integrity: &str,
        tgz_bytes: &[u8],
    ) -> Result<PackageTree, TarballError> {
        if tgz_bytes.len() > self.limits.max_tarball_compressed_bytes {
            return Err(TarballError::CompressedLimitExceeded {
                size: tgz_bytes.len(),
                limit: self.limits.max_tarball_compressed_bytes,
            });
        }

        let expected_sha512 = parse_sha512_sri(integrity)?;
        let actual_sha512 = Sha512::digest(tgz_bytes);
        if actual_sha512.as_slice() != expected_sha512 {
            let expected_b64 = BASE64_STANDARD.encode(expected_sha512);
            let actual_b64 = BASE64_STANDARD.encode(actual_sha512);
            return Err(TarballError::IntegrityMismatch {
                expected: format!("sha512-{expected_b64}"),
                actual: format!("sha512-{actual_b64}"),
            });
        }
        let sha512_hex = hex_lower(&actual_sha512);
        let canonical_sri = format!("sha512-{}", BASE64_STANDARD.encode(actual_sha512));

        self.cas.ensure_layout()?;

        let gz = GzDecoder::new(tgz_bytes);
        let mut reader = BoundedReader::new(gz, self.limits.max_tarball_uncompressed_bytes);
        let parsed = read_tar_entries(&mut reader, &self.limits)?;

        let mut files = BTreeMap::new();
        let mut skipped_native = Vec::new();
        let mut has_package_json = false;
        let mut has_binding_gyp = false;

        for (rel_path, bytes) in parsed {
            if rel_path == "package.json" {
                // Validate root package.json immediately during ingestion.
                let _ = PackageJson::parse_bytes(&bytes)?;
                has_package_json = true;
            }
            if rel_path == "binding.gyp" {
                has_binding_gyp = true;
            }
            if is_native_binary_path(&rel_path) {
                skipped_native.push(rel_path);
                continue;
            }
            let digest = self.cas.write_verified(&bytes)?;
            files.insert(
                rel_path,
                TreeFileEntry {
                    sha256: digest.sha256,
                    size: digest.size,
                },
            );
        }

        if !has_package_json {
            return Err(TarballError::MissingPackageJson);
        }
        skipped_native.sort();

        let mut tree = PackageTree {
            version: 1,
            integrity: canonical_sri,
            files,
            skipped_native,
            has_binding_gyp,
            tree_sha256: String::new(),
        };

        let tree_bytes =
            serde_json::to_vec_pretty(&tree).map_err(|err| TarballError::MalformedTar {
                reason: format!("failed to serialize .tree manifest: {err}"),
            })?;
        tree.tree_sha256 = sha256_hex(&tree_bytes);

        // Publish .tgz first, then .tree ONLY after every referenced CAS blob exists.
        // Verify pre-existing artifacts rather than trusting `.exists()` so tampered
        // cache artifacts fail closed.
        let tgz_target = archive_path(self.cas.cache_root(), registry_origin, &sha512_hex);
        if tgz_target.exists() {
            let expected_tgz_sha256 = sha256_hex(tgz_bytes);
            let _ = read_verified_file(
                &tgz_target,
                &expected_tgz_sha256,
                Some(tgz_bytes.len() as u64),
                self.limits.max_tarball_compressed_bytes,
            )?;
        } else {
            atomic_write_file(&self.cas.tmp_dir(), &tgz_target, tgz_bytes, 0o444)?;
        }
        let tree_target = tree_path(self.cas.cache_root(), registry_origin, &sha512_hex);
        if tree_target.exists() {
            let _ = read_verified_file(
                &tree_target,
                &tree.tree_sha256,
                Some(tree_bytes.len() as u64),
                16 * 1024 * 1024,
            )?;
        } else {
            atomic_write_file(&self.cas.tmp_dir(), &tree_target, &tree_bytes, 0o444)?;
        }

        Ok(tree)
    }
}

fn archive_path(cache_root: &Path, registry_origin: &str, sha512_hex: &str) -> PathBuf {
    CasStore::new(cache_root)
        .npm_origin_dir(registry_origin)
        .join("archives")
        .join(format!("sha512-{sha512_hex}.tgz"))
}

fn tree_path(cache_root: &Path, registry_origin: &str, sha512_hex: &str) -> PathBuf {
    CasStore::new(cache_root)
        .npm_origin_dir(registry_origin)
        .join("trees")
        .join(format!("sha512-{sha512_hex}.tree"))
}

/// Parses an SRI string, requiring a valid `sha512-<base64>` digest and
/// rejecting missing or weak (`sha1`, `md5`) digests.
pub fn parse_sha512_sri(integrity: &str) -> Result<[u8; 64], TarballError> {
    let trimmed = integrity.trim();
    if trimmed.is_empty() {
        return Err(TarballError::MissingIntegrity);
    }

    let mut weak_seen: Option<String> = None;
    for token in trimmed.split_whitespace() {
        let Some((algo, b64)) = token.split_once('-') else {
            return Err(TarballError::MalformedIntegrity {
                integrity: integrity.to_owned(),
            });
        };
        if algo == "sha512" {
            let decoded =
                BASE64_STANDARD
                    .decode(b64)
                    .map_err(|_| TarballError::MalformedIntegrity {
                        integrity: integrity.to_owned(),
                    })?;
            let arr: [u8; 64] =
                decoded
                    .try_into()
                    .map_err(|_| TarballError::MalformedIntegrity {
                        integrity: integrity.to_owned(),
                    })?;
            return Ok(arr);
        }
        weak_seen.get_or_insert_with(|| algo.to_owned());
    }

    Err(TarballError::WeakIntegrity {
        algorithm: weak_seen.unwrap_or_else(|| "unknown".to_owned()),
    })
}

fn is_native_binary_path(rel_path: &str) -> bool {
    let lower = rel_path.to_ascii_lowercase();
    lower.ends_with(".node")
        || lower.ends_with(".dll")
        || lower.ends_with(".so")
        || lower.ends_with(".dylib")
}

struct BoundedReader<R> {
    inner: R,
    read_total: usize,
    limit: usize,
}

impl<R> BoundedReader<R> {
    fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            read_total: 0,
            limit,
        }
    }

    fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.read_total)
    }
}

impl<R: Read> BoundedReader<R> {
    fn read_exact_bounded(&mut self, buf: &mut [u8]) -> Result<bool, TarballError> {
        let mut filled = 0;
        while filled < buf.len() {
            let n =
                self.inner
                    .read(&mut buf[filled..])
                    .map_err(|err| TarballError::MalformedTar {
                        reason: format!("gzip/tar read error: {err}"),
                    })?;
            if n == 0 {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(TarballError::MalformedTar {
                    reason: "unexpected EOF inside tar block".to_owned(),
                });
            }
            self.read_total = self.read_total.saturating_add(n);
            if self.read_total > self.limit {
                return Err(TarballError::UncompressedLimitExceeded { limit: self.limit });
            }
            filled += n;
        }
        Ok(true)
    }

    fn skip_exact_bounded(&mut self, mut remaining: usize) -> Result<(), TarballError> {
        let mut scratch = [0u8; 512];
        while remaining > 0 {
            let chunk = remaining.min(scratch.len());
            if !self.read_exact_bounded(&mut scratch[..chunk])? {
                return Err(TarballError::MalformedTar {
                    reason: "unexpected EOF inside tar padding".to_owned(),
                });
            }
            remaining -= chunk;
        }
        Ok(())
    }
}

/// Strict bounded POSIX/ustar/PAX tar reader operating purely in memory.
fn read_tar_entries<R: Read>(
    reader: &mut BoundedReader<R>,
    limits: &NpmLimits,
) -> Result<Vec<(String, Vec<u8>)>, TarballError> {
    let mut header = [0u8; 512];
    let mut entries = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut top_dir: Option<String> = None;
    let mut entry_count = 0usize;

    let mut next_pax_path: Option<String> = None;
    let mut next_pax_size: Option<u64> = None;
    let mut next_gnu_longname: Option<String> = None;

    loop {
        if !reader.read_exact_bounded(&mut header)? {
            if next_pax_path.is_some() || next_pax_size.is_some() || next_gnu_longname.is_some() {
                return Err(TarballError::MalformedTar {
                    reason: "dangling extended tar header at EOF".to_owned(),
                });
            }
            break;
        }
        if header.iter().all(|&b| b == 0) {
            if next_pax_path.is_some() || next_pax_size.is_some() || next_gnu_longname.is_some() {
                return Err(TarballError::MalformedTar {
                    reason: "dangling extended tar header before EOF marker".to_owned(),
                });
            }
            // Standard tar ends with two 512-byte zero blocks; consume optional second zero block.
            if reader.read_exact_bounded(&mut header)? && !header.iter().all(|&b| b == 0) {
                return Err(TarballError::MalformedTar {
                    reason: "unexpected non-zero block after tar EOF marker".to_owned(),
                });
            }
            break;
        }

        verify_tar_checksum(&header)?;
        let header_size = parse_tar_size(&header[124..136])?;
        let typeflag = header[156];

        match typeflag {
            b'x' => {
                let data = read_meta_payload(reader, header_size, MAX_PAX_HEADER_BYTES)?;
                let (p, s) = parse_pax_records(&data)?;
                if p.is_some() {
                    next_pax_path = p;
                }
                if s.is_some() {
                    next_pax_size = s;
                }
                continue;
            }
            b'g' => {
                let _ = read_meta_payload(reader, header_size, MAX_PAX_HEADER_BYTES)?;
                continue;
            }
            b'L' => {
                let data = read_meta_payload(reader, header_size, limits.max_path_bytes + 1)?;
                let trimmed = data.split(|&b| b == 0).next().unwrap_or(&[]);
                let name =
                    std::str::from_utf8(trimmed).map_err(|_| TarballError::MalformedTar {
                        reason: "GNU longname is not valid UTF-8".to_owned(),
                    })?;
                next_gnu_longname = Some(name.to_owned());
                continue;
            }
            b'0' | b'\0' | b'5' => {}
            other => {
                let _ = (
                    next_pax_path.take(),
                    next_pax_size.take(),
                    next_gnu_longname.take(),
                );
                let raw_name =
                    parse_ustar_path(&header).unwrap_or_else(|_| "<invalid-utf8-path>".to_owned());
                return Err(TarballError::UnsupportedEntryType {
                    path: raw_name,
                    typeflag: other as char,
                });
            }
        }

        entry_count += 1;
        if entry_count > limits.max_tarball_files {
            return Err(TarballError::EntryCountLimitExceeded {
                limit: limits.max_tarball_files,
            });
        }

        let pax_path = next_pax_path.take();
        let gnu_long = next_gnu_longname.take();
        let raw_path = match pax_path.or(gnu_long) {
            Some(path) => path,
            None => parse_ustar_path(&header)?,
        };
        let size = next_pax_size.take().unwrap_or(header_size);

        let is_dir = typeflag == b'5';
        if is_dir && size != 0 {
            return Err(TarballError::MalformedTar {
                reason: format!("directory entry `{raw_path}` has non-zero size"),
            });
        }

        let rel_opt =
            normalize_tar_entry_path(&raw_path, is_dir, limits.max_path_bytes, &mut top_dir)?;

        let size_usize = usize::try_from(size).map_err(|_| TarballError::FileLimitExceeded {
            path: raw_path.clone(),
            size: usize::MAX,
            limit: limits.max_file_bytes,
        })?;
        if size_usize > limits.max_file_bytes {
            return Err(TarballError::FileLimitExceeded {
                path: raw_path,
                size: size_usize,
                limit: limits.max_file_bytes,
            });
        }
        if size_usize > reader.remaining() {
            return Err(TarballError::UncompressedLimitExceeded {
                limit: limits.max_tarball_uncompressed_bytes,
            });
        }

        let mut payload = vec![0u8; size_usize];
        if size_usize > 0 && !reader.read_exact_bounded(&mut payload)? {
            return Err(TarballError::MalformedTar {
                reason: format!("unexpected EOF reading `{raw_path}`"),
            });
        }
        let pad = (512 - (size_usize % 512)) % 512;
        if pad > 0 {
            reader.skip_exact_bounded(pad)?;
        }

        if let Some(rel_path) = rel_opt {
            if !seen_paths.insert(rel_path.clone()) {
                return Err(TarballError::DuplicatePath { path: rel_path });
            }
            entries.push((rel_path, payload));
        }
    }

    Ok(entries)
}

fn verify_tar_checksum(header: &[u8; 512]) -> Result<(), TarballError> {
    let declared = parse_octal(&header[148..156]).ok_or_else(|| TarballError::MalformedTar {
        reason: "invalid tar header checksum field".to_owned(),
    })?;
    let mut sum: u64 = 0;
    for (idx, &byte) in header.iter().enumerate() {
        sum += if (148..156).contains(&idx) {
            u64::from(b' ')
        } else {
            u64::from(byte)
        };
    }
    if sum != declared {
        return Err(TarballError::MalformedTar {
            reason: format!("tar header checksum mismatch (declared {declared}, actual {sum})"),
        });
    }
    Ok(())
}

fn parse_octal(field: &[u8]) -> Option<u64> {
    let trimmed = field
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .collect::<Vec<u8>>();
    let s = std::str::from_utf8(&trimmed).ok()?.trim();
    if s.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(s, 8).ok()
}

fn parse_tar_size(field: &[u8]) -> Result<u64, TarballError> {
    if field[0] & 0x80 != 0 {
        // GNU/POSIX base-256 positive integer encoding.
        if field[0] & 0x40 != 0 {
            return Err(TarballError::MalformedTar {
                reason: "negative tar entry size".to_owned(),
            });
        }
        let mut val: u64 = u64::from(field[0] & 0x7f);
        for &b in &field[1..] {
            val = val
                .checked_mul(256)
                .ok_or_else(|| TarballError::MalformedTar {
                    reason: "tar size overflow".to_owned(),
                })?
                | u64::from(b);
        }
        return Ok(val);
    }
    parse_octal(field).ok_or_else(|| TarballError::MalformedTar {
        reason: "invalid octal size field in tar header".to_owned(),
    })
}

fn read_meta_payload<R: Read>(
    reader: &mut BoundedReader<R>,
    size: u64,
    limit: usize,
) -> Result<Vec<u8>, TarballError> {
    let size_usize = usize::try_from(size).map_err(|_| TarballError::MalformedTar {
        reason: "extended tar header size overflow".to_owned(),
    })?;
    if size_usize > limit {
        return Err(TarballError::MalformedTar {
            reason: format!("extended tar header exceeds {limit}-byte limit"),
        });
    }
    let mut buf = vec![0u8; size_usize];
    if size_usize > 0 && !reader.read_exact_bounded(&mut buf)? {
        return Err(TarballError::MalformedTar {
            reason: "unexpected EOF inside extended tar header".to_owned(),
        });
    }
    let pad = (512 - (size_usize % 512)) % 512;
    if pad > 0 {
        reader.skip_exact_bounded(pad)?;
    }
    Ok(buf)
}

fn parse_pax_records(data: &[u8]) -> Result<(Option<String>, Option<u64>), TarballError> {
    let mut pos = 0;
    let mut path = None;
    let mut size = None;
    while pos < data.len() {
        let rest = &data[pos..];
        let Some(space_rel) = rest.iter().position(|&b| b == b' ') else {
            return Err(TarballError::MalformedTar {
                reason: "PAX record missing length separator".to_owned(),
            });
        };
        let len_str =
            std::str::from_utf8(&rest[..space_rel]).map_err(|_| TarballError::MalformedTar {
                reason: "PAX record length is not UTF-8".to_owned(),
            })?;
        let rec_len: usize = len_str.parse().map_err(|_| TarballError::MalformedTar {
            reason: "PAX record length is not an integer".to_owned(),
        })?;
        if rec_len <= space_rel + 1 || rec_len > rest.len() {
            return Err(TarballError::MalformedTar {
                reason: "PAX record length out of bounds".to_owned(),
            });
        }
        let record = &rest[space_rel + 1..rec_len];
        let line = record.strip_suffix(b"\n").unwrap_or(record);
        if let Some(eq_pos) = line.iter().position(|&b| b == b'=') {
            let key = &line[..eq_pos];
            let val = std::str::from_utf8(&line[eq_pos + 1..]).map_err(|_| {
                TarballError::MalformedTar {
                    reason: "PAX record value is not valid UTF-8".to_owned(),
                }
            })?;
            if key == b"path" {
                path = Some(val.to_owned());
            } else if key == b"size" {
                size = Some(val.parse::<u64>().map_err(|_| TarballError::MalformedTar {
                    reason: "invalid PAX size record".to_owned(),
                })?);
            }
        }
        pos += rec_len;
    }
    Ok((path, size))
}

fn parse_ustar_path(header: &[u8; 512]) -> Result<String, TarballError> {
    let name_bytes = header[0..100].split(|&b| b == 0).next().unwrap_or(&[]);
    let prefix_bytes = header[345..500].split(|&b| b == 0).next().unwrap_or(&[]);
    let name = std::str::from_utf8(name_bytes).map_err(|_| TarballError::MalformedTar {
        reason: "tar entry name is not valid UTF-8".to_owned(),
    })?;
    if prefix_bytes.is_empty() {
        Ok(name.to_owned())
    } else {
        let prefix = std::str::from_utf8(prefix_bytes).map_err(|_| TarballError::MalformedTar {
            reason: "tar entry prefix is not valid UTF-8".to_owned(),
        })?;
        Ok(format!("{prefix}/{name}"))
    }
}

fn normalize_tar_entry_path(
    raw: &str,
    is_dir: bool,
    max_path_bytes: usize,
    top_dir: &mut Option<String>,
) -> Result<Option<String>, TarballError> {
    if raw.is_empty() {
        return Err(TarballError::UnsafeEntryPath {
            path: raw.to_owned(),
            reason: "empty entry path".to_owned(),
        });
    }
    if raw.len() > max_path_bytes {
        return Err(TarballError::UnsafeEntryPath {
            path: raw.to_owned(),
            reason: format!("path exceeds {max_path_bytes}-byte limit"),
        });
    }
    if raw.contains('\0') || raw.contains('\\') || raw.chars().any(char::is_control) {
        return Err(TarballError::UnsafeEntryPath {
            path: raw.to_owned(),
            reason: "path contains NUL, backslash, or control characters".to_owned(),
        });
    }
    if raw.starts_with('/') || crate::specifier::has_windows_drive_prefix(raw) {
        return Err(TarballError::UnsafeEntryPath {
            path: raw.to_owned(),
            reason: "absolute paths and drive prefixes are forbidden".to_owned(),
        });
    }

    let trimmed = if is_dir {
        raw.strip_suffix('/').unwrap_or(raw)
    } else {
        if raw.ends_with('/') {
            return Err(TarballError::UnsafeEntryPath {
                path: raw.to_owned(),
                reason: "regular file path ends with `/`".to_owned(),
            });
        }
        raw
    };

    let segments: Vec<&str> = trimmed.split('/').collect();
    for seg in &segments {
        if seg.is_empty() || *seg == "." || *seg == ".." {
            return Err(TarballError::UnsafeEntryPath {
                path: raw.to_owned(),
                reason: "path contains empty, `.`, or `..` segment".to_owned(),
            });
        }
        if crate::specifier::has_windows_drive_prefix(seg) {
            return Err(TarballError::UnsafeEntryPath {
                path: raw.to_owned(),
                reason: "path segment contains a Windows drive prefix".to_owned(),
            });
        }
        if seg.eq_ignore_ascii_case("node_modules") {
            return Err(TarballError::UnsafeEntryPath {
                path: raw.to_owned(),
                reason: "`node_modules` path segment is forbidden".to_owned(),
            });
        }
    }

    let first = segments[0];
    match top_dir {
        Some(expected) if expected != first => {
            return Err(TarballError::InconsistentTopDirectory {
                expected: expected.clone(),
                actual: first.to_owned(),
            });
        }
        None => {
            *top_dir = Some(first.to_owned());
        }
        _ => {}
    }

    if is_dir {
        return Ok(None);
    }
    if segments.len() < 2 {
        return Err(TarballError::UnsafeEntryPath {
            path: raw.to_owned(),
            reason: "file entry must live under the top-level package directory".to_owned(),
        });
    }
    Ok(Some(segments[1..].join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test category: SRI parsing boundary check rejecting sha1/missing/malformed.
    #[test]
    fn rejects_missing_and_weak_sri() {
        assert!(matches!(
            parse_sha512_sri(""),
            Err(TarballError::MissingIntegrity)
        ));
        assert!(matches!(
            parse_sha512_sri("sha1-2jmj7l5rSw0yVb/vlWAYkK/YBwk="),
            Err(TarballError::WeakIntegrity { .. })
        ));
        assert!(matches!(
            parse_sha512_sri("sha512-not-valid"),
            Err(TarballError::MalformedIntegrity { .. })
        ));
    }
}
