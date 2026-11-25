//! Shared metadata, ECMAScript source, and host helpers for the `neo:path`
//! built-in module.
//!
//! The lexical path implementation lives in `path.js` as a self-contained
//! factory function parameterized by `{ isWindows, readCwd }`, plus
//! `path.esm.js` for the ES module wrapper. Neither file depends on V8 or
//! QuickJS. Provenance and MIT notices are in
//! `neojs-core/src/builtins/path.LICENSE.md` and
//! `neojs-core/THIRD_PARTY_NOTICES.md`.

use crate::{Backend, BuiltinModule};

/// The canonical `neo:path` module specifier and registry key.
pub const SPECIFIER: &str = "neo:path";

/// The named exports of `neo:path`, matching `path.esm.js`.
pub const EXPORTS: &[&str] = &[
    "posix",
    "win32",
    "join",
    "normalize",
    "basename",
    "dirname",
    "extname",
    "isAbsolute",
    "parse",
    "format",
    "common",
    "resolve",
    "relative",
    "toNamespacedPath",
    "SEPARATOR",
    "DELIMITER",
    "SEPARATOR_PATTERN",
];

/// Factory expression `(function createNeoPath(host) { ... })` returning the
/// frozen `neo:path` module object.
pub const FACTORY_SOURCE: &str = include_str!("path.js");

/// ECMAScript module source re-exporting `Neo.getBuiltInModule("neo:path")`.
pub const ESM_SOURCE: &str = include_str!("path.esm.js");

/// Shared descriptor for `neo:path`.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: Backend::Ecmascript,
};

/// Returns `true` when compiled for a Windows target (`cfg!(windows)`).
#[must_use]
pub const fn is_windows() -> bool {
    cfg!(windows)
}

/// Returns the current process working directory as strict UTF-8, or `None`
/// when the directory cannot be read or is not valid UTF-8.
#[must_use]
pub fn current_dir() -> Option<String> {
    let directory = std::env::current_dir().ok()?;
    let text = directory.to_str()?;
    Some(text.to_owned())
}
