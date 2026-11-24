//! Foundation types, bounded discovery, workspace expansion, and import
//! resolution for `neo.jsonc` project manifests.
//!
//! This crate parses the JSONC dialect, validates the fields this project has
//! agreed on, discovers manifests upward within explicit bounds, expands
//! workspace member globs deterministically, and resolves `imports` and
//! workspace `@scope/pkg` references without performing network I/O or
//! installing hooks into any JavaScript runtime. It does **not** apply
//! `include` or `exclude` patterns, fetch a package, or implement npm, JSR,
//! GitHub, or Node module loading.
//!
//! The manifest shape is inspired by Deno's `deno.json`, but it is not a Deno
//! compatibility format. The accepted keys and their exact rules are documented
//! on [`Manifest`]. Unknown keys are rejected instead of ignored.
//!
//! # Example
//!
//! ```
//! use neojs_manifest::Manifest;
//!
//! let manifest = Manifest::parse(
//!     r#"{
//!         // Comments and a trailing comma are accepted.
//!         "name": "@scope/pkg",
//!         "exports": "./mod.ts",
//!         "imports": { "std": "jsr:@scope/std@1.2.3" },
//!     }"#,
//! )
//! .expect("valid manifest");
//!
//! assert_eq!(manifest.name.as_str(), "@scope/pkg");
//! assert_eq!(
//!     manifest.imports.get("std").expect("mapping").canonical(),
//!     "jsr:@scope/std@1.2.3"
//! );
//! ```
//!
//! The `jsr:` mapping above is normalized and resolved as metadata:
//! [`ImportResolver`] preserves [`Support`] on non-local targets so the calling
//! runtime continues to enforce remote and unsupported-scheme policies.
//!
//! # Status
//!
//! | Area | Status |
//! | --- | --- |
//! | JSONC parsing, duplicate-key detection, size limits | Implemented |
//! | Manifest field validation | Implemented for the documented keys |
//! | `gh:`, `github:`, `npm:`, `jsr:`, and `node:` normalization | Implemented as metadata |
//! | Bounded upward discovery ([`discover_manifest`]) | Implemented |
//! | Deterministic workspace expansion ([`Workspace`]) | Implemented |
//! | Pure import-map & workspace resolution ([`ImportResolver`]) | Implemented (not wired into runtimes) |
//! | `include` / `exclude` matching | Not implemented |
//! | npm, JSR, and Node loading by this crate | Not implemented |
//! | GitHub loading | Implemented by the V8 runtime, not this crate |
//! | Node compatibility transform | Not implemented |
//!
//! No function in this crate reads the network or writes a file.
//! [`Manifest::from_path`], [`discover_manifest`], [`Workspace::load`], and
//! [`ImportResolver`] read only bounded local manifest metadata.

mod discovery;
mod error;
pub mod jsonc;
mod manifest;
mod resolver;
mod specifier;
mod workspace;

pub use discovery::DiscoveredManifest;
pub use discovery::MAX_DISCOVERY_DEPTH;
pub use discovery::discover_manifest;
pub use error::Error;
pub use manifest::ExportKey;
pub use manifest::ExportTarget;
pub use manifest::Exports;
pub use manifest::ImportMap;
pub use manifest::MANIFEST_FILE_NAME;
pub use manifest::MAX_MANIFEST_BYTES;
pub use manifest::Manifest;
pub use manifest::PackageName;
pub use manifest::Pattern;
pub use resolver::ImportResolver;
pub use resolver::ResolvedImport;
pub use specifier::GhSpecifier;
pub use specifier::JsrSpecifier;
pub use specifier::MAX_SPECIFIER_BYTES;
pub use specifier::NormalizedSpecifier;
pub use specifier::NpmSpecifier;
pub use specifier::SpecifierKind;
pub use specifier::Support;
pub use specifier::normalize_specifier;
pub use workspace::MAX_WORKSPACE_MEMBERS;
pub use workspace::Workspace;
pub use workspace::WorkspaceMember;
