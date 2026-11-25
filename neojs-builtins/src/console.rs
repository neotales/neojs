//! Shared metadata and ECMAScript source for the `neo:console` built-in module.
//!
//! The formatting and writer installation live in `console.js` as a
//! self-contained factory function parameterized by a host `print(text, isError)`
//! callback and the target `console` object, plus `console.esm.js` for the ES
//! module wrapper. Neither file depends on V8 or QuickJS.

use crate::{Backend, BuiltinModule};

/// The canonical `neo:console` module specifier and registry key.
pub const SPECIFIER: &str = "neo:console";

/// The named exports of `neo:console`, matching `console.esm.js`.
pub const EXPORTS: &[&str] = &["log", "debug", "info", "warn", "error", "default"];

/// Factory expression `(function installNeoConsole(host) { ... })` installing
/// the five formatted console methods onto `host.target`.
pub const INSTALLER_SOURCE: &str = include_str!("console.js");

/// ECMAScript module source re-exporting `Neo.getBuiltInModule("neo:console")`.
pub const ESM_SOURCE: &str = include_str!("console.esm.js");

/// Shared descriptor for `neo:console`.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: Backend::Ecmascript,
};
