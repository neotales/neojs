//! Shared metadata and ECMAScript source for the `neo:assert` built-in module.
//!
//! The runtime logic lives in `assert.js` as a self-contained factory function
//! parameterized by two engine-provided Promise observation hooks
//! (`isNativePromise` and `observePromise`), plus `assert.esm.js` for the ES
//! module wrapper. Neither file depends on V8 or QuickJS.

use crate::{Backend, BuiltinModule};

/// The canonical `neo:assert` module specifier and registry key.
pub const SPECIFIER: &str = "neo:assert";

/// The named exports of `neo:assert`, matching `assert.esm.js`.
pub const EXPORTS: &[&str] = &[
    "default",
    "assert",
    "AssertionError",
    "ok",
    "equals",
    "notEquals",
    "strictEquals",
    "notStrictEquals",
    "fail",
    "throws",
    "doesNotThrow",
    "rejects",
    "doesNotReject",
];

/// Factory expression `(function createNeoAssert(host) { ... })` returning the
/// frozen callable `assert` module object.
pub const FACTORY_SOURCE: &str = include_str!("assert.js");

/// ECMAScript module source re-exporting `Neo.getBuiltInModule("neo:assert")`.
pub const ESM_SOURCE: &str = include_str!("assert.esm.js");

/// Shared descriptor for `neo:assert`.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: Backend::Ecmascript,
};
