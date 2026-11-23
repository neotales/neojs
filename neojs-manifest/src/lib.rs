//! Foundation types for a `neo.jsonc` project manifest.
//!
//! This crate is deliberately narrower than "manifest support". It parses the
//! JSONC dialect, validates the fields this project has agreed on, and
//! normalizes import specifiers into canonical forms. It does **not** discover a
//! manifest automatically, install an import map into a runtime, expand
//! workspaces, apply `include` or `exclude` patterns, fetch a package, or
//! implement npm, JSR, GitHub, or Node module loading.
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
//! The `jsr:` mapping above is normalized but not installable: this crate does
//! not install an import map. Every specifier
//! reports its [`Support`] status, and no runtime transport is installed by this
//! crate.
//!
//! # Status
//!
//! | Area | Status |
//! | --- | --- |
//! | JSONC parsing, duplicate-key detection, size limits | Implemented |
//! | Manifest field validation | Implemented for the documented keys |
//! | `gh:`, `github:`, `npm:`, `jsr:`, and `node:` normalization | Implemented as metadata |
//! | Import-map application | Not implemented |
//! | Workspace expansion | Not implemented |
//! | `include` / `exclude` matching | Not implemented |
//! | npm, JSR, and Node loading by this crate | Not implemented |
//! | GitHub loading | Implemented by the V8 runtime, not this crate |
//! | Node compatibility transform | Not implemented |
//!
//! No function in this crate reads the network or writes a file. [`Manifest::from_path`]
//! reads the one path it is given.

mod error;
pub mod jsonc;
mod manifest;
mod specifier;

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
pub use specifier::GhSpecifier;
pub use specifier::JsrSpecifier;
pub use specifier::MAX_SPECIFIER_BYTES;
pub use specifier::NormalizedSpecifier;
pub use specifier::NpmSpecifier;
pub use specifier::SpecifierKind;
pub use specifier::Support;
pub use specifier::normalize_specifier;
