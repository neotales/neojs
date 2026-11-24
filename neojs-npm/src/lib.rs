//! Engine-independent `npm:` package resolution, verification, and virtual
//! graph core for `neojs`.
//!
//! This crate has zero JavaScript engine dependencies (`deno_core`, `deno_ast`,
//! `v8`, and `rquickjs` are absent) and is offline by default (`default = []`).
//! Enabling the optional `network` feature activates `RemoteNpmRegistry::new`
//! via `neojs-remote/network`.
//!
//! # Key Guarantees
//!
//! - **Zero `node_modules`:** Resolved modules receive peer-context-isolated
//!   virtual `npm://<registry_host>/<pkg>/<version>/<ctx>/<rel_path>` URLs.
//! - **Content-verified CAS execution:** Extracted files live in immutable
//!   `cas/sha256/<aa>/<bb>/<digest>` (`0444`) and are verified on every read
//!   with `O_NOFOLLOW` + exact byte length + SHA-256 before returning bytes.
//! - **Zero auto-install scripts:** Lifecycle hooks (`preinstall`, `install`,
//!   `postinstall`, etc.) are audited into [`ScriptAudit`] and never executed;
//!   native `.node`/`.dll`/`.so`/`.dylib` binaries are excluded from CAS.
//! - **Strict ESM only:** Active conditions are fixed to `["import", "default"]`
//!   in manifest key order, and every loaded file is checked via
//!   [`neojs_module_policy::package_module_format`].
//!
//! # Example
//!
//! ```
//! use neojs_npm::{NpmSpecifier, PackageJson, VersionConstraint};
//!
//! let spec = NpmSpecifier::parse("npm:@scope/pkg@^1.2.3/sub.mjs").unwrap();
//! assert_eq!(spec.name().as_str(), "@scope/pkg");
//! assert_eq!(spec.subpath(), Some("sub.mjs"));
//! assert!(matches!(spec.constraint().unwrap(), VersionConstraint::Range(_)));
//!
//! let pkg = PackageJson::parse_str(r#"{"name":"demo","type":"module","exports":"./index.js"}"#).unwrap();
//! assert!(pkg.ignored_lifecycle_scripts().is_empty());
//! ```

pub mod cas;
pub mod exports;
pub mod package_json;
pub mod resolver;
pub mod semver;
pub mod specifier;
pub mod tarball;

pub use semver as npm_semver;

pub use cas::{CasDigest, CasError, CasStore, sha256_hex};
pub use exports::{
    ACTIVE_CONDITIONS, ExportsError, ExportsResolver, PackageImportTarget, ResolvedPackageFile,
};
pub use package_json::{
    ExportsField, LIFECYCLE_SCRIPT_HOOKS, MAX_PACKAGE_JSON_BYTES, PackageJson, PackageJsonError,
    PeerDependencyMeta, ScriptAudit, TargetPlatform,
};
pub use resolver::{
    DEFAULT_REGISTRY_ORIGIN, LoadedNpmModule, MAX_DISCOVERY_DEPTH, NpmGraphResolver,
    NpmReferrerResolution, NpmRegistry, NpmRootRequest, Packument, PackumentDist, PackumentVersion,
    ProjectPackageResolver, ProjectResolution, RemoteNpmRegistry, ResolvedNpmGraph,
    ResolvedNpmPackage, ResolverError,
};
pub use semver::{
    MAX_COMPARATORS_PER_CLAUSE, MAX_CONSTRAINT_BYTES, MAX_OR_CLAUSES, NpmRange, NpmVersion,
    PrereleaseId, SemverError, VersionConstraint,
};
pub use specifier::{
    MAX_PACKAGE_NAME_BYTES, MAX_SPECIFIER_BYTES, MAX_SUBPATH_BYTES, NpmSpecifier, PackageName,
    ParsedVirtualUrl, SpecifierError, build_virtual_url, parse_virtual_url,
    split_bare_package_specifier,
};
pub use tarball::{NpmLimits, PackageTree, TarballError, TarballIngestor, TreeFileEntry};
