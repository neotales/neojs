//! Deterministic `neo.lock` v1 lockfile parsing, atomic persistence, and
//! integrity verification.
//!
//! `neo.lock` v1 pins requested npm specifiers (`"pkg@^1.0.0" -> "pkg@1.0.0"`)
//! and resolved package records (`version`, `integrity`, optional `treeSha256`,
//! `tarballUrl`, and `dependencies`) using sorted `BTreeMap` collections so
//! serialization is canonical and deterministic.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::cas::{CasError, read_bounded_regular_file, validate_sha256_hex};
use crate::resolver::ResolvedNpmGraph;
use crate::tarball::parse_sha512_sri;

/// Canonical lockfile filename.
pub const LOCKFILE_NAME: &str = "neo.lock";

/// Supported `neo.lock` format version (`1`).
pub const LOCKFILE_VERSION: u32 = 1;

/// Maximum byte size accepted when reading or parsing `neo.lock` (16 MiB).
pub const MAX_LOCKFILE_BYTES: usize = 16 * 1024 * 1024;

static LOCK_TMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Failure returned when reading, parsing, writing, or verifying `neo.lock`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LockfileError {
    /// Lockfile exceeded [`MAX_LOCKFILE_BYTES`].
    #[error("neo.lock exceeds {MAX_LOCKFILE_BYTES}-byte limit ({size} bytes)")]
    TooLarge { size: usize },

    /// JSON syntax, type, duplicate-key, or required-field validation failed.
    #[error("invalid neo.lock: {reason}")]
    InvalidJson { reason: String },

    /// Lockfile `"version"` was not [`LOCKFILE_VERSION`].
    #[error("unsupported neo.lock version {version} (expected {LOCKFILE_VERSION})")]
    UnsupportedVersion { version: u32 },

    /// Referenced package key was absent from `packages`.
    #[error("package `{pkg_key}` not found in neo.lock")]
    PackageNotFound { pkg_key: String },

    /// Stored or supplied integrity digest had an unsupported or malformed format.
    #[error("invalid integrity digest for `{pkg_key}` in neo.lock: {reason}")]
    InvalidIntegrity { pkg_key: String, reason: String },

    /// Actual digest did not match the digest pinned in `neo.lock`.
    #[error("integrity mismatch for `{pkg_key}`: expected {expected}, got {actual}")]
    IntegrityMismatch {
        pkg_key: String,
        expected: String,
        actual: String,
    },

    /// Underlying filesystem operation failed.
    #[error("neo.lock {operation} failed: {reason}")]
    Io {
        operation: &'static str,
        reason: String,
    },
}

/// One pinned package entry inside [`Lockfile::packages`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedPackage {
    /// Exact resolved SemVer version string.
    pub version: String,
    /// Verified integrity digest (`sha512-...` SRI or SHA-256 hex/SRI).
    pub integrity: String,
    /// Optional SHA-256 hex digest of the canonical `.tree` manifest in CAS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_sha256: Option<String>,
    /// Tarball download URL recorded from the registry packument.
    #[serde(default)]
    pub tarball_url: String,
    /// Resolved dependency map (`dep_name -> target_pkg_key`).
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

/// Deterministic `neo.lock` v1 lockfile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lockfile {
    /// Lockfile schema version (must be `1`).
    pub version: u32,
    /// Map of `"<name>@<constraint>"` -> `"<name>@<version>"` package key.
    pub specifiers: BTreeMap<String, String>,
    /// Map of package key (`"<name>@<version>"`) -> pinned package metadata.
    pub packages: BTreeMap<String, LockedPackage>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self::new()
    }
}

impl Lockfile {
    /// Creates an empty `neo.lock` v1 lockfile.
    pub fn new() -> Self {
        Self {
            version: LOCKFILE_VERSION,
            specifiers: BTreeMap::new(),
            packages: BTreeMap::new(),
        }
    }

    /// Parses and validates a `neo.lock` v1 JSON document.
    ///
    /// Rejects payloads over [`MAX_LOCKFILE_BYTES`], duplicate JSON keys,
    /// unsupported `version` values, and packages with empty `version` or
    /// `integrity`.
    pub fn parse(json: &str) -> Result<Self, LockfileError> {
        if json.len() > MAX_LOCKFILE_BYTES {
            return Err(LockfileError::TooLarge { size: json.len() });
        }
        let raw: RawLockfile =
            serde_json::from_str(json).map_err(|err| LockfileError::InvalidJson {
                reason: err.to_string(),
            })?;
        let Some(version) = raw.version else {
            return Err(LockfileError::InvalidJson {
                reason: "missing required `version` field".to_owned(),
            });
        };
        if version != LOCKFILE_VERSION {
            return Err(LockfileError::UnsupportedVersion { version });
        }
        for (spec_key, pkg_key) in &raw.specifiers {
            if spec_key.trim().is_empty() || pkg_key.trim().is_empty() {
                return Err(LockfileError::InvalidJson {
                    reason: "specifiers entries must have non-empty keys and targets".to_owned(),
                });
            }
        }
        for (pkg_key, pkg) in &raw.packages {
            if pkg_key.trim().is_empty() {
                return Err(LockfileError::InvalidJson {
                    reason: "packages keys must be non-empty".to_owned(),
                });
            }
            if pkg.version.trim().is_empty() {
                return Err(LockfileError::InvalidJson {
                    reason: format!("package `{pkg_key}` has an empty version"),
                });
            }
            if pkg.integrity.trim().is_empty() {
                return Err(LockfileError::InvalidJson {
                    reason: format!("package `{pkg_key}` has an empty integrity"),
                });
            }
            if let Some(tree_hash) = &pkg.tree_sha256 {
                validate_sha256_hex(tree_hash).map_err(|err| LockfileError::InvalidIntegrity {
                    pkg_key: pkg_key.clone(),
                    reason: err.to_string(),
                })?;
            }
            for (dep_name, dep_key) in &pkg.dependencies {
                if dep_name.trim().is_empty() || dep_key.trim().is_empty() {
                    return Err(LockfileError::InvalidJson {
                        reason: format!(
                            "package `{pkg_key}` contains an empty dependency key or target"
                        ),
                    });
                }
            }
        }

        Ok(Self {
            version,
            specifiers: raw.specifiers,
            packages: raw.packages,
        })
    }

    /// Serializes this lockfile as deterministic pretty-printed JSON with a
    /// trailing newline.
    pub fn to_json_pretty(&self) -> Result<String, LockfileError> {
        let mut out = serde_json::to_string_pretty(self).map_err(|err| LockfileError::Io {
            operation: "serialize",
            reason: err.to_string(),
        })?;
        out.push('\n');
        Ok(out)
    }

    /// Reads and parses `neo.lock` from `path`, enforcing [`MAX_LOCKFILE_BYTES`]
    /// and rejecting symlinks (`O_NOFOLLOW` on Unix) and non-regular files.
    pub fn read_from_path(path: &Path) -> Result<Self, LockfileError> {
        let bytes =
            read_bounded_regular_file(path, None, MAX_LOCKFILE_BYTES).map_err(|err| match err {
                CasError::TooLarge { size, .. } => LockfileError::TooLarge {
                    size: size as usize,
                },
                CasError::NotRegularFile => LockfileError::Io {
                    operation: "read",
                    reason: "neo.lock is not a regular file".to_owned(),
                },
                other => LockfileError::Io {
                    operation: "read",
                    reason: other.to_string(),
                },
            })?;
        let text = String::from_utf8(bytes).map_err(|err| LockfileError::InvalidJson {
            reason: format!("neo.lock is not valid UTF-8: {err}"),
        })?;
        Self::parse(&text)
    }

    /// Writes this lockfile atomically to `path` via a same-directory temporary
    /// file, `sync_data`, and rename.
    pub fn write_atomic(&self, path: &Path) -> Result<(), LockfileError> {
        let content = self.to_json_pretty()?;
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        fs::create_dir_all(parent).map_err(|err| LockfileError::Io {
            operation: "create parent directory",
            reason: err.to_string(),
        })?;

        let seq = LOCK_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let tmp_path = parent.join(format!(".neo.lock.{pid}.{seq}.tmp"));

        let write_res = (|| -> Result<(), LockfileError> {
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o644);
            }
            let mut file = opts.open(&tmp_path).map_err(|err| LockfileError::Io {
                operation: "create temp file",
                reason: err.to_string(),
            })?;
            file.write_all(content.as_bytes())
                .map_err(|err| LockfileError::Io {
                    operation: "write temp file",
                    reason: err.to_string(),
                })?;
            file.sync_data().map_err(|err| LockfileError::Io {
                operation: "sync temp file",
                reason: err.to_string(),
            })?;
            drop(file);
            fs::rename(&tmp_path, path).map_err(|err| LockfileError::Io {
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

    /// Verifies that `actual_digest` matches the pinned `integrity` for `pkg_key`.
    ///
    /// Supports both SHA-256 digests (`<64-hex>`, `sha256-<64-hex>`, or
    /// `sha256-<base64>`) and `sha512-<base64>` SRI digests, failing closed on
    /// missing packages, weak/malformed digests, or mismatches.
    pub fn verify_integrity(
        &self,
        pkg_key: &str,
        actual_digest: &str,
    ) -> Result<(), LockfileError> {
        let pkg = self
            .packages
            .get(pkg_key)
            .ok_or_else(|| LockfileError::PackageNotFound {
                pkg_key: pkg_key.to_owned(),
            })?;
        verify_digest_match(pkg_key, &pkg.integrity, actual_digest)
    }

    /// Records all roots and package instances from `graph` into this lockfile.
    ///
    /// Returns `true` if any specifier or package entry was added or changed.
    pub fn record_graph(&mut self, graph: &ResolvedNpmGraph) -> bool {
        let mut changed = false;

        // Assign deterministic lockfile keys for every instance in `graph`.
        // When only one context exists for `<name>@<version>`, use `<name>@<version>`;
        // otherwise keep `<name>@<version>~<ctx>` for distinct peer contexts.
        let mut version_counts: BTreeMap<String, usize> = BTreeMap::new();
        for pkg in graph.instances().values() {
            let base = format!("{}@{}", pkg.name, pkg.version);
            *version_counts.entry(base).or_default() += 1;
        }
        let mut instance_keys: BTreeMap<&str, String> = BTreeMap::new();
        for (instance_id, pkg) in graph.instances() {
            let base = format!("{}@{}", pkg.name, pkg.version);
            let key = if version_counts.get(&base).copied().unwrap_or(0) <= 1 || pkg.ctx == "0" {
                base
            } else {
                instance_id.clone()
            };
            instance_keys.insert(instance_id.as_str(), key);
        }

        for ((name, constraint), opt_instance_id) in graph.roots() {
            let Some(instance_id) = opt_instance_id else {
                continue;
            };
            let Some(pkg_key) = instance_keys.get(instance_id.as_str()) else {
                continue;
            };
            let spec_key = format!("{name}@{constraint}");
            if self.specifiers.get(&spec_key) != Some(pkg_key) {
                self.specifiers.insert(spec_key, pkg_key.clone());
                changed = true;
            }
        }

        for (instance_id, pkg) in graph.instances() {
            let pkg_key = instance_keys[instance_id.as_str()].clone();
            let mut deps = BTreeMap::new();
            for (dep_name, dep_instance_id) in &pkg.dependencies {
                if let Some(target_key) = instance_keys.get(dep_instance_id.as_str()) {
                    deps.insert(dep_name.to_string(), target_key.clone());
                }
            }
            for (dep_name, opt_dep) in &pkg.optional_dependencies {
                if let Some(dep_instance_id) = opt_dep
                    && let Some(target_key) = instance_keys.get(dep_instance_id.as_str())
                {
                    deps.entry(dep_name.to_string())
                        .or_insert_with(|| target_key.clone());
                }
            }
            for (peer_name, opt_peer) in &pkg.peer_dependencies {
                if let Some(peer_instance_id) = opt_peer
                    && let Some(target_key) = instance_keys.get(peer_instance_id.as_str())
                {
                    deps.entry(peer_name.to_string())
                        .or_insert_with(|| target_key.clone());
                }
            }

            let entry = LockedPackage {
                version: pkg.version.clone(),
                integrity: pkg.integrity.clone(),
                tree_sha256: Some(pkg.tree_sha256.clone()),
                tarball_url: pkg.tarball_url.clone(),
                dependencies: deps,
            };
            if self.packages.get(&pkg_key) != Some(&entry) {
                self.packages.insert(pkg_key, entry);
                changed = true;
            }
        }

        changed
    }
}

fn verify_digest_match(
    pkg_key: &str,
    expected_raw: &str,
    actual_raw: &str,
) -> Result<(), LockfileError> {
    let expected = expected_raw.trim();
    let actual = actual_raw.trim();

    if let Some(expected_sha256) = parse_sha256_digest(expected) {
        let Some(actual_sha256) = parse_sha256_digest(actual) else {
            return Err(LockfileError::IntegrityMismatch {
                pkg_key: pkg_key.to_owned(),
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            });
        };
        if expected_sha256 != actual_sha256 {
            return Err(LockfileError::IntegrityMismatch {
                pkg_key: pkg_key.to_owned(),
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            });
        }
        return Ok(());
    }

    if expected.starts_with("sha512-") {
        let expected_sha512 =
            parse_sha512_sri(expected).map_err(|err| LockfileError::InvalidIntegrity {
                pkg_key: pkg_key.to_owned(),
                reason: err.to_string(),
            })?;
        let Ok(actual_sha512) = parse_sha512_sri(actual) else {
            return Err(LockfileError::IntegrityMismatch {
                pkg_key: pkg_key.to_owned(),
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            });
        };
        if expected_sha512 != actual_sha512 {
            return Err(LockfileError::IntegrityMismatch {
                pkg_key: pkg_key.to_owned(),
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            });
        }
        return Ok(());
    }

    Err(LockfileError::InvalidIntegrity {
        pkg_key: pkg_key.to_owned(),
        reason: format!("unsupported or malformed integrity `{expected}`"),
    })
}

fn parse_sha256_digest(input: &str) -> Option<[u8; 32]> {
    if let Some(rest) = input.strip_prefix("sha256-") {
        if let Some(hex_bytes) = decode_hex_32(rest) {
            return Some(hex_bytes);
        }
        let decoded = BASE64_STANDARD.decode(rest).ok()?;
        let arr: [u8; 32] = decoded.try_into().ok()?;
        return Some(arr);
    }
    decode_hex_32(input)
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (idx, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        out[idx] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

#[derive(Default)]
struct RawLockfile {
    version: Option<u32>,
    specifiers: BTreeMap<String, String>,
    packages: BTreeMap<String, LockedPackage>,
}

impl<'de> Deserialize<'de> for RawLockfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LockVisitor;

        impl<'de> Visitor<'de> for LockVisitor {
            type Value = RawLockfile;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a neo.lock JSON object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                let mut out = RawLockfile::default();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom(format!(
                            "duplicate key `{key}` in neo.lock"
                        )));
                    }
                    match key.as_str() {
                        "version" => out.version = Some(map.next_value()?),
                        "specifiers" => {
                            out.specifiers = map.next_value::<NoDupStringMap>()?.0;
                        }
                        "packages" => {
                            out.packages = map.next_value::<NoDupPackagesMap>()?.0;
                        }
                        other => {
                            return Err(de::Error::custom(format!(
                                "unknown field `{other}` in neo.lock"
                            )));
                        }
                    }
                }
                Ok(out)
            }
        }

        deserializer.deserialize_map(LockVisitor)
    }
}

struct NoDupStringMap(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for NoDupStringMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = NoDupStringMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string map")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut out = BTreeMap::new();
                while let Some(k) = map.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(de::Error::custom(format!("duplicate key `{k}`")));
                    }
                    let v = map.next_value()?;
                    out.insert(k, v);
                }
                Ok(NoDupStringMap(out))
            }
        }
        deserializer.deserialize_map(V)
    }
}

struct NoDupPackagesMap(BTreeMap<String, LockedPackage>);

impl<'de> Deserialize<'de> for NoDupPackagesMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = NoDupPackagesMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a packages map")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut out = BTreeMap::new();
                while let Some(k) = map.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(de::Error::custom(format!(
                            "duplicate package key `{k}` in neo.lock"
                        )));
                    }
                    let v = map.next_value()?;
                    out.insert(k, v);
                }
                Ok(NoDupPackagesMap(out))
            }
        }
        deserializer.deserialize_map(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_deterministic_regardless_of_insertion_order() {
        let mut lock_a = Lockfile::new();
        lock_a
            .specifiers
            .insert("zeta@^2.0.0".to_owned(), "zeta@2.1.0".to_owned());
        lock_a
            .specifiers
            .insert("alpha@^1.0.0".to_owned(), "alpha@1.2.3".to_owned());
        lock_a.packages.insert(
            "zeta@2.1.0".to_owned(),
            LockedPackage {
                version: "2.1.0".to_owned(),
                integrity: format!("sha256-{}", "b".repeat(64)),
                tree_sha256: None,
                tarball_url: "https://registry.npmjs.org/zeta/-/zeta-2.1.0.tgz".to_owned(),
                dependencies: BTreeMap::from([("alpha".to_owned(), "alpha@1.2.3".to_owned())]),
            },
        );
        lock_a.packages.insert(
            "alpha@1.2.3".to_owned(),
            LockedPackage {
                version: "1.2.3".to_owned(),
                integrity: format!("sha256-{}", "a".repeat(64)),
                tree_sha256: Some("c".repeat(64)),
                tarball_url: "https://registry.npmjs.org/alpha/-/alpha-1.2.3.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );

        let mut lock_b = Lockfile::new();
        lock_b
            .specifiers
            .insert("alpha@^1.0.0".to_owned(), "alpha@1.2.3".to_owned());
        lock_b
            .specifiers
            .insert("zeta@^2.0.0".to_owned(), "zeta@2.1.0".to_owned());
        lock_b.packages.insert(
            "alpha@1.2.3".to_owned(),
            LockedPackage {
                version: "1.2.3".to_owned(),
                integrity: format!("sha256-{}", "a".repeat(64)),
                tree_sha256: Some("c".repeat(64)),
                tarball_url: "https://registry.npmjs.org/alpha/-/alpha-1.2.3.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock_b.packages.insert(
            "zeta@2.1.0".to_owned(),
            LockedPackage {
                version: "2.1.0".to_owned(),
                integrity: format!("sha256-{}", "b".repeat(64)),
                tree_sha256: None,
                tarball_url: "https://registry.npmjs.org/zeta/-/zeta-2.1.0.tgz".to_owned(),
                dependencies: BTreeMap::from([("alpha".to_owned(), "alpha@1.2.3".to_owned())]),
            },
        );

        let json_a = lock_a.to_json_pretty().unwrap();
        let json_b = lock_b.to_json_pretty().unwrap();
        assert_eq!(json_a, json_b);

        let parsed = Lockfile::parse(&json_a).unwrap();
        assert_eq!(parsed, lock_a);
        assert_eq!(parsed.to_json_pretty().unwrap(), json_a);

        // Keys must appear sorted (`alpha` before `zeta`).
        let alpha_pos = json_a.find("\"alpha@^1.0.0\"").unwrap();
        let zeta_pos = json_a.find("\"zeta@^2.0.0\"").unwrap();
        assert!(alpha_pos < zeta_pos);
    }

    #[test]
    fn rejects_corrupt_unknown_version_duplicate_keys_and_empty_fields() {
        assert!(matches!(
            Lockfile::parse("{not json"),
            Err(LockfileError::InvalidJson { .. })
        ));
        assert!(matches!(
            Lockfile::parse(r#"{"specifiers":{},"packages":{}}"#),
            Err(LockfileError::InvalidJson { .. })
        ));
        assert!(matches!(
            Lockfile::parse(r#"{"version":0,"specifiers":{},"packages":{}}"#),
            Err(LockfileError::UnsupportedVersion { version: 0 })
        ));
        assert!(matches!(
            Lockfile::parse(r#"{"version":2,"specifiers":{},"packages":{}}"#),
            Err(LockfileError::UnsupportedVersion { version: 2 })
        ));
        assert!(matches!(
            Lockfile::parse(r#"{"version":1,"unknown":true}"#),
            Err(LockfileError::InvalidJson { .. })
        ));
        assert!(matches!(
            Lockfile::parse(r#"{"version":1,"specifiers":{"a":"a@1","a":"a@2"},"packages":{}}"#),
            Err(LockfileError::InvalidJson { .. })
        ));
        assert!(matches!(
            Lockfile::parse(
                r#"{"version":1,"specifiers":{},"packages":{"a@1":{"version":"","integrity":"sha256-aaaa"}}}"#
            ),
            Err(LockfileError::InvalidJson { .. })
        ));
        assert!(matches!(
            Lockfile::parse(
                r#"{"version":1,"specifiers":{},"packages":{"a@1":{"version":"1.0.0","integrity":""}}}"#
            ),
            Err(LockfileError::InvalidJson { .. })
        ));
    }

    #[test]
    fn atomic_read_and_write_enforce_bounds_and_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(LOCKFILE_NAME);

        let mut lock = Lockfile::new();
        lock.specifiers
            .insert("lodash@^4.17.21".to_owned(), "lodash@4.17.21".to_owned());
        lock.packages.insert(
            "lodash@4.17.21".to_owned(),
            LockedPackage {
                version: "4.17.21".to_owned(),
                integrity: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_owned(),
                tree_sha256: None,
                tarball_url: "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );

        lock.write_atomic(&path).unwrap();
        let loaded = Lockfile::read_from_path(&path).unwrap();
        assert_eq!(loaded, lock);

        // No leftover temp files in directory.
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["neo.lock"]);

        // Oversized lockfile fails closed before JSON parsing.
        let big_path = tmp.path().join("big.lock");
        let f = fs::File::create(&big_path).unwrap();
        f.set_len((MAX_LOCKFILE_BYTES as u64) + 1).unwrap();
        assert!(matches!(
            Lockfile::read_from_path(&big_path),
            Err(LockfileError::TooLarge { .. })
        ));
    }

    #[test]
    fn verify_integrity_fails_closed_on_mismatch_missing_or_weak_digest() {
        let hex_digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let b64_digest = BASE64_STANDARD.encode([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
            0x89, 0xab, 0xcd, 0xef,
        ]);
        let sha512_sri = format!("sha512-{}", BASE64_STANDARD.encode([0x5a; 64]));

        let mut lock = Lockfile::new();
        lock.packages.insert(
            "hex@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: hex_digest.to_owned(),
                tree_sha256: None,
                tarball_url: String::new(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.packages.insert(
            "sri256@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: format!("sha256-{b64_digest}"),
                tree_sha256: None,
                tarball_url: String::new(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.packages.insert(
            "sri512@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: sha512_sri.clone(),
                tree_sha256: None,
                tarball_url: String::new(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.packages.insert(
            "weak@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: "sha1-deadbeef".to_owned(),
                tree_sha256: None,
                tarball_url: String::new(),
                dependencies: BTreeMap::new(),
            },
        );

        assert!(lock.verify_integrity("hex@1.0.0", hex_digest).is_ok());
        assert!(
            lock.verify_integrity("hex@1.0.0", &format!("sha256-{b64_digest}"))
                .is_ok()
        );
        assert!(lock.verify_integrity("sri256@1.0.0", hex_digest).is_ok());
        assert!(lock.verify_integrity("sri512@1.0.0", &sha512_sri).is_ok());

        // Wrong digest fails closed.
        let wrong_hex = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        assert!(matches!(
            lock.verify_integrity("hex@1.0.0", wrong_hex),
            Err(LockfileError::IntegrityMismatch { .. })
        ));
        assert!(matches!(
            lock.verify_integrity("sri256@1.0.0", wrong_hex),
            Err(LockfileError::IntegrityMismatch { .. })
        ));
        assert!(matches!(
            lock.verify_integrity(
                "sri512@1.0.0",
                &format!("sha512-{}", BASE64_STANDARD.encode([0x00; 64]))
            ),
            Err(LockfileError::IntegrityMismatch { .. })
        ));

        // Missing package & weak integrity fail closed.
        assert!(matches!(
            lock.verify_integrity("missing@1.0.0", hex_digest),
            Err(LockfileError::PackageNotFound { .. })
        ));
        assert!(matches!(
            lock.verify_integrity("weak@1.0.0", "sha1-deadbeef"),
            Err(LockfileError::InvalidIntegrity { .. })
        ));
    }
}
