//! Shared metadata and ECMAScript source for the `neo:test` built-in module.
//!
//! The runtime logic lives in `test.js` as a self-contained factory function
//! parameterized by engine-provided host hooks (output, Promise observation,
//! rejection tracking, job/event-loop checks, and optional timers), plus
//! `test.esm.js` for the ES module wrapper. Neither file depends on V8 or
//! QuickJS.

use crate::{Backend, BuiltinModule};

/// The canonical `neo:test` module specifier and registry key.
pub const SPECIFIER: &str = "neo:test";

/// The named exports of `neo:test`, matching `test.esm.js`.
pub const EXPORTS: &[&str] = &["test", "run", "MAX_TESTS", "TestFailureError", "default"];

/// Factory expression `(function createNeoTest(host) { ... })` returning
/// `{ module, ensureHandler }`.
pub const FACTORY_SOURCE: &str = include_str!("test.js");

/// ECMAScript module source re-exporting `Neo.getBuiltInModule("neo:test")`.
pub const ESM_SOURCE: &str = include_str!("test.esm.js");

/// Shared descriptor for `neo:test`.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: Backend::Ecmascript,
};
